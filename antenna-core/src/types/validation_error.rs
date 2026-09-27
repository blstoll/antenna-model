use thiserror::Error;

/// Why a calibration artifact, or one of its parts, cannot exist.
///
/// Returned by [`super::AntennaCalibrationBuilder::build`] and by the artifact loader.
/// Each message names the field or dimension at fault.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    /// A required builder field was never set.
    #[error("{0} is required")]
    MissingField(&'static str),

    /// A required field is empty.
    #[error("Required field '{0}' is empty")]
    EmptyField(String),

    /// The coefficient count does not match the declared shape.
    #[error("Coefficient array size {actual} doesn't match shape {expected}")]
    InconsistentShape { expected: usize, actual: usize },

    /// A knot vector violates the knot-vector invariant set.
    #[error("Invalid knot vector for {dimension}: {reason}")]
    InvalidKnotVector { dimension: String, reason: String },

    /// The spline order is out of range.
    #[error("Invalid spline order: {0}")]
    InvalidSplineOrder(u8),

    /// A correction surface's temperature slabs differ; every slab must be identical.
    #[error(
        "Correction surface temperature slab {temperature_slab} differs from slab 0 at \
         spatial coefficient {coefficient_index} (expected {expected}, got {actual}); \
         schema 5.1 requires every temperature slab to be identical"
    )]
    TemperatureDependentCorrection {
        temperature_slab: usize,
        coefficient_index: usize,
        expected: f64,
        actual: f64,
    },

    /// A range has `min > max` or lies outside physical bounds.
    #[error("Invalid range for {dimension}: [{min}, {max}]")]
    InvalidRange {
        dimension: String,
        min: f64,
        max: f64,
    },

    /// A temperature is not positive.
    #[error("Invalid temperature: {0} K")]
    InvalidTemperature(f64),

    /// A physical parameter is outside its valid domain.
    #[error("Invalid physical parameter '{parameter}' = {value}: {reason}")]
    InvalidPhysicalParameter {
        parameter: String,
        value: f64,
        reason: String,
    },

    /// A recorded [`super::AngularResolution`] field cannot be interpreted.
    #[error("Invalid angular-resolution field '{field}' = {value}: {reason}")]
    InvalidAngularResolution {
        field: String,
        value: f64,
        reason: String,
    },

    /// A correction surface is present with no calibration coverage record.
    #[error(
        "correction_surface is present but calibration_coverage is absent; a correction \
         may only be applied where measurements justify it, so the artifact must record \
         that coverage — regenerate it with calibrate"
    )]
    MissingCoverage,

    /// Calibration coverage extends beyond the correction surface's fitted support on
    /// `dimension`. Containment is inclusive at both bounds.
    #[error(
        "calibration_coverage {dimension} range [{}, {}] is not contained by the \
         correction_surface's fitted support [{}, {}]; coverage must lie within support \
         (bounds inclusive)",
        coverage.0, coverage.1, support.0, support.1
    )]
    CoverageExceedsSupport {
        dimension: String,
        coverage: (f64, f64),
        support: (f64, f64),
    },

    /// A `PartiallyCalibrated` status carries coverage that differs from the artifact's
    /// `calibration_coverage`.
    #[error(
        "calibration_status.coverage and calibration_coverage disagree; a \
         PartiallyCalibrated artifact must carry the same coverage in both"
    )]
    CoverageRecordsDisagree,
}
