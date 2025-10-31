//! Calibration tool library
//!
//! This library provides the core functionality for the antenna calibration CLI tool.

pub mod antenna_config;

pub use antenna_config::{
    AntennaClass, AntennaClassRegistry, AntennaConfiguration, AntennaMetadata, FeedParameters,
    MeshParameters, ParameterBounds, ReflectorGeometry, SurfaceParameters, TunableParameters,
};
