//! Core data types for antenna calibration models.
//!
//! This module defines the fundamental data structures used throughout the antenna
//! model service, including calibration data, B-spline models, and metadata.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Complete calibration data for a single antenna.
///
/// Contains all information needed to evaluate antenna G/T (Gain-to-Temperature)
/// at arbitrary positions and frequencies within the calibrated range.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct AntennaCalibration {
    /// Unique identifier for this antenna
    pub antenna_id: String,

    /// Metadata about the calibration process
    pub metadata: CalibrationMetadata,

    /// 4D B-spline interpolation model
    pub model: BSplineModel4D,

    /// Valid ranges for query parameters
    pub validity_ranges: ValidityRanges,
}

impl AntennaCalibration {
    /// Creates a new builder for constructing an AntennaCalibration.
    pub fn builder() -> AntennaCalibrationBuilder {
        AntennaCalibrationBuilder::default()
    }

    /// Validates that the calibration data is internally consistent.
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Validate antenna ID is not empty
        if self.antenna_id.is_empty() {
            return Err(ValidationError::EmptyField("antenna_id".to_string()));
        }

        // Validate model consistency
        self.model.validate()?;

        // Validate validity ranges
        self.validity_ranges.validate()?;

        Ok(())
    }
}

/// Metadata describing the calibration process and source data.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct CalibrationMetadata {
    /// Human-readable antenna name
    pub antenna_name: String,

    /// ISO 8601 timestamp of calibration
    pub calibration_date: String,

    /// Version of calibration format
    pub format_version: String,

    /// Source of measurement data (e.g., S3 path, file name)
    pub data_source: String,

    /// Root mean squared error of fit (dB)
    pub rmse_db: f64,

    /// R² correlation coefficient of fit
    pub r_squared: f64,

    /// Number of measurement points used in calibration
    pub num_measurements: usize,

    /// Optional notes about the calibration
    pub notes: Option<String>,
}

impl CalibrationMetadata {
    /// Creates a new builder for constructing CalibrationMetadata.
    pub fn builder() -> CalibrationMetadataBuilder {
        CalibrationMetadataBuilder::default()
    }
}

/// 4D B-spline interpolation model.
///
/// Represents a tensor product B-spline over four dimensions:
/// azimuth, elevation, frequency, and temperature.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct BSplineModel4D {
    /// Flattened 4D array of B-spline coefficients.
    /// Indexing: coefficients[i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp))]
    pub coefficients: Vec<f64>,

    /// Shape of coefficient array: [n_azimuth, n_elevation, n_frequency, n_temperature]
    pub shape: [usize; 4],

    /// Knot vector for azimuth dimension (degrees)
    pub knots_azimuth: Vec<f64>,

    /// Knot vector for elevation dimension (degrees)
    pub knots_elevation: Vec<f64>,

    /// Knot vector for frequency dimension (MHz)
    pub knots_frequency: Vec<f64>,

    /// Knot vector for temperature dimension (Kelvin)
    pub knots_temperature: Vec<f64>,

    /// B-spline order (degree + 1). Typically 3 for cubic splines.
    pub spline_order: u8,
}

impl BSplineModel4D {
    /// Creates a new builder for constructing a BSplineModel4D.
    pub fn builder() -> BSplineModel4DBuilder {
        BSplineModel4DBuilder::default()
    }

    /// Validates that the model is internally consistent.
    pub fn validate(&self) -> Result<(), ValidationError> {
        // Check shape consistency
        let expected_size = self.shape.iter().product::<usize>();
        if self.coefficients.len() != expected_size {
            return Err(ValidationError::InconsistentShape {
                expected: expected_size,
                actual: self.coefficients.len(),
            });
        }

        // Check knot vector sizes
        let order = self.spline_order as usize;

        if self.knots_azimuth.len() < self.shape[0] + order {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "azimuth".to_string(),
                reason: format!(
                    "knot vector length {} < shape {} + order {}",
                    self.knots_azimuth.len(),
                    self.shape[0],
                    order
                ),
            });
        }

        if self.knots_elevation.len() < self.shape[1] + order {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "elevation".to_string(),
                reason: format!(
                    "knot vector length {} < shape {} + order {}",
                    self.knots_elevation.len(),
                    self.shape[1],
                    order
                ),
            });
        }

        if self.knots_frequency.len() < self.shape[2] + order {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "frequency".to_string(),
                reason: format!(
                    "knot vector length {} < shape {} + order {}",
                    self.knots_frequency.len(),
                    self.shape[2],
                    order
                ),
            });
        }

        if self.knots_temperature.len() < self.shape[3] + order {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "temperature".to_string(),
                reason: format!(
                    "knot vector length {} < shape {} + order {}",
                    self.knots_temperature.len(),
                    self.shape[3],
                    order
                ),
            });
        }

        // Check knot vectors are non-decreasing
        if !is_non_decreasing(&self.knots_azimuth) {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "azimuth".to_string(),
                reason: "knot vector is not non-decreasing".to_string(),
            });
        }

        if !is_non_decreasing(&self.knots_elevation) {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "elevation".to_string(),
                reason: "knot vector is not non-decreasing".to_string(),
            });
        }

        if !is_non_decreasing(&self.knots_frequency) {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "frequency".to_string(),
                reason: "knot vector is not non-decreasing".to_string(),
            });
        }

        if !is_non_decreasing(&self.knots_temperature) {
            return Err(ValidationError::InvalidKnotVector {
                dimension: "temperature".to_string(),
                reason: "knot vector is not non-decreasing".to_string(),
            });
        }

        // Check spline order is valid
        if self.spline_order < 1 || self.spline_order > 10 {
            return Err(ValidationError::InvalidSplineOrder(self.spline_order));
        }

        Ok(())
    }

    /// Returns the total number of coefficients.
    pub fn num_coefficients(&self) -> usize {
        self.coefficients.len()
    }
}

/// Valid ranges for antenna model parameters.
///
/// Queries outside these ranges will trigger extrapolation warnings.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct ValidityRanges {
    /// Azimuth range in degrees: (min, max)
    pub azimuth_min_max: (f64, f64),

    /// Elevation range in degrees: (min, max)
    pub elevation_min_max: (f64, f64),

    /// Frequency range in MHz: (min, max)
    pub frequency_min_max: (f64, f64),

    /// Constant temperature in Kelvin (for 3D models)
    /// or (min, max) for full 4D temperature support
    pub temperature_const: f64,
}

impl ValidityRanges {
    /// Creates a new builder for constructing ValidityRanges.
    pub fn builder() -> ValidityRangesBuilder {
        ValidityRangesBuilder::default()
    }

    /// Validates that all ranges are well-formed (min <= max).
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.azimuth_min_max.0 > self.azimuth_min_max.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "azimuth".to_string(),
                min: self.azimuth_min_max.0,
                max: self.azimuth_min_max.1,
            });
        }

        if self.elevation_min_max.0 > self.elevation_min_max.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "elevation".to_string(),
                min: self.elevation_min_max.0,
                max: self.elevation_min_max.1,
            });
        }

        if self.frequency_min_max.0 > self.frequency_min_max.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "frequency".to_string(),
                min: self.frequency_min_max.0,
                max: self.frequency_min_max.1,
            });
        }

        // Check reasonable physical ranges
        if self.elevation_min_max.0 < 0.0 || self.elevation_min_max.1 > 90.0 {
            return Err(ValidationError::InvalidRange {
                dimension: "elevation".to_string(),
                min: self.elevation_min_max.0,
                max: self.elevation_min_max.1,
            });
        }

        if self.temperature_const <= 0.0 {
            return Err(ValidationError::InvalidTemperature(self.temperature_const));
        }

        Ok(())
    }

    /// Checks if a query point is within valid ranges.
    pub fn contains(
        &self,
        azimuth: f64,
        elevation: f64,
        frequency: f64,
    ) -> bool {
        azimuth >= self.azimuth_min_max.0
            && azimuth <= self.azimuth_min_max.1
            && elevation >= self.elevation_min_max.0
            && elevation <= self.elevation_min_max.1
            && frequency >= self.frequency_min_max.0
            && frequency <= self.frequency_min_max.1
    }
}

/// Errors that can occur during validation.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// A required field is empty
    EmptyField(String),

    /// Coefficient array size doesn't match shape
    InconsistentShape { expected: usize, actual: usize },

    /// Invalid knot vector
    InvalidKnotVector { dimension: String, reason: String },

    /// Invalid spline order
    InvalidSplineOrder(u8),

    /// Invalid range (min > max or out of physical bounds)
    InvalidRange { dimension: String, min: f64, max: f64 },

    /// Invalid temperature value
    InvalidTemperature(f64),
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
            ValidationError::InvalidRange { dimension, min, max } => {
                write!(f, "Invalid range for {}: [{}, {}]", dimension, min, max)
            }
            ValidationError::InvalidTemperature(temp) => {
                write!(f, "Invalid temperature: {} K", temp)
            }
        }
    }
}

impl std::error::Error for ValidationError {}

// Builder patterns for ergonomic construction

/// Builder for AntennaCalibration.
#[derive(Default)]
pub struct AntennaCalibrationBuilder {
    antenna_id: Option<String>,
    metadata: Option<CalibrationMetadata>,
    model: Option<BSplineModel4D>,
    validity_ranges: Option<ValidityRanges>,
}

impl AntennaCalibrationBuilder {
    pub fn antenna_id(mut self, id: impl Into<String>) -> Self {
        self.antenna_id = Some(id.into());
        self
    }

    pub fn metadata(mut self, metadata: CalibrationMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn model(mut self, model: BSplineModel4D) -> Self {
        self.model = Some(model);
        self
    }

    pub fn validity_ranges(mut self, ranges: ValidityRanges) -> Self {
        self.validity_ranges = Some(ranges);
        self
    }

    pub fn build(self) -> Result<AntennaCalibration, String> {
        Ok(AntennaCalibration {
            antenna_id: self.antenna_id.ok_or("antenna_id is required")?,
            metadata: self.metadata.ok_or("metadata is required")?,
            model: self.model.ok_or("model is required")?,
            validity_ranges: self.validity_ranges.ok_or("validity_ranges is required")?,
        })
    }
}

/// Builder for CalibrationMetadata.
#[derive(Default)]
pub struct CalibrationMetadataBuilder {
    antenna_name: Option<String>,
    calibration_date: Option<String>,
    format_version: Option<String>,
    data_source: Option<String>,
    rmse_db: Option<f64>,
    r_squared: Option<f64>,
    num_measurements: Option<usize>,
    notes: Option<String>,
}

impl CalibrationMetadataBuilder {
    pub fn antenna_name(mut self, name: impl Into<String>) -> Self {
        self.antenna_name = Some(name.into());
        self
    }

    pub fn calibration_date(mut self, date: impl Into<String>) -> Self {
        self.calibration_date = Some(date.into());
        self
    }

    pub fn format_version(mut self, version: impl Into<String>) -> Self {
        self.format_version = Some(version.into());
        self
    }

    pub fn data_source(mut self, source: impl Into<String>) -> Self {
        self.data_source = Some(source.into());
        self
    }

    pub fn rmse_db(mut self, rmse: f64) -> Self {
        self.rmse_db = Some(rmse);
        self
    }

    pub fn r_squared(mut self, r2: f64) -> Self {
        self.r_squared = Some(r2);
        self
    }

    pub fn num_measurements(mut self, num: usize) -> Self {
        self.num_measurements = Some(num);
        self
    }

    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    pub fn build(self) -> Result<CalibrationMetadata, String> {
        Ok(CalibrationMetadata {
            antenna_name: self.antenna_name.ok_or("antenna_name is required")?,
            calibration_date: self.calibration_date.ok_or("calibration_date is required")?,
            format_version: self.format_version.unwrap_or_else(|| "1.0".to_string()),
            data_source: self.data_source.ok_or("data_source is required")?,
            rmse_db: self.rmse_db.ok_or("rmse_db is required")?,
            r_squared: self.r_squared.ok_or("r_squared is required")?,
            num_measurements: self.num_measurements.ok_or("num_measurements is required")?,
            notes: self.notes,
        })
    }
}

/// Builder for BSplineModel4D.
#[derive(Default)]
pub struct BSplineModel4DBuilder {
    coefficients: Option<Vec<f64>>,
    shape: Option<[usize; 4]>,
    knots_azimuth: Option<Vec<f64>>,
    knots_elevation: Option<Vec<f64>>,
    knots_frequency: Option<Vec<f64>>,
    knots_temperature: Option<Vec<f64>>,
    spline_order: Option<u8>,
}

impl BSplineModel4DBuilder {
    pub fn coefficients(mut self, coeffs: Vec<f64>) -> Self {
        self.coefficients = Some(coeffs);
        self
    }

    pub fn shape(mut self, shape: [usize; 4]) -> Self {
        self.shape = Some(shape);
        self
    }

    pub fn knots_azimuth(mut self, knots: Vec<f64>) -> Self {
        self.knots_azimuth = Some(knots);
        self
    }

    pub fn knots_elevation(mut self, knots: Vec<f64>) -> Self {
        self.knots_elevation = Some(knots);
        self
    }

    pub fn knots_frequency(mut self, knots: Vec<f64>) -> Self {
        self.knots_frequency = Some(knots);
        self
    }

    pub fn knots_temperature(mut self, knots: Vec<f64>) -> Self {
        self.knots_temperature = Some(knots);
        self
    }

    pub fn spline_order(mut self, order: u8) -> Self {
        self.spline_order = Some(order);
        self
    }

    pub fn build(self) -> Result<BSplineModel4D, String> {
        Ok(BSplineModel4D {
            coefficients: self.coefficients.ok_or("coefficients are required")?,
            shape: self.shape.ok_or("shape is required")?,
            knots_azimuth: self.knots_azimuth.ok_or("knots_azimuth is required")?,
            knots_elevation: self.knots_elevation.ok_or("knots_elevation is required")?,
            knots_frequency: self.knots_frequency.ok_or("knots_frequency is required")?,
            knots_temperature: self.knots_temperature.ok_or("knots_temperature is required")?,
            spline_order: self.spline_order.unwrap_or(3), // Default to cubic
        })
    }
}

/// Builder for ValidityRanges.
#[derive(Default)]
pub struct ValidityRangesBuilder {
    azimuth_min_max: Option<(f64, f64)>,
    elevation_min_max: Option<(f64, f64)>,
    frequency_min_max: Option<(f64, f64)>,
    temperature_const: Option<f64>,
}

impl ValidityRangesBuilder {
    pub fn azimuth_range(mut self, min: f64, max: f64) -> Self {
        self.azimuth_min_max = Some((min, max));
        self
    }

    pub fn elevation_range(mut self, min: f64, max: f64) -> Self {
        self.elevation_min_max = Some((min, max));
        self
    }

    pub fn frequency_range(mut self, min: f64, max: f64) -> Self {
        self.frequency_min_max = Some((min, max));
        self
    }

    pub fn temperature(mut self, temp: f64) -> Self {
        self.temperature_const = Some(temp);
        self
    }

    pub fn build(self) -> Result<ValidityRanges, String> {
        Ok(ValidityRanges {
            azimuth_min_max: self.azimuth_min_max.ok_or("azimuth_min_max is required")?,
            elevation_min_max: self.elevation_min_max.ok_or("elevation_min_max is required")?,
            frequency_min_max: self.frequency_min_max.ok_or("frequency_min_max is required")?,
            temperature_const: self.temperature_const.ok_or("temperature_const is required")?,
        })
    }
}

// Helper functions

/// Check if a vector is non-decreasing.
fn is_non_decreasing(v: &[f64]) -> bool {
    v.windows(2).all(|w| w[0] <= w[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_non_decreasing() {
        assert!(is_non_decreasing(&[1.0, 2.0, 3.0, 4.0]));
        assert!(is_non_decreasing(&[1.0, 1.0, 2.0, 2.0]));
        assert!(!is_non_decreasing(&[1.0, 3.0, 2.0, 4.0]));
        assert!(is_non_decreasing(&[]));
        assert!(is_non_decreasing(&[1.0]));
    }

    #[test]
    fn test_validity_ranges_builder() {
        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        assert_eq!(ranges.azimuth_min_max, (0.0, 360.0));
        assert_eq!(ranges.elevation_min_max, (0.0, 90.0));
        assert_eq!(ranges.frequency_min_max, (8000.0, 8500.0));
        assert_eq!(ranges.temperature_const, 290.0);
    }

    #[test]
    fn test_validity_ranges_validate() {
        let valid_ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };
        assert!(valid_ranges.validate().is_ok());

        // Invalid: min > max
        let invalid_ranges = ValidityRanges {
            azimuth_min_max: (360.0, 0.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };
        assert!(invalid_ranges.validate().is_err());

        // Invalid: elevation out of range
        let invalid_ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (-10.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };
        assert!(invalid_ranges.validate().is_err());

        // Invalid: negative temperature
        let invalid_ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: -10.0,
        };
        assert!(invalid_ranges.validate().is_err());
    }

    #[test]
    fn test_validity_ranges_contains() {
        let ranges = ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (10.0, 80.0),
            frequency_min_max: (8000.0, 8500.0),
            temperature_const: 290.0,
        };

        assert!(ranges.contains(45.0, 30.0, 8200.0));
        assert!(!ranges.contains(45.0, 5.0, 8200.0)); // elevation too low
        assert!(!ranges.contains(45.0, 30.0, 7000.0)); // frequency too low
    }

    #[test]
    fn test_calibration_metadata_builder() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .notes("Test calibration")
            .build()
            .unwrap();

        assert_eq!(metadata.antenna_name, "Test Antenna");
        assert_eq!(metadata.rmse_db, 0.5);
        assert_eq!(metadata.r_squared, 0.98);
        assert_eq!(metadata.num_measurements, 1000);
        assert_eq!(metadata.notes, Some("Test calibration".to_string()));
        assert_eq!(metadata.format_version, "1.0");
    }

    #[test]
    fn test_bspline_model_builder() {
        let model = BSplineModel4D::builder()
            .coefficients(vec![1.0; 24])
            .shape([2, 3, 2, 2])
            .knots_azimuth(vec![0.0, 0.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 1.0, 1.0])
            .spline_order(3)
            .build()
            .unwrap();

        assert_eq!(model.coefficients.len(), 24);
        assert_eq!(model.shape, [2, 3, 2, 2]);
        assert_eq!(model.spline_order, 3);
        assert_eq!(model.num_coefficients(), 24);
    }

    #[test]
    fn test_bspline_model_validate() {
        // Valid model
        let valid_model = BSplineModel4D {
            coefficients: vec![1.0; 24],
            shape: [2, 3, 2, 2],
            knots_azimuth: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            spline_order: 3,
        };
        assert!(valid_model.validate().is_ok());

        // Invalid: coefficient size doesn't match shape
        let invalid_model = BSplineModel4D {
            coefficients: vec![1.0; 20], // Should be 24
            shape: [2, 3, 2, 2],
            knots_azimuth: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            spline_order: 3,
        };
        assert!(invalid_model.validate().is_err());

        // Invalid: knot vector too short
        let invalid_model = BSplineModel4D {
            coefficients: vec![1.0; 24],
            shape: [2, 3, 2, 2],
            knots_azimuth: vec![0.0, 1.0], // Too short
            knots_elevation: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            spline_order: 3,
        };
        assert!(invalid_model.validate().is_err());

        // Invalid: knot vector not non-decreasing
        let invalid_model = BSplineModel4D {
            coefficients: vec![1.0; 24],
            shape: [2, 3, 2, 2],
            knots_azimuth: vec![0.0, 1.0, 0.5, 1.0, 1.0, 1.0], // Not non-decreasing
            knots_elevation: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            spline_order: 3,
        };
        assert!(invalid_model.validate().is_err());

        // Invalid: spline order out of range
        let invalid_model = BSplineModel4D {
            coefficients: vec![1.0; 24],
            shape: [2, 3, 2, 2],
            knots_azimuth: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            knots_temperature: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            spline_order: 0,
        };
        assert!(invalid_model.validate().is_err());
    }

    #[test]
    fn test_antenna_calibration_builder() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();

        let model = BSplineModel4D::builder()
            .coefficients(vec![1.0; 24])
            .shape([2, 3, 2, 2])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .metadata(metadata)
            .model(model)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        assert_eq!(calibration.antenna_id, "test_antenna");
        assert!(calibration.validate().is_ok());
    }

    #[test]
    fn test_antenna_calibration_validate() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();

        let model = BSplineModel4D::builder()
            .coefficients(vec![1.0; 24])
            .shape([2, 3, 2, 2])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        // Invalid: empty antenna ID
        let invalid_calibration = AntennaCalibration {
            antenna_id: "".to_string(),
            metadata: metadata.clone(),
            model: model.clone(),
            validity_ranges: ranges.clone(),
        };
        assert!(invalid_calibration.validate().is_err());

        // Valid calibration
        let valid_calibration = AntennaCalibration {
            antenna_id: "test".to_string(),
            metadata,
            model,
            validity_ranges: ranges,
        };
        assert!(valid_calibration.validate().is_ok());
    }

    #[test]
    fn test_serialization_round_trip() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let model = BSplineModel4D::builder()
            .coefficients(vec![1.0, 2.0, 3.0, 4.0])
            .shape([2, 2, 1, 1])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let original = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .metadata(metadata)
            .model(model)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        // Test bincode serialization (bincode 2.x API)
        let config = bincode::config::standard();
        let encoded = bincode::encode_to_vec(&original, config).unwrap();
        let (decoded, _): (AntennaCalibration, usize) =
            bincode::decode_from_slice(&encoded, config).unwrap();

        assert_eq!(original, decoded);
        assert_eq!(original.antenna_id, decoded.antenna_id);
        assert_eq!(original.model.coefficients, decoded.model.coefficients);
        assert_eq!(original.model.shape, decoded.model.shape);
    }

    #[test]
    fn test_serialization_round_trip_json() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let model = BSplineModel4D::builder()
            .coefficients(vec![1.0, 2.0, 3.0, 4.0])
            .shape([2, 2, 1, 1])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let original = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .metadata(metadata)
            .model(model)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        // Test JSON serialization
        let json = serde_json::to_string(&original).unwrap();
        let decoded: AntennaCalibration = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
    }
}
