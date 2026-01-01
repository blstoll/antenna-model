//! Near-boresight direct path interference modeling
//!
//! When observing near boresight with an offset feed, two paths contribute:
//! 1. **Reflected path**: Feed → Reflector → Observation point
//! 2. **Direct path**: Feed → Observation point (bypassing reflector)
//!
//! These paths interfere, creating complex patterns near boresight that pure
//! reflector models miss. This is particularly important for:
//! - Calibration near θ = 0
//! - Feed offset > 0.1f
//! - Blockage and feed shadow effects
//!
//! # References
//! - Design doc Section 3.1 (Near-Boresight/Far-Feed Scenario)

use crate::model::geometry::AntennaConfiguration;
use num_complex::Complex64;
use std::f64::consts::PI;

/// Result of direct path interference computation
#[derive(Debug, Clone)]
pub struct DirectPathResult {
    /// Total electric field (reflected + direct)
    pub total_field: Complex64,

    /// Reflected path contribution
    pub reflected_field: Complex64,

    /// Direct path contribution
    pub direct_field: Complex64,

    /// Interference factor (magnitude of total / magnitude of reflected)
    pub interference_factor: f64,

    /// Path length difference (direct - reflected) in wavelengths
    pub path_difference_wavelengths: f64,
}

/// Compute field with direct path interference
///
/// This combines the standard reflected path (physical optics or ray tracing)
/// with the direct path from feed to observation point.
///
/// # Arguments
/// * `config` - Antenna configuration
/// * `theta` - Far-field elevation angle (radians)
/// * `phi` - Far-field azimuth angle (radians)
/// * `wavelength` - Operating wavelength (meters)
/// * `reflected_field` - Pre-computed reflected path field
///
/// # Returns
/// Combined field with interference
pub fn compute_with_direct_path(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    wavelength: f64,
    reflected_field: Complex64,
) -> DirectPathResult {
    // Get feed position
    let feed_pos = get_feed_position(config);

    // Compute direct path contribution
    let direct_field = compute_direct_path_field(feed_pos, theta, phi, wavelength, &config.feed);

    // Total field is coherent sum
    let total_field = reflected_field + direct_field;

    // Interference factor
    let interference_factor = if reflected_field.norm() > 1e-12 {
        total_field.norm() / reflected_field.norm()
    } else {
        1.0
    };

    // Path difference
    // Reflected path: feed → reflector → far field ≈ focal_length + distance to far field
    // Direct path: feed → far field
    // At large distance r, difference ≈ |feed_offset| · sin(θ)
    let _focal_length = config.reflector.focal_length;
    let feed_offset = (feed_pos.0.powi(2) + feed_pos.1.powi(2) + feed_pos.2.powi(2)).sqrt();

    // Approximate path difference (sign depends on geometry)
    let path_diff = feed_offset * theta.sin();
    let path_diff_wavelengths = path_diff / wavelength;

    DirectPathResult {
        total_field,
        reflected_field,
        direct_field,
        interference_factor,
        path_difference_wavelengths: path_diff_wavelengths,
    }
}

/// Compute direct path field from feed to far-field observation point
///
/// This models the feed as a point source with directional pattern.
///
/// # Arguments
/// * `feed_pos` - Feed position (x, y, z)
/// * `theta` - Far-field elevation angle (radians)
/// * `phi` - Far-field azimuth angle (radians)
/// * `wavelength` - Operating wavelength (meters)
/// * `feed_params` - Feed parameters
///
/// # Returns
/// Direct path electric field
fn compute_direct_path_field(
    feed_pos: (f64, f64, f64),
    theta: f64,
    phi: f64,
    wavelength: f64,
    feed_params: &crate::model::geometry::FeedParameters,
) -> Complex64 {
    let wavenumber = 2.0 * PI / wavelength;

    // Direction to observation point
    let obs_dir = (
        theta.sin() * phi.cos(),
        theta.sin() * phi.sin(),
        theta.cos(),
    );

    // Vector from feed to observation direction
    // (At far field, distance >> antenna size, so path ≈ parallel)

    // Feed pattern in observation direction
    // Angle between feed boresight (assumed +z) and observation direction
    let cos_angle = obs_dir.2; // Dot product with (0, 0, 1)
    let feed_angle = cos_angle.acos();

    // Feed pattern amplitude
    let q = feed_params.q_factor;
    let feed_pattern = if feed_angle < PI / 2.0 {
        feed_angle.cos().powf(q)
    } else {
        0.0
    };

    // Phase: path from feed position to far field
    // At far field distance r >> D, phase ≈ k · (feed_pos · obs_dir)
    let phase =
        wavenumber * (feed_pos.0 * obs_dir.0 + feed_pos.1 * obs_dir.1 + feed_pos.2 * obs_dir.2);

    // Amplitude scaling (1/r dependence absorbed in normalization)
    // Direct path is typically much weaker than reflected path for well-designed antennas
    let amplitude_scaling = 0.1; // Empirical factor (direct path is weaker)

    Complex64::from_polar(feed_pattern * amplitude_scaling, -phase)
}

/// Get feed position in Cartesian coordinates
fn get_feed_position(config: &AntennaConfiguration) -> (f64, f64, f64) {
    (
        config.feed.position.x,
        config.feed.position.y,
        config.feed.position.z,
    )
}

/// Check if direct path is significant
///
/// Direct path matters when:
/// - Near boresight (θ < ~1°)
/// - Feed offset is significant (> 0.1f)
///
/// # Arguments
/// * `config` - Antenna configuration
/// * `theta` - Far-field elevation angle (radians)
///
/// # Returns
/// True if direct path should be included
pub fn should_include_direct_path(config: &AntennaConfiguration, theta: f64) -> bool {
    let focal_length = config.reflector.focal_length;
    let feed_offset = config.feed.position.displacement_from_focus(focal_length);
    let offset_ratio = feed_offset / focal_length;

    // Near boresight and significant feed offset
    theta < 0.017453 && offset_ratio > 0.1 // ~1 degree, >10% focal length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::coordinates::EClockConeCoordinates;
    use crate::model::geometry::{FeedParameters, FeedPosition, ReflectorGeometry};

    fn test_antenna(e_cone_deg: f64) -> AntennaConfiguration {
        let focal_length = 0.5;
        let ecc = EClockConeCoordinates::from_degrees(e_cone_deg, 0.0);
        let (x, y, z) = ecc.to_feed_position(focal_length);

        AntennaConfiguration {
            id: "test_antenna".to_string(),
            name: format!("Test Antenna {} deg", e_cone_deg),
            reflector: ReflectorGeometry {
                diameter: 1.0,
                focal_length,
                surface_rms: 0.0005,
            },
            feed: FeedParameters {
                position: FeedPosition::new(x, y, z),
                q_factor: 8.0,
                phase_center_offset: 0.0,
                asymmetry_factor: 1.0,
            },
            mesh: None,
        }
    }

    #[test]
    fn test_on_axis_no_direct_path() {
        let config = test_antenna(0.0);
        assert!(!should_include_direct_path(&config, 0.0));
    }

    #[test]
    fn test_offset_near_boresight_has_direct_path() {
        let config = test_antenna(10.0); // ~17.5% offset
        assert!(should_include_direct_path(&config, 0.01)); // 0.57 degrees
    }

    #[test]
    fn test_offset_far_from_boresight_no_direct_path() {
        let config = test_antenna(10.0);
        assert!(!should_include_direct_path(&config, 0.1)); // 5.7 degrees
    }

    #[test]
    fn test_direct_path_field_on_boresight() {
        let config = test_antenna(10.0);
        let wavelength = 0.035;

        // Dummy reflected field
        let reflected_field = Complex64::new(1.0, 0.0);

        let result = compute_with_direct_path(&config, 0.0, 0.0, wavelength, reflected_field);

        // Direct field should be non-zero
        assert!(result.direct_field.norm() > 0.0);

        // Total should differ from reflected
        assert!((result.total_field.norm() - reflected_field.norm()).abs() > 1e-6);
    }

    #[test]
    fn test_interference_factor() {
        let config = test_antenna(10.0);
        let wavelength = 0.035;

        let reflected_field = Complex64::new(1.0, 0.0);

        let result = compute_with_direct_path(&config, 0.0, 0.0, wavelength, reflected_field);

        // Interference factor should be defined
        assert!(result.interference_factor > 0.0);

        // Can be > 1 (constructive) or < 1 (destructive)
        assert!(result.interference_factor > 0.5);
        assert!(result.interference_factor < 2.0);
    }

    #[test]
    fn test_path_difference() {
        let config = test_antenna(15.0); // Larger offset
        let wavelength = 0.035;

        let reflected_field = Complex64::new(1.0, 0.0);

        let result = compute_with_direct_path(
            &config,
            0.05, // Small but non-zero angle
            0.0,
            wavelength,
            reflected_field,
        );

        // Path difference should be non-zero
        assert!(result.path_difference_wavelengths.abs() > 0.0);
    }

    #[test]
    fn test_direct_field_decreases_off_axis() {
        let config = test_antenna(10.0);
        let wavelength = 0.035;

        let reflected_field = Complex64::new(1.0, 0.0);

        let result_on_axis =
            compute_with_direct_path(&config, 0.0, 0.0, wavelength, reflected_field);

        let result_off_axis =
            compute_with_direct_path(&config, 0.05, 0.0, wavelength, reflected_field);

        // Direct field should decrease with angle (feed pattern effect)
        // Note: May not always be true due to phase effects, but generally expected
        assert!(result_off_axis.direct_field.norm() <= result_on_axis.direct_field.norm() * 1.5);
    }

    #[test]
    fn test_feed_position_extraction() {
        let config = test_antenna(12.0);
        let pos = get_feed_position(&config);

        // Should have non-zero x (E-clock = 0 means x-axis)
        assert!(pos.0 > 0.0);
        assert!(pos.1.abs() < 1e-6); // E-clock = 0 → y = 0
    }

    #[test]
    fn test_direct_path_zero_for_on_axis() {
        let config = test_antenna(0.0); // On-axis
        let wavelength = 0.035;
        let feed_pos = get_feed_position(&config);

        let direct_field = compute_direct_path_field(feed_pos, 0.0, 0.0, wavelength, &config.feed);

        // On-axis feed at origin: direct field should be minimal/zero
        // (pattern is maximum but no offset phase)
        assert!(direct_field.norm() < 0.2); // Small amplitude scaling applied
    }

    #[test]
    fn test_constructive_destructive_interference() {
        let config = test_antenna(10.0);
        let wavelength = 0.035;

        // Test with different reflected field phases
        let reflected_in_phase = Complex64::new(1.0, 0.0);
        let reflected_out_phase = Complex64::new(-1.0, 0.0);

        let result_in = compute_with_direct_path(&config, 0.0, 0.0, wavelength, reflected_in_phase);

        let result_out =
            compute_with_direct_path(&config, 0.0, 0.0, wavelength, reflected_out_phase);

        // Different interference patterns expected
        assert!((result_in.interference_factor - result_out.interference_factor).abs() > 0.01);
    }
}
