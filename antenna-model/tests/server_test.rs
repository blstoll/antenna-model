//! Integration tests for server startup and status endpoint
//!
//! These tests verify that the server can start, respond to requests,
//! and shut down gracefully.
//!
//! # Isolation (roadmap D28)
//!
//! This is the only test file in the workspace that binds a **real socket** — every
//! other HTTP test either drives poem in process (`build_in_process_app`) or goes
//! through `TestServer`, which has always bound port 0. Until 2026-08-15 these two
//! tests bound the literal ports 3001 and 3002, so two concurrent runs of the suite
//! collided, as did any developer whose machine already had those ports in use.
//!
//! What made that worth fixing was not the collision but the **diagnostic**. The bind
//! used to happen inside a `tokio::spawn`ed `start_server_with_config`, so
//! `.expect("Failed to start server")` panicked on a background task that nothing
//! joined; the test body carried on, slept a fixed 500 ms, and requested a port
//! nothing was listening on. The reported failure was therefore
//! `ConnectionRefused` on `/status` — a service-layer conclusion — while the actual
//! `AddrInUse` appeared only as a second, unattributed panic line. Measured during
//! D27 verification: the first reading of that output was "the server is failing to
//! serve `/status`", which sends the reader into the wrong code entirely.
//!
//! Three things keep it fixed, and all three are load-bearing:
//!
//! 1. **Port 0.** The OS assigns a free port and [`api::bind_server_with_config`]
//!    reports it back, so there is no literal to collide on and no coordination
//!    between tests.
//! 2. **Bind before spawn.** The socket is bound in the test's own body, so a bind
//!    failure is *this* test's `Err`, naming `AddrInUse`. Pinned by
//!    [`bind_failure_is_reported_as_a_bind_failure`].
//! 3. **No fixed startup sleep.** Readiness is polled. A fixed sleep produces the
//!    *identical* `ConnectionRefused` signature on a loaded machine, so leaving it in
//!    place would have left a flake indistinguishable from the bug just fixed — and
//!    D18 task 4 measured a 2.7x wall-clock spread on this suite on an idle machine,
//!    so that is not hypothetical.

use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// Build a config pointing at the checked-in test fixtures, on an OS-assigned port.
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
/// production startup path (`bind_server_with_config` + `BoundServer::serve`, which is
/// exactly what `start_server_with_config` is), so it doubles as a regression guard for the
/// exact knob this roadmap unit exists to honor.
///
/// Port `0` asks the OS for a free port; the actual one is read back from
/// [`antenna_model::api::BoundServer::local_addr`] (roadmap D28).
fn test_config() -> antenna_model::config::ServiceConfig {
    let mut config = antenna_model::config::ServiceConfig::with_defaults();
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 0;

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    config.calibration.data_directory = PathBuf::from(&manifest_dir);
    config.calibration.antenna_config_file =
        PathBuf::from(&manifest_dir).join("tests/fixtures/test_antennas.yaml");
    // fail_fast left at its shipped default (true) — see doc comment above.

    config
}

/// Per-request ceiling for every HTTP call in this file.
///
/// No single request here does real work — `/ready` and `/status` are synchronous
/// handlers — so anything approaching this is a wedged server, and saying so beats
/// hanging until the harness's own timeout kills the run with no attribution.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// A running test server: its base URL and the task serving it.
struct RunningServer {
    base_url: String,
    handle: JoinHandle<Result<(), std::io::Error>>,
}

impl RunningServer {
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn abort(self) {
        self.handle.abort();
    }
}

/// Bind the production startup path on an OS-assigned port, serve it on a background
/// task, and return once the service reports ready.
///
/// The bind happens **here**, in the caller's task, so a startup failure (`AddrInUse`,
/// a `fail_fast` calibration failure) fails the calling test with that error rather
/// than panicking on an orphaned background task — roadmap D28.
async fn start_test_server(client: &reqwest::Client) -> RunningServer {
    let server = antenna_model::api::bind_server_with_config(test_config())
        .await
        .expect("server must bind and load calibration");

    let base_url = format!("http://{}", server.local_addr());
    let handle = tokio::spawn(server.serve());

    let running = RunningServer { base_url, handle };
    wait_until_ready(client, &running).await;
    running
}

/// Poll `/ready` until the service answers 200, or fail with a diagnosis.
///
/// Replaces the fixed `sleep(500ms)` this file used to open with. Beyond being faster
/// in the common case (the socket is already bound and listening before `serve` is even
/// spawned, so the first poll normally succeeds), it removes the second producer of the
/// `ConnectionRefused` signature D28 is about: under load, a fixed wait that expires
/// early is indistinguishable in the output from a port collision.
///
/// The server task is checked for early exit on every iteration, so a failure inside
/// `serve` is reported as a server failure instead of timing out as a client one.
///
/// Each attempt is itself bounded by [`ATTEMPT_TIMEOUT`], and that bound is
/// load-bearing rather than belt-and-braces. The deadline below is only consulted
/// *between* attempts, so a request that never returns would never reach it — and
/// binding before spawning `serve` made exactly that reachable: an alive-but-wedged
/// server completes the TCP connect (the socket is listening from the moment it is
/// bound) and then never answers, where previously the connect itself would have been
/// refused. A hang with no diagnostic is the same failure class D28 exists to remove,
/// so the per-attempt timeout keeps the 30 s deadline the thing that actually fires.
async fn wait_until_ready(client: &reqwest::Client, server: &RunningServer) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    loop {
        assert!(
            !server.handle.is_finished(),
            "the server task exited before becoming ready (see the task's own error above)"
        );

        let attempt = timeout(ATTEMPT_TIMEOUT, client.get(server.url("/ready")).send()).await;
        if let Ok(Ok(response)) = attempt {
            if response.status() == 200 {
                return;
            }
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "server did not report ready at {} within 30s",
            server.base_url
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Test that the server can start and is accessible
///
/// This test verifies:
/// - Server binds to a port and reports which one
/// - Status endpoint returns 200 OK
/// - Response contains expected JSON fields
#[tokio::test]
async fn test_server_startup_and_status() {
    let client = reqwest::Client::new();
    let server = start_test_server(&client).await;

    // Make a request to the status endpoint
    let response = timeout(ATTEMPT_TIMEOUT, client.get(server.url("/status")).send())
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

    // Proves `set_antenna_ids()` ran in `bind_server_with_config` on the Healthy path
    // (roadmap S5): before that wiring, /status never reported a populated antenna_count.
    assert!(
        json["antenna_count"].as_u64().unwrap_or(0) > 0,
        "expected /status to report loaded antennas, got: {json}"
    );

    // `start_test_server` already required a 200 from /ready, which proves
    // `state.mark_ready()` ran on the Healthy path (roadmap S5): readiness starts false
    // and is earned only by a completed healthy load. Asserted again here against the
    // *same* server so the readiness claim is not merely a startup precondition — a
    // regression that marked ready and then cleared it would fail here.
    let ready_response = timeout(ATTEMPT_TIMEOUT, client.get(server.url("/ready")).send())
        .await
        .expect("Ready request timed out")
        .expect("Ready request failed");
    assert_eq!(
        ready_response.status(),
        200,
        "expected /ready to be 200 after a healthy calibration load"
    );

    server.abort();
}

/// Test that uptime increases over time
#[tokio::test]
async fn test_status_uptime_increases() {
    let client = reqwest::Client::new();
    let server = start_test_server(&client).await;

    let read_uptime = |client: reqwest::Client, url: String| async move {
        let response = timeout(ATTEMPT_TIMEOUT, client.get(url).send())
            .await
            .expect("status request timed out")
            .expect("status request failed");
        let json: serde_json::Value = response.json().await.expect("Failed to parse JSON");
        json["uptime_seconds"].as_u64().expect("Invalid uptime")
    };

    let uptime1 = read_uptime(client.clone(), server.url("/status")).await;

    // `uptime_seconds` has one-second granularity, so this necessarily waits for a tick.
    // Poll for it rather than sleeping a fixed 1100 ms: the tick can land at any point
    // in the second following startup, so a fixed wait is both slower than it needs to
    // be in the common case and — on a loaded machine — no more certain.
    //
    // The loop breaks on the deadline as well as on success, so the assertion below is
    // the one that decides the test rather than restating a condition the loop already
    // guaranteed (P13: a guard nothing can falsify is not a guard).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let uptime2 = loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let uptime = read_uptime(client.clone(), server.url("/status")).await;
        if uptime > uptime1 || tokio::time::Instant::now() >= deadline {
            break uptime;
        }
    };

    assert!(
        uptime2 > uptime1,
        "uptime did not advance past {uptime1}s within 10s (last read: {uptime2}s)"
    );

    server.abort();
}

/// Roadmap D28: a bind failure must fail the test that caused it, naming `AddrInUse`.
///
/// This is the guard on the diagnostic, not on the ports. The old arrangement could not
/// satisfy it at all: the bind lived inside a spawned task, so this assertion had no
/// value to inspect — the caller saw a later `ConnectionRefused` from a request instead.
///
/// The squatter socket is bound on an OS-assigned port and *held for the duration*, so
/// the collision is manufactured rather than waited for. `SO_REUSEADDR` (which tokio sets)
/// does not permit a second bind to an actively listening address, which is what makes
/// this deterministic.
#[tokio::test]
async fn bind_failure_is_reported_as_a_bind_failure() {
    let squatter = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test must be able to bind its own socket");
    let occupied_port = squatter
        .local_addr()
        .expect("bound socket must have an address")
        .port();

    let mut config = test_config();
    config.server.port = occupied_port;

    let error = antenna_model::api::bind_server_with_config(config)
        .await
        .err()
        .expect("binding an occupied port must fail");

    assert_eq!(
        error.kind(),
        std::io::ErrorKind::AddrInUse,
        "a port collision must surface as AddrInUse at the caller, not as a later \
         ConnectionRefused from a request to a server that never started: {error}"
    );

    drop(squatter);
}
