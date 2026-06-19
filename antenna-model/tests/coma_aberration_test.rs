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
    compute_gain_db, AntennaConfiguration, FeedParameters, FeedPosition, IntegrationParams,
    ReflectorGeometry,
};

/// Test that feed at focal point produces maximum gain at boresight
#[test]
fn test_feed_at_focus_maximum_gain() {
    let freq_hz = 8450.0e6; // X-band
    let focal_length = 13.6; // 34m dish, f/D = 0.4

    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();
    let feed = FeedParameters::new(
        FeedPosition::at_focus(focal_length),
        10.0, // q_factor
        0.0,  // phase_center_offset
        1.0,  // asymmetry_factor
    )
    .unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    )
    .unwrap();

    // Aperture-directivity reference (no aperture-efficiency constant): the
    // uniform-aperture maximum is G_uniform = 4πA/λ², A = π(D/2)².
    // For the 34 m dish at 8.45 GHz (λ ≈ 0.03548 m) this is ≈ 69.6 dBi.
    let wavelength = 299792458.0 / freq_hz;
    let aperture_area = std::f64::consts::PI * (34.0_f64 / 2.0).powi(2);
    let uniform_max =
        10.0 * (4.0 * std::f64::consts::PI * aperture_area / (wavelength * wavelength)).log10();

    let result = compute_gain_db(0.0, 0.0, &config, freq_hz, &IntegrationParams::fast()).unwrap();
    let gain_boresight = result.gain;

    println!("Feed at focal point:");
    println!("  Uniform-aperture max: {:.2} dBi", uniform_max);
    println!("  Actual gain: {:.2} dBi", gain_boresight);
    println!("  Taper loss: {:.2} dB", uniform_max - gain_boresight);

    // The q=10 cos^q feed on a deep dish (f/D=0.4, edge angle ≈ 64°) is heavily
    // tapered, so taper efficiency is well below a uniform aperture. The directivity
    // formula yields ≈ 63.6 dBi (no RMS error here), i.e. ~6 dB taper loss — larger
    // than the old hardcoded η=0.65 assumption but physically consistent with a deep
    // taper (spillover is unmodeled and absorbed by calibration). Lock to [62.5, 65.0].
    assert!(
        gain_boresight > 62.5 && gain_boresight < 65.0,
        "Feed at focus boresight gain {:.2} dBi out of expected [62.5, 65.0] (uniform max {:.2} dBi)",
        gain_boresight,
        uniform_max
    );
    // Must remain below the uniform-aperture maximum.
    assert!(
        gain_boresight < uniform_max,
        "Gain {:.2} dBi must not exceed uniform-aperture max {:.2} dBi",
        gain_boresight,
        uniform_max
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
    println!(
        "  Expected steering: {:.2}° ({:.4} rad)",
        expected_steering_deg, expected_steering_rad
    );

    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();

    // Feed displaced in +X direction
    let feed = FeedParameters::new(
        FeedPosition::new(feed_displacement, 0.0, focal_length),
        10.0,
        0.0,
        1.0,
    )
    .unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    )
    .unwrap();

    // Scan pattern to find peak
    let mut max_gain = f64::NEG_INFINITY;
    let mut peak_theta = 0.0;

    // Scan from -10° to +10° in phi=0 plane
    for theta_deg in -100..=100 {
        let theta = (theta_deg as f64 * 0.1).to_radians();
        let result =
            compute_gain_db(theta, 0.0, &config, freq_hz, &IntegrationParams::fast()).unwrap();
        let gain = result.gain;

        if gain > max_gain {
            max_gain = gain;
            peak_theta = theta;
        }
    }

    let peak_theta_deg = peak_theta.to_degrees();
    println!(
        "  Pattern peak at: {:.2}° with gain {:.2} dBi",
        peak_theta_deg, max_gain
    );

    // Peak should be offset from boresight by approximately the expected steering angle
    // (Direction is opposite to feed displacement for reflected beam)
    let steering_error = (peak_theta_deg.abs() - expected_steering_deg).abs();

    println!("  Steering error: {:.2}°", steering_error);

    // With full path-length coma model, beam steering should be accurate
    assert!(
        steering_error < 1.0,
        "Beam should steer by ~{:.2}°, but peak is at {:.2}°",
        expected_steering_deg,
        peak_theta_deg
    );
}

/// Test gain reduction at boresight due to feed displacement
#[test]
fn test_gain_loss_from_feed_displacement() {
    let freq_hz = 8450.0e6;
    let focal_length = 13.6;
    let reflector = ReflectorGeometry::new(34.0, focal_length, 0.0).unwrap();

    // Test 1: Feed at focus
    let feed_at_focus =
        FeedParameters::new(FeedPosition::at_focus(focal_length), 10.0, 0.0, 1.0).unwrap();

    let config_focus = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector.clone(),
        feed_at_focus,
        None,
    )
    .unwrap();

    let result_at_focus =
        compute_gain_db(0.0, 0.0, &config_focus, freq_hz, &IntegrationParams::fast()).unwrap();
    let gain_at_focus = result_at_focus.gain;

    // Test 2: Feed displaced
    let feed_displaced =
        FeedParameters::new(FeedPosition::new(1.19, 0.0, focal_length), 10.0, 0.0, 1.0).unwrap();

    let config_displaced = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed_displaced,
        None,
    )
    .unwrap();

    let result_displaced = compute_gain_db(
        0.0,
        0.0,
        &config_displaced,
        freq_hz,
        &IntegrationParams::fast(),
    )
    .unwrap();
    let gain_displaced = result_displaced.gain;

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
        10.0,
        0.0,
        1.0,
    )
    .unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    )
    .unwrap();

    // Sample pattern in +theta and -theta directions (phi=0 plane)
    let result_plus_5deg = compute_gain_db(
        5.0f64.to_radians(),
        0.0,
        &config,
        freq_hz,
        &IntegrationParams::fast(),
    )
    .unwrap();
    let gain_plus_5deg = result_plus_5deg.gain;

    let result_minus_5deg = compute_gain_db(
        (-5.0f64).to_radians(),
        0.0,
        &config,
        freq_hz,
        &IntegrationParams::fast(),
    )
    .unwrap();
    let gain_minus_5deg = result_minus_5deg.gain;

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
    let feed = FeedParameters::new(FeedPosition::at_focus(focal_length), 10.0, 0.0, 1.0).unwrap();

    let config = AntennaConfiguration::new(
        "test".to_string(),
        "Test".to_string(),
        reflector,
        feed,
        None,
    )
    .unwrap();

    let result = compute_gain_db(0.0, 0.0, &config, freq_hz, &IntegrationParams::fast()).unwrap();
    let gain = result.gain;

    // Locks in current behavior to detect regressions. Re-baselined for the
    // aperture-directivity gain formula (taper efficiency built into the integral;
    // no hardcoded aperture-efficiency constant). 34 m dish, q=10 feed, f/D=0.4,
    // 0.5 mm RMS (≈0.14 dB Ruze loss at 8.45 GHz): boresight gain ≈ 63.5 dBi.
    let expected_gain = 63.48;
    let tolerance = 0.5;

    assert!(
        (gain - expected_gain).abs() < tolerance,
        "Regression: feed at focus gain changed from {:.2} to {:.2} dBi",
        expected_gain,
        gain
    );
}
