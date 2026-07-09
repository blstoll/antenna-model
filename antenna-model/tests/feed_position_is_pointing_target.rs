//! Regression guard for THE anchor bug (docs/domain-contract.md, parameter glossary):
//! the API `feed_position` is the feed's *pointing* location — an Earth aim point — NOT
//! the feed's fixed physical offset in the antenna frame.
//!
//! `compute_feed_position_from_pointing` is the exact function the `/h3` path uses to
//! turn (`feed_position`, `reflector_boresight`, `vehicle_position`) into a physical feed
//! displacement (see `src/service/h3_link_budget.rs`). Because the physical displacement
//! is resolved *relative to the vehicle*, the SAME feed/reflector aim points viewed from
//! DIFFERENT vehicle positions must yield DIFFERENT physical feed displacements.
//!
//! If someone ever reintroduces the "feed_position = fixed physical location" misreading,
//! the two displacements below would come out identical and this test fails.

use antenna_model::api::schemas::Position3D;
use antenna_model::model::compute_feed_position_from_pointing;

const FOCAL_LENGTH_M: f64 = 5.0;
const REFLECTOR_DIAMETER_M: f64 = 10.0; // f/D = 0.5

#[test]
fn feed_position_resolves_relative_to_vehicle_not_absolute() {
    // Fixed Earth aim points (geodetic: lon°, lat°, alt m).
    let feed_pointing = Position3D::new(0.2, 0.1, 0.0);
    let reflector_pointing = Position3D::new(0.0, 0.0, 0.0);

    // Two distinct vehicle positions (700 km altitude), differing in longitude.
    let vehicle_a = Position3D::new(0.0, 0.0, 700_000.0);
    let vehicle_b = Position3D::new(0.5, 0.0, 700_000.0);

    let phys_a = compute_feed_position_from_pointing(
        &feed_pointing,
        &reflector_pointing,
        &vehicle_a,
        FOCAL_LENGTH_M,
        REFLECTOR_DIAMETER_M,
        None,
    )
    .expect("config A should resolve");

    let phys_b = compute_feed_position_from_pointing(
        &feed_pointing,
        &reflector_pointing,
        &vehicle_b,
        FOCAL_LENGTH_M,
        REFLECTOR_DIAMETER_M,
        None,
    )
    .expect("config B should resolve");

    // Same aim points, different vehicle → the physical feed displacement must differ.
    let d = ((phys_a.0 - phys_b.0).powi(2)
        + (phys_a.1 - phys_b.1).powi(2)
        + (phys_a.2 - phys_b.2).powi(2))
    .sqrt();

    assert!(
        d > 1e-6,
        "feed_position must resolve relative to the vehicle: identical physical feed \
         displacement for different vehicle positions means the aim-point semantic was \
         lost. phys_a={phys_a:?} phys_b={phys_b:?} (|Δ|={d})"
    );
}
