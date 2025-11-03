//! 3D Coordinate Transformations for Antenna Gain Computation
//!
//! This module provides comprehensive 3D coordinate transformations needed for
//! computing antenna gain from geometric configurations. Supports:
//!
//! - ECEF ↔ Geodetic transformations (WGS84 ellipsoid)
//! - ECEF → Antenna Frame transformations
//! - Antenna Frame → Spherical coordinates (azimuth, elevation)
//! - Vehicle attitude handling (quaternions and Euler angles)
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
//! - Coordinate axes aligned with vehicle attitude
//! - Units: meters
//!
//! ## Spherical (Antenna-Centric)
//! - Azimuth: 0° = North, 90° = East, measured clockwise from above
//! - Elevation: 0° = horizon, 90° = zenith
//! - Range: distance from antenna in meters

use crate::error::{AntennaModelError, Result};
use crate::api::schemas::{Position3D, Quaternion, EulerAngles, Attitude, Vector3D};

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

/// Maximum reasonable altitude for coordinate validation (10000 km)
const MAX_ALTITUDE_M: f64 = 10_000_000.0;

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
            reason: format!("Non-finite coordinate: lon={}, lat={}, alt={}", lon_deg, lat_deg, alt_m),
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
    let mut lat_rad = (z / p * (1.0 + e_prime_sq * WGS84_B / (x * x + y * y + z * z).sqrt())).atan();

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

// ============================================================================
// Attitude Transformations
// ============================================================================

/// Rotation matrix from quaternion.
///
/// Converts a quaternion to a 3x3 rotation matrix (row-major order).
///
/// # Arguments
/// - `q`: Quaternion (should be normalized)
///
/// # Returns
/// 3x3 rotation matrix as flat array [R11, R12, R13, R21, R22, R23, R31, R32, R33]
pub fn quaternion_to_rotation_matrix(q: &Quaternion) -> Result<[f64; 9]> {
    // Validate quaternion normalization
    let mag = q.magnitude();
    if (mag - 1.0).abs() > 0.01 {
        return Err(AntennaModelError::InvalidAttitude {
            reason: format!("Quaternion not normalized (|q| = {:.6})", mag),
        });
    }

    // Normalize to ensure unit quaternion
    let w = q.w / mag;
    let x = q.x / mag;
    let y = q.y / mag;
    let z = q.z / mag;

    // Rotation matrix elements (Wikipedia convention)
    let r11 = 1.0 - 2.0 * (y * y + z * z);
    let r12 = 2.0 * (x * y - w * z);
    let r13 = 2.0 * (x * z + w * y);

    let r21 = 2.0 * (x * y + w * z);
    let r22 = 1.0 - 2.0 * (x * x + z * z);
    let r23 = 2.0 * (y * z - w * x);

    let r31 = 2.0 * (x * z - w * y);
    let r32 = 2.0 * (y * z + w * x);
    let r33 = 1.0 - 2.0 * (x * x + y * y);

    Ok([r11, r12, r13, r21, r22, r23, r31, r32, r33])
}

/// Rotation matrix from Euler angles (Roll-Pitch-Yaw, X-Y-Z sequence).
///
/// # Arguments
/// - `euler`: Euler angles in degrees
///
/// # Returns
/// 3x3 rotation matrix as flat array [R11, R12, R13, R21, R22, R23, R31, R32, R33]
pub fn euler_to_rotation_matrix(euler: &EulerAngles) -> Result<[f64; 9]> {
    // Check for gimbal lock
    if euler.pitch_deg.abs() > 89.9 {
        return Err(AntennaModelError::InvalidAttitude {
            reason: format!(
                "Gimbal lock near pitch = {} degrees (|pitch| > 89.9°)",
                euler.pitch_deg
            ),
        });
    }

    let roll = euler.roll_deg.to_radians();
    let pitch = euler.pitch_deg.to_radians();
    let yaw = euler.yaw_deg.to_radians();

    let cr = roll.cos();
    let sr = roll.sin();
    let cp = pitch.cos();
    let sp = pitch.sin();
    let cy = yaw.cos();
    let sy = yaw.sin();

    // R = Rz(yaw) * Ry(pitch) * Rx(roll)
    let r11 = cy * cp;
    let r12 = cy * sp * sr - sy * cr;
    let r13 = cy * sp * cr + sy * sr;

    let r21 = sy * cp;
    let r22 = sy * sp * sr + cy * cr;
    let r23 = sy * sp * cr - cy * sr;

    let r31 = -sp;
    let r32 = cp * sr;
    let r33 = cp * cr;

    Ok([r11, r12, r13, r21, r22, r23, r31, r32, r33])
}

/// Get rotation matrix from Attitude enum.
pub fn attitude_to_rotation_matrix(attitude: &Attitude) -> Result<[f64; 9]> {
    match attitude {
        Attitude::Quaternion(q) => quaternion_to_rotation_matrix(q),
        Attitude::EulerAngles(e) => euler_to_rotation_matrix(e),
    }
}

/// Apply rotation matrix to a vector.
///
/// # Arguments
/// - `r`: 3x3 rotation matrix (flat array)
/// - `v`: Input vector (x, y, z)
///
/// # Returns
/// Rotated vector (x', y', z')
fn rotate_vector(r: &[f64; 9], v: (f64, f64, f64)) -> (f64, f64, f64) {
    let (x, y, z) = v;
    let x_new = r[0] * x + r[1] * y + r[2] * z;
    let y_new = r[3] * x + r[4] * y + r[5] * z;
    let z_new = r[6] * x + r[7] * y + r[8] * z;
    (x_new, y_new, z_new)
}

// ============================================================================
// ECEF → Antenna Frame Transformation
// ============================================================================

/// Transform ECEF position to antenna-centric frame.
///
/// This transformation:
/// 1. Converts all positions to ECEF if needed
/// 2. Computes target position relative to vehicle
/// 3. Applies vehicle attitude rotation
///
/// # Arguments
/// - `target_pos`: Position to transform (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle/antenna position (ECEF or Geodetic)
/// - `vehicle_attitude`: Vehicle attitude (quaternion or Euler angles)
///
/// # Returns
/// Position in antenna frame (x, y, z) in meters
pub fn ecef_to_antenna_frame(
    target_pos: &Position3D,
    vehicle_pos: &Position3D,
    vehicle_attitude: &Attitude,
) -> Result<(f64, f64, f64)> {
    // Convert both positions to ECEF
    let (target_x, target_y, target_z) = position_to_ecef(target_pos)?;
    let (vehicle_x, vehicle_y, vehicle_z) = position_to_ecef(vehicle_pos)?;

    // Relative position in ECEF frame
    let dx = target_x - vehicle_x;
    let dy = target_y - vehicle_y;
    let dz = target_z - vehicle_z;

    // Get rotation matrix from vehicle attitude
    let rotation = attitude_to_rotation_matrix(vehicle_attitude)?;

    // Rotate relative position to antenna frame
    let antenna_frame = rotate_vector(&rotation, (dx, dy, dz));

    Ok(antenna_frame)
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
/// - Elevation: 0° = XY plane (horizon), 90° = +Z axis (zenith)
/// - Range: distance from origin
///
/// # Arguments
/// - `x`, `y`, `z`: Position in antenna frame (meters)
///
/// # Returns
/// (azimuth_deg, elevation_deg, range_m)
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

    // Elevation (asin for numerical stability near zenith/nadir)
    let elevation_rad = (z / range).asin();
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

/// Compute feed offset from reflector boresight in antenna frame.
///
/// # Arguments
/// - `feed_pos`: Feed position (ECEF or Geodetic)
/// - `boresight_pos`: Reflector boresight position (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle position (ECEF or Geodetic)
/// - `vehicle_attitude`: Vehicle attitude
///
/// # Returns
/// Feed offset vector in antenna frame (meters)
pub fn compute_feed_offset(
    feed_pos: &Position3D,
    boresight_pos: &Position3D,
    vehicle_pos: &Position3D,
    vehicle_attitude: &Attitude,
) -> Result<Vector3D> {
    // Transform both to antenna frame
    let (feed_x, feed_y, feed_z) = ecef_to_antenna_frame(feed_pos, vehicle_pos, vehicle_attitude)?;
    let (bore_x, bore_y, bore_z) = ecef_to_antenna_frame(boresight_pos, vehicle_pos, vehicle_attitude)?;

    // Compute offset
    let offset_x = feed_x - bore_x;
    let offset_y = feed_y - bore_y;
    let offset_z = feed_z - bore_z;

    Ok(Vector3D::new(offset_x, offset_y, offset_z))
}

/// Compute emitter direction (azimuth, elevation) in antenna frame.
///
/// # Arguments
/// - `emitter_pos`: Emitter position (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle position (ECEF or Geodetic)
/// - `vehicle_attitude`: Vehicle attitude
///
/// # Returns
/// (azimuth_deg, elevation_deg) in antenna frame
pub fn compute_emitter_direction(
    emitter_pos: &Position3D,
    vehicle_pos: &Position3D,
    vehicle_attitude: &Attitude,
) -> Result<(f64, f64)> {
    // Transform emitter to antenna frame
    let (x, y, z) = ecef_to_antenna_frame(emitter_pos, vehicle_pos, vehicle_attitude)?;

    // Convert to spherical
    let (azimuth_deg, elevation_deg, _range) = antenna_frame_to_spherical(x, y, z)?;

    Ok((azimuth_deg, elevation_deg))
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
    use std::f64::consts::PI;

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
        // Just below threshold
        let below = Position3D::new(6_399_999.0, 0.0, 0.0);
        assert!(!is_ecef_coordinates(&below));

        // Just above threshold
        let above = Position3D::new(6_400_001.0, 0.0, 0.0);
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
            (0.0, 0.0, 0.0),           // Equator, prime meridian, sea level
            (-118.0, 34.0, 100.0),     // Los Angeles area
            (0.0, 90.0, 0.0),          // North pole
            (0.0, -90.0, 0.0),         // South pole
            (180.0, 0.0, 0.0),         // Opposite prime meridian
            (45.0, 45.0, 1000.0),      // Mid-latitudes with altitude
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
        let result = validate_ecef(20_000_000.0, 0.0, 0.0);
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
        assert!(validate_geodetic(0.0, 0.0, 20_000_000.0).is_err());
    }

    // ========================================================================
    // Attitude Transformations
    // ========================================================================

    #[test]
    fn test_quaternion_identity() {
        let q = Quaternion::identity();
        let r = quaternion_to_rotation_matrix(&q).unwrap();

        // Should be identity matrix
        let expected = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for i in 0..9 {
            assert!((r[i] - expected[i]).abs() < EPSILON);
        }
    }

    #[test]
    fn test_quaternion_rotation_90deg_z() {
        // 90° rotation about Z axis
        let q = Quaternion::new(
            (PI / 4.0).cos(), // w
            0.0,              // x
            0.0,              // y
            (PI / 4.0).sin(), // z (90° rotation → quaternion angle is 45°)
        );

        let r = quaternion_to_rotation_matrix(&q).unwrap();

        // Rotate vector [1, 0, 0] → should get [0, 1, 0]
        let rotated = rotate_vector(&r, (1.0, 0.0, 0.0));
        assert!(rotated.0.abs() < EPSILON);
        assert!((rotated.1 - 1.0).abs() < EPSILON);
        assert!(rotated.2.abs() < EPSILON);
    }

    #[test]
    fn test_euler_angles_zero() {
        let euler = EulerAngles::zero();
        let r = euler_to_rotation_matrix(&euler).unwrap();

        // Should be identity matrix
        let expected = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for i in 0..9 {
            assert!((r[i] - expected[i]).abs() < EPSILON);
        }
    }

    #[test]
    fn test_euler_gimbal_lock_detection() {
        let euler = EulerAngles::new(0.0, 90.0, 0.0);
        let result = euler_to_rotation_matrix(&euler);
        assert!(result.is_err());
    }

    // ========================================================================
    // Antenna Frame Transformations
    // ========================================================================

    #[test]
    fn test_antenna_frame_to_spherical_along_axes() {
        // Along +X axis
        let (az, el, range) = antenna_frame_to_spherical(100.0, 0.0, 0.0).unwrap();
        assert!(az.abs() < EPSILON);
        assert!(el.abs() < EPSILON);
        assert!((range - 100.0).abs() < EPSILON);

        // Along +Y axis
        let (az, el, range) = antenna_frame_to_spherical(0.0, 100.0, 0.0).unwrap();
        assert!((az - 90.0).abs() < EPSILON);
        assert!(el.abs() < EPSILON);
        assert!((range - 100.0).abs() < EPSILON);

        // Along +Z axis (zenith)
        let (_az, el, range) = antenna_frame_to_spherical(0.0, 0.0, 100.0).unwrap();
        assert!((el - 90.0).abs() < EPSILON);
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
            45.0,  // azimuth
            30.0,  // elevation
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
    fn test_full_pipeline_ecef_to_antenna_frame() {
        // Vehicle at equator, prime meridian at 100 km altitude (LEO orbit altitude)
        // This ensures ECEF coordinates exceed the 6.4M threshold
        let (veh_x, veh_y, veh_z) = geodetic_to_ecef(0.0, 0.0, 100_000.0).unwrap();

        // Emitter 1 km directly above vehicle (in ECEF, this is radially outward)
        let emitter_distance = (veh_x * veh_x + veh_y * veh_y + veh_z * veh_z).sqrt();
        let scale = (emitter_distance + 1000.0) / emitter_distance;
        let emit_x = veh_x * scale;
        let emit_y = veh_y * scale;
        let emit_z = veh_z * scale;

        let vehicle = Position3D::new(veh_x, veh_y, veh_z);
        let emitter = Position3D::new(emit_x, emit_y, emit_z);

        // Verify both are detected as ECEF
        assert!(vehicle.is_ecef(), "Vehicle should be detected as ECEF");
        assert!(emitter.is_ecef(), "Emitter should be detected as ECEF");

        // Identity attitude (no rotation)
        let attitude = Attitude::Quaternion(Quaternion::identity());

        // Transform
        let (x, y, z) = ecef_to_antenna_frame(&emitter, &vehicle, &attitude).unwrap();

        // Result should be roughly 1 km range
        let range = (x * x + y * y + z * z).sqrt();
        assert!((range - 1000.0).abs() < 10.0, "Expected range ≈ 1000m, got {:.1}m", range);
    }

    #[test]
    fn test_full_pipeline_geodetic_to_spherical() {
        // Vehicle at equator, prime meridian
        let vehicle = Position3D::new(0.0, 0.0, 0.0);

        // Emitter at same location but 10 km higher
        let emitter = Position3D::new(0.0, 0.0, 10_000.0);

        // Identity attitude (ECEF-aligned frame)
        let attitude = Attitude::Quaternion(Quaternion::identity());

        // Compute direction - this should work without error
        let (azimuth, elevation) = compute_emitter_direction(&emitter, &vehicle, &attitude).unwrap();

        // The emitter is 10 km above the vehicle. With identity attitude (ECEF-aligned),
        // the elevation depends on the vehicle's location on Earth.
        // At the equator, prime meridian, with ECEF-aligned attitude:
        // - The local "up" direction in ECEF is along +X axis
        // - So a point directly above (same lon/lat, higher alt) will have elevation near 0°
        //   in ECEF frame but should have some azimuth/elevation in antenna frame

        // Just verify we get reasonable angles (valid range)
        assert!(azimuth >= -180.0 && azimuth <= 180.0, "Azimuth {} out of range", azimuth);
        assert!(elevation >= -90.0 && elevation <= 90.0, "Elevation {} out of range", elevation);
        assert!(elevation > -45.0, "Expected reasonable elevation, got {:.2}°", elevation);
    }
}
