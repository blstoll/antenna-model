//! Antenna Model Service Library
//!
//! This library provides the core functionality for the antenna model service,
//! including REST API server, B-spline interpolation, and calibration data management.

pub mod error;
pub mod api;
pub mod config;
pub mod data;

// Re-export commonly used types for convenience
pub use data::{AntennaCalibration, BSplineModel4D, CalibrationMetadata, ValidityRanges};

pub use config::{AntennaConfig, ServiceConfig};

// Re-export error types from error module
pub use error::{
    AntennaModelError, ApiError, ApiResult, ComputationError, ComputationResult, ConfigError,
    ConfigResult, DataError, DataResult, ErrorContext, Result, ValidationError, ValidationResult,
};
