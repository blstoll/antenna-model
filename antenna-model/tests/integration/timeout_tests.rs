//! Request Timeout Tests (roadmap S2)
//!
//! Verifies that `server.request_timeout_secs` is actually enforced: a request
//! whose processing exceeds the configured timeout returns `504 Gateway Timeout`
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

/// Build a ServiceConfig pointed at the integration test fixtures. The request
/// timeout is supplied separately via `start_with_config_and_timeout`, so it is
/// not set here.
fn fixture_config() -> ServiceConfig {
    let mut cfg = ServiceConfig::with_defaults();
    cfg.server.host = "127.0.0.1".to_string();
    cfg.server.port = 0;
    cfg.server.max_body_size_bytes = 10_485_760;

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

/// A heatmap heavy enough that its compute dwarfs the sub-second test deadline
/// by a wide margin. The large (13 m) Ka-band offset-feed antenna is the most
/// expensive per-point integration in the fixtures (high D/λ, wide-angle coma);
/// a 12x12 grid over the full 0-45 deg quadrant costs hundreds of ms — far above
/// the 50 ms deadline the test sets, yet bounded so the un-cancellable
/// background rayon (the S2/S3 limitation) finishes in well under a second.
fn heavy_heatmap_request() -> HeatmapRequest {
    let mut req = builders::simple_heatmap_request();
    req.antenna_id = "test_large".to_string();
    req.feed_id = "ka_band".to_string();
    req.frequency_mhz = 26_000.0;
    req.grid_config = GridConfig::Rectangular {
        azimuth_range_deg: RangeConfig {
            min: 0.0,
            max: 45.0,
            step: 4.0,
        },
        elevation_range_deg: RangeConfig {
            min: 0.0,
            max: 45.0,
            step: 4.0,
        },
    };
    req
}

/// The compute-heavy heatmap endpoint must honor the request timeout: when
/// compute exceeds the deadline the client gets 504 Gateway Timeout with the
/// standard JSON body, and the 504 is correlatable (carries `x-request-id`,
/// echoing a client-supplied id).
///
/// The deadline is set to 50 ms via the `Duration` seam so the assertion rests
/// on a large margin (hundreds of ms of compute vs 50 ms), not on exact
/// wall-clock timing — robust across hardware and future integrator speedups.
/// The *deterministic* firing of the timeout mechanism itself is pinned
/// separately by the sleep-based middleware unit tests in `api::middleware`.
#[tokio::test]
async fn test_heavy_heatmap_times_out_with_504() {
    let timeout = std::time::Duration::from_millis(50);
    let server = TestServer::start_with_config_and_timeout(fixture_config(), timeout)
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let request = heavy_heatmap_request();
    let custom_id = "timeout-correlation-test-id";

    let start = std::time::Instant::now();
    let response = client
        .post(format!("{}/api/v1/heatmap", server.base_url))
        .header("Content-Type", "application/json")
        .header("x-request-id", custom_id)
        .json(&request)
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    let status = response.status();
    let echoed_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    assert_eq!(
        status, 504,
        "a heatmap exceeding the request timeout must return 504 Gateway Timeout (elapsed {elapsed:?})"
    );

    // The 504 must be correlatable: RequestId (outermost) attaches the id even on
    // the timeout error path, echoing the client-supplied value.
    assert_eq!(
        echoed_id.as_deref(),
        Some(custom_id),
        "the 504 response must carry the x-request-id correlation header"
    );

    let err: ErrorResponse = response.json().await.unwrap();
    assert_eq!(
        err.error, "request_timeout",
        "timeout body must be the standard ErrorResponse with code request_timeout"
    );

    server.shutdown().await;
}
