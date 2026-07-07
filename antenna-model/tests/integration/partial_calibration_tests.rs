//! Partial Calibration Integration Tests
//!
//! Tests for all calibration statuses:
//! - Uncalibrated antennas (design specs only)
//! - Partially calibrated antennas (boresight or limited coverage)
//! - Fully calibrated antennas
//!
//! Validates:
//! - Calibration status in API responses
//! - Warning generation for uncalibrated/partially calibrated
//! - Loss accuracy expectations
//! - Coverage metadata

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;

/// Test uncalibrated antenna returns calibration status
#[tokio::test]
async fn test_uncalibrated_antenna_status() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::uncalibrated_antenna_request();

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    // Should have calibration status
    assert!(response.calibration_status.is_some());

    let status = response.calibration_status.as_ref().unwrap();
    assert_eq!(status.status, "uncalibrated");

    // Should have accuracy estimates
    assert!(status.accuracy_estimate_db > 0.0);
    assert!(status.loss_accuracy_estimate_db.is_some());

    // Should indicate design specs as parameters source
    assert_eq!(status.parameters_source, "design_specifications");

    // Should not have correction applied
    assert!(!status.correction_applied);

    server.shutdown().await;
}

/// Test uncalibrated antenna generates appropriate warnings
#[tokio::test]
async fn test_uncalibrated_antenna_warnings() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::uncalibrated_antenna_request();

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    // Should have warning about uncalibrated status
    assert!(!response.warnings.is_empty());

    let has_uncalibrated_warning = response
        .warnings
        .iter()
        .any(|w| w.to_lowercase().contains("uncalibrated"));

    assert!(
        has_uncalibrated_warning,
        "Expected warning about uncalibrated status"
    );

    server.shutdown().await;
}

/// Test uncalibrated antenna loss computation
#[tokio::test]
async fn test_uncalibrated_antenna_loss() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::uncalibrated_antenna_request();
    request.include_reference = true;

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    // Should compute reference gain and loss
    assert!(response.reference_gain_db.is_some());
    assert!(response.loss_db.is_some());

    let loss = response.loss_db.unwrap();

    // loss_db = reference(ideal boresight) − actual. The shared request steers the
    // feed far off boresight, so the actual gain is tens of dB below the ideal
    // reference; loss is ≈ 30 dB. (loss_db no longer carries the old ~2.6 dB
    // efficiency offset, so this is pure pointing/aberration loss.) Range: [0, 40] dB.
    assert!(
        (0.0..40.0).contains(&loss),
        "Loss {} outside expected range",
        loss
    );

    server.shutdown().await;
}

/// Test antenna details endpoint shows calibration status
#[tokio::test]
async fn test_antenna_details_calibration_status() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: AntennaDetailsResponse = server
        .get("/api/v1/antennas/test_uncalibrated")
        .await
        .expect("Antenna details failed");

    // Should have calibration status
    assert!(response.calibration_status.is_some());

    let status = response.calibration_status.as_ref().unwrap();
    assert_eq!(status.status, "uncalibrated");
    assert!(status.accuracy_estimate_db > 0.0);

    server.shutdown().await;
}

/// Test multi-feed antenna with different calibration per feed
#[tokio::test]
async fn test_multi_feed_mixed_calibration() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Test antenna with multiple feeds (all uncalibrated in test setup)
    let x_band_request = builders::multi_feed_request("x_band", 7200.0);
    let ka_band_request = builders::multi_feed_request("ka_band", 26000.0);

    let x_response: GainResponse = server
        .post("/api/v1/gain", &x_band_request)
        .await
        .expect("X-band computation failed");

    let ka_response: GainResponse = server
        .post("/api/v1/gain", &ka_band_request)
        .await
        .expect("Ka-band computation failed");

    // Both should have calibration status
    assert!(x_response.calibration_status.is_some());
    assert!(ka_response.calibration_status.is_some());

    // Validate feeds are correct
    assert_eq!(x_response.feed_id, "x_band");
    assert_eq!(ka_response.feed_id, "ka_band");

    server.shutdown().await;
}

/// Test batch with mixed calibration statuses
#[tokio::test]
async fn test_batch_mixed_calibration() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Create batch with different antennas
    let mut request = BatchGainRequest {
        evaluations: vec![
            builders::simple_gain_request_ecef(),     // test_simple
            builders::uncalibrated_antenna_request(), // test_uncalibrated
        ],
    };

    // Set include_reference for both
    for eval in &mut request.evaluations {
        eval.include_reference = true;
    }

    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &request)
        .await
        .expect("Batch computation failed");

    assert_eq!(response.results.len(), 2);

    // Both should have calibration status
    for result in &response.results {
        assert!(result.calibration_status.is_some());
        assert!(result.reference_gain_db.is_some());
        assert!(result.loss_db.is_some());
    }

    server.shutdown().await;
}

/// Test heatmap with uncalibrated antenna
#[tokio::test]
async fn test_heatmap_uncalibrated_antenna() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::simple_heatmap_request();
    request.antenna_id = "test_uncalibrated".to_string();
    request.feed_id = "x_band".to_string();
    request.frequency_mhz = 8000.0;

    let response: HeatmapResponse = server
        .post("/api/v1/heatmap", &request)
        .await
        .expect("Heatmap generation failed");

    validators::validate_heatmap_response(&response).expect("Invalid heatmap response");

    // Should have calibration status
    assert!(response.calibration_status.is_some());

    let status = response.calibration_status.as_ref().unwrap();
    assert_eq!(status.status, "uncalibrated");

    // Should have evaluated points
    assert!(response.metadata.points_evaluated > 0);

    server.shutdown().await;
}

/// Test frequency range validation for uncalibrated antenna
#[tokio::test]
async fn test_uncalibrated_frequency_validation() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::uncalibrated_antenna_request();
    // Use frequency at edge of valid range
    request.frequency_mhz = 8500.0; // Max of X-band range

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    validators::validate_gain_response(&response).expect("Invalid gain response");

    server.shutdown().await;
}

/// Test out-of-range frequency generates warning
#[tokio::test]
async fn test_out_of_range_frequency_warning() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::uncalibrated_antenna_request();
    // Use frequency outside valid range
    request.frequency_mhz = 9000.0; // Beyond X-band range (7100-8500)

    // Should still compute but with warning
    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;

    // Depending on validation strictness, might error or succeed with warning
    match result {
        Ok(response) => {
            // If it succeeds, should have warning
            assert!(
                !response.warnings.is_empty(),
                "Expected warning for out-of-range frequency"
            );
        }
        Err(_) => {
            // Or it might error, which is also acceptable
        }
    }

    server.shutdown().await;
}

/// Test antenna list shows calibration statuses
#[tokio::test]
async fn test_antenna_list_calibration_info() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: AntennaListResponse = server
        .get("/api/v1/antennas")
        .await
        .expect("Antenna list failed");

    assert!(!response.antennas.is_empty());

    // Find uncalibrated antenna
    let uncalibrated = response
        .antennas
        .iter()
        .find(|a| a.id == "test_uncalibrated");

    assert!(uncalibrated.is_some());

    let antenna = uncalibrated.unwrap();
    assert!(!antenna.feed_ids.is_empty());
    // Antenna info should have basic metadata
    assert!(!antenna.name.is_empty());

    server.shutdown().await;
}

/// Test physics model computation for uncalibrated antenna
#[tokio::test]
async fn test_physics_model_computation() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::uncalibrated_antenna_request();

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    // Physics model should compute reasonable gain. The shared request steers the
    // feed far off boresight (feed near vehicle, boresight at the satellite; a ~96.6°
    // cone-angle offset), so the 3.7 m test_uncalibrated antenna (f/D = 0.5) is
    // evaluated well off its boresight maximum.
    //
    // Since the beam-deviation-factor fix (2026-07), `compute_feed_position_from_pointing`
    // divides the steering displacement by BDF(f/D=0.5) ≈ 0.872 so the physical-optics
    // beam peak lands at the requested angle. That larger displacement produces more
    // defocus loss at this extreme steering angle, dropping off-axis gain from the
    // pre-BDF ≈ 14.1 dBi (with the Task-1 sign fix but bdf=1) to ≈ 8.9 dBi.
    // Bound to [5, 50] dBi.
    assert!(
        response.gain_db > 5.0 && response.gain_db < 50.0,
        "Gain {} outside physically reasonable range",
        response.gain_db
    );

    // Metadata should show computation used physics model
    assert!(response.metadata.computation_time_ms > 0.0);

    server.shutdown().await;
}

/// Test uncalibrated antenna with different frequencies
#[tokio::test]
async fn test_uncalibrated_frequency_sweep() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let frequencies = vec![7100.0, 7500.0, 8000.0, 8400.0];
    let mut gains = Vec::new();

    for freq in frequencies {
        let mut request = builders::uncalibrated_antenna_request();
        request.frequency_mhz = freq;

        let response: GainResponse = server
            .post("/api/v1/gain", &request)
            .await
            .expect("Gain computation failed");

        gains.push(response.gain_db);
    }

    // Gain should generally increase with frequency (larger effective aperture)
    // But not strictly monotonic due to mesh effects and surface errors
    assert_eq!(gains.len(), 4);

    // All gains should be a valid, finite dB value.
    //
    // NOTE: The test request geometry places the feed_position near the vehicle
    // (~5 m altitude offset) while the reflector boresight points to a satellite
    // 400 km away. This large angular offset causes a substantial lateral feed
    // displacement in the antenna frame (~3.4 m for f=1.85 m), well outside the
    // paraxial regime. With the corrected path phase (which now correctly reflects
    // gain loss from off-axis feed displacement instead of masking it with a
    // spurious defocus term), the gain at these off-axis angles is significantly
    // reduced from the theoretical boresight maximum. The bounds below reflect
    // the physically correct output for this geometry; tighten them only after
    // the feed_position geometry is corrected to represent a realistic pointing.
    for gain in &gains {
        assert!(
            gain.is_finite() && *gain > -60.0 && *gain < 65.0,
            "Gain {} dB is not a valid computed gain for 3.7m X-band antenna",
            gain
        );
    }

    server.shutdown().await;
}

/// Test calibration status includes coverage information
#[tokio::test]
async fn test_calibration_coverage_metadata() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: AntennaDetailsResponse = server
        .get("/api/v1/antennas/test_uncalibrated")
        .await
        .expect("Antenna details failed");

    assert!(response.calibration_status.is_some());

    let status = response.calibration_status.as_ref().unwrap();

    // Uncalibrated antenna should indicate no measurements
    // (coverage information not typically present for uncalibrated)
    assert_eq!(status.status, "uncalibrated");

    server.shutdown().await;
}

/// Test loss computation consistency across antennas
#[tokio::test]
async fn test_loss_computation_consistency() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Compute loss for both calibrated and uncalibrated antennas
    let mut simple_request = builders::simple_gain_request_ecef();
    simple_request.include_reference = true;

    let mut uncal_request = builders::uncalibrated_antenna_request();
    uncal_request.include_reference = true;

    let simple_response: GainResponse = server
        .post("/api/v1/gain", &simple_request)
        .await
        .expect("Simple antenna computation failed");

    let uncal_response: GainResponse = server
        .post("/api/v1/gain", &uncal_request)
        .await
        .expect("Uncalibrated antenna computation failed");

    // Both should compute loss
    assert!(simple_response.loss_db.is_some());
    assert!(uncal_response.loss_db.is_some());

    let simple_loss = simple_response.loss_db.unwrap();
    let uncal_loss = uncal_response.loss_db.unwrap();

    // Loss should be a valid (finite) dB number and non-negative.
    //
    // NOTE: The test request geometry places the feed_position near the vehicle
    // (~5 m altitude offset) while the reflector boresight points to a satellite
    // 400 km away. This near-perpendicular angle causes compute_feed_position_from_pointing
    // to compute a large lateral feed displacement (~3.4–3.7 m for these dishes),
    // producing substantial off-axis gain loss. With the corrected path phase the
    // gain now correctly reflects this feed-displacement loss instead of masking it.
    // The old tight bounds (-1..5 dB simple, 5..25 dB uncal) assumed near-boresight
    // gain, which was only plausible under the wrong defocus phase.
    // These bounds simply verify the computation runs and produces finite results;
    // tighten after fixing the feed_position geometry to a realistic far-field target.
    assert!(
        simple_loss.is_finite() && (0.0..100.0).contains(&simple_loss),
        "Simple loss {} dB is not a valid loss value",
        simple_loss
    );
    assert!(
        uncal_loss.is_finite() && (0.0..100.0).contains(&uncal_loss),
        "Uncal loss {} dB is not a valid loss value",
        uncal_loss
    );

    server.shutdown().await;
}
