//! Calibration-artifact data types: what one `.bin` artifact carries for one
//! antenna-feed combination.
//!
//! [`AntennaCalibration`] is the root; every other type here is reachable from it.
//! Structs are constructed with their `builder()`, whose `build()` fails naming the first
//! required field left unset and does not validate; check a value with `validate()`.
//!
//! # Wire format: postcard is positional
//!
//! Artifacts are encoded with [`postcard`], which is non-self-describing and decodes
//! fields by position. On any type reachable from [`AntennaCalibration`], do **not**
//! add `#[serde(skip_serializing_if = ...)]`, `#[serde(skip)]`, `#[serde(flatten)]`, or
//! untagged/internally-tagged enums: a conditionally omitted field shifts every field
//! after it and decodes into the wrong values without error. `#[serde(default)]` alone
//! is harmless. Adding, removing, reordering, or retyping a field changes the layout and
//! requires a schema MAJOR and container bump — see [`CALIBRATION_SCHEMA_VERSION`].

mod angular_resolution;
mod antenna_calibration;
mod bspline_model;
mod calibration_coverage;
mod calibration_status;
mod feed_parameters;
mod measurement_density;
mod mesh_parameters;
mod metadata;
mod parameter_source;
mod physical_config;
mod reflector_geometry;
mod validation_error;
mod validity_ranges;

pub use angular_resolution::{AngularResolution, MIN_KNOTS_PER_LOBE_PERIOD};
pub use antenna_calibration::{AntennaCalibration, AntennaCalibrationBuilder};
pub use bspline_model::{BSplineModel4D, BSplineModel4DBuilder};
pub use calibration_coverage::{
    CalibrationCoverage, CalibrationCoverageBuilder, BORESIGHT_COVERAGE_CONE_DEG,
};
pub use calibration_status::CalibrationStatus;
pub use feed_parameters::{FeedParameters, FeedParametersBuilder};
pub use measurement_density::MeasurementDensity;
pub use mesh_parameters::{MeshParameters, MeshParametersBuilder};
pub use metadata::{CalibrationMetadata, CalibrationMetadataBuilder, CALIBRATION_SCHEMA_VERSION};
pub use parameter_source::ParameterSource;
pub use physical_config::{PhysicalAntennaConfig, PhysicalAntennaConfigBuilder};
pub use reflector_geometry::{ReflectorGeometry, ReflectorGeometryBuilder};
pub use validation_error::ValidationError;
pub use validity_ranges::{ValidityRanges, ValidityRangesBuilder};
