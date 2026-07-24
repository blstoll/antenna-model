//! Integration tests for server startup and status endpoint
//!
//! These tests verify that the server can start, respond to requests,
//! and shut down gracefully.

use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;

/// Build a config pointing at the checked-in test fixtures, on the given port.
///
/// `antenna_model::api::start_server` (the plain host/port convenience wrapper these tests
/// used to call) resolves `ServiceConfig::with_defaults()`'s calibration paths
/// (`calibration_data/antennas.yaml`) relative to the process CWD, which `cargo test` sets
/// to the crate directory (`antenna-model/`) — not the workspace root where
/// `calibration_data/` actually lives. Before roadmap S5 that load failure was silently
/// swallowed and the server started with an empty repository anyway; now that
/// `calibration.fail_fast` (default `true`) is honored, that same failure legitimately
/// aborts startup. So these tests build an explicit config against
/// `tests/fixtures/test_antennas.yaml` instead.
///
/// `data_directory` is `CARGO_MANIFEST_DIR` itself (not the fixtures subdirectory):
/// `test_antennas.yaml`'s `calibration_file` entries are written as
/// `tests/fixtures/calibration_data/...`, i.e. relative to the crate root, so that's what
/// `data_directory` must be for every entry to resolve. `fail_fast` is left at its shipped
/// default (`true`) deliberately — this is the only test in the suite that drives the real
/// production startup path (`start_server_with_config`), so it doubles as a regression
/// guard for the exact knob this roadmap unit exists to honor.
fn test_config(port: u16) -> antenna_model::config::ServiceConfig {
    let mut config = antenna_model::config::ServiceConfig::with_defaults();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = port;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    config.calibration.data_directory = PathBuf::from(&manifest_dir);
    config.calibration.antenna_config_file =
        PathBuf::from(&manifest_dir).join("tests/fixtures/test_antennas.yaml");
    // fail_fast left at its shipped default (true) — see doc comment above.

    config
}

/// Test that the server can start and is accessible
///
/// This test verifies:
/// - Server binds to the configured port
/// - Status endpoint returns 200 OK
/// - Response contains expected JSON fields
#[tokio::test]
async fn test_server_startup_and_status() {
    // Start the server in a background task
    let server_handle = tokio::spawn(async {
        antenna_model::api::start_server_with_config(test_config(3001))
            .await
            .expect("Failed to start server");
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Make a request to the status endpoint
    let client = reqwest::Client::new();
    let response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:3001/status").send(),
    )
    .await
    .expect("Request timed out")
    .expect("Request failed");

    // Verify response
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json; charset=utf-8"
    );

    // Parse JSON body
    let json: serde_json::Value = response.json().await.expect("Failed to parse JSON");

    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
    assert!(json["uptime_seconds"].is_u64());

    // Proves `set_antenna_ids()` ran in `start_server_with_config` on the Healthy path
    // (roadmap S5): before that wiring, /status never reported a populated antenna_count.
    assert!(
        json["antenna_count"].as_u64().unwrap_or(0) > 0,
        "expected /status to report loaded antennas, got: {json}"
    );

    // Proves `state.mark_ready()` ran in `start_server_with_config` on the Healthy path
    // (roadmap S5): readiness starts false and is earned only by a completed healthy load.
    let ready_response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:3001/ready").send(),
    )
    .await
    .expect("Ready request timed out")
    .expect("Ready request failed");
    assert_eq!(
        ready_response.status(),
        200,
        "expected /ready to be 200 after a healthy calibration load"
    );

    // Abort the server task
    server_handle.abort();
}

/// Test that uptime increases over time
#[tokio::test]
async fn test_status_uptime_increases() {
    // Start the server
    let server_handle = tokio::spawn(async {
        antenna_model::api::start_server_with_config(test_config(3002))
            .await
            .expect("Failed to start server");
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    let client = reqwest::Client::new();

    // First request
    let response1 = client
        .get("http://127.0.0.1:3002/status")
        .send()
        .await
        .expect("First request failed");
    let json1: serde_json::Value = response1.json().await.expect("Failed to parse JSON");
    let uptime1 = json1["uptime_seconds"].as_u64().expect("Invalid uptime");

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(1100)).await;

    // Second request
    let response2 = client
        .get("http://127.0.0.1:3002/status")
        .send()
        .await
        .expect("Second request failed");
    let json2: serde_json::Value = response2.json().await.expect("Failed to parse JSON");
    let uptime2 = json2["uptime_seconds"].as_u64().expect("Invalid uptime");

    // Uptime should have increased by at least 1 second
    assert!(uptime2 > uptime1);

    server_handle.abort();
}
