//! Request Timeout Tests (roadmap S2)
//!
//! Verifies that `server.request_timeout_secs` is actually enforced: a request
//! whose processing exceeds the configured timeout returns `408 Request Timeout`
//! with the project's standard JSON `ErrorResponse` body.
//!
//! The heavy compute path (heatmap/batch/h3) runs rayon synchronously; the
//! handlers offload it to `tokio::task::spawn_blocking` so the async task yields
//! at a real `.await`, letting the timeout middleware fire. Note (honest
//! limitation): the timeout bounds the *response*, not the background compute —
//! the rayon work is not cancelled and runs to completion (see S3).

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use antenna_model::config::ServiceConfig;
use std::path::PathBuf;

/// Build a ServiceConfig pointed at the integration test fixtures with a
/// caller-supplied request timeout (seconds).
fn config_with_timeout(request_timeout_secs: u64) -> ServiceConfig {
    let mut cfg = ServiceConfig::with_defaults();
    cfg.server.host = "127.0.0.1".to_string();
    cfg.server.port = 0;
    cfg.server.max_body_size_bytes = 10_485_760;
    cfg.server.request_timeout_secs = request_timeout_secs;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixtures_dir = PathBuf::from(&manifest_dir).join("tests/fixtures");
    cfg.calibration.data_directory = fixtures_dir.clone();
    cfg.calibration.antenna_config_file = fixtures_dir.join("test_antennas.yaml");
    cfg.calibration.fail_fast = false;

    cfg.performance.worker_threads = 2;
    cfg.performance.max_batch_size = 1000;
    cfg.performance.enable_parallel_processing = true;

    cfg
}

/// A deliberately heavy heatmap request: the large (13 m) Ka-band offset-feed
/// antenna is the most expensive per-point integration in the fixtures (high
/// D/λ, wide-angle coma). A 23x23 grid spanning the full 0-45 deg quadrant keeps
/// the expensive wide-angle points while bounding the point count, so the
/// compute reliably exceeds a 1-second timeout (empirically several seconds)
/// without spawning minutes of un-cancellable background rayon work.
fn heavy_heatmap_request() -> HeatmapRequest {
    let mut req = builders::simple_heatmap_request();
    req.antenna_id = "test_large".to_string();
    req.feed_id = "ka_band".to_string();
    req.frequency_mhz = 26_000.0;
    req.grid_config = GridConfig::Rectangular {
        azimuth_range_deg: RangeConfig {
            min: 0.0,
            max: 45.0,
            step: 2.0,
        },
        elevation_range_deg: RangeConfig {
            min: 0.0,
            max: 45.0,
            step: 2.0,
        },
    };
    req
}

#[tokio::test]
async fn test_heavy_heatmap_times_out_with_408() {
    // Tiny timeout (1 s, the smallest the whole-second config permits) against a
    // heavy heatmap that reliably takes longer to compute.
    let config = config_with_timeout(1);
    let server = TestServer::start_with_config(Some(config)).await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let request = heavy_heatmap_request();

    let start = std::time::Instant::now();
    let response = client
        .post(format!("{}/api/v1/heatmap", server.base_url))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(
        response.status(),
        408,
        "a heatmap exceeding request_timeout_secs must return 408 Request Timeout (elapsed {:?})",
        elapsed
    );

    let err: ErrorResponse = response.json().await.unwrap();
    assert_eq!(
        err.error, "request_timeout",
        "timeout body must be the standard ErrorResponse with code request_timeout"
    );

    server.shutdown().await;
}
