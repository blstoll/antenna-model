use std::fmt;

/// Why a calibration artifact, or one of its parts, is not internally consistent.
///
/// Returned by every `validate()` in [`crate::types`]. Each message names the field or
/// dimension at fault.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// A required field is empty.
    EmptyField(String),

    /// The coefficient count does not match the declared shape.
    InconsistentShape { expected: usize, actual: usize },

    /// A knot vector violates the knot-vector invariant set.
    InvalidKnotVector { dimension: String, reason: String },

    /// The spline order is out of range.
    InvalidSplineOrder(u8),

    /// A correction surface's temperature slabs differ; every slab must be identical.
    TemperatureDependentCorrection {
        temperature_slab: usize,
        coefficient_index: usize,
        expected: f64,
        actual: f64,
    },

    /// A range has `min > max` or lies outside physical bounds.
    InvalidRange {
        dimension: String,
        min: f64,
        max: f64,
    },

    /// A temperature is not positive.
    InvalidTemperature(f64),

    /// A physical parameter is outside its valid domain.
    InvalidPhysicalParameter {
        parameter: String,
        value: f64,
        reason: String,
    },

    /// A recorded [`super::AngularResolution`] field cannot be interpreted.
    InvalidAngularResolution {
        field: String,
        value: f64,
        reason: String,
    },

    /// A correction surface is present with no calibration coverage record.
    MissingCoverage,

    /// Calibration coverage extends beyond the correction surface's fitted support on
    /// `dimension`. Containment is inclusive at both bounds.
    CoverageExceedsSupport {
        dimension: String,
        coverage: (f64, f64),
        support: (f64, f64),
    },

    /// A `PartiallyCalibrated` status carries coverage that differs from the artifact's
    /// `calibration_coverage`.
    CoverageRecordsDisagree,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::EmptyField(field) => {
                write!(f, "Required field '{}' is empty", field)
            }
            ValidationError::InconsistentShape { expected, actual } => {
                write!(
                    f,
                    "Coefficient array size {} doesn't match shape {}",
                    actual, expected
                )
            }
            ValidationError::InvalidKnotVector { dimension, reason } => {
                write!(f, "Invalid knot vector for {}: {}", dimension, reason)
            }
            ValidationError::InvalidSplineOrder(order) => {
                write!(f, "Invalid spline order: {}", order)
            }
            ValidationError::TemperatureDependentCorrection {
                temperature_slab,
                coefficient_index,
                expected,
                actual,
            } => write!(
                f,
                "Correction surface temperature slab {} differs from slab 0 at spatial \
                 coefficient {} (expected {}, got {}); schema 5.1 requires every \
                 temperature slab to be identical",
                temperature_slab, coefficient_index, expected, actual
            ),
            ValidationError::InvalidRange {
                dimension,
                min,
                max,
            } => {
                write!(f, "Invalid range for {}: [{}, {}]", dimension, min, max)
            }
            ValidationError::InvalidTemperature(temp) => {
                write!(f, "Invalid temperature: {} K", temp)
            }
            ValidationError::InvalidPhysicalParameter {
                parameter,
                value,
                reason,
            } => {
                write!(
                    f,
                    "Invalid physical parameter '{}' = {}: {}",
                    parameter, value, reason
                )
            }
            ValidationError::InvalidAngularResolution {
                field,
                value,
                reason,
            } => {
                write!(
                    f,
                    "Invalid angular-resolution field '{}' = {}: {}",
                    field, value, reason
                )
            }
            ValidationError::MissingCoverage => write!(
                f,
                "correction_surface is present but calibration_coverage is absent; a \
                 correction may only be applied where measurements justify it, so the \
                 artifact must record that coverage — regenerate it with calibrate"
            ),
            ValidationError::CoverageExceedsSupport {
                dimension,
                coverage,
                support,
            } => write!(
                f,
                "calibration_coverage {} range [{}, {}] is not contained by the \
                 correction_surface's fitted support [{}, {}]; coverage must lie within \
                 support (bounds inclusive)",
                dimension, coverage.0, coverage.1, support.0, support.1
            ),
            ValidationError::CoverageRecordsDisagree => write!(
                f,
                "calibration_status.coverage and calibration_coverage disagree; a \
                 PartiallyCalibrated artifact must carry the same coverage in both"
            ),
        }
    }
}

impl std::error::Error for ValidationError {}
