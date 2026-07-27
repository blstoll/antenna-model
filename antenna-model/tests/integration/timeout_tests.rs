//! Request Timeout Tests (roadmap S2)
//!
//! Verifies that `server.request_timeout_secs` is actually enforced: a request
//! whose processing exceeds the configured timeout returns `504 Gateway Timeout`
//! with the project's standard JSON `ErrorResponse` body.
//!
//! The compute paths (gain/heatmap/batch/h3) run rayon synchronously; the
//! handlers offload it to `tokio::task::spawn_blocking` so the async task yields
//! at a real `.await`, letting the timeout middleware fire. Note (honest
//! limitation): the timeout bounds the *response*, not the background compute —
//! the rayon work is not cancelled and runs to completion (see S3).
//!
//! The single-gain case (roadmap S2b) is pinned on a **paused clock** and asserts
//! no wall-clock threshold at all; see `test_heavy_single_gain_times_out_with_504`
//! for why that works here and why it has to run in process rather than over a
//! socket.

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use antenna_model::config::ServiceConfig;
use std::path::PathBuf;
use std::time::Duration;

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

/// A *single* gain evaluation expensive enough to still be running when the
/// paused-clock test advances the deadline: the 13 m Ka-band offset-feed antenna
/// (highest D/lambda in the fixtures, and the lateral feed offset forces the
/// expensive asymmetric azimuthal-mode path rather than the cheap symmetric J0
/// one), with the emitter off the boresight target.
///
/// Its cost (~2.7 s in debug, ~140 ms in release) is a **race margin, not a
/// threshold**: the test advances the clock microseconds after the handler
/// offloads, so any compute above that is sufficient and the margin is ~5 orders
/// of magnitude. Nothing asserts on this duration.
fn heavy_gain_request() -> GainRequest {
    use antenna_model::model::coordinates_3d::geodetic_to_ecef;

    let (veh_x, veh_y, veh_z) = geodetic_to_ecef(-118.1234, 34.5678, 100.0).unwrap();
    // Boresight aimed at one satellite...
    let (bore_x, bore_y, bore_z) = geodetic_to_ecef(-117.0, 35.0, 400_000.0).unwrap();
    // ...while the emitter sits at another, tens of degrees away (same construction
    // as the off-axis warning tests).
    let (emit_x, emit_y, emit_z) = geodetic_to_ecef(-125.0, 28.0, 400_000.0).unwrap();

    GainRequest {
        antenna_id: "test_large".to_string(),
        feed_id: "ka_band".to_string(),
        vehicle_position: Position3D {
            x: veh_x,
            y: veh_y,
            z: veh_z,
            coordinate_system: Some(CoordinateSystem::ECEF),
        },
        reflector_boresight: Position3D {
            x: bore_x,
            y: bore_y,
            z: bore_z,
            coordinate_system: Some(CoordinateSystem::ECEF),
        },
        feed_pointing_location: Position3D {
            x: bore_x,
            y: bore_y,
            z: bore_z,
            coordinate_system: Some(CoordinateSystem::ECEF),
        },
        emitter_position: Position3D {
            x: emit_x,
            y: emit_y,
            z: emit_z,
            coordinate_system: Some(CoordinateSystem::ECEF),
        },
        frequency_mhz: 26_000.0,
        pointing_frequency_mhz: None,
        include_reference: false,
        vehicle_attitude: None,
    }
}

/// Roadmap S2b: `POST /api/v1/gain` must be *preemptable* by the request
/// timeout. It used to run its physics inline on the async task; `RequestTimeout`
/// is a `tokio::time::timeout` around the endpoint future, and a future that
/// never yields is never preempted — so a slow single gain returned a late 200
/// instead of a 504, and `server.request_timeout_secs` was unenforceable on the
/// service's primary endpoint. The handler now offloads to `spawn_blocking` like
/// batch/heatmap/h3.
///
/// **No wall-clock threshold.** The assertion does not depend on how long the
/// physics takes, only on *whether the handler releases the executor at all*. It
/// runs on a **paused clock**, where mocked time advances only while the runtime
/// is idle — and "the executor is not idle" is precisely the bug:
///
/// - **Fixed:** the handler offloads and parks at the `spawn_blocking` join. The
///   runtime goes idle, the `advance` past the deadline takes effect, the timer
///   fires → **504 `request_timeout`**.
/// - **Broken (inline):** the task computes without ever yielding, so the runtime
///   never goes idle, mocked time cannot move, and `tokio::time::timeout` polls an
///   already-ready inner future. The request timeout can never fire, whatever the
///   deadline and however long the compute runs. Verified against the pre-S2b
///   handler: 2.68 s of real compute, mocked time advanced 31 s, still no
///   `request_timeout`.
///
/// So the 504 must carry `request_timeout` **specifically**, and that is the load-
/// bearing assertion — a status-only check would be satisfied by S3's
/// `computation_budget_exceeded`, which is a different mechanism. (With the
/// deliberately small `integration_budget_ms` below, the pre-S2b handler fails this
/// test on exactly that code rather than on the status; in production, with the
/// 30 s default budget, its symptom was the late 200 described above.)
///
/// # Why in-process, not over a socket
///
/// This drives the app through `Endpoint::call` rather than `TestServer` + a real
/// TCP client. Over a socket the request reaching the handler depends on real
/// loopback I/O, which the mocked clock cannot order: measured on this harness, a
/// 35 s mocked sleep completed in **320 µs of real time**, i.e. the deadline
/// elapsed before the request arrived, the timer registered *after* the jump, and
/// the request then returned a late 200. In process, the request reaches the
/// handler within the first poll of the spawned task, so `yield_now` is a
/// sufficient and deterministic ordering barrier. Nothing about the middleware
/// stack is bypassed — `create_routes_with_timeout` builds the same one the server
/// binds to a port.
#[tokio::test(start_paused = true)]
async fn test_heavy_single_gain_times_out_with_504() {
    let timeout = Duration::from_secs(30);

    // The 504 is produced in mocked time, microseconds in, but the offloaded rayon
    // work is NOT cancelled by it (S2's standing limitation) and the runtime waits
    // for the blocking task at drop — so the *test* would otherwise pay the full
    // ~2.7 s compute. Bound it with S3's real per-integration budget, which is what
    // that budget is for. This cannot change the assertion: the request timeout
    // fires in mocked time long before 250 ms of real time elapse, so the response
    // is always `request_timeout`, never `computation_budget_exceeded`.
    let mut config = fixture_config();
    config.performance.integration_budget_ms = 250;

    let app = build_in_process_app(config, timeout).expect("in-process app must build");

    let request = heavy_gain_request();
    let handle = tokio::spawn(async move {
        let app = app;
        call_json(&app, "/api/v1/gain", &request).await
    });

    // Let the request reach the handler and register the timeout, then jump past
    // the deadline and let the 504 propagate.
    tokio::task::yield_now().await;
    tokio::time::advance(timeout + Duration::from_secs(1)).await;
    tokio::task::yield_now().await;

    let (status, body) = handle.await.expect("request task must not panic");

    assert_eq!(
        status,
        504,
        "a single gain that occupies the executor must return 504, not a late 200 — body: {}",
        String::from_utf8_lossy(&body)
    );

    let err: ErrorResponse = serde_json::from_slice(&body).expect("standard JSON error body");
    assert_eq!(
        err.error, "request_timeout",
        "the 504 must come from the request-timeout middleware, not S3's per-integration budget"
    );
}

/// Control for S2b: moving the compute to the blocking pool must not change the
/// served answer. A normal single-gain request under a generous deadline still
/// returns 200 with a real gain value.
#[tokio::test]
async fn test_single_gain_under_timeout_still_succeeds() {
    let timeout = Duration::from_secs(30);
    let server = TestServer::start_with_config_and_timeout(fixture_config(), timeout)
        .await
        .unwrap();

    let response: GainResponse = server
        .post("/api/v1/gain", &builders::simple_gain_request_ecef())
        .await
        .expect("a normal single-gain request must still succeed");

    assert!(
        response.gain_db.is_finite(),
        "gain must be a real value, got {}",
        response.gain_db
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
