//! Per-integration wall-clock budget tests (roadmap S3)
//!
//! Verifies that `performance.integration_budget_ms` is actually enforced: a single
//! aperture integration that runs past the configured budget aborts with the typed
//! `ComputationError::TimeBudgetExceeded`, surfaced on the compute endpoints as
//! `504 Gateway Timeout` with the standard JSON `ErrorResponse` and machine code
//! `computation_budget_exceeded`.
//!
//! S3 caps ONE integral (cooperatively, inside the radial loop). It is distinct from S2's
//! `request_timeout_secs` (the whole-request wall-clock, code `request_timeout`) and from
//! S4's concurrency admission control. The two codes let ops tell "middleware gave up
//! waiting" from "a single integral was aborted."

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use antenna_model::config::ServiceConfig;
use antenna_model::model::coordinates_3d::geodetic_to_ecef;
use std::path::PathBuf;

/// Fixture config with a **tiny** integration budget (1 ms). The whole-request timeout is
/// left generous (30 s) so S3's per-integration budget — not S2's request timeout — is what
/// fires: the single heavy integration below costs far more than 1 ms but far less than 30 s.
fn tiny_budget_config() -> ServiceConfig {
    let mut cfg = ServiceConfig::with_defaults();
    cfg.server.host = "127.0.0.1".to_string();
    cfg.server.port = 0;
    cfg.server.max_body_size_bytes = 10_485_760;
    cfg.server.request_timeout_secs = 30;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixtures_dir = PathBuf::from(&manifest_dir).join("tests/fixtures");
    cfg.calibration.data_directory = fixtures_dir.clone();
    cfg.calibration.antenna_config_file = fixtures_dir.join("test_antennas.yaml");
    cfg.calibration.fail_fast = false;

    cfg.performance.worker_threads = 2;
    cfg.performance.max_batch_size = 1000;
    cfg.performance.enable_parallel_processing = true;
    // The knob under test: 1 ms bounds a single integration well below its real cost.
    cfg.performance.integration_budget_ms = 1;

    cfg
}

/// A single-gain request on the large (13 m) Ka-band offset-feed antenna with the emitter
/// ~69° off boresight. That wide-angle integration on a high-`D/λ` dish uses thousands of
/// radial samples — comfortably above the [`BUDGET_CHECK_STRIDE`] so the budget is polled
/// mid-loop — and costs ~100 ms, dwarfing the 1 ms budget by a wide, hardware-robust margin
/// (not exact-timing coupling).
fn heavy_off_axis_gain_request() -> GainRequest {
    let mut req = builders::multi_feed_request("ka_band", 26_000.0);
    // `simple_gain_request_ecef` aims the boresight at the original satellite
    // (-117, 35, 400 km); move the emitter to a satellite ~69° off boresight so the served
    // integration is wide-angle and heavy (verified to trip a 1 ms budget).
    let (x, y, z) = geodetic_to_ecef(-120.0, 30.0, 400_000.0).unwrap();
    req.emitter_position = Position3D {
        x,
        y,
        z,
        coordinate_system: Some(CoordinateSystem::ECEF),
    };
    req
}

/// A single over-budget integration on the `/gain` endpoint must return 504 with the
/// standard JSON body and machine code `computation_budget_exceeded`.
#[tokio::test]
async fn test_over_budget_single_gain_returns_504() {
    let server = TestServer::start_with_config(Some(tiny_budget_config()))
        .await
        .expect("Failed to start test server");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let request = heavy_off_axis_gain_request();
    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .unwrap();

    let status = response.status();
    assert_eq!(
        status, 504,
        "a single integration exceeding the budget must return 504"
    );

    let err: ErrorResponse = response.json().await.unwrap();
    assert_eq!(
        err.error, "computation_budget_exceeded",
        "the 504 body must carry the S3-specific machine code (distinct from S2's request_timeout)"
    );

    server.shutdown().await;
}

/// The generous default budget must leave a normal single-gain request succeeding — proving
/// the knob is a live abort, not an always-on failure. Uses a light near-boresight request
/// so the assertion is fast; the heavy over-budget path is exercised above.
#[tokio::test]
async fn test_within_budget_single_gain_succeeds() {
    let mut cfg = tiny_budget_config();
    cfg.performance.integration_budget_ms = 30_000; // generous default
    let server = TestServer::start_with_config(Some(cfg))
        .await
        .expect("Failed to start test server");

    let request = builders::multi_feed_request("ka_band", 26_000.0);
    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("gain computation should succeed under the generous budget");
    assert!(
        response.gain_db.is_finite(),
        "gain must be finite under the generous budget, got {}",
        response.gain_db
    );

    server.shutdown().await;
}
