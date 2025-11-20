//! 3D Coordinate Transformations for Antenna Gain Computation
//!
//! This module provides comprehensive 3D coordinate transformations needed for
//! computing antenna gain from geometric configurations. Supports:
//!
//! - ECEF ↔ Geodetic transformations (WGS84 ellipsoid)
//! - ECEF → Antenna Frame transformations
//! - Antenna Frame → Spherical coordinates (azimuth, elevation)
//! - Geometric computations (feed offset, emitter direction)
//! - Beam squint correction
//!
//! # Coordinate System Conventions
//!
//! ## ECEF (Earth-Centered Earth-Fixed)
//! - Origin at Earth's center of mass
//! - X-axis: passes through intersection of equator and prime meridian
//! - Y-axis: passes through equator at 90° East longitude
//! - Z-axis: passes through North Pole
//! - Units: meters
//!
//! ## Geodetic (WGS84)
//! - Longitude: -180° to +180° (positive East)
//! - Latitude: -90° to +90° (positive North)
//! - Altitude: meters above WGS84 ellipsoid
//!
//! ## Antenna Frame
//! - Origin at antenna reference point (typically reflector vertex or vehicle position)
//! - Coordinate axes defined by boresight direction
//! - Units: meters
//!
//! ## Spherical (Antenna-Centric)
//! - Azimuth: 0° = +X axis, 90° = +Y axis (measured counterclockwise from above)
//! - **Elevation: POLAR ANGLE from boresight (+Z axis)**
//!   - 0° = boresight (along +Z, perfect alignment)
//!   - 90° = perpendicular to boresight (in XY plane)
//!   - This is the **physics convention** (polar angle θ in spherical coordinates)
//!   - NOT horizon-based elevation (which would be 0° at horizon, 90° at zenith)
//! - Range: distance from antenna in meters
//!
//! **IMPORTANT**: This module uses polar angle throughout. When interfacing with
//! other coordinate systems, ensure consistent interpretation of "elevation".

use crate::api::schemas::{Position3D, Vector3D};
use crate::error::{AntennaModelError, Result};

// ============================================================================
// WGS84 Ellipsoid Parameters
// ============================================================================

/// WGS84 semi-major axis (equatorial radius) in meters
const WGS84_A: f64 = 6_378_137.0;

/// WGS84 flattening factor
const WGS84_F: f64 = 1.0 / 298.257223563;

/// WGS84 semi-minor axis (polar radius) in meters
const WGS84_B: f64 = WGS84_A * (1.0 - WGS84_F);

/// WGS84 first eccentricity squared
const WGS84_E2: f64 = 2.0 * WGS84_F - WGS84_F * WGS84_F;

/// Threshold for detecting ECEF coordinates (6400 km in meters)
pub const ECEF_THRESHOLD_M: f64 = 6_400_000.0;

/// Maximum reasonable altitude for coordinate validation (4000000 km, allows HEO satellites)
const MAX_ALTITUDE_M: f64 = 400_000_000.0;

/// Maximum reasonable ECEF coordinate magnitude (Earth radius + max altitude)
const MAX_ECEF_M: f64 = WGS84_A + MAX_ALTITUDE_M;

// ============================================================================
// Coordinate System Detection
// ============================================================================

/// Detect coordinate system from Position3D magnitude.
///
/// Returns true if coordinates are ECEF, false if Geodetic.
///
/// Detection logic: If |x| > 6400 km OR |y| > 6400 km OR |z| > 6400 km → ECEF
pub fn is_ecef_coordinates(pos: &Position3D) -> bool {
    pos.is_ecef()
}

/// Validate ECEF coordinates are reasonable.
///
/// Checks:
/// - No NaN or Inf values
/// - Magnitude does not exceed Earth radius + maximum orbital altitude
pub fn validate_ecef(x: f64, y: f64, z: f64) -> Result<()> {
    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "ECEF position".to_string(),
            reason: format!("Non-finite coordinate: x={}, y={}, z={}", x, y, z),
        });
    }

    let magnitude = (x * x + y * y + z * z).sqrt();
    if magnitude > MAX_ECEF_M {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "ECEF position".to_string(),
            reason: format!(
                "ECEF magnitude ({:.1} km) exceeds maximum ({:.1} km)",
                magnitude / 1000.0,
                MAX_ECEF_M / 1000.0
            ),
        });
    }

    Ok(())
}

/// Validate Geodetic coordinates are reasonable.
///
/// Checks:
/// - Longitude: -180 to +180 degrees
/// - Latitude: -90 to +90 degrees
/// - Altitude: < maximum orbital altitude
pub fn validate_geodetic(lon_deg: f64, lat_deg: f64, alt_m: f64) -> Result<()> {
    if !lon_deg.is_finite() || !lat_deg.is_finite() || !alt_m.is_finite() {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "Geodetic position".to_string(),
            reason: format!(
                "Non-finite coordinate: lon={}, lat={}, alt={}",
                lon_deg, lat_deg, alt_m
            ),
        });
    }

    if !(-180.0..=180.0).contains(&lon_deg) {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "longitude".to_string(),
            reason: format!("Longitude {} out of range [-180, 180] degrees", lon_deg),
        });
    }

    if !(-90.0..=90.0).contains(&lat_deg) {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "latitude".to_string(),
            reason: format!("Latitude {} out of range [-90, 90] degrees", lat_deg),
        });
    }

    if alt_m > MAX_ALTITUDE_M {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "altitude".to_string(),
            reason: format!(
                "Altitude {:.1} km exceeds maximum {:.1} km",
                alt_m / 1000.0,
                MAX_ALTITUDE_M / 1000.0
            ),
        });
    }

    Ok(())
}

// ============================================================================
// ECEF ↔ Geodetic Transformations (WGS84)
// ============================================================================

/// Convert Geodetic coordinates to ECEF.
///
/// Uses WGS84 ellipsoid parameters.
///
/// # Arguments
/// - `lon_deg`: Longitude in degrees (-180 to +180)
/// - `lat_deg`: Latitude in degrees (-90 to +90)
/// - `alt_m`: Altitude above ellipsoid in meters
///
/// # Returns
/// ECEF coordinates (x, y, z) in meters
pub fn geodetic_to_ecef(lon_deg: f64, lat_deg: f64, alt_m: f64) -> Result<(f64, f64, f64)> {
    validate_geodetic(lon_deg, lat_deg, alt_m)?;

    let lon_rad = lon_deg.to_radians();
    let lat_rad = lat_deg.to_radians();

    // Radius of curvature in prime vertical
    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();

    // ECEF coordinates
    let x = (n + alt_m) * cos_lat * lon_rad.cos();
    let y = (n + alt_m) * cos_lat * lon_rad.sin();
    let z = (n * (1.0 - WGS84_E2) + alt_m) * sin_lat;

    Ok((x, y, z))
}

/// Convert ECEF coordinates to Geodetic.
///
/// Uses iterative Bowring's method for accuracy.
///
/// # Arguments
/// - `x`, `y`, `z`: ECEF coordinates in meters
///
/// # Returns
/// Geodetic coordinates: (longitude_deg, latitude_deg, altitude_m)
pub fn ecef_to_geodetic(x: f64, y: f64, z: f64) -> Result<(f64, f64, f64)> {
    validate_ecef(x, y, z)?;

    // Handle special case: origin
    let p = (x * x + y * y).sqrt();
    if p < 1e-6 && z.abs() < 1e-6 {
        return Ok((0.0, 0.0, -WGS84_A));
    }

    // Longitude (always well-defined except at origin)
    let lon_rad = y.atan2(x);
    let lon_deg = lon_rad.to_degrees();

    // Latitude and altitude using Bowring's method (iterative)
    let e_prime_sq = (WGS84_A * WGS84_A - WGS84_B * WGS84_B) / (WGS84_B * WGS84_B);

    // Initial guess
    let mut lat_rad =
        (z / p * (1.0 + e_prime_sq * WGS84_B / (x * x + y * y + z * z).sqrt())).atan();

    // Iterate to convergence
    for _ in 0..10 {
        let sin_lat = lat_rad.sin();
        let _cos_lat = lat_rad.cos();
        let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        let new_lat_rad = (z + n * WGS84_E2 * sin_lat).atan2(p);

        // Check convergence (1e-12 radians ≈ 6e-8 degrees ≈ 0.004 mm at equator)
        if (new_lat_rad - lat_rad).abs() < 1e-12 {
            lat_rad = new_lat_rad;
            break;
        }
        lat_rad = new_lat_rad;
    }

    let lat_deg = lat_rad.to_degrees();

    // Altitude
    let sin_lat = lat_rad.sin();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    let alt_m = p / lat_rad.cos() - n;

    Ok((lon_deg, lat_deg, alt_m))
}

/// Helper: Convert Position3D to ECEF coordinates.
fn position_to_ecef(pos: &Position3D) -> Result<(f64, f64, f64)> {
    if pos.is_ecef() {
        validate_ecef(pos.x, pos.y, pos.z)?;
        Ok((pos.x, pos.y, pos.z))
    } else {
        geodetic_to_ecef(pos.x, pos.y, pos.z)
    }
}

// ============================================================================
// Antenna Frame → Spherical Coordinates
// ============================================================================

/// Convert antenna frame Cartesian to spherical coordinates (azimuth, elevation, range).
///
/// # Coordinate Conventions
/// - Azimuth: 0° = +X axis, 90° = +Y axis, measured counterclockwise from above
/// - **Elevation: POLAR ANGLE from boresight (+Z axis)**
///   - 0° = boresight (z = +range, perfect alignment with +Z)
///   - 90° = perpendicular to boresight (in XY plane, z = 0)
///   - This is θ in standard spherical coordinates (θ, φ)
/// - Range: distance from origin
///
/// **CRITICAL**: This function returns POLAR ANGLE (0° = boresight), NOT horizon-based
/// elevation (0° = horizon). This matches the physics convention used throughout
/// the codebase and is compatible with `EClockConeCoordinates::from_azimuth_elevation()`.
///
/// # Arguments
/// - `x`, `y`, `z`: Position in antenna frame (meters)
///
/// # Returns
/// (azimuth_deg, elevation_deg_polar, range_m)
pub fn antenna_frame_to_spherical(x: f64, y: f64, z: f64) -> Result<(f64, f64, f64)> {
    let range = (x * x + y * y + z * z).sqrt();

    // Handle singularity at origin
    if range < 1e-6 {
        return Err(AntennaModelError::CoordinateTransformError {
            details: "Cannot compute spherical coordinates at origin (range ≈ 0)".to_string(),
        });
    }

    // Azimuth (atan2 handles all quadrants)
    let azimuth_rad = y.atan2(x);
    let azimuth_deg = azimuth_rad.to_degrees();

    // Elevation is POLAR ANGLE from boresight (+Z axis): 0° = boresight, 90° = perpendicular
    // Using acos so that when z=range (perfectly aligned with +Z), elevation=0°
    let elevation_rad = (z / range).acos();
    let elevation_deg = elevation_rad.to_degrees();

    // Handle singularity warnings at zenith/nadir
    if elevation_deg.abs() > 89.9 {
        // Azimuth is ambiguous near zenith/nadir, but we still return a value
        // Caller should be aware that azimuth is poorly defined
    }

    Ok((azimuth_deg, elevation_deg, range))
}

// ============================================================================
// Geometric Computations
// ============================================================================

/// Compute emitter direction (azimuth, elevation) using reflector boresight for orientation.
///
/// The boresight vector (from vehicle to reflector_boresight) establishes the antenna Z-axis.
///
/// # Arguments
/// - `emitter_pos`: Emitter position (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle position (ECEF or Geodetic)
/// - `boresight_pos`: Reflector boresight position (ECEF or Geodetic)
///
/// # Returns
/// (azimuth_deg, elevation_deg) in antenna frame
pub fn compute_emitter_direction(
    emitter_pos: &Position3D,
    vehicle_pos: &Position3D,
    boresight_pos: &Position3D,
) -> Result<(f64, f64)> {
    // Convert all positions to ECEF
    let (emitter_x, emitter_y, emitter_z) = position_to_ecef(emitter_pos)?;
    let (vehicle_x, vehicle_y, vehicle_z) = position_to_ecef(vehicle_pos)?;
    let (bore_x, bore_y, bore_z) = position_to_ecef(boresight_pos)?;

    // Compute boresight vector (defines antenna Z-axis)
    let bore_dx = bore_x - vehicle_x;
    let bore_dy = bore_y - vehicle_y;
    let bore_dz = bore_z - vehicle_z;
    let bore_mag = (bore_dx * bore_dx + bore_dy * bore_dy + bore_dz * bore_dz).sqrt();

    if bore_mag < 1e-6 {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "reflector_boresight".to_string(),
            reason: "Boresight position too close to vehicle position (< 1mm separation)"
                .to_string(),
        });
    }

    // Normalize boresight vector
    let z_x = bore_dx / bore_mag;
    let z_y = bore_dy / bore_mag;
    let z_z = bore_dz / bore_mag;

    // Compute emitter vector relative to vehicle
    let emitter_dx = emitter_x - vehicle_x;
    let emitter_dy = emitter_y - vehicle_y;
    let emitter_dz = emitter_z - vehicle_z;

    // Define antenna frame:
    // Z-axis: boresight direction
    // X-axis: perpendicular to Z in the plane containing Z and Earth's Z-axis (or East if aligned)
    // Y-axis: completes right-hand system

    // Choose X-axis perpendicular to Z-axis
    // Use cross product with a reference vector (prefer Earth Z-axis, fallback to East)
    let (ref_x, ref_y, ref_z) = if z_z.abs() < 0.99 {
        // Use Earth Z-axis as reference (not aligned with boresight)
        (0.0, 0.0, 1.0)
    } else {
        // Boresight nearly aligned with Earth Z → use East direction
        (0.0, 1.0, 0.0)
    };

    // Compute X-axis: cross product of reference with Z-axis, then normalize
    let x_x_raw = ref_y * z_z - ref_z * z_y;
    let x_y_raw = ref_z * z_x - ref_x * z_z;
    let x_z_raw = ref_x * z_y - ref_y * z_x;
    let x_mag = (x_x_raw * x_x_raw + x_y_raw * x_y_raw + x_z_raw * x_z_raw).sqrt();

    let x_x = x_x_raw / x_mag;
    let x_y = x_y_raw / x_mag;
    let x_z = x_z_raw / x_mag;

    // Compute Y-axis: cross product of Z with X (completes right-hand system)
    let y_x = z_y * x_z - z_z * x_y;
    let y_y = z_z * x_x - z_x * x_z;
    let y_z = z_x * x_y - z_y * x_x;

    // Transform emitter to antenna frame
    let antenna_x = emitter_dx * x_x + emitter_dy * x_y + emitter_dz * x_z;
    let antenna_y = emitter_dx * y_x + emitter_dy * y_y + emitter_dz * y_z;
    let antenna_z = emitter_dx * z_x + emitter_dy * z_y + emitter_dz * z_z;

    // Convert to spherical coordinates
    let (azimuth_deg, elevation_deg, _range) =
        antenna_frame_to_spherical(antenna_x, antenna_y, antenna_z)?;

    Ok((azimuth_deg, elevation_deg))
}

/// Compute feed offset from reflector boresight using boresight-based orientation.
///
/// # Arguments
/// - `feed_pos`: Feed position (ECEF or Geodetic)
/// - `boresight_pos`: Reflector boresight position (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle position (ECEF or Geodetic)
///
/// # Returns
/// Feed offset vector in antenna frame (meters)
pub fn compute_feed_offset_v2(
    feed_pos: &Position3D,
    boresight_pos: &Position3D,
    vehicle_pos: &Position3D,
) -> Result<Vector3D> {
    // Convert all to ECEF
    let (feed_x, feed_y, feed_z) = position_to_ecef(feed_pos)?;
    let (bore_x, bore_y, bore_z) = position_to_ecef(boresight_pos)?;
    let (vehicle_x, vehicle_y, vehicle_z) = position_to_ecef(vehicle_pos)?;

    // Compute feed offset directly in ECEF, then transform to antenna frame
    // Boresight vector (defines antenna Z-axis)
    let bore_dx = bore_x - vehicle_x;
    let bore_dy = bore_y - vehicle_y;
    let bore_dz = bore_z - vehicle_z;
    let bore_mag = (bore_dx * bore_dx + bore_dy * bore_dy + bore_dz * bore_dz).sqrt();

    if bore_mag < 1e-6 {
        return Err(AntennaModelError::InvalidCoordinate {
            param: "reflector_boresight".to_string(),
            reason: "Boresight position too close to vehicle position (< 1mm separation)"
                .to_string(),
        });
    }

    // Normalize boresight vector
    let z_x = bore_dx / bore_mag;
    let z_y = bore_dy / bore_mag;
    let z_z = bore_dz / bore_mag;

    // Feed offset vector in ECEF
    let feed_offset_dx = feed_x - bore_x;
    let feed_offset_dy = feed_y - bore_y;
    let feed_offset_dz = feed_z - bore_z;

    // Define antenna frame axes (same as in compute_emitter_direction_v2)
    let (ref_x, ref_y, ref_z) = if z_z.abs() < 0.99 {
        (0.0, 0.0, 1.0)
    } else {
        (0.0, 1.0, 0.0)
    };

    let x_x_raw = ref_y * z_z - ref_z * z_y;
    let x_y_raw = ref_z * z_x - ref_x * z_z;
    let x_z_raw = ref_x * z_y - ref_y * z_x;
    let x_mag = (x_x_raw * x_x_raw + x_y_raw * x_y_raw + x_z_raw * x_z_raw).sqrt();

    let x_x = x_x_raw / x_mag;
    let x_y = x_y_raw / x_mag;
    let x_z = x_z_raw / x_mag;

    let y_x = z_y * x_z - z_z * x_y;
    let y_y = z_z * x_x - z_x * x_z;
    let y_z = z_x * x_y - z_y * x_x;

    // Transform feed offset to antenna frame
    let offset_x = feed_offset_dx * x_x + feed_offset_dy * x_y + feed_offset_dz * x_z;
    let offset_y = feed_offset_dx * y_x + feed_offset_dy * y_y + feed_offset_dz * y_z;
    let offset_z = feed_offset_dx * z_x + feed_offset_dy * z_y + feed_offset_dz * z_z;

    Ok(Vector3D::new(offset_x, offset_y, offset_z))
}

// ============================================================================
// Feed Position Computation from Pointing Directions
// ============================================================================

/// Compute physical feed position from pointing directions (Earth positions).
///
/// When the API provides `feed_position` and `reflector_boresight` as Earth positions
/// (points where the feed and reflector are aimed), this function computes the
/// corresponding physical feed displacement in the antenna's coordinate frame.
///
/// # Arguments
/// - `feed_pointing_pos`: Earth position where feed is aimed (ECEF or Geodetic)
/// - `reflector_pointing_pos`: Earth position where reflector is aimed (ECEF or Geodetic)
/// - `vehicle_pos`: Satellite/antenna position (ECEF or Geodetic)
/// - `focal_length`: Antenna focal length in meters
///
/// # Returns
/// Physical feed position (x, y, z) in antenna frame relative to reflector vertex (meters)
pub fn compute_feed_position_from_pointing(
    feed_pointing_pos: &Position3D,
    reflector_pointing_pos: &Position3D,
    vehicle_pos: &Position3D,
    focal_length: f64,
) -> Result<(f64, f64, f64)> {
    // Compute pointing directions for both feed and reflector
    let (feed_az, feed_el) =
        compute_emitter_direction(feed_pointing_pos, vehicle_pos, reflector_pointing_pos)?;
    let (refl_az, refl_el) =
        compute_emitter_direction(reflector_pointing_pos, vehicle_pos, reflector_pointing_pos)?;

    // Reflector boresight should be at (0, 0) by definition
    // The angular offset between feed and reflector is the feed displacement angle
    let delta_az = feed_az - refl_az;
    let delta_el = feed_el - refl_el;

    // IMPORTANT: delta_el is POLAR ANGLE from boresight (from compute_emitter_direction_v2)
    // This matches the convention expected by EClockConeCoordinates::from_azimuth_elevation()
    // which was corrected to use polar angle (0° = boresight) instead of horizon-based elevation

    // Convert azimuth/elevation offset to E-cone/E-clock
    // E-cone = polar angle from boresight (equals delta_el directly)
    // E-clock = azimuthal angle around boresight (equals delta_az directly)
    use crate::model::coordinates::EClockConeCoordinates;
    let ecc = EClockConeCoordinates::from_azimuth_elevation(delta_az, delta_el);

    // Convert angular offset to absolute physical feed position
    // Returns (x, y, z) position in antenna frame with origin at reflector vertex
    let (x, y, z) = ecc.to_feed_position(focal_length);

    Ok((x, y, z))
}

// ============================================================================
// Beam Squint Correction
// ============================================================================

/// Apply beam squint correction for frequency-dependent beam pointing.
///
/// When the antenna is mechanically pointed at `pointing_frequency` but
/// operating at `operating_frequency`, the beam direction shifts due to
/// frequency-dependent phase effects.
///
/// # Arguments
/// - `azimuth_deg`: Uncorrected azimuth (degrees)
/// - `elevation_deg`: Uncorrected elevation (degrees)
/// - `pointing_freq_mhz`: Frequency at which antenna is pointed
/// - `operating_freq_mhz`: Actual operating frequency
/// - `antenna_diameter_m`: Antenna diameter (for computing squint magnitude)
///
/// # Returns
/// (corrected_azimuth_deg, corrected_elevation_deg, squint_magnitude_deg)
///
/// # Note
/// This is a simplified model. Real beam squint depends on antenna design,
/// feed configuration, and higher-order effects.
pub fn apply_beam_squint_correction(
    azimuth_deg: f64,
    elevation_deg: f64,
    pointing_freq_mhz: f64,
    operating_freq_mhz: f64,
    antenna_diameter_m: f64,
) -> (f64, f64, f64) {
    // If frequencies are the same (within 0.1%), no correction needed
    if (pointing_freq_mhz - operating_freq_mhz).abs() / pointing_freq_mhz < 0.001 {
        return (azimuth_deg, elevation_deg, 0.0);
    }

    // Frequency ratio
    let freq_ratio = operating_freq_mhz / pointing_freq_mhz;

    // Beam squint scales inversely with frequency (higher freq → tighter beam → larger angular shift)
    // Rough approximation: squint ≈ (1 - freq_ratio) * beamwidth
    // Beamwidth (HPBW) ≈ 70 * λ / D degrees (for parabolic dish)
    let wavelength_m = 299.792458 / operating_freq_mhz; // c / f (f in MHz → m/s / MHz = m)
    let beamwidth_deg = 70.0 * wavelength_m / antenna_diameter_m;

    // Squint magnitude (simplified linear model)
    let squint_deg = (1.0 - freq_ratio) * beamwidth_deg * 0.5;

    // Apply squint correction (in direction of boresight)
    // For simplicity, apply radially from boresight
    let corrected_azimuth = azimuth_deg;
    let corrected_elevation = elevation_deg + squint_deg;

    (corrected_azimuth, corrected_elevation, squint_deg.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-6;

    // ========================================================================
    // Coordinate System Detection
    // ========================================================================

    #[test]
    fn test_coordinate_detection_ecef() {
        let ecef = Position3D::new(6_500_000.0, 100_000.0, 200_000.0);
        assert!(is_ecef_coordinates(&ecef));
    }

    #[test]
    fn test_coordinate_detection_geodetic() {
        let geodetic = Position3D::new(-118.0, 34.0, 100.0);
        assert!(!is_ecef_coordinates(&geodetic));
    }

    #[test]
    fn test_coordinate_detection_boundary() {
        // Just below threshold (1000 km = 1,000,000 m)
        let below = Position3D::new(999_999.0, 0.0, 0.0);
        assert!(!is_ecef_coordinates(&below));

        // Just above threshold
        let above = Position3D::new(1_000_001.0, 0.0, 0.0);
        assert!(is_ecef_coordinates(&above));
    }

    // ========================================================================
    // ECEF ↔ Geodetic Transformations
    // ========================================================================

    #[test]
    fn test_geodetic_to_ecef_equator_prime_meridian() {
        // Point on equator at prime meridian, sea level
        let (x, y, z) = geodetic_to_ecef(0.0, 0.0, 0.0).unwrap();

        // Should be approximately at equatorial radius
        assert!((x - WGS84_A).abs() < 1.0); // Within 1 meter
        assert!(y.abs() < 1.0);
        assert!(z.abs() < 1.0);
    }

    #[test]
    fn test_geodetic_to_ecef_north_pole() {
        // North pole at sea level
        let (x, y, z) = geodetic_to_ecef(0.0, 90.0, 0.0).unwrap();

        // Should be at polar radius
        assert!(x.abs() < 1.0);
        assert!(y.abs() < 1.0);
        assert!((z - WGS84_B).abs() < 1.0);
    }

    #[test]
    fn test_geodetic_to_ecef_with_altitude() {
        // 1000 km altitude above equator
        let altitude = 1_000_000.0;
        let (x, y, z) = geodetic_to_ecef(0.0, 0.0, altitude).unwrap();

        let expected_x = WGS84_A + altitude;
        assert!((x - expected_x).abs() < 1.0);
        assert!(y.abs() < 1.0);
        assert!(z.abs() < 1.0);
    }

    #[test]
    fn test_ecef_to_geodetic_roundtrip() {
        let test_cases = vec![
            (0.0, 0.0, 0.0),       // Equator, prime meridian, sea level
            (-118.0, 34.0, 100.0), // Los Angeles area
            (0.0, 90.0, 0.0),      // North pole
            (0.0, -90.0, 0.0),     // South pole
            (180.0, 0.0, 0.0),     // Opposite prime meridian
            (45.0, 45.0, 1000.0),  // Mid-latitudes with altitude
        ];

        for (lon, lat, alt) in test_cases {
            let (x, y, z) = geodetic_to_ecef(lon, lat, alt).unwrap();
            let (lon2, lat2, alt2) = ecef_to_geodetic(x, y, z).unwrap();

            assert!(
                (lon - lon2).abs() < 1e-6,
                "Longitude roundtrip failed: {} vs {}",
                lon,
                lon2
            );
            assert!(
                (lat - lat2).abs() < 1e-6,
                "Latitude roundtrip failed: {} vs {}",
                lat,
                lat2
            );
            assert!(
                (alt - alt2).abs() < 1.0,
                "Altitude roundtrip failed: {} vs {}",
                alt,
                alt2
            );
        }
    }

    // ========================================================================
    // Validation
    // ========================================================================

    #[test]
    fn test_validate_ecef_invalid_magnitude() {
        // Too large (beyond maximum orbital altitude)
        let result = validate_ecef(380_000_000.0, 390_000_000.0, 390_000_000.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_ecef_nan() {
        let result = validate_ecef(f64::NAN, 0.0, 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_geodetic_out_of_range() {
        // Invalid longitude
        assert!(validate_geodetic(200.0, 0.0, 0.0).is_err());

        // Invalid latitude
        assert!(validate_geodetic(0.0, 100.0, 0.0).is_err());

        // Invalid altitude
        assert!(validate_geodetic(0.0, 0.0, 405_000_000.0).is_err());
    }

    // ========================================================================
    // Antenna Frame Transformations
    // ========================================================================

    #[test]
    fn test_antenna_frame_to_spherical_along_axes() {
        // Along +X axis
        let (az, el, range) = antenna_frame_to_spherical(100.0, 0.0, 0.0).unwrap();
        assert!(az.abs() < EPSILON);
        assert!(el.abs() - 90.0 < EPSILON);
        assert!((range - 100.0).abs() < EPSILON);

        // Along +Y axis
        let (az, el, range) = antenna_frame_to_spherical(0.0, 100.0, 0.0).unwrap();
        assert!((az - 90.0).abs() < EPSILON);
        assert!(el.abs() - 90.0 < EPSILON);
        assert!((range - 100.0).abs() < EPSILON);

        // Along +Z axis (zenith)
        let (_az, el, range) = antenna_frame_to_spherical(0.0, 0.0, 100.0).unwrap();
        assert!((el).abs() < EPSILON);
        assert!((range - 100.0).abs() < EPSILON);
    }

    #[test]
    fn test_antenna_frame_to_spherical_origin_error() {
        let result = antenna_frame_to_spherical(0.0, 0.0, 0.0);
        assert!(result.is_err());
    }

    // ========================================================================
    // Beam Squint
    // ========================================================================

    #[test]
    fn test_beam_squint_no_correction_same_frequency() {
        let (az, el, squint) = apply_beam_squint_correction(
            45.0,   // azimuth
            30.0,   // elevation
            8400.0, // pointing freq
            8400.0, // operating freq
            34.0,   // antenna diameter
        );

        assert!((az - 45.0).abs() < EPSILON);
        assert!((el - 30.0).abs() < EPSILON);
        assert!(squint.abs() < EPSILON);
    }

    #[test]
    fn test_beam_squint_correction_applied() {
        let (az, el, squint) = apply_beam_squint_correction(
            0.0,    // azimuth
            0.0,    // elevation
            8400.0, // pointing freq
            8450.0, // operating freq (slightly higher)
            34.0,   // antenna diameter
        );

        // Azimuth should be unchanged (radial correction)
        assert!((az - 0.0).abs() < EPSILON);

        // Elevation should change
        assert!(el.abs() > 0.0);

        // Squint magnitude should be non-zero
        assert!(squint > 0.0);
    }

    // ========================================================================
    // Integration Tests
    // ========================================================================

    #[test]
    fn test_full_pipeline_geodetic_to_spherical() {
        // Vehicle at equator, prime meridian
        let vehicle = Position3D::new(0.0, 0.0, 42_000_000.);

        // Boresight pointing to GEO altitude (defines antenna Z-axis)
        let boresight = Position3D::new(10.0, 15.0, 0.0);

        // Emitter in front of antenna, along boresight direction
        let emitter = Position3D::new(10.5, 15.0, 100.0);

        // Compute direction - this should work without error
        let (azimuth, elevation) =
            compute_emitter_direction(&emitter, &vehicle, &boresight).unwrap();

        // Emitter is directly along boresight direction, so elevation should be near 0°
        // (elevation is polar angle from boresight: 0° = on-axis)

        // Just verify we get reasonable angles (valid range)
        assert!(
            azimuth >= -180.0 && azimuth <= 180.0,
            "Azimuth {} out of range",
            azimuth
        );
        assert!(
            elevation >= 0.0 && elevation <= 180.0,
            "Elevation {} out of range",
            elevation
        );
        // Emitter should be nearly on-axis (elevation close to 0°)
        assert!(
            elevation < 1.0,
            "Expected on-axis elevation near 0°, got {:.2}°",
            elevation
        );
    }
}
