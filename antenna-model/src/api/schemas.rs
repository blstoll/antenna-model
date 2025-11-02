//! API request and response schemas
//!
//! This module defines the data structures for API requests and responses,
//! all using serde for JSON serialization/deserialization.
//!
//! # 3D Coordinate System Support
//!
//! All 3D positions support automatic coordinate system detection:
//! - **ECEF** (Earth-Centered Earth-Fixed): Detected when |x|, |y|, or |z| > 6400 km
//! - **Geodetic**: Otherwise (x=longitude degrees, y=latitude degrees, z=altitude meters)
//!
//! # Multi-Feed Support
//!
//! Antennas can have multiple feeds, identified by composite `(antenna_id, feed_id)` pairs.

use serde::{Deserialize, Serialize};

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
/// # Examples
///
/// ```
/// # use antenna_model::api::schemas::Position3D;
/// // ECEF coordinates (meters) - exceeds 6400 km threshold
/// let ecef = Position3D::new(6500000.0, 100000.0, 200000.0);
/// assert_eq!(ecef.coordinate_system(), "ECEF");
///
/// // Geodetic coordinates (lon, lat degrees, alt meters)
/// let geodetic = Position3D::new(-118.1234, 34.5678, 100.0);
/// assert_eq!(geodetic.coordinate_system(), "Geodetic");
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Position3D {
    /// X coordinate: ECEF X (meters) OR longitude (degrees)
    pub x: f64,
    /// Y coordinate: ECEF Y (meters) OR latitude (degrees)
    pub y: f64,
    /// Z coordinate: ECEF Z (meters) OR altitude (meters)
    pub z: f64,
}

impl Position3D {
    /// Threshold for ECEF detection (6400 km in meters)
    const ECEF_THRESHOLD_M: f64 = 6_400_000.0;

    /// Create a new Position3D
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Detect coordinate system based on magnitude.
    ///
    /// Returns "ECEF" if any coordinate exceeds 6400 km, otherwise "Geodetic".
    pub fn coordinate_system(&self) -> &'static str {
        if self.x.abs() > Self::ECEF_THRESHOLD_M
            || self.y.abs() > Self::ECEF_THRESHOLD_M
            || self.z.abs() > Self::ECEF_THRESHOLD_M
        {
            "ECEF"
        } else {
            "Geodetic"
        }
    }

    /// Check if this is likely ECEF coordinates
    pub fn is_ecef(&self) -> bool {
        self.coordinate_system() == "ECEF"
    }

    /// Check if this is likely Geodetic coordinates
    pub fn is_geodetic(&self) -> bool {
        self.coordinate_system() == "Geodetic"
    }
}

/// Quaternion representation of attitude/orientation.
///
/// Quaternion format: q = w + xi + yj + zk
/// Should be normalized: |q| = 1
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Quaternion {
    /// W component (scalar part)
    pub w: f64,
    /// X component (i)
    pub x: f64,
    /// Y component (j)
    pub y: f64,
    /// Z component (k)
    pub z: f64,
}

impl Quaternion {
    /// Create a new quaternion
    pub fn new(w: f64, x: f64, y: f64, z: f64) -> Self {
        Self { w, x, y, z }
    }

    /// Compute the magnitude of the quaternion
    pub fn magnitude(&self) -> f64 {
        (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    /// Check if quaternion is normalized (within tolerance)
    pub fn is_normalized(&self, tolerance: f64) -> bool {
        (self.magnitude() - 1.0).abs() < tolerance
    }

    /// Identity quaternion (no rotation)
    pub fn identity() -> Self {
        Self::new(1.0, 0.0, 0.0, 0.0)
    }
}

/// Euler angles representation of attitude/orientation.
///
/// Convention: Roll-Pitch-Yaw (X-Y-Z rotation sequence)
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct EulerAngles {
    /// Roll angle in degrees (rotation about X axis)
    pub roll_deg: f64,
    /// Pitch angle in degrees (rotation about Y axis)
    pub pitch_deg: f64,
    /// Yaw angle in degrees (rotation about Z axis)
    pub yaw_deg: f64,
}

impl EulerAngles {
    /// Create new Euler angles
    pub fn new(roll_deg: f64, pitch_deg: f64, yaw_deg: f64) -> Self {
        Self {
            roll_deg,
            pitch_deg,
            yaw_deg,
        }
    }

    /// Zero rotation (no rotation)
    pub fn zero() -> Self {
        Self::new(0.0, 0.0, 0.0)
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
/// feed, and emitter, along with vehicle attitude and operating frequency.
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
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GainRequest {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier (for multi-feed antennas)
    pub feed_id: String,

    /// Vehicle position (ECEF or Geodetic, auto-detected)
    pub vehicle_position: Position3D,

    /// Vehicle attitude (quaternion or Euler angles)
    pub vehicle_attitude: Attitude,

    /// Reflector boresight position (ECEF or Geodetic)
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

/// Vehicle attitude (quaternion or Euler angles).
///
/// Use either quaternion OR Euler angles, not both.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Attitude {
    /// Quaternion representation
    Quaternion(Quaternion),
    /// Euler angles representation
    EulerAngles(EulerAngles),
}

/// Response from antenna gain computation.
///
/// Contains computed gain, optional reference gain and loss, geometry information,
/// warnings, and performance metadata.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GainResponse {
    /// Antenna identifier
    pub antenna_id: String,

    /// Feed identifier
    pub feed_id: String,

    /// Computed gain in dB
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
}

/// Computed geometry information.
///
/// Details about the geometric configuration computed from 3D positions.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GeometryInfo {
    /// Feed offset from reflector boresight in meters
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

    /// Vehicle attitude
    pub vehicle_attitude: Attitude,

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

    /// Root mean squared error in dB
    pub rmse_db: f64,

    /// R² correlation coefficient
    pub r_squared: f64,

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

    /// Create invalid attitude error
    pub fn invalid_attitude(reason: impl Into<String>) -> Self {
        Self::new("InvalidAttitude", reason.into())
    }

    /// Create coordinate transform error
    pub fn coordinate_transform_error(details: impl Into<String>) -> Self {
        Self::new("CoordinateTransformError", "Coordinate transformation failed")
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
        // ECEF coordinates (large magnitude - exceeds 6400 km threshold)
        let ecef = Position3D::new(6500000.0, 100000.0, 200000.0);
        assert_eq!(ecef.coordinate_system(), "ECEF");
        assert!(ecef.is_ecef());
        assert!(!ecef.is_geodetic());

        // Another ECEF example
        let ecef2 = Position3D::new(100000.0, 6500000.0, 200000.0);
        assert_eq!(ecef2.coordinate_system(), "ECEF");

        // And another (Z coordinate exceeds threshold)
        let ecef3 = Position3D::new(100000.0, 200000.0, 6500000.0);
        assert_eq!(ecef3.coordinate_system(), "ECEF");
    }

    #[test]
    fn test_position3d_geodetic_detection() {
        // Geodetic coordinates (small magnitude)
        let geodetic = Position3D::new(-118.1234, 34.5678, 100.0);
        assert_eq!(geodetic.coordinate_system(), "Geodetic");
        assert!(!geodetic.is_ecef());
        assert!(geodetic.is_geodetic());
    }

    #[test]
    fn test_position3d_boundary_detection() {
        // Just below threshold - should be Geodetic
        let below = Position3D::new(6_399_999.0, 0.0, 0.0);
        assert_eq!(below.coordinate_system(), "Geodetic");

        // Just above threshold - should be ECEF
        let above = Position3D::new(6_400_001.0, 0.0, 0.0);
        assert_eq!(above.coordinate_system(), "ECEF");

        // Negative coordinates
        let negative = Position3D::new(-6_400_001.0, 0.0, 0.0);
        assert_eq!(negative.coordinate_system(), "ECEF");
    }

    #[test]
    fn test_position3d_serialization() {
        let pos = Position3D::new(1.0, 2.0, 3.0);
        let json = serde_json::to_string(&pos).unwrap();
        let deserialized: Position3D = serde_json::from_str(&json).unwrap();
        assert_eq!(pos, deserialized);
    }

    // ========================================================================
    // Quaternion Tests
    // ========================================================================

    #[test]
    fn test_quaternion_magnitude() {
        let q = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        assert_eq!(q.magnitude(), 1.0);

        let q2 = Quaternion::new(0.5, 0.5, 0.5, 0.5);
        assert!((q2.magnitude() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_quaternion_normalization_check() {
        let normalized = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        assert!(normalized.is_normalized(0.01));

        let not_normalized = Quaternion::new(2.0, 0.0, 0.0, 0.0);
        assert!(!not_normalized.is_normalized(0.01));
    }

    #[test]
    fn test_quaternion_identity() {
        let id = Quaternion::identity();
        assert_eq!(id.w, 1.0);
        assert_eq!(id.x, 0.0);
        assert_eq!(id.y, 0.0);
        assert_eq!(id.z, 0.0);
    }

    #[test]
    fn test_quaternion_serialization() {
        let q = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let json = serde_json::to_string(&q).unwrap();
        let deserialized: Quaternion = serde_json::from_str(&json).unwrap();
        assert_eq!(q, deserialized);
    }

    // ========================================================================
    // EulerAngles Tests
    // ========================================================================

    #[test]
    fn test_euler_angles_zero() {
        let zero = EulerAngles::zero();
        assert_eq!(zero.roll_deg, 0.0);
        assert_eq!(zero.pitch_deg, 0.0);
        assert_eq!(zero.yaw_deg, 0.0);
    }

    #[test]
    fn test_euler_angles_serialization() {
        let angles = EulerAngles::new(10.0, 20.0, 30.0);
        let json = serde_json::to_string(&angles).unwrap();
        let deserialized: EulerAngles = serde_json::from_str(&json).unwrap();
        assert_eq!(angles, deserialized);
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
            vehicle_position: Position3D::new(4510731.123, 4510731.456, 3488865.789),
            vehicle_attitude: Attitude::Quaternion(Quaternion::identity()),
            reflector_boresight: Position3D::new(4510732.0, 4510732.0, 3488950.0),
            feed_position: Position3D::new(4510731.5, 4510731.5, 3488870.0),
            emitter_position: Position3D::new(4520000.0, 4520000.0, 3500000.0),
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
            vehicle_attitude: Attitude::EulerAngles(EulerAngles::new(0.0, 5.0, 180.0)),
            reflector_boresight: Position3D::new(-117.0, 35.0, 400000.0),
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
            vehicle_attitude: Attitude::Quaternion(Quaternion::identity()),
            reflector_boresight: Position3D::new(0.0, 0.0, 0.0),
            feed_position: Position3D::new(0.0, 0.0, 0.0),
            emitter_position: Position3D::new(0.0, 0.0, 0.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
        };

        let json = serde_json::to_string(&request).unwrap();

        // Check field naming
        assert!(json.contains("\"antenna_id\""));
        assert!(json.contains("\"feed_id\""));
        assert!(json.contains("\"vehicle_position\""));
        assert!(json.contains("\"vehicle_attitude\""));
        assert!(json.contains("\"reflector_boresight\""));
        assert!(json.contains("\"feed_position\""));
        assert!(json.contains("\"emitter_position\""));
        assert!(json.contains("\"frequency_mhz\""));
        assert!(json.contains("\"include_reference\""));
    }
}
