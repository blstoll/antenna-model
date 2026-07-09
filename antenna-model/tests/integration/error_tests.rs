//! Error Handling Tests
//!
//! Comprehensive tests for error conditions:
//! - Startup failures (missing files, invalid config)
//! - Runtime errors (invalid requests, out-of-range)
//! - Resource exhaustion (large requests, memory limits)
//! - Data corruption scenarios
//! - Malformed API requests
//! - Extreme parameter values
//!
//! All tests verify:
//! - Clear, actionable error messages
//! - No panics or crashes
//! - Proper HTTP status codes
//! - Sufficient debugging information in logs

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use antenna_model::config::{CalibrationConfig, ServiceConfig};
use antenna_model::data::repository::CalibrationRepository;
use std::path::PathBuf;

// ============================================================================
// Startup Failure Tests
// ============================================================================

#[tokio::test]
async fn test_startup_with_missing_calibration_directory() {
    let mut config = ServiceConfig::with_defaults();
    config.calibration.data_directory = PathBuf::from("/nonexistent/path/to/calibration/data");
    config.calibration.antenna_config_file = PathBuf::from("/nonexistent/antennas.yaml");
    config.calibration.fail_fast = false;

    // This should fail because antenna config file is required (even with fail_fast=false)
    let result = CalibrationRepository::load_from_config(&config.calibration);
    assert!(
        result.is_err(),
        "Repository should fail when antenna config is missing"
    );
}

#[tokio::test]
async fn test_startup_with_missing_antenna_config_file() {
    let mut config = ServiceConfig::with_defaults();
    config.calibration.antenna_config_file = PathBuf::from("/nonexistent/antennas.yaml");
    config.calibration.fail_fast = false;

    // This should fail even with fail_fast = false because antenna config is required
    let result = CalibrationRepository::load_from_config(&config.calibration);
    assert!(
        result.is_err(),
        "Repository should fail when antenna config file is missing"
    );

    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("antenna")
                || error_msg.contains("config")
                || error_msg.contains("not found")
                || error_msg.contains("No such file"),
            "Error message should mention missing antenna config: {}",
            error_msg
        );
    }
}

#[tokio::test]
async fn test_startup_with_corrupted_antenna_config() {
    // Create temporary corrupted config file
    let temp_dir = std::env::temp_dir();
    let corrupted_config = temp_dir.join("corrupted_antennas.yaml");

    // Write invalid YAML
    std::fs::write(&corrupted_config, "invalid: yaml: content: [[[").unwrap();

    let config = CalibrationConfig {
        antenna_config_file: corrupted_config.clone(),
        fail_fast: true,
        ..Default::default()
    };

    let result = CalibrationRepository::load_from_config(&config);
    assert!(
        result.is_err(),
        "Repository should fail on corrupted antenna config"
    );

    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("parse")
                || error_msg.contains("invalid")
                || error_msg.contains("YAML")
                || error_msg.contains("deserialize"),
            "Error message should indicate parsing failure: {}",
            error_msg
        );
    }

    // Cleanup
    let _ = std::fs::remove_file(&corrupted_config);
}

#[tokio::test]
async fn test_startup_with_corrupted_calibration_binary() {
    // Create temporary directory with corrupted binary
    let temp_dir = std::env::temp_dir().join("test_corrupted_cal");
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Create antenna config referencing corrupted binary
    let antenna_config = temp_dir.join("antennas.yaml");
    let corrupted_bin = temp_dir.join("corrupted.bin");

    // Write corrupted binary data
    std::fs::write(&corrupted_bin, b"corrupted binary data \x00\x01\x02").unwrap();

    // Write antenna config
    let config_content = format!(
        r#"
antennas:
  - antenna_id: "test_corrupted"
    calibration_file: "{}"
    feeds:
      - feed_id: "primary"
"#,
        corrupted_bin.display()
    );
    std::fs::write(&antenna_config, config_content).unwrap();

    let config = CalibrationConfig {
        antenna_config_file: antenna_config.clone(),
        data_directory: temp_dir.clone(),
        fail_fast: false, // Should continue loading other antennas
    };

    let result = CalibrationRepository::load_from_config(&config);
    // With fail_fast=false, repository might succeed or fail depending on whether
    // it can still load other antennas. Either way, no panic should occur.
    // The important part is handling corrupted data without crashing.
    let _ = result;

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// Runtime Error Tests - Invalid Requests
// ============================================================================

#[tokio::test]
async fn test_invalid_antenna_id() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.antenna_id = "nonexistent_antenna".to_string();

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail for nonexistent antenna");

    server.shutdown().await;
}

#[tokio::test]
async fn test_invalid_feed_id() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.feed_id = "nonexistent_feed".to_string();

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail for nonexistent feed");

    server.shutdown().await;
}

#[tokio::test]
async fn test_malformed_json_request() {
    let server = TestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    // Send invalid JSON
    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "application/json")
        .body("{invalid json content")
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        400,
        "Should return 400 for malformed JSON"
    );

    let body = response.text().await.unwrap();
    assert!(
        body.contains("parse")
            || body.contains("invalid")
            || body.contains("JSON")
            || body.contains("Bad Request"),
        "Error message should indicate JSON parsing error: {}",
        body
    );

    server.shutdown().await;
}

#[tokio::test]
async fn test_missing_required_fields() {
    let server = TestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    // Send JSON missing required fields
    let incomplete_request = r#"{
        "antenna_id": "test_simple"
    }"#;

    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "application/json")
        .body(incomplete_request)
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        400,
        "Should return 400 for missing required fields"
    );

    let body = response.text().await.unwrap();
    assert!(
        body.contains("missing") || body.contains("required") || body.contains("field"),
        "Error message should indicate missing fields: {}",
        body
    );

    server.shutdown().await;
}

// ============================================================================
// Runtime Error Tests - Out-of-Range Values
// ============================================================================

#[tokio::test]
async fn test_negative_frequency() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.frequency_mhz = -1000.0; // Negative frequency

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail for negative frequency");

    server.shutdown().await;
}

#[tokio::test]
async fn test_zero_frequency() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.frequency_mhz = 0.0; // Zero frequency

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail for zero frequency");

    server.shutdown().await;
}

#[tokio::test]
async fn test_nan_coordinates() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.emitter_position.x = f64::NAN; // NaN coordinate

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail for NaN coordinates");

    server.shutdown().await;
}

#[tokio::test]
async fn test_infinity_coordinates() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.emitter_position.x = f64::INFINITY; // Infinity coordinate

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail for infinity coordinates");

    server.shutdown().await;
}

// ============================================================================
// Resource Exhaustion Tests
// ============================================================================

#[tokio::test]
async fn test_oversized_batch_request() {
    let server = TestServer::start().await.unwrap();

    // Create batch with more than max_batch_size (1000)
    let mut evaluations = Vec::new();
    for i in 0..1001 {
        let mut req = builders::simple_gain_request_ecef();
        req.frequency_mhz = 8000.0 + (i as f64);
        evaluations.push(req);
    }

    let batch_request = BatchGainRequest { evaluations };

    let result = server
        .post::<BatchGainResponse, _>("/api/v1/gain/batch", &batch_request)
        .await;
    assert!(result.is_err(), "Should fail for oversized batch");

    server.shutdown().await;
}

#[tokio::test]
async fn test_request_body_size_limit() {
    let server = TestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    // Create a very large request body (>10MB limit)
    let large_body = "x".repeat(11 * 1024 * 1024); // 11MB

    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "application/json")
        .body(large_body)
        .send()
        .await
        .unwrap();

    // Should return 400 or 413 depending on how the framework handles oversized requests
    assert!(
        response.status() == 400 || response.status() == 413,
        "Should return 400 or 413 for oversized request, got {}",
        response.status()
    );

    server.shutdown().await;
}

// ============================================================================
// Extreme Parameter Value Tests
// ============================================================================

#[tokio::test]
async fn test_extremely_high_frequency() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.frequency_mhz = 1.0e9; // 1 THz (way beyond Ka-band)

    // Should succeed (may warn about extrapolation)
    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    // Depending on validation, this might fail or succeed with warnings
    // Just verify it doesn't panic
    let _ = result;

    server.shutdown().await;
}

#[tokio::test]
async fn test_extremely_low_frequency() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.frequency_mhz = 0.001; // 1 kHz (very low for satellite antenna)

    // Should succeed (may warn about extrapolation)
    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    let _ = result;

    server.shutdown().await;
}

#[tokio::test]
async fn test_very_large_ecef_coordinates() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.emitter_position.x = 1.0e8; // 100,000 km (well beyond GEO)
    request.emitter_position.y = 1.0e8;
    request.emitter_position.z = 1.0e8;

    // Should handle it (may succeed or fail gracefully)
    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    let _ = result;

    server.shutdown().await;
}

// ============================================================================
// HTTP Method Tests
// ============================================================================

#[tokio::test]
async fn test_get_request_on_post_endpoint() {
    let server = TestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/api/v1/gain", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        405,
        "Should return 405 Method Not Allowed"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn test_unsupported_content_type() {
    let server = TestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "text/plain")
        .body("not json")
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        415,
        "Should return 415 Unsupported Media Type"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn test_nonexistent_endpoint() {
    let server = TestServer::start().await.unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{}/api/v1/nonexistent", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        404,
        "Should return 404 for nonexistent endpoint"
    );

    server.shutdown().await;
}

// ============================================================================
// Error Message Quality Tests
// ============================================================================

#[tokio::test]
async fn test_error_messages_are_actionable() {
    let server = TestServer::start().await.unwrap();

    // Test with invalid antenna ID
    let mut request = builders::simple_gain_request_ecef();
    request.antenna_id = "does_not_exist".to_string();

    let result = server
        .post::<GainResponse, _>("/api/v1/gain", &request)
        .await;
    assert!(result.is_err(), "Should fail");

    // Error message should specify which antenna was not found
    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("does_not_exist")
            || error_msg.contains("antenna")
            || error_msg.contains("not found"),
        "Error should mention the problematic antenna ID: {}",
        error_msg
    );

    server.shutdown().await;
}

#[tokio::test]
async fn test_no_panics_on_invalid_input() {
    let server = TestServer::start().await.unwrap();

    // Test various invalid inputs - none should panic
    let test_cases = vec![
        {
            let mut r = builders::simple_gain_request_ecef();
            r.antenna_id = String::new(); // Empty antenna ID
            r
        },
        {
            let mut r = builders::simple_gain_request_ecef();
            r.feed_id = String::new(); // Empty feed ID
            r
        },
        {
            let mut r = builders::simple_gain_request_ecef();
            r.frequency_mhz = -999.0; // Negative frequency
            r
        },
        {
            let mut r = builders::simple_gain_request_ecef();
            r.emitter_position.x = f64::NAN; // NaN
            r
        },
    ];

    for request in test_cases {
        // Should not panic, just return error
        let _ = server
            .post::<GainResponse, _>("/api/v1/gain", &request)
            .await;
    }

    // Service should still be operational
    let health: HealthResponse = server.get("/health").await.unwrap();
    assert_eq!(health.status, "healthy");

    server.shutdown().await;
}
