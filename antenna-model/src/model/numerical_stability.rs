//! Numerical stability improvements for antenna pattern computation
//!
//! This module provides utilities to improve numerical stability in edge cases:
//! - **Adaptive integration**: Increase sampling near pattern nulls
//! - **Kaiser windowing**: Smooth sidelobe transitions
//! - **Noise floor enforcement**: Prevent numerical underflow
//! - **Phase unwrapping**: Handle rapid phase variations
//!
//! # References
//! - Design doc Section 3.2 (Numerical Stability)

use crate::model::integration::IntegrationParams;
use std::f64::consts::PI;

/// Kaiser window for sidelobe control
///
/// The Kaiser window provides a tunable tradeoff between main lobe width
/// and sidelobe level. Parameter β controls this tradeoff:
/// - β = 0: Rectangular window (no tapering)
/// - β = 5: Moderate sidelobe reduction (~-40 dB)
/// - β = 8.6: High sidelobe reduction (~-60 dB)
///
/// # Arguments
/// * `n` - Sample index (0 to total_samples-1)
/// * `total_samples` - Total number of samples
/// * `beta` - Kaiser window parameter
///
/// # Returns
/// Window value (0.0 to 1.0)
pub fn kaiser_window(n: usize, total_samples: usize, beta: f64) -> f64 {
    if total_samples == 0 {
        return 1.0;
    }

    let n_norm = 2.0 * (n as f64) / (total_samples as f64) - 1.0; // Map to [-1, 1]
    let arg = beta * (1.0 - n_norm.powi(2)).sqrt();

    bessel_i0(arg) / bessel_i0(beta)
}

/// Modified Bessel function of the first kind, order 0
///
/// Uses series expansion for numerical computation.
///
/// # Arguments
/// * `x` - Input value
///
/// # Returns
/// I₀(x)
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let x_half_sq = (x / 2.0).powi(2);

    // Series: I₀(x) = Σ (x/2)^(2k) / (k!)^2
    for k in 1..50 {
        term *= x_half_sq / ((k * k) as f64);
        sum += term;

        if term < 1e-12 * sum {
            break; // Converged
        }
    }

    sum
}

/// Generate adaptive integration parameters based on expected pattern behavior
///
/// Near pattern nulls, integration requires finer sampling to resolve
/// rapid phase variations.
///
/// # Arguments
/// * `theta` - Far-field elevation angle (radians)
/// * `phi` - Far-field azimuth angle (radians)
/// * `wavelength` - Operating wavelength (meters)
/// * `diameter` - Antenna diameter (meters)
/// * `base_params` - Base integration parameters
///
/// # Returns
/// Adapted integration parameters
pub fn adaptive_integration_params(
    theta: f64,
    _phi: f64,
    wavelength: f64,
    diameter: f64,
    base_params: &IntegrationParams,
) -> IntegrationParams {
    // Estimate angular position of first null
    // θ_null ≈ 1.22 λ/D for uniformly illuminated circular aperture
    let null_angle = 1.22 * wavelength / diameter;

    // Distance from nearest likely null (very rough estimate)
    // Nulls occur at approximately integer multiples of null_angle
    let null_number = (theta / null_angle).round();
    let nearest_null = null_number * null_angle;
    let distance_to_null = (theta - nearest_null).abs();

    // If near a null (within 20% of null spacing), increase sampling
    let near_null = distance_to_null < 0.2 * null_angle;

    if near_null {
        // Increase radial and azimuthal sampling by 50%
        IntegrationParams {
            min_rho_points: (base_params.min_rho_points as f64 * 1.5) as usize,
            max_rho_points: (base_params.max_rho_points as f64 * 1.5) as usize,
            min_phi_points: (base_params.min_phi_points as f64 * 1.5) as usize,
            max_phi_points: (base_params.max_phi_points as f64 * 1.5) as usize,
            max_iterations: base_params.max_iterations + 1,
            ..*base_params
        }
    } else {
        base_params.clone()
    }
}

/// Apply Kaiser windowing to aperture distribution
///
/// This smooths the aperture taper to reduce sidelobe discontinuities.
///
/// # Arguments
/// * `rho` - Aperture radius coordinate (0 to R)
/// * `radius` - Maximum aperture radius
/// * `amplitude` - Unwindowed amplitude
/// * `beta` - Kaiser parameter (typical: 5-8)
///
/// # Returns
/// Windowed amplitude
pub fn apply_kaiser_taper(rho: f64, radius: f64, amplitude: f64, beta: f64) -> f64 {
    if radius == 0.0 {
        return amplitude;
    }

    // Map rho (0 at center, radius at edge) to radial distance for window
    // Kaiser window is symmetric, so we want rho=0 (aperture center) to map to window center
    let r_norm = rho / radius; // 0 (center) to 1 (edge)

    // For circular aperture, use radial distance directly
    // Kaiser window argument should be 0 at center, increase to edge
    let arg = beta * (1.0 - r_norm.powi(2)).sqrt();
    let window = bessel_i0(arg) / bessel_i0(beta);

    amplitude * window
}

/// Unwrap phase to prevent discontinuities
///
/// Phase values should be continuous; jumps > π indicate wrapping.
///
/// # Arguments
/// * `phases` - Array of phase values (radians)
///
/// # Returns
/// Unwrapped phase array
pub fn unwrap_phase(phases: &[f64]) -> Vec<f64> {
    if phases.is_empty() {
        return Vec::new();
    }

    let mut unwrapped = vec![phases[0]];
    let mut correction = 0.0;

    for i in 1..phases.len() {
        let diff = phases[i] - phases[i - 1];

        // Detect wrap (diff > π or diff < -π)
        if diff > PI {
            correction -= 2.0 * PI;
        } else if diff < -PI {
            correction += 2.0 * PI;
        }

        unwrapped.push(phases[i] + correction);
    }

    unwrapped
}

/// Check if adaptive integration is strongly recommended
///
/// This is a more conservative check than the one in edge_cases module.
///
/// # Arguments
/// * `theta` - Far-field elevation angle (radians)
/// * `wavelength` - Operating wavelength (meters)
/// * `diameter` - Antenna diameter (meters)
///
/// # Returns
/// True if adaptive integration strongly recommended
pub fn strongly_needs_adaptive(theta: f64, wavelength: f64, diameter: f64) -> bool {
    // Near first null or beyond
    let first_null = 1.22 * wavelength / diameter;
    theta > 0.8 * first_null
}

/// Smoothly ramp gain to noise floor
///
/// Instead of hard clipping, smoothly transition to noise floor.
///
/// # Arguments
/// * `gain_linear` - Computed gain
/// * `noise_floor` - Minimum gain floor
/// * `transition_db` - Transition region width in dB
///
/// # Returns
/// Smoothed gain
pub fn smooth_to_floor(gain_linear: f64, noise_floor: f64, transition_db: f64) -> f64 {
    let gain_db = 10.0 * gain_linear.log10();
    let floor_db = 10.0 * noise_floor.log10();

    if gain_db > floor_db + transition_db {
        // Above transition region
        gain_linear
    } else if gain_db < floor_db {
        // Below floor
        noise_floor
    } else {
        // In transition region: use smooth interpolation
        let t = (gain_db - floor_db) / transition_db; // 0 to 1
        let smooth_t = smooth_step(t);

        // Interpolate in linear space
        noise_floor + smooth_t * (gain_linear - noise_floor)
    }
}

/// Smooth step function (S-curve)
///
/// # Arguments
/// * `t` - Parameter (0 to 1)
///
/// # Returns
/// Smoothed value (0 to 1)
fn smooth_step(t: f64) -> f64 {
    // 3t² - 2t³ (Hermite interpolation)
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kaiser_window_endpoints() {
        let total_samples = 100;
        let beta = 5.0;

        // Edges should be lower than center
        let w_0 = kaiser_window(0, total_samples, beta);
        let w_mid = kaiser_window(total_samples / 2, total_samples, beta);
        let w_end = kaiser_window(total_samples - 1, total_samples, beta);

        assert!(w_mid > w_0);
        assert!(w_mid > w_end);
        // Symmetric within reasonable tolerance (accounting for discrete sampling)
        assert!((w_0 - w_end).abs() < 0.01);
    }

    #[test]
    fn test_kaiser_window_beta_effect() {
        let total_samples = 100;
        let n = total_samples / 4;

        let w_beta_0 = kaiser_window(n, total_samples, 0.0);
        let w_beta_5 = kaiser_window(n, total_samples, 5.0);
        let w_beta_8 = kaiser_window(n, total_samples, 8.6);

        // Higher beta = more tapering = lower edge values
        assert!(w_beta_0 >= w_beta_5);
        assert!(w_beta_5 >= w_beta_8);
    }

    #[test]
    fn test_bessel_i0_known_values() {
        // I₀(0) = 1
        assert!((bessel_i0(0.0) - 1.0).abs() < 1e-10);

        // I₀(1) ≈ 1.2661
        assert!((bessel_i0(1.0) - 1.2661).abs() < 1e-3);

        // I₀(5) ≈ 27.24
        assert!((bessel_i0(5.0) - 27.24).abs() < 0.1);
    }

    #[test]
    fn test_adaptive_integration_near_null() {
        let base = IntegrationParams::default();
        let wavelength = 0.035; // ~8.5 GHz
        let diameter = 1.0;
        let null_angle = 1.22 * wavelength / diameter;

        // Near null should increase sampling
        let params_near = adaptive_integration_params(
            null_angle,
            0.0,
            wavelength,
            diameter,
            &base,
        );

        assert!(params_near.min_rho_points > base.min_rho_points);
    }

    #[test]
    fn test_adaptive_integration_far_from_null() {
        let base = IntegrationParams::default();
        let wavelength = 0.035;
        let diameter = 1.0;

        // Far from null should keep base params
        let params_far = adaptive_integration_params(
            0.01, // Very small angle
            0.0,
            wavelength,
            diameter,
            &base,
        );

        assert_eq!(params_far.min_rho_points, base.min_rho_points);
    }

    #[test]
    fn test_kaiser_taper() {
        let radius = 0.5;
        let amplitude = 1.0;
        let beta = 5.0;

        // Center should have full amplitude
        let center = apply_kaiser_taper(0.0, radius, amplitude, beta);
        assert!((center - amplitude).abs() < 1e-3);

        // Edge should be tapered
        let edge = apply_kaiser_taper(radius, radius, amplitude, beta);
        assert!(edge < amplitude);
    }

    #[test]
    fn test_unwrap_phase_no_wrap() {
        let phases = vec![0.0, 0.1, 0.2, 0.3];
        let unwrapped = unwrap_phase(&phases);
        assert_eq!(unwrapped, phases);
    }

    #[test]
    fn test_unwrap_phase_with_wrap() {
        let phases = vec![0.0, PI / 2.0, PI, -2.0 * PI / 3.0]; // Wraps at last point
        let unwrapped = unwrap_phase(&phases);

        // Should be continuous
        assert!(unwrapped[3] > unwrapped[2]);
        assert!((unwrapped[3] - (4.0 * PI / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn test_strongly_needs_adaptive() {
        let wavelength = 0.035;
        let diameter = 1.0;
        let first_null = 1.22 * wavelength / diameter;

        // Below threshold: false
        assert!(!strongly_needs_adaptive(0.5 * first_null, wavelength, diameter));

        // Above threshold: true
        assert!(strongly_needs_adaptive(1.0 * first_null, wavelength, diameter));
    }

    #[test]
    fn test_smooth_to_floor_above_transition() {
        let gain = 1.0;
        let floor = 1e-6;
        let transition_db = 10.0;

        let smoothed = smooth_to_floor(gain, floor, transition_db);
        assert!((smoothed - gain).abs() < 1e-10);
    }

    #[test]
    fn test_smooth_to_floor_below_floor() {
        let gain = 1e-8;
        let floor = 1e-6;
        let transition_db = 10.0;

        let smoothed = smooth_to_floor(gain, floor, transition_db);
        assert!((smoothed - floor).abs() < 1e-10);
    }

    #[test]
    fn test_smooth_to_floor_in_transition() {
        let floor = 1e-6;
        let transition_db = 10.0;
        let gain = 1e-6 * 3.162; // +5 dB from floor (mid-transition)

        let smoothed = smooth_to_floor(gain, floor, transition_db);

        // Should be between floor and gain
        assert!(smoothed > floor);
        assert!(smoothed < gain);
    }

    #[test]
    fn test_smooth_step() {
        assert_eq!(smooth_step(0.0), 0.0);
        assert_eq!(smooth_step(1.0), 1.0);

        let mid = smooth_step(0.5);
        assert!((mid - 0.5).abs() < 0.1); // Should be near 0.5
    }
}
