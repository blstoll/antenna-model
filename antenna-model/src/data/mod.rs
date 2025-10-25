//! Data management module for antenna calibration.
//!
//! This module contains data structures, serialization, and repository
//! functionality for managing antenna calibration data.

pub mod types;

// Re-export commonly used types for convenience
pub use types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, BSplineModel4DBuilder,
    CalibrationMetadata, CalibrationMetadataBuilder, ValidityRanges, ValidityRangesBuilder,
    ValidationError,
};
