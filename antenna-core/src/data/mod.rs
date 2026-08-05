//! Calibration artifact data structures and the ANTC loader.
//!
//! The service-side repository (antennas.yaml loading, caching) stays in the
//! `antenna-model` crate (`data::repository`) — it depends on the service
//! configuration system. This module owns the artifact types and the loader
//! shared with `calibrate`.

pub mod loader;
pub mod types;

// Re-export commonly used types for convenience
pub use types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, BSplineModel4DBuilder,
    CalibrationMetadata, CalibrationMetadataBuilder, ValidationError, ValidityRanges,
    ValidityRangesBuilder,
};
