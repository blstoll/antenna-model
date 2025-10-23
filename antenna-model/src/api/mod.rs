//! API module - REST API server and routing
//!
//! This module contains the web server setup, routing, handlers, and middleware
//! for the antenna model service REST API.

pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod schemas;

use poem::{listener::TcpListener, Server};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::signal;
use tracing::info;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    /// Server start time for uptime calculation
    pub start_time: SystemTime,
    /// Application version from Cargo.toml
    pub version: &'static str,
}

impl AppState {
    /// Create new application state
    pub fn new() -> Self {
        Self {
            start_time: SystemTime::now(),
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// Get uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Start the API server
///
/// This function initializes the web server with routes and middleware,
/// then starts listening on the specified host and port.
pub async fn start_server(host: &str, port: u16) -> Result<(), std::io::Error> {
    let state = Arc::new(AppState::new());

    info!("Starting antenna model service v{}", state.version);
    info!("Server binding to {}:{}", host, port);

    let app = routes::create_routes(state);

    let addr = format!("{}:{}", host, port);

    info!("Server ready to accept connections");

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

/// Wait for shutdown signal (SIGTERM or SIGINT)
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
