//! Input Validation Layer
//!
//! Provides comprehensive validation for all API request types,
//! including 3D coordinates, frequencies, and grid configurations.
//!
//! # Validation Rules
//!
//! - **Position Validation**:
//!   - ECEF: |x|, |y|, |z| < 400,000 km (allows HEO satellites)
//!   - Geodetic: lon [-180, 180], lat [-90, 90], alt < 400,000 km
//!   - NaN, Inf detection
//!
//! - **Frequency Validation**:
//!   - Operating frequency: [100, 50000] MHz (system spec)
//!   - Pointing frequency: same range if specified
//!
//! - **Composite Identifier Validation**:
//!   - Antenna ID exists in repository
//!   - Feed ID exists for specified antenna

use crate::api::schemas::{
    BatchGainRequest, CoordinateSystem, GainRequest, GridConfig, H3LinkBudgetRequest,
    HeatmapRequest, Position3D, RangeConfig,
};
use crate::data::repository::CalibrationRepository;
use crate::error::{ValidationError, ValidationResult};

// ============================================================================
// Constants
// ============================================================================

/// Maximum coordinate magnitude for ECEF (400,000 km in meters, allows HEO satellites)
const MAX_ECEF_MAGNITUDE_M: f64 = 400_000_000.0;

/// Maximum geodetic longitude (degrees)
const MAX_LONGITUDE_DEG: f64 = 180.0;

/// Minimum geodetic longitude (degrees)
const MIN_LONGITUDE_DEG: f64 = -180.0;

/// Maximum geodetic latitude (degrees)
const MAX_LATITUDE_DEG: f64 = 90.0;

/// Minimum geodetic latitude (degrees)
const MIN_LATITUDE_DEG: f64 = -90.0;

/// Maximum altitude (400,000 km in meters, allows HEO satellites)
const MAX_ALTITUDE_M: f64 = 400_000_000.0;

/// Minimum operating frequency (100 MHz per system spec)
const MIN_FREQUENCY_MHZ: f64 = 100.0;

/// Maximum operating frequency (50 GHz = 50,000 MHz per system spec)
const MAX_FREQUENCY_MHZ: f64 = 50_000.0;

/// Maximum batch size (shared with batch.rs to avoid divergence)
pub const MAX_BATCH_SIZE: usize = 1000;

/// Maximum heatmap grid points
const MAX_HEATMAP_POINTS: usize = 100_000;

// ============================================================================
// Public Validation Functions
// ============================================================================

/// Validate a gain computation request.
///
/// Checks all fields including positions, frequencies, and
/// verifies that the antenna/feed combination exists in the repository.
///
/// # Arguments
///
/// * `request` - The gain request to validate
/// * `repository` - Calibration repository to check antenna/feed existence
///
/// # Errors
///
/// Returns `ValidationError` if any validation check fails.
pub fn validate_gain_request(
    request: &GainRequest,
    repository: &CalibrationRepository,
) -> ValidationResult<()> {
    // Validate antenna/feed existence
    validate_antenna_feed_exists(&request.antenna_id, &request.feed_id, repository)?;

    // Validate all positions
    validate_position(&request.vehicle_position, "vehicle_position")?;
    validate_position(&request.reflector_boresight, "reflector_boresight")?;
    validate_position(&request.feed_position, "feed_position")?;
    validate_position(&request.emitter_position, "emitter_position")?;

    // Validate operating frequency
    validate_frequency(request.frequency_mhz, "frequency_mhz")?;

    // Validate pointing frequency if specified
    if let Some(pointing_freq) = request.pointing_frequency_mhz {
        validate_frequency(pointing_freq, "pointing_frequency_mhz")?;
    }

    // Validate vehicle attitude quaternion norm (must be unit quaternion)
    if let Some([w, x, y, z]) = request.vehicle_attitude {
        validate_quaternion_norm(w, x, y, z, "vehicle_attitude")?;
    }

    Ok(())
}

/// Validate a batch gain computation request.
///
/// Validates the batch size and each individual gain request.
///
/// # Arguments
///
/// * `request` - The batch request to validate
/// * `repository` - Calibration repository for antenna/feed validation
///
/// # Errors
///
/// Returns `ValidationError` if batch size exceeds limit or any
/// individual request is invalid.
pub fn validate_batch_gain_request(
    request: &BatchGainRequest,
    repository: &CalibrationRepository,
) -> ValidationResult<()> {
    // Check batch size limit
    if request.evaluations.len() > MAX_BATCH_SIZE {
        return Err(ValidationError::BatchSizeLimitExceeded {
            size: request.evaluations.len(),
            limit: MAX_BATCH_SIZE,
        });
    }

    if request.evaluations.is_empty() {
        return Err(ValidationError::InvalidValue {
            param: "evaluations".to_string(),
            reason: "batch must contain at least one evaluation".to_string(),
        });
    }

    // Validate each request in the batch
    for (i, gain_request) in request.evaluations.iter().enumerate() {
        validate_gain_request(gain_request, repository).map_err(|e| {
            ValidationError::InvalidValue {
                param: format!("evaluations[{}]", i),
                reason: e.to_string(),
            }
        })?;
    }

    Ok(())
}

/// Validate a heatmap generation request.
///
/// Validates positions, frequency, grid configuration, and
/// ensures the total number of grid points doesn't exceed limits.
///
/// # Arguments
///
/// * `request` - The heatmap request to validate
/// * `repository` - Calibration repository for antenna/feed validation
///
/// # Errors
///
/// Returns `ValidationError` if any validation check fails.
pub fn validate_heatmap_request(
    request: &HeatmapRequest,
    repository: &CalibrationRepository,
) -> ValidationResult<()> {
    // Validate antenna/feed existence
    validate_antenna_feed_exists(&request.antenna_id, &request.feed_id, repository)?;

    // Validate all positions
    validate_position(&request.vehicle_position, "vehicle_position")?;
    validate_position(&request.reflector_boresight, "reflector_boresight")?;
    validate_position(&request.feed_position, "feed_position")?;

    // Validate frequency
    validate_frequency(request.frequency_mhz, "frequency_mhz")?;

    // Validate pointing frequency if specified
    if let Some(pointing_freq) = request.pointing_frequency_mhz {
        validate_frequency(pointing_freq, "pointing_frequency_mhz")?;
    }

    // Validate grid configuration
    validate_grid_config(&request.grid_config)?;

    Ok(())
}

/// Validate an H3 link budget request.
///
/// Checks frequency positivity and n_rings limit.
///
/// # Arguments
///
/// * `req` - The H3 link budget request to validate
///
/// # Errors
///
/// Returns `ValidationError` if any validation check fails.
pub fn validate_h3_link_budget_request(req: &H3LinkBudgetRequest) -> ValidationResult<()> {
    // Validate all positions
    validate_position(&req.vehicle_position, "vehicle_position")?;
    validate_position(&req.reflector_boresight, "reflector_boresight")?;
    validate_position(&req.feed_position, "feed_position")?;

    // Validate frequency (handles NaN and enforces [100, 50000] MHz range)
    validate_frequency(req.frequency_mhz, "frequency_mhz")?;

    // n_rings limit: max 10
    if req.n_rings > 10 {
        return Err(ValidationError::InvalidValue {
            param: "n_rings".to_string(),
            reason: "n_rings must be ≤ 10".to_string(),
        });
    }

    // Validate vehicle attitude quaternion norm (must be unit quaternion)
    if let Some([w, x, y, z]) = req.vehicle_attitude {
        validate_quaternion_norm(w, x, y, z, "vehicle_attitude")?;
    }

    Ok(())
}

// ============================================================================
// Position Validation
// ============================================================================

/// Validate a 3D position (ECEF or Geodetic).
///
/// Automatically detects coordinate system and applies appropriate validation.
fn validate_position(position: &Position3D, param_name: &str) -> ValidationResult<()> {
    // Check for NaN or Inf
    if !position.x.is_finite() {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.x", param_name),
            reason: format!("value is not finite: {}", position.x),
        });
    }
    if !position.y.is_finite() {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.y", param_name),
            reason: format!("value is not finite: {}", position.y),
        });
    }
    if !position.z.is_finite() {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.z", param_name),
            reason: format!("value is not finite: {}", position.z),
        });
    }

    // Validate based on detected coordinate system
    if position.is_ecef() {
        validate_ecef_position(position, param_name)
    } else {
        validate_geodetic_position(position, param_name)
    }
}

/// Validate ECEF coordinates.
///
/// Ensures coordinates are within reasonable Earth vicinity (< 10,000 km from center).
fn validate_ecef_position(position: &Position3D, param_name: &str) -> ValidationResult<()> {
    if position.x.abs() > MAX_ECEF_MAGNITUDE_M {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.x", param_name),
            reason: format!(
                "ECEF X coordinate {} exceeds maximum {} m",
                position.x, MAX_ECEF_MAGNITUDE_M
            ),
        });
    }
    if position.y.abs() > MAX_ECEF_MAGNITUDE_M {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.y", param_name),
            reason: format!(
                "ECEF Y coordinate {} exceeds maximum {} m",
                position.y, MAX_ECEF_MAGNITUDE_M
            ),
        });
    }
    if position.z.abs() > MAX_ECEF_MAGNITUDE_M {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.z", param_name),
            reason: format!(
                "ECEF Z coordinate {} exceeds maximum {} m",
                position.z, MAX_ECEF_MAGNITUDE_M
            ),
        });
    }
    Ok(())
}

/// Validate Geodetic coordinates.
///
/// Ensures longitude, latitude, and altitude are within valid ranges.
fn validate_geodetic_position(position: &Position3D, param_name: &str) -> ValidationResult<()> {
    // Validate longitude (x)
    if position.x < MIN_LONGITUDE_DEG || position.x > MAX_LONGITUDE_DEG {
        return Err(ValidationError::AngleOutOfRange {
            angle_type: format!("{}.longitude", param_name),
            value: position.x,
            min: MIN_LONGITUDE_DEG,
            max: MAX_LONGITUDE_DEG,
        });
    }

    // Validate latitude (y)
    if position.y < MIN_LATITUDE_DEG || position.y > MAX_LATITUDE_DEG {
        return Err(ValidationError::AngleOutOfRange {
            angle_type: format!("{}.latitude", param_name),
            value: position.y,
            min: MIN_LATITUDE_DEG,
            max: MAX_LATITUDE_DEG,
        });
    }

    // Validate altitude (z)
    if position.z < -10_000.0 || position.z > MAX_ALTITUDE_M {
        return Err(ValidationError::InvalidValue {
            param: format!("{}.altitude", param_name),
            reason: format!(
                "altitude {} m is outside valid range [-10000, {}]",
                position.z, MAX_ALTITUDE_M
            ),
        });
    }

    Ok(())
}

// ============================================================================
// Frequency Validation
// ============================================================================

/// Validate operating frequency.
///
/// Ensures frequency is within system specification [100, 50000] MHz.
fn validate_frequency(frequency_mhz: f64, param_name: &str) -> ValidationResult<()> {
    if !frequency_mhz.is_finite() {
        return Err(ValidationError::InvalidValue {
            param: param_name.to_string(),
            reason: format!("value is not finite: {}", frequency_mhz),
        });
    }

    if !(MIN_FREQUENCY_MHZ..=MAX_FREQUENCY_MHZ).contains(&frequency_mhz) {
        return Err(ValidationError::FrequencyOutOfRange { frequency_mhz });
    }

    Ok(())
}

// ============================================================================
// Antenna/Feed Validation
// ============================================================================

/// Validate that antenna and feed exist in repository.
fn validate_antenna_feed_exists(
    antenna_id: &str,
    feed_id: &str,
    repository: &CalibrationRepository,
) -> ValidationResult<()> {
    // Check antenna ID format (non-empty, no special characters)
    if antenna_id.is_empty() {
        return Err(ValidationError::InvalidAntennaId {
            antenna_id: antenna_id.to_string(),
            reason: "antenna ID cannot be empty".to_string(),
        });
    }

    if feed_id.is_empty() {
        return Err(ValidationError::InvalidValue {
            param: "feed_id".to_string(),
            reason: "feed ID cannot be empty".to_string(),
        });
    }

    // Check if antenna and feed exist in repository
    if !repository.has_calibration(antenna_id, feed_id) {
        // Determine if antenna exists at all
        if repository.list_feeds(antenna_id).is_empty() {
            return Err(ValidationError::InvalidAntennaId {
                antenna_id: antenna_id.to_string(),
                reason: "antenna not found in calibration repository".to_string(),
            });
        } else {
            // Antenna exists but feed doesn't
            let available_feeds = repository.list_feeds(antenna_id);
            return Err(ValidationError::InvalidValue {
                param: "feed_id".to_string(),
                reason: format!(
                    "feed '{}' not found for antenna '{}'. Available feeds: {:?}",
                    feed_id, antenna_id, available_feeds
                ),
            });
        }
    }

    Ok(())
}

// ============================================================================
// Quaternion Validation
// ============================================================================

/// Validate that a quaternion `[w, x, y, z]` is approximately unit-normalised.
///
/// The norm must be within 1e-3 of 1.0 to be accepted. This tolerates small
/// floating-point round-off while catching obviously unnormalised inputs.
fn validate_quaternion_norm(
    w: f64,
    x: f64,
    y: f64,
    z: f64,
    param_name: &str,
) -> ValidationResult<()> {
    let norm = (w * w + x * x + y * y + z * z).sqrt();
    if (norm - 1.0).abs() > 1e-3 {
        return Err(ValidationError::InvalidValue {
            param: param_name.to_string(),
            reason: format!(
                "quaternion norm is {:.6} (expected 1.0 ± 1e-3); \
                 supply a unit quaternion",
                norm
            ),
        });
    }
    Ok(())
}

// ============================================================================
// Coordinate Ambiguity Warning
// ============================================================================

/// Altitude threshold above which a geodetic position is considered ambiguous (100 km).
///
/// Positions auto-detected as geodetic with |z| > this value could plausibly be ECEF
/// and should carry an explicit `coordinate_system` tag to prevent misclassification.
const GEODETIC_AMBIGUITY_ALTITUDE_M: f64 = 100_000.0;

/// Emit a warning if a position is auto-detected as geodetic but has suspiciously
/// high altitude (> 100 km), which could indicate an ECEF position near the
/// threshold or a legitimate high-altitude geodetic position (e.g. GEO orbit).
///
/// If the caller has set an explicit `coordinate_system`, no warning is emitted.
fn warn_if_ambiguous(pos: &Position3D, name: &str, warnings: &mut Vec<String>) {
    if pos.coordinate_system.is_none() {
        if let CoordinateSystem::Geodetic = pos.coordinate_system() {
            if pos.z.abs() > GEODETIC_AMBIGUITY_ALTITUDE_M {
                warnings.push(format!(
                    "{name}: auto-detected as geodetic with altitude {:.0} km; \
                     set coordinate_system explicitly to avoid ECEF misclassification",
                    pos.z / 1000.0
                ));
            }
        }
    }
}

/// Return ambiguity warnings for all positions in a `GainRequest`.
///
/// These are non-fatal advisories: they do not block processing but indicate
/// positions where auto-detection may produce incorrect results. Callers should
/// merge these into the response warnings list.
pub fn coordinate_ambiguity_warnings(request: &GainRequest) -> Vec<String> {
    let mut warnings = Vec::new();
    warn_if_ambiguous(&request.vehicle_position, "vehicle_position", &mut warnings);
    warn_if_ambiguous(
        &request.reflector_boresight,
        "reflector_boresight",
        &mut warnings,
    );
    warn_if_ambiguous(&request.feed_position, "feed_position", &mut warnings);
    warn_if_ambiguous(&request.emitter_position, "emitter_position", &mut warnings);
    warnings
}

// ============================================================================
// Grid Configuration Validation
// ============================================================================

/// Validate grid configuration for heatmap generation.
fn validate_grid_config(grid_config: &GridConfig) -> ValidationResult<()> {
    match grid_config {
        GridConfig::Rectangular {
            azimuth_range_deg,
            elevation_range_deg,
        } => {
            validate_range_config(azimuth_range_deg, "azimuth_range_deg")?;
            validate_range_config(elevation_range_deg, "elevation_range_deg")?;

            // Check total grid points
            let total_points = azimuth_range_deg.num_points() * elevation_range_deg.num_points();
            if total_points > MAX_HEATMAP_POINTS {
                return Err(ValidationError::InvalidGrid {
                    dimension: "rectangular".to_string(),
                    reason: format!(
                        "total grid points {} exceeds maximum {} ({}x{} grid)",
                        total_points,
                        MAX_HEATMAP_POINTS,
                        azimuth_range_deg.num_points(),
                        elevation_range_deg.num_points()
                    ),
                });
            }

            Ok(())
        }
        GridConfig::H3 {
            h3_resolution,
            center_azimuth_deg,
            center_elevation_deg,
            field_of_view_deg,
        } => {
            // Validate H3 resolution (0-15)
            if *h3_resolution > 15 {
                return Err(ValidationError::InvalidGrid {
                    dimension: "h3_resolution".to_string(),
                    reason: format!("H3 resolution {} exceeds maximum 15", h3_resolution),
                });
            }

            // Validate center azimuth
            if !center_azimuth_deg.is_finite() {
                return Err(ValidationError::InvalidGrid {
                    dimension: "center_azimuth_deg".to_string(),
                    reason: format!("value is not finite: {}", center_azimuth_deg),
                });
            }

            // Validate center elevation
            if !center_elevation_deg.is_finite()
                || *center_elevation_deg < -90.0
                || *center_elevation_deg > 90.0
            {
                return Err(ValidationError::AngleOutOfRange {
                    angle_type: "center_elevation".to_string(),
                    value: *center_elevation_deg,
                    min: -90.0,
                    max: 90.0,
                });
            }

            // Validate field of view
            if !field_of_view_deg.is_finite()
                || *field_of_view_deg <= 0.0
                || *field_of_view_deg > 180.0
            {
                return Err(ValidationError::InvalidGrid {
                    dimension: "field_of_view_deg".to_string(),
                    reason: format!(
                        "field of view {} must be in range (0, 180] degrees",
                        field_of_view_deg
                    ),
                });
            }

            Ok(())
        }
    }
}

/// Validate a range configuration.
fn validate_range_config(range: &RangeConfig, dimension: &str) -> ValidationResult<()> {
    // Check for finite values
    if !range.min.is_finite() || !range.max.is_finite() || !range.step.is_finite() {
        return Err(ValidationError::InvalidGrid {
            dimension: dimension.to_string(),
            reason: "range contains non-finite values (NaN or Inf)".to_string(),
        });
    }

    // Check min < max
    if range.min >= range.max {
        return Err(ValidationError::InvalidGrid {
            dimension: dimension.to_string(),
            reason: format!("min ({}) must be less than max ({})", range.min, range.max),
        });
    }

    // Check positive step
    if range.step <= 0.0 {
        return Err(ValidationError::InvalidGrid {
            dimension: dimension.to_string(),
            reason: format!("step ({}) must be positive", range.step),
        });
    }

    // Check step isn't too small (would create too many points)
    let num_points = range.num_points();
    if num_points > MAX_HEATMAP_POINTS {
        return Err(ValidationError::InvalidGrid {
            dimension: dimension.to_string(),
            reason: format!(
                "step size {} too small: would create {} points (max {} per dimension)",
                range.step, num_points, MAX_HEATMAP_POINTS
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schemas::CoordinateSystem;
    use crate::data::repository::CalibrationRepository;

    // Helper to create a test repository with sample data
    fn create_test_repository() -> CalibrationRepository {
        CalibrationRepository::new()
    }

    // ========================================================================
    // Coordinate Ambiguity Warning Tests
    // ========================================================================

    #[test]
    fn test_warn_if_ambiguous_high_altitude_no_tag() {
        // z = 200 km altitude, no explicit tag → warning expected
        let pos = Position3D::new(0.0, 0.0, 200_000.0);
        let mut warnings = Vec::new();
        warn_if_ambiguous(&pos, "test_position", &mut warnings);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("test_position"));
        assert!(warnings[0].contains("200 km"));
        assert!(warnings[0].contains("coordinate_system"));
    }

    #[test]
    fn test_warn_if_ambiguous_high_altitude_with_explicit_tag() {
        // Same position but with explicit Geodetic tag → no warning
        let mut pos = Position3D::new(0.0, 0.0, 200_000.0);
        pos.coordinate_system = Some(CoordinateSystem::Geodetic);
        let mut warnings = Vec::new();
        warn_if_ambiguous(&pos, "test_position", &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_warn_if_ambiguous_low_altitude_no_warning() {
        // z = 100 m, well below threshold → no warning
        let pos = Position3D::new(-118.0, 34.0, 100.0);
        let mut warnings = Vec::new();
        warn_if_ambiguous(&pos, "test_position", &mut warnings);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_coordinate_ambiguity_warnings_full_request() {
        // Build a GainRequest where emitter is at 500 km altitude (geodetic) — no explicit tag.
        // At 500 km, auto-detection says Geodetic (500_000 < 6_400_000), but altitude is
        // > 100 km, so it triggers the ambiguity warning.
        let request = GainRequest {
            antenna_id: "test".to_string(),
            feed_id: "feed".to_string(),
            vehicle_position: Position3D::new(-118.0, 34.0, 100.0),
            reflector_boresight: Position3D::new(-118.0, 34.0, 110.0),
            feed_position: Position3D::new(-118.0, 34.0, 105.0),
            // LEO altitude in geodetic form, no tag — ambiguous
            emitter_position: Position3D::new(0.0, 0.0, 500_000.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        };
        let warnings = coordinate_ambiguity_warnings(&request);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("emitter_position"));
    }

    // ========================================================================
    // Position Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_ecef_position_valid() {
        let pos = Position3D::new(6_500_000.0, 100_000.0, 200_000.0);
        assert!(validate_position(&pos, "test_pos").is_ok());
    }

    #[test]
    fn test_validate_ecef_position_exceeds_max() {
        let pos = Position3D::new(450_000_000.0, 0.0, 0.0);
        let result = validate_position(&pos, "test_pos");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_validate_geodetic_position_valid() {
        let pos = Position3D::new(-118.1234, 34.5678, 100.0);
        assert!(validate_position(&pos, "test_pos").is_ok());
    }

    #[test]
    fn test_validate_geodetic_position_invalid_longitude() {
        let pos = Position3D::new(-200.0, 34.0, 100.0);
        let result = validate_position(&pos, "test_pos");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("longitude"));
    }

    #[test]
    fn test_validate_geodetic_position_invalid_latitude() {
        let pos = Position3D::new(-118.0, 100.0, 100.0);
        let result = validate_position(&pos, "test_pos");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("latitude"));
    }

    #[test]
    fn test_validate_geodetic_position_invalid_altitude() {
        let pos = Position3D::new(-118.0, 34.0, 420_000_000.0);
        let result = validate_position(&pos, "test_pos");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("test_pos.z"));
    }

    #[test]
    fn test_validate_position_nan() {
        let pos = Position3D::new(f64::NAN, 0.0, 0.0);
        let result = validate_position(&pos, "test_pos");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not finite"));
    }

    #[test]
    fn test_validate_position_infinity() {
        let pos = Position3D::new(f64::INFINITY, 0.0, 0.0);
        let result = validate_position(&pos, "test_pos");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not finite"));
    }

    // ========================================================================
    // Frequency Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_frequency_valid() {
        assert!(validate_frequency(8400.0, "frequency_mhz").is_ok());
        assert!(validate_frequency(100.0, "frequency_mhz").is_ok());
        assert!(validate_frequency(50_000.0, "frequency_mhz").is_ok());
    }

    #[test]
    fn test_validate_frequency_too_low() {
        let result = validate_frequency(50.0, "frequency_mhz");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("outside supported range"));
    }

    #[test]
    fn test_validate_frequency_too_high() {
        let result = validate_frequency(60_000.0, "frequency_mhz");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("outside supported range"));
    }

    #[test]
    fn test_validate_frequency_nan() {
        let result = validate_frequency(f64::NAN, "frequency_mhz");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not finite"));
    }

    // ========================================================================
    // Antenna/Feed Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_antenna_feed_empty_antenna_id() {
        let repo = create_test_repository();
        let result = validate_antenna_feed_exists("", "feed1", &repo);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_antenna_feed_empty_feed_id() {
        let repo = create_test_repository();
        let result = validate_antenna_feed_exists("antenna1", "", &repo);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_antenna_feed_not_found() {
        let repo = create_test_repository();
        let result = validate_antenna_feed_exists("nonexistent", "feed1", &repo);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // ========================================================================
    // Grid Configuration Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_rectangular_grid_valid() {
        let grid = GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 360.0, 5.0),
            elevation_range_deg: RangeConfig::new(0.0, 90.0, 2.0),
        };
        assert!(validate_grid_config(&grid).is_ok());
    }

    #[test]
    fn test_validate_rectangular_grid_invalid_range() {
        let grid = GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(360.0, 0.0, 5.0), // min > max
            elevation_range_deg: RangeConfig::new(0.0, 90.0, 2.0),
        };
        let result = validate_grid_config(&grid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be less"));
    }

    #[test]
    fn test_validate_rectangular_grid_negative_step() {
        let grid = GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 360.0, -5.0),
            elevation_range_deg: RangeConfig::new(0.0, 90.0, 2.0),
        };
        let result = validate_grid_config(&grid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be positive"));
    }

    #[test]
    fn test_validate_rectangular_grid_too_many_points() {
        let grid = GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 360.0, 0.001),
            elevation_range_deg: RangeConfig::new(0.0, 90.0, 0.001),
        };
        let result = validate_grid_config(&grid);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // Error is caught at the individual range validation level
        assert!(err_msg.contains("too small") && err_msg.contains("points"));
    }

    #[test]
    fn test_validate_h3_grid_valid() {
        let grid = GridConfig::H3 {
            h3_resolution: 7,
            center_azimuth_deg: 180.0,
            center_elevation_deg: 45.0,
            field_of_view_deg: 30.0,
        };
        assert!(validate_grid_config(&grid).is_ok());
    }

    #[test]
    fn test_validate_h3_grid_invalid_resolution() {
        let grid = GridConfig::H3 {
            h3_resolution: 20,
            center_azimuth_deg: 180.0,
            center_elevation_deg: 45.0,
            field_of_view_deg: 30.0,
        };
        let result = validate_grid_config(&grid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_validate_h3_grid_invalid_elevation() {
        let grid = GridConfig::H3 {
            h3_resolution: 7,
            center_azimuth_deg: 180.0,
            center_elevation_deg: 100.0,
            field_of_view_deg: 30.0,
        };
        let result = validate_grid_config(&grid);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("outside valid range"));
    }

    #[test]
    fn test_validate_h3_grid_invalid_fov() {
        let grid = GridConfig::H3 {
            h3_resolution: 7,
            center_azimuth_deg: 180.0,
            center_elevation_deg: 45.0,
            field_of_view_deg: 200.0,
        };
        let result = validate_grid_config(&grid);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("field of view"));
    }

    // ========================================================================
    // Range Config Tests
    // ========================================================================

    #[test]
    fn test_validate_range_config_nan() {
        let range = RangeConfig::new(f64::NAN, 360.0, 5.0);
        let result = validate_range_config(&range, "test_range");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("non-finite values"));
    }

    // ========================================================================
    // H3 Link Budget Validation Tests
    // ========================================================================

    /// Helper to build a valid H3LinkBudgetRequest for testing.
    fn valid_h3_request() -> H3LinkBudgetRequest {
        H3LinkBudgetRequest {
            antenna_id: "antenna_1".to_string(),
            feed_id: "feed_0".to_string(),
            vehicle_position: Position3D::new(-118.0, 34.0, 100.0),
            reflector_boresight: Position3D::new(-118.1, 34.1, 200.0),
            feed_position: Position3D::new(-118.0, 34.0, 150.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            n_rings: 3,
            h3_resolution: None,
            temperature_k: None,
            vehicle_attitude: None,
        }
    }

    #[test]
    fn test_h3_request_valid() {
        let req = valid_h3_request();
        assert!(validate_h3_link_budget_request(&req).is_ok());
    }

    #[test]
    fn test_h3_request_n_rings_too_large() {
        let mut req = valid_h3_request();
        req.n_rings = 11;
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("n_rings"));
    }

    #[test]
    fn test_h3_request_n_rings_max_valid() {
        let mut req = valid_h3_request();
        req.n_rings = 10;
        assert!(validate_h3_link_budget_request(&req).is_ok());
    }

    #[test]
    fn test_h3_request_frequency_zero() {
        let mut req = valid_h3_request();
        req.frequency_mhz = 0.0;
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("frequency"));
    }

    #[test]
    fn test_h3_request_frequency_nan() {
        let mut req = valid_h3_request();
        req.frequency_mhz = f64::NAN;
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not finite"));
    }

    #[test]
    fn test_h3_request_frequency_too_high() {
        let mut req = valid_h3_request();
        req.frequency_mhz = 60_000.0;
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("outside supported range"));
    }

    #[test]
    fn test_h3_request_invalid_vehicle_position() {
        let mut req = valid_h3_request();
        req.vehicle_position = Position3D::new(f64::NAN, 0.0, 0.0);
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("vehicle_position"));
    }

    #[test]
    fn test_h3_request_invalid_reflector_boresight() {
        let mut req = valid_h3_request();
        req.reflector_boresight = Position3D::new(-200.0, 34.0, 100.0);
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("reflector_boresight"));
    }

    #[test]
    fn test_h3_request_invalid_feed_position() {
        let mut req = valid_h3_request();
        req.feed_position = Position3D::new(-118.0, 100.0, 150.0);
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("feed_position"));
    }

    // ========================================================================
    // Vehicle Attitude Quaternion Validation Tests
    // ========================================================================

    #[test]
    fn test_quaternion_norm_valid_unit() {
        // Identity quaternion: norm = 1.0 → valid
        assert!(validate_quaternion_norm(1.0, 0.0, 0.0, 0.0, "q").is_ok());
        // 90° rotation about Z axis: norm ≈ 1.0 → valid
        let s = std::f64::consts::FRAC_1_SQRT_2;
        assert!(validate_quaternion_norm(s, 0.0, 0.0, s, "q").is_ok());
        // Slightly off-unit but within 1e-3 tolerance → valid
        assert!(validate_quaternion_norm(1.0005, 0.0, 0.0, 0.0, "q").is_ok());
    }

    #[test]
    fn test_quaternion_norm_invalid_non_unit() {
        // norm = 2.0 → rejected
        let result = validate_quaternion_norm(2.0, 0.0, 0.0, 0.0, "vehicle_attitude");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("vehicle_attitude"), "error: {msg}");
        assert!(
            msg.contains("norm") || msg.contains("unit"),
            "error should mention norm: {msg}"
        );

        // zero quaternion: norm = 0.0 → rejected
        let result = validate_quaternion_norm(0.0, 0.0, 0.0, 0.0, "vehicle_attitude");
        assert!(result.is_err());
    }

    #[test]
    fn test_h3_request_non_unit_quaternion_rejected() {
        // Uses validate_h3_link_budget_request which does not check antenna/feed existence.
        let mut req = valid_h3_request();
        req.vehicle_attitude = Some([0.0, 0.0, 0.0, 0.0]); // zero quaternion
        let result = validate_h3_link_budget_request(&req);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("vehicle_attitude"), "error: {msg}");
    }

    #[test]
    fn test_h3_request_unit_quaternion_accepted() {
        let mut req = valid_h3_request();
        req.vehicle_attitude = Some([1.0, 0.0, 0.0, 0.0]);
        assert!(validate_h3_link_budget_request(&req).is_ok());
    }

    // ========================================================================
    // Batch Request Validation Tests
    // ========================================================================

    #[test]
    fn test_validate_batch_empty() {
        let repo = create_test_repository();
        let request = BatchGainRequest {
            evaluations: vec![],
        };
        let result = validate_batch_gain_request(&request, &repo);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("at least one evaluation"));
    }

    #[test]
    fn test_validate_batch_exceeds_limit() {
        let repo = create_test_repository();
        let mut evaluations = Vec::new();
        for _ in 0..1001 {
            evaluations.push(GainRequest {
                antenna_id: "test".to_string(),
                feed_id: "test_feed".to_string(),
                vehicle_position: Position3D::new(0.0, 0.0, 0.0),
                reflector_boresight: Position3D::new(0.0, 0.0, 0.0),
                feed_position: Position3D::new(0.0, 0.0, 0.0),
                emitter_position: Position3D::new(0.0, 0.0, 0.0),
                frequency_mhz: 8400.0,
                pointing_frequency_mhz: None,
                include_reference: false,
                vehicle_attitude: None,
            });
        }
        let request = BatchGainRequest { evaluations };
        let result = validate_batch_gain_request(&request, &repo);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeds limit of 1000"));
    }
}
