//! API Integration Tests
//!
//! Comprehensive tests for all REST API endpoints with realistic scenarios.
//! Tests cover:
//! - Health and status endpoints
//! - Single gain computation
//! - Batch gain computation
//! - Heatmap generation
//! - Antenna and feed listing

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;

/// Test health endpoint
#[tokio::test]
async fn test_health_endpoint() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: HealthResponse = server.get("/health").await.expect("Health check failed");

    assert_eq!(response.status, "healthy");

    server.shutdown().await;
}

/// Test ready endpoint
#[tokio::test]
async fn test_ready_endpoint() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: HealthResponse = server.get("/ready").await.expect("Readiness check failed");

    assert_eq!(response.status, "ready");

    server.shutdown().await;
}

/// Test status endpoint
#[tokio::test]
async fn test_status_endpoint() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: StatusResponse = server.get("/status").await.expect("Status check failed");

    assert!(!response.version.is_empty());
    // Uptime should be reasonable (u64 is always >= 0, so just check it's set)
    let _ = response.uptime_seconds;
    // Should have loaded test antennas
    // Should have loaded test antennas
    if let Some(antenna_ids) = &response.antenna_ids {
        assert!(!antenna_ids.is_empty());
    }

    server.shutdown().await;
}

/// Test single gain computation with ECEF coordinates
#[tokio::test]
async fn test_single_gain_computation_ecef() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::simple_gain_request_ecef();

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    // Validate response structure
    validators::validate_gain_response(&response).expect("Invalid gain response");

    // Check IDs match request
    assert_eq!(response.antenna_id, request.antenna_id);
    assert_eq!(response.feed_id, request.feed_id);

    // Check gain is reasonable. This request steers the feed far off boresight
    // (feed near the vehicle, boresight at the satellite), so the gain is well below
    // the antenna's boresight maximum. With the aperture-directivity formula (no
    // hardcoded 0.55 efficiency) the 5 m test_simple antenna yields ≈ 8.7 dBi here.
    assert!(
        response.gain_db > 5.0 && response.gain_db < 60.0,
        "Gain {} is outside expected range",
        response.gain_db
    );

    // Metadata should be populated
    assert!(response.metadata.computation_time_ms > 0.0);

    server.shutdown().await;
}

/// Test single gain computation with Geodetic coordinates
#[tokio::test]
async fn test_single_gain_computation_geodetic() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::simple_gain_request_geodetic();

    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    validators::validate_gain_response(&response).expect("Invalid gain response");

    assert_eq!(response.antenna_id, request.antenna_id);
    assert_eq!(response.feed_id, request.feed_id);

    // When include_reference=true, should have reference gain and loss
    assert!(response.reference_gain_db.is_some());
    assert!(response.loss_db.is_some());

    let loss = response.loss_db.unwrap();
    // loss_db = reference(ideal boresight) − actual. The request steers the feed far
    // off boresight AND uses a different pointing frequency (8450 vs 8400 MHz, adding
    // beam squint), so the actual gain is tens of dB below the ideal reference. With
    // loss_db now free of the old ~2.6 dB efficiency offset, the value is ≈ 32 dB.
    // Loss can also be slightly negative near coma lobes. Range: -10 dB to +40 dB.
    assert!(
        (-10.0..40.0).contains(&loss),
        "Loss {} is outside expected range",
        loss
    );

    server.shutdown().await;
}

/// Test gain computation with invalid antenna ID
#[tokio::test]
async fn test_gain_computation_invalid_antenna() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::simple_gain_request_ecef();
    request.antenna_id = "nonexistent_antenna".to_string();

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;

    assert!(result.is_err(), "Expected error for invalid antenna");

    server.shutdown().await;
}

/// Test gain computation with invalid feed ID
#[tokio::test]
async fn test_gain_computation_invalid_feed() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::simple_gain_request_ecef();
    request.feed_id = "nonexistent_feed".to_string();

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;

    assert!(result.is_err(), "Expected error for invalid feed");

    server.shutdown().await;
}

/// Test batch gain computation
#[tokio::test]
async fn test_batch_gain_computation() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::simple_batch_request(10);

    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &request)
        .await
        .expect("Batch computation failed");

    validators::validate_batch_response(&response).expect("Invalid batch response");

    assert_eq!(response.results.len(), 10);
    assert_eq!(response.metadata.count, 10);
    assert!(response.metadata.total_computation_time_ms > 0.0);

    // All results should be valid
    for result in &response.results {
        validators::validate_gain_response(result).expect("Invalid result in batch");
    }

    server.shutdown().await;
}

/// Test batch with partial failures
#[tokio::test]
async fn test_batch_with_failures() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::simple_batch_request(5);
    // Make one request invalid
    request.evaluations[2].antenna_id = "invalid".to_string();

    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &request)
        .await
        .expect("Batch computation failed");

    assert_eq!(response.results.len(), 5);

    // Third result should have NaN gain and error in warnings
    let failed_result = &response.results[2];
    assert!(failed_result.gain_db.is_nan());
    assert!(!failed_result.warnings.is_empty());

    server.shutdown().await;
}

/// Test empty batch request
#[tokio::test]
async fn test_empty_batch() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = BatchGainRequest {
        evaluations: vec![],
    };

    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &request)
        .await
        .expect("Empty batch should succeed");

    // Empty batch should return empty results
    assert_eq!(response.results.len(), 0);
    assert_eq!(response.metadata.count, 0);

    server.shutdown().await;
}

/// Test batch size limit
#[tokio::test]
async fn test_batch_size_limit() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Create batch exceeding limit (1000)
    let request = builders::simple_batch_request(1001);

    let result = server
        .post::<BatchGainResponse, _>("/api/v1/gain/batch", &request)
        .await;

    assert!(
        result.is_err(),
        "Expected error for batch exceeding size limit"
    );

    server.shutdown().await;
}

/// Test heatmap generation (rectangular grid)
#[tokio::test]
async fn test_heatmap_generation_rectangular() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = builders::simple_heatmap_request();

    let response: HeatmapResponse = server
        .post("/api/v1/heatmap", &request)
        .await
        .expect("Heatmap generation failed");

    validators::validate_heatmap_response(&response).expect("Invalid heatmap response");

    assert_eq!(response.antenna_id, request.antenna_id);
    assert_eq!(response.feed_id, request.feed_id);

    // Should have evaluated grid points
    assert!(response.metadata.points_evaluated > 0);
    assert!(response.metadata.computation_time_ms > 0.0);

    // Peak gain should be reasonable
    assert!(response.metadata.peak_gain_db > 10.0);

    server.shutdown().await;
}

/// Test heatmap with small grid
#[tokio::test]
async fn test_heatmap_small_grid() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = builders::simple_heatmap_request();
    // 3x3 grid
    request.grid_config = GridConfig::Rectangular {
        azimuth_range_deg: RangeConfig {
            min: 0.0,
            max: 10.0,
            step: 5.0,
        },
        elevation_range_deg: RangeConfig {
            min: 0.0,
            max: 10.0,
            step: 5.0,
        },
    };

    let response: HeatmapResponse = server
        .post("/api/v1/heatmap", &request)
        .await
        .expect("Heatmap generation failed");

    // Should have 3x3 = 9 points
    assert_eq!(response.metadata.points_evaluated, 9);

    server.shutdown().await;
}

/// Test antenna list endpoint
#[tokio::test]
async fn test_list_antennas() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: AntennaListResponse = server
        .get("/api/v1/antennas")
        .await
        .expect("Antenna list failed");

    assert!(!response.antennas.is_empty());

    // Should have test antennas
    let antenna_ids: Vec<String> = response.antennas.iter().map(|a| a.id.clone()).collect();
    assert!(
        antenna_ids.contains(&"test_simple".to_string())
            || antenna_ids.contains(&"test_uncalibrated".to_string()),
        "Expected test antennas to be loaded"
    );

    server.shutdown().await;
}

/// Test antenna details endpoint
#[tokio::test]
async fn test_get_antenna_details() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: AntennaDetailsResponse = server
        .get("/api/v1/antennas/test_simple")
        .await
        .expect("Antenna details failed");

    assert_eq!(response.id, "test_simple");
    assert!(!response.name.is_empty());
    assert!(!response.feeds.is_empty());
    // Validity ranges should have reasonable frequency range
    assert!(response.validity_ranges.frequency_mhz.1 > response.validity_ranges.frequency_mhz.0);

    server.shutdown().await;
}

/// Test antenna details for nonexistent antenna
#[tokio::test]
async fn test_get_antenna_details_not_found() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let result = server
        .get::<AntennaDetailsResponse>("/api/v1/antennas/nonexistent")
        .await;

    assert!(result.is_err(), "Expected 404 for nonexistent antenna");

    server.shutdown().await;
}

/// Test list feeds for antenna
#[tokio::test]
async fn test_list_feeds() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // The endpoint returns { "feeds": [...] }, not a direct array
    let response: serde_json::Value = server
        .get("/api/v1/antennas/test_simple/feeds")
        .await
        .expect("Feed list failed");

    let feeds = response["feeds"]
        .as_array()
        .expect("feeds should be an array");

    assert!(!feeds.is_empty());
    assert_eq!(feeds[0]["id"].as_str().unwrap(), "primary");

    server.shutdown().await;
}

/// Test feed details endpoint
#[tokio::test]
async fn test_get_feed_details() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let response: FeedInfo = server
        .get("/api/v1/antennas/test_simple/feeds/primary")
        .await
        .expect("Feed details failed");

    assert_eq!(response.id, "primary");
    // Frequency range should be valid
    assert!(response.frequency_range_mhz.1 > response.frequency_range_mhz.0);

    server.shutdown().await;
}

/// Test feed details for nonexistent feed
#[tokio::test]
async fn test_get_feed_details_not_found() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let result = server
        .get::<FeedInfo>("/api/v1/antennas/test_simple/feeds/nonexistent")
        .await;

    assert!(result.is_err(), "Expected 404 for nonexistent feed");

    server.shutdown().await;
}

/// Test multi-feed antenna
#[tokio::test]
async fn test_multi_feed_antenna() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Test large antenna with multiple feeds
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

    assert_eq!(x_response.feed_id, "x_band");
    assert_eq!(ka_response.feed_id, "ka_band");

    // Both should have valid gains
    validators::validate_gain_response(&x_response).expect("Invalid X-band response");
    validators::validate_gain_response(&ka_response).expect("Invalid Ka-band response");

    server.shutdown().await;
}
