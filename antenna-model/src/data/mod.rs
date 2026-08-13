//! Data management module for antenna calibration.
//!
//! The artifact types and ANTC loader live in `antenna-core` (roadmap D4) and
//! are re-exported here so existing `antenna_model::data::…` paths keep
//! resolving. The service-side repository stays local — it depends on the
//! service configuration system.

pub use antenna_core::data::{loader, types};

pub mod repository;

// Re-export commonly used types for convenience
pub use types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, BSplineModel4DBuilder,
    CalibrationMetadata, CalibrationMetadataBuilder, ValidationError, ValidityRanges,
    ValidityRangesBuilder,
};

// Re-export repository for easy access
pub use repository::CalibrationRepository;
