//! Integration test for feed steering scenarios
//!
//! Verifies that the evaluator correctly handles:
//! 1. Perfect alignment: feed_position == reflector_boresight → maximum gain
//! 2. Large feed offset: feed steered away from boresight → reduced gain

use antenna_model::api::schemas::{GainRequest, Position3D};
use antenna_model::data::repository::CalibrationRepository;
use antenna_model::data::types::{
    AntennaCalibration, CalibrationMetadata, CalibrationStatus, FeedParameters, MeshParameters,
    PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
};
use antenna_model::service::evaluator::compute_gain_from_request;

fn create_test_calibration(antenna_id: &str, feed_id: &str) -> AntennaCalibration {
    let metadata = CalibrationMetadata::builder()
        .antenna_name("DSN 34m Test Antenna")
        .calibration_date("2025-01-01T00:00:00Z")
        .format_version("2.0")
        .data_source("test")
        .rmse_db(0.5)
        .r_squared(0.99)
        .num_measurements(1000)
        .build()
        .unwrap();

    AntennaCalibration::builder()
        .antenna_id(antenna_id)
        .feed_id(feed_id)
        .metadata(metadata)
        .physical_config(PhysicalAntennaConfig {
            reflector: ReflectorGeometry {
                diameter_m: 34.0,
                focal_length_m: 13.6, // f/D = 0.4
                f_over_d_ratio: 0.4,
                surface_rms_mm: 0.5,
            },
            feed: FeedParameters {
                position: (0.0, 0.0, 13.6), // Nominal position at focal point
                q_factor: 10.0,
                phase_center_offset_m: 0.0,
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
        .unwrap()
}

#[test]
fn test_feed_steering_perfect_alignment() {
    // Create test calibration
    let mut repo = CalibrationRepository::new();
    repo.add_calibration(create_test_calibration("test_antenna", "x_band"));

    // GEO satellite perfect alignment scenario (from geo_perfect_alignment.json)
    let request = GainRequest {
        antenna_id: "test_antenna".to_string(),
        feed_id: "x_band".to_string(),
        vehicle_position: Position3D::new(-19794863.29, -37228723.27, 0.0),
        reflector_boresight: Position3D::new(-2485073.18, -4673742.90, 3546502.48),
        feed_position: Position3D::new(-2485073.18, -4673742.90, 3546502.48), // Same as boresight
        emitter_position: Position3D::new(-2485073.18, -4673742.90, 3546502.48),
        frequency_mhz: 8450.0,
        pointing_frequency_mhz: None,
        include_reference: true,
    };

    let response = compute_gain_from_request(&request, &repo)
        .expect("Failed to compute gain for perfect alignment");

    println!("\n=== Perfect Alignment ===");
    println!("Gain: {:.2} dBi", response.gain_db);
    println!("Reference: {:.2} dBi", response.reference_gain_db.unwrap());
    println!("Loss: {:.2} dB", response.loss_db.unwrap());
    println!("Emitter elevation: {:.4}°", response.geometry.emitter_elevation_deg);

    // Verify results
    assert!(response.gain_db > 66.0, "Gain should be near theoretical maximum (~67 dBi)");
    assert!(response.gain_db < 68.0, "Gain should not exceed theoretical maximum");

    // Emitter elevation should be very small (nearly at boresight)
    assert!(
        response.geometry.emitter_elevation_deg.abs() < 0.1,
        "Emitter should be at boresight (elevation ≈ 0°), got {:.4}°",
        response.geometry.emitter_elevation_deg
    );
}

#[test]
fn test_feed_steering_large_offset() {
    // Create test calibration
    let mut repo = CalibrationRepository::new();
    repo.add_calibration(create_test_calibration("test_antenna", "x_band"));

    // GEO satellite with large feed offset scenario (from geo_large_feed_offset.json)
    let request = GainRequest {
        antenna_id: "test_antenna".to_string(),
        feed_id: "x_band".to_string(),
        vehicle_position: Position3D::new(-19794863.29, -37228723.27, 0.0),
        reflector_boresight: Position3D::new(-2485073.18, -4673742.90, 3546502.48),
        feed_position: Position3D::new(-4831642.29, -1948496.21, 3667577.84), // ~5° from boresight
        emitter_position: Position3D::new(-2225583.04, -4185713.15, 4252983.55),
        frequency_mhz: 8450.0,
        pointing_frequency_mhz: None,
        include_reference: true,
    };

    let response = compute_gain_from_request(&request, &repo)
        .expect("Failed to compute gain for large feed offset");

    println!("\n=== Large Feed Offset ===");
    println!("Gain: {:.2} dBi", response.gain_db);
    println!("Reference: {:.2} dBi", response.reference_gain_db.unwrap());
    println!("Loss: {:.2} dB", response.loss_db.unwrap());
    println!("Emitter elevation: {:.4}°", response.geometry.emitter_elevation_deg);

    // With large feed offset, gain should be significantly reduced
    // Feed aberrations (coma) should reduce gain by several dB
    assert!(
        response.gain_db < 65.0,
        "Gain should be reduced due to large feed offset, got {:.2} dBi",
        response.gain_db
    );

    // Loss should be substantial (multiple dB) due to feed aberrations
    let loss = response.loss_db.unwrap();
    assert!(
        loss > 2.0,
        "Loss should be > 2 dB due to feed aberrations, got {:.2} dB",
        loss
    );
}

#[test]
fn test_feed_steering_produces_different_gains() {
    // Create test calibration
    let mut repo = CalibrationRepository::new();
    repo.add_calibration(create_test_calibration("test_antenna", "x_band"));

    // Base parameters
    let vehicle_pos = Position3D::new(-19794863.29, -37228723.27, 0.0);
    let reflector_boresight = Position3D::new(-2485073.18, -4673742.90, 3546502.48);
    let emitter_pos = Position3D::new(-2485073.18, -4673742.90, 3546502.48);

    // Scenario 1: Feed at boresight
    let request1 = GainRequest {
        antenna_id: "test_antenna".to_string(),
        feed_id: "x_band".to_string(),
        vehicle_position: vehicle_pos.clone(),
        reflector_boresight: reflector_boresight.clone(),
        feed_position: reflector_boresight.clone(), // At boresight
        emitter_position: emitter_pos.clone(),
        frequency_mhz: 8450.0,
        pointing_frequency_mhz: None,
        include_reference: false,
    };

    // Scenario 2: Feed offset from boresight
    let request2 = GainRequest {
        antenna_id: "test_antenna".to_string(),
        feed_id: "x_band".to_string(),
        vehicle_position: vehicle_pos.clone(),
        reflector_boresight: reflector_boresight.clone(),
        feed_position: Position3D::new(-4831642.29, -1948496.21, 3667577.84), // Offset
        emitter_position: emitter_pos.clone(),
        frequency_mhz: 8450.0,
        pointing_frequency_mhz: None,
        include_reference: false,
    };

    let response1 = compute_gain_from_request(&request1, &repo).unwrap();
    let response2 = compute_gain_from_request(&request2, &repo).unwrap();

    println!("\n=== Feed Steering Comparison ===");
    println!("Feed at boresight:  {:.2} dBi", response1.gain_db);
    println!("Feed offset:        {:.2} dBi", response2.gain_db);
    println!("Difference:         {:.2} dB", response1.gain_db - response2.gain_db);

    // The two scenarios MUST produce different gains
    // This is the core bug fix - previously both produced the same gain
    assert!(
        (response1.gain_db - response2.gain_db).abs() > 1.0,
        "Feed steering should produce significantly different gains! \
         Boresight: {:.2} dBi, Offset: {:.2} dBi",
        response1.gain_db,
        response2.gain_db
    );

    // Feed at boresight should have higher gain
    assert!(
        response1.gain_db > response2.gain_db,
        "Feed at boresight should have higher gain than offset feed"
    );
}
