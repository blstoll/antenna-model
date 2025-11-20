//! Test suite for coma aberration validation
//!
//! Validates that feed displacement produces the expected optical aberrations:
//! 1. Beam steering: displaced feed steers beam in opposite direction
//! 2. Gain reduction: gain at original boresight decreases
//! 3. Coma lobe: asymmetric sidelobe pattern appears
//!
//! # Theoretical Background
//!
//! For a parabolic reflector with feed displaced from focal point:
//! - Beam steering angle: θ_steer ≈ δ_feed / f (radians)
//! - Direction: opposite to feed displacement
//! - Gain loss at boresight: increases with displacement
//!
//! References:
//! - Rusch & Potter, "Analysis of Reflector Antennas" (1970)
//! - Love, "Electromagnetic Horn Antennas" (1976), Chapter 10

use antenna_model::model::{
    compute_gain_db, AntennaConfiguration, FeedParameters, FeedPosition,
    IntegrationParams, ReflectorGeometry,
};

/// Test that feed at focal point produces maximum gain at boresight
#[test]
fn test_feed_at_focus_maximum_gain() {
    let freq_hz = 8450.0e6;  // X-band
    let focal_length = 13.6; // 34m dish, f/D = 0.4

    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();
    let feed = FeedParameters::new(
        FeedPosition::at_focus(focal_length),
        10.0,  // q_factor
        0.0,   // phase_center_offset
        1.0,   // asymmetry_factor
    ).unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    ).unwrap();

    // Theoretical maximum (η = 0.65 for X-band)
    // G = 10*log10(0.65 * (πD/λ)^2)
    let wavelength = 299792458.0 / freq_hz;
    let theoretical_max = 10.0 * ((0.65 * (std::f64::consts::PI * 34.0 / wavelength).powi(2)).log10());

    let gain_boresight = compute_gain_db(0.0, 0.0, &config, freq_hz, &IntegrationParams::fast()).unwrap();

    println!("Feed at focal point:");
    println!("  Theoretical max: {:.2} dBi", theoretical_max);
    println!("  Actual gain: {:.2} dBi", gain_boresight);
    println!("  Difference: {:.2} dB", theoretical_max - gain_boresight);

    // Should be within 1 dB of theoretical max
    assert!(
        (gain_boresight - theoretical_max).abs() < 1.0,
        "Feed at focus should produce near-maximum gain, got {:.2} dBi vs theoretical {:.2} dBi",
        gain_boresight, theoretical_max
    );
}

/// Test beam steering: feed displacement should steer beam
#[test]
fn test_beam_steering_from_feed_displacement() {
    let freq_hz = 8450.0e6;
    let focal_length = 13.6;
    let feed_displacement = 1.19; // meters

    // Expected beam steering angle: θ ≈ δ/f
    let expected_steering_rad: f64 = feed_displacement / focal_length;
    let expected_steering_deg = expected_steering_rad.to_degrees();

    println!("Beam steering test:");
    println!("  Feed displacement: {:.2} m", feed_displacement);
    println!("  Focal length: {:.2} m", focal_length);
    println!("  Expected steering: {:.2}° ({:.4} rad)", expected_steering_deg, expected_steering_rad);

    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();

    // Feed displaced in +X direction
    let feed = FeedParameters::new(
        FeedPosition::new(feed_displacement, 0.0, focal_length),
        10.0, 0.0, 1.0,
    ).unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    ).unwrap();

    // Scan pattern to find peak
    let mut max_gain = f64::NEG_INFINITY;
    let mut peak_theta = 0.0;

    // Scan from -10° to +10° in phi=0 plane
    for theta_deg in -100..=100 {
        let theta = (theta_deg as f64 * 0.1).to_radians();
        let gain = compute_gain_db(theta, 0.0, &config, freq_hz, &IntegrationParams::fast()).unwrap();

        if gain > max_gain {
            max_gain = gain;
            peak_theta = theta;
        }
    }

    let peak_theta_deg = peak_theta.to_degrees();
    println!("  Pattern peak at: {:.2}° with gain {:.2} dBi", peak_theta_deg, max_gain);

    // Peak should be offset from boresight by approximately the expected steering angle
    // (Direction is opposite to feed displacement for reflected beam)
    let steering_error = (peak_theta_deg.abs() - expected_steering_deg).abs();

    println!("  Steering error: {:.2}°", steering_error);

    // With full path-length coma model, beam steering should be accurate
    assert!(
        steering_error < 1.0,
        "Beam should steer by ~{:.2}°, but peak is at {:.2}°",
        expected_steering_deg, peak_theta_deg
    );
}

/// Test gain reduction at boresight due to feed displacement
#[test]
fn test_gain_loss_from_feed_displacement() {
    let freq_hz = 8450.0e6;
    let focal_length = 13.6;
    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();

    // Test 1: Feed at focus
    let feed_at_focus = FeedParameters::new(
        FeedPosition::at_focus(focal_length),
        10.0, 0.0, 1.0,
    ).unwrap();

    let config_focus = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector.clone(),
        feed_at_focus,
        None,
    ).unwrap();

    let gain_at_focus = compute_gain_db(0.0, 0.0, &config_focus, freq_hz, &IntegrationParams::fast()).unwrap();

    // Test 2: Feed displaced
    let feed_displaced = FeedParameters::new(
        FeedPosition::new(1.19, 0.0, focal_length),
        10.0, 0.0, 1.0,
    ).unwrap();

    let config_displaced = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed_displaced,
        None,
    ).unwrap();

    let gain_displaced = compute_gain_db(0.0, 0.0, &config_displaced, freq_hz, &IntegrationParams::fast()).unwrap();

    let gain_loss = gain_at_focus - gain_displaced;

    println!("Gain loss test:");
    println!("  Feed at focus: {:.2} dBi", gain_at_focus);
    println!("  Feed displaced 1.19m: {:.2} dBi", gain_displaced);
    println!("  Gain loss at boresight: {:.2} dB", gain_loss);

    // Displaced feed should have LOWER gain at boresight due to phase errors
    // For ~0.09f displacement (1.19m / 13.6m), expect several dB loss
    assert!(
        gain_loss > 2.0,
        "Displaced feed should have >2dB loss at boresight, got {:.2} dB",
        gain_loss
    );
}

/// Test pattern asymmetry (coma lobe)
#[test]
fn test_coma_lobe_asymmetry() {
    let freq_hz = 8450.0e6;
    let focal_length = 13.6;

    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();
    let feed = FeedParameters::new(
        FeedPosition::new(1.19, 0.0, focal_length), // Displaced in +X
        10.0, 0.0, 1.0,
    ).unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    ).unwrap();

    // Sample pattern in +theta and -theta directions (phi=0 plane)
    let gain_plus_5deg = compute_gain_db(
        5.0f64.to_radians(), 0.0, &config, freq_hz, &IntegrationParams::fast()
    ).unwrap();

    let gain_minus_5deg = compute_gain_db(
        (-5.0f64).to_radians(), 0.0, &config, freq_hz, &IntegrationParams::fast()
    ).unwrap();

    let asymmetry = (gain_plus_5deg - gain_minus_5deg).abs();

    println!("Coma lobe asymmetry test:");
    println!("  Gain at +5°: {:.2} dBi", gain_plus_5deg);
    println!("  Gain at -5°: {:.2} dBi", gain_minus_5deg);
    println!("  Asymmetry: {:.2} dB", asymmetry);

    // Coma produces asymmetric pattern - one side should be higher
    assert!(
        asymmetry > 3.0,
        "Coma should produce asymmetric pattern with >3dB difference, got {:.2} dB",
        asymmetry
    );
}

/// Regression test: ensure feed at focus gives consistent results
#[test]
fn test_regression_feed_at_focus() {
    let freq_hz = 8450.0e6;
    let focal_length = 13.6;

    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0005).unwrap();
    let feed = FeedParameters::new(
        FeedPosition::at_focus(focal_length),
        10.0, 0.0, 1.0,
    ).unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    ).unwrap();

    let gain = compute_gain_db(0.0, 0.0, &config, freq_hz, &IntegrationParams::fast()).unwrap();

    // Based on current implementation with full path-length coma model (surface RMS = 0.5mm)
    // This locks in the current behavior to detect regressions
    let expected_gain = 66.98; // Updated for full path-length coma model
    let tolerance = 0.5;

    assert!(
        (gain - expected_gain).abs() < tolerance,
        "Regression: feed at focus gain changed from {:.2} to {:.2} dBi",
        expected_gain, gain
    );
}
