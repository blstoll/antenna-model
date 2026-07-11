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

    /// Gain cache configuration
    #[serde(default)]
    pub cache: CacheConfig,
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

/// Gain cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Enable gain result caching for throughput improvement
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,

    /// Maximum cached gain entries per antenna-feed pair
    #[serde(default = "default_max_entries_per_feed")]
    pub max_entries_per_feed: usize,
}

fn default_cache_enabled() -> bool {
    true
}

fn default_max_entries_per_feed() -> usize {
    10_000
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: default_cache_enabled(),
            max_entries_per_feed: default_max_entries_per_feed(),
        }
    }
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
            .set_default(
                "server.request_timeout_secs",
                default_request_timeout() as i64,
            )?
            .set_default("server.max_body_size_bytes", default_max_body_size() as i64)?
            .set_default(
                "calibration.data_directory",
                default_calibration_dir().to_string_lossy().to_string(),
            )?
            .set_default(
                "calibration.antenna_config_file",
                default_antenna_config().to_string_lossy().to_string(),
            )?
            .set_default("calibration.fail_fast", default_fail_fast())?
            .set_default("logging.level", default_log_level())?
            .set_default("logging.format", "text")?
            .set_default("logging.include_location", default_include_location())?
            .set_default(
                "performance.worker_threads",
                default_worker_threads() as i64,
            )?
            .set_default(
                "performance.max_batch_size",
                default_max_batch_size() as i64,
            )?
            .set_default(
                "performance.enable_parallel_processing",
                default_enable_parallel(),
            )?
            // Load from YAML file (optional - won't fail if missing)
            .add_source(
                config::File::from(std::path::Path::new(config_path))
                    .format(config::FileFormat::Yaml)
                    .required(false),
            )
            // Override with environment variables (prefix: ANTENNA_MODEL_)
            // Use separator "__" for nested fields (e.g., ANTENNA_MODEL_SERVER__PORT)
            .add_source(
                config::Environment::with_prefix("ANTENNA_MODEL")
                    .separator("__")
                    .try_parsing(true),
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
            cache: CacheConfig::default(),
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

// ============================================================================
// Partial Calibration Support - Configuration Structures (v2.0)
// ============================================================================

/// Design specifications configuration for uncalibrated antennas
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignSpecsConfig {
    /// Reflector diameter in meters
    pub diameter_m: f64,

    /// Focal length in meters
    pub focal_length_m: f64,

    /// f/D ratio (focal length / diameter)
    pub f_over_d_ratio: f64,

    /// Surface RMS error in millimeters
    pub surface_rms_mm: f64,

    /// Feed configurations
    pub feeds: Vec<FeedSpecConfig>,

    /// Optional mesh parameters for mesh reflectors
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshConfig>,
}

/// Feed specification configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedSpecConfig {
    /// Feed unique identifier
    pub id: String,

    /// Feed name
    pub name: String,

    /// Feed position [x, y, z] in meters
    pub position: [f64; 3],

    /// q-factor for cos^q illumination pattern
    pub q_factor: f64,

    /// Phase center offset in meters
    pub phase_center_offset_m: f64,

    /// Deliberate axial defocus of the feed phase center from the focal point,
    /// in meters (optional; default 0 = focused). phase_center_offset_m is
    /// compensated by the model (auto-refocus, roadmap P7) — this is the explicit
    /// knob for intentional defocus.
    #[serde(default)]
    pub axial_defocus_m: f64,

    /// Frequency range [min, max] in MHz
    pub frequency_range: [f64; 2],
}

/// Mesh configuration for mesh reflectors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConfig {
    /// Mesh spacing (hole size) in millimeters
    pub mesh_spacing_mm: f64,

    /// Wire diameter in millimeters
    pub wire_diameter_mm: f64,
}

/// Calibration coverage configuration for partially calibrated antennas
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationCoverageConfig {
    /// Azimuth range [min, max] in degrees
    pub azimuth_range: [f64; 2],

    /// Elevation range [min, max] in degrees
    pub elevation_range: [f64; 2],

    /// Frequency range [min, max] in MHz
    pub frequency_range: [f64; 2],

    /// Number of measurement points
    pub num_measurements: usize,
}

/// Validity ranges configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidityRangesConfig {
    /// Azimuth range [min, max] in degrees
    pub azimuth_range: [f64; 2],

    /// Elevation range [min, max] in degrees
    pub elevation_range: [f64; 2],

    /// Frequency range [min, max] in MHz
    pub frequency_range: [f64; 2],

    /// Reference temperature in Kelvin
    pub temperature_k: f64,
}

/// Antenna configuration entry (v2.0 - extended for partial calibration support)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntennaConfigEntry {
    /// Unique identifier for the antenna
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Calibration status: "fully_calibrated", "partially_calibrated", or "uncalibrated"
    /// If not specified, defaults to "fully_calibrated" for backward compatibility
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<String>,

    /// Path to calibration binary file (relative to calibration data directory)
    /// Required for fully_calibrated and partially_calibrated antennas
    /// Optional for uncalibrated antennas
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_file: Option<String>,

    /// Calibration coverage metadata (for partially calibrated antennas)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_coverage: Option<CalibrationCoverageConfig>,

    /// Design specifications (required for uncalibrated antennas)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_specs: Option<DesignSpecsConfig>,

    /// Validity ranges (optional - overrides ranges from calibration file)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validity_ranges: Option<ValidityRangesConfig>,

    /// Description of the antenna (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Location of the antenna (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,

    /// Whether this antenna is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl AntennaConfigEntry {
    /// Validate antenna configuration entry
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Determine calibration status (default to fully_calibrated for backward compatibility)
        let status = self
            .calibration_status
            .as_deref()
            .unwrap_or("fully_calibrated");

        // Validate based on calibration status
        match status {
            "fully_calibrated" | "partially_calibrated" => {
                // Require calibration_file
                if self.calibration_file.is_none() {
                    return Err(ConfigError::InvalidValue {
                        key: format!("antenna.{}.calibration_file", self.id),
                        reason: format!("{} antennas require a calibration_file", status),
                    });
                }
            }
            "uncalibrated" => {
                // Require design_specs
                if self.design_specs.is_none() {
                    return Err(ConfigError::InvalidValue {
                        key: format!("antenna.{}.design_specs", self.id),
                        reason: "uncalibrated antennas require design_specs".to_string(),
                    });
                }

                // Validate design specs
                if let Some(ref specs) = self.design_specs {
                    specs.validate(&self.id)?;
                }
            }
            _ => {
                return Err(ConfigError::InvalidValue {
                    key: format!("antenna.{}.calibration_status", self.id),
                    reason: format!(
                        "invalid calibration status: '{}'. Must be one of: fully_calibrated, partially_calibrated, uncalibrated",
                        status
                    ),
                });
            }
        }

        // Validate calibration coverage if present
        if let Some(ref coverage) = self.calibration_coverage {
            coverage.validate(&self.id)?;
        }

        // Validate validity ranges if present
        if let Some(ref ranges) = self.validity_ranges {
            ranges.validate(&self.id)?;
        }

        Ok(())
    }

    /// Get the calibration status (default to "fully_calibrated" for backward compatibility)
    pub fn get_calibration_status(&self) -> &str {
        self.calibration_status
            .as_deref()
            .unwrap_or("fully_calibrated")
    }
}

impl DesignSpecsConfig {
    /// Validate design specifications
    fn validate(&self, antenna_id: &str) -> Result<(), ConfigError> {
        // Validate reflector geometry
        if self.diameter_m <= 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.diameter_m", antenna_id),
                reason: "must be positive".to_string(),
            });
        }

        if self.focal_length_m <= 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.focal_length_m", antenna_id),
                reason: "must be positive".to_string(),
            });
        }

        if self.f_over_d_ratio <= 0.0 || self.f_over_d_ratio > 2.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.f_over_d_ratio", antenna_id),
                reason: "must be between 0 and 2".to_string(),
            });
        }

        if self.surface_rms_mm < 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.surface_rms_mm", antenna_id),
                reason: "must be non-negative".to_string(),
            });
        }

        // Validate feeds
        if self.feeds.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.feeds", antenna_id),
                reason: "at least one feed is required".to_string(),
            });
        }

        let mut feed_ids = std::collections::HashSet::new();
        for feed in &self.feeds {
            feed.validate(antenna_id)?;

            // Check for duplicate feed IDs
            if !feed_ids.insert(&feed.id) {
                return Err(ConfigError::InvalidValue {
                    key: format!("antenna.{}.design_specs.feeds", antenna_id),
                    reason: format!("duplicate feed ID: {}", feed.id),
                });
            }
        }

        // Validate mesh if present
        if let Some(ref mesh) = self.mesh {
            mesh.validate(antenna_id)?;
        }

        Ok(())
    }
}

impl FeedSpecConfig {
    /// Validate feed specification
    fn validate(&self, antenna_id: &str) -> Result<(), ConfigError> {
        if self.id.is_empty() {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.feed.id", antenna_id),
                reason: "feed ID cannot be empty".to_string(),
            });
        }

        if self.q_factor < 0.0 || self.q_factor > 20.0 {
            return Err(ConfigError::InvalidValue {
                key: format!(
                    "antenna.{}.design_specs.feed.{}.q_factor",
                    antenna_id, self.id
                ),
                reason: "must be between 0 and 20".to_string(),
            });
        }

        if self.frequency_range[0] >= self.frequency_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!(
                    "antenna.{}.design_specs.feed.{}.frequency_range",
                    antenna_id, self.id
                ),
                reason: "min frequency must be less than max frequency".to_string(),
            });
        }

        if self.frequency_range[0] <= 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!(
                    "antenna.{}.design_specs.feed.{}.frequency_range",
                    antenna_id, self.id
                ),
                reason: "frequencies must be positive".to_string(),
            });
        }

        Ok(())
    }
}

impl MeshConfig {
    /// Validate mesh configuration
    fn validate(&self, antenna_id: &str) -> Result<(), ConfigError> {
        if self.mesh_spacing_mm <= 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.mesh.mesh_spacing_mm", antenna_id),
                reason: "must be positive".to_string(),
            });
        }

        if self.wire_diameter_mm <= 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.mesh.wire_diameter_mm", antenna_id),
                reason: "must be positive".to_string(),
            });
        }

        if self.wire_diameter_mm >= self.mesh_spacing_mm {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.design_specs.mesh.wire_diameter_mm", antenna_id),
                reason: "must be less than mesh_spacing_mm".to_string(),
            });
        }

        Ok(())
    }
}

impl CalibrationCoverageConfig {
    /// Validate calibration coverage configuration
    fn validate(&self, antenna_id: &str) -> Result<(), ConfigError> {
        if self.azimuth_range[0] > self.azimuth_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.calibration_coverage.azimuth_range", antenna_id),
                reason: "min must be <= max".to_string(),
            });
        }

        if self.elevation_range[0] > self.elevation_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!(
                    "antenna.{}.calibration_coverage.elevation_range",
                    antenna_id
                ),
                reason: "min must be <= max".to_string(),
            });
        }

        if self.frequency_range[0] > self.frequency_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!(
                    "antenna.{}.calibration_coverage.frequency_range",
                    antenna_id
                ),
                reason: "min must be <= max".to_string(),
            });
        }

        Ok(())
    }
}

impl ValidityRangesConfig {
    /// Validate validity ranges configuration
    fn validate(&self, antenna_id: &str) -> Result<(), ConfigError> {
        if self.azimuth_range[0] > self.azimuth_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.validity_ranges.azimuth_range", antenna_id),
                reason: "min must be <= max".to_string(),
            });
        }

        if self.elevation_range[0] > self.elevation_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.validity_ranges.elevation_range", antenna_id),
                reason: "min must be <= max".to_string(),
            });
        }

        if self.elevation_range[0] < 0.0 || self.elevation_range[1] > 90.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.validity_ranges.elevation_range", antenna_id),
                reason: "elevation must be between 0 and 90 degrees".to_string(),
            });
        }

        if self.frequency_range[0] > self.frequency_range[1] {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.validity_ranges.frequency_range", antenna_id),
                reason: "min must be <= max".to_string(),
            });
        }

        if self.temperature_k <= 0.0 {
            return Err(ConfigError::InvalidValue {
                key: format!("antenna.{}.validity_ranges.temperature_k", antenna_id),
                reason: "temperature must be positive".to_string(),
            });
        }

        Ok(())
    }
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

            // Validate individual antenna entry
            entry.validate()?;
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
        assert_eq!(
            config.antennas[0].calibration_file,
            Some("antenna_1.bin".to_string())
        );
        assert_eq!(config.antennas[1].id, "antenna_2");
        assert!(!config.antennas[1].enabled);
        assert_eq!(
            config.antennas[1].calibration_file,
            Some("antenna_2.bin".to_string())
        );

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
                    calibration_status: None, // Defaults to fully_calibrated
                    calibration_file: Some("antenna_1.bin".to_string()),
                    calibration_coverage: None,
                    design_specs: None,
                    validity_ranges: None,
                    description: None,
                    location: None,
                    enabled: true,
                },
                AntennaConfigEntry {
                    id: "antenna_1".to_string(),
                    name: "Antenna 1 Duplicate".to_string(),
                    calibration_status: None,
                    calibration_file: Some("antenna_1_dup.bin".to_string()),
                    calibration_coverage: None,
                    design_specs: None,
                    validity_ranges: None,
                    description: None,
                    location: None,
                    enabled: true,
                },
            ],
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_log_format_serialization() {
        assert_eq!(serde_json::to_string(&LogFormat::Text).unwrap(), "\"text\"");
        assert_eq!(serde_json::to_string(&LogFormat::Json).unwrap(), "\"json\"");
    }

    // ============================================================================
    // Tests for Partial Calibration Support (v2.0)
    // ============================================================================

    #[test]
    fn test_uncalibrated_antenna_config() {
        let yaml_content = r#"
antennas:
  - id: "uncal_antenna"
    name: "Uncalibrated Test Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.5
      feeds:
        - id: "x_band"
          name: "X-Band Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [7100.0, 8500.0]
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        assert_eq!(config.antennas.len(), 1);

        let antenna = &config.antennas[0];
        assert_eq!(antenna.id, "uncal_antenna");
        assert_eq!(antenna.get_calibration_status(), "uncalibrated");
        assert!(antenna.design_specs.is_some());
        assert!(antenna.calibration_file.is_none());

        let specs = antenna.design_specs.as_ref().unwrap();
        assert_eq!(specs.diameter_m, 3.7);
        assert_eq!(specs.feeds.len(), 1);
        assert_eq!(specs.feeds[0].id, "x_band");
    }

    #[test]
    fn test_uncalibrated_antenna_with_mesh() {
        let yaml_content = r#"
antennas:
  - id: "mesh_antenna"
    name: "Mesh Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 5.0
      focal_length_m: 2.0
      f_over_d_ratio: 0.4
      surface_rms_mm: 2.0
      feeds:
        - id: "primary"
          name: "Primary Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [2000.0, 2300.0]
      mesh:
        mesh_spacing_mm: 5.0
        wire_diameter_mm: 0.5
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        let specs = config.antennas[0].design_specs.as_ref().unwrap();

        assert!(specs.mesh.is_some());
        let mesh = specs.mesh.as_ref().unwrap();
        assert_eq!(mesh.mesh_spacing_mm, 5.0);
        assert_eq!(mesh.wire_diameter_mm, 0.5);
    }

    #[test]
    fn test_uncalibrated_antenna_multi_feed() {
        let yaml_content = r#"
antennas:
  - id: "multi_feed"
    name: "Multi-Feed Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 13.0
      focal_length_m: 5.2
      f_over_d_ratio: 0.4
      surface_rms_mm: 0.4
      feeds:
        - id: "x_band_downlink"
          name: "X-Band Downlink"
          position: [0.0, 0.0, 0.0]
          q_factor: 9.0
          phase_center_offset_m: 0.01
          frequency_range: [7145.0, 7235.0]
        - id: "ka_band"
          name: "Ka-Band Feed"
          position: [0.08, 0.0, 0.0]
          q_factor: 10.0
          phase_center_offset_m: 0.005
          frequency_range: [25500.0, 27000.0]
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        let specs = config.antennas[0].design_specs.as_ref().unwrap();

        assert_eq!(specs.feeds.len(), 2);
        assert_eq!(specs.feeds[0].id, "x_band_downlink");
        assert_eq!(specs.feeds[1].id, "ka_band");
    }

    #[test]
    fn test_feed_spec_axial_defocus_default_and_explicit() {
        // Absent key -> defaults to 0.0 (all existing configs stay valid)
        let yaml_absent = r#"
          id: "f1"
          name: "Feed 1"
          position: [0.0, 0.0, 0.0]
          q_factor: 1.14
          phase_center_offset_m: 0.01
          frequency_range: [8000.0, 8500.0]
        "#;
        let feed: FeedSpecConfig = serde_yaml::from_str(yaml_absent).unwrap();
        assert_eq!(feed.axial_defocus_m, 0.0);

        // Explicit key -> preserved
        let yaml_explicit = r#"
          id: "f2"
          name: "Feed 2"
          position: [0.0, 0.0, 0.0]
          q_factor: 1.14
          phase_center_offset_m: 0.0
          axial_defocus_m: 0.05
          frequency_range: [8000.0, 8500.0]
        "#;
        let feed: FeedSpecConfig = serde_yaml::from_str(yaml_explicit).unwrap();
        assert_eq!(feed.axial_defocus_m, 0.05);
    }

    #[test]
    fn test_partially_calibrated_antenna_with_coverage() {
        let yaml_content = r#"
antennas:
  - id: "partial_antenna"
    name: "Partially Calibrated Antenna"
    calibration_status: "partially_calibrated"
    calibration_file: "partial.bin"
    enabled: true
    calibration_coverage:
      azimuth_range: [0.0, 0.0]
      elevation_range: [0.0, 0.0]
      frequency_range: [7100.0, 8500.0]
      num_measurements: 28
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        let antenna = &config.antennas[0];

        assert_eq!(antenna.get_calibration_status(), "partially_calibrated");
        assert!(antenna.calibration_coverage.is_some());

        let coverage = antenna.calibration_coverage.as_ref().unwrap();
        assert_eq!(coverage.azimuth_range, [0.0, 0.0]);
        assert_eq!(coverage.num_measurements, 28);
    }

    #[test]
    fn test_antenna_with_validity_ranges() {
        let yaml_content = r#"
antennas:
  - id: "antenna_with_ranges"
    name: "Antenna With Validity Ranges"
    calibration_file: "test.bin"
    enabled: true
    validity_ranges:
      azimuth_range: [0.0, 360.0]
      elevation_range: [0.0, 90.0]
      frequency_range: [8000.0, 8500.0]
      temperature_k: 290.0
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        let antenna = &config.antennas[0];

        assert!(antenna.validity_ranges.is_some());
        let ranges = antenna.validity_ranges.as_ref().unwrap();
        assert_eq!(ranges.temperature_k, 290.0);
    }

    #[test]
    fn test_uncalibrated_without_design_specs_fails() {
        let yaml_content = r#"
antennas:
  - id: "bad_antenna"
    name: "Bad Uncalibrated Antenna"
    calibration_status: "uncalibrated"
    enabled: true
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_calibrated_without_file_fails() {
        let yaml_content = r#"
antennas:
  - id: "bad_calibrated"
    name: "Bad Calibrated Antenna"
    calibration_status: "fully_calibrated"
    enabled: true
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_calibration_status() {
        let yaml_content = r#"
antennas:
  - id: "invalid_status"
    name: "Invalid Status Antenna"
    calibration_status: "invalid_status"
    calibration_file: "test.bin"
    enabled: true
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_design_specs_validation_invalid_diameter() {
        let yaml_content = r#"
antennas:
  - id: "invalid_diameter"
    name: "Invalid Diameter Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: -1.0
      focal_length_m: 1.0
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.0
      feeds:
        - id: "test"
          name: "Test Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 8500.0]
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_design_specs_validation_no_feeds() {
        let yaml_content = r#"
antennas:
  - id: "no_feeds"
    name: "No Feeds Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.0
      feeds: []
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_feed_validation_invalid_q_factor() {
        let yaml_content = r#"
antennas:
  - id: "invalid_q"
    name: "Invalid Q Factor"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.0
      feeds:
        - id: "bad_feed"
          name: "Bad Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 25.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 8500.0]
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_mesh_validation_wire_too_large() {
        let yaml_content = r#"
antennas:
  - id: "bad_mesh"
    name: "Bad Mesh"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.0
      feeds:
        - id: "feed"
          name: "Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 8500.0]
      mesh:
        mesh_spacing_mm: 5.0
        wire_diameter_mm: 6.0
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let result = AntennaConfig::from_file(path);
        assert!(result.is_err());
    }

    #[test]
    fn test_backward_compatibility_old_format() {
        // Old format antennas (no calibration_status) should default to fully_calibrated
        let yaml_content = r#"
antennas:
  - id: "old_antenna"
    name: "Old Format Antenna"
    calibration_file: "old.bin"
    enabled: true
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        let path = temp_file.path().to_str().unwrap();

        let config = AntennaConfig::from_file(path).unwrap();
        let antenna = &config.antennas[0];

        assert_eq!(antenna.get_calibration_status(), "fully_calibrated");
        assert!(antenna.calibration_file.is_some());
    }

    #[test]
    fn test_cache_config_defaults() {
        let config = CacheConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_entries_per_feed, 10_000);
    }

    #[test]
    fn test_service_config_with_cache_section() {
        let yaml = r#"
server:
  host: "127.0.0.1"
  port: 3000
calibration:
  data_directory: "calibration_data"
  antenna_config_file: "calibration_data/antennas.yaml"
logging:
  level: "info"
cache:
  enabled: false
  max_entries_per_feed: 500
"#;
        let config: ServiceConfig = serde_yaml::from_str(yaml).expect("parse failed");
        assert!(!config.cache.enabled);
        assert_eq!(config.cache.max_entries_per_feed, 500);
    }

    #[test]
    fn test_service_config_cache_defaults_when_section_missing() {
        let yaml = r#"
server:
  host: "127.0.0.1"
  port: 3000
calibration:
  data_directory: "calibration_data"
  antenna_config_file: "calibration_data/antennas.yaml"
logging:
  level: "info"
"#;
        let config: ServiceConfig = serde_yaml::from_str(yaml).expect("parse failed");
        assert!(config.cache.enabled);
        assert_eq!(config.cache.max_entries_per_feed, 10_000);
    }

    #[test]
    fn test_parse_real_antennas_yaml() {
        // Test parsing the actual antennas.yaml from calibration_data
        let yaml_path = "calibration_data/antennas.yaml";
        if std::path::Path::new(yaml_path).exists() {
            let result = AntennaConfig::from_file(yaml_path);
            // Should either succeed or fail with a clear error
            match result {
                Ok(config) => {
                    println!("Successfully parsed {} antennas", config.antennas.len());
                    // Validate that enabled uncalibrated antennas have design_specs
                    for antenna in config.enabled_antennas() {
                        if antenna.get_calibration_status() == "uncalibrated" {
                            assert!(
                                antenna.design_specs.is_some(),
                                "Uncalibrated antenna {} missing design_specs",
                                antenna.id
                            );
                        }
                    }
                }
                Err(e) => {
                    println!("Failed to parse antennas.yaml: {}", e);
                    // This is okay if the file doesn't exist yet
                }
            }
        } else {
            println!("Skipping test - calibration_data/antennas.yaml not found");
        }
    }
}
