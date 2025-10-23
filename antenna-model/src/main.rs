//! Antenna Model Service - Main Entry Point
//!
//! This service provides REST API access to calibrated antenna models
//! using 4D B-spline interpolation.

mod api;

use tracing::error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() {
    // Initialize tracing/logging
    init_tracing();

    // Default configuration for Sprint 1
    // In Sprint 3, this will be loaded from config files
    let host = "127.0.0.1";
    let port = 3000;

    // Start the API server
    if let Err(e) = api::start_server(host, port).await {
        error!(error = %e, "Failed to start server");
        std::process::exit(1);
    }
}

/// Initialize tracing subscriber for structured logging
///
/// Sets up logging with the following features:
/// - JSON formatting for structured logs
/// - Environment-based log level filtering (via RUST_LOG)
/// - Defaults to INFO level if RUST_LOG not set
fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();
}
