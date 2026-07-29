//! API request and response schemas
//!
//! This module defines the data structures for API requests and responses,
//! all using serde for JSON serialization/deserialization.
//!
//! # 3D Coordinate System Support
//!
//! Every 3D position states its frame explicitly via the required `coordinate_system`
//! field:
//! - **ECEF** (Earth-Centered Earth-Fixed): x, y, z in meters from Earth's centre
//! - **Geodetic**: x = longitude degrees, y = latitude degrees, z = altitude meters
//!
//! There is no magnitude-based auto-detection (removed by roadmap unit C8 stage 2): a
//! request that omits the tag is rejected rather than guessed at.
//!
//! # Multi-Feed Support
//!
//! Antennas can have multiple feeds, identified by composite `(antenna_id, feed_id)` pairs.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::data::types::{CalibrationCoverage, CalibrationStatus};

/// The response-warning vocabulary (roadmap unit C8 stage 3).
///
/// Defined in [`crate::warnings`] rather than here because the model layer
/// produces warnings too and does not otherwise depend on the API layer — the
/// same reason [`crate::error`] is a root module. Re-exported so the wire types
/// a client cares about all resolve under `api::schemas`.
pub use crate::warnings::{ApiWarning, WarningCode};

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

/// 3D position in an explicitly named coordinate system.
///
/// Supports two coordinate systems, selected by the **required** `coordinate_system`
/// field:
/// - **ECEF** (Earth-Centered Earth-Fixed), `"ecef"`
///   - x, y, z in meters from Earth's centre of mass
/// - **Geodetic** (WGS84), `"geodetic"`
///   - x = longitude in degrees (-180 to 180)
///   - y = latitude in degrees (-90 to 90)
///   - z = altitude in meters (above the WGS84 ellipsoid)
///
/// # Why the tag is required
///
/// Until roadmap unit C8 stage 2 the field was optional and the frame was inferred from
/// coordinate magnitude (ECEF above a 6400 km threshold, geodetic below). That heuristic
/// is not decidable: a geodetic GEO satellite at `z = 35,786,000` m and an ECEF position
/// are indistinguishable by magnitude, so untagged GEO positions silently misparsed as
/// near-Earth-centre ECEF and returned a confidently wrong gain. The frame is now stated,
/// never guessed — a body that omits `coordinate_system` is rejected with a 400 naming the
/// field.
///
/// # Examples
///
/// ```
/// # use antenna_model::api::schemas::{CoordinateSystem, Position3D};
/// // ECEF, meters from Earth's centre
/// let ecef = Position3D::ecef(6_500_000.0, 0.0, 0.0);
/// assert_eq!(ecef.coordinate_system, CoordinateSystem::ECEF);
/// assert!(ecef.is_ecef());
///
/// // Earth-surface ECEF is stated, not inferred from its magnitude
/// let ecef_surface = Position3D::ecef(6_378_137.0, 0.0, 100_000.0);
/// assert!(ecef_surface.is_ecef());
///
/// // Geodetic (lon, lat degrees, alt meters)
/// let geodetic = Position3D::geodetic(-118.1234, 34.5678, 100.0);
/// assert_eq!(geodetic.coordinate_system, CoordinateSystem::Geodetic);
///
/// // A GEO satellite's altitude no longer competes with the ECEF threshold
/// let geo = Position3D::geodetic(0.0, 0.0, 35_786_000.0);
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
    /// Required frame tag naming how to read `x`, `y`, `z`. There is no default and no
    /// magnitude-based inference; omitting it is a deserialization error.
    pub coordinate_system: CoordinateSystem,
}

impl Position3D {
    /// Create an ECEF position (x, y, z in meters from Earth's centre).
    pub fn ecef(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            coordinate_system: CoordinateSystem::ECEF,
        }
    }

    /// Create a geodetic (WGS84) position: longitude °E, latitude °N, altitude meters.
    pub fn geodetic(longitude_deg: f64, latitude_deg: f64, altitude_m: f64) -> Self {
        Self {
            x: longitude_deg,
            y: latitude_deg,
            z: altitude_m,
            coordinate_system: CoordinateSystem::Geodetic,
        }
    }

    /// Check if this position uses ECEF coordinates
    pub fn is_ecef(&self) -> bool {
        self.coordinate_system == CoordinateSystem::ECEF
    }

    /// Check if this position uses Geodetic coordinates
    pub fn is_geodetic(&self) -> bool {
        self.coordinate_system == CoordinateSystem::Geodetic
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

    /// Vehicle position (ECEF or Geodetic, per its `coordinate_system`)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    ///
    /// This position, together with `vehicle_position`, establishes the dish
    /// pointing direction. The vector from vehicle to boresight defines the
    /// antenna Z-axis.
    pub reflector_boresight: Position3D,

    /// Feed pointing target (ECEF or Geodetic).
    ///
    /// **This is the Earth location the feed's beam is aimed at — NOT the
    /// feed's physical location on the antenna.** The service converts the
    /// angular offset between this aim point and `reflector_boresight` into a
    /// physical feed displacement in the antenna frame (including the beam
    /// deviation factor). To model an unsteered (focused) feed, set this equal
    /// to `reflector_boresight`.
    pub feed_pointing_location: Position3D,

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

    /// Optional vehicle attitude as a unit quaternion `[w, x, y, z]` (body → ECEF).
    ///
    /// Body axes convention: body +Z = antenna boresight direction, body +X = azimuth-zero
    /// (E-clock zero) reference. When supplied, the antenna-frame X-axis is derived from
    /// body +X rotated into ECEF and projected perpendicular to the boresight, giving a
    /// **deterministic, calibration-consistent** azimuth reference.
    ///
    /// When omitted, azimuth zero is derived from the Earth-Z / East cross-product heuristic
    /// (approximate and discontinuous near boresight ∥ Earth-Z); see
    /// `compute_emitter_direction` for details.
    ///
    /// The quaternion must be normalised to unit length (norm within 1e-3 of 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle_attitude: Option<[f64; 4]>,
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

    /// Why this evaluation produced no gain, when `gain_db` is `null`.
    ///
    /// Present **only** on a failed item inside a `/api/v1/gain/batch` response:
    /// `/api/v1/gain` reports a failure as an HTTP error instead, so a 200 body
    /// from it never carries this field. Absent on every successful evaluation.
    ///
    /// Introduced by roadmap unit **C8 stage 3**, closing the hazard unit C2
    /// recorded: a failed batch item used to be a `gain_db: null` plus a
    /// `"Computation failed: …"` string in `warnings`, so a client that did not
    /// inspect every item's prose could not tell a failure from a quality caveat.
    /// `code` is one of [`error_codes::ALL`] — the same vocabulary the HTTP error
    /// bodies use, so the *reason* survives (a timed-out item reports
    /// `computation_budget_exceeded`, not a generic failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<GainError>,

    /// Warnings about result quality (extrapolation, off-axis validity, …).
    ///
    /// Each entry carries a stable [`WarningCode`] and a human-readable message;
    /// branch on `code`, display `message`. See [`crate::warnings`] for the
    /// vocabulary and its stability contract.
    pub warnings: Vec<ApiWarning>,

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
    /// This is the **total** offset actually used for this request: the antenna's
    /// static design offset (reported as `FeedInfo.design_feed_offset_m`) plus the
    /// displacement induced by steering the beam to `feed_pointing_location`.
    /// It is a physical displacement in the antenna frame — *not* an Earth
    /// location, and not to be confused with `feed_pointing_location`.
    ///
    /// `x` and `y` are the lateral displacement of the feed from the optical axis;
    /// `z` is the axial displacement from the focal point (**positive = away from
    /// the reflector vertex**, matching the phase model's `delta_z` convention).
    /// For an on-axis (boresight-aimed) feed all three components are ~zero.
    pub physical_feed_offset_m: Vector3D,

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

    /// Physical spillover loss folded into `gain_db`, in dB (a small **negative**
    /// value). `null` when physical spillover was NOT applied — i.e. the antenna
    /// has a correction surface (which absorbs spillover empirically). Present only
    /// on the uncalibrated path, so consumers can tell which model variant produced
    /// the number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spillover_loss_db: Option<f64>,
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

    /// Feed pointing target (ECEF or Geodetic).
    ///
    /// **This is the Earth location the feed's beam is aimed at — NOT the
    /// feed's physical location on the antenna.** The service converts the
    /// angular offset between this aim point and `reflector_boresight` into a
    /// physical feed displacement in the antenna frame (including the beam
    /// deviation factor). To model an unsteered (focused) feed, set this equal
    /// to `reflector_boresight`.
    pub feed_pointing_location: Position3D,

    /// Operating frequency in MHz
    pub frequency_mhz: f64,

    /// Pointing frequency in MHz (for beam squint correction, optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointing_frequency_mhz: Option<f64>,

    /// Grid configuration (rectangular)
    pub grid_config: GridConfig,
}

/// Grid configuration for heatmap generation.
///
/// A **single-variant tagged enum by design**: `grid_type` stays in the wire contract so a
/// second grid family can be added later without a breaking change (feature F5 would merge
/// `/api/v1/h3-heatmap` back in here). Do not collapse this into a plain struct.
///
/// The `H3` variant that lived here until C8 stage 4 (2026-07-28) was a `NotImplemented`
/// stub — it parsed and validated, then failed. The real H3 grid is the separate
/// `POST /api/v1/h3-heatmap` endpoint. An `h3` tag is now an unknown variant → 400.
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

    /// Grid data (rectangular)
    pub grid: GridData,

    /// Aggregated, deduplicated warnings across all grid points.
    ///
    /// Each entry carries a stable [`WarningCode`] and a human-readable message.
    /// Deduplication is on the whole warning (code **and** message), so a warning
    /// whose message varies per point would appear once per point — producers of
    /// grid-safe warnings keep their messages constant per (antenna, frequency).
    pub warnings: Vec<ApiWarning>,

    /// Heatmap metadata
    pub metadata: HeatmapMetadata,

    /// Calibration status and accuracy information (v2.0)
    /// Optional for backward compatibility - will be populated by service layer in Task 6.8
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,
}

/// Grid data for heatmap.
///
/// Single-variant tagged enum for the same reason as [`GridConfig`] — `grid_type` stays on
/// the wire. The `H3` variant was removed by C8 stage 4 (2026-07-28).
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
}

/// Heatmap computation metadata.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct HeatmapMetadata {
    /// Number of grid points evaluated
    pub points_evaluated: usize,

    /// Total computation time in milliseconds
    pub computation_time_ms: f64,

    /// Highest gain in dB over the grid points (or H3 cells) that evaluated successfully.
    /// This is the reference every `loss_db` in the response is measured against.
    ///
    /// When *no* point evaluated successfully there is no peak, and the sentinel
    /// `-999999.0` is reported (never `null`); `failed_points` then equals
    /// `points_evaluated`.
    pub peak_gain_db: f64,

    /// Number of grid points that failed to compute (loss replaced with sentinel 999999.0).
    /// On `/api/v1/h3-heatmap` a failed cell is omitted from `cells` entirely, so this is
    /// the only place it is reported.
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

    /// Vehicle position (ECEF or Geodetic, per its `coordinate_system`)
    pub vehicle_position: Position3D,

    /// Reflector boresight position (ECEF or Geodetic)
    pub reflector_boresight: Position3D,

    /// Feed pointing target (ECEF or Geodetic).
    ///
    /// **This is the Earth location the feed's beam is aimed at — NOT the
    /// feed's physical location on the antenna.** The service converts the
    /// angular offset between this aim point and `reflector_boresight` into a
    /// physical feed displacement in the antenna frame (including the beam
    /// deviation factor). To model an unsteered (focused) feed, set this equal
    /// to `reflector_boresight`.
    pub feed_pointing_location: Position3D,

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

    /// Optional vehicle attitude as a unit quaternion `[w, x, y, z]` (body → ECEF).
    ///
    /// Body axes convention: body +Z = antenna boresight direction, body +X = azimuth-zero
    /// (E-clock zero) reference. When supplied, the antenna-frame X-axis is derived from
    /// body +X rotated into ECEF and projected perpendicular to the boresight, giving a
    /// **deterministic, calibration-consistent** azimuth reference.
    ///
    /// When omitted, azimuth zero is derived from the Earth-Z / East cross-product heuristic
    /// (approximate and discontinuous near boresight ∥ Earth-Z); see
    /// `compute_emitter_direction` for details.
    ///
    /// The quaternion must be normalised to unit length (norm within 1e-3 of 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle_attitude: Option<[f64; 4]>,
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

    /// Gain relative to the grid peak, in dB — always ≥ 0, and exactly 0 at the peak cell.
    ///
    /// Computed as `metadata.peak_gain_db - gain_db`, where `peak_gain_db` is the highest
    /// gain over the cells **actually evaluated** (roadmap C9). This is the same rule
    /// `/api/v1/heatmap` applies, so the field means one thing on both heatmap endpoints,
    /// and the response is internally re-derivable from the values it reports.
    ///
    /// The peak of the *grid* is not necessarily the peak of the *beam*: a grid that does
    /// not contain the beam peak understates loss for every cell in it.
    ///
    /// Both gains are on the same basis by construction — the reference *is* one of the
    /// cells. A grid can still straddle two bases where it leaves calibration coverage
    /// (in-coverage cells corrected, out-of-coverage cells physics-only), as on
    /// `/api/v1/heatmap`.
    pub loss_db: f64,

    /// Free-space path loss in dB
    pub free_space_path_loss_db: f64,

    /// Total path loss in dB: `free_space_path_loss_db + loss_db`. Since `loss_db` is
    /// referenced to the grid peak it is never negative, so this is never below the
    /// free-space path loss.
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

    /// Aggregated, deduplicated warnings across all cells.
    ///
    /// Each entry carries a stable [`WarningCode`] and a human-readable message;
    /// see [`HeatmapResponse::warnings`] for the deduplication rule.
    pub warnings: Vec<ApiWarning>,

    /// Computation metadata
    pub metadata: HeatmapMetadata,

    /// Calibration status and accuracy information (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calibration_status: Option<CalibrationStatusInfo>,

    /// Beam squint magnitude applied (degrees), when the pointing frequency differs from
    /// the operating frequency. Omitted when no squint is applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beam_squint_deg: Option<f64>,
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

    /// The feed's **design** offset from the focal point in the antenna frame
    /// (meters) — a static property of this antenna's configuration, identical
    /// for every request. It is a physical displacement, *not* an Earth location:
    /// it is not the aim point `feed_pointing_location`. The per-request total
    /// (design offset + beam-steering displacement) is reported as
    /// `GeometryInfo.physical_feed_offset_m`.
    pub design_feed_offset_m: Vector3D,

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

    /// Create a degraded response.
    ///
    /// The service is alive and responding, but has no calibration data loaded, so it
    /// cannot answer gain requests. Deliberately still served with HTTP 200: `/health` is
    /// the Kubernetes **liveness** probe, and a restart does not fix missing calibration
    /// data — returning non-200 here would produce an endless CrashLoopBackOff. Readiness
    /// (`/ready`) is the signal that keeps traffic away. See roadmap S5.
    pub fn degraded() -> Self {
        Self {
            status: "degraded".to_string(),
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

/// The canonical error-code vocabulary served in `ErrorResponse.error`.
///
/// Every error body the service emits carries one of these codes, and nothing
/// else. Emission sites reference these constants rather than repeating string
/// literals, so a typo is a compile error instead of a new undocumented code on
/// the wire (roadmap unit C3).
///
/// The set is mirrored in two places that are **not** compiler-checked — keep
/// them in step when adding a code:
///
/// - `openapi.yaml` (the `ErrorResponse` schema's `error` enum), and
/// - `docs/api-documentation.md` (the error-code table).
///
/// Codes are `snake_case`. An earlier `PascalCase` set existed only as unused
/// `ErrorResponse` convenience constructors and never reached the wire; it was
/// deleted in C3.
///
/// The status noted on each code is the one it always carries. Which status a given
/// *error* gets is decided in `api::error_response`
/// (`validation_status` / `service_status`), not here — this module owns the names.
pub mod error_codes {
    /// The named antenna does not exist in the calibration repository (404).
    pub const ANTENNA_NOT_FOUND: &str = "antenna_not_found";

    /// The antenna exists but the named feed does not (404).
    pub const FEED_NOT_FOUND: &str = "feed_not_found";

    /// The request deserialized but is semantically invalid (422, from either the
    /// pre-check or the service layer — roadmap C2 made the two agree).
    pub const VALIDATION_ERROR: &str = "validation_error";

    /// A position or coordinate value is out of range or untransformable (422).
    pub const INVALID_COORDINATE: &str = "invalid_coordinate";

    /// The request body could not be read or parsed (400).
    pub const INVALID_REQUEST_BODY: &str = "invalid_request_body";

    /// A requested option is recognized but unimplemented — currently only the
    /// `/heatmap` H3 grid-type stub, which C8 stage 4 removes (422).
    pub const NOT_IMPLEMENTED: &str = "not_implemented";

    /// The request body exceeds `server.max_body_size_bytes` (413).
    pub const PAYLOAD_TOO_LARGE: &str = "payload_too_large";

    /// The request exceeded `server.request_timeout_secs` (504).
    pub const REQUEST_TIMEOUT: &str = "request_timeout";

    /// A single aperture integration exceeded
    /// `performance.integration_budget_ms` (504).
    pub const COMPUTATION_BUDGET_EXCEEDED: &str = "computation_budget_exceeded";

    /// Admission control rejected the request; a `Retry-After` header
    /// accompanies it (503).
    pub const SERVICE_OVERLOADED: &str = "service_overloaded";

    /// An unexpected server-side failure (500).
    pub const INTERNAL_ERROR: &str = "internal_error";

    /// Every code above, in the order documented. Used by the drift test and
    /// available to consumers that need to enumerate the vocabulary.
    pub const ALL: &[&str] = &[
        ANTENNA_NOT_FOUND,
        FEED_NOT_FOUND,
        VALIDATION_ERROR,
        INVALID_COORDINATE,
        INVALID_REQUEST_BODY,
        NOT_IMPLEMENTED,
        PAYLOAD_TOO_LARGE,
        REQUEST_TIMEOUT,
        COMPUTATION_BUDGET_EXCEEDED,
        SERVICE_OVERLOADED,
        INTERNAL_ERROR,
    ];
}

/// Why a single evaluation inside a batch produced no gain.
///
/// Carried by [`GainResponse::error`]; see that field for when it is present.
/// Structurally a two-field subset of [`ErrorResponse`] — same `code` vocabulary
/// ([`error_codes::ALL`]), no `field`/`details`, because a batch item failure is
/// reported per item rather than as the HTTP outcome.
///
/// The field is named `code` (not `error`, as in [`ErrorResponse`]) because it sits
/// inside a member already named `error`; `"error": {"error": …}` reads as a
/// mistake.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct GainError {
    /// Machine-readable failure class, one of [`error_codes::ALL`].
    pub code: String,

    /// Human-readable explanation.
    pub message: String,
}

impl GainError {
    /// Create a per-item error from a code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Standardized error response.
///
/// Returned for all error conditions with appropriate HTTP status codes.
/// The `error` field always carries one of [`error_codes::ALL`].
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Position3D Tests
    // ========================================================================

    // The tests below replace the pre-C8-stage-2 auto-detection suite
    // (`test_position3d_{ecef,geodetic,boundary}_detection`,
    // `test_detection_threshold_is_6400km`,
    // `test_explicit_coordinate_system_overrides_detection`,
    // `test_position3d_backward_compatible_deserialization`,
    // `test_position3d_no_coordinate_system_not_serialized`). They assert the required-field
    // contract that took the heuristic's place rather than being dropped: the frame is
    // whatever the caller declared, at any magnitude, in both directions.

    #[test]
    fn the_declared_frame_is_the_frame_at_any_magnitude() {
        // Values that the old 6400 km heuristic would have classified as ECEF...
        let big_geodetic = Position3D::geodetic(6_500_000.0, 0.0, 35_786_000.0);
        assert!(big_geodetic.is_geodetic());
        assert!(!big_geodetic.is_ecef());

        // ...and values it would have classified as geodetic.
        let small_ecef = Position3D::ecef(100.0, 100.0, 100.0);
        assert!(small_ecef.is_ecef());
        assert!(!small_ecef.is_geodetic());

        // Earth-surface ECEF (6378 km equatorial radius) sat just under the old threshold
        // and was the heuristic's most common misclassification. It is now unremarkable.
        assert!(Position3D::ecef(6_378_137.0, 0.0, 0.0).is_ecef());
        assert_eq!(
            Position3D::ecef(-2_500_000.0, -4_500_000.0, 3_600_000.0).coordinate_system,
            CoordinateSystem::ECEF
        );
    }

    #[test]
    fn a_position_without_a_coordinate_system_is_rejected() {
        // Was `test_position3d_backward_compatible_deserialization`, which asserted the
        // opposite. C8 stage 2 is the sanctioned break: there is no default, so an
        // untagged body fails to parse instead of being guessed at.
        let json = r#"{"x":1.0,"y":2.0,"z":3.0}"#;
        let err = serde_json::from_str::<Position3D>(json)
            .expect_err("an untagged position must not deserialize");
        assert!(
            err.to_string().contains("coordinate_system"),
            "the parse error must name the missing field, got: {err}"
        );
    }

    #[test]
    fn test_position3d_serialization() {
        for pos in [
            Position3D::geodetic(1.0, 2.0, 3.0),
            Position3D::ecef(1.0, 2.0, 3.0),
        ] {
            let json = serde_json::to_string(&pos).unwrap();
            // The tag is always on the wire — it is no longer skipped when absent,
            // because it can no longer be absent.
            assert!(
                json.contains("coordinate_system"),
                "serialized position must carry its frame: {json}"
            );
            let deserialized: Position3D = serde_json::from_str(&json).unwrap();
            assert_eq!(pos, deserialized);
        }
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
            vehicle_position: Position3D::ecef(4510731.123, 4510731.456, 3488865.789),
            reflector_boresight: Position3D::ecef(4510732.0, 4510732.0, 3488950.0),
            feed_pointing_location: Position3D::ecef(4510731.5, 4510731.5, 3488870.0),
            emitter_position: Position3D::ecef(4520000.0, 4520000.0, 3500000.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: Some(8450.0),
            include_reference: true,
            vehicle_attitude: None,
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
            vehicle_position: Position3D::geodetic(-118.1234, 34.5678, 100.0),
            reflector_boresight: Position3D::geodetic(-118.1234, 34.5679, 110.0), // 10m above vehicle
            feed_pointing_location: Position3D::geodetic(-118.124, 34.568, 105.0),
            emitter_position: Position3D::geodetic(-117.0, 35.0, 400000.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
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
    fn h3_grid_type_is_rejected_as_an_unknown_variant() {
        // The `h3` grid type on /api/v1/heatmap was a NotImplemented stub until C8
        // stage 4 removed it; the real H3 grid is the separate POST /api/v1/h3-heatmap
        // endpoint. An `h3` tag is now an unknown variant — i.e. a body that cannot be
        // parsed, which under roadmap C2's policy is a 400, not a 422.
        let json = r#"{"grid_type":"h3","h3_resolution":7,"center_azimuth_deg":180.0,"center_elevation_deg":45.0,"field_of_view_deg":30.0}"#;

        let err = serde_json::from_str::<GridConfig>(json)
            .expect_err("an `h3` grid_type must not deserialize");

        assert!(
            err.to_string().contains("unknown variant"),
            "expected an unknown-variant parse error, got: {err}"
        );
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

    #[test]
    fn test_health_response_degraded() {
        // Shape is unchanged (one `status` field) — only the value differs. S5 explicitly
        // requires keeping the /health response shape.
        let response = HealthResponse::degraded();
        assert_eq!(response.status, "degraded");

        let json = serde_json::to_value(&response).expect("serialize");
        assert_eq!(json["status"], "degraded");
        assert_eq!(
            json.as_object().map(|o| o.len()),
            Some(1),
            "HealthResponse must stay a single-field object"
        );
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

    /// The vocabulary is `snake_case`, unique, and non-empty.
    ///
    /// The PascalCase constructors this replaced (`ErrorResponse::antenna_not_found`
    /// and friends, emitting `"AntennaNotFound"`) were deleted in C3 — they had zero
    /// callers, so the PascalCase codes they document never reached the wire.
    #[test]
    fn error_code_vocabulary_is_snake_case_and_unique() {
        use std::collections::HashSet;

        for code in error_codes::ALL {
            assert!(!code.is_empty(), "error code must not be empty");
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "error code {code:?} is not snake_case"
            );
        }

        let unique: HashSet<_> = error_codes::ALL.iter().collect();
        assert_eq!(
            unique.len(),
            error_codes::ALL.len(),
            "error_codes::ALL contains a duplicate"
        );
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
            vehicle_position: Position3D::geodetic(0.0, 0.0, 0.0),
            reflector_boresight: Position3D::geodetic(0.0, 0.0, 10.0), // 10m above vehicle
            feed_pointing_location: Position3D::geodetic(0.0, 0.0, 23.6), // 10m + 13.6m focal length
            emitter_position: Position3D::geodetic(100.0, 100.0, 100.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        };

        let json = serde_json::to_string(&request).unwrap();

        // Check field naming
        assert!(json.contains("\"antenna_id\""));
        assert!(json.contains("\"feed_id\""));
        assert!(json.contains("\"vehicle_position\""));
        assert!(json.contains("\"reflector_boresight\""));
        assert!(json.contains("\"feed_pointing_location\""));
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
        assert!(!info.correction_applied);
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
        assert!(!info.correction_applied);
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
        assert!(!info.correction_applied);
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
                physical_feed_offset_m: Vector3D::new(0.0, 0.0, 0.1),
                emitter_azimuth_deg: 10.0,
                emitter_elevation_deg: 45.0,
                beam_squint_deg: None,
            },
            error: None,
            warnings: vec![],
            metadata: ComputationMetadata {
                computation_time_ms: 50.0,
                coordinate_transform_ms: Some(10.0),
                physics_model_ms: Some(30.0),
                correction_surface_ms: Some(5.0),
                extrapolated: false,
                spillover_loss_db: None,
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
                "physical_feed_offset_m": {"x": 0.0, "y": 0.0, "z": 0.1},
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
            vehicle_position: Position3D::ecef(4510731.0, 4510731.0, 3488865.0),
            reflector_boresight: Position3D::ecef(4510732.0, 4510732.0, 3488950.0),
            feed_pointing_location: Position3D::ecef(4510731.5, 4510731.5, 3488870.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: Some(8450.0),
            n_rings: 3,
            h3_resolution: Some(7),
            temperature_k: Some(290.0),
            vehicle_attitude: None,
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
