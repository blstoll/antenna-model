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
use poem::{listener::TcpListener, Server};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::signal;
use tracing::info;

/// Application state shared across handlers
///
/// This state is thread-safe (via Arc) and contains:
/// - Server metadata (version, start time)
/// - Configuration settings
/// - Future: Calibration repository, connection pools
#[derive(Clone)]
pub struct AppState {
    /// Server start time for uptime calculation
    pub start_time: SystemTime,

    /// Application version from Cargo.toml
    pub version: &'static str,

    /// Service configuration
    pub config: Arc<ServiceConfig>,
}

impl AppState {
    /// Create new application state with configuration
    pub fn new(config: ServiceConfig) -> Self {
        Self {
            start_time: SystemTime::now(),
            version: env!("CARGO_PKG_VERSION"),
            config: Arc::new(config),
        }
    }

    /// Create application state with default configuration
    pub fn with_defaults() -> Self {
        Self::new(ServiceConfig::with_defaults())
    }

    /// Get uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Get server bind address from configuration
    pub fn bind_address(&self) -> String {
        self.config.bind_address()
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
    let state = Arc::new(AppState::new(config.clone()));

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
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
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
        let state = AppState::new(config);

        assert_eq!(state.version, env!("CARGO_PKG_VERSION"));
        assert!(state.uptime_seconds() == 0 || state.uptime_seconds() == 1);
    }

    #[test]
    fn test_app_state_uptime() {
        use std::time::Duration;
        use std::thread::sleep;

        let state = AppState::with_defaults();
        let uptime1 = state.uptime_seconds();

        sleep(Duration::from_millis(100));

        let uptime2 = state.uptime_seconds();
        assert!(uptime2 >= uptime1);
    }

    #[test]
    fn test_app_state_bind_address() {
        let config = ServiceConfig::with_defaults();
        let state = AppState::new(config);

        assert_eq!(state.bind_address(), "127.0.0.1:3000");
    }

    #[test]
    fn test_app_state_with_custom_config() {
        let mut config = ServiceConfig::with_defaults();
        config.server.host = "0.0.0.0".to_string();
        config.server.port = 8080;

        let state = AppState::new(config);
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
