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

// Re-export commonly used types
pub use antenna_config::{
    AntennaClass, AntennaClassRegistry, AntennaConfiguration, AntennaMetadata, FeedParameters,
    MeshParameters, ParameterBounds, ReflectorGeometry, SurfaceParameters, TunableParameters,
};
