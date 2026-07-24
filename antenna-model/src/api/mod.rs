//! API module - REST API server and routing
//!
//! This module contains the web server setup, routing, handlers, and middleware
//! for the antenna model service REST API.
//!
//! # Production Features (Sprint 5)
//! - Configuration-driven server setup
//! - Production-grade middleware (request IDs, timing, error handling)
//! - Structured logging with request correlation
//! - Graceful shutdown with signal handling
//! - Connection management and resource cleanup

pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod schemas;

use crate::config::ServiceConfig;
use crate::data::repository::CalibrationRepository;
use crate::service::GainCache;
use poem::{listener::TcpListener, Server};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::signal;
use tracing::info;

/// Application state shared across handlers
///
/// This state is thread-safe (via Arc) and contains:
/// - Server metadata (version, start time)
/// - Configuration settings
/// - Readiness state for health checks
/// - Calibration repository (Task 5.4)
#[derive(Clone)]
pub struct AppState {
    /// Server start time for uptime calculation
    pub start_time: SystemTime,

    /// Application version from Cargo.toml
    pub version: &'static str,

    /// Service configuration
    pub config: Arc<ServiceConfig>,

    /// Readiness state — false until the calibration load completes, true while serving,
    /// false again once graceful shutdown begins (roadmap S5).
    pub ready: Arc<AtomicBool>,

    /// Loaded antenna IDs (will be populated by Task 5.4 - Calibration Repository)
    /// Using Arc to allow sharing without cloning Vec
    pub antenna_ids: Arc<parking_lot::RwLock<Vec<String>>>,

    /// Calibration data repository (Task 5.4)
    pub repository: CalibrationRepository,

    /// Gain cache for memoizing physics model results
    pub cache: Arc<GainCache>,
}

impl AppState {
    /// Create new application state with configuration and repository
    pub fn new(config: ServiceConfig, repository: CalibrationRepository) -> Self {
        let cache = Arc::new(GainCache::new(
            config.cache.enabled,
            config.cache.max_entries_per_feed,
        ));
        Self {
            start_time: SystemTime::now(),
            version: env!("CARGO_PKG_VERSION"),
            config: Arc::new(config),
            // Readiness starts FALSE (roadmap S5): constructing a state is no evidence that
            // the service can serve anything. The production path earns readiness only by
            // completing a healthy calibration load (`start_server_with_config`, via
            // `initialize_repository` + `LoadOutcome::Healthy`) and surrenders it again at
            // the top of graceful shutdown (`begin_shutdown`).
            ready: Arc::new(AtomicBool::new(false)),
            antenna_ids: Arc::new(parking_lot::RwLock::new(Vec::new())),
            repository,
            cache,
        }
    }

    /// Create application state with default configuration and empty repository
    pub fn with_defaults() -> Self {
        Self::new(ServiceConfig::with_defaults(), CalibrationRepository::new())
    }

    /// Get uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Get server bind address from configuration
    pub fn bind_address(&self) -> String {
        self.config.bind_address()
    }

    /// Check if service is ready to accept requests
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Mark service as ready
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Relaxed);
    }

    /// Mark service as not ready
    pub fn mark_not_ready(&self) {
        self.ready.store(false, Ordering::Relaxed);
    }

    /// Get loaded antenna IDs
    pub fn get_antenna_ids(&self) -> Vec<String> {
        self.antenna_ids.read().clone()
    }

    /// Set antenna IDs (called by calibration repository during initialization)
    pub fn set_antenna_ids(&self, ids: Vec<String>) {
        *self.antenna_ids.write() = ids;
    }

    /// Get memory usage in bytes (if available)
    ///
    /// Returns the current process memory usage. On some platforms this may not be available.
    pub fn get_memory_usage(&self) -> Option<u64> {
        // Try to get memory usage from /proc/self/statm on Linux
        #[cfg(target_os = "linux")]
        {
            use std::fs;
            if let Ok(contents) = fs::read_to_string("/proc/self/statm") {
                // statm format: size resident shared text lib data dt
                // We want RSS (resident set size) which is the second field
                // Each page is typically 4096 bytes
                if let Some(rss_pages) = contents.split_whitespace().nth(1) {
                    if let Ok(pages) = rss_pages.parse::<u64>() {
                        return Some(pages * 4096);
                    }
                }
            }
        }

        // On macOS, we could use task_info but that requires unsafe code
        // For now, return None on non-Linux platforms
        None
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Start the API server with configuration
///
/// This function initializes the web server with:
/// - Production-grade middleware stack
/// - Configuration-driven settings
/// - Graceful shutdown handling
/// - Structured logging
///
/// # Arguments
/// * `config` - Service configuration loaded from file or defaults
///
/// # Returns
/// * `Ok(())` - Server ran successfully and shut down gracefully
/// * `Err(std::io::Error)` - Failed to start or run the server
pub async fn start_server_with_config(config: ServiceConfig) -> Result<(), std::io::Error> {
    // Load calibration repository
    info!(
        calibration_dir = ?config.calibration.data_directory,
        antenna_config = ?config.calibration.antenna_config_file,
        fail_fast = config.calibration.fail_fast,
        "Loading calibration data"
    );

    let (repository, load_outcome) = initialize_repository(&config.calibration)?;

    // Apply the configured rayon worker-thread count ONCE at startup (roadmap S4).
    // `0` = auto-detect (leave rayon's global pool to size itself). A positive value
    // builds the global pool with that many threads. `build_global` can only succeed
    // once per process and errors if a pool already exists — in that case we log and
    // continue on the existing pool (never panic: repo rule).
    apply_worker_threads(config.performance.worker_threads);

    let state = Arc::new(AppState::new(config.clone(), repository));

    // Publish the loaded set so /status reports it. Before S5, production never called
    // set_antenna_ids, so antenna_count/antenna_ids were always omitted from /status.
    state.set_antenna_ids(state.repository.list_antennas());

    match load_outcome {
        LoadOutcome::Healthy => {
            state.mark_ready();
            info!(
                antenna_count = state.repository.antenna_count(),
                "Service marked READY"
            );
        }
        LoadOutcome::Degraded => {
            tracing::warn!(
                "Service starting DEGRADED: no calibration data loaded. Readiness stays \
                 false and /health reports \"degraded\"; gain requests will 404."
            );
        }
    }

    info!(
        version = state.version,
        bind_address = %state.bind_address(),
        max_body_size = state.config.server.max_body_size_bytes,
        request_timeout = state.config.server.request_timeout_secs,
        "Starting Antenna Model Service"
    );

    // Log configuration details
    info!(
        calibration_dir = ?state.config.calibration.data_directory,
        antenna_config = ?state.config.calibration.antenna_config_file,
        fail_fast = state.config.calibration.fail_fast,
        "Calibration configuration"
    );

    info!(
        worker_threads_configured = state.config.performance.worker_threads,
        worker_threads_effective = rayon::current_num_threads(),
        max_batch_size = state.config.performance.max_batch_size,
        parallel_processing = state.config.performance.enable_parallel_processing,
        max_concurrent_heavy_requests = state.config.performance.max_concurrent_heavy_requests,
        admission_retry_after_secs = state.config.performance.admission_retry_after_secs,
        "Performance configuration"
    );

    // Create routes with middleware
    let app = routes::create_routes(state.clone());

    let addr = state.bind_address();

    info!("Server ready to accept connections on {}", addr);

    // Graceful shutdown (roadmap S5): flip readiness false and pause so load balancers
    // stop sending new work, then drain in-flight requests under a bounded timeout, then
    // run cleanup. Before S5 this future only logged, the drain was unbounded (`None`),
    // and `shutdown_cleanup` had no caller at all.
    let readiness_delay = Duration::from_secs(state.config.server.shutdown_readiness_delay_secs);
    let drain_timeout = Duration::from_secs(state.config.server.shutdown_timeout_secs);
    let shutdown_state = state.clone();

    info!(
        readiness_delay_secs = readiness_delay.as_secs(),
        drain_timeout_secs = drain_timeout.as_secs(),
        "Graceful shutdown configured"
    );

    let result = Server::new(TcpListener::bind(&addr))
        .run_with_graceful_shutdown(
            app,
            async move {
                shutdown_signal().await;
                info!("Graceful shutdown initiated");
                begin_shutdown(&shutdown_state, readiness_delay).await;
            },
            Some(drain_timeout),
        )
        .await;

    // Runs on both the clean and the errored path — cleanup is exactly what must not be
    // skipped when the server came down badly.
    shutdown_cleanup(&state).await;

    result
}

/// Start the API server (legacy interface for backward compatibility)
///
/// This is a convenience wrapper that creates a default configuration
/// with the specified host and port.
///
/// # Arguments
/// * `host` - Host to bind to (e.g., "0.0.0.0" or "127.0.0.1")
/// * `port` - Port to listen on
pub async fn start_server(host: &str, port: u16) -> Result<(), std::io::Error> {
    let mut config = ServiceConfig::with_defaults();
    config.server.host = host.to_string();
    config.server.port = port;

    start_server_with_config(config).await
}

/// Outcome of the startup calibration load (roadmap S5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadOutcome {
    /// At least one calibration loaded — the service can answer gain requests.
    Healthy,
    /// Nothing loaded and `calibration.fail_fast` is off. The server starts so that
    /// `/health` and `/status` can report *why* it is useless, but readiness stays false
    /// so no load balancer routes real traffic to it.
    Degraded,
}

/// Load the calibration repository and classify the outcome (roadmap S5).
///
/// This is the seam that makes `calibration.fail_fast` real. Before S5,
/// `start_server_with_config` caught the load error unconditionally and continued with an
/// empty repository, so the shipped `fail_fast: true` default was silently ignored.
///
/// # Returns
/// * `Ok((repo, Healthy))` — at least one calibration loaded.
/// * `Ok((empty, Degraded))` — load failed but `fail_fast` is off; start anyway, not ready.
/// * `Err(io::Error)` — load failed and `fail_fast` is on. The caller returns this up to
///   `main`, which logs it and exits nonzero. No `process::exit` inside the API layer.
pub(crate) fn initialize_repository(
    config: &crate::config::CalibrationConfig,
) -> Result<(CalibrationRepository, LoadOutcome), std::io::Error> {
    match CalibrationRepository::load_from_config(config) {
        Ok(repo) => {
            info!(
                antenna_count = repo.antenna_count(),
                calibration_count = repo.calibration_count(),
                "Calibration data loaded successfully"
            );
            Ok((repo, LoadOutcome::Healthy))
        }
        Err(e) if config.fail_fast => {
            // Relative calibration paths (the shipped default) resolve against the
            // process's current directory, which is easy to get wrong when launching from
            // somewhere other than the workspace root. Naming the CWD here is what turns
            // "No such file or directory" into an actionable diagnosis instead of a guess.
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            tracing::error!(
                error = %e,
                current_dir = %cwd,
                "Failed to load calibration data and calibration.fail_fast is set; refusing to start"
            );
            Err(std::io::Error::other(format!(
                "calibration load failed and calibration.fail_fast is enabled: {e} \
                 (current working directory: {cwd}; relative calibration paths in \
                 config are resolved against this directory)"
            )))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to load calibration data; starting DEGRADED with an empty repository \
                 (readiness stays false, /health reports degraded). Set calibration.fail_fast \
                 to abort startup instead."
            );
            Ok((CalibrationRepository::new(), LoadOutcome::Degraded))
        }
    }
}

/// Apply the configured rayon worker-thread count to the global pool (roadmap S4).
///
/// `worker_threads == 0` is a no-op (rayon auto-detects from CPU count — the sane
/// default). A positive value calls [`rayon::ThreadPoolBuilder::build_global`], which
/// succeeds at most **once per process**. If a global pool already exists (e.g. a prior
/// call, or a test harness that touched rayon), `build_global` returns `Err`; we log a
/// warning and continue on the existing pool rather than failing startup. This never
/// panics (repo rule: no `unwrap`/`expect`/`panic` on the production path).
fn apply_worker_threads(worker_threads: usize) {
    if worker_threads == 0 {
        return; // auto-detect
    }
    match rayon::ThreadPoolBuilder::new()
        .num_threads(worker_threads)
        .build_global()
    {
        Ok(()) => {
            info!(
                worker_threads,
                "Configured the global rayon thread pool from performance.worker_threads"
            );
        }
        Err(e) => {
            tracing::warn!(
                requested = worker_threads,
                effective = rayon::current_num_threads(),
                error = %e,
                "performance.worker_threads requested but a global rayon pool already \
                 exists; continuing on the existing pool"
            );
        }
    }
}

/// Wait for shutdown signal (SIGTERM or SIGINT)
///
/// This function listens for:
/// - Ctrl+C (SIGINT) - typical terminal interrupt
/// - SIGTERM - typical Kubernetes pod termination signal
///
/// When a signal is received, the function returns, allowing
/// the server to begin graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            tracing::error!("Failed to install Ctrl+C handler: {}", e);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!("Failed to install SIGTERM handler: {}", e);
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl+C signal");
        },
        _ = terminate => {
            info!("Received SIGTERM signal");
        },
    }
}

/// First half of graceful shutdown: stop advertising readiness, then pause (roadmap S5).
///
/// Runs *inside* the future handed to `run_with_graceful_shutdown`, because poem stops
/// accepting new connections the moment that future resolves. Flipping readiness and
/// sleeping here — rather than after — is what gives load balancers a window to route new
/// traffic elsewhere while this instance is still able to serve it.
async fn begin_shutdown(state: &AppState, readiness_delay: std::time::Duration) {
    state.mark_not_ready();
    info!(
        readiness_delay_secs = readiness_delay.as_secs(),
        "Readiness set to false; pausing before draining in-flight requests"
    );

    if !readiness_delay.is_zero() {
        tokio::time::sleep(readiness_delay).await;
    }
}

/// Perform graceful shutdown cleanup
///
/// This function is called during shutdown to:
/// - Close active connections gracefully
/// - Flush logs and metrics
/// - Release resources
/// - Log final status
///
/// Future enhancements may include:
/// - Draining request queues
/// - Saving in-memory state
/// - Notifying dependent services
pub async fn shutdown_cleanup(state: &AppState) {
    let uptime = state.uptime_seconds();

    info!(
        version = state.version,
        uptime_seconds = uptime,
        "Server shutting down"
    );

    // Future: Add cleanup tasks here
    // - Close database connections
    // - Flush metrics
    // - Save state if needed

    info!("Shutdown cleanup completed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_creation() {
        let config = ServiceConfig::with_defaults();
        let repository = CalibrationRepository::new();
        let state = AppState::new(config, repository);

        assert_eq!(state.version, env!("CARGO_PKG_VERSION"));
        assert!(state.uptime_seconds() == 0 || state.uptime_seconds() == 1);
    }

    #[test]
    fn test_app_state_uptime() {
        use std::thread::sleep;
        use std::time::Duration;

        let state = AppState::with_defaults();
        let uptime1 = state.uptime_seconds();

        sleep(Duration::from_millis(100));

        let uptime2 = state.uptime_seconds();
        assert!(uptime2 >= uptime1);
    }

    #[test]
    fn test_app_state_bind_address() {
        let config = ServiceConfig::with_defaults();
        let repository = CalibrationRepository::new();
        let state = AppState::new(config, repository);

        assert_eq!(state.bind_address(), "127.0.0.1:3000");
    }

    #[test]
    fn test_app_state_with_custom_config() {
        let mut config = ServiceConfig::with_defaults();
        config.server.host = "0.0.0.0".to_string();
        config.server.port = 8080;

        let repository = CalibrationRepository::new();
        let state = AppState::new(config, repository);
        assert_eq!(state.bind_address(), "0.0.0.0:8080");
    }

    #[test]
    fn test_app_state_clone() {
        let state1 = AppState::with_defaults();
        let state2 = state1.clone();

        // Both should have the same start time
        assert_eq!(state1.start_time, state2.start_time);
        assert_eq!(state1.version, state2.version);
    }

    #[tokio::test]
    async fn test_shutdown_cleanup() {
        let state = AppState::with_defaults();

        // Should not panic
        shutdown_cleanup(&state).await;
    }

    #[test]
    fn test_app_state_starts_not_ready() {
        // Readiness is a startup lifecycle signal (roadmap S5): it must be false until
        // calibration data has actually loaded. A default-constructed state has loaded
        // nothing, so it must not advertise readiness.
        let state = AppState::with_defaults();
        assert!(
            !state.is_ready(),
            "AppState must start NOT ready; readiness is set only after the calibration load"
        );

        state.mark_ready();
        assert!(state.is_ready());
        state.mark_not_ready();
        assert!(!state.is_ready());
    }

    /// Build a CalibrationConfig pointing at a nonexistent antenna config file.
    fn broken_calibration_config(fail_fast: bool) -> crate::config::CalibrationConfig {
        crate::config::CalibrationConfig {
            data_directory: std::env::temp_dir().join("s5_does_not_exist"),
            antenna_config_file: std::env::temp_dir().join("s5_does_not_exist/antennas.yaml"),
            fail_fast,
        }
    }

    #[test]
    fn test_initialize_repository_fail_fast_returns_err() {
        // calibration.fail_fast = true means "refuse to start on a load failure". Before
        // S5 this Err was swallowed and the server booted empty (roadmap S5).
        // Note: can't use `.expect_err(...)` here — it requires the Ok type (which
        // includes CalibrationRepository) to be Debug, and CalibrationRepository
        // deliberately does not derive Debug (see Task 2's note).
        let err = match initialize_repository(&broken_calibration_config(true)) {
            Ok(_) => panic!("fail_fast must propagate the load failure to the caller"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("fail_fast"),
            "the startup error must name the knob that caused the abort, got: {msg}"
        );
    }

    #[test]
    fn test_initialize_repository_without_fail_fast_starts_degraded() {
        let (repo, outcome) = initialize_repository(&broken_calibration_config(false))
            .expect("fail_fast=false must start the server anyway");
        assert_eq!(outcome, LoadOutcome::Degraded);
        assert_eq!(
            repo.antenna_count(),
            0,
            "a degraded start serves from an empty repository"
        );
    }

    #[test]
    fn test_initialize_repository_healthy_on_real_fixtures() {
        // test_antennas.yaml's calibration_file entries (e.g.
        // "tests/fixtures/calibration_data/...") are written relative to the crate root,
        // not to a `data_directory` of the fixtures dir itself — so data_directory must be
        // CARGO_MANIFEST_DIR for every entry to resolve. fail_fast: true is the point of
        // this test: it proves the full fixture set (5 antennas / 7 calibrations) loads
        // cleanly, not just "at least one antenna survived a partial failure".
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let config = crate::config::CalibrationConfig {
            data_directory: std::path::PathBuf::from(manifest_dir),
            antenna_config_file: std::path::PathBuf::from(manifest_dir)
                .join("tests/fixtures/test_antennas.yaml"),
            fail_fast: true,
        };

        let (repo, outcome) = initialize_repository(&config).expect("fixture config must load cleanly under fail_fast: true — all fixture entries resolve from CARGO_MANIFEST_DIR");
        assert_eq!(outcome, LoadOutcome::Healthy);
        assert!(
            repo.antenna_count() > 0,
            "a healthy load must produce a non-empty repository"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_begin_shutdown_flips_readiness_and_waits() {
        use std::time::Duration;
        use tokio::time::Instant;

        let state = AppState::with_defaults();
        state.mark_ready();
        assert!(state.is_ready());

        let started = Instant::now();
        begin_shutdown(&state, Duration::from_secs(5)).await;

        assert!(
            !state.is_ready(),
            "shutdown must flip readiness false so load balancers stop routing new traffic"
        );
        assert!(
            started.elapsed() >= Duration::from_secs(5),
            "the pre-drain delay must actually elapse"
        );
    }

    #[tokio::test]
    async fn test_begin_shutdown_zero_delay_does_not_wait() {
        use std::time::Duration;

        let state = AppState::with_defaults();
        state.mark_ready();

        // Default config: no delay. Must still flip readiness, and must not sleep.
        begin_shutdown(&state, Duration::ZERO).await;
        assert!(!state.is_ready());
    }

    #[test]
    fn test_apply_worker_threads_never_panics() {
        // Force the global rayon pool to exist first (a trivial parallel op initializes
        // it at its default size), so the subsequent positive-value call deterministically
        // takes `build_global`'s already-initialized `Err` branch — which must log and
        // continue, never panic. Doing it this way also avoids pinning the shared test
        // process's pool to a small thread count. The `0` call is the no-op path.
        use rayon::prelude::*;
        let _warm: i32 = (0..8).into_par_iter().sum();

        apply_worker_threads(0); // auto-detect: no-op
        apply_worker_threads(4); // pool already exists -> graceful Err path, no panic
        assert!(rayon::current_num_threads() >= 1);
    }
}
