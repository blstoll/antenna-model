//! Request Timeout Tests (roadmap S2)
//!
//! Verifies that `server.request_timeout_secs` is actually enforced: a request
//! whose processing exceeds the configured timeout returns `504 Gateway Timeout`
//! with the project's standard JSON `ErrorResponse` body.
//!
//! The compute path (single gain/heatmap/batch/h3) runs rayon synchronously; all
//! four handlers offload it to `tokio::task::spawn_blocking` so the async task
//! yields at a real `.await`, letting the timeout middleware fire. Single gain
//! was the exception until roadmap S2b — it computed inline, so its future never
//! yielded and the timeout could not preempt it. Note (honest limitation): the
//! timeout bounds the *response*, not the background compute — the rayon work is
//! not cancelled and runs to completion (see S3).

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

/// A single-gain request expensive enough that its compute dwarfs a sub-second deadline
/// by a wide margin: wide-angle Ka on the 13 m offset-feed antenna, the most expensive
/// per-point integration in the fixtures (high D/λ plus coma from the laterally-offset
/// feed — the hot case roadmap P10-perf tracks). Measured at ~0.9 s on the reference
/// machine, against the 50 ms deadline the test sets.
///
/// Two geometry details are load-bearing, and getting either wrong makes the request
/// *cheap* while still returning 200 — a silently useless test:
///
/// - `feed_position` is a **pointing target**, not a physical offset (see
///   `docs/domain-contract.md`). The shared builder aims it at a nearby ground point,
///   which derives a >0.5f feed offset and routes to the cheap ray-tracing stub instead
///   of the integrator. Pointing it at the boresight target keeps the offset at the
///   design 0.08 m and stays on `StandardPhysicalOptics`.
/// - The emitter must be off-axis but **in front of** the dish. Push it past 90° of
///   elevation and the evaluation lands on the rear sidelobe floor, which returns without
///   integrating at all (~3 ms).
fn heavy_gain_request() -> GainRequest {
    use antenna_model::model::coordinates_3d::geodetic_to_ecef;

    let mut req = builders::simple_gain_request_ecef();
    req.antenna_id = "test_large".to_string();
    req.feed_id = "ka_band".to_string();
    req.frequency_mhz = 26_000.0;

    let ecef = |lon, lat, alt| {
        let (x, y, z) = geodetic_to_ecef(lon, lat, alt).unwrap();
        Position3D {
            x,
            y,
            z,
            coordinate_system: Some(CoordinateSystem::ECEF),
        }
    };

    // Boresight (and the feed's pointing target) straight up from the vehicle; emitter
    // ~25 deg azimuth / 17 deg elevation away from it.
    let zenith = ecef(-118.1234, 34.5678, 400_000.0);
    req.reflector_boresight = zenith.clone();
    req.feed_position = zenith;
    req.emitter_position = ecef(-117.0, 35.0, 400_000.0);
    req
}

/// Roadmap S2b: `POST /api/v1/gain` must honor the request timeout.
///
/// It was the only heavy-compute handler that ran its physics inline on the async task
/// instead of `spawn_blocking`, so its future never yielded and `RequestTimeout`'s
/// `tokio::time::timeout` could never preempt it — the configured
/// `server.request_timeout_secs` was decorative on this route, leaving S3's
/// `performance.integration_budget_ms` as the only live bound.
///
/// This test discriminates the two implementations directly rather than by wall-clock:
/// `tokio::time::timeout` polls its inner future first, so an inline compute that runs to
/// completion in one poll returns **200 regardless of how long it took**. Only a handler
/// that actually yields can produce the 504. The deadline is set well below the compute
/// cost via the `Duration` seam, so the assertion rests on a large margin, not on exact
/// timing.
#[tokio::test]
async fn test_heavy_single_gain_times_out_with_504() {
    let timeout = std::time::Duration::from_millis(50);
    let server = TestServer::start_with_config_and_timeout(fixture_config(), timeout)
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "application/json")
        .json(&heavy_gain_request())
        .send()
        .await
        .unwrap();

    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(
        status, 504,
        "a single-gain request exceeding the request timeout must return 504 — if this is \
         200, the handler is computing inline and the timeout cannot preempt it (S2b)"
    );

    let err: ErrorResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(
        err.error, "request_timeout",
        "timeout body must be the standard ErrorResponse with code request_timeout"
    );

    server.shutdown().await;
}

/// Roadmap S2b control: a *cheap* single gain still completes normally under a generous
/// deadline. Moving the compute to `spawn_blocking` must not change the success path
/// (status, body, or the `JoinError` handling that now sits between handler and client).
#[tokio::test]
async fn test_cheap_single_gain_completes_under_timeout() {
    let timeout = std::time::Duration::from_secs(30);
    let server = TestServer::start_with_config_and_timeout(fixture_config(), timeout)
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("Content-Type", "application/json")
        .json(&builders::simple_gain_request_ecef())
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let gain: GainResponse = response.json().await.unwrap();
    assert!(
        gain.gain_db.is_finite(),
        "the spawn_blocking success path must return a real gain, got {}",
        gain.gain_db
    );

    server.shutdown().await;
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
