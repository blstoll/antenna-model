//! Far-Field Pattern Computation
//!
//! This module computes antenna gain patterns, efficiency factors, and G/T ratios
//! from the far-field electric field obtained via aperture integration.
//!
//! # Pattern Computation Pipeline
//!
//! 1. **Electric Field**: Computed via aperture integration (integration module)
//! 2. **Gain**: Derived from electric field magnitude with efficiency factors
//! 3. **G/T Ratio**: Combines gain with noise temperature
//!
//! # Efficiency Factors
//!
//! Multiple efficiency factors reduce the ideal gain:
//! - **Ruze Efficiency**: Surface errors reduce gain as η_ruze = exp(-(4πσ/λ)²)
//! - **Mesh Reflection Efficiency**: Wire mesh reflectors have frequency-dependent reflectivity (inductive-grid model)
//! - **Illumination Efficiency**: Non-uniform illumination reduces effective aperture
//! - **Spillover Efficiency**: Feed pattern energy missing the reflector
//! - **Sidelobe Floor (F7)**: Ruze-scattered power combines with the pattern as an incoherent power sum forward and serves floor-only behind the dish (opt-in via `IntegrationParams::apply_sidelobe_floor`)
//!
//! # References
//! - Design doc Section 2.1 (Core Physical Optics Model)
//! - Design doc Section 2.4 (Mesh Reflector Efficiency)
//! - Implementation plan Sprint 2, Task 2.5

use std::f64::consts::{FRAC_PI_2, PI};

use num_complex::Complex64;

/// Warning message emitted when the aperture integration loop exhausts its
/// iteration budget without meeting the convergence criterion.  Extracted as a
/// constant so the text stays consistent across all four gain-computation helpers
/// and the existing test can rely on `.contains("did not converge")`.
const INTEGRATION_NONCONVERGENCE_WARNING: &str =
    "aperture integration did not converge; gain accuracy may be degraded";

/// Warning message emitted when a feed offset exceeds the severe threshold
/// (> 0.5·f) and gain is computed by the acknowledged ray-tracing stub
/// (`ray_trace.rs`; real ray tracing is gated as feature F2). Extracted as a
/// `pub` constant (roadmap unit P3) so the honest "not fully implemented" text
/// stays byte-identical across the model dispatch that pushes it here and the
/// service-layer re-emission that surfaces it on `/h3-heatmap` cache hits
/// (`service::evaluator::ray_trace_stub_warning`).
pub const RAY_TRACING_STUB_WARNING: &str =
    "WARNING: Ray tracing for large feed offsets (>0.5f) is not fully implemented; \
     gain accuracy may be degraded.";

use crate::error::{ComputationError, ComputationResult};
use crate::model::{
    edge_cases::{
        analyze_edge_cases, apply_gain_floor, apply_gain_floor_db, needs_adaptive_integration,
        ComputationMode, SPILLOVER_MAX_OFFSET_RATIO,
    },
    geometry::AntennaConfiguration,
    integration::{integrate_amplitude_squared, integrate_aperture, IntegrationParams},
    ray_trace::compute_gain_ray_trace,
    wavelength_from_frequency,
};

/// Result of gain computation including warnings
///
/// This struct bundles the computed gain value with any warnings generated
/// during edge case analysis or the computation process.
#[derive(Debug, Clone)]
pub struct GainComputationResult {
    /// Computed gain in linear units (or dB if from compute_gain_db)
    pub gain: f64,

    /// Warnings from edge case analysis and computation
    pub warnings: Vec<String>,

    /// Physical spillover loss (dB, negative) folded into `gain` when
    /// `IntegrationParams::apply_spillover` was set; `None` otherwise.
    pub spillover_loss_db: Option<f64>,
}

/// Select integration parameters based on angle and configuration
///
/// Uses denser sampling near pattern nulls where rapid phase changes
/// require higher accuracy.
fn select_integration_params(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    base_params: &IntegrationParams,
) -> IntegrationParams {
    if needs_adaptive_integration(theta, phi, config) {
        tracing::debug!(
            "Using adaptive integration (theta={:.3} rad, near null region)",
            theta
        );
        base_params.with_adaptive_refinement()
    } else {
        base_params.clone()
    }
}

/// Compute Ruze efficiency for surface errors
///
/// The Ruze equation quantifies the gain loss due to random surface errors:
/// ```text
/// η_ruze = exp(-(4π·σ/λ)²)
/// ```
///
/// where σ is the RMS surface error and λ is the wavelength.
///
/// # Arguments
/// - `surface_rms`: RMS surface error in meters
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Efficiency factor (0 to 1)
///
/// # Examples
/// ```
/// use antenna_model::model::pattern::ruze_efficiency;
///
/// // 1mm RMS error at 8.4 GHz (λ ≈ 35.7mm)
/// let efficiency = ruze_efficiency(0.001, 0.0357);
/// // Should be about 88% (good but not perfect surface)
/// assert!(efficiency > 0.85 && efficiency < 0.90);
///
/// // 5mm RMS error at same frequency (poor surface)
/// let efficiency_poor = ruze_efficiency(0.005, 0.0357);
/// // Should be significantly lower (about 2%)
/// assert!(efficiency_poor < 0.05);
/// ```
pub fn ruze_efficiency(surface_rms: f64, wavelength: f64) -> f64 {
    let ratio = 4.0 * PI * surface_rms / wavelength;
    (-ratio * ratio).exp()
}

/// Compute overall antenna efficiency
///
/// Combines Ruze efficiency (surface errors) and mesh reflection efficiency
/// (inductive-grid model, if mesh present).
///
/// # Arguments
/// - `config`: Antenna configuration
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Combined efficiency factor (0 to 1)
pub fn overall_efficiency(config: &AntennaConfiguration, wavelength: f64) -> f64 {
    // Ruze efficiency (surface errors)
    let eta_ruze = ruze_efficiency(config.reflector.surface_rms, wavelength);

    // Mesh reflection efficiency using inductive-grid model (if mesh present)
    let eta_mesh = if let Some(ref mesh) = config.mesh {
        crate::model::mesh::mesh_reflection_efficiency(mesh.spacing, mesh.wire_diameter, wavelength)
    } else {
        1.0 // Solid reflector - no mesh loss
    };

    // Combined efficiency
    eta_ruze * eta_mesh
}

/// Solid angle (steradians) over which surface-scattered power is spread by the
/// sidelobe-floor model (F7).
///
/// **Set to 4π (isotropic), which is the only power-conserving choice.** The floor is
/// combined with the pattern as an incoherent power sum (`gain + floor`, linear) in the
/// forward hemisphere and served floor-only behind the dish (F7 redesign 2026-07-16),
/// but the power-conservation argument below considers the floor's own budget over the
/// whole 4π sphere, independent of how it is combined with the pattern at any given angle.
/// A pedestal of directivity `D` applied over 4π radiates a power fraction `D`. Since
/// the power available to the pedestal is exactly `p_scatter = 1 − η_ruze`, requiring
/// `D ≤ p_scatter` forces `Ω = 4π` and the level reduces to
///
/// ```text
/// floor_linear = p_scatter · 4π/Ω = p_scatter        (Ω = 4π)
/// ```
///
/// i.e. the scattered fraction, radiated isotropically, has directivity equal to that
/// fraction. Any Ω < 4π describes power concentrated into a *cone* and MUST NOT be
/// applied across the full sphere: the F7 first cut used Ω = 0.25 sr that way and
/// implied ~50× the scattered power actually available (136–326% of everything the
/// antenna radiates). Two useful consequences of Ω = 4π:
///
/// - **Bounded:** `p_scatter ≤ 1` ⇒ the floor can never exceed **0 dBi**, so it cannot
///   swamp the main beam or a near-in sidelobe of any real dish. (The power-sum
///   combination in [`compute_gain`] therefore only ever meaningfully lifts genuine
///   deep sidelobe/null angles — adding a ≤0 dBi floor to a strong main-beam value
///   changes it negligibly.)
/// - **Self-calibrating:** no free fudge constant remains.
///
/// This is D-independent — the level tracks surface quality, not aperture size — which
/// matches the NTIA 84-164 observation that the wide-angle floor is nearly
/// D/λ-independent. A per-antenna angular *shape* (correlation length) is deferred
/// roadmap unit F9. Changing this changes `gain_physics` on the uncalibrated off-axis
/// path → bump `PHYSICS_MODEL_VERSION`.
const OMEGA_SCATTER: f64 = 4.0 * PI;

/// Surface-scatter sidelobe floor (LINEAR gain, relative to isotropic).
///
/// A **best-estimate** off-axis floor, not a conservative envelope (F7 register decision
/// revised 2026-07-12: the primary consumers — link budget and G/T — need accuracy, and a
/// one-sided upper bound is *anti*-conservative for desired-signal margin).
///
/// Level: the Ruze-scattered fraction `p_scatter = 1 − η_ruze`, radiated isotropically
/// (see [`OMEGA_SCATTER`]), then multiplied by `η_mesh` so the floor shares the SAME
/// efficiency basis as the pattern it is power-summed with in [`compute_gain`] (which
/// carries η_ruze × η_mesh). Applied only via `IntegrationParams::apply_sidelobe_floor`.
///
/// # Honest scope — this is EMPIRICAL, not a first-principles derivation
///
/// It is validated against, not derived from, measured data: it tracks the NTIA 84-164
/// wide-angle **median** sidelobe level to within ≈ ±3 dB (−2.0 dB at 4 GHz, +2.9 dB at
/// 6 GHz). The residual has structure: Ruze scatter scales as `(rms/λ)²` while the
/// measured floor is nearly frequency-flat — direct evidence that the real wide-angle
/// floor is dominated by **spillover, blockage and edge diffraction**, which this model
/// does not have. So `(1 − η_ruze)` acts here as a *surface-quality scaling term* that
/// carries those unmodeled mechanisms, not as a literal scattered-power budget.
///
/// Treat a floored value as "about what a real dish of this surface quality does out
/// here," ±3 dB — not as a per-antenna sidelobe prediction. For a conservative
/// *envelope* (interference / regulatory screening), use the ITU mask or calibration
/// data; the envelope is deliberately NOT the served point estimate.
pub fn sidelobe_floor_gain(config: &AntennaConfiguration, wavelength: f64) -> f64 {
    let eta_ruze = ruze_efficiency(config.reflector.surface_rms, wavelength);
    let p_scatter = 1.0 - eta_ruze;

    // Share the pattern's efficiency basis: the pattern gain has already been reduced by
    // mesh reflection loss, so the scattered power that reaches the far field is too.
    let eta_mesh = match config.mesh {
        Some(ref mesh) => crate::model::mesh::mesh_reflection_efficiency(
            mesh.spacing,
            mesh.wire_diameter,
            wavelength,
        ),
        None => 1.0,
    };

    p_scatter * (4.0 * PI / OMEGA_SCATTER) * eta_mesh
}

/// Compute theoretical maximum gain for a circular aperture
///
/// For a uniformly illuminated circular aperture:
/// ```text
/// G_max = η_ap · (π·D/λ)²
/// ```
/// where η_ap is the aperture efficiency (typically 0.5-0.7 for tapered illumination).
///
/// # Arguments
/// - `diameter`: Aperture diameter in meters
/// - `wavelength`: Wavelength in meters
/// - `aperture_efficiency`: Aperture efficiency (default ~0.55 for typical feeds)
///
/// # Returns
/// Maximum gain (linear, not dB)
pub fn theoretical_max_gain(diameter: f64, wavelength: f64, aperture_efficiency: f64) -> f64 {
    let aperture_area = PI * (diameter / 2.0).powi(2);
    let wavelength_squared = wavelength * wavelength;

    aperture_efficiency * 4.0 * PI * aperture_area / wavelength_squared
}

/// Compute antenna gain from electric field magnitude
///
/// Converts the far-field electric field to gain by:
/// 1. Computing power density from field magnitude
/// 2. Normalizing to isotropic radiator
/// 3. Applying efficiency corrections
///
/// # Arguments
/// - `theta`: Polar angle (radians)
/// - `phi`: Azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `params`: Integration parameters
///
/// # Returns
/// Gain (linear, not dB)
///
/// # Examples
/// ```no_run
/// use antenna_model::model::pattern::compute_gain;
/// use antenna_model::model::integration::IntegrationParams;
/// # use antenna_model::model::geometry::{AntennaConfiguration, ReflectorGeometry, FeedParameters};
///
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
/// let result = compute_gain(
///     0.0,                 // On-axis
///     0.0,
///     &config,
///     8.4e9,
///     &IntegrationParams::default(),
/// )?;
///
/// println!("Gain: {:.2} dB", 10.0 * result.gain.log10());
/// # Ok(())
/// # }
/// ```
pub fn compute_gain(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<GainComputationResult> {
    let wavelength = wavelength_from_frequency(frequency_hz);

    // Analyze edge cases and select appropriate computation mode
    let analysis = analyze_edge_cases(config, theta, phi);

    // Log warnings from edge case analysis
    for warning in &analysis.warnings {
        tracing::warn!("{}", warning);
    }

    // Dispatch based on computation mode
    let mut warnings = analysis.warnings;

    // F7 rear-hemisphere policy (maintainer decision 2026-07-16): behind the dish the
    // aperture-PO term is categorically invalid — P10-tail measured a genuinely
    // converged but fictitious backlobe there — so when the statistical floor is active
    // the served value is the floor ALONE, and the (pathologically expensive,
    // discarded-anyway) rear aperture integration is skipped entirely. The NTIA 84-164
    // data calibrating the floor spans 1-180 deg, so the floor is data-backed here.
    // theta = 90 deg exactly is forward (matches the service rear-warning gate, which
    // fires strictly beyond 90 deg). With the flag off (corrected-physics antennas /
    // direct model users), rear queries keep returning raw PO plus the rear warning.
    if params.apply_sidelobe_floor && theta.abs() > FRAC_PI_2 {
        let gain = apply_gain_floor(sidelobe_floor_gain(config, wavelength));
        return Ok(GainComputationResult {
            gain,
            warnings,
            spillover_loss_db: None,
        });
    }

    let gain = match analysis.mode {
        ComputationMode::StandardPhysicalOptics => compute_gain_standard(
            theta,
            phi,
            config,
            frequency_hz,
            wavelength,
            params,
            &mut warnings,
        )?,
        ComputationMode::RayTracing => {
            // Ray tracing is a stub: aperture sampling is used but true spillover and
            // geometric ray-reflector intersection are not fully implemented.
            warnings.push(RAY_TRACING_STUB_WARNING.to_string());
            compute_gain_ray_tracing(
                theta,
                phi,
                config,
                frequency_hz,
                wavelength,
                params,
                &mut warnings,
            )?
        }
    };

    // Physical spillover efficiency (uncalibrated path only; gated by the caller).
    // `analysis.spillover_fraction` is the LOST fraction, so η = 1 − fraction.
    //
    // Only fold spillover in for feed offsets ≤ SPILLOVER_MAX_OFFSET_RATIO·f (0.3f), the
    // regime where `estimate_spillover` is trusted. Beyond 0.3f its `offset_factor` term
    // (`1 + 2·offset_ratio`) is an unvalidated empirical extrapolation — P1 classed
    // large-offset spillover as F2/ray-tracing territory. For the deeper dishes it
    // saturates to 100% (effective edge angle ≥ π/2) and would clamp gain to the
    // degenerate −60 dB floor; for shallow (high-f/D) dishes it instead returns a small
    // but still-unvalidated value. Either way it is not served here: those offsets carry
    // a degraded-accuracy warning from `analyze_edge_cases` (moderate for 0.3f–0.5f,
    // ray-tracing for >0.5f) and keep their pre-P1 gain. Gating on the offset ratio (not
    // the mode enum) preserves P1's approved behavior after P2 folded 0.3f–0.5f into
    // StandardPhysicalOptics. (P1 finding 2026-07-09; boundary made explicit by P2
    // 2026-07-16.)
    let (gain, spillover_loss_db) =
        if params.apply_spillover && analysis.feed_offset_ratio <= SPILLOVER_MAX_OFFSET_RATIO {
            let eta = (1.0 - analysis.spillover_fraction).clamp(1e-6, 1.0);
            (gain * eta, Some(10.0 * eta.log10()))
        } else {
            (gain, None)
        };

    // F7 statistical sidelobe floor (redesign 2026-07-16): incoherent POWER SUM with the
    // idealised-PO pattern. In linear gain the dB-domain power sum
    // 10*log10(10^(PO/10) + 10^(floor/10)) is simply addition. Scattered energy adds to
    // the coherent pattern in power, so this is the physically motivated combination:
    // continuous everywhere (no theta_valid seam in heatmaps), converging to the floor
    // exactly where idealised PO under-predicts. The floor is a best-estimate MEDIAN
    // wide-angle level (register F7, 2026-07-12), power-conserving by construction
    // (Omega = 4*pi => floor = (1 - eta_ruze)*eta_mesh <= 1, i.e. <= 0 dBi).
    let gain = if params.apply_sidelobe_floor {
        gain + sidelobe_floor_gain(config, wavelength)
    } else {
        gain
    };

    // Apply gain floor for numerical stability
    let gain = apply_gain_floor(gain);

    Ok(GainComputationResult {
        gain,
        warnings,
        spillover_loss_db,
    })
}

/// Absolute gain from the raw aperture integral (standard directivity formula):
/// ```text
/// gain(θ,φ) = η · (4π/λ²) · |I|² / ∬|A|² dA,   I = ∬ A e^{jΨ} ρ dρ dφ'
/// ```
/// where `I` is the RAW, un-normalized aperture integral returned by
/// [`integrate_aperture`] (`IntegrationResult::field`) — NOT [`compute_far_field`],
/// whose `jk/(2λ)` normalization (magnitude π/λ²) would be wrongly squared into the
/// directivity. Taper/illumination efficiency is built into the ratio |I|²/∬|A|²,
/// so no separate aperture-efficiency constant is needed.
///
/// `η` here is the Ruze (surface) × mesh efficiency from [`overall_efficiency`].
/// **Spillover efficiency is NOT modeled** — the calibration correction surface
/// absorbs it (and any other residual systematic offset).
///
/// The Huygens obliquity factor (1+cosθ)/2 is applied here as a field factor (F7, 2026-07-16).
fn absolute_gain_from_integral(
    raw_field: Complex64,
    theta: f64,
    config: &AntennaConfiguration,
    wavelength: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    // Denominator integrand |A|²ρ is phase-free and smooth, so the min grid suffices
    // even when the field integral adaptively refines.
    let amp_sq = integrate_amplitude_squared(config, params.min_rho_points, params.min_phi_points);
    if amp_sq <= 1e-20 {
        return Err(ComputationError::NumericalInstability {
            operation: "absolute_gain_from_integral".to_string(),
            reason: "amplitude integral is zero".to_string(),
        });
    }
    // Huygens obliquity (element) factor (1+cosθ)/2 on the FIELD (F7 redesign,
    // maintainer decision 2026-07-16): the textbook element factor of an aperture
    // (Huygens) source. Applied here — OUTSIDE the aperture integral — because it is
    // θ-only: the P10 quadrature, its convergence self-check, and the independent
    // Hankel-oracle test (which compares raw `integrate_aperture` fields) are all
    // unaffected. Equals 1 at θ=0 (boresight anchors unchanged), −6.02 dB on power at
    // θ=90°, and suppresses the fictitious converged rear backlobe P10-tail measured.
    let obliquity = (1.0 + theta.cos()) / 2.0;
    let directivity =
        4.0 * PI / (wavelength * wavelength) * raw_field.norm_sqr() * (obliquity * obliquity)
            / amp_sq;
    Ok(directivity * overall_efficiency(config, wavelength))
}

/// Standard physical optics gain computation
fn compute_gain_standard(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    wavelength: f64,
    params: &IntegrationParams,
    warnings: &mut Vec<String>,
) -> ComputationResult<f64> {
    // Select integration parameters (adaptive near nulls)
    let effective_params = select_integration_params(theta, phi, config, params);

    // Raw aperture integral I = ∬ A e^{jΨ} ρ dρ dφ' at the requested angle.
    // The directivity formula uses this raw value, not the normalized far field.
    let result = integrate_aperture(theta, phi, config, frequency_hz, &effective_params)?;

    if !result.converged {
        warnings.push(INTEGRATION_NONCONVERGENCE_WARNING.to_string());
    }

    absolute_gain_from_integral(result.field, theta, config, wavelength, &effective_params)
}

/// Ray tracing gain computation for large feed offsets
///
/// Ray tracing produces a *relative* pattern only (a |E|²-like quantity). To make it
/// absolute we anchor it to the directivity of the boresight physical-optics aperture
/// field (computed via [`absolute_gain_from_integral`]) and scale by the ray-traced
/// relative pattern `ray_gain(θ,φ) / ray_gain(0,0)`. This keeps ray tracing on the
/// same absolute scale as the standard PO path. The ray-tracing model itself is a stub
/// (see the warning emitted by the caller); only the normalization is corrected here.
fn compute_gain_ray_tracing(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    wavelength: f64,
    params: &IntegrationParams,
    warnings: &mut Vec<String>,
) -> ComputationResult<f64> {
    // Compute gain using ray tracing (relative pattern)
    let ray_gain_relative = compute_gain_ray_trace(config, theta, phi, wavelength);
    let on_axis_ray_gain = compute_gain_ray_trace(config, 0.0, 0.0, wavelength);

    let relative_gain = if on_axis_ray_gain > 1e-20 {
        ray_gain_relative / on_axis_ray_gain
    } else {
        return Err(ComputationError::NumericalInstability {
            operation: "compute_gain_ray_tracing".to_string(),
            reason: "On-axis ray trace field is zero or near-zero".to_string(),
        });
    };

    // Absolute boresight gain from the physical-optics aperture integral, used as the
    // anchor for the ray-traced relative pattern.
    let on_axis = integrate_aperture(0.0, 0.0, config, frequency_hz, params)?;

    if !on_axis.converged {
        warnings.push(INTEGRATION_NONCONVERGENCE_WARNING.to_string());
    }

    // theta = 0.0: this is the BORESIGHT anchor for the ray-traced relative pattern
    // (obliquity = 1 exactly). The stub's off-axis relative pattern deliberately does
    // NOT receive the obliquity factor — it is an acknowledged low-accuracy path
    // (roadmap P3: flagged on all endpoints via RAY_TRACING_STUB_WARNING).
    let boresight_gain =
        absolute_gain_from_integral(on_axis.field, 0.0, config, wavelength, params)?;

    Ok(boresight_gain * relative_gain)
}

/// Compute antenna gain in dB
///
/// Wrapper around `compute_gain` that returns the result in dB.
///
/// # Arguments
/// - `theta`: Polar angle (radians)
/// - `phi`: Azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `params`: Integration parameters
///
/// # Returns
/// Gain in dB (dBi) with warnings
pub fn compute_gain_db(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<GainComputationResult> {
    let result = compute_gain(theta, phi, config, frequency_hz, params)?;

    // result.gain already has floor applied from compute_gain, but double-check
    if result.gain <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "compute_gain_db".to_string(),
            reason: format!("Gain is non-positive: {}", result.gain),
        });
    }

    let gain_db = 10.0 * result.gain.log10();

    // Apply dB floor for numerical stability
    Ok(GainComputationResult {
        gain: apply_gain_floor_db(gain_db),
        warnings: result.warnings,
        spillover_loss_db: result.spillover_loss_db,
    })
}

/// Compute G/T ratio
///
/// The G/T ratio (gain-to-noise-temperature) is a key figure of merit for
/// receiving antennas:
/// ```text
/// G/T = G / T_sys  (linear)
/// G/T_dB = G_dB - 10·log₁₀(T_sys)
/// ```
///
/// # Arguments
/// - `theta`: Polar angle (radians)
/// - `phi`: Azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `temperature_k`: System noise temperature in Kelvin
/// - `params`: Integration parameters
///
/// # Returns
/// G/T ratio in dB/K
///
/// # Examples
/// ```no_run
/// use antenna_model::model::pattern::compute_g_over_t;
/// use antenna_model::model::integration::IntegrationParams;
/// # use antenna_model::model::geometry::{AntennaConfiguration, ReflectorGeometry, FeedParameters};
///
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
/// let g_over_t = compute_g_over_t(
///     0.0,                 // On-axis
///     0.0,
///     &config,
///     8.4e9,
///     50.0,                // 50K system temperature
///     &IntegrationParams::default(),
/// )?;
///
/// println!("G/T: {:.2} dB/K", g_over_t);
/// # Ok(())
/// # }
/// ```
pub fn compute_g_over_t(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    temperature_k: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    if temperature_k <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "compute_g_over_t".to_string(),
            reason: format!("Temperature must be positive, got {}", temperature_k),
        });
    }

    let result = compute_gain_db(theta, phi, config, frequency_hz, params)?;
    let gain_db = result.gain;

    // G/T in dB/K = G(dB) - 10·log₁₀(T)
    let g_over_t_db = gain_db - 10.0 * temperature_k.log10();

    Ok(g_over_t_db)
}

/// Compute beamwidth at specified gain drop from peak
///
/// Searches for the angle where the gain drops by the specified amount
/// from the peak (on-axis) gain.
///
/// # Arguments
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `gain_drop_db`: Gain drop from peak in dB (e.g., 3.0 for half-power beamwidth);
///   must be > 0
/// - `phi`: Azimuthal cut angle (radians, typically 0 for E-plane)
/// - `params`: Integration parameters
///
/// # Returns
/// Half-angle to the first −`gain_drop_db` crossing from boresight (radians).
///
/// # Algorithm
/// 1. **March outward** from boresight in steps of `0.1·λ/D` to bracket the FIRST
///    crossing of `peak − gain_drop_db`. This avoids locking onto a sidelobe crossing
///    that a naive bisection over `[0, π/4]` would produce.
/// 2. **Bisect** within the identified bracket `[theta_lo, theta_hi]` to 1e-5 rad
///    precision (30 iterations max).
///
/// # Errors
/// Returns [`ComputationError::NumericalInstability`] if no crossing is found within
/// π/4 (45°). This indicates either `gain_drop_db` is larger than the dynamic range
/// of the pattern within the search window, or the pattern is unusually broad.
pub fn compute_beamwidth(
    config: &AntennaConfiguration,
    frequency_hz: f64,
    gain_drop_db: f64,
    phi: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    const BISECTION_ITERS: usize = 30;
    const BISECTION_TOL_RAD: f64 = 1e-5;

    if gain_drop_db <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "compute_beamwidth".to_string(),
            reason: format!("gain_drop_db must be > 0, got {gain_drop_db}"),
        });
    }

    // On-axis peak gain and the target threshold.
    let result_peak = compute_gain_db(0.0, phi, config, frequency_hz, params)?;
    let target_gain = result_peak.gain - gain_drop_db;

    // Step size: 0.1·λ/D — small enough to resolve the main lobe without
    // overshooting into a sidelobe on the first step.
    let wavelength = wavelength_from_frequency(frequency_hz);
    let step = 0.1 * wavelength / config.reflector.diameter;

    // March outward from boresight to bracket the FIRST crossing.
    let mut theta_lo = 0.0_f64;
    let mut theta_hi = step;
    loop {
        if theta_hi > PI / 4.0 {
            return Err(ComputationError::NumericalInstability {
                operation: "compute_beamwidth".to_string(),
                reason: format!("no -{gain_drop_db} dB crossing found within 45 deg"),
            });
        }
        let g = compute_gain_db(theta_hi, phi, config, frequency_hz, params)?.gain;
        if g < target_gain {
            break;
        }
        theta_lo = theta_hi;
        theta_hi += step;
    }

    // Bisect within [theta_lo, theta_hi] to BISECTION_TOL_RAD precision.
    for _ in 0..BISECTION_ITERS {
        let mid = 0.5 * (theta_lo + theta_hi);
        let g = compute_gain_db(mid, phi, config, frequency_hz, params)?.gain;
        if g > target_gain {
            theta_lo = mid;
        } else {
            theta_hi = mid;
        }
        if theta_hi - theta_lo < BISECTION_TOL_RAD {
            break;
        }
    }

    Ok(0.5 * (theta_lo + theta_hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{
        FeedParameters, FeedPosition, MeshParameters, MeshPattern, ReflectorGeometry,
    };

    fn test_antenna() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.001).unwrap(); // 1m, f/D=0.5, 1mm RMS
        let feed_pos = FeedPosition::at_focus(0.5);
        let feed = FeedParameters::new(feed_pos, 8.0, 0.0, 1.0).unwrap();
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap();

        AntennaConfiguration::new(
            "test".to_string(),
            "Test Antenna".to_string(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap()
    }

    #[test]
    fn test_ruze_efficiency_perfect_surface() {
        let wavelength = 0.0357; // ~8.4 GHz

        // Perfect surface (RMS = 0)
        let efficiency = ruze_efficiency(0.0, wavelength);
        assert!((efficiency - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_ruze_efficiency_small_error() {
        let wavelength = 0.0357; // ~8.4 GHz

        // 1mm RMS error at 8.4 GHz
        // (4π × 0.001 / 0.0357)² ≈ 12.4, exp(-12.4) ≈ 0.88
        let efficiency = ruze_efficiency(0.001, wavelength);
        assert!(efficiency > 0.85);
        assert!(efficiency < 0.90);
    }

    #[test]
    fn test_ruze_efficiency_large_error() {
        let wavelength = 0.0357; // ~8.4 GHz

        // 10mm RMS error - should be poor at this frequency
        let efficiency = ruze_efficiency(0.010, wavelength);
        assert!(efficiency < 0.5);
        assert!(efficiency > 0.0);
    }

    #[test]
    fn test_ruze_efficiency_frequency_dependence() {
        let surface_rms = 0.005; // 5mm RMS

        // Higher frequency (shorter wavelength) = worse efficiency
        let eff_high_freq = ruze_efficiency(surface_rms, 0.01); // 30 GHz
        let eff_low_freq = ruze_efficiency(surface_rms, 0.1); // 3 GHz

        assert!(eff_low_freq > eff_high_freq);
    }

    #[test]
    fn test_overall_efficiency_no_mesh() {
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.001).unwrap();
        let feed_pos = FeedPosition::at_focus(0.5);
        let feed = FeedParameters::new(feed_pos, 8.0, 0.0, 1.0).unwrap();

        let config = AntennaConfiguration::new(
            "test".to_string(),
            "Test".to_string(),
            reflector,
            feed,
            None, // No mesh
        )
        .unwrap();

        let wavelength = 0.0357;
        let efficiency = overall_efficiency(&config, wavelength);

        // Should just be Ruze efficiency
        let expected = ruze_efficiency(0.001, wavelength);
        assert!((efficiency - expected).abs() < 1e-10);
    }

    #[test]
    fn test_overall_efficiency_with_mesh() {
        let config = test_antenna();
        let wavelength = 0.0357;

        let efficiency = overall_efficiency(&config, wavelength);

        // Should be product of Ruze efficiency and inductive-grid mesh reflection efficiency.
        // For 5mm mesh (spacing=0.005, wire_diameter=0.0005) at λ=0.0357m:
        //   log_term = ln(0.005 / (π × 0.0005)) = ln(3.183) ≈ 1.157
        //   X = (0.005/0.0357) × 1.157 ≈ 0.162
        //   |R|² = 1/(1 + 4×0.162²) ≈ 0.905
        let ruze = ruze_efficiency(0.001, wavelength);
        let mesh = crate::model::mesh::mesh_reflection_efficiency(0.005, 0.0005, wavelength);
        let expected = ruze * mesh;

        assert!((efficiency - expected).abs() < 1e-10);
    }

    #[test]
    fn test_theoretical_max_gain() {
        let diameter = 1.0; // 1m
        let wavelength = 0.0357; // ~8.4 GHz
        let aperture_eff = 0.55;

        let gain = theoretical_max_gain(diameter, wavelength, aperture_eff);

        // For 1m diameter at 8.4 GHz with 55% efficiency:
        // G ≈ 0.55 × (π × 1 / 0.0357)² ≈ 0.55 × 7765 ≈ 4271 ≈ 36.3 dB
        let gain_db = 10.0 * gain.log10();
        assert!(gain_db > 35.0 && gain_db < 37.0);
    }

    #[test]
    fn test_boresight_gain_reflects_taper_efficiency() {
        // No mesh, ideal surface → efficiency = 1, so boresight gain is pure aperture
        // directivity with the q=8 taper. Must be below the uniform-aperture max and
        // within a few dB of it (taper efficiency ~0.7-0.9).
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("t".into(), "T".into(), reflector, feed, None).unwrap();
        let params = IntegrationParams::default();
        let result = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        let wl = 0.0357_f64; // ~8.4 GHz
        let uniform_db = 10.0 * (4.0 * PI * (PI * 0.25) / (wl * wl)).log10(); // 4πA/λ², A=π(0.5)²
        assert!(
            result.gain < uniform_db,
            "taper must cost gain: {} vs {uniform_db}",
            result.gain
        );
        assert!(
            result.gain > uniform_db - 6.0,
            "taper loss implausibly large: {} vs {uniform_db}",
            result.gain
        );
    }

    #[test]
    fn test_compute_gain_positive() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let result = compute_gain(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        let gain = result.gain;

        // Gain should be positive
        assert!(gain > 0.0);

        // Boresight gain for the 1 m test dish (q=8 taper, 1 mm RMS, 5 mm mesh) at
        // 8.4 GHz is ~34.7 dBi from the aperture-directivity formula. In linear units
        // that is 10^(34.7/10) ≈ 2950. Bound it to [2000, 5000] (≈33.0–37.0 dBi):
        // a meaningful window around the derived value, not just "finite".
        assert!(gain > 2000.0, "boresight gain {gain} (linear) too low");
        assert!(gain < 5000.0, "boresight gain {gain} (linear) too high");
    }

    #[test]
    fn test_compute_gain_db_reasonable() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let result = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        let gain_db = result.gain;

        // Aperture-directivity formula: uniform-aperture max for a 1 m dish at 8.4 GHz
        // (λ≈0.0357 m) is 10·log10(4πA/λ²) ≈ 38.9 dBi. The q=8 taper (~1.5–2 dB),
        // 1 mm RMS Ruze (~0.5 dB) and mesh (~0.4 dB) losses give ~34.7 dBi.
        // Bound to [33.0, 37.0] dBi.
        assert!(gain_db > 33.0, "boresight gain {gain_db} dBi too low");
        assert!(gain_db < 37.0, "boresight gain {gain_db} dBi too high");
    }

    #[test]
    fn test_gain_decreases_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let result_on_axis = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        let gain_on_axis = result_on_axis.gain;
        let result_off_axis =
            compute_gain_db(5.0_f64.to_radians(), 0.0, &config, 8.4e9, &params).unwrap();
        let gain_off_axis = result_off_axis.gain;

        // Gain should decrease off-axis
        assert!(gain_off_axis < gain_on_axis);
    }

    #[test]
    fn test_compute_g_over_t() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let g_over_t = compute_g_over_t(0.0, 0.0, &config, 8.4e9, 50.0, &params).unwrap();

        // G/T = G(dB) − 10·log10(T). Boresight gain ≈ 34.7 dBi (see above),
        // T = 50 K → 10·log10(50) ≈ 17.0 dB, so G/T ≈ 17.7 dB/K.
        // Bound to [15.0, 20.0] dB/K.
        assert!(g_over_t > 15.0, "G/T {g_over_t} dB/K too low");
        assert!(g_over_t < 20.0, "G/T {g_over_t} dB/K too high");
    }

    #[test]
    fn test_compute_g_over_t_invalid_temperature() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let result = compute_g_over_t(0.0, 0.0, &config, 8.4e9, -10.0, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_beamwidth() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // Compute half-power beamwidth (3 dB drop from boresight)
        let hpbw = compute_beamwidth(&config, 8.4e9, 3.0, 0.0, &params).unwrap();

        // 1 m dish at 8.4 GHz: full HPBW ≈ 1.05–1.2·λ/D ≈ 2.1°–2.5°;
        // this function returns boresight→−3dB (half of that), widened by the
        // q=8 taper. Anything outside 0.9°–2.0° indicates a phase model bug.
        let half_deg = hpbw.to_degrees();
        assert!(
            half_deg > 0.9 && half_deg < 2.0,
            "boresight→-3dB half-angle = {half_deg}°; expected 0.9°–2.0°"
        );
    }

    /// AC#2 guard: when `gain_drop_db` is so large that the main lobe never drops
    /// that far within π/4, `compute_beamwidth` must return `Err` rather than a
    /// silent (wrong) number.
    ///
    /// 200 dB is far beyond any physically realistic pattern dynamic range, so the
    /// march never finds a crossing and the function returns `NumericalInstability`.
    #[test]
    fn test_compute_beamwidth_no_crossing_returns_err() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // 200 dB drop is unreachable within π/4 for any realistic aperture pattern.
        let result = compute_beamwidth(&config, 8.4e9, 200.0, 0.0, &params);
        assert!(
            result.is_err(),
            "expected Err for unreachable gain_drop_db=200, got Ok({:?})",
            result.ok()
        );
    }

    #[test]
    fn test_beamwidth_does_not_lock_on_sidelobe() {
        let config = test_antenna();
        let params = IntegrationParams::fast();
        let hpbw = compute_beamwidth(&config, 32e9, 3.0, 0.0, &params).unwrap();
        let first_null = 1.22 * wavelength_from_frequency(32e9) / 1.0; // diameter = 1.0 m
        assert!(
            hpbw < first_null,
            "beamwidth {hpbw} beyond first null {first_null}"
        );
    }

    #[test]
    fn test_beamwidth_rejects_nonpositive_drop() {
        let config = test_antenna();
        let params = IntegrationParams::fast();
        assert!(compute_beamwidth(&config, 8.4e9, 0.0, 0.0, &params).is_err());
    }

    #[test]
    fn test_adaptive_integration_selection() {
        let config = test_antenna();
        let base_params = IntegrationParams::default();

        // Near boresight (theta < 0.1) - should not use adaptive
        let params_near = select_integration_params(0.05, 0.0, &config, &base_params);
        assert_eq!(params_near.min_rho_points, base_params.min_rho_points);
        assert_eq!(params_near.min_phi_points, base_params.min_phi_points);

        // Near null region (theta > 0.1) - should use adaptive (doubled points)
        let params_null = select_integration_params(0.2, 0.0, &config, &base_params);
        assert_eq!(params_null.min_rho_points, base_params.min_rho_points * 2);
        assert_eq!(params_null.min_phi_points, base_params.min_phi_points * 2);
        assert_eq!(params_null.max_rho_points, base_params.max_rho_points * 2);
        assert_eq!(params_null.max_phi_points, base_params.max_phi_points * 2);
        assert!(
            (params_null.relative_tolerance - base_params.relative_tolerance / 2.0).abs() < 1e-10
        );
    }

    #[test]
    fn test_non_convergence_warning_propagated() {
        // END-TO-END (P10 Task 3): since the Hankel / mode integrator now carries the
        // runtime convergence self-check (D-6), a real production geometry CAN return
        // converged=false, and compute_gain must surface INTEGRATION_NONCONVERGENCE_WARNING.
        // Drive the full compute_gain path with a dish whose radial Nyquist rate at θ=90°
        // exceeds 2× the integrator's radial safety cap: the adaptive density clamps below
        // Nyquist (aliased) while the self-check's 2N leg samples finer, so the two
        // disagree and the result is flagged — never silently returned. A 750 m symmetric
        // dish at 40 GHz (D/λ = 1e5) sits well past the cap.
        let reflector = ReflectorGeometry::new(750.0, 375.0, 0.0).unwrap();
        let feed = FeedParameters::new(FeedPosition::at_focus(375.0), 2.0, 0.0, 1.0).unwrap();
        let config =
            AntennaConfiguration::new("huge".into(), "Huge".into(), reflector, feed, None).unwrap();
        let params = IntegrationParams::fast();

        let result = compute_gain_db(90f64.to_radians(), 0.0, &config, 40.0e9, &params).unwrap();

        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("did not converge")),
            "compute_gain must propagate the non-convergence warning end-to-end; got {:?}",
            result.warnings
        );
        assert_eq!(
            INTEGRATION_NONCONVERGENCE_WARNING,
            "aperture integration did not converge; gain accuracy may be degraded"
        );
    }

    #[test]
    fn test_spillover_applied_only_when_flagged() {
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(0.5) // f/D = 0.5
            .surface_rms(0.001)
            .build()
            .unwrap();
        let feed = FeedParameters::builder()
            .at_focus(0.5)
            .q_factor(8.0)
            .build()
            .unwrap();
        let config = AntennaConfiguration::builder()
            .id("spill")
            .name("Spill")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let analysis = analyze_edge_cases(&config, 0.0, 0.0);
        let expected_loss_db = 10.0 * (1.0 - analysis.spillover_fraction).log10();

        let mut params = IntegrationParams::fast();

        let base = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        assert!(base.spillover_loss_db.is_none());

        params.apply_spillover = true;
        let with = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        let reported = with.spillover_loss_db.expect("loss reported when applied");

        assert!(
            (reported - expected_loss_db).abs() < 1e-9,
            "reported {reported} vs {expected_loss_db}"
        );
        assert!(
            (with.gain - base.gain - expected_loss_db).abs() < 1e-6,
            "gain delta must equal reported loss"
        );
        // Over-tapered feeds (q=8, f/D=0.5) spill very little power past the rim, so the
        // modeled loss is small in magnitude — but it must be negative (it reduces gain)
        // and physically bounded. (The ~0.4-1 dB textbook spillover figure applies to
        // broad feeds q~2-4, not these highly-directive designs — see roadmap P1 finding.)
        assert!(
            reported < 0.0,
            "spillover loss must reduce gain: {reported}"
        );
        assert!(
            reported > -3.0,
            "spillover loss implausibly large: {reported}"
        );
    }

    #[test]
    fn test_spillover_not_applied_outside_standard_po() {
        // Large feed offset (0.35 lateral, f = 0.5 → offset ratio 0.7 > 0.5) routes to
        // RayTracing mode, where estimate_spillover's linear extrapolation is invalid.
        // Even with apply_spillover = true, no spillover must be folded in there.
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(0.5)
            .surface_rms(0.001)
            .build()
            .unwrap();
        let feed = FeedParameters::builder()
            .position(FeedPosition::new(0.35, 0.0, 0.5)) // 0.35 m from focus → ratio 0.7
            .q_factor(8.0)
            .build()
            .unwrap();
        let config = AntennaConfiguration::builder()
            .id("spill_large")
            .name("Spill Large Offset")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        // Confirm this geometry does NOT route through standard physical optics.
        let analysis = analyze_edge_cases(&config, 0.05, 0.0);
        assert_ne!(analysis.mode, ComputationMode::StandardPhysicalOptics);

        let mut params = IntegrationParams::fast();
        params.apply_spillover = true;
        let result = compute_gain_db(0.05, 0.0, &config, 8.4e9, &params).unwrap();
        assert!(
            result.spillover_loss_db.is_none(),
            "spillover must not be applied outside StandardPhysicalOptics, got {:?}",
            result.spillover_loss_db
        );
    }

    #[test]
    fn test_spillover_not_applied_in_moderate_offset_band_post_p2() {
        // P2 regression: removing the HigherOrderAberrations mode folded the 0.3f–0.5f
        // offset band into StandardPhysicalOptics. Spillover must STILL be excluded there
        // (offset ratio > SPILLOVER_MAX_OFFSET_RATIO = 0.3), because estimate_spillover's
        // small-offset approximation saturates to ~100% in this band and would clamp gain
        // to the degenerate −60 dB floor. This preserves P1's exact behavior: the spillover
        // regime did not widen when the mode was removed. (Guards against silently serving
        // a hypothetical uncalibrated 0.3f–0.5f antenna −60 dB garbage — no enabled antenna
        // is near this band, max served offset 0.027f.)
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(0.5)
            .surface_rms(0.001)
            .build()
            .unwrap();
        let feed = FeedParameters::builder()
            .position(FeedPosition::new(0.2, 0.0, 0.5)) // 0.2 m from focus → ratio 0.4
            .q_factor(8.0)
            .build()
            .unwrap();
        let config = AntennaConfiguration::builder()
            .id("spill_moderate")
            .name("Spill Moderate Offset")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        // Post-P2 this 0.4f offset routes through StandardPhysicalOptics...
        let analysis = analyze_edge_cases(&config, 0.05, 0.0);
        assert_eq!(analysis.mode, ComputationMode::StandardPhysicalOptics);
        assert!(analysis.feed_offset_ratio > SPILLOVER_MAX_OFFSET_RATIO);

        // ...but spillover must NOT be folded in (offset beyond estimate_spillover validity).
        let mut params = IntegrationParams::fast();
        params.apply_spillover = true;
        let result = compute_gain_db(0.05, 0.0, &config, 8.4e9, &params).unwrap();
        assert!(
            result.spillover_loss_db.is_none(),
            "spillover must not be applied in the 0.3f–0.5f band post-P2, got {:?}",
            result.spillover_loss_db
        );
    }

    /// P2 regression (0.3f–0.5f offset band): a moderate lateral feed offset now
    /// routes through `StandardPhysicalOptics` — the exact geometric coma phase
    /// (`phase::phase_feed_displacement`) covers this band, and the double-counting
    /// `HigherOrderAberrations` mode was removed. This test pins the exact-only
    /// (mode-removed) gain and confirms the routing.
    ///
    /// The pinned value DIFFERS from the pre-P2 mode output by construction: the
    /// removed Seidel terms were wrong-sign/wrong-scale/wrong-shape and had been
    /// added on top of the already-complete exact phase (see the completeness pin
    /// `edge_cases::exact_feed_displacement_phase_contains_all_low_order_aberrations`).
    /// That difference is the fix (hence the `PHYSICS_MODEL_VERSION` bump to 4).
    ///
    /// The wide-angle backlobe check also guards the P10 non-aliasing property: the
    /// off-axis integrator (shared by this path) must not produce the 20–35 dB-too-high
    /// plateau the retired aliasing 2D quadrature did.
    #[test]
    fn p2_moderate_offset_exact_only_gain_pinned_and_routes_standard_po() {
        // 3 m dish, f/D = 0.5, lateral feed offset 0.6 m → ratio 0.4, in the
        // (0.3f, 0.5f] band that formerly forced the removed HigherOrderAberrations mode.
        let reflector = ReflectorGeometry::builder()
            .diameter(3.0)
            .focal_length(1.5)
            .surface_rms(0.0005)
            .build()
            .unwrap();
        let feed = FeedParameters::builder()
            .position(FeedPosition::new(0.6, 0.0, 1.5))
            .q_factor(8.0)
            .build()
            .unwrap();
        let config = AntennaConfiguration::builder()
            .id("p2mod")
            .name("P2 moderate offset")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let freq = 8.4e9;
        let params = IntegrationParams::fast();

        // Confirm the 0.4f offset now routes through StandardPhysicalOptics (not the
        // removed higher-order mode, and not ray tracing which begins above 0.5f).
        let analysis = analyze_edge_cases(&config, 0.3, 0.0);
        assert_eq!(
            analysis.mode,
            ComputationMode::StandardPhysicalOptics,
            "0.4f offset must route to StandardPhysicalOptics post-P2, got {:?}",
            analysis.mode
        );

        let g = |deg: f64| {
            compute_gain_db(deg.to_radians(), 0.0, &config, freq, &params)
                .unwrap()
                .gain
        };

        // Pin the exact-only boresight gain. At this 0.4f offset the coma is severe enough
        // that boresight remains the pattern peak (the beam is degraded, not cleanly
        // steered) at ~16.05 dBi. This is the exact-coma result AFTER removing the
        // wrong-sign Seidel double-count — the value the served path produces for this
        // band, and by construction it differs from the pre-P2 mode output.
        let g0 = g(0.0);
        assert!(
            (g0 - 16.05).abs() < 0.30,
            "exact-only (mode-removed) boresight gain should be ~16.05 dBi, got {g0:.2}"
        );

        // P10 non-aliasing guard: the wide-angle backlobe stays far below the peak.
        let peak = [0.0_f64, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0]
            .into_iter()
            .map(g)
            .fold(f64::NEG_INFINITY, f64::max);
        let g90 = g(90.0);
        assert!(g90.is_finite(), "wide-angle gain must be finite, got {g90}");
        assert!(
            g90 < peak - 25.0,
            "90° gain {g90:.2} dBi must be >=25 dB below peak {peak:.2} dBi \
             (an aliased pattern would sit near the peak)"
        );
    }

    /// Shared config for the F7 sidelobe-floor tests: 1m/f0.5 dish, 1.5mm surface
    /// RMS, X-band feed. Surface RMS is nonzero so the floor is nonzero.
    fn sidelobe_floor_test_antenna() -> AntennaConfiguration {
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(0.5)
            .surface_rms(0.0015) // 1.5mm RMS
            .build()
            .unwrap();
        let feed = FeedParameters::builder()
            .at_focus(0.5)
            .q_factor(8.0)
            .build()
            .unwrap();
        AntennaConfiguration::builder()
            .id("sidelobe_floor_test")
            .name("Sidelobe Floor Test")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap()
    }

    #[test]
    fn test_sidelobe_floor_zero_at_zero_surface_rms() {
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(0.5)
            .surface_rms(0.0) // ideal surface
            .build()
            .unwrap();
        let feed = FeedParameters::builder()
            .at_focus(0.5)
            .q_factor(8.0)
            .build()
            .unwrap();
        let config = AntennaConfiguration::builder()
            .id("ideal")
            .name("Ideal")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let wavelength = wavelength_from_frequency(8.4e9);
        let floor = sidelobe_floor_gain(&config, wavelength);
        assert_eq!(
            floor, 0.0,
            "zero surface RMS must give an exactly-zero floor (no scattered power)"
        );
    }

    #[test]
    fn test_sidelobe_floor_positive_and_monotonic_in_surface_rms() {
        let wavelength = wavelength_from_frequency(8.4e9);

        let mk = |surface_rms: f64| {
            let reflector = ReflectorGeometry::builder()
                .diameter(1.0)
                .focal_length(0.5)
                .surface_rms(surface_rms)
                .build()
                .unwrap();
            let feed = FeedParameters::builder()
                .at_focus(0.5)
                .q_factor(8.0)
                .build()
                .unwrap();
            AntennaConfiguration::builder()
                .id("mono")
                .name("Mono")
                .reflector(reflector)
                .feed(feed)
                .build()
                .unwrap()
        };

        let config_small = mk(0.001); // 1mm RMS
        let config_large = mk(0.003); // 3mm RMS

        let floor_small = sidelobe_floor_gain(&config_small, wavelength);
        let floor_large = sidelobe_floor_gain(&config_large, wavelength);

        assert!(floor_small > 0.0, "floor must be positive for nonzero RMS");
        assert!(
            floor_large > floor_small,
            "floor must strictly increase with surface_rms: {floor_small} vs {floor_large}"
        );
    }

    /// F7 redesign (2026-07-16): the floor combines with the pattern as an incoherent
    /// POWER SUM (linear addition — scattered energy adds in power), not max(). Deep
    /// nulls are lifted to ~the floor; the main beam is perturbed by an amount that is
    /// negligible (< 0.001 dB) because pattern >> floor there — but NOT byte-identical,
    /// by design (the sum is continuous everywhere; no seam).
    #[test]
    fn test_sidelobe_floor_power_sum_lifts_deep_null_preserves_main_beam() {
        let config = sidelobe_floor_test_antenna();
        let params_off = IntegrationParams::fast();
        let params_on = IntegrationParams {
            apply_sidelobe_floor: true,
            ..IntegrationParams::fast()
        };

        let wavelength = wavelength_from_frequency(8.4e9);
        let floor = sidelobe_floor_gain(&config, wavelength);
        assert!(floor > 0.0, "floor must be positive for this config");

        // Deep off-axis angle: well past the main beam and first few sidelobes for
        // this 1m/8.4GHz dish (HPBW ~2.5deg), pattern gain << floor here.
        let theta_deep_null = 10.0_f64.to_radians();
        let off = compute_gain(theta_deep_null, 0.0, &config, 8.4e9, &params_off).unwrap();
        let on = compute_gain(theta_deep_null, 0.0, &config, 8.4e9, &params_on).unwrap();

        assert!(
            off.gain < floor,
            "pattern at deep-null angle must be below the floor for this test to be meaningful: \
             pattern={} floor={floor}",
            off.gain
        );

        // Exact linear power-sum identity.
        assert!(
            (on.gain - (off.gain + floor)).abs() <= 1e-12 * (off.gain + floor),
            "flag-on gain must equal pattern + floor (linear power sum): on={} expected={}",
            on.gain,
            off.gain + floor
        );
        assert!(on.gain > off.gain, "deep null must be lifted by the floor");

        // Main beam: pattern >> floor, so the sum perturbs the dB value negligibly.
        for theta_deg in [0.0_f64, 1.0] {
            let theta = theta_deg.to_radians();
            let off = compute_gain(theta, 0.0, &config, 8.4e9, &params_off).unwrap();
            let on = compute_gain(theta, 0.0, &config, 8.4e9, &params_on).unwrap();
            assert!(
                (on.gain - (off.gain + floor)).abs() <= 1e-12 * on.gain,
                "sum identity must hold in the main beam too (theta={theta_deg}deg)"
            );
            let delta_db = 10.0 * (on.gain / off.gain).log10();
            assert!(
                delta_db < 1e-3,
                "main-beam perturbation must be < 0.001 dB, got {delta_db} dB \
                 (theta={theta_deg}deg)"
            );
        }
    }

    /// F7 redesign rear-hemisphere policy (maintainer 2026-07-16): behind the dish
    /// (theta > 90 deg) the aperture-PO term is categorically invalid, so with the floor
    /// active the returned gain is the floor ALONE — and the (pathologically expensive,
    /// discarded-anyway) rear aperture integration is skipped entirely. With the flag
    /// off, rear queries keep returning raw PO (callers gate honesty via the
    /// rear-hemisphere warning).
    #[test]
    fn test_sidelobe_floor_rear_hemisphere_is_floor_only() {
        let config = sidelobe_floor_test_antenna();
        let params_on = IntegrationParams {
            apply_sidelobe_floor: true,
            ..IntegrationParams::fast()
        };
        let wavelength = wavelength_from_frequency(8.4e9);
        let floor = sidelobe_floor_gain(&config, wavelength);
        assert!(floor > 0.0);

        for theta_deg in [90.5_f64, 120.0, 163.0, 180.0] {
            let theta = theta_deg.to_radians();
            let on = compute_gain(theta, 0.0, &config, 8.4e9, &params_on).unwrap();
            assert_eq!(
                on.gain, floor,
                "rear gain (theta={theta_deg}deg) must be exactly the floor (PO excluded)"
            );
            assert!(
                on.spillover_loss_db.is_none(),
                "no PO term behind the dish => no spillover loss to report"
            );
        }

        // Boundary: theta = 90 deg exactly is FORWARD (power sum, PO included) — matches
        // the service rear-warning gate, which fires only STRICTLY beyond 90 deg.
        let at_90 = compute_gain(90.0_f64.to_radians(), 0.0, &config, 8.4e9, &params_on).unwrap();
        assert!(
            at_90.gain >= floor,
            "at exactly 90deg the power sum must include the floor term"
        );
    }

    #[test]
    fn test_sidelobe_floor_flag_off_matches_pre_f7_behavior() {
        // Flag defaults to false; compute_gain's returned gain must equal the
        // pre-F7 pipeline exactly (raw standard-PO gain, then apply_gain_floor —
        // no spillover, since apply_spillover is also false here, and no sidelobe
        // floor). Reconstructed independently via the private helpers rather than
        // by re-deriving compute_gain, so this test would actually fail if the
        // floor seam were wired in unconditionally.
        let config = sidelobe_floor_test_antenna();
        let params = IntegrationParams::fast();
        assert!(
            !params.apply_sidelobe_floor,
            "apply_sidelobe_floor must default to false"
        );
        assert!(!params.apply_spillover);

        let wavelength = wavelength_from_frequency(8.4e9);

        // Deep-null angle included: this is exactly where a wrongly-unconditional
        // floor would diverge from the pre-F7 pipeline.
        for theta_deg in [0.0_f64, 1.0, 5.0, 10.0, 20.0] {
            let theta = theta_deg.to_radians();

            let mut warnings = Vec::new();
            let raw_gain = compute_gain_standard(
                theta,
                0.0,
                &config,
                8.4e9,
                wavelength,
                &params,
                &mut warnings,
            )
            .unwrap();
            let expected = apply_gain_floor(raw_gain);

            let result = compute_gain(theta, 0.0, &config, 8.4e9, &params).unwrap();

            assert_eq!(
                result.gain, expected,
                "theta={theta_deg}deg: flag-off gain must equal the pre-F7 pipeline exactly"
            );
        }
    }

    #[test]
    fn test_edge_case_warnings_propagation() {
        use crate::model::geometry::FeedPosition;

        // Create antenna with a severe feed offset to trigger warnings.
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(1.0)
            .surface_rms(0.001)
            .build()
            .unwrap();

        // Feed displaced by 0.6m (0.6f) — exceeds the 0.5f severe threshold, so the
        // edge-case analysis emits a ray-tracing feed-offset warning that must propagate
        // through compute_gain. (Pre-P2 this test used a 0.4f offset to trigger the
        // removed HigherOrderAberrations warning; that band is now warning-free.)
        let feed = FeedParameters::new(FeedPosition::new(0.6, 0.0, 1.0), 8.0, 0.0, 1.0).unwrap();

        let config = AntennaConfiguration::builder()
            .id("test_warnings")
            .name("Test Warnings")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let params = IntegrationParams::fast();

        // Compute gain and check that warnings are included
        let result = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Should have warnings about large feed offset
        assert!(
            !result.warnings.is_empty(),
            "Expected warnings for large feed offset, but got none"
        );

        // Check that warning mentions feed offset
        let has_offset_warning = result
            .warnings
            .iter()
            .any(|w| w.contains("offset") || w.contains("aberration"));
        assert!(
            has_offset_warning,
            "Expected warning about feed offset or aberrations, got: {:?}",
            result.warnings
        );

        // Gain value should still be a valid dB number (no NaN/inf, and above the floor).
        // With a 0.4f feed displacement the boresight gain is significantly reduced and may
        // legitimately fall below 0 dBi, so we only check the floor and upper bound.
        // (The old assertion "> 0.0 dBi" was only valid under the wrong spurious-defocus phase.)
        assert!(result.gain.is_finite());
        assert!(result.gain >= -60.0); // Must be at or above the gain floor
        assert!(result.gain < 100.0); // Reasonable upper bound
    }
}
