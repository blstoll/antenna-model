//! API request and response schemas
//!
//! This module defines the data structures for API requests and responses,
//! all using serde for JSON serialization/deserialization.
//!
//! # 3D Coordinate System Support
//!
//! All 3D positions support automatic coordinate system detection:
//! - **ECEF** (Earth-Centered Earth-Fixed): Detected when |x|, |y|, or |z| > 1000 km
//! - **Geodetic**: Otherwise (x=longitude degrees, y=latitude degrees, z=altitude meters)
//!
//! # Multi-Feed Support
//!
//! Antennas can have multiple feeds, identified by composite `(antenna_id, feed_id)` pairs.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::data::types::{CalibrationCoverage, CalibrationStatus};

/// Coordinate system type for 3D positions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordinateSystem {
    /// Earth-Centered Earth-Fixed coordinates (x, y, z in meters)
    #[serde(rename = "ecef")]
    ECEF,
    /// Geodetic coordinates (longitude degrees, latitude degrees, altitude meters)
    Geodetic,
}

/// Custom serialization for f64 that handles NaN as null in JSON
mod nan_as_null {
    use super::*;

    pub fn serialize<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.is_nan() {
            serializer.serialize_none()
        } else {
            serializer.serialize_f64(*value)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<f64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<f64> = Option::deserialize(deserializer)?;
        Ok(opt.unwrap_or(f64::NAN))
    }
}

// ============================================================================
// Core Types
// ============================================================================

/// 3D position with automatic coordinate system detection.
///
/// Supports two coordinate systems:
/// - **ECEF** (Earth-Centered Earth-Fixed): When |x| > 6400 km OR |y| > 6400 km OR |z| > 6400 km
///   - x, y, z in meters
/// - **Geodetic**: Otherwise
///   - x = longitude in degrees (-180 to 180)
///   - y = latitude in degrees (-90 to 90)
///   - z = altitude in meters (above WGS84 ellipsoid)
///
/// # Detection threshold
///
/// The 6400 km (6,400,000 m) threshold aligns with Earth's radius (~6371 km):
/// - Geodetic: lon ≤ 180°, lat ≤ 90°, alt up to ~400,000 km for HEO/GEO satellites
/// - ECEF on/above Earth surface: minimum polar radius ~6357 km, so any ECEF component on
///   the surface exceeds the threshold.
///
/// Note: geodetic altitudes above 6400 km are legal (GEO orbit ~35,786 km). Use the
/// `coordinate_system` field to provide an explicit override and avoid ambiguity.
///
/// # Examples
///
/// ```
/// # use antenna_model::api::schemas::{CoordinateSystem, Position3D};
/// // ECEF coordinates above the 6400 km threshold auto-detect correctly
/// let ecef = Position3D::new(6_500_000.0, 0.0, 0.0);
/// assert_eq!(ecef.coordinate_system(), CoordinateSystem::ECEF);
///
/// // Earth-surface ECEF (equatorial radius 6378 km < 6400 km threshold): needs explicit tag
/// let mut ecef_surface = Position3D::new(6_378_137.0, 0.0, 100_000.0);
/// ecef_surface.coordinate_system = Some(CoordinateSystem::ECEF);
/// assert_eq!(ecef_surface.coordinate_system(), CoordinateSystem::ECEF);
///
/// // Geodetic coordinates (lon, lat degrees, alt meters)
/// let geodetic = Position3D::new(-118.1234, 34.5678, 100.0);
/// assert_eq!(geodetic.coordinate_system(), CoordinateSystem::Geodetic);
///
/// // High-altitude geodetic (GEO satellite) - set explicit tag to prevent misclassification
/// let mut geo = Position3D::new(0.0, 0.0, 35_786_000.0);
/// geo.coordinate_system = Some(CoordinateSystem::Geodetic);
/// assert!(geo.is_geodetic());
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Position3D {
    /// X coordinate: ECEF X (meters) OR longitude (degrees)
    pub x: f64,
    /// Y coordinate: ECEF Y (meters) OR latitude (degrees)
    pub y: f64,
    /// Z coordinate: ECEF Z (meters) OR altitude (meters)
    pub z: f64,
    /// Optional explicit coordinate system override. When `None`, auto-detected by magnitude.
    /// Set this field to avoid ambiguity for high-altitude geodetic positions (e.g. GEO orbit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_system: Option<CoordinateSystem>,
}

impl Position3D {
    /// Threshold for ECEF auto-detection (6400 km in meters).
    ///
    /// Geodetic coordinates use degrees for lon/lat (max ±180/±90) and meters for altitude.
    /// ECEF coordinates are in meters from Earth's center (polar radius ~6357 km, equatorial ~6378 km).
    /// Using 6400 km threshold aligns with Earth's radius:
    /// - Geodetic: lon/lat in degrees (≤ 180/90), alt can legally be hundreds of thousands of km
    /// - ECEF on/above Earth surface: at least one component ≥ Earth's minimum polar radius (~6357 km)
    ///
    /// High-altitude geodetic positions (e.g. GEO satellite at z=35,786,000 m) can exceed this
    /// threshold and will be misclassified without an explicit `coordinate_system` override.
    pub const ECEF_THRESHOLD_M: f64 = 6_400_000.0;

    /// Create a new Position3D with auto-detection enabled (coordinate_system = None).
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            coordinate_system: None,
        }
    }

    /// Determine the coordinate system for this position.
    ///
    /// If `coordinate_system` is set explicitly, that value is returned.
    /// Otherwise, auto-detection is performed: returns `CoordinateSystem::ECEF` if any
    /// coordinate magnitude exceeds `ECEF_THRESHOLD_M` (6400 km), otherwise `CoordinateSystem::Geodetic`.
    pub fn coordinate_system(&self) -> CoordinateSystem {
        if let Some(cs) = self.coordinate_system {
            return cs;
        }
        if self.x.abs() > Self::ECEF_THRESHOLD_M
            || self.y.abs() > Self::ECEF_THRESHOLD_M
            || self.z.abs() > Self::ECEF_THRESHOLD_M
        {
            CoordinateSystem::ECEF
        } else {
            CoordinateSystem::Geodetic
        }
    }

    /// Check if this position uses ECEF coordinates
    pub fn is_ecef(&self) -> bool {
        self.coordinate_system() == CoordinateSystem::ECEF
    }

    /// Check if this position uses Geodetic coordinates
    pub fn is_geodetic(&self) -> bool {
        self.coordinate_system() == CoordinateSystem::Geodetic
    }
}

/// 3D vector (used for feed offsets, etc.)
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Vector3D {
    /// X component
    pub x: f64,
    /// Y component
    pub y: f64,
    /// Z component
    pub z: f64,
}

impl Vector3D {
    /// Create a new vector
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Zero vector
    pub fn zero() -> Self {
        Self::new(0.0, 0.0, 0.0)
    }
}

// ============================================================================
// Gain Computation Request/Response
// ============================================================================

/// Request for antenna gain computation from 3D geometry.
///
/// Computes antenna gain given 3D positions of vehicle, reflector boresight,
/// feed, and emitter, along with operating frequency.
///
/// # Coordinate Systems
///
/// All Position3D fields support both ECEF and Geodetic coordinates with
/// automatic detection. Mix-and-match is allowed (e.g., vehicle in Geodetic,
/// emitter in ECEF).
///
/// # Multi-Feed Support
///
/// Use composite identifier `(antenna_id, feed_id)` to specify which feed
/// configuration to use.
///
/// # Beam Squint Correction
///
/// If `pointing_frequency_mhz` differs from `frequency_mhz`, beam squint
/// correction is applied to account for frequency-dependent beam pointing.
///
/// # Orientation
///
/// The `reflector_boresight` position establishes the dish pointing direction.
/// The vector from `vehicle_position` to `reflector_boresight` defines the
/// boresight axis of the antenna coordinate frame.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GainRequest {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier (for multi-feed antennas)
    pub feed_id: String,

    /// Vehicle position (ECEF or Geodetic, auto-detected)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    ///
    /// This position, together with `vehicle_position`, establishes the dish
    /// pointing direction. The vector from vehicle to boresight defines the
    /// antenna Z-axis.
    pub reflector_boresight: Position3D,

    /// Feed position (ECEF or Geodetic)
    pub feed_position: Position3D,

    /// Emitter position (ECEF or Geodetic)
    pub emitter_position: Position3D,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Pointing frequency in MHz (for beam squint correction, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_frequency_mhz: Option<f64>,

    /// Include reference gain computation (ideal: feed at focus, pointing at emitter)
    #[serde(default)]
    pub include_reference: bool,
}

/// Response from antenna gain computation.
///
/// Contains computed gain, optional reference gain and loss, geometry information,
/// warnings, calibration status, and performance metadata.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GainResponse {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Computed gain in dB (serialized as null when NaN for failed evaluations)
    #[serde(with = "nan_as_null")]
    pub gain_db: f64,

    /// Reference gain in dB (if include_reference=true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_gain_db: Option<f64>,

    /// Loss in dB (reference - actual, if reference computed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_db: Option<f64>,

    /// Computed geometry information
    pub geometry: GeometryInfo,

    /// Warnings (e.g., extrapolation, beam squint applied)
    pub warnings: Vec<String>,

    /// Computation metadata (timing, flags)
    pub metadata: ComputationMetadata,

    /// Calibration status and accuracy information (v2.0)
    /// Optional for backward compatibility - will be populated by service layer in Task 6.8
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

/// Computed geometry information.
///
/// Details about the geometric configuration computed from 3D positions.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GeometryInfo {
    /// Physical feed offset from the focal point in the antenna frame (meters).
    ///
    /// `x` and `y` are the lateral displacement of the feed from the optical axis;
    /// `z` is the axial displacement from the focal point (positive toward the reflector vertex).
    /// For an on-axis (boresight-aimed) feed all three components are ~zero.
    pub feed_offset_meters: Vector3D,

    /// Emitter azimuth in antenna frame (degrees)
    pub emitter_azimuth_deg: f64,

    /// Emitter elevation in antenna frame (degrees)
    pub emitter_elevation_deg: f64,

    /// Beam squint correction applied in degrees (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beam_squint_deg: Option<f64>,
}

/// Computation performance metadata.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ComputationMetadata {
    /// Total computation time in milliseconds
    pub computation_time_ms: f64,

    /// Coordinate transformation time in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coordinate_transform_ms: Option<f64>,

    /// Physics model computation time in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physics_model_ms: Option<f64>,

    /// Correction surface interpolation time in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correction_surface_ms: Option<f64>,

    /// Whether the query was extrapolated (outside calibrated range)
    pub extrapolated: bool,
}

// ============================================================================
// Batch Evaluation Request/Response
// ============================================================================

/// Request for batch gain computation.
///
/// Process multiple gain requests in parallel for improved throughput.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BatchGainRequest {
    /// List of gain computation requests
    pub evaluations: Vec<GainRequest>,
}

/// Response from batch gain computation.
///
/// Contains results for all evaluations and aggregate metadata.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BatchGainResponse {
    /// Results for each evaluation
    pub results: Vec<GainResponse>,

    /// Aggregate metadata
    pub metadata: BatchMetadata,
}

/// Aggregate metadata for batch computation.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BatchMetadata {
    /// Total computation time for batch in milliseconds
    pub total_computation_time_ms: f64,

    /// Number of evaluations
    pub count: usize,

    /// Number of evaluations that failed (NaN gain_db)
    pub failure_count: usize,
}

// ============================================================================
// Heatmap Request/Response
// ============================================================================

/// Request for loss heatmap generation.
///
/// Generates a 2D grid of loss values across antenna field of view.
/// Supports rectangular (azimuth/elevation) or H3 hexagonal grids.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HeatmapRequest {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Vehicle position (ECEF or Geodetic)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    pub reflector_boresight: Position3D,

    /// Feed position (ECEF or Geodetic)
    pub feed_position: Position3D,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Pointing frequency in MHz (for beam squint correction, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_frequency_mhz: Option<f64>,

    /// Grid configuration (rectangular or H3 hexagonal)
    pub grid_config: GridConfig,
}

/// Grid configuration for heatmap generation.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "grid_type", rename_all = "lowercase")]
pub enum GridConfig {
    /// Rectangular azimuth/elevation grid
    Rectangular {
        /// Azimuth range configuration
        azimuth_range_deg: RangeConfig,
        /// Elevation range configuration
        elevation_range_deg: RangeConfig,
    },
    /// H3 hexagonal grid
    H3 {
        /// H3 resolution (0-15, higher = finer resolution)
        h3_resolution: u8,
        /// Center azimuth in degrees
        center_azimuth_deg: f64,
        /// Center elevation in degrees
        center_elevation_deg: f64,
        /// Field of view in degrees
        field_of_view_deg: f64,
    },
}

/// Range configuration for rectangular grid.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RangeConfig {
    /// Minimum value in degrees
    pub min: f64,
    /// Maximum value in degrees
    pub max: f64,
    /// Step size in degrees
    pub step: f64,
}

impl RangeConfig {
    /// Create a new range configuration
    pub fn new(min: f64, max: f64, step: f64) -> Self {
        Self { min, max, step }
    }

    /// Calculate number of points in range
    pub fn num_points(&self) -> usize {
        if self.step <= 0.0 {
            return 0;
        }
        ((self.max - self.min) / self.step).ceil() as usize + 1
    }
}

/// Response from heatmap generation.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HeatmapResponse {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Grid data (rectangular or H3)
    pub grid: GridData,

    /// Warnings (e.g., some points extrapolated)
    pub warnings: Vec<String>,

    /// Heatmap metadata
    pub metadata: HeatmapMetadata,

    /// Calibration status and accuracy information (v2.0)
    /// Optional for backward compatibility - will be populated by service layer in Task 6.8
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

/// Grid data for heatmap.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "grid_type", rename_all = "lowercase")]
pub enum GridData {
    /// Rectangular grid data
    Rectangular {
        /// Azimuth values in degrees
        azimuth_values: Vec<f64>,
        /// Elevation values in degrees
        elevation_values: Vec<f64>,
        /// Loss values in dB (2D array: rows are elevation, columns are azimuth)
        loss_db: Vec<Vec<f64>>,
    },
    /// H3 hexagonal grid data
    H3 {
        /// H3 cell indices
        h3_indices: Vec<String>,
        /// Loss values in dB (one per H3 cell)
        loss_db: Vec<f64>,
    },
}

/// Heatmap computation metadata.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HeatmapMetadata {
    /// Number of grid points evaluated
    pub points_evaluated: usize,

    /// Total computation time in milliseconds
    pub computation_time_ms: f64,

    /// Peak gain in dB (reference for loss calculation)
    pub peak_gain_db: f64,

    /// Number of grid points that failed to compute (gain replaced with sentinel 999999.0)
    pub failed_points: usize,
}

// ============================================================================
// H3 Link Budget Request/Response
// ============================================================================

/// Request for H3-based link budget computation.
///
/// Computes per-cell link budget across a hexagonal grid of H3 cells
/// centered on the antenna boresight projection, covering `n_rings` rings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct H3LinkBudgetRequest {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier (for multi-feed antennas)
    pub feed_id: String,

    /// Vehicle position (ECEF or Geodetic, auto-detected)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    pub reflector_boresight: Position3D,

    /// Feed position (ECEF or Geodetic)
    pub feed_position: Position3D,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Pointing frequency in MHz (for beam squint correction, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_frequency_mhz: Option<f64>,

    /// Number of H3 rings around the center cell
    pub n_rings: u32,

    /// H3 resolution (0-15, higher = finer). Uses a default when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h3_resolution: Option<u8>,

    /// System noise temperature in Kelvin (used for G/T computation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_k: Option<f64>,
}

/// Per-cell link budget result for a single H3 cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct H3CellResult {
    /// H3 cell index (string representation)
    pub cell_id: String,

    /// Cell center longitude in degrees
    pub center_lon: f64,

    /// Cell center latitude in degrees
    pub center_lat: f64,

    /// Azimuth to cell center in antenna frame (degrees)
    pub azimuth_deg: f64,

    /// Elevation to cell center in antenna frame (degrees)
    pub elevation_deg: f64,

    /// Distance from vehicle to cell center in km
    pub distance_km: f64,

    /// Antenna gain toward cell center in dB
    pub gain_db: f64,

    /// Gain relative to the grid-center cell (feed ground target), in dB.
    /// Computed as `boresight_gain_db - gain_db`, where `boresight_gain_db` is the
    /// gain toward the center H3 cell (the cell nearest the feed pointing location),
    /// not the true beam peak (which may lie at a slightly different direction).
    /// Both `boresight_gain_db` and `gain_db` are on the same basis (physics +
    /// correction surface, if applicable), so loss_db is internally consistent.
    pub loss_db: f64,

    /// Free-space path loss in dB
    pub free_space_path_loss_db: f64,

    /// Total path loss (free-space + other losses) in dB
    pub total_path_loss_db: f64,

    /// G/T (Gain-over-Temperature) in dB/K (present only when temperature_k was provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub g_over_t_db: Option<f64>,
}

/// Response from H3-based link budget computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct H3LinkBudgetResponse {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// H3 cell index of the center cell (string representation)
    pub center_cell_id: String,

    /// H3 resolution used
    pub h3_resolution: u8,

    /// Per-cell results
    pub cells: Vec<H3CellResult>,

    /// Warnings (e.g., extrapolated cells, out-of-range queries)
    pub warnings: Vec<String>,

    /// Computation metadata
    pub metadata: HeatmapMetadata,

    /// Calibration status and accuracy information (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

// ============================================================================
// Antenna and Feed Information
// ============================================================================

/// Response listing available antennas.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AntennaListResponse {
    /// List of available antennas
    pub antennas: Vec<AntennaInfo>,
}

/// Information about an antenna.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AntennaInfo {
    /// Antenna identifier
    pub id: String,

    /// Human-readable antenna name
    pub name: String,

    /// Whether antenna is enabled
    pub enabled: bool,

    /// Number of feeds available
    pub feed_count: usize,

    /// List of available feed IDs
    pub feed_ids: Vec<String>,
}

/// Detailed information about a specific antenna.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AntennaDetailsResponse {
    /// Antenna identifier
    pub id: String,

    /// Human-readable antenna name
    pub name: String,

    /// Whether antenna is enabled
    pub enabled: bool,

    /// List of available feeds
    pub feeds: Vec<FeedInfo>,

    /// Validity ranges for queries
    pub validity_ranges: ValidityRangesInfo,

    /// Calibration metadata
    pub calibration: CalibrationInfo,

    /// Physical parameters
    pub physical_parameters: PhysicalParametersInfo,

    /// Calibration status and accuracy information (v2.0)
    /// Optional for backward compatibility - will be populated by service layer in Task 6.8
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

/// Information about a feed.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FeedInfo {
    /// Feed identifier
    pub id: String,

    /// Feed position offset from focal point (meters)
    pub position_offset: Vector3D,

    /// Frequency range in MHz
    pub frequency_range_mhz: (f64, f64),

    /// Feed pattern q-factor
    pub q_factor: f64,
}

/// Validity ranges information.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ValidityRangesInfo {
    /// Azimuth range in degrees (min, max)
    pub azimuth_deg: (f64, f64),

    /// Elevation range in degrees (min, max)
    pub elevation_deg: (f64, f64),

    /// Frequency range in MHz (min, max)
    pub frequency_mhz: (f64, f64),

    /// Temperature in Kelvin
    pub temperature_k: f64,
}

/// Calibration information.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CalibrationInfo {
    /// Calibration date (ISO 8601)
    pub date: String,

    /// Format version
    pub version: String,

    /// Data source
    pub source: String,

    /// Root mean squared error in dB (None for uncalibrated antennas)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rmse_db: Option<f64>,

    /// R² correlation coefficient (None for uncalibrated antennas)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_squared: Option<f64>,

    /// Number of measurement points
    pub num_measurements: usize,
}

/// Physical parameters information.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct PhysicalParametersInfo {
    /// Dish diameter in meters
    pub diameter_m: f64,

    /// Focal length in meters
    pub focal_length_m: f64,

    /// f/D ratio
    pub f_over_d_ratio: f64,

    /// Surface RMS error in millimeters
    pub surface_rms_mm: f64,

    /// Mesh parameters (if mesh reflector)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshInfo>,
}

/// Mesh reflector information.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MeshInfo {
    /// Mesh spacing in millimeters
    pub mesh_spacing_mm: f64,

    /// Wire diameter in millimeters
    pub wire_diameter_mm: f64,
}

// ============================================================================
// Calibration Status Information (v2.0 - Partial Calibration Support)
// ============================================================================

/// Calibration status information included in API responses.
///
/// Indicates the level of calibration data available and expected accuracy
/// for antenna gain predictions. This information helps users understand
/// the quality and reliability of the returned predictions.
///
/// # Status Levels
///
/// - **fully_calibrated**: Dense measurement grid with full correction surface (±1 dB)
/// - **partially_calibrated**: Limited measurements (boresight or sparse grid) (±1-3 dB)
/// - **uncalibrated**: Design specifications only, no measurements (±3-5 dB absolute, ±2 dB loss)
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CalibrationStatusInfo {
    /// Calibration status: "fully_calibrated", "partially_calibrated", or "uncalibrated"
    pub status: String,

    /// Expected accuracy estimate in dB
    pub accuracy_estimate_db: f64,

    /// Expected loss (relative gain) accuracy in dB (only for uncalibrated antennas)
    /// Better than absolute accuracy due to systematic error cancellation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_accuracy_estimate_db: Option<f64>,

    /// Measurement coverage information (only for partially calibrated antennas)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageInfo>,

    /// Whether correction surface was applied to this result
    pub correction_applied: bool,

    /// Source of physical parameters: "measurement_tuned", "design_specifications", or "factory_calibrated"
    pub parameters_source: String,
}

impl From<&CalibrationStatus> for CalibrationStatusInfo {
    fn from(status: &CalibrationStatus) -> Self {
        match status {
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db,
            } => CalibrationStatusInfo {
                status: "fully_calibrated".to_string(),
                accuracy_estimate_db: *accuracy_estimate_db,
                loss_accuracy_estimate_db: None,
                coverage: None,
                correction_applied: false, // Will be updated by service layer
                parameters_source: "measurement_tuned".to_string(),
            },
            CalibrationStatus::PartiallyCalibrated {
                accuracy_estimate_db,
                coverage,
            } => CalibrationStatusInfo {
                status: "partially_calibrated".to_string(),
                accuracy_estimate_db: *accuracy_estimate_db,
                loss_accuracy_estimate_db: None,
                coverage: Some(CoverageInfo::from(coverage)),
                correction_applied: false, // Will be updated by service layer
                parameters_source: "measurement_tuned".to_string(),
            },
            CalibrationStatus::Uncalibrated {
                accuracy_estimate_db,
                loss_accuracy_estimate_db,
            } => CalibrationStatusInfo {
                status: "uncalibrated".to_string(),
                accuracy_estimate_db: *accuracy_estimate_db,
                loss_accuracy_estimate_db: Some(*loss_accuracy_estimate_db),
                coverage: None,
                correction_applied: false,
                parameters_source: "design_specifications".to_string(),
            },
        }
    }
}

/// Measurement coverage information for partially calibrated antennas.
///
/// Describes the spatial, frequency, and measurement density of calibration data.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CoverageInfo {
    /// Azimuth coverage range in degrees (min, max)
    pub azimuth_range_deg: (f64, f64),

    /// Elevation coverage range in degrees (min, max)
    pub elevation_range_deg: (f64, f64),

    /// Frequency coverage range in MHz (min, max)
    pub frequency_range_mhz: (f64, f64),

    /// Total number of measurement points
    pub num_measurements: usize,

    /// Whether this is boresight-only calibration (single spatial point)
    pub is_boresight_only: bool,
}

impl From<&CalibrationCoverage> for CoverageInfo {
    fn from(coverage: &CalibrationCoverage) -> Self {
        CoverageInfo {
            azimuth_range_deg: coverage.azimuth_range,
            elevation_range_deg: coverage.elevation_range,
            frequency_range_mhz: coverage.frequency_range,
            num_measurements: coverage.num_measurements,
            is_boresight_only: coverage.is_boresight_only(),
        }
    }
}

// ============================================================================
// Health and Status
// ============================================================================

/// Health check response (liveness probe).
///
/// Returns 200 when service is responsive.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HealthResponse {
    /// Health status - "healthy" when operational
    pub status: String,
}

impl HealthResponse {
    /// Create a healthy response
    pub fn healthy() -> Self {
        Self {
            status: "healthy".to_string(),
        }
    }
}

/// Status endpoint response (readiness probe).
///
/// Returns detailed service status including loaded antennas,
/// uptime, version, and operational status.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StatusResponse {
    /// Service status - "ok" when operational
    pub status: String,

    /// Application version from Cargo.toml
    pub version: String,

    /// Uptime in seconds since server start
    pub uptime_seconds: u64,

    /// Number of loaded antennas
    #[serde(skip_serializing_if = "Option::is_none")]
    pub antenna_count: Option<usize>,

    /// List of loaded antenna IDs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub antenna_ids: Option<Vec<String>>,

    /// Memory usage in bytes (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
}

impl StatusResponse {
    /// Create a new status response with "ok" status
    pub fn ok(version: String, uptime_seconds: u64) -> Self {
        Self {
            status: "ok".to_string(),
            version,
            uptime_seconds,
            antenna_count: None,
            antenna_ids: None,
            memory_bytes: None,
        }
    }

    /// Add antenna information
    pub fn with_antennas(mut self, antenna_ids: Vec<String>) -> Self {
        self.antenna_count = Some(antenna_ids.len());
        self.antenna_ids = Some(antenna_ids);
        self
    }

    /// Add memory usage information
    pub fn with_memory(mut self, memory_bytes: u64) -> Self {
        self.memory_bytes = Some(memory_bytes);
        self
    }
}

// ============================================================================
// Error Response
// ============================================================================

/// Standardized error response.
///
/// Returned for all error conditions with appropriate HTTP status codes.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ErrorResponse {
    /// Error type/category
    pub error: String,

    /// Human-readable error message
    pub message: String,

    /// Field that caused the error (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,

    /// Additional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl ErrorResponse {
    /// Create a new error response
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
            field: None,
            details: None,
        }
    }

    /// Add field information
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Add additional details
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Create antenna not found error
    pub fn antenna_not_found(antenna_id: impl Into<String>) -> Self {
        Self::new(
            "AntennaNotFound",
            format!("Antenna '{}' not found", antenna_id.into()),
        )
    }

    /// Create feed not found error
    pub fn feed_not_found(antenna_id: impl Into<String>, feed_id: impl Into<String>) -> Self {
        Self::new(
            "FeedNotFound",
            format!(
                "Feed '{}' not found for antenna '{}'",
                feed_id.into(),
                antenna_id.into()
            ),
        )
    }

    /// Create invalid parameter error
    pub fn invalid_parameter(param: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new("InvalidParameter", reason.into()).with_field(param.into())
    }

    /// Create invalid coordinate error
    pub fn invalid_coordinate(param: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new("InvalidCoordinate", reason.into()).with_field(param.into())
    }

    /// Create coordinate transform error
    pub fn coordinate_transform_error(details: impl Into<String>) -> Self {
        Self::new(
            "CoordinateTransformError",
            "Coordinate transformation failed",
        )
        .with_details(details.into())
    }

    /// Create computation error
    pub fn computation_error(details: impl Into<String>) -> Self {
        Self::new("ComputationError", "Antenna gain computation failed")
            .with_details(details.into())
    }

    /// Create internal error
    pub fn internal_error(details: impl Into<String>) -> Self {
        Self::new("InternalError", "Internal server error").with_details(details.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Position3D Tests
    // ========================================================================

    #[test]
    fn test_position3d_ecef_detection() {
        // ECEF coordinates at high-altitude satellite (components > 6400 km auto-detect)
        let ecef = Position3D::new(10_000_000.0, 5_000_000.0, 2_000_000.0);
        assert_eq!(ecef.coordinate_system(), CoordinateSystem::ECEF);
        assert!(ecef.is_ecef());
        assert!(!ecef.is_geodetic());

        // Earth-surface ECEF: component just above 6400 km auto-detects correctly
        let ecef2 = Position3D::new(6_401_000.0, 0.0, 0.0);
        assert_eq!(ecef2.coordinate_system(), CoordinateSystem::ECEF);

        // Real Earth-surface ECEF points (equatorial radius 6378 km < threshold) need
        // explicit tag for correct detection — set it to preserve intent
        let mut ecef_surface = Position3D::new(6_378_137.0, 0.0, 0.0); // equator, prime meridian
        ecef_surface.coordinate_system = Some(CoordinateSystem::ECEF);
        assert!(ecef_surface.is_ecef());
        assert!(!ecef_surface.is_geodetic());

        // Explicit tag also works for typical mid-latitude ECEF surface points
        let mut ecef_la = Position3D::new(-2_500_000.0, -4_500_000.0, 3_600_000.0); // ~LA area
        ecef_la.coordinate_system = Some(CoordinateSystem::ECEF);
        assert!(ecef_la.is_ecef());
    }

    #[test]
    fn test_position3d_geodetic_detection() {
        // Geodetic coordinates (small magnitude)
        let geodetic = Position3D::new(-118.1234, 34.5678, 100.0);
        assert_eq!(geodetic.coordinate_system(), CoordinateSystem::Geodetic);
        assert!(!geodetic.is_ecef());
        assert!(geodetic.is_geodetic());
    }

    #[test]
    fn test_position3d_boundary_detection() {
        // Just below threshold (6400 km = 6,400,000 m) - should be Geodetic
        let below = Position3D::new(6_399_000.0, 0.0, 0.0);
        assert_eq!(below.coordinate_system(), CoordinateSystem::Geodetic);

        // Just above threshold - should be ECEF
        let above = Position3D::new(6_401_000.0, 0.0, 0.0);
        assert_eq!(above.coordinate_system(), CoordinateSystem::ECEF);

        // Negative coordinates
        let negative = Position3D::new(-6_401_000.0, 0.0, 0.0);
        assert_eq!(negative.coordinate_system(), CoordinateSystem::ECEF);
    }

    #[test]
    fn test_detection_threshold_is_6400km() {
        assert!(!Position3D::new(6_399_000.0, 0.0, 0.0).is_ecef());
        assert!(Position3D::new(6_401_000.0, 0.0, 0.0).is_ecef());
    }

    #[test]
    fn test_explicit_coordinate_system_overrides_detection() {
        // GEO altitude in geodetic form - explicit override forces Geodetic
        let mut pos = Position3D::new(0.0, 0.0, 35_786_000.0);
        pos.coordinate_system = Some(CoordinateSystem::Geodetic);
        assert!(pos.is_geodetic());

        // Small values normally Geodetic - explicit ECEF override forces ECEF
        let mut pos2 = Position3D::new(100.0, 100.0, 100.0);
        pos2.coordinate_system = Some(CoordinateSystem::ECEF);
        assert!(pos2.is_ecef());
    }

    #[test]
    fn test_position3d_backward_compatible_deserialization() {
        // Bare JSON without coordinate_system should deserialize fine (backward compat)
        let json = r#"{"x":1.0,"y":2.0,"z":3.0}"#;
        let pos: Position3D = serde_json::from_str(json).unwrap();
        assert_eq!(pos.x, 1.0);
        assert_eq!(pos.y, 2.0);
        assert_eq!(pos.z, 3.0);
        assert_eq!(pos.coordinate_system, None);
    }

    #[test]
    fn test_position3d_explicit_coordinate_system_round_trip() {
        let mut pos = Position3D::new(1.0, 2.0, 3.0);
        pos.coordinate_system = Some(CoordinateSystem::ECEF);
        let json = serde_json::to_string(&pos).unwrap();
        let deserialized: Position3D = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.coordinate_system, Some(CoordinateSystem::ECEF));
    }

    #[test]
    fn test_position3d_no_coordinate_system_not_serialized() {
        // coordinate_system: None should NOT appear in serialized JSON
        let pos = Position3D::new(1.0, 2.0, 3.0);
        let json = serde_json::to_string(&pos).unwrap();
        assert!(!json.contains("coordinate_system"));
    }

    #[test]
    fn test_position3d_serialization() {
        let pos = Position3D::new(1.0, 2.0, 3.0);
        let json = serde_json::to_string(&pos).unwrap();
        let deserialized: Position3D = serde_json::from_str(&json).unwrap();
        assert_eq!(pos.x, deserialized.x);
        assert_eq!(pos.y, deserialized.y);
        assert_eq!(pos.z, deserialized.z);
        assert_eq!(deserialized.coordinate_system, None);
    }

    // ========================================================================
    // Vector3D Tests
    // ========================================================================

    #[test]
    fn test_vector3d_zero() {
        let zero = Vector3D::zero();
        assert_eq!(zero.x, 0.0);
        assert_eq!(zero.y, 0.0);
        assert_eq!(zero.z, 0.0);
    }

    #[test]
    fn test_vector3d_serialization() {
        let vec = Vector3D::new(1.0, 2.0, 3.0);
        let json = serde_json::to_string(&vec).unwrap();
        let deserialized: Vector3D = serde_json::from_str(&json).unwrap();
        assert_eq!(vec, deserialized);
    }

    // ========================================================================
    // GainRequest Tests
    // ========================================================================

    #[test]
    fn test_gain_request_serialization() {
        let request = GainRequest {
            antenna_id: "antenna_1".to_string(),
            feed_id: "x_band_feed".to_string(),
            vehicle_position: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4510731.123, 4510731.456, 3488865.789)
            },
            reflector_boresight: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4510732.0, 4510732.0, 3488950.0)
            },
            feed_position: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4510731.5, 4510731.5, 3488870.0)
            },
            emitter_position: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4520000.0, 4520000.0, 3500000.0)
            },
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: Some(8450.0),
            include_reference: true,
        };

        let json = serde_json::to_string(&request).unwrap();
        let deserialized: GainRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request.antenna_id, deserialized.antenna_id);
        assert_eq!(request.feed_id, deserialized.feed_id);
    }

    #[test]
    fn test_gain_request_with_euler_angles() {
        let request = GainRequest {
            antenna_id: "antenna_1".to_string(),
            feed_id: "x_band_feed".to_string(),
            vehicle_position: Position3D::new(-118.1234, 34.5678, 100.0),
            reflector_boresight: Position3D::new(-118.1234, 34.5679, 110.0), // 10m above vehicle
            feed_position: Position3D::new(-118.124, 34.568, 105.0),
            emitter_position: Position3D::new(-117.0, 35.0, 400000.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
        };

        let json = serde_json::to_string(&request).unwrap();
        let deserialized: GainRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request.antenna_id, deserialized.antenna_id);
    }

    // ========================================================================
    // RangeConfig Tests
    // ========================================================================

    #[test]
    fn test_range_config_num_points() {
        let range = RangeConfig::new(0.0, 10.0, 2.0);
        assert_eq!(range.num_points(), 6); // 0, 2, 4, 6, 8, 10

        let range2 = RangeConfig::new(0.0, 360.0, 5.0);
        assert_eq!(range2.num_points(), 73); // 0, 5, 10, ..., 360
    }

    #[test]
    fn test_range_config_zero_step() {
        let range = RangeConfig::new(0.0, 10.0, 0.0);
        assert_eq!(range.num_points(), 0);
    }

    // ========================================================================
    // GridConfig Tests
    // ========================================================================

    #[test]
    fn test_grid_config_rectangular_serialization() {
        let grid = GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 360.0, 5.0),
            elevation_range_deg: RangeConfig::new(0.0, 90.0, 2.0),
        };

        let json = serde_json::to_string(&grid).unwrap();
        assert!(json.contains("\"grid_type\":\"rectangular\""));

        let deserialized: GridConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, GridConfig::Rectangular { .. }));
    }

    #[test]
    fn test_grid_config_h3_serialization() {
        let grid = GridConfig::H3 {
            h3_resolution: 7,
            center_azimuth_deg: 180.0,
            center_elevation_deg: 45.0,
            field_of_view_deg: 30.0,
        };

        let json = serde_json::to_string(&grid).unwrap();
        assert!(json.contains("\"grid_type\":\"h3\""));

        let deserialized: GridConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, GridConfig::H3 { .. }));
    }

    // ========================================================================
    // StatusResponse Tests
    // ========================================================================

    #[test]
    fn test_status_response_serialization() {
        let response = StatusResponse::ok("0.1.0".to_string(), 3600);
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"version\":\"0.1.0\""));
        assert!(json.contains("\"uptime_seconds\":3600"));
    }

    #[test]
    fn test_status_response_deserialization() {
        let json = r#"{"status":"ok","version":"0.1.0","uptime_seconds":3600}"#;
        let response: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.status, "ok");
        assert_eq!(response.version, "0.1.0");
        assert_eq!(response.uptime_seconds, 3600);
    }

    #[test]
    fn test_status_response_ok_constructor() {
        let response = StatusResponse::ok("1.2.3".to_string(), 7200);
        assert_eq!(response.status, "ok");
        assert_eq!(response.version, "1.2.3");
        assert_eq!(response.uptime_seconds, 7200);
    }

    #[test]
    fn test_status_response_with_antennas() {
        let response = StatusResponse::ok("1.0.0".to_string(), 100)
            .with_antennas(vec!["antenna_1".to_string(), "antenna_2".to_string()]);

        assert_eq!(response.antenna_count, Some(2));
        assert_eq!(
            response.antenna_ids,
            Some(vec!["antenna_1".to_string(), "antenna_2".to_string()])
        );
    }

    #[test]
    fn test_status_response_with_memory() {
        let response = StatusResponse::ok("1.0.0".to_string(), 100).with_memory(1024 * 1024);

        assert_eq!(response.memory_bytes, Some(1024 * 1024));
    }

    // ========================================================================
    // HealthResponse Tests
    // ========================================================================

    #[test]
    fn test_health_response_healthy() {
        let response = HealthResponse::healthy();
        assert_eq!(response.status, "healthy");
    }

    #[test]
    fn test_health_response_serialization() {
        let response = HealthResponse::healthy();
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"healthy\""));
    }

    // ========================================================================
    // ErrorResponse Tests
    // ========================================================================

    #[test]
    fn test_error_response_basic() {
        let error = ErrorResponse::new("TestError", "Test message");
        assert_eq!(error.error, "TestError");
        assert_eq!(error.message, "Test message");
        assert!(error.field.is_none());
        assert!(error.details.is_none());
    }

    #[test]
    fn test_error_response_with_field() {
        let error = ErrorResponse::new("TestError", "Test message").with_field("test_field");
        assert_eq!(error.field, Some("test_field".to_string()));
    }

    #[test]
    fn test_error_response_with_details() {
        let error = ErrorResponse::new("TestError", "Test message").with_details("More info");
        assert_eq!(error.details, Some("More info".to_string()));
    }

    #[test]
    fn test_error_response_antenna_not_found() {
        let error = ErrorResponse::antenna_not_found("antenna_1");
        assert_eq!(error.error, "AntennaNotFound");
        assert!(error.message.contains("antenna_1"));
    }

    #[test]
    fn test_error_response_feed_not_found() {
        let error = ErrorResponse::feed_not_found("antenna_1", "feed_1");
        assert_eq!(error.error, "FeedNotFound");
        assert!(error.message.contains("antenna_1"));
        assert!(error.message.contains("feed_1"));
    }

    #[test]
    fn test_error_response_invalid_parameter() {
        let error = ErrorResponse::invalid_parameter("frequency", "must be positive");
        assert_eq!(error.error, "InvalidParameter");
        assert_eq!(error.field, Some("frequency".to_string()));
    }

    #[test]
    fn test_error_response_serialization() {
        let error = ErrorResponse::new("TestError", "Test message")
            .with_field("test_field")
            .with_details("More info");

        let json = serde_json::to_string(&error).unwrap();
        let deserialized: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(error, deserialized);
    }

    // ========================================================================
    // Field Naming Tests (snake_case)
    // ========================================================================

    #[test]
    fn test_field_naming_snake_case() {
        let response = StatusResponse::ok("1.0.0".to_string(), 100);
        let json = serde_json::to_string(&response).unwrap();

        // Check that field names are snake_case
        assert!(json.contains("\"uptime_seconds\""));
        assert!(!json.contains("\"uptimeSeconds\""));
    }

    #[test]
    fn test_gain_request_field_naming() {
        let request = GainRequest {
            antenna_id: "antenna_1".to_string(),
            feed_id: "x_band_feed".to_string(),
            vehicle_position: Position3D::new(0.0, 0.0, 0.0),
            reflector_boresight: Position3D::new(0.0, 0.0, 10.0), // 10m above vehicle
            feed_position: Position3D::new(0.0, 0.0, 23.6),       // 10m + 13.6m focal length
            emitter_position: Position3D::new(100.0, 100.0, 100.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
        };

        let json = serde_json::to_string(&request).unwrap();

        // Check field naming
        assert!(json.contains("\"antenna_id\""));
        assert!(json.contains("\"feed_id\""));
        assert!(json.contains("\"vehicle_position\""));
        assert!(json.contains("\"reflector_boresight\""));
        assert!(json.contains("\"feed_position\""));
        assert!(json.contains("\"emitter_position\""));
        assert!(json.contains("\"frequency_mhz\""));
        assert!(json.contains("\"include_reference\""));
    }

    // ========================================================================
    // CalibrationStatusInfo Tests (v2.0 - Partial Calibration Support)
    // ========================================================================

    #[test]
    fn test_calibration_status_info_from_fully_calibrated() {
        use crate::data::types::CalibrationStatus;

        let status = CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        };

        let info = CalibrationStatusInfo::from(&status);

        assert_eq!(info.status, "fully_calibrated");
        assert_eq!(info.accuracy_estimate_db, 1.0);
        assert_eq!(info.loss_accuracy_estimate_db, None);
        assert_eq!(info.coverage, None);
        assert_eq!(info.correction_applied, false);
        assert_eq!(info.parameters_source, "measurement_tuned");
    }

    #[test]
    fn test_calibration_status_info_from_partially_calibrated() {
        use crate::data::types::{CalibrationCoverage, CalibrationStatus};

        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (2000.0, 2300.0),
            num_measurements: 25,
            has_correction_surface: true,
        };

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        };

        let info = CalibrationStatusInfo::from(&status);

        assert_eq!(info.status, "partially_calibrated");
        assert_eq!(info.accuracy_estimate_db, 1.5);
        assert_eq!(info.loss_accuracy_estimate_db, None);
        assert!(info.coverage.is_some());
        assert_eq!(info.correction_applied, false);
        assert_eq!(info.parameters_source, "measurement_tuned");

        let coverage_info = info.coverage.unwrap();
        assert_eq!(coverage_info.azimuth_range_deg, (0.0, 0.0));
        assert_eq!(coverage_info.elevation_range_deg, (0.0, 0.0));
        assert_eq!(coverage_info.frequency_range_mhz, (2000.0, 2300.0));
        assert_eq!(coverage_info.num_measurements, 25);
        assert!(coverage_info.is_boresight_only);
    }

    #[test]
    fn test_calibration_status_info_from_uncalibrated() {
        use crate::data::types::CalibrationStatus;

        let status = CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        };

        let info = CalibrationStatusInfo::from(&status);

        assert_eq!(info.status, "uncalibrated");
        assert_eq!(info.accuracy_estimate_db, 3.0);
        assert_eq!(info.loss_accuracy_estimate_db, Some(2.0));
        assert_eq!(info.coverage, None);
        assert_eq!(info.correction_applied, false);
        assert_eq!(info.parameters_source, "design_specifications");
    }

    #[test]
    fn test_calibration_status_info_serialization_fully_calibrated() {
        use crate::data::types::CalibrationStatus;

        let status = CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        };

        let info = CalibrationStatusInfo::from(&status);
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"status\":\"fully_calibrated\""));
        assert!(json.contains("\"accuracy_estimate_db\":1.0"));
        assert!(!json.contains("loss_accuracy_estimate_db")); // Should be omitted
        assert!(!json.contains("coverage")); // Should be omitted
        assert!(json.contains("\"correction_applied\":false"));
        assert!(json.contains("\"parameters_source\":\"measurement_tuned\""));

        // Test deserialization
        let deserialized: CalibrationStatusInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn test_calibration_status_info_serialization_partially_calibrated() {
        use crate::data::types::{CalibrationCoverage, CalibrationStatus};

        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 90.0),
            frequency_range: (8000.0, 8500.0),
            num_measurements: 1000,
            has_correction_surface: true,
        };

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        };

        let info = CalibrationStatusInfo::from(&status);
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"status\":\"partially_calibrated\""));
        assert!(json.contains("\"accuracy_estimate_db\":1.5"));
        assert!(json.contains("\"coverage\""));
        assert!(json.contains("\"azimuth_range_deg\""));
        assert!(json.contains("\"num_measurements\":1000"));
        assert!(json.contains("\"is_boresight_only\":false"));

        // Test deserialization
        let deserialized: CalibrationStatusInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.status, "partially_calibrated");
        assert!(deserialized.coverage.is_some());
    }

    #[test]
    fn test_calibration_status_info_serialization_uncalibrated() {
        use crate::data::types::CalibrationStatus;

        let status = CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        };

        let info = CalibrationStatusInfo::from(&status);
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"status\":\"uncalibrated\""));
        assert!(json.contains("\"accuracy_estimate_db\":3.0"));
        assert!(json.contains("\"loss_accuracy_estimate_db\":2.0"));
        assert!(!json.contains("\"coverage\"")); // Should be omitted
        assert!(json.contains("\"parameters_source\":\"design_specifications\""));

        // Test deserialization
        let deserialized: CalibrationStatusInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn test_coverage_info_from_calibration_coverage() {
        use crate::data::types::CalibrationCoverage;

        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 90.0),
            frequency_range: (2000.0, 2300.0),
            num_measurements: 500,
            has_correction_surface: true,
        };

        let info = CoverageInfo::from(&coverage);

        assert_eq!(info.azimuth_range_deg, (0.0, 360.0));
        assert_eq!(info.elevation_range_deg, (0.0, 90.0));
        assert_eq!(info.frequency_range_mhz, (2000.0, 2300.0));
        assert_eq!(info.num_measurements, 500);
        assert!(!info.is_boresight_only);
    }

    #[test]
    fn test_coverage_info_boresight_only_detection() {
        use crate::data::types::CalibrationCoverage;

        // Boresight only - single spatial point
        let boresight_coverage = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (2000.0, 2300.0),
            num_measurements: 25,
            has_correction_surface: false,
        };

        let boresight_info = CoverageInfo::from(&boresight_coverage);
        assert!(boresight_info.is_boresight_only);

        // Sparse grid - not boresight only
        let sparse_coverage = CalibrationCoverage {
            azimuth_range: (-5.0, 5.0),
            elevation_range: (-5.0, 5.0),
            frequency_range: (2000.0, 2300.0),
            num_measurements: 100,
            has_correction_surface: true,
        };

        let sparse_info = CoverageInfo::from(&sparse_coverage);
        assert!(!sparse_info.is_boresight_only);
    }

    #[test]
    fn test_coverage_info_serialization() {
        use crate::data::types::CalibrationCoverage;

        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 360.0),
            elevation_range: (0.0, 90.0),
            frequency_range: (8000.0, 8500.0),
            num_measurements: 1000,
            has_correction_surface: true,
        };

        let info = CoverageInfo::from(&coverage);
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"azimuth_range_deg\":[0.0,360.0]"));
        assert!(json.contains("\"elevation_range_deg\":[0.0,90.0]"));
        assert!(json.contains("\"frequency_range_mhz\":[8000.0,8500.0]"));
        assert!(json.contains("\"num_measurements\":1000"));
        assert!(json.contains("\"is_boresight_only\":false"));

        // Test deserialization
        let deserialized: CoverageInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn test_gain_response_with_calibration_status() {
        use crate::data::types::CalibrationStatus;

        let status = CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        };

        let response = GainResponse {
            antenna_id: "antenna_1".to_string(),
            feed_id: "x_band".to_string(),
            gain_db: 45.5,
            reference_gain_db: Some(50.0),
            loss_db: Some(4.5),
            geometry: GeometryInfo {
                feed_offset_meters: Vector3D::new(0.0, 0.0, 0.1),
                emitter_azimuth_deg: 10.0,
                emitter_elevation_deg: 45.0,
                beam_squint_deg: None,
            },
            warnings: vec![],
            metadata: ComputationMetadata {
                computation_time_ms: 50.0,
                coordinate_transform_ms: Some(10.0),
                physics_model_ms: Some(30.0),
                correction_surface_ms: Some(5.0),
                extrapolated: false,
            },
            calibration_status: Some(CalibrationStatusInfo::from(&status)),
        };

        let json = serde_json::to_string(&response).unwrap();

        assert!(json.contains("\"gain_db\":45.5"));
        assert!(json.contains("\"calibration_status\""));
        assert!(json.contains("\"status\":\"fully_calibrated\""));
        assert!(json.contains("\"accuracy_estimate_db\":1.0"));

        // Test deserialization
        let deserialized: GainResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.antenna_id, "antenna_1");
        assert_eq!(
            deserialized.calibration_status.unwrap().status,
            "fully_calibrated"
        );
    }

    #[test]
    fn test_gain_response_backward_compatibility_without_calibration_status() {
        // Test that responses without calibration_status still deserialize correctly
        let json = r#"{
            "antenna_id": "antenna_1",
            "feed_id": "x_band",
            "gain_db": 45.5,
            "geometry": {
                "feed_offset_meters": {"x": 0.0, "y": 0.0, "z": 0.1},
                "emitter_azimuth_deg": 10.0,
                "emitter_elevation_deg": 45.0
            },
            "warnings": [],
            "metadata": {
                "computation_time_ms": 50.0,
                "extrapolated": false
            }
        }"#;

        let deserialized: GainResponse = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.antenna_id, "antenna_1");
        assert_eq!(deserialized.feed_id, "x_band");
        assert_eq!(deserialized.gain_db, 45.5);
        assert!(deserialized.calibration_status.is_none()); // No calibration status in old format
    }

    // ========================================================================
    // H3LinkBudgetRequest / H3CellResult Tests
    // ========================================================================

    #[test]
    fn test_h3_link_budget_request_serde_round_trip() {
        let request = H3LinkBudgetRequest {
            antenna_id: "antenna_1".to_string(),
            feed_id: "x_band_feed".to_string(),
            vehicle_position: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4510731.0, 4510731.0, 3488865.0)
            },
            reflector_boresight: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4510732.0, 4510732.0, 3488950.0)
            },
            feed_position: Position3D {
                coordinate_system: Some(CoordinateSystem::ECEF),
                ..Position3D::new(4510731.5, 4510731.5, 3488870.0)
            },
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: Some(8450.0),
            n_rings: 3,
            h3_resolution: Some(7),
            temperature_k: Some(290.0),
        };

        let json = serde_json::to_string(&request).unwrap();
        let deserialized: H3LinkBudgetRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(request, deserialized);
    }

    #[test]
    fn test_h3_cell_result_g_over_t_absent_when_none() {
        let result = H3CellResult {
            cell_id: "8a2a100d2dfffff".to_string(),
            center_lon: -118.1234,
            center_lat: 34.5678,
            azimuth_deg: 45.0,
            elevation_deg: 30.0,
            distance_km: 500.0,
            gain_db: 42.0,
            loss_db: 3.0,
            free_space_path_loss_db: 180.0,
            total_path_loss_db: 183.0,
            g_over_t_db: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(!json.contains("g_over_t_db"));
    }
}
