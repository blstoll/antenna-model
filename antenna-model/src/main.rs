//! Antenna Model Service - Main Entry Point
//!
//! This service provides REST API access to calibrated antenna models
//! using 4D B-spline interpolation.

use antenna_model::api;
use antenna_model::config::{LogFormat, ServiceConfig};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

#[tokio::main]
async fn main() {
    // Load configuration from file or use defaults
    let config = load_configuration();

    // Initialize tracing/logging with configuration
    init_tracing(&config);

    // Log startup information
    info!(
        host = %config.server.host,
        port = config.server.port,
        log_level = %config.logging.level,
        "Starting Antenna Model Service"
    );

    // Start the API server with configured host and port
    if let Err(e) = api::start_server(&config.server.host, config.server.port).await {
        error!(error = %e, "Failed to start server");
        std::process::exit(1);
    }
}

/// Load service configuration
///
/// Attempts to load configuration from the default file path (config/service.yaml).
/// Falls back to default configuration if the file doesn't exist or fails to parse.
/// Environment variables can override file-based configuration.
fn load_configuration() -> ServiceConfig {
    match ServiceConfig::from_default_file() {
        Ok(config) => {
            // Successfully loaded configuration
            config
        }
        Err(e) => {
            // Log warning and use defaults if config file is missing or invalid
            eprintln!("Warning: Failed to load configuration: {}", e);
            eprintln!("Using default configuration values");
            ServiceConfig::with_defaults()
        }
    }
}

/// Initialize tracing subscriber for structured logging
///
/// Sets up logging with the following features:
/// - Configurable log level from config or RUST_LOG environment variable
/// - JSON or text formatting based on configuration
/// - Optional file and line number inclusion
fn init_tracing(config: &ServiceConfig) {
    // RUST_LOG environment variable takes precedence over config file
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| config.logging.level.clone());

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&log_level));

    let fmt_layer = if config.logging.format == LogFormat::Json {
        // JSON format for structured logging
        tracing_subscriber::fmt::layer()
            .json()
            .with_target(true)
            .with_file(config.logging.include_location)
            .with_line_number(config.logging.include_location)
            .boxed()
    } else {
        // Human-readable text format
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_file(config.logging.include_location)
            .with_line_number(config.logging.include_location)
            .boxed()
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();
}
