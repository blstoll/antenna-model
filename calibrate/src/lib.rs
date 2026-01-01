//! Antenna Calibration Tool Library
//!
//! This library provides the core functionality for calibrating antenna models
//! from measurement data. It supports:
//!
//! - Full calibration from dense measurement grids
//! - Boresight calibration from frequency sweeps at az=0, el=0
//! - Antenna configuration with hybrid parameter approach
//! - Physical parameter optimization (optional)
//! - Correction surface fitting to residuals
//! - Validation and quality metrics
//! - Calibration artifact generation and serialization

// Compiler and linter configuration
#![deny(unsafe_code)]
// Allow missing docs for internal calibration tool details
#![allow(missing_docs, missing_debug_implementations)]
// Allow unwrap/expect in calibration tool (CLI, not production service)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod antenna_config;
pub mod boresight_calibration;
pub mod correction_surface;
pub mod design_specs_loader;
pub mod frequency_correction;
pub mod parameter_tuner;
pub mod parser;
pub mod serializer;
pub mod validator;

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

pub use validator::{
    validate_calibration, AngularRegionStats, CrossValidationResults, FrequencyBandStats,
    OutlierPoint, ValidationConfig, ValidationError, ValidationReport,
};

pub use serializer::{
    artifact_format_info, export_metadata_json, export_validation_json, load_artifact,
    save_artifact, ArtifactMetadata, CalibrationArtifact, SerializationError,
};

pub use design_specs_loader::{DesignSpecs, FeedSpecs, MeshSpecs, ReflectorSpecs, TuningBounds};

pub use boresight_calibration::{
    build_calibration_artifact, calibrate_boresight, BoresightCalibrationResult,
    BoresightMeasurement, BoresightMeasurements, BoresightTunableParameters,
};

pub use frequency_correction::{
    fit_frequency_correction, should_fit_correction, FrequencyCorrectionError,
};
