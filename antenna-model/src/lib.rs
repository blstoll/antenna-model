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
pub mod service;

// The physics engine and the shared error/warning vocabulary live in
// `antenna-core` (roadmap D4); re-export the modules so every existing
// `antenna_model::{error,model,warnings}::…` path — and `crate::…` within
// this crate — keeps resolving unchanged.
pub use antenna_core::{error, model, warnings};

// Re-export commonly used types for convenience
pub use data::{AntennaCalibration, BSplineModel4D, CalibrationMetadata, ValidityRanges};

pub use config::{AntennaConfig, ServiceConfig};

// Re-export the response-warning vocabulary (roadmap C8 stage 3)
pub use warnings::{ApiWarning, WarningCode};

// Re-export error types from error module
pub use error::{
    AntennaModelError, ApiError, ApiResult, ComputationError, ComputationResult, ConfigError,
    ConfigResult, DataError, DataResult, ErrorContext, Result, ValidationError, ValidationResult,
};
