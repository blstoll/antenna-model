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
    validate_calibration, ValidationConfig, ValidationReport, CrossValidationResults,
    OutlierPoint, FrequencyBandStats, AngularRegionStats, ValidationError,
};

pub use serializer::{
    save_artifact, load_artifact, export_metadata_json, export_validation_json,
    CalibrationArtifact, ArtifactMetadata, SerializationError, artifact_format_info,
};

pub use design_specs_loader::{
    DesignSpecs, FeedSpecs, MeshSpecs, ReflectorSpecs, TuningBounds,
};

pub use boresight_calibration::{
    calibrate_boresight, build_calibration_artifact, BoresightMeasurement,
    BoresightMeasurements, BoresightTunableParameters, BoresightCalibrationResult,
};

pub use frequency_correction::{
    fit_frequency_correction, should_fit_correction, FrequencyCorrectionError,
};
