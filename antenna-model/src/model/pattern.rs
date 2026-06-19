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
//!
//! # References
//! - Design doc Section 2.1 (Core Physical Optics Model)
//! - Design doc Section 2.4 (Mesh Reflector Efficiency)
//! - Implementation plan Sprint 2, Task 2.5

use std::f64::consts::PI;

use num_complex::Complex64;

/// Warning message emitted when the aperture integration loop exhausts its
/// iteration budget without meeting the convergence criterion.  Extracted as a
/// constant so the text stays consistent across all four gain-computation helpers
/// and the existing test can rely on `.contains("did not converge")`.
const INTEGRATION_NONCONVERGENCE_WARNING: &str =
    "aperture integration did not converge; gain accuracy may be degraded";

use crate::error::{ComputationError, ComputationResult};
use crate::model::{
    direct_path::compute_with_direct_path,
    edge_cases::{
        analyze_edge_cases, apply_gain_floor, apply_gain_floor_db, needs_adaptive_integration,
        ComputationMode,
    },
    geometry::AntennaConfiguration,
    integration::{
        far_field_normalization, integrate_amplitude_squared, integrate_aperture, IntegrationParams,
    },
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
    let gain = match analysis.mode {
        ComputationMode::StandardPhysicalOptics => {
            compute_gain_standard(theta, phi, config, frequency_hz, wavelength, params, &mut warnings)?
        }
        ComputationMode::HigherOrderAberrations => {
            tracing::debug!(
                "Using higher-order aberrations mode (feed offset ratio: {:.3})",
                analysis.feed_offset_ratio
            );
            compute_gain_higher_order(theta, phi, config, frequency_hz, wavelength, params, &mut warnings)?
        }
        ComputationMode::RayTracing => {
            // Ray tracing is a stub: aperture sampling is used but true spillover and
            // geometric ray-reflector intersection are not fully implemented.
            warnings.push(
                "WARNING: Ray tracing for large feed offsets (>0.5f) is not fully implemented; \
                 gain accuracy may be degraded."
                    .to_string(),
            );
            compute_gain_ray_tracing(theta, phi, config, frequency_hz, wavelength, params, &mut warnings)?
        }
        ComputationMode::NearBoresightDirectPath => {
            compute_gain_direct_path(theta, phi, config, frequency_hz, wavelength, params, &mut warnings)?
        }
    };

    // Apply gain floor for numerical stability
    let gain = apply_gain_floor(gain);

    Ok(GainComputationResult { gain, warnings })
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
fn absolute_gain_from_integral(
    raw_field: Complex64,
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
    let directivity = 4.0 * PI / (wavelength * wavelength) * raw_field.norm_sqr() / amp_sq;
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

    absolute_gain_from_integral(result.field, config, wavelength, &effective_params)
}

/// Higher-order aberrations gain computation for moderate feed offsets
///
/// Uses the same approach as standard physical optics but includes
/// explicit Seidel aberration terms (astigmatism, field curvature, distortion)
/// in the phase computation.
fn compute_gain_higher_order(
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

    // Create integration params with higher-order aberrations enabled
    let ho_params = IntegrationParams {
        use_higher_order_aberrations: true,
        ..effective_params
    };

    // Raw aperture integral with higher-order aberrations included in the phase.
    let result = integrate_aperture(theta, phi, config, frequency_hz, &ho_params)?;

    if !result.converged {
        warnings.push(INTEGRATION_NONCONVERGENCE_WARNING.to_string());
    }

    absolute_gain_from_integral(result.field, config, wavelength, &ho_params)
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

    let boresight_gain = absolute_gain_from_integral(on_axis.field, config, wavelength, params)?;

    Ok(boresight_gain * relative_gain)
}

/// Direct path interference gain computation for near-boresight scenarios
fn compute_gain_direct_path(
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

    // Raw reflected-only aperture integral I = ∬ A e^{jΨ} ρ dρ dφ'. Its directivity is
    // the absolute reflected-path gain.
    let reflected = integrate_aperture(theta, phi, config, frequency_hz, &effective_params)?;

    if !reflected.converged {
        warnings.push(INTEGRATION_NONCONVERGENCE_WARNING.to_string());
    }

    let reflected_gain =
        absolute_gain_from_integral(reflected.field, config, wavelength, &effective_params)?;

    // The direct-path module combines a *normalized* reflected far field with a direct
    // contribution. We apply its effect as a dimensionless ratio
    // |total|² / |reflected|², which is invariant to the (shared) normalization, then
    // scale the absolute reflected-path gain by it. This keeps the direct-path result on
    // the same absolute (directivity) scale as the standard PO path.
    //
    // Derive the normalized field from the already-computed `reflected.field` to avoid
    // running the aperture integral a second time with identical arguments.
    let e_reflected_normalized = far_field_normalization(wavelength) * reflected.field;
    let direct = compute_with_direct_path(config, theta, phi, wavelength, e_reflected_normalized);

    let reflected_norm_sq = e_reflected_normalized.norm_sqr();
    let direct_path_factor = if reflected_norm_sq > 1e-30 {
        direct.total_field.norm_sqr() / reflected_norm_sq
    } else {
        1.0
    };

    Ok(reflected_gain * direct_path_factor)
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
/// - `gain_drop_db`: Gain drop from peak in dB (e.g., 3.0 for half-power beamwidth)
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

    // Bisect within [theta_lo, theta_hi] to ~1e-5 rad precision.
    for _ in 0..30 {
        let mid = 0.5 * (theta_lo + theta_hi);
        let g = compute_gain_db(mid, phi, config, frequency_hz, params)?.gain;
        if g > target_gain {
            theta_lo = mid;
        } else {
            theta_hi = mid;
        }
        if theta_hi - theta_lo < 1e-5 {
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
        // Force non-convergence by capping to a single iteration with an impossible
        // tolerance.  compute_gain must include a warning containing "did not converge"
        // in the returned warnings vec.
        let config = test_antenna();
        let params = IntegrationParams {
            max_iterations: 1,
            relative_tolerance: 1e-15,
            ..IntegrationParams::fast()
        };
        // Use an off-boresight angle to ensure the standard PO path is exercised.
        let result = compute_gain(0.3, 0.0, &config, 8.4e9, &params).unwrap();
        let has_convergence_warning = result
            .warnings
            .iter()
            .any(|w| w.contains("did not converge"));
        assert!(
            has_convergence_warning,
            "Expected a convergence warning but got: {:?}",
            result.warnings
        );
    }

    #[test]
    fn test_edge_case_warnings_propagation() {
        use crate::model::geometry::FeedPosition;

        // Create antenna with large feed offset to trigger warnings
        let reflector = ReflectorGeometry::builder()
            .diameter(1.0)
            .focal_length(1.0)
            .surface_rms(0.001)
            .build()
            .unwrap();

        // Feed displaced by 0.4m (0.4f) - should trigger higher-order aberrations warning
        let feed = FeedParameters::new(FeedPosition::new(0.4, 0.0, 1.0), 8.0, 0.0, 1.0).unwrap();

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
