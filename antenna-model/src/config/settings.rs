//! Configuration settings for the Antenna Model Service
//!
//! This module provides configuration loading from YAML files with environment variable overrides.

use crate::error::ConfigError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Server configuration
    pub server: ServerConfig,

    /// Calibration data configuration
    pub calibration: CalibrationConfig,

    /// Logging configuration
    pub logging: LoggingConfig,

    /// Performance tuning parameters
    #[serde(default)]
    pub performance: PerformanceConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host to bind to (e.g., "0.0.0.0" or "127.0.0.1")
    #[serde(default = "default_host")]
    pub host: String,

    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,

    /// Request timeout in seconds
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,

    /// Maximum request body size in bytes
    #[serde(default = "default_max_body_size")]
    pub max_body_size_bytes: usize,
}

/// Calibration data configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationConfig {
    /// Directory containing calibration artifacts
    #[serde(default = "default_calibration_dir")]
    pub data_directory: PathBuf,

    /// Antenna configuration file path
    #[serde(default = "default_antenna_config")]
    pub antenna_config_file: PathBuf,

    /// Whether to fail fast on missing calibration data
    #[serde(default = "default_fail_fast")]
    pub fail_fast: bool,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format (text or json)
    #[serde(default = "default_log_format")]
    pub format: LogFormat,

    /// Whether to include file and line numbers
    #[serde(default = "default_include_location")]
    pub include_location: bool,
}

/// Log format options
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable text format
    Text,
    /// JSON format for structured logging
    Json,
}

/// Performance tuning configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Number of worker threads for batch processing (0 = auto-detect)
    #[serde(default = "default_worker_threads")]
    pub worker_threads: usize,

    /// Maximum batch size for batch evaluation requests
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,

    /// Enable parallel processing for batch requests
    #[serde(default = "default_enable_parallel")]
    pub enable_parallel_processing: bool,
}

// Default value functions
fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

fn default_request_timeout() -> u64 {
    30
}

fn default_max_body_size() -> usize {
    10 * 1024 * 1024 // 10 MB
}

fn default_calibration_dir() -> PathBuf {
    PathBuf::from("calibration_data")
}

fn default_antenna_config() -> PathBuf {
    PathBuf::from("calibration_data/antennas.yaml")
}

fn default_fail_fast() -> bool {
    true
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> LogFormat {
    LogFormat::Text
}

fn default_include_location() -> bool {
    false
}

fn default_worker_threads() -> usize {
    0 // Auto-detect
}

fn default_max_batch_size() -> usize {
    1000
}

fn default_enable_parallel() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            request_timeout_secs: default_request_timeout(),
            max_body_size_bytes: default_max_body_size(),
        }
    }
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self {
            data_directory: default_calibration_dir(),
            antenna_config_file: default_antenna_config(),
            fail_fast: default_fail_fast(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
            include_location: default_include_location(),
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            worker_threads: default_worker_threads(),
            max_batch_size: default_max_batch_size(),
            enable_parallel_processing: default_enable_parallel(),
        }
    }
}

impl ServiceConfig {
    /// Load configuration from a YAML file path with environment variable overrides
    ///
    /// # Arguments
    /// * `config_path` - Path to the YAML configuration file
    ///
    /// # Environment Variables
    /// Configuration values can be overridden using environment variables with the prefix
    /// `ANTENNA_MODEL_`. For example:
    /// - `ANTENNA_MODEL_SERVER__PORT=8080` overrides `server.port`
    /// - `ANTENNA_MODEL_LOGGING__LEVEL=debug` overrides `logging.level`
    ///
    /// # Example
    /// ```no_run
    /// use antenna_model::config::ServiceConfig;
    ///
    /// let config = ServiceConfig::from_file("config/service.yaml").unwrap();
    /// println!("Server listening on {}:{}", config.server.host, config.server.port);
    /// ```
    pub fn from_file(config_path: &str) -> Result<Self, ConfigError> {
        let settings = config::Config::builder()
            // Start with default values
            .set_default("server.host", default_host())?
            .set_default("server.port", default_port() as i64)?
            .set_default("server.request_timeout_secs", default_request_timeout() as i64)?
            .set_default("server.max_body_size_bytes", default_max_body_size() as i64)?
            .set_default("calibration.data_directory", default_calibration_dir().to_string_lossy().to_string())?
            .set_default("calibration.antenna_config_file", default_antenna_config().to_string_lossy().to_string())?
            .set_default("calibration.fail_fast", default_fail_fast())?
            .set_default("logging.level", default_log_level())?
            .set_default("logging.format", "text")?
            .set_default("logging.include_location", default_include_location())?
            .set_default("performance.worker_threads", default_worker_threads() as i64)?
            .set_default("performance.max_batch_size", default_max_batch_size() as i64)?
            .set_default("performance.enable_parallel_processing", default_enable_parallel())?
            // Load from YAML file (optional - won't fail if missing)
            .add_source(
                config::File::from(std::path::Path::new(config_path))
                    .format(config::FileFormat::Yaml)
                    .required(false)
            )
            // Override with environment variables (prefix: ANTENNA_MODEL_)
            // Use separator "__" for nested fields (e.g., ANTENNA_MODEL_SERVER__PORT)
            .add_source(
                config::Environment::with_prefix("ANTENNA_MODEL")
                    .separator("__")
                    .try_parsing(true)
            )
            .build()?;

        let config: ServiceConfig = settings.try_deserialize()?;
        config.validate()?;

        Ok(config)
    }

    /// Load configuration from the default path ("config/service.yaml")
    pub fn from_default_file() -> Result<Self, ConfigError> {
        Self::from_file("config/service.yaml")
    }

    /// Create configuration with all default values
    pub fn with_defaults() -> Self {
        Self {
            server: ServerConfig::default(),
            calibration: CalibrationConfig::default(),
            logging: LoggingConfig::default(),
            performance: PerformanceConfig::default(),
        }
    }

    /// Validate the configuration
    fn validate(&self) -> Result<(), ConfigError> {
        // Validate server configuration
        if self.server.port == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.port".to_string(),
                reason: "must be greater than 0".to_string(),
            });
        }

        if self.server.request_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.request_timeout_secs".to_string(),
                reason: "must be greater than 0".to_string(),
            });
        }

        if self.server.max_body_size_bytes == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.max_body_size_bytes".to_string(),
                reason: "must be greater than 0".to_string(),
            });
        }

        // Validate logging configuration
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.logging.level.to_lowercase().as_str()) {
            return Err(ConfigError::InvalidValue {
                key: "logging.level".to_string(),
                reason: format!("must be one of: {}", valid_levels.join(", ")),
            });
        }

        // Validate performance configuration
        if self.performance.max_batch_size == 0 {
            return Err(ConfigError::InvalidValue {
                key: "performance.max_batch_size".to_string(),
                reason: "must be greater than 0".to_string(),
            });
        }

        Ok(())
    }

    /// Get the server bind address as a string
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}

/// Antenna configuration entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntennaConfigEntry {
    /// Unique identifier for the antenna
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Path to calibration binary file (relative to calibration data directory)
    pub calibration_file: String,

    /// Whether this antenna is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Antenna configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntennaConfig {
    /// List of antenna configurations
    pub antennas: Vec<AntennaConfigEntry>,
}

impl AntennaConfig {
    /// Load antenna configuration from a YAML file
    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path).map_err(|e| ConfigError::FileNotFound {
            path: format!("{}: {}", path, e),
        })?;

        let config: AntennaConfig =
            serde_yaml::from_str(&contents).map_err(|e| ConfigError::ParseError {
                path: path.to_string(),
                reason: format!("Failed to parse antenna config: {}", e),
            })?;

        config.validate()?;
        Ok(config)
    }

    /// Validate antenna configuration
    fn validate(&self) -> Result<(), ConfigError> {
        if self.antennas.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "antennas".to_string(),
                reason: "at least one antenna configuration is required".to_string(),
            });
        }

        // Check for duplicate antenna IDs
        let mut ids = std::collections::HashSet::new();
        for entry in &self.antennas {
            if entry.id.is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "antenna.id".to_string(),
                    reason: "antenna ID cannot be empty".to_string(),
                });
            }
            if !ids.insert(&entry.id) {
                return Err(ConfigError::InvalidValue {
                    key: "antenna.id".to_string(),
                    reason: format!("duplicate antenna ID: {}", entry.id),
                });
            }
        }

        Ok(())
    }

    /// Get enabled antennas only
    pub fn enabled_antennas(&self) -> Vec<&AntennaConfigEntry> {
        self.antennas.iter().filter(|a| a.enabled).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_service_config_defaults() {
        let config = ServiceConfig::with_defaults();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.logging.level, "info");
        assert_eq!(config.performance.max_batch_size, 1000);
    }

    #[test]
    fn test_service_config_from_yaml() {
        let yaml_content = r#"
server:
  host: "0.0.0.0"
  port: 8080

calibration:
  data_directory: "/app/data"
  antenna_config_file: "/app/data/antennas.yaml"

logging:
  level: "debug"
  format: "json"
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = ServiceConfig::from_file(path).unwrap();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.format, LogFormat::Json);
    }

    #[test]
    fn test_service_config_validation() {
        let mut config = ServiceConfig::with_defaults();

        // Valid config should pass
        assert!(config.validate().is_ok());

        // Invalid port
        config.server.port = 0;
        assert!(config.validate().is_err());
        config.server.port = 3000;

        // Invalid log level
        config.logging.level = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_bind_address() {
        let config = ServiceConfig::with_defaults();
        assert_eq!(config.bind_address(), "127.0.0.1:3000");
    }

    #[test]
    fn test_antenna_config_from_yaml() {
        let yaml_content = r#"
antennas:
  - id: "antenna_1"
    name: "Test Antenna 1"
    calibration_file: "antenna_1.bin"
    enabled: true

  - id: "antenna_2"
    name: "Test Antenna 2"
    calibration_file: "antenna_2.bin"
    enabled: false
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        assert_eq!(config.antennas.len(), 2);
        assert_eq!(config.antennas[0].id, "antenna_1");
        assert!(config.antennas[0].enabled);
        assert_eq!(config.antennas[1].id, "antenna_2");
        assert!(!config.antennas[1].enabled);

        let enabled = config.enabled_antennas();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "antenna_1");
    }

    #[test]
    fn test_antenna_config_validation() {
        // Empty config should fail
        let config = AntennaConfig { antennas: vec![] };
        assert!(config.validate().is_err());

        // Duplicate IDs should fail
        let config = AntennaConfig {
            antennas: vec![
                AntennaConfigEntry {
                    id: "antenna_1".to_string(),
                    name: "Antenna 1".to_string(),
                    calibration_file: "antenna_1.bin".to_string(),
                    enabled: true,
                },
                AntennaConfigEntry {
                    id: "antenna_1".to_string(),
                    name: "Antenna 1 Duplicate".to_string(),
                    calibration_file: "antenna_1_dup.bin".to_string(),
                    enabled: true,
                },
            ],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_log_format_serialization() {
        assert_eq!(
            serde_json::to_string(&LogFormat::Text).unwrap(),
            "\"text\""
        );
        assert_eq!(
            serde_json::to_string(&LogFormat::Json).unwrap(),
            "\"json\""
        );
    }
}
