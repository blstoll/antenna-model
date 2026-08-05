//! antenna-core — the physics engine and calibration data model shared by the
//! `antenna-model` REST service and the `calibrate` CLI (roadmap unit D4).
//!
//! This crate deliberately contains no web stack: no poem, no h3o, no tokio.
//! The `openapi` feature gates the `utoipa::ToSchema` derives the service
//! needs for spec generation, so a package-scoped `calibrate` build compiles
//! no utoipa (a whole-workspace build still unifies features across members).

// Compiler and linter configuration (kept identical to antenna-model's —
// moved code must stay under the same unwrap/expect/panic policy).
#![deny(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
#![allow(missing_docs, missing_debug_implementations)]

pub mod error;
pub mod warnings;

// Re-export the response-warning vocabulary (roadmap C8 stage 3)
pub use warnings::{ApiWarning, WarningCode};

// Re-export error types from error module
pub use error::{
    AntennaModelError, ApiError, ApiResult, ComputationError, ComputationResult, ConfigError,
    ConfigResult, DataError, DataResult, ErrorContext, Result, ValidationError, ValidationResult,
};
