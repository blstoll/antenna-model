//! Antenna Calibration Tool Library
//!
//! This library provides the core functionality for calibrating antenna models
//! from measurement data. It supports:
//!
//! - Antenna configuration with hybrid parameter approach
//! - Physical parameter optimization (optional)
//! - Correction surface fitting to residuals
//! - Validation and quality metrics

pub mod antenna_config;
pub mod parser;

// Re-export commonly used types
pub use antenna_config::{
    AntennaClass, AntennaClassRegistry, AntennaConfiguration, AntennaMetadata, FeedParameters,
    MeshParameters, ParameterBounds, ReflectorGeometry, SurfaceParameters, TunableParameters,
};

pub use parser::{
    create_sample_csv, parse_measurements, parse_measurements_sync, DataQualityReport,
    MeasurementData, MeasurementPoint,
};
