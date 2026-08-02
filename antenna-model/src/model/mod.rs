//! Physical Optics Model Components
//!
//! This module contains the physical optics computation engine for parabolic
//! reflector antenna pattern modeling. It includes:
//!
//! - **Geometry**: Physical antenna parameters (reflector, feed, mesh)
//! - **Coordinates**: Coordinate system transformations
//! - **Phase**: Phase functions for aperture integration
//! - **Illumination**: Feed pattern models and aperture illumination
//! - **Integration**: Aperture integration engine for far-field patterns
//! - **Pattern**: Far-field pattern computation, gain, and G/T calculations
//! - **Mesh**: Wire mesh reflector physics (transparency, angle effects, polarization)
//! - **Edge Cases**: Detection and handling of edge cases (large feed offsets, spillover)
//! - **Ray Trace**: Ray tracing for large feed offset scenarios

pub mod bessel;
pub mod coordinates;
pub mod coordinates_3d;
pub mod correction_interpolator;
pub mod edge_cases;
/// Mixed-radix FFT backing the aperture integrator's φ' transform (roadmap P10-perf).
///
/// Crate-internal: it exists for `integration.rs` and is not part of the public model API.
pub(crate) mod fft;
pub mod geometry;
pub mod illumination;
pub mod integration;
pub mod mesh;
pub mod pattern;
pub mod phase;
pub mod ray_trace;

/// Version of the physics model's gain computation.
///
/// Correction surfaces are fitted to `measured − physics` residuals, so any change
/// that alters `gain_physics` output for identical inputs invalidates surfaces fitted
/// against the older model. Calibration artifacts record the version they were fitted
/// against (`CalibrationMetadata::physics_model_version`) and the loader warns on
/// mismatch (`data/loader.rs`).
///
/// # Bump policy
/// Bump whenever a change alters `gain_physics` output for identical inputs
/// (new efficiency terms, phase-model changes, defocus semantics, ...).
///
/// # History
/// - 1: baseline at introduction (P1b) — post-P1 model (spillover applied on the
///   uncalibrated path, fractional-q spillover fix)
/// - 2: P7 auto-refocus — `phase_center_offset` no longer contributes axial defocus
///   (compensated feed property); deliberate defocus via the new `axial_defocus` field
/// - 3: P10 off-axis integrator (2026-07) — the Hankel / azimuthal-mode aperture
///   integrator replaced the aliasing fixed-density quadrature, correcting
///   `gain_physics` at off-axis angles for identical inputs (converged physical
///   optics, no aliasing). NOTE: the F7 statistical sidelobe floor is a
///   service-layer param (`IntegrationParams::apply_sidelobe_floor`), OFF on the
///   served path per decision D-2 (superseded in v5 — the F7 redesign turned the
///   floor ON via a power sum), and is NOT part of the calibration-fitting
///   physics — calibrated antennas never had it applied, so it does not gate this
///   version.
/// - 4: P2 removal of the `HigherOrderAberrations` computation mode (2026-07) — feed
///   offsets in the 0.3f–0.5f band now route through `StandardPhysicalOptics`, whose
///   exact geometric coma phase (`phase::phase_feed_displacement`) already carries the
///   full low-order aberration content. The removed mode stacked wrong-sign/wrong-scale
///   Seidel terms on top of that exact phase (double-count), so served gain in that
///   offset band changes by construction — that IS the fix. No enabled antenna enters
///   this band (max served offset 0.027f), so no served value changes in practice.
///   The bump follows P1b's policy (stamp whenever `gain_physics` changes for identical
///   inputs — it does, in the 0.3f–0.5f band), independent of whether any *currently
///   enabled* antenna is affected. No `.bin` calibration artifacts exist in the wild, so
///   the loader's version-mismatch warning (warn, never error) fires against nothing
///   today; it exists to flag genuinely stale surfaces once artifacts are produced.
/// - 5: F7 redesign (2026-07-16) — Huygens obliquity factor (1+cosθ)/2 on the far-field
///   conversion (all antennas), and the statistical Ruze sidelobe floor re-enabled on the
///   uncorrected-physics path as an incoherent power sum forward / floor-only behind the
///   dish (gated on `physics_is_uncorrected()`).
/// - 6: P12 radial convergence on the azimuthal-mode path (2026-07-31) — the asymmetric
///   (Jₘ) branch of `integration::integrate_aperture` now verifies its RADIAL quadrature
///   instead of assuming it. Before this, that branch sized `n_rho` once and self-checked
///   only azimuthal mode truncation, so `converged = true` asserted nothing about the axis
///   that actually failed. Served gain moves on **every feed that is laterally offset or has
///   `asymmetry_factor != 1.0`** — five of the enabled feeds — by up to several dB at some
///   angles: measured against a converged reference, `gs_3.7m`/`x_band_feed` at θ=5° was
///   0.82 dB low, `dsn_34m`/`x_band` at θ=0.10° 1.17 dB low, and D12's UHF fixture at θ=16°
///   **7.08 dB** low; all now land within 0.013 dB. The symmetric (J₀) branch is untouched
///   and its boresight reference anchors do not move. Also in this bump: `adaptive()`'s
///   `min_rho_points` 16 → 32 (D-B), which matches `calibrate`'s `default()` and closes
///   D17's leftover preset divergence. See
///   `docs/findings-2026-07-31-p12-mode-path-radial-budget.md`.
/// - 7: The steered-feed φ' cap removed (2026-07-31, same unit as 6, landed immediately
///   after it). `MODE_PHI_STEERED_MAX` clamped the φ' DFT to 64 samples for any feed past
///   `δ/f = 0.05`, documented as safe because the `M`-vs-`M+1` self-check would report
///   non-convergence. It cannot: φ' under-sampling corrupts every `gₘ` including the two
///   being compared, and at θ=0 the check is identically zero because `Jₘ(0) = 0` for
///   `m > 0`. Measured against the 2D Simpson oracle on a 34 m dish with a 1.19 m offset
///   (δ/f = 0.0875, a routine ~5° beam steer): the clamp was wrong by up to **+82 dB**,
///   silently. `n_phi` is now sized from the aperture function's azimuthal bandwidth
///   `B = k·δ·(R/f)`, no longer rounded to a power of two, with `MODE_PHI_MAX` raised
///   512 → 2048; when the ceiling still binds, `ModeSizing::azimuthally_resolved` is false
///   and `converged` follows. Served gain moves on any evaluation with `δ/f > 0.05` —
///   which no enabled antenna's *design* feed reaches (max 0.027), but request-driven
///   steering does. Costs ~69× more per evaluation in that regime; P10-perf's FFT for the
///   `gₘ` φ'-DFT is what recovers it. See the findings doc §7.
/// - 8: P13 retired the mode path's radial **pre-gate** (2026-08-01). On expensive geometries
///   P12 had let a cheap `{0,1}`-mode partial leg *certify* radial convergence, in which case
///   the integrator returned the COARSE `N` leg; everywhere else the honest N-vs-2N check ran
///   and the FINE `2N` leg was returned. Those geometries now take the honest path too, so
///   served gain moves there — by the difference between the two legs, measured **+0.0126 →
///   +0.0008 dB** at `dsn_34m`/`ka_band` θ=5° (16× more accurate) — for ~28% more work. The
///   affected regime is the wide-angle / high-frequency asymmetric one: the four `dsn_34m` Ka
///   points and `dsn_34m` X-band beyond ~20°. Everything below the old work threshold is
///   bit-identical, since it never reached the pre-gate.
///
///   The pre-gate was retired rather than re-tuned because both of its premises had expired.
///   Its economic premise — that a 2-mode leg is far cheaper than a full one — held when the
///   φ' transform was an `O(n_phi·M)` DFT (~18% of a full leg); after P10-perf made it an FFT
///   a probe leg is **66%** of a full one, so it saved ~28% rather than ~3×, and it bought that
///   by returning the less accurate leg. Its safety premise — `RADIAL_PRE_GATE_SAFETY = 32`
///   bounding the probe-to-total error ratio — was measured across a θ × D/λ sweep at **43.5×**
///   on `dsn_34m` Ka θ=90°, i.e. the constant did not bound the quantity it existed to bound,
///   on a served antenna at a served angle. See
///   `docs/findings-2026-08-01-p13-pre-gate-retirement.md`.
/// - 9: P14 Bessel accuracy (2026-08-01). Two changes in `model::bessel`, neither a change of
///   *model*: the Miller downward recurrence now starts an offset that scales with the
///   turning-point width (`12·x^(1/3)`, derived) instead of a flat 40 orders, and `J₀`/`J₁`
///   use the convergent ascending series below |x| = 8 instead of a rational fit that was
///   ~3e-9 absolute and evaluated `J₀(0)` as `1 + 2.83e-9`.
///
///   **This bump is bookkeeping, not a warning.** It follows P1b's literal policy — stamp
///   whenever `gain_physics` changes for identical inputs — and it does change, at the
///   **~1e-7 dB** level: 2.8e-9 relative in field amplitude at boresight from the `J₀` seed,
///   and up to 2e-8 relative from the recurrence at the served `MODE_M_MAX = 254`. That is
///   seven orders inside the mode-truncation budget and further still inside any measurement,
///   so no reference anchor, oracle cross-check or convergence pin moved. Unlike versions 6–8
///   there is no served value a reader should go re-examine; what changed is that the
///   routine's error stopped *growing with argument*, which is what makes raising
///   `MODE_M_MAX` safe.
///
///   **Cost — this one is not free.** The Miller start offset grows 40 → 45/56/71/76 at
///   `|x|` = 50/100/200/254, and an A/B of the production recurrence at both offsets (release,
///   200k reps) measures the *sweep itself* **+13.5% to +19.4%**, ~+15% at the served
///   `MODE_M_MAX = 254`. The served mode path absorbs a fraction of that: P10-perf left ~85% of
///   a sweep in aperture-plane evaluation rather than the `Jₘ` ladder, bounding the end-to-end
///   effect at ≲2% — a bound from that published profile, not an end-to-end A/B. Reference
///   points measured after this change, `dsn_34m` X-band on the mode path: **4.7 / 37.7 /
///   57.6 ms** at θ = 5° / 45° / 90°. Well inside S3's wall-clock budget, and the full test
///   suite is unmoved at 227 s. Flagged because this repo tracks mode-path wall clock against
///   that budget, and a speed-for-accuracy trade should be visible where the accuracy claim is.
pub const PHYSICS_MODEL_VERSION: u32 = 9;

// Re-export commonly used types
pub use bessel::{bessel_j0, bessel_j1, bessel_jn};

pub use coordinates::{
    normalize_angle, normalize_angle_symmetric, ApertureCoordinates, EClockConeCoordinates,
    FarFieldCoordinates,
};

pub use coordinates_3d::{
    antenna_frame_to_spherical, apply_beam_squint_correction, beam_deviation_factor,
    compute_emitter_direction, compute_emitter_direction_with_attitude,
    compute_feed_position_from_pointing, ecef_to_enu_rotation, ecef_to_geodetic, geodetic_to_ecef,
    is_ecef_coordinates, normalize_azimuth_deg, quaternion_rotate, squint_corrected_direction,
    validate_ecef, validate_geodetic,
};

pub use correction_interpolator::{evaluate_correction, CorrectionResult};

pub use geometry::{
    AntennaConfiguration, AntennaConfigurationBuilder, FeedParameters, FeedParametersBuilder,
    FeedPosition, MeshParameters, MeshParametersBuilder, MeshPattern, ReflectorGeometry,
    ReflectorGeometryBuilder,
};

pub use illumination::{
    cos_q_pattern, edge_taper_db, feed_angle, illumination_amplitude, q_factor_from_taper,
};

pub use integration::{
    compute_far_field, far_field_normalization, integrate_aperture, IntegrationParams,
    IntegrationResult,
};

pub use pattern::{
    compute_beamwidth, compute_g_over_t, compute_gain, compute_gain_db, g_over_t_from_gain_db,
    overall_efficiency, ruze_efficiency, theoretical_max_gain,
};

pub use phase::{
    angle_of_incidence, phase_feed_displacement, phase_mesh, phase_path, phase_surface_error,
    phase_total, wavelength_from_frequency, wavenumber,
};

pub use mesh::mesh_reflection_efficiency;

pub use edge_cases::{
    analyze_edge_cases, apply_gain_floor, apply_gain_floor_db, needs_adaptive_integration,
    ComputationMode, EdgeCaseAnalysis, MIN_GAIN_FLOOR, MIN_GAIN_FLOOR_DB, SEVERE_OFFSET_THRESHOLD,
};

pub use ray_trace::{compute_gain_ray_trace, ray_trace_aperture, Ray, RayTraceResult};
