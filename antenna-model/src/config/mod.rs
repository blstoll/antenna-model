//! Configuration management for the Antenna Model Service
//!
//! This module provides configuration loading from YAML files with support for
//! environment variable overrides.

pub mod settings;

// Re-export commonly used types
pub use settings::{LogFormat, ServiceConfig};

// Re-export all types for advanced usage
#[allow(unused_imports)]
pub use settings::{
    AntennaConfig, AntennaConfigEntry, CalibrationConfig, FeedSpecConfig, LoggingConfig,
    PerformanceConfig, ServerConfig,
};
