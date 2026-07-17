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
///   served path per decision D-2, and is NOT part of the calibration-fitting
///   physics — calibrated antennas never had it applied, so it does not gate this
///   version.
/// - 4: P2 removal of the `HigherOrderAberrations` computation mode (2026-07) — feed
///   offsets in the 0.3f–0.5f band now route through `StandardPhysicalOptics`, whose
///   exact geometric coma phase (`phase::phase_feed_displacement`) already carries the
///   full low-order aberration content. The removed mode stacked wrong-sign/wrong-scale
///   Seidel terms on top of that exact phase (double-count), so served gain in that
///   offset band changes by construction — that IS the fix. No enabled antenna enters
///   this band (max served offset 0.027f), so no served value changes in practice.
pub const PHYSICS_MODEL_VERSION: u32 = 4;

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
    compute_beamwidth, compute_g_over_t, compute_gain, compute_gain_db, overall_efficiency,
    ruze_efficiency, theoretical_max_gain,
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
