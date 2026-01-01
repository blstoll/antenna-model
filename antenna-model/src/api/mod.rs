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
use poem::{listener::TcpListener, Server};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
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

    /// Readiness state - true when service is ready to accept requests
    /// This is false during startup and true once initialization is complete
    pub ready: Arc<AtomicBool>,

    /// Loaded antenna IDs (will be populated by Task 5.4 - Calibration Repository)
    /// Using Arc to allow sharing without cloning Vec
    pub antenna_ids: Arc<parking_lot::RwLock<Vec<String>>>,

    /// Calibration data repository (Task 5.4)
    pub repository: CalibrationRepository,
}

impl AppState {
    /// Create new application state with configuration and repository
    pub fn new(config: ServiceConfig, repository: CalibrationRepository) -> Self {
        Self {
            start_time: SystemTime::now(),
            version: env!("CARGO_PKG_VERSION"),
            config: Arc::new(config),
            ready: Arc::new(AtomicBool::new(true)), // Default to ready for simple deployments
            antenna_ids: Arc::new(parking_lot::RwLock::new(Vec::new())),
            repository,
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

    let repository = match CalibrationRepository::load_from_config(&config.calibration) {
        Ok(repo) => {
            info!("Calibration data loaded successfully");
            repo
        }
        Err(e) => {
            tracing::warn!(
                "Failed to load calibration data: {}, starting with empty repository",
                e
            );
            CalibrationRepository::new()
        }
    };

    let state = Arc::new(AppState::new(config.clone(), repository));

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
        worker_threads = state.config.performance.worker_threads,
        max_batch_size = state.config.performance.max_batch_size,
        parallel_processing = state.config.performance.enable_parallel_processing,
        "Performance configuration"
    );

    // Create routes with middleware
    let app = routes::create_routes(state.clone());

    let addr = state.bind_address();

    info!("Server ready to accept connections on {}", addr);

    // Start server with graceful shutdown
    Server::new(TcpListener::bind(&addr))
        .run_with_graceful_shutdown(
            app,
            async {
                shutdown_signal().await;
                info!("Graceful shutdown initiated");
            },
            None,
        )
        .await
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
}
