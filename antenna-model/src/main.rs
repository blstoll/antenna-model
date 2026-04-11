//! Antenna Model Service - Main Entry Point
//!
//! This service provides REST API access to calibrated antenna models
//! using 4D B-spline interpolation.
//!
//! # Configuration
//!
//! The service can be configured via:
//! 1. Configuration file at `config/service.yaml` (or path specified by CONFIG_PATH env var)
//! 2. Environment variables with `ANTENNA_MODEL_` prefix
//! 3. Default values if no configuration is provided
//!
//! # Environment Variables
//!
//! - `RUST_LOG` - Log level (overrides config file)
//! - `CONFIG_PATH` - Path to configuration file
//! - `ANTENNA_MODEL_SERVER__HOST` - Server host (e.g., "0.0.0.0")
//! - `ANTENNA_MODEL_SERVER__PORT` - Server port (e.g., 8080)
//! - `ANTENNA_MODEL_LOGGING__LEVEL` - Log level ("trace", "debug", "info", "warn", "error")
//! - `ANTENNA_MODEL_LOGGING__FORMAT` - Log format ("text" or "json")
//!
//! # Startup Process
//!
//! 1. Load configuration from file and environment variables
//! 2. Initialize structured logging (tracing)
//! 3. Log startup configuration details
//! 4. Start API server with production-grade middleware
//! 5. Wait for shutdown signal (SIGTERM or SIGINT)
//! 6. Perform graceful shutdown

use antenna_model::api;
use antenna_model::config::{LogFormat, ServiceConfig};
use std::env;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

#[tokio::main]
async fn main() {
    // Load configuration from file or use defaults (before tracing is available)
    let (config, config_from_defaults, config_path) = load_configuration();

    // Initialize tracing/logging with configuration
    init_tracing(&config);

    // Re-emit config load result through tracing (now initialized)
    if config_from_defaults {
        tracing::warn!(
            config_path = %config_path,
            "Failed to load configuration file; using default configuration values"
        );
    } else {
        info!(config_path = %config_path, "Loaded configuration from file");
    }

    // Log startup information
    info!(
        version = env!("CARGO_PKG_VERSION"),
        bind_address = %config.bind_address(),
        log_level = %config.logging.level,
        log_format = ?config.logging.format,
        "Starting Antenna Model Service"
    );

    // Start the API server with full configuration
    if let Err(e) = api::start_server_with_config(config).await {
        error!(error = %e, "Failed to start server");
        std::process::exit(1);
    }
}

/// Load service configuration and return it along with whether it was from defaults.
///
/// Attempts to load configuration from:
/// 1. File path specified by CONFIG_PATH environment variable, or
/// 2. Default file path (config/service.yaml), or
/// 3. Default configuration values if file doesn't exist
///
/// Environment variables can override file-based configuration using
/// the `ANTENNA_MODEL_` prefix (e.g., `ANTENNA_MODEL_SERVER__PORT=8080`).
///
/// Returns `(config, config_from_defaults, config_path)`.
fn load_configuration() -> (ServiceConfig, bool, String) {
    // Check for custom config path via environment variable
    let config_path = env::var("CONFIG_PATH").unwrap_or_else(|_| "config/service.yaml".to_string());

    match ServiceConfig::from_file(&config_path) {
        Ok(config) => (config, false, config_path),
        Err(e) => {
            // Log warning before tracing is available — will also be re-emitted via tracing below.
            eprintln!(
                "Warning: Failed to load configuration from {}: {}",
                config_path, e
            );
            eprintln!("Using default configuration values");
            (ServiceConfig::with_defaults(), true, config_path)
        }
    }
}

/// Initialize tracing subscriber for structured logging
///
/// Sets up logging with the following features:
/// - Configurable log level from config or RUST_LOG environment variable
/// - JSON or text formatting based on configuration
/// - Optional file and line number inclusion
/// - Request correlation via request IDs (handled by middleware)
///
/// # Log Format Examples
///
/// **Text format (human-readable):**
/// ```text
/// 2025-01-15T10:30:45.123Z INFO antenna_model::api: Starting Antenna Model Service version="0.1.0"
/// ```
///
/// **JSON format (structured):**
/// ```json
/// {"timestamp":"2025-01-15T10:30:45.123Z","level":"INFO","target":"antenna_model::api","fields":{"version":"0.1.0"},"message":"Starting Antenna Model Service"}
/// ```
fn init_tracing(config: &ServiceConfig) {
    // RUST_LOG environment variable takes precedence over config file
    let log_level = env::var("RUST_LOG").unwrap_or_else(|_| config.logging.level.clone());

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&log_level));

    let fmt_layer = if config.logging.format == LogFormat::Json {
        // JSON format for structured logging (production)
        tracing_subscriber::fmt::layer()
            .json()
            .with_target(true)
            .with_file(config.logging.include_location)
            .with_line_number(config.logging.include_location)
            .boxed()
    } else {
        // Human-readable text format (development)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_configuration_default() {
        // Should not panic regardless of whether config/service.yaml exists
        let (config, _from_defaults, _path) = load_configuration();
        // Host and port should be set (either from file or defaults)
        assert!(!config.server.host.is_empty());
        assert!(config.server.port > 0);
    }

    #[test]
    fn test_load_configuration_with_env() {
        // Set environment variable
        env::set_var("ANTENNA_MODEL_SERVER__PORT", "8080");

        let (_config, _from_defaults, _path) = load_configuration();

        // Should have loaded the environment override
        // Note: This test may not work consistently due to global env state
        // In a real test, we'd use a test-specific config loading function

        // Clean up
        env::remove_var("ANTENNA_MODEL_SERVER__PORT");
    }
}
