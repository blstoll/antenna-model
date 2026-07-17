//! Shared test-only fixtures for the service layer (peer-review dedup,
//! 2026-07-17).
//!
//! `service::evaluator` and `service::batch` each carried their own copy of
//! the rear-hemisphere test geometry and the uncalibrated-antenna repository
//! builder, written independently and at risk of drifting apart (see
//! `docs/domain-contract.md` on why silently-diverging geometry fixtures are
//! dangerous in this codebase). This module is the single canonical copy;
//! both `evaluator::tests` and `batch::tests` call into it.
//!
//! Every consuming test must keep its own
//! `emitter_elevation_deg.abs() > 90.0` precondition assert at the call
//! site — that self-check intentionally stays in the tests, not here, so a
//! coordinate-frame regression fails loudly in the test that relies on it.

use crate::api::schemas::{CoordinateSystem, GainRequest, Position3D};
use crate::data::repository::CalibrationRepository;
use crate::data::types::{
    AntennaCalibration, CalibrationMetadata, CalibrationStatus, FeedParameters, MeshParameters,
    PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
};
use crate::model::coordinates_3d::geodetic_to_ecef;

/// Build a request whose emitter is placed BEHIND the dish: the reflector
/// boresight points at a 400 km satellite (-117, 35), the feed is aimed at
/// that same target (feed at focus → StandardPhysicalOptics were the mode
/// dispatch reached, though the rear-hemisphere early return in
/// `model::pattern::compute_gain` bypasses mode dispatch entirely), and the
/// emitter is dropped to ground level at a different lon/lat (-120, 30, alt
/// 0), which places it ~104 deg off the boresight axis — REAR hemisphere.
pub(crate) fn rear_hemisphere_request() -> GainRequest {
    let ecef = |lon: f64, lat: f64, alt: f64| {
        let (x, y, z) = geodetic_to_ecef(lon, lat, alt).unwrap();
        let mut p = Position3D::new(x, y, z);
        p.coordinate_system = Some(CoordinateSystem::ECEF);
        p
    };
    GainRequest {
        antenna_id: "test_antenna".to_string(),
        feed_id: "test_feed".to_string(),
        vehicle_position: ecef(-118.1234, 34.5678, 100.0),
        reflector_boresight: ecef(-117.0, 35.0, 400_000.0),
        // Feed aimed at boresight target → feed at focus → StandardPhysicalOptics.
        feed_position: ecef(-117.0, 35.0, 400_000.0),
        emitter_position: ecef(-120.0, 30.0, 0.0),
        frequency_mhz: 8400.0,
        pointing_frequency_mhz: None,
        include_reference: false,
        vehicle_attitude: None,
    }
}

/// Build a repository holding a single UNCALIBRATED antenna (no correction
/// surface ⇒ `physics_is_uncorrected()` ⇒ F7 floor ON) with a nonzero surface
/// RMS so the statistical floor is a clearly nonzero pedestal.
///
/// Feed design position is `(0, 0, 0)` — zero additional physical offset on
/// top of whatever the request's steering computes — matching
/// `evaluator::tests::create_test_calibration`'s convention. (The prior
/// `batch.rs` copy used `(0, 0, 5.0)`, which stacks an extra +5 m of axial
/// defocus on top of the steering-derived focus position; that discrepancy
/// was invisible because every consumer of this fixture queries the rear
/// hemisphere, where `compute_gain`'s early return serves the floor alone
/// and never reaches the mode dispatch that would have made the offset
/// matter.)
pub(crate) fn create_uncalibrated_repository() -> CalibrationRepository {
    let mut repo = CalibrationRepository::new();
    let metadata = CalibrationMetadata::builder()
        .antenna_name("Test Antenna")
        .calibration_date("2025-01-01T00:00:00Z")
        .format_version("2.0")
        .data_source("test")
        .rmse_db(0.5)
        .r_squared(0.99)
        .num_measurements(1000)
        .build()
        .unwrap();
    let calibration = AntennaCalibration::builder()
        .antenna_id("test_antenna")
        .feed_id("test_feed")
        .metadata(metadata)
        .physical_config(PhysicalAntennaConfig {
            reflector: ReflectorGeometry {
                diameter_m: 10.0,
                focal_length_m: 5.0,
                f_over_d_ratio: 0.5,
                surface_rms_mm: 1.5,
            },
            feed: FeedParameters {
                // Feed at focal point - zero offset from optical axis.
                position: (0.0, 0.0, 0.0),
                q_factor: 8.0,
                phase_center_offset_m: 0.0,
                axial_defocus_m: 0.0,
            },
            mesh: Some(MeshParameters {
                mesh_spacing_mm: 5.0,
                wire_diameter_mm: 0.5,
            }),
        })
        .validity_ranges(ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (1000.0, 10000.0),
            temperature_const: 290.0,
        })
        .calibration_status(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        })
        .build()
        .unwrap();
    assert!(calibration.correction_surface.is_none());
    repo.add_calibration(calibration);
    repo
}
