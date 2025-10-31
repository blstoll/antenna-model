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
pub mod correction_surface;
pub mod parameter_tuner;
pub mod parser;

// Re-export commonly used types
pub use antenna_config::{
    AntennaClass, AntennaClassRegistry, AntennaConfiguration, AntennaMetadata, FeedParameters,
    MeshParameters, ParameterBounds, ReflectorGeometry, SurfaceParameters, TunableParameters,
};

pub use correction_surface::{
    compute_residuals, fit_correction_surface, CorrectionSurface, CorrectionSurfaceError,
    CorrectionSurfaceParams, FitStatistics, ResidualPoint,
};

pub use parameter_tuner::{tune_parameters, TuningMode, TuningResult};

pub use parser::{
    create_sample_csv, parse_measurements, parse_measurements_sync, DataQualityReport,
    MeasurementData, MeasurementPoint,
};
