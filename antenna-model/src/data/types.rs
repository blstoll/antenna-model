//! Core data types for antenna calibration models.
//!
//! This module defines the fundamental data structures used throughout the antenna
//! model service, including calibration data, B-spline models, and metadata.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Complete calibration data for a single antenna-feed combination (v2.0 physics-based).
///
/// Contains all information needed to evaluate antenna G/T (Gain-to-Temperature)
/// using a physics-based model with optional correction surfaces.
///
/// # v2.0 Hybrid Model
///
/// The v2.0 model combines:
/// 1. **Physical optics model** (`physical_config`) - Primary model based on reflector
///    geometry, feed parameters, and mesh characteristics
/// 2. **Correction surface** (`correction_surface`) - Optional B-spline model that
///    corrects for residual errors (measured - physics model)
///
/// At runtime: `G/T_final = PhysicsModel(physical_config) + CorrectionSurface(freq, cone, clock)`
///
/// # Multi-Feed Support
///
/// Each calibration artifact represents one antenna-feed combination. For antennas with
/// multiple feeds (e.g., S-band, X-band, Ka-band), separate calibration files are created
/// with different `feed_id` values. The repository aggregates these using composite
/// `(antenna_id, feed_id)` identifiers.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct AntennaCalibration {
    /// Unique identifier for this antenna
    pub antenna_id: String,

    /// Unique identifier for this feed (e.g., "x_band", "s_band", "primary")
    /// Allows multiple feeds per antenna with different calibrations
    pub feed_id: String,

    /// Metadata about the calibration process
    pub metadata: CalibrationMetadata,

    /// Physical antenna configuration (v2.0 - primary model)
    pub physical_config: PhysicalAntennaConfig,

    /// B-spline correction surface (v2.0 - optional, for residual corrections)
    /// This is fitted to (measured - physics_model) residuals during calibration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_surface: Option<BSplineModel4D>,

    /// Valid ranges for query parameters
    pub validity_ranges: ValidityRanges,

    // ========== v2.0 Partial Calibration Support ==========
    /// Calibration status indicating level of calibration data available (v2.0).
    /// Optional for backward compatibility with existing .bin files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatus>,

    /// Calibration coverage metadata for partially calibrated antennas (v2.0).
    /// Only present for PartiallyCalibrated status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_coverage: Option<CalibrationCoverage>,
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

        // Validate feed ID is not empty
        if self.feed_id.is_empty() {
            return Err(ValidationError::EmptyField("feed_id".to_string()));
        }

        // Validate physical configuration
        self.physical_config.validate()?;

        // Validate correction surface if present
        if let Some(ref correction) = self.correction_surface {
            correction.validate()?;
        }

        // Validate validity ranges
        self.validity_ranges.validate()?;

        // Validate calibration coverage if present
        if let Some(ref coverage) = self.calibration_coverage {
            coverage.validate()?;
        }

        Ok(())
    }
}

/// Metadata describing the calibration process and source data (v2.0).
///
/// # v2.0 Quality Metrics
///
/// Tracks quality metrics for both the physics-only model and the combined
/// physics + correction surface model.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct CalibrationMetadata {
    /// Human-readable antenna name
    pub antenna_name: String,

    /// ISO 8601 timestamp of calibration
    pub calibration_date: String,

    /// Version of calibration format (v2.0 uses "2.0")
    pub format_version: String,

    /// Source of measurement data (e.g., S3 path, file name)
    pub data_source: String,

    /// Root mean squared error of combined model (physics + correction) in dB
    pub rmse_db: f64,

    /// R² correlation coefficient of combined model
    pub r_squared: f64,

    /// Number of measurement points used in calibration
    pub num_measurements: usize,

    /// Optional notes about the calibration
    pub notes: Option<String>,

    // ========== v2.0-specific fields ==========
    /// RMSE of physics-only model (before correction) in dB (v2.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics_only_rmse_db: Option<f64>,

    /// RMSE improvement from adding correction surface in dB (v2.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_improvement_db: Option<f64>,

    /// Whether physical parameter tuning was performed (v2.0)
    #[serde(default)]
    pub parameters_tuned: bool,

    /// Reference to antenna class for shared parameters (v2.0)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub antenna_class: Option<String>,

    // ========== v2.0 Partial Calibration Support ==========
    /// Source of physical parameters (v2.0 - partial calibration support)
    /// Optional for backward compatibility with existing .bin files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters_source: Option<ParameterSource>,

    /// Measurement density indicator (v2.0 - partial calibration support)
    /// Optional for backward compatibility with existing .bin files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_density: Option<MeasurementDensity>,

    // ========== Physics-model versioning (roadmap P1b) ==========
    /// Version of the physics model this calibration was fitted against
    /// (see `crate::model::PHYSICS_MODEL_VERSION`). 0 = unknown (artifact predates
    /// the version stamp). NOTE: adding this field changed the bincode layout;
    /// pre-P1b `.bin` artifacts no longer decode (none exist — sanctioned by P1b).
    #[serde(default)]
    pub physics_model_version: u32,
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

// ============================================================================
// Physics-based Antenna Model Structures (v2.0)
// ============================================================================

/// Physical reflector geometry parameters.
///
/// Describes the parabolic dish reflector geometry used in the physical optics model.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct ReflectorGeometry {
    /// Dish diameter in meters
    pub diameter_m: f64,

    /// Focal length in meters
    pub focal_length_m: f64,

    /// f/D ratio (focal length / diameter), typically 0.3 - 0.5
    pub f_over_d_ratio: f64,

    /// Surface RMS error in millimeters (for Ruze equation)
    pub surface_rms_mm: f64,
}

impl ReflectorGeometry {
    /// Creates a new builder for constructing ReflectorGeometry.
    pub fn builder() -> ReflectorGeometryBuilder {
        ReflectorGeometryBuilder::default()
    }

    /// Validates that the geometry parameters are physically reasonable.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.diameter_m <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "diameter_m".to_string(),
                value: self.diameter_m,
                reason: "must be positive".to_string(),
            });
        }

        if self.focal_length_m <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "focal_length_m".to_string(),
                value: self.focal_length_m,
                reason: "must be positive".to_string(),
            });
        }

        if self.f_over_d_ratio <= 0.0 || self.f_over_d_ratio > 2.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "f_over_d_ratio".to_string(),
                value: self.f_over_d_ratio,
                reason: "must be between 0 and 2".to_string(),
            });
        }

        // f_over_d_ratio is redundant with focal_length_m / diameter_m; reject
        // artifacts where the stored ratio contradicts the geometry (>1%).
        let implied_f_over_d = self.focal_length_m / self.diameter_m;
        if (self.f_over_d_ratio - implied_f_over_d).abs() > 0.01 * implied_f_over_d {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "f_over_d_ratio".to_string(),
                value: self.f_over_d_ratio,
                reason: format!(
                    "inconsistent with focal_length_m/diameter_m = {implied_f_over_d:.4}"
                ),
            });
        }

        if self.surface_rms_mm < 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "surface_rms_mm".to_string(),
                value: self.surface_rms_mm,
                reason: "must be non-negative".to_string(),
            });
        }

        Ok(())
    }
}

/// Feed antenna parameters.
///
/// Describes the feed horn characteristics and position for physical optics computation.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct FeedParameters {
    /// Feed position in Cartesian coordinates (x, y, z) in meters
    pub position: (f64, f64, f64),

    /// q-factor for cos^q illumination pattern (typically 6-12)
    pub q_factor: f64,

    /// Phase center offset from feed aperture in meters
    pub phase_center_offset_m: f64,
}

impl FeedParameters {
    /// Creates a new builder for constructing FeedParameters.
    pub fn builder() -> FeedParametersBuilder {
        FeedParametersBuilder::default()
    }

    /// Validates that the feed parameters are physically reasonable.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.q_factor < 0.0 || self.q_factor > 20.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "q_factor".to_string(),
                value: self.q_factor,
                reason: "must be between 0 and 20".to_string(),
            });
        }

        Ok(())
    }
}

/// Mesh reflector parameters.
///
/// Describes wire mesh characteristics for mesh reflector antennas.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct MeshParameters {
    /// Mesh spacing (hole size) in millimeters
    pub mesh_spacing_mm: f64,

    /// Wire diameter in millimeters
    pub wire_diameter_mm: f64,
}

impl MeshParameters {
    /// Creates a new builder for constructing MeshParameters.
    pub fn builder() -> MeshParametersBuilder {
        MeshParametersBuilder::default()
    }

    /// Validates that the mesh parameters are physically reasonable.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.mesh_spacing_mm <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "mesh_spacing_mm".to_string(),
                value: self.mesh_spacing_mm,
                reason: "must be positive".to_string(),
            });
        }

        if self.wire_diameter_mm <= 0.0 {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "wire_diameter_mm".to_string(),
                value: self.wire_diameter_mm,
                reason: "must be positive".to_string(),
            });
        }

        if self.wire_diameter_mm >= self.mesh_spacing_mm {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "wire_diameter_mm".to_string(),
                value: self.wire_diameter_mm,
                reason: "must be less than mesh_spacing_mm".to_string(),
            });
        }

        Ok(())
    }
}

/// Complete physical antenna configuration.
///
/// Combines all physical parameters needed for physics-based antenna modeling.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct PhysicalAntennaConfig {
    /// Reflector geometry
    pub reflector: ReflectorGeometry,

    /// Feed parameters
    pub feed: FeedParameters,

    /// Mesh parameters (None for solid reflectors)
    pub mesh: Option<MeshParameters>,
}

impl PhysicalAntennaConfig {
    /// Creates a new builder for constructing PhysicalAntennaConfig.
    pub fn builder() -> PhysicalAntennaConfigBuilder {
        PhysicalAntennaConfigBuilder::default()
    }

    /// Validates all physical parameters.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.reflector.validate()?;
        self.feed.validate()?;
        if let Some(ref mesh) = self.mesh {
            mesh.validate()?;
        }
        Ok(())
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
    pub fn contains(&self, azimuth: f64, elevation: f64, frequency: f64) -> bool {
        azimuth >= self.azimuth_min_max.0
            && azimuth <= self.azimuth_min_max.1
            && elevation >= self.elevation_min_max.0
            && elevation <= self.elevation_min_max.1
            && frequency >= self.frequency_min_max.0
            && frequency <= self.frequency_min_max.1
    }
}

// ============================================================================
// Calibration Status and Coverage Types (v2.0 - Partial Calibration Support)
// ============================================================================

/// Calibration status indicating the level of calibration data available.
///
/// This enum supports three calibration levels:
/// 1. **Fully Calibrated** - Dense measurement grid with full correction surface
/// 2. **Partially Calibrated** - Limited measurements (boresight or sparse grid)
/// 3. **Uncalibrated** - Design specifications only, no measurements
///
/// Each status includes accuracy estimates to help users understand prediction quality.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum CalibrationStatus {
    /// Fully calibrated with dense measurement grid across azimuth, elevation, and frequency.
    /// Provides highest accuracy (typically ±1 dB in main lobe and first sidelobe).
    FullyCalibrated {
        /// Expected accuracy in dB (typically ±1.0)
        accuracy_estimate_db: f64,
    },

    /// Partially calibrated with limited measurement coverage.
    /// May have boresight-only measurements or sparse spatial grid.
    /// Accuracy varies by coverage: ±1-1.5 dB in-coverage, ±2-3 dB out-of-coverage.
    PartiallyCalibrated {
        /// Expected accuracy in dB (varies by coverage)
        accuracy_estimate_db: f64,
        /// Measurement coverage details
        coverage: CalibrationCoverage,
    },

    /// Uncalibrated - uses design specifications only (no measurements).
    /// Absolute gain accuracy is lower (±3-5 dB), but loss accuracy is better
    /// (±2 dB) due to systematic error cancellation in subtraction.
    Uncalibrated {
        /// Expected absolute gain accuracy in dB (typically ±3.0)
        accuracy_estimate_db: f64,
        /// Expected loss (relative gain) accuracy in dB (typically ±2.0)
        /// Better than absolute due to error cancellation
        loss_accuracy_estimate_db: f64,
    },
}

impl CalibrationStatus {
    /// Returns the accuracy estimate in dB for this calibration status.
    pub fn accuracy_estimate_db(&self) -> f64 {
        match self {
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db,
            } => *accuracy_estimate_db,
            CalibrationStatus::PartiallyCalibrated {
                accuracy_estimate_db,
                ..
            } => *accuracy_estimate_db,
            CalibrationStatus::Uncalibrated {
                accuracy_estimate_db,
                ..
            } => *accuracy_estimate_db,
        }
    }

    /// Returns true if this calibration has a correction surface.
    pub fn has_correction_surface(&self) -> bool {
        match self {
            CalibrationStatus::FullyCalibrated { .. } => true,
            CalibrationStatus::PartiallyCalibrated { coverage, .. } => {
                coverage.has_correction_surface
            }
            CalibrationStatus::Uncalibrated { .. } => false,
        }
    }

    /// Returns a human-readable status string.
    pub fn status_string(&self) -> &str {
        match self {
            CalibrationStatus::FullyCalibrated { .. } => "fully_calibrated",
            CalibrationStatus::PartiallyCalibrated { .. } => "partially_calibrated",
            CalibrationStatus::Uncalibrated { .. } => "uncalibrated",
        }
    }
}

/// Measurement coverage for partially calibrated antennas.
///
/// Describes the spatial, frequency, and measurement density of partial calibration data.
/// Used to determine whether queries are in-coverage (use correction surface) or
/// out-of-coverage (physics model only).
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct CalibrationCoverage {
    /// Azimuth coverage range in degrees (min, max)
    /// For boresight-only: (0.0, 0.0)
    pub azimuth_range: (f64, f64),

    /// Elevation coverage range in degrees (min, max)
    /// For boresight-only: (0.0, 0.0)
    pub elevation_range: (f64, f64),

    /// Frequency coverage range in MHz (min, max)
    pub frequency_range: (f64, f64),

    /// Total number of measurement points
    pub num_measurements: usize,

    /// Whether a correction surface was fitted to measurements
    /// If false, only physical parameters were tuned
    pub has_correction_surface: bool,
}

impl CalibrationCoverage {
    /// Creates a new builder for constructing CalibrationCoverage.
    pub fn builder() -> CalibrationCoverageBuilder {
        CalibrationCoverageBuilder::default()
    }

    /// Checks if this is boresight-only coverage (single spatial point).
    pub fn is_boresight_only(&self) -> bool {
        self.azimuth_range.0 == self.azimuth_range.1
            && self.elevation_range.0 == self.elevation_range.1
    }

    /// Checks if a query point is within the calibrated coverage.
    pub fn contains(&self, azimuth: f64, elevation: f64, frequency: f64) -> bool {
        azimuth >= self.azimuth_range.0
            && azimuth <= self.azimuth_range.1
            && elevation >= self.elevation_range.0
            && elevation <= self.elevation_range.1
            && frequency >= self.frequency_range.0
            && frequency <= self.frequency_range.1
    }

    /// Validates that coverage parameters are well-formed.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.azimuth_range.0 > self.azimuth_range.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "azimuth".to_string(),
                min: self.azimuth_range.0,
                max: self.azimuth_range.1,
            });
        }

        if self.elevation_range.0 > self.elevation_range.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "elevation".to_string(),
                min: self.elevation_range.0,
                max: self.elevation_range.1,
            });
        }

        if self.frequency_range.0 > self.frequency_range.1 {
            return Err(ValidationError::InvalidRange {
                dimension: "frequency".to_string(),
                min: self.frequency_range.0,
                max: self.frequency_range.1,
            });
        }

        Ok(())
    }
}

/// Source of physical antenna parameters.
///
/// Indicates how the physical parameters (surface RMS, q-factor, mesh properties)
/// were determined. This helps users understand parameter confidence.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum ParameterSource {
    /// Parameters from design specifications (vendor data, CAD models)
    /// Typical accuracy: ±20-30% on individual parameters
    DesignSpecifications,

    /// Parameters tuned from boresight measurements only
    /// Better accuracy at boresight: ±5-10% on parameters
    BoresightTuning {
        /// Number of boresight measurements used for tuning
        num_measurements: usize,
    },

    /// Parameters tuned from partial measurement grid
    /// Good accuracy in-coverage: ±5-10% on parameters
    PartialGridTuning {
        /// Number of grid measurements used for tuning
        num_measurements: usize,
    },

    /// Parameters tuned from full measurement grid
    /// Best accuracy everywhere: ±3-5% on parameters
    FullGridTuning {
        /// Number of grid measurements used for tuning
        num_measurements: usize,
    },
}

impl ParameterSource {
    /// Returns the number of measurements used (if any).
    pub fn num_measurements(&self) -> Option<usize> {
        match self {
            ParameterSource::DesignSpecifications => None,
            ParameterSource::BoresightTuning { num_measurements }
            | ParameterSource::PartialGridTuning { num_measurements }
            | ParameterSource::FullGridTuning { num_measurements } => Some(*num_measurements),
        }
    }

    /// Returns true if parameters were tuned from measurements.
    pub fn is_tuned(&self) -> bool {
        !matches!(self, ParameterSource::DesignSpecifications)
    }
}

/// Measurement density indicator.
///
/// Describes the spatial density of measurement data relative to the antenna beamwidth.
/// Higher density provides better accuracy and supports finer correction surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum MeasurementDensity {
    /// No measurements (uncalibrated - design specs only)
    None,

    /// Boresight-only measurements (single spatial point, multiple frequencies)
    BoresightOnly,

    /// Sparse spatial sampling (2-5 measurement points per beamwidth)
    /// Sufficient for parameter tuning, limited correction surface
    Sparse {
        /// Average measurement points per beamwidth
        points_per_beam: f64,
    },

    /// Dense spatial sampling (>10 measurement points per beamwidth)
    /// Supports high-quality correction surface
    Dense {
        /// Average measurement points per beamwidth
        points_per_beam: f64,
    },
}

impl MeasurementDensity {
    /// Returns the measurement points per beamwidth (if applicable).
    pub fn points_per_beam(&self) -> Option<f64> {
        match self {
            MeasurementDensity::Sparse { points_per_beam }
            | MeasurementDensity::Dense { points_per_beam } => Some(*points_per_beam),
            _ => None,
        }
    }

    /// Returns true if measurements are available.
    pub fn has_measurements(&self) -> bool {
        !matches!(self, MeasurementDensity::None)
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
    InvalidRange {
        dimension: String,
        min: f64,
        max: f64,
    },

    /// Invalid temperature value
    InvalidTemperature(f64),

    /// Invalid physical parameter (v2.0)
    InvalidPhysicalParameter {
        parameter: String,
        value: f64,
        reason: String,
    },
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
        }
    }
}

impl std::error::Error for ValidationError {}

// Builder patterns for ergonomic construction

/// Builder for AntennaCalibration (v2.0).
#[derive(Default)]
pub struct AntennaCalibrationBuilder {
    antenna_id: Option<String>,
    feed_id: Option<String>,
    metadata: Option<CalibrationMetadata>,
    physical_config: Option<PhysicalAntennaConfig>,
    correction_surface: Option<BSplineModel4D>,
    validity_ranges: Option<ValidityRanges>,
    calibration_status: Option<CalibrationStatus>,
    calibration_coverage: Option<CalibrationCoverage>,
}

impl AntennaCalibrationBuilder {
    pub fn antenna_id(mut self, id: impl Into<String>) -> Self {
        self.antenna_id = Some(id.into());
        self
    }

    pub fn feed_id(mut self, id: impl Into<String>) -> Self {
        self.feed_id = Some(id.into());
        self
    }

    pub fn metadata(mut self, metadata: CalibrationMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn physical_config(mut self, config: PhysicalAntennaConfig) -> Self {
        self.physical_config = Some(config);
        self
    }

    pub fn correction_surface(mut self, correction: BSplineModel4D) -> Self {
        self.correction_surface = Some(correction);
        self
    }

    pub fn validity_ranges(mut self, ranges: ValidityRanges) -> Self {
        self.validity_ranges = Some(ranges);
        self
    }

    pub fn calibration_status(mut self, status: CalibrationStatus) -> Self {
        self.calibration_status = Some(status);
        self
    }

    pub fn calibration_coverage(mut self, coverage: CalibrationCoverage) -> Self {
        self.calibration_coverage = Some(coverage);
        self
    }

    pub fn build(self) -> Result<AntennaCalibration, String> {
        Ok(AntennaCalibration {
            antenna_id: self.antenna_id.ok_or("antenna_id is required")?,
            feed_id: self.feed_id.ok_or("feed_id is required")?,
            metadata: self.metadata.ok_or("metadata is required")?,
            physical_config: self.physical_config.ok_or("physical_config is required")?,
            correction_surface: self.correction_surface,
            validity_ranges: self.validity_ranges.ok_or("validity_ranges is required")?,
            calibration_status: self.calibration_status,
            calibration_coverage: self.calibration_coverage,
        })
    }
}

/// Builder for CalibrationMetadata (v2.0).
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
    physics_only_rmse_db: Option<f64>,
    correction_improvement_db: Option<f64>,
    parameters_tuned: bool,
    antenna_class: Option<String>,
    parameters_source: Option<ParameterSource>,
    measurement_density: Option<MeasurementDensity>,
    physics_model_version: Option<u32>,
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

    pub fn physics_only_rmse_db(mut self, rmse: f64) -> Self {
        self.physics_only_rmse_db = Some(rmse);
        self
    }

    pub fn correction_improvement_db(mut self, improvement: f64) -> Self {
        self.correction_improvement_db = Some(improvement);
        self
    }

    pub fn parameters_tuned(mut self, tuned: bool) -> Self {
        self.parameters_tuned = tuned;
        self
    }

    pub fn antenna_class(mut self, class: impl Into<String>) -> Self {
        self.antenna_class = Some(class.into());
        self
    }

    pub fn parameters_source(mut self, source: ParameterSource) -> Self {
        self.parameters_source = Some(source);
        self
    }

    pub fn measurement_density(mut self, density: MeasurementDensity) -> Self {
        self.measurement_density = Some(density);
        self
    }

    pub fn physics_model_version(mut self, version: u32) -> Self {
        self.physics_model_version = Some(version);
        self
    }

    pub fn build(self) -> Result<CalibrationMetadata, String> {
        Ok(CalibrationMetadata {
            antenna_name: self.antenna_name.ok_or("antenna_name is required")?,
            calibration_date: self
                .calibration_date
                .ok_or("calibration_date is required")?,
            format_version: self.format_version.unwrap_or_else(|| "2.0".to_string()),
            data_source: self.data_source.ok_or("data_source is required")?,
            rmse_db: self.rmse_db.ok_or("rmse_db is required")?,
            r_squared: self.r_squared.ok_or("r_squared is required")?,
            num_measurements: self
                .num_measurements
                .ok_or("num_measurements is required")?,
            notes: self.notes,
            physics_only_rmse_db: self.physics_only_rmse_db,
            correction_improvement_db: self.correction_improvement_db,
            parameters_tuned: self.parameters_tuned,
            antenna_class: self.antenna_class,
            parameters_source: self.parameters_source,
            measurement_density: self.measurement_density,
            physics_model_version: self.physics_model_version.unwrap_or(0),
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
            knots_temperature: self
                .knots_temperature
                .ok_or("knots_temperature is required")?,
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
            elevation_min_max: self
                .elevation_min_max
                .ok_or("elevation_min_max is required")?,
            frequency_min_max: self
                .frequency_min_max
                .ok_or("frequency_min_max is required")?,
            temperature_const: self
                .temperature_const
                .ok_or("temperature_const is required")?,
        })
    }
}

// ============================================================================
// Builders for Physics-based Structures (v2.0)
// ============================================================================

/// Builder for ReflectorGeometry.
#[derive(Default)]
pub struct ReflectorGeometryBuilder {
    diameter_m: Option<f64>,
    focal_length_m: Option<f64>,
    f_over_d_ratio: Option<f64>,
    surface_rms_mm: Option<f64>,
}

impl ReflectorGeometryBuilder {
    pub fn diameter_m(mut self, diameter: f64) -> Self {
        self.diameter_m = Some(diameter);
        self
    }

    pub fn focal_length_m(mut self, focal_length: f64) -> Self {
        self.focal_length_m = Some(focal_length);
        self
    }

    pub fn f_over_d_ratio(mut self, ratio: f64) -> Self {
        self.f_over_d_ratio = Some(ratio);
        self
    }

    pub fn surface_rms_mm(mut self, rms: f64) -> Self {
        self.surface_rms_mm = Some(rms);
        self
    }

    pub fn build(self) -> Result<ReflectorGeometry, String> {
        Ok(ReflectorGeometry {
            diameter_m: self.diameter_m.ok_or("diameter_m is required")?,
            focal_length_m: self.focal_length_m.ok_or("focal_length_m is required")?,
            f_over_d_ratio: self.f_over_d_ratio.ok_or("f_over_d_ratio is required")?,
            surface_rms_mm: self.surface_rms_mm.ok_or("surface_rms_mm is required")?,
        })
    }
}

/// Builder for FeedParameters.
#[derive(Default)]
pub struct FeedParametersBuilder {
    position: Option<(f64, f64, f64)>,
    q_factor: Option<f64>,
    phase_center_offset_m: Option<f64>,
}

impl FeedParametersBuilder {
    pub fn position(mut self, x: f64, y: f64, z: f64) -> Self {
        self.position = Some((x, y, z));
        self
    }

    pub fn q_factor(mut self, q: f64) -> Self {
        self.q_factor = Some(q);
        self
    }

    pub fn phase_center_offset_m(mut self, offset: f64) -> Self {
        self.phase_center_offset_m = Some(offset);
        self
    }

    pub fn build(self) -> Result<FeedParameters, String> {
        Ok(FeedParameters {
            position: self.position.ok_or("position is required")?,
            q_factor: self.q_factor.ok_or("q_factor is required")?,
            phase_center_offset_m: self.phase_center_offset_m.unwrap_or(0.0),
        })
    }
}

/// Builder for MeshParameters.
#[derive(Default)]
pub struct MeshParametersBuilder {
    mesh_spacing_mm: Option<f64>,
    wire_diameter_mm: Option<f64>,
}

impl MeshParametersBuilder {
    pub fn mesh_spacing_mm(mut self, spacing: f64) -> Self {
        self.mesh_spacing_mm = Some(spacing);
        self
    }

    pub fn wire_diameter_mm(mut self, diameter: f64) -> Self {
        self.wire_diameter_mm = Some(diameter);
        self
    }

    pub fn build(self) -> Result<MeshParameters, String> {
        Ok(MeshParameters {
            mesh_spacing_mm: self.mesh_spacing_mm.ok_or("mesh_spacing_mm is required")?,
            wire_diameter_mm: self
                .wire_diameter_mm
                .ok_or("wire_diameter_mm is required")?,
        })
    }
}

/// Builder for PhysicalAntennaConfig.
#[derive(Default)]
pub struct PhysicalAntennaConfigBuilder {
    reflector: Option<ReflectorGeometry>,
    feed: Option<FeedParameters>,
    mesh: Option<MeshParameters>,
}

impl PhysicalAntennaConfigBuilder {
    pub fn reflector(mut self, reflector: ReflectorGeometry) -> Self {
        self.reflector = Some(reflector);
        self
    }

    pub fn feed(mut self, feed: FeedParameters) -> Self {
        self.feed = Some(feed);
        self
    }

    pub fn mesh(mut self, mesh: MeshParameters) -> Self {
        self.mesh = Some(mesh);
        self
    }

    pub fn build(self) -> Result<PhysicalAntennaConfig, String> {
        Ok(PhysicalAntennaConfig {
            reflector: self.reflector.ok_or("reflector is required")?,
            feed: self.feed.ok_or("feed is required")?,
            mesh: self.mesh,
        })
    }
}

/// Builder for CalibrationCoverage.
#[derive(Default)]
pub struct CalibrationCoverageBuilder {
    azimuth_range: Option<(f64, f64)>,
    elevation_range: Option<(f64, f64)>,
    frequency_range: Option<(f64, f64)>,
    num_measurements: Option<usize>,
    has_correction_surface: Option<bool>,
}

impl CalibrationCoverageBuilder {
    pub fn azimuth_range(mut self, min: f64, max: f64) -> Self {
        self.azimuth_range = Some((min, max));
        self
    }

    pub fn elevation_range(mut self, min: f64, max: f64) -> Self {
        self.elevation_range = Some((min, max));
        self
    }

    pub fn frequency_range(mut self, min: f64, max: f64) -> Self {
        self.frequency_range = Some((min, max));
        self
    }

    pub fn num_measurements(mut self, num: usize) -> Self {
        self.num_measurements = Some(num);
        self
    }

    pub fn has_correction_surface(mut self, has_surface: bool) -> Self {
        self.has_correction_surface = Some(has_surface);
        self
    }

    pub fn build(self) -> Result<CalibrationCoverage, String> {
        Ok(CalibrationCoverage {
            azimuth_range: self.azimuth_range.ok_or("azimuth_range is required")?,
            elevation_range: self.elevation_range.ok_or("elevation_range is required")?,
            frequency_range: self.frequency_range.ok_or("frequency_range is required")?,
            num_measurements: self
                .num_measurements
                .ok_or("num_measurements is required")?,
            has_correction_surface: self.has_correction_surface.unwrap_or(false),
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
    fn test_reflector_geometry_rejects_inconsistent_f_over_d() {
        let geom = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 5.0,
            f_over_d_ratio: 0.6, // truth is 0.5
            surface_rms_mm: 0.5,
        };
        assert!(geom.validate().is_err());

        let consistent = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 5.0,
            f_over_d_ratio: 0.5,
            surface_rms_mm: 0.5,
        };
        assert!(consistent.validate().is_ok());
    }

    // Helper function to create a test physical config
    fn create_test_physical_config() -> PhysicalAntennaConfig {
        let reflector = ReflectorGeometry::builder()
            .diameter_m(34.0)
            .focal_length_m(13.6)
            .f_over_d_ratio(0.4)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.1)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap()
    }

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
        assert_eq!(metadata.format_version, "2.0");
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

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("primary")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        assert_eq!(calibration.antenna_id, "test_antenna");
        assert_eq!(calibration.feed_id, "primary");
        assert!(calibration.validate().is_ok());
        assert!(calibration.correction_surface.is_none());
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

        let physical_config = create_test_physical_config();

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
            feed_id: "primary".to_string(),
            metadata: metadata.clone(),
            physical_config: physical_config.clone(),
            correction_surface: None,
            validity_ranges: ranges.clone(),
            calibration_status: None,
            calibration_coverage: None,
        };
        assert!(invalid_calibration.validate().is_err());

        // Invalid: empty feed ID
        let invalid_calibration = AntennaCalibration {
            antenna_id: "test".to_string(),
            feed_id: "".to_string(),
            metadata: metadata.clone(),
            physical_config: physical_config.clone(),
            correction_surface: None,
            validity_ranges: ranges.clone(),
            calibration_status: None,
            calibration_coverage: None,
        };
        assert!(invalid_calibration.validate().is_err());

        // Valid calibration
        let valid_calibration = AntennaCalibration {
            antenna_id: "test".to_string(),
            feed_id: "primary".to_string(),
            metadata,
            physical_config,
            correction_surface: None,
            validity_ranges: ranges,
            calibration_status: None,
            calibration_coverage: None,
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

        let physical_config = create_test_physical_config();

        let correction = BSplineModel4D::builder()
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
            .feed_id("x_band")
            .metadata(metadata)
            .physical_config(physical_config)
            .correction_surface(correction)
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
        assert_eq!(original.feed_id, decoded.feed_id);
        assert_eq!(
            original.physical_config.reflector.diameter_m,
            decoded.physical_config.reflector.diameter_m
        );
        assert!(original.correction_surface.is_some());
        assert!(decoded.correction_surface.is_some());
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

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let original = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("primary")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        // Test JSON serialization
        let json = serde_json::to_string(&original).unwrap();
        let decoded: AntennaCalibration = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
        assert_eq!(decoded.feed_id, "primary");
    }

    // ============================================================================
    // Tests for Partial Calibration Support (v2.0)
    // ============================================================================

    #[test]
    fn test_calibration_status_fully_calibrated() {
        let status = CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        };

        assert_eq!(status.accuracy_estimate_db(), 1.0);
        assert!(status.has_correction_surface());
        assert_eq!(status.status_string(), "fully_calibrated");
    }

    #[test]
    fn test_calibration_status_partially_calibrated() {
        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 28,
            has_correction_surface: true,
        };

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        };

        assert_eq!(status.accuracy_estimate_db(), 1.5);
        assert!(status.has_correction_surface());
        assert_eq!(status.status_string(), "partially_calibrated");
    }

    #[test]
    fn test_calibration_status_uncalibrated() {
        let status = CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        };

        assert_eq!(status.accuracy_estimate_db(), 3.0);
        assert!(!status.has_correction_surface());
        assert_eq!(status.status_string(), "uncalibrated");
    }

    #[test]
    fn test_calibration_coverage_builder() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(7100.0, 8500.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        assert_eq!(coverage.azimuth_range, (0.0, 360.0));
        assert_eq!(coverage.elevation_range, (0.0, 90.0));
        assert_eq!(coverage.frequency_range, (7100.0, 8500.0));
        assert_eq!(coverage.num_measurements, 100);
        assert!(coverage.has_correction_surface);
    }

    #[test]
    fn test_calibration_coverage_is_boresight_only() {
        let boresight = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 28,
            has_correction_surface: false,
        };
        assert!(boresight.is_boresight_only());

        let limited = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (30.0, 60.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 324,
            has_correction_surface: true,
        };
        assert!(!limited.is_boresight_only());
    }

    #[test]
    fn test_calibration_coverage_contains() {
        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (30.0, 60.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 324,
            has_correction_surface: true,
        };

        assert!(coverage.contains(45.0, 45.0, 8000.0));
        assert!(!coverage.contains(45.0, 20.0, 8000.0)); // elevation too low
        assert!(!coverage.contains(45.0, 45.0, 9000.0)); // frequency too high
    }

    #[test]
    fn test_calibration_coverage_validate() {
        let valid_coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 90.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 100,
            has_correction_surface: true,
        };
        assert!(valid_coverage.validate().is_ok());

        let invalid_coverage = CalibrationCoverage {
            azimuth_range: (360.0, 0.0), // Invalid: min > max
            elevation_range: (0.0, 90.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 100,
            has_correction_surface: true,
        };
        assert!(invalid_coverage.validate().is_err());
    }

    #[test]
    fn test_parameter_source_design_specifications() {
        let source = ParameterSource::DesignSpecifications;
        assert_eq!(source.num_measurements(), None);
        assert!(!source.is_tuned());
    }

    #[test]
    fn test_parameter_source_boresight_tuning() {
        let source = ParameterSource::BoresightTuning {
            num_measurements: 28,
        };
        assert_eq!(source.num_measurements(), Some(28));
        assert!(source.is_tuned());
    }

    #[test]
    fn test_parameter_source_partial_grid_tuning() {
        let source = ParameterSource::PartialGridTuning {
            num_measurements: 324,
        };
        assert_eq!(source.num_measurements(), Some(324));
        assert!(source.is_tuned());
    }

    #[test]
    fn test_parameter_source_full_grid_tuning() {
        let source = ParameterSource::FullGridTuning {
            num_measurements: 3312,
        };
        assert_eq!(source.num_measurements(), Some(3312));
        assert!(source.is_tuned());
    }

    #[test]
    fn test_measurement_density_none() {
        let density = MeasurementDensity::None;
        assert_eq!(density.points_per_beam(), None);
        assert!(!density.has_measurements());
    }

    #[test]
    fn test_measurement_density_boresight_only() {
        let density = MeasurementDensity::BoresightOnly;
        assert_eq!(density.points_per_beam(), None);
        assert!(density.has_measurements());
    }

    #[test]
    fn test_measurement_density_sparse() {
        let density = MeasurementDensity::Sparse {
            points_per_beam: 3.5,
        };
        assert_eq!(density.points_per_beam(), Some(3.5));
        assert!(density.has_measurements());
    }

    #[test]
    fn test_measurement_density_dense() {
        let density = MeasurementDensity::Dense {
            points_per_beam: 15.0,
        };
        assert_eq!(density.points_per_beam(), Some(15.0));
        assert!(density.has_measurements());
    }

    #[test]
    fn test_antenna_calibration_with_partial_calibration_fields() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .parameters_source(ParameterSource::BoresightTuning {
                num_measurements: 28,
            })
            .measurement_density(MeasurementDensity::BoresightOnly)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 0.0)
            .elevation_range(0.0, 0.0)
            .frequency_range(7100.0, 8500.0)
            .num_measurements(28)
            .has_correction_surface(false)
            .build()
            .unwrap();

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        };

        let calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("x_band")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .calibration_status(status)
            .calibration_coverage(coverage)
            .build()
            .unwrap();

        assert!(calibration.validate().is_ok());
        assert!(calibration.calibration_status.is_some());
        assert!(calibration.calibration_coverage.is_some());
    }

    #[test]
    fn test_serialization_backward_compatibility() {
        // Test that old calibrations (without new fields) can still be deserialized
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        // Build calibration without new optional fields
        let calibration = AntennaCalibration {
            antenna_id: "test_antenna".to_string(),
            feed_id: "primary".to_string(),
            metadata,
            physical_config,
            correction_surface: None,
            validity_ranges: ranges,
            calibration_status: None,   // Old format - no status
            calibration_coverage: None, // Old format - no coverage
        };

        // Should still be valid
        assert!(calibration.validate().is_ok());

        // Should serialize/deserialize correctly
        let config = bincode::config::standard();
        let encoded = bincode::encode_to_vec(&calibration, config).unwrap();
        let (decoded, _): (AntennaCalibration, usize) =
            bincode::decode_from_slice(&encoded, config).unwrap();

        assert_eq!(calibration, decoded);
        assert!(decoded.calibration_status.is_none());
        assert!(decoded.calibration_coverage.is_none());
    }

    #[test]
    fn test_partial_calibration_serialization_round_trip() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 0.0)
            .elevation_range(0.0, 0.0)
            .frequency_range(7100.0, 8500.0)
            .num_measurements(28)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        };

        // Test bincode serialization
        let config = bincode::config::standard();
        let encoded = bincode::encode_to_vec(&status, config).unwrap();
        let (decoded, _): (CalibrationStatus, usize) =
            bincode::decode_from_slice(&encoded, config).unwrap();

        assert_eq!(status, decoded);

        // Test JSON serialization
        let json = serde_json::to_string(&status).unwrap();
        let decoded_json: CalibrationStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded_json);
    }

    // ============================================================================
    // BSpline validation tests (ANTC-hardening, Task 5)
    // ============================================================================

    /// Helper: build a minimal valid order-3 BSplineModel4D (shape [2,2,2,1]).
    fn make_valid_bspline() -> BSplineModel4D {
        BSplineModel4D {
            coefficients: vec![0.0; 8],
            shape: [2, 2, 2, 1],
            // knot vector length must be >= shape[i] + spline_order
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![200.0, 200.0, 200.0, 350.0, 350.0, 350.0],
            spline_order: 3,
        }
    }

    #[test]
    fn test_bspline_validate_valid_model() {
        let model = make_valid_bspline();
        assert!(
            model.validate().is_ok(),
            "Expected valid model to pass, got: {:?}",
            model.validate().err()
        );
    }

    #[test]
    fn test_bspline_validate_rejects_short_knots() {
        // knots_azimuth has only 2 elements but order=3 requires shape[0]+order = 2+3 = 5 elements
        let model = BSplineModel4D {
            coefficients: vec![0.0; 8],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 360.0], // too short for order 3 (needs >= 5)
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![200.0, 200.0, 200.0, 350.0, 350.0, 350.0],
            spline_order: 3,
        };
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for too-short knot vector"
        );
        match model.validate().unwrap_err() {
            ValidationError::InvalidKnotVector { dimension, .. } => {
                assert_eq!(dimension, "azimuth");
            }
            other => panic!("Expected InvalidKnotVector, got {:?}", other),
        }
    }

    #[test]
    fn test_bspline_validate_rejects_non_monotonic_knots() {
        let mut model = make_valid_bspline();
        // Break monotonicity in elevation knots
        model.knots_elevation = vec![0.0, 0.0, 90.0, 50.0, 90.0, 90.0]; // 90 then 50 is decreasing
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for non-monotonic knot vector"
        );
        match model.validate().unwrap_err() {
            ValidationError::InvalidKnotVector { dimension, .. } => {
                assert_eq!(dimension, "elevation");
            }
            other => panic!("Expected InvalidKnotVector, got {:?}", other),
        }
    }

    #[test]
    fn test_bspline_validate_rejects_coefficient_shape_mismatch() {
        let mut model = make_valid_bspline();
        // shape says 2*2*2*1 = 8 coefficients, but we give 7
        model.coefficients = vec![0.0; 7];
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for coefficient/shape mismatch"
        );
        match model.validate().unwrap_err() {
            ValidationError::InconsistentShape { expected, actual } => {
                assert_eq!(expected, 8);
                assert_eq!(actual, 7);
            }
            other => panic!("Expected InconsistentShape, got {:?}", other),
        }
    }

    #[test]
    fn test_bspline_validate_rejects_zero_spline_order() {
        let mut model = make_valid_bspline();
        model.spline_order = 0;
        assert!(
            model.validate().is_err(),
            "Expected validation to fail for spline_order=0"
        );
    }
}
