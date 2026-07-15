//! Aperture Integration Engine
//!
//! This module implements numerical integration over the reflector aperture
//! to compute far-field antenna patterns using physical optics.
//!
//! # Mathematical Foundation
//!
//! The far-field electric field is computed via aperture integration:
//! ```text
//! E(θ,φ) = (jk·exp(-jkr))/(2λr) ∬_Aperture A(ρ,φ') · exp[jΨ(ρ,φ')] · ρ dρ dφ'
//! ```
//!
//! where:
//! - A(ρ,φ') is the aperture illumination amplitude (from feed pattern)
//! - Ψ(ρ,φ') is the total phase (path + coma + surface + mesh)
//! - Integration limits: ρ ∈ [0, D/2], φ' ∈ [0, 2π]
//!
//! # Numerical Methods
//!
//! Uses composite Simpson's rule with adaptive refinement:
//! - 2D integration via nested 1D integration
//! - Adaptive grid refinement for accuracy
//! - Convergence monitoring
//!
//! # References
//! - Design doc Section 2.1 (Core Physical Optics Model)
//! - Implementation plan Sprint 2, Task 2.4

use num_complex::Complex64;
use std::f64::consts::PI;

use crate::error::{ComputationError, ComputationResult};
use crate::model::{
    bessel::{bessel_j0, bessel_jn},
    edge_cases::higher_order_aberrations,
    geometry::AntennaConfiguration,
    illumination::illumination_amplitude,
    wavelength_from_frequency, wavenumber,
};
// `phase_total` and `ApertureCoordinates` are only used by the retained 2D reference
// integrand (`aperture_integrand`), which is now test-only: since P10 Task 2 the
// production asymmetric path uses `azimuthal_mode_field`, not the 2D quadrature.
// Gate the imports so production builds stay warning-clean under `-D warnings`.
#[cfg(test)]
use crate::model::{coordinates::ApertureCoordinates, phase::phase_total};

/// Floor for the adaptive φ' Fourier-coefficient sample count `n_phi` used to build
/// `g_m(ρ)`. A small coma (the served feeds) needs only a handful of modes, so this floor
/// keeps near-boresight / small-offset evaluations cheap.
const MODE_PHI_MIN: usize = 64;

/// Ceiling for the adaptive φ' sample count `n_phi`. The azimuthal bandwidth of the
/// aperture-plane phase `g(ρ,φ')` is physically bounded by `k·R` (the aperture's k-space
/// radius), so `n_phi` never needs to exceed `~2·k·R`; this cap bounds the pathological
/// heavily-steered-feed case (the ray-tracing regime, D-5). Empirically 512 resolves the
/// `g_0` DFT to convergence even for a feed steered a full aperture-radius off-axis
/// (`k·R ≈ 443`), where n_phi=64/128 alias badly — the root of the interim off-axis error.
const MODE_PHI_MAX: usize = 512;

/// Reduced `n_phi` ceiling for the large-steering regime (feed steered past `δ/f >`
/// [`MODE_STEERING_RATIO`]). Such a feed's aperture-plane phase has a very wide azimuthal
/// spectrum whose exact resolution is neither affordable within the latency budget (a
/// dense off-axis sweep would need hundreds of modes per point) nor calibrated-grade for
/// an idealized PO model (beyond a few degrees of beam-steer, blockage/aberration/
/// diffraction dominate the real off-axis level, not coma). Beyond the threshold the mode
/// count is capped and the `M`-vs-`M+1` self-check honestly reports `converged=false`.
/// Physical offset feeds (δ/f ≪ threshold) keep the full [`MODE_PHI_MAX`]. Kept above
/// `2·(steered M cap)` — see [`mode_count_for`].
const MODE_PHI_STEERED_MAX: usize = 64;

/// Feed-steering ratio `δ/f` (≈ the beam-steer angle in radians) above which the coma is
/// strong enough that the azimuthal-mode expansion is performance-capped rather than fully
/// resolved. `radial_points_for` / `mode_count_for` apply reduced caps beyond it (fewer
/// φ' samples, capped mode count, capped coma radial density), and the self-check flags the
/// resulting under-resolution.
///
/// Every physical served offset feed sits an order of magnitude below this (the largest,
/// gs_3.7m X-band, is δ/f ≈ 0.027; dsn_34m/13m ≈ 0.01–0.02), so they are always fully
/// resolved. It trips only for strongly steered feeds — a beam-steer test displacement
/// (δ/f ≈ 0.09, ~5°) or request-driven steering of order the focal length (the D-5
/// ray-tracing regime), where the exact off-axis level is neither affordable nor
/// PO-trustworthy.
const MODE_STEERING_RATIO: f64 = 0.05;

/// Azimuthal-mode truncation ceiling `M_max`. The runtime count is sized adaptively from
/// the coma strength AND the observation angle by [`mode_count_for`] (only modes with
/// `m ≲ k·R·sinθ` survive the `Jₘ(kρsinθ)` kernel), then clamped here; the `M`-vs-`M+1`
/// self-check (D-6) flags any residual under-resolution. Kept strictly below
/// `MODE_PHI_MAX/2 − 1` so even the `M+1` probe mode stays above the φ'-Nyquist of the
/// largest `n_phi`.
const MODE_M_MAX: u32 = 254;

// The azimuthal DFT that builds g_m(ρ) needs > 2·M samples in φ' or the top modes alias
// (Nyquist). The self-check probes one extra mode (M+1); `mode_count_for` additionally
// clamps the runtime M to `n_phi/2 − 2`, but this guard pins the constant ceilings so a
// future bump cannot silently break the invariant even at the maximum n_phi.
const _: () = assert!(MODE_PHI_MAX > 2 * (MODE_M_MAX as usize + 1));

/// Absolute safety ceiling on the radial sample count handed to the Hankel / mode
/// integrator (P10 Task 3, D-4). The working density is derived from `(D/λ, θ)` by
/// [`radial_points_for`]; this only bounds pathological requests (e.g. a 300 m dish at
/// Q-band, θ=90°) so a single evaluation cannot allocate unbounded work. The runtime
/// convergence self-check recomputes at `2·N`, so the hard allocation limit is `2×` this.
/// Chosen comfortably above the largest enabled antenna's need — `gbt_100m` q-band at
/// θ=90° lands near `4·(D/λ) ≈ 5.7·10⁴` (`radial_points_for_gbt_qband_is_tens_of_thousands`),
/// whose `2·N` self-check leg (~1.1·10⁵) stays under `2×` this cap so it still converges.
const RADIAL_POINTS_SAFETY_MAX: usize = 65_537; // 2^16 + 1 (odd)

/// Performance ceiling (in radial cycles) on the θ-independent aperture-plane coma
/// contribution to [`radial_points_for`], applied ONLY in the large-steering regime
/// (`δ/f >` [`MODE_STEERING_RATIO`]). Physical offset feeds resolve their coma fully (up
/// to the physical `D/(2λ)` ceiling); this bounds only strongly-steered feeds, keeping a
/// dense off-axis sweep (or a metres-steered ray-tracing evaluation) within budget. See
/// the call site.
const MODE_RADIAL_CYCLE_CAP: f64 = 8.0;

/// Complex integration result
///
/// The aperture integration produces a complex-valued field in the far zone.
/// Both real and imaginary parts are needed for phase information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntegrationResult {
    /// Complex electric field value
    pub field: Complex64,

    /// Estimated integration error (magnitude).
    ///
    /// For the P10 Hankel / azimuthal-mode integrator this is the magnitude of the
    /// runtime convergence self-check difference (D-6): `|I(2N) − I(N)|` for the
    /// symmetric radial path, or `|I(M+1) − I(M)|` (the top-mode contribution) for the
    /// asymmetric mode path. It is always finite and non-negative; a value larger than
    /// `relative_tolerance · |field|` is what sets `converged = false`.
    pub error_estimate: f64,

    /// Number of function evaluations performed
    pub num_evaluations: usize,

    /// Whether the integration passed its runtime convergence self-check (D-6).
    ///
    /// `true`  – the coarse/fine estimates agree: `error_estimate ≤
    ///           relative_tolerance · |field|` (or below `absolute_tolerance`). The
    ///           returned `field` is the finer estimate (`I(2N)` / `I(M+1)`).
    /// `false` – the estimates disagree by more than tolerance (radial density hit the
    ///           safety cap below Nyquist, or the mode count was insufficient). The
    ///           returned `field` is still the best (finer) estimate and `error_estimate`
    ///           holds the honest coarse/fine difference — the value is NEVER silently
    ///           returned as converged. `compute_gain_standard` surfaces this as the
    ///           `INTEGRATION_NONCONVERGENCE_WARNING`.
    pub converged: bool,
}

/// Integration parameters for convergence control
#[derive(Debug, Clone)]
pub struct IntegrationParams {
    /// Minimum number of radial integration points
    pub min_rho_points: usize,

    /// Maximum number of radial integration points (for adaptive refinement)
    pub max_rho_points: usize,

    /// Minimum number of azimuthal integration points
    pub min_phi_points: usize,

    /// Maximum number of azimuthal integration points
    pub max_phi_points: usize,

    /// Relative error tolerance for adaptive refinement
    pub relative_tolerance: f64,

    /// Absolute error tolerance
    pub absolute_tolerance: f64,

    /// Maximum number of refinement iterations
    pub max_iterations: usize,

    /// Include higher-order Seidel aberrations in phase computation
    ///
    /// When true, adds astigmatism, field curvature, and distortion terms
    /// for feeds with moderate offsets (0.3f - 0.5f).
    pub use_higher_order_aberrations: bool,

    /// Fold physical feed-spillover efficiency into the returned gain.
    ///
    /// Decided by the SERVICE layer (set only for antennas with no correction
    /// surface — the surface otherwise absorbs spillover empirically). The model
    /// itself never inspects calibration; it only reads this bool.
    pub apply_spillover: bool,

    /// Apply the Ruze scattered-power sidelobe floor (F7): `gain = max(pattern, floor)`.
    ///
    /// Off by default everywhere in this module — enabling it is a SERVICE-layer
    /// decision (a later task wires it in for uncalibrated antennas only). See
    /// `pattern::sidelobe_floor_gain` for the physical model.
    pub apply_sidelobe_floor: bool,
}

impl Default for IntegrationParams {
    fn default() -> Self {
        Self {
            min_rho_points: 32,       // Minimum for radial direction
            max_rho_points: 128,      // Maximum for adaptive refinement
            min_phi_points: 64,       // Azimuthal (full 2π circle)
            max_phi_points: 256,      // Maximum azimuthal points
            relative_tolerance: 1e-4, // 0.01% relative error
            absolute_tolerance: 1e-8, // Absolute error floor
            max_iterations: 5,        // Refinement iteration limit
            use_higher_order_aberrations: false,
            apply_spillover: false,
            apply_sidelobe_floor: false,
        }
    }
}

impl IntegrationParams {
    /// Canonical parameters for the SERVED (production) path.
    ///
    /// This is the single constructor the service layer should use (see
    /// `service::evaluator` and `service::h3_link_budget`). Since the P10
    /// off-axis integrator landed, the number of radial samples is derived
    /// ADAPTIVELY from `(D/λ, θ)` by `radial_points_for` — roughly
    /// `N_ρ ≈ 4·(D/λ)·sinθ` — so the physical correctness of the off-axis
    /// pattern no longer depends on this preset's magnitude. In this new
    /// regime the `min_rho_points`/`max_rho_points` fields are just:
    ///   * `min_rho_points` — a DENSITY FLOOR (cheap near-boresight cases), and
    ///   * `max_rho_points` — a safety knob / fallback size for the
    ///     `#[cfg(test)]`-only fixed-density 2D Simpson path.
    ///
    /// They no longer gate the served pattern's correctness.
    ///
    /// INERT on the served path: `min_phi_points`, `max_phi_points`, and `max_iterations`
    /// are NOT read by either production integrator. The served φ' sample count comes from
    /// `mode_count_for` (not `min/max_phi_points`), the radial density from
    /// `radial_points_for`, and there is no adaptive refinement loop (the runtime
    /// convergence check is a single N-vs-2N / M-vs-(M+1) comparison, not an iteration
    /// count). These three fields survive only for the `#[cfg(test)]`-only 2D reference
    /// (`integrate_2d_adaptive` / `integrate_2d_simpson_public_shim`) and for struct
    /// compatibility with the other presets — tuning them here does nothing to a served
    /// evaluation.
    pub fn adaptive() -> Self {
        Self {
            min_rho_points: 16,
            max_rho_points: 64,
            min_phi_points: 32,
            max_phi_points: 128,
            relative_tolerance: 1e-3,
            absolute_tolerance: 1e-7,
            max_iterations: 3,
            use_higher_order_aberrations: false,
            apply_spillover: false,
            apply_sidelobe_floor: false,
        }
    }

    /// Create fast integration parameters (lower accuracy, faster).
    ///
    /// NOTE: since the P10 adaptive off-axis integrator landed, this preset no
    /// longer gates production correctness — the served radial density is
    /// derived adaptively from `(D/λ, θ)` regardless of these values (see
    /// [`IntegrationParams::adaptive`]). Retained for the many tests that
    /// construct it directly; prefer `adaptive()` for the served path.
    pub fn fast() -> Self {
        Self {
            min_rho_points: 16,
            max_rho_points: 64,
            min_phi_points: 32,
            max_phi_points: 128,
            relative_tolerance: 1e-3,
            absolute_tolerance: 1e-7,
            max_iterations: 3,
            use_higher_order_aberrations: false,
            apply_spillover: false,
            apply_sidelobe_floor: false,
        }
    }

    /// Create high-accuracy integration parameters (slower, more accurate).
    ///
    /// As with [`IntegrationParams::fast`], since the P10 adaptive integrator
    /// landed this preset no longer gates production correctness (the served
    /// radial density is adaptive — see [`IntegrationParams::adaptive`]). Kept
    /// for tests that need a high-density floor.
    pub fn high_accuracy() -> Self {
        Self {
            min_rho_points: 64,
            max_rho_points: 256,
            min_phi_points: 128,
            max_phi_points: 512,
            relative_tolerance: 1e-6,
            absolute_tolerance: 1e-10,
            max_iterations: 8,
            use_higher_order_aberrations: false,
            apply_spillover: false,
            apply_sidelobe_floor: false,
        }
    }

    /// Enable higher-order aberrations for moderate feed offsets
    pub fn with_higher_order_aberrations(mut self) -> Self {
        self.use_higher_order_aberrations = true;
        self
    }

    /// Create adaptive integration parameters with doubled sampling density
    ///
    /// Used near pattern nulls where rapid phase changes require finer sampling
    /// to maintain numerical accuracy.
    pub fn with_adaptive_refinement(&self) -> Self {
        Self {
            min_rho_points: self.min_rho_points * 2,
            max_rho_points: self.max_rho_points * 2,
            min_phi_points: self.min_phi_points * 2,
            max_phi_points: self.max_phi_points * 2,
            relative_tolerance: self.relative_tolerance / 2.0, // Tighter tolerance
            ..self.clone()
        }
    }
}

/// Integrate aperture field to compute far-field pattern
///
/// Performs 2D numerical integration over the reflector aperture using
/// composite Simpson's rule with adaptive refinement.
///
/// # Arguments
/// - `theta`: Polar angle in far field (radians, from boresight)
/// - `phi`: Azimuthal angle in far field (radians)
/// - `config`: Antenna configuration (geometry, feed, mesh)
/// - `frequency_hz`: Operating frequency in Hz
/// - `params`: Integration parameters (convergence tolerances, grid sizes)
///
/// # Returns
/// `IntegrationResult` containing complex field value, error estimate, and evaluation count
///
/// # Errors
/// Returns `ComputationError` if:
/// - Integration fails to converge within max iterations
/// - Invalid antenna configuration
///
/// # Examples
/// ```
/// use antenna_model::model::integration::{integrate_aperture, IntegrationParams};
/// use antenna_model::model::geometry::{AntennaConfiguration, ReflectorGeometry, FeedParameters};
///
/// // Example integration at boresight (θ=0)
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let reflector = ReflectorGeometry::builder()
/// #     .diameter(1.0)
/// #     .focal_length(0.5)
/// #     .surface_rms(0.001)
/// #     .build()?;
/// # let feed = FeedParameters::builder()
/// #     .at_focus(0.5)
/// #     .q_factor(8.0)
/// #     .build()?;
/// # let config = AntennaConfiguration::builder()
/// #     .id("test")
/// #     .name("Test")
/// #     .reflector(reflector)
/// #     .feed(feed)
/// #     .build()?;
/// let result = integrate_aperture(
///     0.0,               // theta (boresight)
///     0.0,               // phi
///     &config,
///     8.4e9,             // 8.4 GHz
///     &IntegrationParams::default(),
/// )?;
///
/// println!("Field magnitude: {}", result.field.norm());
/// println!("Error estimate: {}", result.error_estimate);
/// # Ok(())
/// # }
/// ```
pub fn integrate_aperture(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<IntegrationResult> {
    // Validate inputs
    if !theta.is_finite() || !phi.is_finite() || !frequency_hz.is_finite() {
        return Err(ComputationError::NumericalInstability {
            operation: "integrate_aperture".to_string(),
            reason: "Angles and frequency must be finite".to_string(),
        });
    }

    if frequency_hz <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "integrate_aperture".to_string(),
            reason: format!("Frequency must be positive, got {}", frequency_hz),
        });
    }

    // Calculate wavelength and wavenumber
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);

    // P10 Task 1: azimuthally symmetric apertures (no lateral feed offset and no
    // higher-order aberration terms) reduce EXACTLY to a 1D radial Hankel (J₀)
    // transform. Unlike the retired 2D quadrature, this does NOT alias off-axis for
    // electrically large dishes (the P0 bug). The asymmetric / coma case uses the Jₘ
    // azimuthal-mode expansion below.
    let is_symmetric = config.feed.position.radial_displacement() == 0.0
        // Azimuthally-symmetric illumination only: a non-unity asymmetry_factor makes
        // illumination_amplitude φ'-dependent (elliptical beam), which breaks the J₀
        // collapse (it assumes A has no φ' dependence). Such feeds take the mode path.
        && config.feed.asymmetry_factor == 1.0
        && !params.use_higher_order_aberrations;
    if is_symmetric {
        // Adaptive radial density from (D/λ, θ) at ~2× Nyquist (Task 3, D-6), with a
        // runtime N-vs-2N self-check: recompute at 2N and compare. Agreement within
        // tolerance ⇒ converged; disagreement (density hit the safety cap below Nyquist)
        // ⇒ converged=false with an honest error estimate — never silently returned.
        let n1 = radial_points_for(config, theta, wavelength, params);
        let n2 = radial_check_points(n1);
        let f1 = hankel_radial_field(config, theta, phi, k, n1);
        let f2 = hankel_radial_field(config, theta, phi, k, n2);
        let (field, error_estimate, converged) = self_check(f1, f2, params, HANKEL_SELF_CHECK_RTOL);
        return Ok(IntegrationResult {
            field,
            error_estimate,
            num_evaluations: n1 + n2,
            converged,
        });
    }

    // P10 Task 2/3: asymmetric aperture — a lateral feed offset (coma), an azimuthally
    // dependent illumination (`asymmetry_factor != 1.0`), and/or higher-order Seidel
    // terms. Route through the azimuthal-mode (Jₘ) expansion — the general, non-aliasing
    // closed form (the symmetric Hankel path above is its m=0-only special case).
    //
    // Adaptive sizing (Task 3, D-6):
    //   `n_rho`          — from (D/λ, θ) at ~2× Nyquist, shared with the symmetric path.
    //   `n_phi`          — φ' samples for the `g_m(ρ)` Fourier coefficients (adaptive).
    //   `m_max`          — mode truncation from the coma strength `k·δ·(R/f)`.
    // Runtime M-vs-(M+1) self-check: `azimuthal_mode_field_inner` returns both the full
    // sum (modes 0..=M+1) and the contribution of the top probe mode (M+1). If that top
    // mode contributes more than the relative tolerance, the truncation is insufficient
    // ⇒ converged=false with an honest error estimate.
    let n_rho = radial_points_for(config, theta, wavelength, params);
    let (m_max, n_phi) = mode_count_for(config, wavelength, theta);
    // Probe one extra mode (M+1) so the self-check can measure its contribution in a
    // single φ' sweep. `mode_count_for` kept m_max ≤ n_phi/2 − 2, so the probe is alias-free.
    let m_probe = m_max + 1;
    let (field, top_contrib) = azimuthal_mode_field_inner(
        config,
        theta,
        phi,
        k,
        n_rho,
        n_phi,
        m_probe,
        params.use_higher_order_aberrations,
    );
    // I(M) = I(M+1) − (top-mode contribution); the self-check compares I(M) vs I(M+1).
    let f_m = field - top_contrib;
    let (field, error_estimate, converged) = self_check(f_m, field, params, MODE_SELF_CHECK_RTOL);
    Ok(IntegrationResult {
        field,
        error_estimate,
        num_evaluations: n_rho * n_phi,
        converged,
    })
}

/// Radial sample count for the self-check's fine (2N) leg: ~double `n1`, kept odd, and
/// bounded by the absolute allocation ceiling. Staying above `n1` is what makes the
/// N-vs-2N comparison meaningful; when `n1` was already clamped to the safety cap below
/// Nyquist, this finer leg exposes the disagreement so it is flagged, not hidden.
#[inline]
fn radial_check_points(n1: usize) -> usize {
    let n2 = (2 * n1)
        .saturating_sub(1)
        .min(2 * RADIAL_POINTS_SAFETY_MAX + 1);
    if n2.is_multiple_of(2) {
        n2 + 1
    } else {
        n2
    }
}

/// Relative-tolerance FLOOR for the Hankel / mode convergence self-check (D-6).
///
/// The self-check compares the field at `N` and `2N` (radial) or `M` and `M+1` (modes).
/// At the adaptive ~2× Nyquist radial density (4 samples/cycle) a *converged* Simpson
/// integral still shows an `N`-vs-`2N` field difference of a few tenths of a percent —
/// far above the `1e-4`..`1e-3` `relative_tolerance` the retired 2D adaptive loop used,
/// which would spuriously flag physically-accurate results (e.g. gbt_100m q-band at
/// θ=90°: 0.6 % ≈ 0.05 dB, well inside the < 0.1 dB accuracy budget).
///
/// This floor sets the gate to the accuracy budget instead: a 2 % `N`-vs-`2N` field
/// difference bounds the *returned* (finer, `2N`) estimate's own error to ≈ diff/15
/// (Richardson, Simpson is O(h⁴)) ⇒ < 0.15 % ⇒ < 0.013 dB. It stays far below the O(1)
/// (~100 %) mismatch that genuine under-resolution (density capped below Nyquist, or too
/// few modes) produces, so real non-convergence is still caught. The effective tolerance
/// is `max(params.relative_tolerance, this)` — a caller may loosen further but never
/// tighten below the physically-meaningful floor.
const HANKEL_SELF_CHECK_RTOL: f64 = 2.0e-2;

/// Relative-tolerance FLOOR for the azimuthal-mode TRUNCATION self-check (D-6), used ONLY
/// on the Jₘ mode path — deliberately tighter than [`HANKEL_SELF_CHECK_RTOL`].
///
/// The 2 % radial floor is justified by Richardson extrapolation: for a *converged* O(h⁴)
/// Simpson integral the returned `2N` estimate's own error is ≈ `diff/15`, so a 2 %
/// `N`-vs-`2N` field difference bounds the returned error to < 0.013 dB. That `diff/15`
/// benefit does NOT exist for a mode-TRUNCATION tail. There the self-check `diff` is just
/// the single `M+1` mode's contribution `|I(M+1) − I(M)|`, whereas the returned field's
/// actual error is the ENTIRE unmeasured tail `|Σ_{m≥M+2}|`. For a slowly-decaying
/// azimuthal spectrum that tail can be comparable to `diff`, so a 2 % `M+1` diff could hide
/// up to ≈ 0.17 dB of truncation error — above the documented < 0.1 dB budget.
///
/// Tail model (conservative): assume the modes beyond `M+1` form a geometric tail with
/// ratio ≤ 0.5, so `Σ_{m≥M+2} ≤ term(M+1) = diff` — i.e. the returned error is at most one
/// more `diff`. Gating `diff ≤ 0.5 %` then keeps the returned amplitude error ≤ 0.5 %
/// ≈ 0.043 dB, comfortably inside the < 0.1 dB budget (and still under budget even for a
/// somewhat slower tail: ratio ≈ 0.7 gives a tail ≈ 2.3·diff ≈ 1.16 % ≈ 0.1 dB, the edge).
/// The `+6` mode margin in [`mode_count_for`] pushes the `M+1` probe well into the
/// negligible tail for every real (physically-offset / asymmetric-illumination) case, so
/// they stay `converged=true` despite the tighter gate. Effective tolerance is
/// `max(params.relative_tolerance, this)` — same loosen-not-tighten floor semantics.
const MODE_SELF_CHECK_RTOL: f64 = 5.0e-3;

/// Runtime convergence verdict (D-6): compare a coarse and a fine field estimate and
/// decide whether the integrator converged. Returns `(field, error_estimate, converged)`
/// where `field` is ALWAYS the finer estimate (`fine`) and `error_estimate` is the
/// finite, non-negative coarse/fine magnitude difference. `converged` is true iff that
/// difference is within the effective relative tolerance times `|fine|`, or below
/// `absolute_tolerance`.
///
/// `rtol_floor` is the physically-justified relative-tolerance floor for THIS check: the
/// radial N-vs-2N path passes [`HANKEL_SELF_CHECK_RTOL`] (Richardson `diff/15` benefit),
/// while the mode M-vs-(M+1) truncation path passes the tighter [`MODE_SELF_CHECK_RTOL`]
/// (no Richardson benefit for a truncation tail — see that constant's docstring). The
/// effective tolerance is `max(params.relative_tolerance, rtol_floor)`, so a caller may
/// loosen but never tighten below the floor.
#[inline]
fn self_check(
    coarse: Complex64,
    fine: Complex64,
    params: &IntegrationParams,
    rtol_floor: f64,
) -> (Complex64, f64, bool) {
    let diff = (fine - coarse).norm();
    let magnitude = fine.norm();
    let rtol = params.relative_tolerance.max(rtol_floor);
    let converged =
        diff <= rtol * magnitude.max(params.absolute_tolerance) || diff < params.absolute_tolerance;
    (fine, diff, converged)
}

/// Perform 2D integration using composite Simpson's rule
///
/// Integrates over rectangular domain [rho_min, rho_max] × [phi_min, phi_max]
/// using nested 1D Simpson's rule.
///
/// Returns (integrated_value, num_evaluations)
///
/// Retained as a test-only near-in reference (the small-dish regime where the 2D
/// quadrature is trustworthy) and as the trusted oracle for the azimuthal-mode
/// integrator. Since P10 Task 2 it is no longer on any production code path — the
/// production off-axis integral goes through `hankel_radial_field` (symmetric) or
/// `azimuthal_mode_field` (asymmetric), which do not alias.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn integrate_2d_simpson(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    k: f64,
    wavelength: f64,
    rho_min: f64,
    rho_max: f64,
    phi_min: f64,
    phi_max: f64,
    n_rho: usize,
    n_phi: usize,
    use_higher_order_aberrations: bool,
) -> (Complex64, usize) {
    // Ensure odd number of points for Simpson's rule
    let n_rho = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let n_phi = if n_phi.is_multiple_of(2) {
        n_phi + 1
    } else {
        n_phi
    };

    let h_rho = (rho_max - rho_min) / (n_rho - 1) as f64;
    let h_phi = (phi_max - phi_min) / (n_phi - 1) as f64;

    let mut sum = Complex64::new(0.0, 0.0);
    let mut num_evaluations = 0;

    // Outer integral over φ' using Simpson's rule
    for j in 0..n_phi {
        let phi_prime = phi_min + j as f64 * h_phi;
        let phi_weight = simpson_weight(j, n_phi);

        // Inner integral over ρ using Simpson's rule
        let mut inner_sum = Complex64::new(0.0, 0.0);

        for i in 0..n_rho {
            let rho = rho_min + i as f64 * h_rho;
            let rho_weight = simpson_weight(i, n_rho);

            // Evaluate integrand
            let integrand_value = aperture_integrand(
                rho,
                phi_prime,
                theta,
                phi,
                config,
                k,
                wavelength,
                use_higher_order_aberrations,
            );

            num_evaluations += 1;

            // Accumulate with weights and Jacobian (ρ for polar coordinates)
            inner_sum += integrand_value * rho * rho_weight;
        }

        // Accumulate outer integral
        sum += inner_sum * phi_weight;
    }

    // Apply Simpson's rule scaling factors
    let integral = sum * h_rho * h_phi / 9.0; // 1/9 = (1/3) * (1/3) for 2D Simpson's

    (integral, num_evaluations)
}

/// Retained 2D adaptive Simpson refinement loop — the interim carrier of the
/// non-convergence sentinel (`converged=false`, `error_estimate=INFINITY`).
///
/// This is the exact loop `integrate_aperture` used before P10 Task 2. Now that the
/// production asymmetric path uses `azimuthal_mode_field`, this loop is off every
/// production code path; it is kept test-only so the two non-convergence tests
/// (`test_non_convergence_is_reported` here and `test_non_convergence_warning_propagated`
/// in `pattern.rs`) can pin the 2D mechanism directly until Task 3 reworks the runtime
/// convergence self-check into the Hankel/mode paths.
#[cfg(test)]
pub(crate) fn integrate_2d_adaptive(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> IntegrationResult {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);
    let rho_max = config.reflector.diameter / 2.0;
    let phi_min = 0.0;
    let phi_max = 2.0 * PI;

    let mut n_rho = params.min_rho_points;
    let mut n_phi = params.min_phi_points;
    let mut previous_result = Complex64::new(0.0, 0.0);
    let mut num_evaluations = 0;
    let mut last_difference = f64::INFINITY;

    for iteration in 0..params.max_iterations {
        let (result, evals) = integrate_2d_simpson(
            theta,
            phi,
            config,
            k,
            wavelength,
            0.0,
            rho_max,
            phi_min,
            phi_max,
            n_rho,
            n_phi,
            params.use_higher_order_aberrations,
        );
        num_evaluations += evals;

        if iteration > 0 {
            let difference = (result - previous_result).norm();
            let magnitude = result.norm();
            last_difference = difference;
            let relative_error = if magnitude > params.absolute_tolerance {
                difference / magnitude
            } else {
                difference
            };
            if relative_error < params.relative_tolerance || difference < params.absolute_tolerance
            {
                return IntegrationResult {
                    field: result,
                    error_estimate: difference,
                    num_evaluations,
                    converged: true,
                };
            }
        }

        previous_result = result;
        n_rho = (n_rho * 3 / 2).min(params.max_rho_points);
        n_phi = (n_phi * 3 / 2).min(params.max_phi_points);
        if n_rho >= params.max_rho_points && n_phi >= params.max_phi_points {
            break;
        }
    }

    IntegrationResult {
        field: previous_result,
        error_estimate: last_difference,
        num_evaluations,
        converged: false,
    }
}

/// Single fixed-density 2D Simpson evaluation at `params.max_rho_points ×
/// params.max_phi_points` — the converged near-in reference used by the azimuthal-mode
/// cross-validation test on small dishes (where the 2D quadrature is trustworthy).
#[cfg(test)]
fn integrate_2d_simpson_public_shim(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> Complex64 {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);
    let rho_max = config.reflector.diameter / 2.0;
    let (field, _) = integrate_2d_simpson(
        theta,
        phi,
        config,
        k,
        wavelength,
        0.0,
        rho_max,
        0.0,
        2.0 * PI,
        params.max_rho_points,
        params.max_phi_points,
        params.use_higher_order_aberrations,
    );
    field
}

/// Adaptive radial sample count for the Hankel / mode integrator at ~2× Nyquist
/// (P10 Task 3, D-6).
///
/// The chirp and the `Jₘ(kρ·sinθ)` kernel oscillate at radial rate `≈ (D/λ)·sinθ`
/// cycles across `[0, R]`, so the Nyquist count is `N ≈ 2·(D/λ)·sinθ`. We take ~2× that
/// (`N ≈ 4·(D/λ)·sinθ`), floored at `params.min_rho_points` and capped at
/// [`RADIAL_POINTS_SAFETY_MAX`], forced odd for Simpson's rule.
///
/// The count sums the radial oscillation of EVERY integrand phase term, not just the
/// θ-dependent kernel — critically the θ-INDEPENDENT aperture-plane phase (lateral coma,
/// axial defocus), which oscillates radially even at θ=0. Missing it silently aliases a
/// steered/offset feed at boresight (the P0 signature — off-axis gain far too high). Each
/// term's cycle count across `[0, R]`:
/// - chirp + `Jₘ` kernel (θ-dependent): `(D/λ)·|sinθ|`
/// - lateral coma (θ-independent):      `(δ/λ)·(R/f)`, capped at the physical maximum
///   radial spatial frequency `R/λ = D/(2λ)` (a purely-radial aperture phase gradient can
///   never exceed `k`, so the linear-steer estimate is clamped there for large `δ/f`)
/// - axial defocus (θ-independent):     `(|axial|/λ)·(R/f)²`, capped the same way
///
/// At θ=0 with a centered feed all terms vanish and the count drops to
/// `params.min_rho_points` — the cheap near-boresight case (the P10 throughput fix). It
/// deliberately does NOT read `params.max_rho_points`: that preset knob sizes the
/// retained test-only 2D reference, whereas the production density is derived here from
/// the physics (D-4). Forced odd for Simpson; capped at [`RADIAL_POINTS_SAFETY_MAX`].
fn radial_points_for(
    config: &AntennaConfiguration,
    theta: f64,
    wavelength: f64,
    params: &IntegrationParams,
) -> usize {
    let d_lambda = config.reflector.diameter / wavelength;
    let r = config.reflector.diameter / 2.0;
    let f = config.reflector.focal_length;
    let r_over_f = r / f;
    // Physical ceiling on any single aperture-plane term's radial cycles: a radial phase
    // gradient cannot exceed k, i.e. R/λ = D/(2λ) cycles across [0, R].
    let radial_cycle_ceiling = 0.5 * d_lambda;
    let delta = config.feed.position.radial_displacement();
    let axial = (config.feed.position.z - f + config.feed.axial_defocus).abs();

    // In the large-steering regime (δ/f above the threshold — a strongly steered or
    // metres-steered feed) the coma radial oscillation is capped for performance:
    // resolving it exactly would need thousands of radial × azimuthal samples per point
    // and blow the latency budget, and the exact off-axis level is not PO-trustworthy
    // there anyway. Physical offset feeds (δ/f ≪ threshold, every served feed) keep the
    // full physical resolution up to the D/(2λ) ceiling.
    let coma_cap = if delta / f > MODE_STEERING_RATIO {
        MODE_RADIAL_CYCLE_CAP.min(radial_cycle_ceiling)
    } else {
        radial_cycle_ceiling
    };

    let kernel_cycles = d_lambda * theta.sin().abs();
    let coma_cycles = ((delta / wavelength) * r_over_f).min(coma_cap);
    let defocus_cycles = ((axial / wavelength) * r_over_f * r_over_f).min(radial_cycle_ceiling);
    let cycles = kernel_cycles + coma_cycles + defocus_cycles;

    // ~2× Nyquist: 4 samples per cycle.
    let target = 4.0 * cycles;
    // Guard against a non-finite target (e.g. wavelength underflow) — fall back to floor.
    let target = if target.is_finite() {
        target.ceil() as usize
    } else {
        params.min_rho_points
    };
    let n = target
        .max(params.min_rho_points)
        .min(RADIAL_POINTS_SAFETY_MAX);
    if n.is_multiple_of(2) {
        n + 1
    } else {
        n
    }
}

/// Adaptive azimuthal sizing `(m_max, n_phi)` for the coma / asymmetric path (P10 Task 3).
///
/// Two distinct quantities are derived here:
///
/// - **`n_phi`** (φ' DFT sample count) must resolve the azimuthal spectrum of the INPUT
///   aperture-plane function `g(ρ,φ')`, whose maximum significant mode `B` is the wider of
///   two drivers (physically capped at `k·R`, the aperture's k-space radius — a
///   purely-azimuthal phase gradient cannot exceed `k`): the coma spread
///   `spread = k·δ·(R/f)` from a lateral feed offset, OR an illumination floor of `6` when
///   `asymmetry_factor != 1.0` (an elliptical feed modulates the effective q-factor by
///   `cos(2φ')`, so `g` carries m=±2 plus weaker ±4, ±6 harmonics even for a CENTERED feed,
///   δ=0). We take `n_phi ≈ 2·B` (rounded up to a power of two), floored/capped at
///   [`MODE_PHI_MIN`] / [`MODE_PHI_MAX`]. Under-sizing `n_phi` aliases high input modes into
///   `g_0` and is the exact defect that made a heavily-steered feed read far too high
///   off-axis. Only the pure-symmetric, pure-axial-defocus case (`asymmetry_factor==1.0`
///   AND no lateral coma) has no azimuthal content and takes the cheap `(1, MODE_PHI_MIN)`
///   fast path.
///
/// - **`m_max`** (modes actually summed) need only include modes that survive the
///   `Jₘ(kρ·sinθ)` kernel: `Jₘ(x) ≈ 0` for `m ≳ x`, so at observation angle θ only
///   `m ≲ k·R·sinθ` contribute. `m_max = min(1.5·B + 6, k·R·|sinθ| + 6)`, clamped to
///   [`MODE_M_MAX`] and to `n_phi/2 − 2` (so the `M+1` self-check probe stays alias-free).
///   At θ=0 only `m=0` survives (`Jₘ(0)=0, m>0`), so `m_max` collapses to the margin and
///   the sum is cheap even when `n_phi` is large.
///
/// Validated: `spread ≈ 8.8` needs `M ≈ 16`; `dsn_34m` X-band `spread ≈ 33` needs
/// `M ≈ 46`; a feed steered a full aperture radius off-axis needs `n_phi ≈ 512`.
fn mode_count_for(config: &AntennaConfiguration, wavelength: f64, theta: f64) -> (u32, usize) {
    let k = wavenumber(wavelength);
    let r = config.reflector.diameter / 2.0;
    let delta = config.feed.position.radial_displacement();
    let r_over_f = r / config.reflector.focal_length;
    let k_r = k * r; // physical azimuthal-bandwidth ceiling
    let spread = k * delta * r_over_f;

    // Coma-driven azimuthal bandwidth of g(ρ,φ') from a lateral feed offset, physically
    // capped at k·R. Zero for a centered feed (δ=0) or a non-finite spread.
    let coma_bandwidth = if spread.is_finite() && spread > 0.0 {
        spread.min(k_r)
    } else {
        0.0
    };

    // Illumination-driven azimuthal bandwidth. When `asymmetry_factor != 1.0`,
    // `illumination_amplitude` modulates the effective q-factor by `cos(2φ')`, so the
    // aperture function g(ρ,φ') carries a genuine m=±2 fundamental PLUS weaker m=±4, ±6
    // harmonics from the nonlinear `cos_q_pattern(ψ, q(φ'))` dependence — even for a
    // CENTERED feed (δ=0, coma_bandwidth=0). A bandwidth floor of 6 ensures those
    // harmonics are resolved; without it `m_max=1` under-resolves the m=2 content AND the
    // M-vs-(M+1) self-check would see the large m=2 jump and spuriously flag non-convergence.
    let asym_bandwidth = if config.feed.asymmetry_factor != 1.0 {
        6.0
    } else {
        0.0
    };

    // Combine the two drivers (take the wider). Pure-symmetric, pure-axial-defocus feeds
    // (asymmetry_factor==1.0 AND no lateral coma) genuinely have no azimuthal content, so
    // they keep the cheap (1, MODE_PHI_MIN) fast path.
    let bandwidth = coma_bandwidth.max(asym_bandwidth);
    if bandwidth <= 0.0 {
        return (1, MODE_PHI_MIN);
    }

    // In the ray-tracing regime (δ/f ≫ 1, D-5 stub) cap n_phi low for performance; every
    // physical offset feed (δ/f ≪ 1) keeps the full MODE_PHI_MAX so its coma spectrum
    // resolves exactly. (dsn_34m Ka-band, the widest physical spectrum at ~125 modes,
    // needs n_phi ≈ 512 = MODE_PHI_MAX.)
    let n_phi_cap = if delta / config.reflector.focal_length > MODE_STEERING_RATIO {
        MODE_PHI_STEERED_MAX
    } else {
        MODE_PHI_MAX
    };

    // n_phi ≈ 2·B, rounded to a power of two, floored/capped.
    let n_phi_target = (2.0 * bandwidth).ceil() as usize + 8;
    let n_phi = n_phi_target
        .next_power_of_two()
        .clamp(MODE_PHI_MIN, n_phi_cap);

    // Modes that survive the Jₘ(kρ·sinθ) kernel at this θ.
    let m_theta = (k_r * theta.sin().abs()).ceil() + 6.0;
    let m_spectrum = (1.5 * bandwidth).ceil() + 6.0;
    let m_cap = (n_phi / 2).saturating_sub(2) as f64;
    let m_max = m_spectrum
        .min(m_theta)
        .min(m_cap)
        .min(MODE_M_MAX as f64)
        .max(1.0) as u32;
    (m_max, n_phi)
}

/// Symmetric-aperture (no lateral feed offset) Hankel radial field.
///
/// For an azimuthally symmetric aperture the closed-form φ' integral (Jacobi–Anger)
/// collapses the 2D aperture integral to the 1D radial transform
/// ```text
/// I(θ) = 2π ∫₀^R exp(j·k·ρ²/(4f)·(1−cosθ)) · A(ρ) · exp(j·Ψ_ρonly) · J₀(kρ sinθ) · ρ dρ
/// ```
/// where `Ψ_ρonly` is the ρ-only (azimuthally symmetric) phase: axial-defocus (feed
/// z-offset + deliberate `axial_defocus`, folded in via the exact geometric
/// `phase_feed_displacement` with zero lateral offset) plus mesh phase. Evaluated by
/// composite Simpson's rule over ρ with `n_rho` (forced odd) points.
///
/// At θ=0: `sinθ=0 ⇒ J₀(0)=1` and the chirp vanishes, so the integral reduces to
/// `2π ∫ A(ρ)·exp(j·Ψ_ρonly)·ρ dρ` — identical to the 2D path on-axis.
fn hankel_radial_field(
    config: &AntennaConfiguration,
    theta: f64,
    _phi: f64,
    k: f64,
    n_rho: usize,
) -> Complex64 {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    let mesh_spacing = config.mesh.as_ref().map_or(0.0, |m| m.spacing);
    // Axial defocus (feed z-offset + deliberate axial_defocus) adds a ρ-only quadratic
    // phase that is azimuthally symmetric — fold it into the phase. Lateral offset is
    // excluded here by the caller (symmetric path only).
    let axial = config.feed.position.z - f + config.feed.axial_defocus;

    let n = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let h = r_max / (n - 1) as f64;
    let sin_theta = theta.sin();
    let one_minus_cos = 1.0 - theta.cos();

    let mut sum = Complex64::new(0.0, 0.0);
    for i in 0..n {
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        let amp = illumination_amplitude(rho, 0.0, &config.feed, f);
        // Dish-depth chirp (ρ-only, θ-dependent — the parabola's equiphase term).
        // NOTE: must stay in sync with phase_path's term1 in phase.rs — it is
        // duplicated from there because phase_path returns term1−term2 fused and
        // only term1 (this ρ²/(4f)·(1−cosθ) chirp) is wanted here.
        let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
        // Axial defocus: exact geometric ρ-only phase (φ'-independent when lateral=0).
        let defocus = if axial != 0.0 {
            crate::model::phase::phase_feed_displacement(rho, 0.0, 0.0, 0.0, axial, f, k)
        } else {
            0.0
        };
        // Mesh phase (ρ-only, via the surface incidence angle θ_inc ≈ ρ/(2f)).
        let mesh = if mesh_spacing > 0.0 {
            let theta_inc = rho / (2.0 * f);
            crate::model::phase::phase_mesh(mesh_spacing, theta_inc, k)
        } else {
            0.0
        };
        let j0 = bessel_j0(k * rho * sin_theta);
        let phase = chirp + defocus + mesh;
        sum += Complex64::new(0.0, phase).exp() * amp * j0 * rho * w;
    }
    sum * (h / 3.0) * 2.0 * PI
}

/// Config-derived, ρ/φ'-independent constants for the aperture-plane function `g(ρ,φ')`.
///
/// Computed ONCE per [`azimuthal_mode_field_inner`] call and shared across every
/// `(ρ, φ')` evaluation (`n_rho · n_phi_coeff` of them). Hoisting these out of the hot
/// path removes the per-evaluation `radial_displacement()` (a `hypot`), `atan2`, and
/// axial-offset arithmetic — hundreds of thousands of transcendental calls per gain on
/// the large offset-feed dishes.
struct AperturePlaneConst<'a> {
    feed: &'a crate::model::geometry::FeedParameters,
    /// Focal length (m).
    f: f64,
    /// Lateral feed offset magnitude `δ` (m); coma driver.
    delta: f64,
    /// Azimuth of the lateral offset, `atan2(y, x)` (rad).
    alpha: f64,
    /// Axial phase-center offset from focus (m); defocus driver.
    axial: f64,
    /// Mesh wire spacing (m); `0.0` if no mesh.
    mesh_spacing: f64,
}

impl<'a> AperturePlaneConst<'a> {
    fn new(config: &'a AntennaConfiguration) -> Self {
        let f = config.reflector.focal_length;
        Self {
            feed: &config.feed,
            f,
            delta: config.feed.position.radial_displacement(),
            alpha: config.feed.position.y.atan2(config.feed.position.x),
            axial: config.feed.position.z - f + config.feed.axial_defocus,
            mesh_spacing: config.mesh.as_ref().map_or(0.0, |m| m.spacing),
        }
    }
}

/// θ-independent aperture-plane function
/// ```text
/// g(ρ,φ') = A(ρ,φ') · exp( j·[ Ψ_feed_displacement(ρ,φ') + Ψ_higher_order(ρ,φ') + Ψ_mesh(ρ) ] )
/// ```
/// i.e. the full aperture integrand phase MINUS the parabolic dish-depth chirp
/// `k·ρ²/(4f)·(1−cosθ)` and MINUS the Fourier kernel `−k·ρ·sinθ·cos(φ−φ')` (both added,
/// respectively folded, in the radial loop of [`azimuthal_mode_field_inner`]). Neither
/// the observation angle θ nor φ enters here — this is what makes the φ'-Fourier
/// coefficients `g_m(ρ)` reusable across all θ.
///
/// The guards mirror `aperture_integrand`/`phase_total` exactly (lateral coma + axial
/// defocus via the exact geometric `phase_feed_displacement`; higher-order Seidel only
/// for a laterally displaced feed; mesh phase when a mesh with positive spacing is
/// present) so the mode integrator and the 2D reference agree wherever both are valid.
/// The config-derived constants arrive precomputed in [`AperturePlaneConst`].
#[inline]
fn aperture_plane_g(
    c: &AperturePlaneConst,
    rho: f64,
    phi_prime: f64,
    k: f64,
    use_higher_order: bool,
) -> Complex64 {
    let amp = illumination_amplitude(rho, phi_prime, c.feed, c.f);

    let mut phase = 0.0;
    if c.delta > 0.0 || c.axial != 0.0 {
        phase += crate::model::phase::phase_feed_displacement(
            rho, phi_prime, c.delta, c.alpha, c.axial, c.f, k,
        );
    }
    if use_higher_order && c.delta > 0.0 {
        phase += higher_order_aberrations(rho, phi_prime, c.delta, c.alpha, c.f, k);
    }
    // Mesh phase (ρ-only); guard on spacing > 0.0 for consistency with `phase_total`
    // and `hankel_radial_field` (a zero-spacing mesh would divide by zero in phase_mesh).
    if c.mesh_spacing > 0.0 {
        let theta_inc = rho / (2.0 * c.f);
        phase += crate::model::phase::phase_mesh(c.mesh_spacing, theta_inc, k);
    }
    Complex64::new(0.0, phase).exp() * amp
}

/// `(−j)^m` for integer `m` (which may be negative): `(−j)^m = exp(−j·m·π/2)`.
#[inline]
fn pow_neg_j(m: i32) -> Complex64 {
    Complex64::new(0.0, -(m as f64) * std::f64::consts::FRAC_PI_2).exp()
}

/// Azimuthal-mode-expansion aperture field for an asymmetric (coma / azimuthally
/// dependent) aperture:
/// ```text
/// I(θ,φ) = 2π · Σ_{m=−M}^{M} (−j)^m e^{jmφ} · R_m(θ)
/// R_m(θ)  = ∫₀^R exp(j·k·ρ²/(4f)·(1−cosθ)) · g_m(ρ) · J_m(kρ sinθ) · ρ dρ
/// g_m(ρ)  = (1/2π) ∫₀^{2π} g(ρ,φ') e^{−jmφ'} dφ'          (θ-independent)
/// ```
/// The negative modes reuse `g_{-m}(ρ)` (the `e^{+jmφ'}` coefficient) and the identity
/// `J_{-m}(a) = (−1)^m J_m(a)`, with `(−j)^{-m} = e^{+jmπ/2}`. For a real, `+x`-offset
/// feed the sum is real-symmetric (`g_{-m} = conj(g_m)`); the code does NOT assume that
/// — the served Ka-band feeds are offset along `+y`.
///
/// Radial quadrature is composite Simpson over ρ (`n_rho` forced odd); each `g_m(ρ)` is
/// a uniform-grid DFT over φ' with `n_phi_coeff` samples (trapezoid == rectangle on a
/// periodic grid), computed once per ρ and shared across modes. `J_m` is evaluated with
/// the in-house `bessel_jn`, accurate at every argument magnitude reached here.
///
/// The symmetric aperture is exactly the `M = 0` special case (only `g_0` survives,
/// `J_0`), reproducing [`hankel_radial_field`].
///
/// Thin wrapper returning only the full mode sum; see [`azimuthal_mode_field_inner`] for
/// the variant that also returns the top-mode contribution for the convergence self-check.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn azimuthal_mode_field(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    k: f64,
    n_rho: usize,
    n_phi_coeff: usize,
    m_max: u32,
    use_higher_order: bool,
) -> Complex64 {
    azimuthal_mode_field_inner(
        config,
        theta,
        phi,
        k,
        n_rho,
        n_phi_coeff,
        m_max,
        use_higher_order,
    )
    .0
}

/// Azimuthal-mode field, returning `(total, top_contribution)`:
/// - `total` = `I(θ,φ)` summed over all modes `0..=m_max` (both `±m`).
/// - `top_contribution` = the part of `total` contributed by the top mode `±m_max`.
///
/// This lets the caller obtain BOTH `I(M+1)` (`total`, calling with `m_max = M+1`) and
/// `I(M) = total − top_contribution` from a SINGLE φ' sweep, so the runtime M-vs-(M+1)
/// convergence self-check (D-6) costs no extra integration. See [`azimuthal_mode_field`].
#[allow(clippy::too_many_arguments)]
fn azimuthal_mode_field_inner(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    k: f64,
    n_rho: usize,
    n_phi_coeff: usize,
    m_max: u32,
    use_higher_order: bool,
) -> (Complex64, Complex64) {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    // Config-derived constants for g(ρ,φ'), computed once (hoisted out of the hot loop).
    let apc = AperturePlaneConst::new(config);
    let n = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let h = r_max / (n - 1) as f64;
    let dphi = 2.0 * PI / n_phi_coeff as f64;
    let sin_theta = theta.sin();
    let one_minus_cos = 1.0 - theta.cos();
    let mmax = m_max as usize;

    // Precompute the φ' twiddle factors e^{−jmφ'_j} (θ- and ρ-independent — the φ' grid
    // is fixed). This lifts n_rho·n_phi·m complex exponentials out of the radial loop into
    // a one-time n_phi·m table. e^{+jmφ'_j} is just its conjugate.
    // Flat layout: twiddle[m * n_phi_coeff + j].
    let mut twiddle = vec![Complex64::new(0.0, 0.0); (mmax + 1) * n_phi_coeff];
    for (m, chunk) in twiddle.chunks_mut(n_phi_coeff).enumerate() {
        for (jj, t) in chunk.iter_mut().enumerate() {
            *t = Complex64::new(0.0, -(m as f64) * jj as f64 * dphi).exp();
        }
    }

    // Radial accumulators for R_{+m} and R_{-m} (m = 0..=m_max); Simpson scale applied
    // once at the end. r_neg[0] is unused (m=0 has no distinct negative counterpart).
    let mut r_pos = vec![Complex64::new(0.0, 0.0); mmax + 1];
    let mut r_neg = vec![Complex64::new(0.0, 0.0); mmax + 1];

    // Per-ρ Fourier-coefficient buffers, reused each radial step to avoid reallocation.
    let mut gm_pos = vec![Complex64::new(0.0, 0.0); mmax + 1];
    let mut gm_neg = vec![Complex64::new(0.0, 0.0); mmax + 1];

    for i in 0..n {
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        // Dish-depth chirp (ρ-only, θ-dependent — the parabola's equiphase term).
        // NOTE: must stay in sync with phase_path's term1 in phase.rs — it is
        // duplicated from there because phase_path returns term1−term2 fused and
        // only term1 (this ρ²/(4f)·(1−cosθ) chirp) is wanted here.
        let chirp = k * rho * rho / (4.0 * f) * one_minus_cos;
        let chirp_factor = Complex64::new(0.0, chirp).exp();
        let a = k * rho * sin_theta;

        // g_m(ρ) via a single φ' sweep filling both +m (e^{−jmφ'}) and −m (e^{+jmφ'}).
        for g in gm_pos.iter_mut() {
            *g = Complex64::new(0.0, 0.0);
        }
        for g in gm_neg.iter_mut() {
            *g = Complex64::new(0.0, 0.0);
        }
        for jj in 0..n_phi_coeff {
            let phip = jj as f64 * dphi;
            let g = aperture_plane_g(&apc, rho, phip, k, use_higher_order);
            for m in 0..=mmax {
                let t = twiddle[m * n_phi_coeff + jj]; // e^{−jmφ'_j}
                gm_pos[m] += g * t;
                gm_neg[m] += g * t.conj(); // e^{+jmφ'_j}
            }
        }
        let norm = dphi / (2.0 * PI);
        for m in 0..=mmax {
            gm_pos[m] *= norm;
            gm_neg[m] *= norm;
        }

        // Radial integrand contribution for each mode.
        for (m, (rp, rn)) in r_pos.iter_mut().zip(r_neg.iter_mut()).enumerate() {
            let jm = bessel_jn(m as u32, a); // J_m(a); J_{-m} = (−1)^m J_m
            let base = chirp_factor * jm * rho * w;
            *rp += base * gm_pos[m];
            if m > 0 {
                let sign = if m % 2 == 0 { 1.0 } else { -1.0 };
                *rn += base * gm_neg[m] * sign;
            }
        }
    }

    let scale = h / 3.0;
    // I(θ,φ) = 2π Σ_{m=−M}^{M} (−j)^m e^{jmφ} R_m(θ). Track the top mode's (±m_max)
    // contribution separately so the caller can form I(M) = total − top for the D-6
    // self-check without a second sweep.
    let mut acc = r_pos[0] * scale; // m = 0: (−j)^0 = 1, e^0 = 1
    let mut top = Complex64::new(0.0, 0.0);
    for m in 1..=mmax {
        let mf = m as f64;
        let epos = Complex64::new(0.0, mf * phi).exp();
        let eneg = Complex64::new(0.0, -mf * phi).exp();
        let contrib = pow_neg_j(m as i32) * epos * r_pos[m] * scale
            + pow_neg_j(-(m as i32)) * eneg * r_neg[m] * scale;
        acc += contrib;
        if m == mmax {
            top = contrib;
        }
    }
    (acc * 2.0 * PI, top * 2.0 * PI)
}

/// Simpson's rule weight for index i in array of n points
///
/// Returns:
/// - 1 for first and last points
/// - 4 for odd interior indices
/// - 2 for even interior indices
#[inline]
fn simpson_weight(i: usize, n: usize) -> f64 {
    if i == 0 || i == n - 1 {
        1.0
    } else if i % 2 == 1 {
        4.0
    } else {
        2.0
    }
}

/// Aperture integrand function
///
/// Computes the integrand at a single aperture point (ρ, φ') for observation
/// direction (θ, φ).
///
/// # Formula
/// ```text
/// Integrand = A(ρ,φ') · exp[j·Ψ(ρ,φ')]
/// ```
///
/// where:
/// - A(ρ,φ') is the illumination amplitude from the feed
/// - Ψ(ρ,φ') is the total phase (path + coma + surface + mesh)
///
/// # Arguments
/// - `rho`: Radial coordinate in aperture (meters)
/// - `phi_prime`: Azimuthal coordinate in aperture (radians)
/// - `theta`: Observation polar angle (radians)
/// - `phi`: Observation azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `k`: Wavenumber (rad/m)
/// - `wavelength`: Wavelength (meters)
///
/// # Returns
/// Complex integrand value
///
/// Test-only since P10 Task 2: it is the single-point integrand of the retained 2D
/// reference (`integrate_2d_simpson`), which no longer runs in production.
#[cfg(test)]
#[inline]
#[allow(clippy::too_many_arguments)]
fn aperture_integrand(
    rho: f64,
    phi_prime: f64,
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    k: f64,
    _wavelength: f64,
    use_higher_order_aberrations: bool,
) -> Complex64 {
    // Calculate illumination amplitude
    let amplitude =
        illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);

    // Create aperture coordinates
    let aperture = ApertureCoordinates { rho, phi_prime };

    // Calculate feed displacement from position.
    // Lateral (xy-plane) displacement drives coma; axial (z) offset drives defocus.
    let feed_displacement = config.feed.position.radial_displacement();
    let feed_displacement_angle = config.feed.position.y.atan2(config.feed.position.x);
    // Axial offset of the feed's PHASE CENTER from the focal point: physical
    // z-offset plus any DELIBERATE defocus (positive = away from the vertex,
    // matching phase_feed_displacement's delta_z). The feed's own
    // phase_center_offset is assumed compensated by per-band feed positioning
    // (auto-refocus, roadmap P7 decided 2026-07-10) and does not contribute.
    let feed_axial_offset =
        config.feed.position.z - config.reflector.focal_length + config.feed.axial_defocus;

    // Calculate angle of incidence (simplified - assumes small angles)
    // For parabolic reflector, theta_incident ≈ ρ/(2f)
    let theta_incident = rho / (2.0 * config.reflector.focal_length);

    // Get mesh spacing (0.0 if no mesh)
    let mesh_spacing = config.mesh.as_ref().map_or(0.0, |m| m.spacing);

    // Surface error at this point (ρ, φ')
    //
    // FUTURE ENHANCEMENT: Spatially-varying surface error model
    // Currently uses ideal surface (surface_error = 0.0) for all aperture points.
    // The Ruze efficiency factor in pattern.rs handles the statistical effect
    // of surface RMS on overall gain, which is sufficient for most applications.
    //
    // For higher fidelity modeling of specific antennas with measured surface maps:
    // - Option 1: Zernike polynomial expansion of measured surface
    // - Option 2: Interpolate from measured surface map (x, y, z points)
    // - Option 3: Use correction surface from calibration (already implemented)
    //
    // Rationale for current approach:
    // - Calibration correction surface (B-spline) captures measured deviations
    // - Ruze statistical model is accurate for random surface errors
    // - Explicit surface modeling adds complexity with marginal accuracy gain
    let surface_error = 0.0;

    // Calculate total phase
    let mut total_phase = phase_total(
        aperture,
        theta,
        phi,
        config.reflector.focal_length,
        feed_displacement,
        feed_displacement_angle,
        feed_axial_offset,
        surface_error,
        theta_incident,
        mesh_spacing,
        k,
    );

    // Add higher-order Seidel aberrations if enabled
    // These include astigmatism, field curvature, and distortion terms
    if use_higher_order_aberrations && feed_displacement > 0.0 {
        total_phase += higher_order_aberrations(
            rho,
            phi_prime,
            feed_displacement,
            feed_displacement_angle,
            config.reflector.focal_length,
            k,
        );
    }

    // Combine: A(ρ,φ') · exp(j·Ψ)
    let phase_factor = Complex64::new(0.0, total_phase).exp();

    amplitude * phase_factor
}

/// ∬ |A(ρ,φ')|² ρ dρ dφ' over the aperture — denominator of the aperture-directivity
/// formula. Uses the same illumination model and Simpson scheme as the field integral.
///
/// The directivity of an aperture is
/// ```text
/// D(θ,φ) = (4π/λ²) · |∬ A e^{jΨ} ρ dρ dφ'|² / ∬ |A|² ρ dρ dφ'
/// ```
/// This function computes the (real, phase-free) denominator. The numerator is the
/// raw aperture integral from [`integrate_aperture`] (i.e. `IntegrationResult::field`),
/// NOT the normalized [`compute_far_field`] value.
pub fn integrate_amplitude_squared(
    config: &AntennaConfiguration,
    n_rho: usize,
    n_phi: usize,
) -> f64 {
    let rho_max = config.reflector.diameter / 2.0;

    // Ensure odd number of points for Simpson's rule.
    let n_rho = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let n_phi = if n_phi.is_multiple_of(2) {
        n_phi + 1
    } else {
        n_phi
    };

    let h_rho = rho_max / (n_rho - 1) as f64;
    let h_phi = 2.0 * PI / (n_phi - 1) as f64;

    let mut sum = 0.0;
    for j in 0..n_phi {
        let phi_prime = j as f64 * h_phi;
        let wj = simpson_weight(j, n_phi);
        let mut inner = 0.0;
        for i in 0..n_rho {
            let rho = i as f64 * h_rho;
            let a =
                illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);
            inner += a * a * rho * simpson_weight(i, n_rho);
        }
        sum += inner * wj;
    }

    sum * h_rho * h_phi / 9.0
}

/// Compute far-field normalization factor
///
/// The complete far-field formula includes a normalization factor:
/// ```text
/// E(θ,φ) = (jk·exp(-jkr))/(2λr) × [aperture integral]
/// ```
///
/// This function computes the normalization factor, excluding the r-dependent
/// terms which are typically omitted in pattern calculations (relative patterns).
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Complex normalization factor (jk)/(2λ)
pub fn far_field_normalization(wavelength: f64) -> Complex64 {
    let k = wavenumber(wavelength);

    // (jk) / (2λ) = (j * 2π/λ) / (2λ) = jπ/λ²
    Complex64::new(0.0, 1.0) * k / (2.0 * wavelength)
}

/// Compute normalized far-field electric field
///
/// Combines aperture integration with normalization factor to produce
/// the complete far-field electric field (excluding r-dependent terms).
///
/// # Arguments
/// - `theta`: Polar angle (radians)
/// - `phi`: Azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `params`: Integration parameters
///
/// # Returns
/// Complex electric field value (normalized, excluding 1/r factor)
pub fn compute_far_field(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<Complex64> {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let integration_result = integrate_aperture(theta, phi, config, frequency_hz, params)?;

    let normalization = far_field_normalization(wavelength);

    Ok(normalization * integration_result.field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{FeedParameters, FeedPosition, MeshParameters, ReflectorGeometry};

    /// Create a simple test antenna configuration
    fn test_antenna() -> AntennaConfiguration {
        use crate::model::geometry::MeshPattern;

        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(); // 1m diameter, f/D=0.5, ideal surface
        let feed_pos = FeedPosition::at_focus(0.5);
        let feed = FeedParameters::new(feed_pos, 8.0, 0.0, 1.0).unwrap(); // q=8, no offset, symmetric
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap(); // 5mm spacing, 0.5mm wire

        AntennaConfiguration::new(
            "test_antenna".to_string(),
            "Test Antenna".to_string(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap()
    }

    /// Large synthetic dish (10 m, f/D=0.5, feed at focus, broad q≈2 feed, no mesh):
    /// D/λ ≈ 280 at 8.4 GHz, so the 2D quadrature ALIASES off-axis (returns a
    /// spuriously high, roughly flat value) while the exact 1D Hankel transform must
    /// fall monotonically well below boresight.
    fn large_test_antenna() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(10.0, 5.0, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(5.0), 2.0, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("large".into(), "Large".into(), reflector, feed, None).unwrap()
    }

    /// Small dish (`D/λ ≈ 104` at X-band) with a lateral feed offset (coma). The 2D
    /// quadrature is trustworthy at this size, so it is the near-in ground truth against
    /// which the azimuthal-mode expansion is validated. `lateral = 0.05 m` matches the
    /// served `gs_3.7m` x-band feed; `q = 2` gives a broad illumination (heavy edge
    /// content, a stress test for the mode truncation).
    fn offset_feed_test_antenna(diameter: f64, focal: f64, lateral: f64) -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(diameter, focal, 0.0).unwrap();
        let mut pos = FeedPosition::at_focus(focal);
        pos.x = lateral; // lateral offset in +x → breaks azimuthal symmetry (coma)
        let feed = FeedParameters::new(pos, 2.0, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("off".into(), "Off".into(), reflector, feed, None).unwrap()
    }

    #[test]
    fn azimuthal_modes_match_2d_small_dish_with_offset() {
        // 3.7 m dish, X-band, lateral feed offset 0.05 m (served gs_3.7m x-band feed).
        // The 2D quadrature is ground truth near-in here (D/λ ~ 104, small), so the mode
        // expansion must match it AND reproduce coma asymmetry (|E(φ=0)| ≠ |E(φ=π)|).
        let config = offset_feed_test_antenna(3.7, 1.85, 0.05);
        let f = 8.4e9;
        let mut hi = IntegrationParams::high_accuracy();
        hi.min_rho_points = 512;
        hi.max_rho_points = 512;
        hi.min_phi_points = 1024;
        hi.max_phi_points = 1024;
        hi.max_iterations = 1;
        let k = wavenumber(wavelength_from_frequency(f));

        for deg in [0.0_f64, 1.0, 5.0, 20.0] {
            let th = deg.to_radians();
            let ref_field = integrate_2d_simpson_public_shim(th, 0.0, &config, f, &hi);
            let mode_field = azimuthal_mode_field(&config, th, 0.0, k, 4097, 128, 48, false);
            let d_db = 20.0 * (mode_field.norm() / ref_field.norm()).log10();
            assert!(d_db.abs() < 0.1, "θ={deg}°: mode vs 2D Δ={d_db:.4} dB");
        }

        // Coma asymmetry: off-axis in the +x plane (φ=0) vs the −x plane (φ=π) must differ.
        let th = 3.0_f64.to_radians();
        let plus = azimuthal_mode_field(&config, th, 0.0, k, 4097, 128, 48, false).norm();
        let minus = azimuthal_mode_field(&config, th, PI, k, 4097, 128, 48, false).norm();
        assert!(
            (plus - minus).abs() / plus.max(minus) > 1e-3,
            "coma asymmetry absent: |E(φ=0)|={plus}, |E(φ=π)|={minus}"
        );
    }

    /// The azimuthal-mode integrator must reproduce the symmetric Hankel path exactly
    /// when the aperture is symmetric (m=0-only special case) — a consistency self-check
    /// that the ±m assembly and normalisation are correct.
    #[test]
    fn azimuthal_modes_reduce_to_hankel_when_symmetric() {
        let config = large_test_antenna(); // symmetric: feed at focus, asymmetry_factor=1
        let f = 8.4e9;
        let k = wavenumber(wavelength_from_frequency(f));
        for deg in [0.0_f64, 1.0, 5.0, 20.0, 90.0] {
            let th = deg.to_radians();
            let hankel = hankel_radial_field(&config, th, 0.0, k, 4097);
            let modes = azimuthal_mode_field(&config, th, 0.0, k, 4097, 64, 4, false);
            let rel = (hankel - modes).norm() / hankel.norm().max(1e-30);
            assert!(rel < 1e-9, "θ={deg}°: Hankel vs modes rel diff {rel:.2e}");
        }
    }

    #[test]
    fn hankel_symmetric_is_physical_off_axis() {
        // Large dish (D/λ ~ 280): 2D fast() aliases to a high, flat value off-axis;
        // the Hankel form must fall monotonically and stay well below boresight.
        let config = large_test_antenna(); // 10 m dish, feed at focus (symmetric)
        let f = 8.4e9;
        let g = |deg: f64| {
            let th = deg.to_radians();
            let r = integrate_aperture(th, 0.0, &config, f, &IntegrationParams::default()).unwrap();
            r.field.norm_sqr()
        };
        let g0 = g(0.0);
        // Off-axis power must be far below boresight and must DECREASE with angle
        // (the aliasing signature is a roughly flat high value — this rejects it).
        assert!(g(5.0) < g0 * 1e-2, "5deg not far below boresight");
        assert!(g(20.0) < g(5.0), "pattern must fall from 5deg to 20deg");
        assert!(g(90.0) < g(20.0), "pattern must fall from 20deg to 90deg");
    }

    /// A gbt_100m-like dish (`D=100 m`, `f=60 m`, symmetric feed) at Q-band. Adaptive
    /// density: `radial_points_for` at θ=90° must be O(10⁴) (≈ `4·D/λ`), NOT the O(10⁸)
    /// a fixed 2D grid at true Nyquist would imply, and the gain eval there must pass its
    /// N-vs-2N self-check (converged=true).
    fn gbt_like_antenna() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(100.0, 60.0, 0.000_275).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(60.0), 3.15, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("gbt".into(), "GBT".into(), reflector, feed, None).unwrap()
    }

    #[test]
    fn radial_points_for_gbt_qband_is_tens_of_thousands() {
        let config = gbt_like_antenna();
        let f_hz = 43.0e9; // Q-band
        let wl = wavelength_from_frequency(f_hz);
        let p = IntegrationParams::default();
        let n = radial_points_for(&config, PI / 2.0, wl, &p);
        println!("radial_points_for(gbt_100m q-band {f_hz:.0} Hz, θ=90°) = {n}");
        assert!(
            (10_000..200_000).contains(&n),
            "expected O(10^4) radial points, got {n}"
        );
        // A full gain eval at θ=90° must pass the N-vs-2N self-check.
        let r = integrate_aperture(PI / 2.0, 0.0, &config, f_hz, &p).unwrap();
        println!(
            "gbt q-band θ=90°: |field|={:.4e} converged={} err={:.3e} evals={}",
            r.field.norm(),
            r.converged,
            r.error_estimate,
            r.num_evaluations
        );
        assert!(
            r.converged,
            "gbt q-band θ=90° must converge (err={:.3e})",
            r.error_estimate
        );
    }

    #[test]
    fn radial_density_scales_with_dlambda_sintheta() {
        let small = test_antenna(); // 1 m
        let large = large_test_antenna(); // 10 m
        let wl = wavelength_from_frequency(8.4e9);
        let p = IntegrationParams::default();
        // θ=0 → floor (chirp & J_m kernel vanish; no oversampling), forced odd.
        let n0 = radial_points_for(&small, 0.0, wl, &p);
        assert_eq!(n0, p.min_rho_points | 1, "θ=0 must drop to the odd floor");
        // θ=90° → count ∝ D/λ, so the 10× larger dish needs ~10× the points.
        let ns = radial_points_for(&small, PI / 2.0, wl, &p);
        let nl = radial_points_for(&large, PI / 2.0, wl, &p);
        assert!(
            nl > ns * 5,
            "θ=90° density must scale with D/λ: large={nl} small={ns}"
        );
        assert!(ns % 2 == 1 && nl % 2 == 1, "counts must be odd for Simpson");
    }

    /// A dsn_34m-like dish with the served X-band lateral feed offset (`δ = 0.15 m`,
    /// `k·δ·(R/f) ≈ 33 rad`). The adaptive mode count must resolve the wide coma spectrum:
    /// the M-vs-(M+1) self-check must report converged=true at every angle, and the
    /// pattern must be physical off-axis (far below boresight, no rise with θ — the
    /// aliasing signature).
    fn dsn34m_like_xband() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(34.0, 13.6, 0.000_25).unwrap();
        let mut pos = FeedPosition::at_focus(13.6);
        pos.x = 0.15; // served x_band lateral offset
        let feed = FeedParameters::new(pos, 1.14, 0.0, 1.0).unwrap();
        AntennaConfiguration::new("dsn".into(), "DSN".into(), reflector, feed, None).unwrap()
    }

    #[test]
    fn dsn34m_offset_feed_mode_count_converges() {
        let config = dsn34m_like_xband();
        let f_hz = 8.4e9;
        let wl = wavelength_from_frequency(f_hz);
        for deg in [0.0_f64, 1.0, 5.0, 20.0, 90.0] {
            let (m, n_phi) = mode_count_for(&config, wl, deg.to_radians());
            println!("dsn_34m x-band θ={deg:>4}°: adaptive M={m} n_phi={n_phi}");
        }

        let p = IntegrationParams::default();
        let g = |deg: f64| integrate_aperture(deg.to_radians(), 0.0, &config, f_hz, &p).unwrap();
        let r0 = g(0.0);
        let g0 = r0.field.norm_sqr();
        let mut prev = f64::INFINITY;
        for deg in [1.0_f64, 5.0, 20.0, 90.0] {
            let r = g(deg);
            let power = r.field.norm_sqr();
            println!(
                "dsn_34m x-band θ={deg:>4}°: rel_power={:.3e} converged={} err={:.3e}",
                power / g0,
                r.converged,
                r.error_estimate
            );
            assert!(
                r.converged,
                "dsn_34m θ={deg}° mode count must converge (M vs M+1)"
            );
            // Physical: every off-axis angle far below boresight and not rising with θ.
            assert!(
                power < g0 * 1e-2,
                "dsn_34m θ={deg}° not far below boresight"
            );
            assert!(
                power <= prev * 1.5,
                "dsn_34m θ={deg}° pattern rose with θ (aliasing signature)"
            );
            prev = power;
        }
    }

    #[test]
    fn unconverged_is_flagged_not_silently_returned() {
        // A dish whose radial Nyquist rate (2·D/λ) EXCEEDS 2× the safety cap at θ=90°:
        // the adaptive count clamps to RADIAL_POINTS_SAFETY_MAX (below Nyquist → aliased),
        // while the self-check's 2N leg samples above Nyquist. They disagree → the result
        // MUST be flagged non-converged, never silently returned. D/λ = 100000 (750 m dish
        // at 40 GHz) ⇒ Nyquist = 2·10⁵ ≫ cap (6.5·10⁴), 2N leg = 1.31·10⁵ still < Nyquist
        // is NOT enough — so use a size where 2N clears Nyquist too. Here 2N ≈ 1.31·10⁵ and
        // Nyquist = 2·10⁵: the coarse leg is badly aliased and the fine leg less so, giving
        // a large, honest disagreement.
        let reflector = ReflectorGeometry::new(750.0, 375.0, 0.0).unwrap(); // f/D = 0.5
        let feed = FeedParameters::new(FeedPosition::at_focus(375.0), 2.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("huge".into(), "Huge".into(), reflector, feed, None).unwrap();
        let f_hz = 40.0e9; // λ = 0.0075 m → D/λ = 100000
        let r = integrate_aperture(PI / 2.0, 0.0, &config, f_hz, &IntegrationParams::default())
            .unwrap();
        println!(
            "huge dish θ=90°: converged={} err={:.3e} evals={}",
            r.converged, r.error_estimate, r.num_evaluations
        );
        assert!(
            !r.converged,
            "must flag non-convergence when density is capped below Nyquist"
        );
        // Even when flagged, the error estimate stays a finite, non-negative number.
        assert!(r.error_estimate.is_finite() && r.error_estimate >= 0.0);
    }

    #[test]
    fn asymmetric_amplitude_feed_bypasses_symmetric_hankel_path() {
        // A centered feed (no lateral offset) with a non-unity asymmetry_factor has an
        // azimuthally-dependent (elliptical) illumination, so it must NOT take the J₀
        // Hankel path — that path hardcodes phi_prime=0 and ignores observation φ.
        // Proof: the retained 2D path yields an observation-φ-dependent field, whereas
        // the Hankel path would return the identical value for every φ.
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.5).unwrap();
        let config =
            AntennaConfiguration::new("asym".into(), "Asym".into(), reflector, feed, None).unwrap();
        let params = IntegrationParams::fast();

        let theta = 0.05; // ~2.9° off-axis, where the elliptical beam is resolvable
        let g_e = integrate_aperture(theta, 0.0, &config, 8.4e9, &params)
            .unwrap()
            .field
            .norm();
        let g_h = integrate_aperture(theta, PI / 2.0, &config, 8.4e9, &params)
            .unwrap()
            .field
            .norm();

        // Non-trivial φ dependence proves the 2D (non-Hankel) path was taken.
        assert!(
            (g_e - g_h).abs() > 1e-6 * g_e.max(g_h),
            "asymmetric centered feed must retain φ dependence (2D path): E-plane={g_e}, H-plane={g_h}"
        );
    }

    #[test]
    fn asymmetric_illumination_centered_feed_converges_and_matches_2d() {
        // Review FIX 1: a CENTERED feed (δ=0) with a non-unity asymmetry_factor has NO
        // lateral coma (spread=0), but `illumination_amplitude` modulates the effective
        // q-factor by cos(2φ'), so the aperture function g(ρ,φ') carries genuine m=±2
        // (plus weaker ±4, ±6) azimuthal content. `mode_count_for` must NOT early-return
        // (1, MODE_PHI_MIN): with m_max=1 the mode sum under-resolves that content AND the
        // M-vs-(M+1) self-check sees the large m=2 jump → spurious converged=false.
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(); // small dish → fast
        let feed = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.5).unwrap();
        let config =
            AntennaConfiguration::new("asym".into(), "Asym".into(), reflector, feed, None).unwrap();
        let f_hz = 8.4e9;
        let wl = wavelength_from_frequency(f_hz);
        let params = IntegrationParams::default();

        let theta = 5.0_f64.to_radians();
        let r_off = integrate_aperture(theta, 0.0, &config, f_hz, &params).unwrap();
        let r_on = integrate_aperture(0.0, 0.0, &config, f_hz, &params).unwrap();

        // Not spuriously flagged, and physically plausible (finite, positive, below boresight).
        assert!(
            r_off.converged,
            "asymmetric-illumination centered feed must converge (err={:.3e})",
            r_off.error_estimate
        );
        let (off, on) = (r_off.field.norm(), r_on.field.norm());
        assert!(
            off.is_finite() && off > 0.0,
            "off-axis field must be finite/positive"
        );
        assert!(
            off < on,
            "off-axis must be below boresight: off={off} on={on}"
        );

        // The mode-path result must match the trusted 2D quadrature reference to <0.1 dB —
        // proof the m=±2, ±4, ±6 content is actually RESOLVED, not merely un-warned.
        let k = wavenumber(wl);
        let mut hi = IntegrationParams::high_accuracy();
        hi.max_rho_points = 513;
        hi.max_phi_points = 1025;
        let ref_field = integrate_2d_simpson_public_shim(theta, 0.0, &config, f_hz, &hi);
        let (m_max, n_phi) = mode_count_for(&config, wl, theta);
        assert!(
            m_max >= 6,
            "asym illumination must resolve at least ~m=6, got M={m_max}"
        );
        let mode_field = azimuthal_mode_field(&config, theta, 0.0, k, 4097, n_phi, m_max, false);
        let d_db = 20.0 * (mode_field.norm() / ref_field.norm()).log10();
        assert!(
            d_db.abs() < 0.1,
            "mode vs 2D Δ={d_db:.4} dB (M={m_max}, n_phi={n_phi})"
        );
    }

    #[test]
    fn test_simpson_weight() {
        // Test Simpson's rule weights
        let n = 5; // 5 points

        assert_eq!(simpson_weight(0, n), 1.0); // First point
        assert_eq!(simpson_weight(1, n), 4.0); // Odd interior
        assert_eq!(simpson_weight(2, n), 2.0); // Even interior
        assert_eq!(simpson_weight(3, n), 4.0); // Odd interior
        assert_eq!(simpson_weight(4, n), 1.0); // Last point
    }

    #[test]
    fn test_integration_params_default() {
        let params = IntegrationParams::default();

        assert!(params.min_rho_points > 0);
        assert!(params.max_rho_points >= params.min_rho_points);
        assert!(params.relative_tolerance > 0.0);
        assert!(params.max_iterations > 0);
    }

    #[test]
    fn test_integration_params_fast() {
        let params = IntegrationParams::fast();
        let default_params = IntegrationParams::default();

        // Fast should use fewer points
        assert!(params.min_rho_points <= default_params.min_rho_points);
        assert!(params.max_rho_points <= default_params.max_rho_points);
    }

    #[test]
    fn test_integration_params_high_accuracy() {
        let params = IntegrationParams::high_accuracy();
        let default_params = IntegrationParams::default();

        // High accuracy should use more points and tighter tolerance
        assert!(params.max_rho_points >= default_params.max_rho_points);
        assert!(params.relative_tolerance <= default_params.relative_tolerance);
    }

    #[test]
    fn test_aperture_integrand_on_axis() {
        let config = test_antenna();
        let wavelength = 0.0357; // ~8.4 GHz
        let k = wavenumber(wavelength);

        // On-axis (θ=0, φ=0), center of aperture (ρ=0)
        let integrand = aperture_integrand(0.0, 0.0, 0.0, 0.0, &config, k, wavelength, false);

        // At center, amplitude should be near maximum, phase should be well-defined
        assert!(integrand.norm() > 0.0);
        assert!(integrand.norm() <= 1.0);
    }

    #[test]
    fn test_aperture_integrand_symmetry() {
        let config = test_antenna();
        let wavelength = 0.0357;
        let k = wavenumber(wavelength);

        // For symmetric feed and ideal surface, pattern should have azimuthal symmetry
        let rho = 0.2;
        let theta = 0.1;

        let integrand_0 = aperture_integrand(rho, 0.0, theta, 0.0, &config, k, wavelength, false);
        let integrand_90 = aperture_integrand(
            rho,
            PI / 2.0,
            theta,
            PI / 2.0,
            &config,
            k,
            wavelength,
            false,
        );

        // Magnitudes should be equal due to symmetry
        assert!((integrand_0.norm() - integrand_90.norm()).abs() < 1e-6);
    }

    #[test]
    fn test_integrate_aperture_on_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast(); // Use fast for quicker tests

        let result = integrate_aperture(
            0.0, // theta (on-axis)
            0.0, // phi
            &config, 8.4e9, // 8.4 GHz
            &params,
        )
        .unwrap();

        // On-axis field should be non-zero
        assert!(result.field.norm() > 0.0);

        // Should have performed evaluations
        assert!(result.num_evaluations > 0);

        // On-axis integration with fast params must converge (smooth, no phase oscillation).
        assert!(result.converged, "on-axis fast integration must converge");
        // A converged result must have a finite, non-negative error estimate.
        assert!(
            result.error_estimate.is_finite(),
            "converged error_estimate must be finite"
        );
        assert!(result.error_estimate >= 0.0);
    }

    #[test]
    fn test_integrate_aperture_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // Small off-axis angle
        let result = integrate_aperture(
            0.05, // theta (small angle ~2.9°)
            0.0,  // phi
            &config, 8.4e9, &params,
        )
        .unwrap();

        // Off-axis field should be non-zero but smaller than on-axis
        assert!(result.field.norm() > 0.0);
    }

    #[test]
    fn test_integrate_aperture_convergence() {
        let config = test_antenna();

        // Test that higher accuracy params give better results
        let fast_params = IntegrationParams::fast();
        let accurate_params = IntegrationParams::high_accuracy();

        let fast_result = integrate_aperture(0.0, 0.0, &config, 8.4e9, &fast_params).unwrap();
        let accurate_result =
            integrate_aperture(0.0, 0.0, &config, 8.4e9, &accurate_params).unwrap();

        // Both must converge so the error-estimate comparison below is meaningful.
        assert!(
            fast_result.converged,
            "fast on-axis integration must converge"
        );
        assert!(
            accurate_result.converged,
            "accurate on-axis integration must converge"
        );

        // High accuracy should have lower error estimate
        assert!(accurate_result.error_estimate <= fast_result.error_estimate * 2.0);

        // Results should be similar
        let difference = (fast_result.field - accurate_result.field).norm();
        let magnitude = accurate_result.field.norm();
        assert!(difference / magnitude < 0.1); // Within 10%
    }

    #[test]
    fn test_integrate_aperture_invalid_inputs() {
        let config = test_antenna();
        let params = IntegrationParams::default();

        // Invalid frequency
        let result = integrate_aperture(0.0, 0.0, &config, -1.0, &params);
        assert!(result.is_err());

        // Invalid angle (NaN)
        let result = integrate_aperture(f64::NAN, 0.0, &config, 8.4e9, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_integrate_amplitude_squared_positive_finite() {
        let config = test_antenna();

        // The denominator of the directivity formula must be a positive, finite
        // real number for a physically-illuminated aperture.
        let amp_sq = integrate_amplitude_squared(&config, 33, 65);
        assert!(amp_sq.is_finite());
        assert!(amp_sq > 0.0);

        // Sanity upper bound: |A| <= 1 everywhere, so the integral is at most the
        // area integral ∬ ρ dρ dφ' = π(D/2)² = π·0.25 ≈ 0.785 for the 1m test dish.
        let area = PI * (config.reflector.diameter / 2.0).powi(2);
        assert!(amp_sq <= area + 1e-9);
    }

    #[test]
    fn test_far_field_normalization() {
        let wavelength = 0.0357; // ~8.4 GHz
        let norm = far_field_normalization(wavelength);

        // Should be purely imaginary (j factor)
        assert!(norm.re.abs() < 1e-10);
        assert!(norm.im != 0.0);

        // Magnitude should be k/(2λ) = π/λ²
        let expected_magnitude = PI / (wavelength * wavelength);
        assert!((norm.norm() - expected_magnitude).abs() < 1e-6);
    }

    #[test]
    fn test_compute_far_field() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let field = compute_far_field(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Far field should be non-zero
        assert!(field.norm() > 0.0);

        // Should be complex-valued
        // (May have both real and imaginary parts depending on phase)
    }

    #[test]
    fn test_pattern_decreases_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // On-axis field
        let field_on_axis = compute_far_field(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Off-axis field (5 degrees)
        let field_off_axis =
            compute_far_field(5.0_f64.to_radians(), 0.0, &config, 8.4e9, &params).unwrap();

        // Pattern should decrease off-axis
        assert!(field_off_axis.norm() < field_on_axis.norm());
    }

    #[test]
    fn test_non_convergence_is_reported() {
        // The 2D adaptive refinement loop is what carries the non-convergence sentinel
        // (converged=false, error_estimate=INFINITY). Since P10 Task 2 NO production
        // aperture goes through that loop — symmetric feeds take the exact 1D Hankel path
        // and asymmetric/coma feeds take the Jₘ mode expansion, both of which return
        // converged=true in the interim (the real runtime self-check is Task 3). This
        // test therefore pins the retained 2D mechanism DIRECTLY via `integrate_2d_adaptive`
        // (the exact loop `integrate_aperture` used pre-Task-2), pending Task 3 folding a
        // convergence self-check into the Hankel/mode paths.
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::new(0.01, 0.0, 0.5), 8.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("off".into(), "Off".into(), reflector, feed, None).unwrap();
        let params = IntegrationParams {
            max_iterations: 1, // cannot converge: convergence check needs iteration > 0
            relative_tolerance: 1e-15,
            ..IntegrationParams::fast()
        };
        let result = integrate_2d_adaptive(0.3, 0.0, &config, 8.4e9, &params);
        assert!(!result.converged);
        // With max_iterations == 1 the loop runs a single iteration and the convergence
        // check (iteration > 0) is never reached, so no inter-iteration difference is
        // ever computed.  last_difference remains at its INFINITY sentinel value.
        assert_eq!(result.error_estimate, f64::INFINITY);
    }

    #[test]
    fn test_integration_2d_simpson_basic() {
        let config = test_antenna();
        let wavelength = 0.0357;
        let k = wavenumber(wavelength);

        // Simple integration test
        let (result, evals) = integrate_2d_simpson(
            0.0, // theta
            0.0, // phi
            &config,
            k,
            wavelength,
            0.0,      // rho_min
            0.5,      // rho_max (half diameter)
            0.0,      // phi_min
            2.0 * PI, // phi_max
            17,       // n_rho (odd)
            33,       // n_phi (odd)
            false,    // use_higher_order_aberrations
        );

        // Should produce non-zero result
        assert!(result.norm() > 0.0);

        // Should have performed expected number of evaluations
        assert_eq!(evals, 17 * 33);
    }

    /// Auto-refocus (roadmap P7): phase_center_offset is a recorded feed property
    /// the model compensates — it must NOT change gain. Deliberate defocus goes
    /// through the explicit axial_defocus field instead.
    #[test]
    fn test_phase_center_offset_alone_produces_no_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let feed_pco = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.05, 1.0).unwrap();

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_pco = crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_pco), 8.4e9, &params)
            .unwrap()
            .gain;

        assert!(
            (g_focused - g_pco).abs() < 1e-9,
            "phase_center_offset is auto-refocused and must not change gain: \
             focused={g_focused:.6}, pco={g_pco:.6}"
        );
    }

    /// The defocus math stays live through the explicit field: a 5 cm deliberate
    /// axial defocus must cost >1 dB at 8.4 GHz (same physics the old
    /// test_phase_center_offset_produces_defocus_loss pinned).
    #[test]
    fn test_axial_defocus_produces_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let mut feed_defocused =
            FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        feed_defocused.axial_defocus = 0.05;

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_defocused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_defocused), 8.4e9, &params)
                .unwrap()
                .gain;

        assert!(
            g_focused - g_defocused > 1.0,
            "5 cm axial_defocus must cost >1 dB defocus at 8.4 GHz: \
             focused={g_focused:.2}, defocused={g_defocused:.2}"
        );
    }
}
