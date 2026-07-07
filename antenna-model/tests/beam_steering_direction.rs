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
