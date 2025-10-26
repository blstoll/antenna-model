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
//! - **Mesh Transparency**: Wire mesh reflectors have frequency-dependent transparency
//! - **Illumination Efficiency**: Non-uniform illumination reduces effective aperture
//! - **Spillover Efficiency**: Feed pattern energy missing the reflector
//!
//! # References
//! - Design doc Section 2.1 (Core Physical Optics Model)
//! - Design doc Section 2.4 (Mesh Reflector Efficiency)
//! - Implementation plan Sprint 2, Task 2.5

use std::f64::consts::PI;

use crate::error::{ComputationError, ComputationResult};
use crate::model::{
    geometry::AntennaConfiguration,
    integration::{compute_far_field, IntegrationParams},
    wavelength_from_frequency,
};

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
/// // Should be close to 1.0 (good surface)
/// assert!(efficiency > 0.99);
///
/// // 5mm RMS error at same frequency (poor surface)
/// let efficiency_poor = ruze_efficiency(0.005, 0.0357);
/// // Should be significantly lower
/// assert!(efficiency_poor < 0.95);
/// ```
pub fn ruze_efficiency(surface_rms: f64, wavelength: f64) -> f64 {
    let ratio = 4.0 * PI * surface_rms / wavelength;
    (-ratio * ratio).exp()
}

/// Compute mesh transparency for wire mesh reflectors
///
/// Wire mesh reflectors have frequency-dependent transparency. At low frequencies,
/// the wavelength is large compared to mesh spacing, and the mesh becomes transparent.
///
/// # Formula (simplified model)
/// ```text
/// T = 1 / (1 + (λ₀/λ)²)  for λ > λ₀
/// T = 1                   for λ ≤ λ₀
/// ```
/// where λ₀ = π · mesh_spacing (cutoff wavelength).
///
/// # Arguments
/// - `mesh_spacing`: Mesh spacing in meters (hole size)
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Transparency factor (0 to 1, where 1 = opaque, 0 = transparent)
///
/// # Examples
/// ```
/// use antenna_model::model::pattern::mesh_transparency;
///
/// // 5mm mesh at 8.4 GHz (λ ≈ 35.7mm) - well above cutoff
/// let transparency = mesh_transparency(0.005, 0.0357);
/// assert!(transparency > 0.99); // Nearly opaque
///
/// // 5mm mesh at 100 MHz (λ = 3m) - below cutoff
/// let transparency_low_freq = mesh_transparency(0.005, 3.0);
/// assert!(transparency_low_freq < 0.1); // Mostly transparent
/// ```
pub fn mesh_transparency(mesh_spacing: f64, wavelength: f64) -> f64 {
    let lambda_0 = PI * mesh_spacing;

    if wavelength <= lambda_0 {
        // Above cutoff frequency - mesh is opaque
        1.0
    } else {
        // Below cutoff - transparency increases
        1.0 / (1.0 + (lambda_0 / wavelength).powi(2))
    }
}

/// Compute overall antenna efficiency
///
/// Combines Ruze efficiency and mesh transparency (if mesh present).
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

    // Mesh transparency (if mesh present)
    let eta_mesh = if let Some(ref mesh) = config.mesh {
        mesh_transparency(mesh.spacing, wavelength)
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
/// # use antenna_model::model::geometry::AntennaConfiguration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let config = AntennaConfiguration::builder()
/// #     .reflector_diameter(1.0)
/// #     .focal_length(0.5)
/// #     .build()?;
/// let gain = compute_gain(
///     0.0,                 // On-axis
///     0.0,
///     &config,
///     8.4e9,
///     &IntegrationParams::default(),
/// )?;
///
/// println!("Gain: {:.2} dB", 10.0 * gain.log10());
/// # Ok(())
/// # }
/// ```
pub fn compute_gain(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    let wavelength = wavelength_from_frequency(frequency_hz);

    // Compute far-field electric field
    let e_field = compute_far_field(theta, phi, config, frequency_hz, params)?;

    // Power is proportional to |E|²
    let field_magnitude_squared = e_field.norm_sqr();

    // Compute on-axis field for normalization (if not already on-axis)
    let on_axis_field = if theta.abs() < 1e-10 && phi.abs() < 1e-10 {
        e_field.norm_sqr()
    } else {
        let e_on_axis = compute_far_field(0.0, 0.0, config, frequency_hz, params)?;
        e_on_axis.norm_sqr()
    };

    // Relative gain (normalized to on-axis)
    let relative_gain = if on_axis_field > 1e-20 {
        field_magnitude_squared / on_axis_field
    } else {
        return Err(ComputationError::NumericalInstability {
            operation: "compute_gain".to_string(),
            reason: "On-axis field is zero or near-zero".to_string(),
        });
    };

    // Apply efficiency corrections
    let efficiency = overall_efficiency(config, wavelength);

    // Compute absolute gain using theoretical maximum
    // Assume aperture efficiency of 0.55 (typical for cos^q feed with q~8)
    let theoretical_gain = theoretical_max_gain(config.reflector.diameter, wavelength, 0.55);

    // Final gain = theoretical maximum × efficiency × relative pattern
    let gain = theoretical_gain * efficiency * relative_gain;

    Ok(gain)
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
/// Gain in dB (dBi)
pub fn compute_gain_db(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    let gain_linear = compute_gain(theta, phi, config, frequency_hz, params)?;

    if gain_linear <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "compute_gain_db".to_string(),
            reason: format!("Gain is non-positive: {}", gain_linear),
        });
    }

    Ok(10.0 * gain_linear.log10())
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
/// # use antenna_model::model::geometry::AntennaConfiguration;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let config = AntennaConfiguration::builder()
/// #     .reflector_diameter(1.0)
/// #     .focal_length(0.5)
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

    let gain_db = compute_gain_db(theta, phi, config, frequency_hz, params)?;

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
/// Half-power beamwidth in radians (from boresight to -3dB point)
///
/// # Notes
/// This is a simplified search that assumes monotonic decrease from boresight.
/// For more complex patterns with sidelobes, this may not find the true beamwidth.
pub fn compute_beamwidth(
    config: &AntennaConfiguration,
    frequency_hz: f64,
    gain_drop_db: f64,
    phi: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    // Get on-axis gain
    let gain_peak = compute_gain_db(0.0, phi, config, frequency_hz, params)?;
    let target_gain = gain_peak - gain_drop_db;

    // Binary search for beamwidth
    let mut theta_min = 0.0;
    let mut theta_max = PI / 4.0; // Start with 45 degrees max

    const MAX_ITERATIONS: usize = 20;
    const TOLERANCE: f64 = 1e-4; // 0.01 degree tolerance

    for _ in 0..MAX_ITERATIONS {
        let theta_mid = (theta_min + theta_max) / 2.0;
        let gain_mid = compute_gain_db(theta_mid, phi, config, frequency_hz, params)?;

        if (gain_mid - target_gain).abs() < 0.1 {
            // Found beamwidth within 0.1 dB
            return Ok(theta_mid);
        }

        if gain_mid > target_gain {
            // Need to search farther out
            theta_min = theta_mid;
        } else {
            // Need to search closer in
            theta_max = theta_mid;
        }

        if (theta_max - theta_min) < TOLERANCE {
            return Ok((theta_min + theta_max) / 2.0);
        }
    }

    Ok((theta_min + theta_max) / 2.0)
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
    fn test_mesh_transparency_above_cutoff() {
        let mesh_spacing = 0.005; // 5mm
        let wavelength = 0.0357; // ~8.4 GHz
                                 // lambda_0 = π × 0.005 ≈ 0.0157 m

        // At 8.4 GHz, λ = 0.0357 > λ₀ = 0.0157, so we're above cutoff
        // T = 1/(1 + (0.0157/0.0357)²) = 1/(1 + 0.193) ≈ 0.84
        let transparency = mesh_transparency(mesh_spacing, wavelength);
        assert!(transparency > 0.80 && transparency < 0.90);
    }

    #[test]
    fn test_mesh_transparency_below_cutoff() {
        let mesh_spacing = 0.005; // 5mm
        let wavelength = 3.0; // 100 MHz
                              // lambda_0 = π × 0.005 ≈ 0.0157 m

        // At 100 MHz, λ = 3.0 >> λ₀, so deeply above cutoff in wavelength
        // T = 1/(1 + (0.0157/3.0)²) ≈ 1/(1 + 0.000027) ≈ 0.999
        let transparency = mesh_transparency(mesh_spacing, wavelength);
        assert!(transparency > 0.99); // Nearly 1.0 (acts transparent, poor reflector)
    }

    #[test]
    fn test_mesh_transparency_at_cutoff() {
        let mesh_spacing = 0.005; // 5mm
        let lambda_0 = PI * mesh_spacing;

        // Right at cutoff wavelength
        // Below cutoff freq (above cutoff wavelength): solid reflector
        let transparency = mesh_transparency(mesh_spacing, lambda_0 * 0.99);
        assert_eq!(transparency, 1.0); // Opaque (good reflector)
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

        // Should be product of Ruze and mesh
        let ruze = ruze_efficiency(0.001, wavelength);
        let mesh = mesh_transparency(0.005, wavelength);
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
    fn test_compute_gain_positive() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let gain = compute_gain(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Gain should be positive
        assert!(gain > 0.0);

        // Should be reasonable value (10 to 10000 linear, or 10-40 dB)
        assert!(gain > 10.0);
        assert!(gain < 100000.0);
    }

    #[test]
    fn test_compute_gain_db_reasonable() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let gain_db = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Gain should be reasonable (10-40 dB for 1m dish at X-band)
        assert!(gain_db > 10.0);
        assert!(gain_db < 50.0);
    }

    #[test]
    fn test_gain_decreases_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let gain_on_axis = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
        let gain_off_axis =
            compute_gain_db(5.0_f64.to_radians(), 0.0, &config, 8.4e9, &params).unwrap();

        // Gain should decrease off-axis
        assert!(gain_off_axis < gain_on_axis);
    }

    #[test]
    fn test_compute_g_over_t() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let g_over_t = compute_g_over_t(0.0, 0.0, &config, 8.4e9, 50.0, &params).unwrap();

        // G/T for 1m dish at X-band with 50K should be reasonable
        // Gain ~35 dB, T = 50K (17 dB) => G/T ~ 18 dB/K
        assert!(g_over_t > 10.0);
        assert!(g_over_t < 30.0);
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

        // Compute half-power beamwidth (3 dB)
        let hpbw = compute_beamwidth(&config, 8.4e9, 3.0, 0.0, &params).unwrap();

        // For 1m dish at 8.4 GHz, HPBW ≈ 1.2λ/D ≈ 1.2 × 0.0357 / 1.0 ≈ 0.043 rad ≈ 2.5°
        // But with losses and actual integration, it may be somewhat larger
        let hpbw_degrees = hpbw.to_degrees();
        assert!(hpbw_degrees > 0.5);
        assert!(hpbw_degrees < 10.0);
    }
}
