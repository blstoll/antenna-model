//! Antenna Model Service Library
//!
//! This library provides the core functionality for the antenna model service,
//! including REST API server, B-spline interpolation, and calibration data management.

// Compiler and linter configuration
#![deny(unsafe_code)]
// Warn about unwrap/expect/panic in production code, but allow in tests
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
// Allow missing docs for builder patterns and internal implementation details
#![allow(missing_docs, missing_debug_implementations)]

pub mod api;
pub mod config;
pub mod data;
pub mod error;
pub mod model;
pub mod service;

// Re-export commonly used types for convenience
pub use data::{AntennaCalibration, BSplineModel4D, CalibrationMetadata, ValidityRanges};

pub use config::{AntennaConfig, ServiceConfig};

// Re-export error types from error module
pub use error::{
    AntennaModelError, ApiError, ApiResult, ComputationError, ComputationResult, ConfigError,
    ConfigResult, DataError, DataResult, ErrorContext, Result, ValidationError, ValidationResult,
};
