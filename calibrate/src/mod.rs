//! Calibration tool library
//!
//! This library provides the core functionality for the antenna calibration CLI tool.

pub mod antenna_config;
pub mod parser;

pub use antenna_config::{
    AntennaClass, AntennaClassRegistry, AntennaConfiguration, AntennaMetadata, FeedParameters,
    MeshParameters, ParameterBounds, ReflectorGeometry, SurfaceParameters, TunableParameters,
};

pub use parser::{
    create_sample_csv, parse_measurements, parse_measurements_sync, DataQualityReport,
    MeasurementData, MeasurementPoint,
};
