//! Regression tests for the feed-steering direction convention.
//!
//! A lateral feed offset steers a paraboloid's beam to the OPPOSITE side of
//! boresight (beam deviation). `to_feed_position` must therefore place the
//! feed at clock angle φ+180° when the caller asks for a beam at clock φ.
//! Before the 2026-07 fix the feed was placed at φ, putting the beam peak
//! 180° away from every steered-feed aim point (~70 dB error at the target).

use antenna_model::model::{
    compute_gain_db, AntennaConfiguration, EClockConeCoordinates, FeedParameters, FeedPosition,
    IntegrationParams, ReflectorGeometry,
};
use std::f64::consts::PI;

const FREQ_HZ: f64 = 8.4e9;

fn dish_with_feed(x: f64, y: f64, z: f64) -> AntennaConfiguration {
    let reflector = ReflectorGeometry::new(10.0, 5.0, 0.0).unwrap(); // D=10 m, f=5 m, ideal surface
    let feed = FeedParameters::new(FeedPosition::new(x, y, z), 8.0, 0.0, 1.0).unwrap();
    AntennaConfiguration::new("probe".into(), "Probe".into(), reflector, feed, None).unwrap()
}

fn gain_db(config: &AntennaConfiguration, el_deg: f64, az_rad: f64) -> f64 {
    compute_gain_db(
        el_deg.to_radians(),
        az_rad,
        config,
        FREQ_HZ,
        &IntegrationParams::default(),
    )
    .unwrap()
    .gain
}

/// Steer toward az=0°, el=2°: gain on the requested side must dominate.
#[test]
fn steered_beam_lands_on_requested_azimuth_x() {
    let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 2.0);
    let (fx, fy, fz) = ecc.to_feed_position(5.0);
    let config = dish_with_feed(fx, fy, fz);

    let g_target = gain_db(&config, 2.0, 0.0);
    let g_opposite = gain_db(&config, 2.0, PI);
    assert!(
        g_target > g_opposite + 30.0,
        "beam must land on the requested side: target={g_target:.1} dBi, opposite={g_opposite:.1} dBi"
    );
}

/// Steer toward az=90°, el=2°: independently catches y-axis sign errors.
#[test]
fn steered_beam_lands_on_requested_azimuth_y() {
    let ecc = EClockConeCoordinates::from_azimuth_elevation(90.0, 2.0);
    let (fx, fy, fz) = ecc.to_feed_position(5.0);
    let config = dish_with_feed(fx, fy, fz);

    let g_target = gain_db(&config, 2.0, PI / 2.0);
    let g_opposite = gain_db(&config, 2.0, 3.0 * PI / 2.0);
    assert!(
        g_target > g_opposite + 30.0,
        "beam must land on the requested side: target={g_target:.1} dBi, opposite={g_opposite:.1} dBi"
    );
}

/// The feed itself must sit OPPOSITE the aim direction.
#[test]
fn feed_is_displaced_opposite_the_aim_direction() {
    let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 5.0);
    let (fx, fy, _fz) = ecc.to_feed_position(5.0);
    assert!(fx < -0.1, "aim at +x must displace the feed toward -x, got fx={fx}");
    assert!(fy.abs() < 1e-9, "no y component expected, got fy={fy}");
}

/// With the beam deviation factor applied, the beam peak must land at the
/// requested steering angle (within a tenth of the ~0.25° beamwidth grid).
/// Without BDF the peak sits at ~2°·0.87 ≈ 1.75° and this test fails.
#[test]
fn steered_beam_peaks_at_requested_angle() {
    use antenna_model::model::beam_deviation_factor;

    // BDF unit checks (Lo 1960, K = 0.36)
    let bdf_half = beam_deviation_factor(0.5);
    assert!(
        (0.86..=0.88).contains(&bdf_half),
        "BDF(f/D=0.5) = {bdf_half}, expected ~0.871"
    );
    assert!(beam_deviation_factor(10.0) > 0.99);

    // Steer 2° at az=0 on the D=10 m, f=5 m dish via the E-cone path directly
    // (unit-level; the service path is exercised by evaluator tests).
    let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 2.0);
    let (fx, fy, fz) = ecc.to_feed_position_with_bdf(5.0, bdf_half);
    let config = dish_with_feed(fx, fy, fz);

    // Scan elevation on the az=0 cut for the peak.
    let mut peak = (0.0_f64, f64::NEG_INFINITY);
    let mut el = 1.0_f64;
    while el <= 3.0 {
        let g = gain_db(&config, el, 0.0);
        if g > peak.1 {
            peak = (el, g);
        }
        el += 0.05;
    }
    assert!(
        (peak.0 - 2.0).abs() <= 0.15,
        "beam peak at {:.2}°, expected 2.00° ± 0.15°",
        peak.0
    );
}
