//! Antenna Model Service Library
//!
//! This library provides the core functionality for the antenna model service,
//! including REST API server, B-spline interpolation, and calibration data management.

pub mod api;
pub mod config;
pub mod data;

// Re-export commonly used types for convenience
pub use data::{
    AntennaCalibration, BSplineModel4D, CalibrationMetadata, ValidityRanges, ValidationError,
};

pub use config::{AntennaConfig, ConfigError, ServiceConfig};
