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
//! **The two headline timeout cases here are pinned on a paused clock, in process,
//! and assert no wall-clock threshold at all.** The single-gain case (roadmap S2b)
//! was written that way; the heatmap case was converted to it 2026-08-15 under D18,
//! having cost 131 s as a socket test that won a race against a *real* 50 ms
//! deadline by being expensive. See `test_heavy_single_gain_times_out_with_504`
//! for why the mocked clock works here and why it cannot be used over a socket.
//!
//! If you add another timeout case, start from that pattern rather than from a
//! real deadline plus a heavy request: sizing a request to outlast a real
//! deadline couples the test's cost to the physics' cost, and the physics here is
//! ~19x dearer in debug — which is exactly how the heatmap case reached 131 s.
//!
//! The one deliberate exception is
//! [`test_request_timeout_504_over_a_real_socket`] (roadmap D29 item 1), which
//! keeps a real deadline precisely because hyper's serialization of *this* 504 is
//! what it exists to cover. It inverts the sizing rather than repeating it: the
//! deadline shrinks to 1 ms instead of the request growing, so the margin is won
//! by making the deadline small rather than the compute large. Read its docs
//! before adding a second real-deadline test.

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

/// A heatmap whose compute is still running when the paused-clock test advances
/// past the deadline. The large (13 m) Ka-band offset-feed antenna is the most
/// expensive per-point integration in the fixtures (high D/λ, wide-angle coma).
///
/// Like [`heavy_gain_request`], the cost here is a **race margin, not a
/// threshold** — nothing asserts on it — so the grid is deliberately the smallest
/// one that clears the margin rather than the largest one that fits a wall-clock
/// budget. A 2x2 grid is already ~4 Ka-band integrations, against the microseconds
/// the test needs them to outlast.
///
/// It was a 12x12 grid (0-45 deg at 4 deg steps) until 2026-08-15, when the test
/// moved to the paused clock. That grid was sized by a comment claiming "hundreds
/// of ms" of compute, a *release*-profile figure; tests run in debug, where the
/// same geometry is ~19x dearer (~2.7 s per point), so 144 points cost **131 s**
/// and made this the single most expensive test in the suite. Do not re-size this
/// grid against a release measurement.
fn heavy_heatmap_request() -> HeatmapRequest {
    let mut req = builders::simple_heatmap_request();
    req.antenna_id = "test_large".to_string();
    req.feed_id = "ka_band".to_string();
    req.frequency_mhz = 26_000.0;
    req.grid_config = GridConfig::Rectangular {
        azimuth_range_deg: RangeConfig {
            min: 0.0,
            max: 45.0,
            step: 45.0,
        },
        elevation_range_deg: RangeConfig {
            min: 0.0,
            max: 45.0,
            step: 45.0,
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
            coordinate_system: CoordinateSystem::ECEF,
        },
        reflector_boresight: Position3D {
            x: bore_x,
            y: bore_y,
            z: bore_z,
            coordinate_system: CoordinateSystem::ECEF,
        },
        feed_pointing_location: Position3D {
            x: bore_x,
            y: bore_y,
            z: bore_z,
            coordinate_system: CoordinateSystem::ECEF,
        },
        emitter_position: Position3D {
            x: emit_x,
            y: emit_y,
            z: emit_z,
            coordinate_system: CoordinateSystem::ECEF,
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

/// Roadmap D29 item 1: an S2 `request_timeout` 504 must survive **real TCP and real
/// hyper**, carrying the standard JSON `ErrorResponse` and the `x-request-id` echo.
///
/// # Why this exists as a socket test when the other two do not
///
/// Both paused-clock tests above run in process, through `Endpoint::call`. That
/// covers the middleware stack faithfully but stops short of the wire: a regression
/// in how *this particular* 504 response is serialized by hyper would pass there and
/// ship. The two partial substitutes each cover half of the combination and neither
/// covers it whole — `budget_tests::test_over_budget_single_gain_returns_504` is a
/// socket-level 504 with the standard body, but from S3's per-integration budget
/// (`computation_budget_exceeded`, a different middleware); `error_tests` asserts the
/// `x-request-id` echo over a socket, but on a 413.
///
/// # Why this is not the 131 s test coming back
///
/// The retired socket heatmap test won its race by making the *request* expensive
/// enough to outlast a real 50 ms deadline — which coupled its cost to the physics'
/// cost, in debug, where the physics is ~19x dearer. This inverts that: the request
/// is the ordinary single-gain one the other socket tests use, and the **deadline**
/// is 1 ms, tokio's timer granularity. A request only has to be slower than the
/// deadline, not dramatically slower, once nothing is being asserted about the margin.
///
/// # The flake direction is one-sided, which is the point
///
/// A real deadline can only flake by the deadline *not* being crossed — i.e. by the
/// gain compute finishing inside 1 ms. Measured on this harness (the control test's
/// own `computation_time_ms`, same request, debug profile): **91.8 ms**, a ~92x
/// margin. Every force that could disturb the timing (machine load, a contended
/// nextest run, a slower CPU) pushes the compute *up*, further past the deadline.
/// There is no mechanism by which load makes this test pass a request it should have
/// timed out. That asymmetry is why a real deadline is acceptable here and was not
/// acceptable for the heatmap case, whose margin ran the other way. The margin is not
/// asserted on and must not become a threshold — if the physics ever gets 90x
/// cheaper, shrink the deadline, do not grow the request.
///
/// Note what the 1 ms deadline does *not* break: `/health`, which `TestServer` polls
/// during startup, is a synchronous handler. `RequestTimeout` is a
/// `tokio::time::timeout` around the endpoint future, and a future that is `Ready` on
/// its first poll is never preempted whatever the deadline — the same property, read
/// in the other direction, that S2b exists to guarantee for `/gain`.
///
/// **Control:** [`test_single_gain_under_timeout_still_succeeds`] issues the *same*
/// request over the *same* socket harness under a generous deadline and asserts 200,
/// so the 504 below is attributable to the deadline rather than to anything about the
/// request or the transport.
#[tokio::test]
async fn test_request_timeout_504_over_a_real_socket() {
    // 1 ms: tokio's timer granularity, ~92x below the request's measured debug-profile
    // compute. Not a threshold — nothing asserts on either figure.
    let timeout = Duration::from_millis(1);
    let server = TestServer::start_with_config_and_timeout(fixture_config(), timeout)
        .await
        .expect("Failed to start test server");

    let custom_id = "socket-timeout-correlation-id";
    let response = server
        .client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("content-type", "application/json")
        .header("x-request-id", custom_id)
        .json(&builders::simple_gain_request_ecef())
        .send()
        .await
        .expect("the 504 must arrive as an HTTP response, not a transport error");

    let status = response.status();
    let echoed_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = response.bytes().await.expect("readable response body");

    assert_eq!(
        status,
        504,
        "a request exceeding the deadline must return 504 over a socket — body: {}",
        String::from_utf8_lossy(&body)
    );

    let err: ErrorResponse = serde_json::from_slice(&body)
        .expect("the 504 must serialize as the standard JSON ErrorResponse over the wire");
    assert_eq!(
        err.error, "request_timeout",
        "the 504 must come from the request-timeout middleware, not S3's per-integration budget"
    );

    assert_eq!(
        echoed_id.as_deref(),
        Some(custom_id),
        "the 504 response must carry the x-request-id correlation header over the wire"
    );

    server.shutdown().await;
}

/// The compute-heavy heatmap endpoint must honor the request timeout: when
/// compute exceeds the deadline the client gets 504 Gateway Timeout with the
/// standard JSON body, and the 504 is correlatable (carries `x-request-id`,
/// echoing a client-supplied id).
///
/// Runs on the **paused clock**, in process, for the same reasons as
/// `test_heavy_single_gain_times_out_with_504` above — read that test's docs for
/// why the mocked clock cannot be used over a socket. Nothing here asserts a
/// wall-clock threshold.
///
/// # Why this is not a socket test any more (2026-08-15)
///
/// It used to bind a real server, set a 50 ms deadline, and win the race by
/// making the request expensive enough to outlast it in *real* time. That cost
/// **131 s** — the most expensive test in the suite — because the grid had been
/// sized against release-profile figures while tests run in debug (see
/// [`heavy_heatmap_request`]). Under a mocked clock the deadline is crossed by
/// `advance`, so the race no longer has to be won with compute, and the grid
/// shrinks to a 2x2.
///
/// **Negative control run at conversion time** (per P13 — a guard nothing has
/// falsified is not known to have power): deleting the `advance` call and leaving
/// everything else identical makes this test **fail with 200**, on a response body
/// whose metadata reports `computation_time_ms: 1172`. So the 504 is genuinely
/// produced by crossing the deadline, not by anything incidental, and the race
/// margin is real — the clock is advanced microseconds in, against ~1.2 s of
/// compute, ~5 orders of magnitude, the same margin the S2b test relies on.
///
/// All three assertions survive the move: `build_in_process_app` calls
/// `create_routes_with_timeout`, and that and the production `create_routes`
/// delegate to the **same `build_app`** — so the middleware stack under test,
/// `RequestId` included, is the one the server binds to a port.
///
/// What is *not* covered here is real TCP and hyper's serialization of a 504.
/// When this test moved in process that combination — a `request_timeout` 504,
/// standard JSON body, `x-request-id` echo, over a socket — was left uncovered and
/// filed as roadmap **D29** item 1, since the two socket-level substitutes each
/// hold only half of it (`budget_tests::test_over_budget_single_gain_returns_504`
/// is a socket 504 with the standard body but on S3's
/// `computation_budget_exceeded`; `error_tests` asserts the header echo over a
/// socket but on a 413). **Closed 2026-08-15 by
/// [`test_request_timeout_504_over_a_real_socket`]**, which covers it without
/// restoring a heavy request: it shrinks the *deadline* to 1 ms rather than growing
/// the compute, so the gap is closed at ~0.2 s instead of the 131 s this test cost.
#[tokio::test(start_paused = true)]
async fn test_heavy_heatmap_times_out_with_504() {
    let timeout = Duration::from_secs(30);

    // Same reasoning as the single-gain test above: the 504 is produced in mocked
    // time, but the offloaded rayon work is not cancelled by it and the runtime
    // waits for the blocking task at drop, so the *test* would otherwise pay the
    // full grid compute. Bound it with S3's real per-integration budget. This
    // cannot change the assertion — the request timeout fires in mocked time long
    // before 250 ms of real time elapse, so the response is always
    // `request_timeout`, never `computation_budget_exceeded`, and the assertion on
    // the specific code below is what proves it.
    let mut config = fixture_config();
    config.performance.integration_budget_ms = 250;

    let app = build_in_process_app(config, timeout).expect("in-process app must build");

    let request = heavy_heatmap_request();
    let custom_id = "timeout-correlation-test-id";

    let handle = tokio::spawn(async move {
        let app = app;
        call_json_with_headers(
            &app,
            "/api/v1/heatmap",
            &request,
            &[("x-request-id", custom_id)],
        )
        .await
    });

    // Let the request reach the handler and register the timeout, then jump past
    // the deadline and let the 504 propagate.
    tokio::task::yield_now().await;
    tokio::time::advance(timeout + Duration::from_secs(1)).await;
    tokio::task::yield_now().await;

    let (status, headers, body) = handle.await.expect("request task must not panic");

    assert_eq!(
        status,
        504,
        "a heatmap exceeding the request timeout must return 504 Gateway Timeout — body: {}",
        String::from_utf8_lossy(&body)
    );

    // The 504 must be correlatable: RequestId (outermost) attaches the id even on
    // the timeout error path, echoing the client-supplied value.
    let echoed_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    assert_eq!(
        echoed_id.as_deref(),
        Some(custom_id),
        "the 504 response must carry the x-request-id correlation header"
    );

    let err: ErrorResponse = serde_json::from_slice(&body).expect("standard JSON error body");
    assert_eq!(
        err.error, "request_timeout",
        "timeout body must be the standard ErrorResponse with code request_timeout"
    );
}
