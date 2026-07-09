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

use crate::api::schemas::Position3D;
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

/// Maximum reasonable altitude for coordinate validation (400,000 km, allows HEO satellites)
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
    let cos_lat = lat_rad.cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    // p/cos(lat) − N is 0/0 at the poles (cos(lat)=0); use the z-based form there.
    let alt_m = if cos_lat.abs() > 1e-4 {
        p / cos_lat - n
    } else {
        z / sin_lat - n * (1.0 - WGS84_E2)
    };

    Ok((lon_deg, lat_deg, alt_m))
}

/// Convert Position3D to ECEF coordinates.
pub(crate) fn position_to_ecef(pos: &Position3D) -> Result<(f64, f64, f64)> {
    if pos.is_ecef() {
        validate_ecef(pos.x, pos.y, pos.z)?;
        Ok((pos.x, pos.y, pos.z))
    } else {
        geodetic_to_ecef(pos.x, pos.y, pos.z)
    }
}

/// Compute the 3×3 rotation matrix from ECEF to local ENU frame.
///
/// The matrix R satisfies `[E; N; U] = R × [x; y; z]_ECEF`.
/// To convert an ENU offset to ECEF use `R^T` (transpose), since R is orthogonal.
///
/// Rows of R are the ENU basis vectors expressed in ECEF:
/// - Row 0 (East):  `[-sin(lon),  cos(lon),       0      ]`
/// - Row 1 (North): `[-sin(lat)cos(lon), -sin(lat)sin(lon), cos(lat)]`
/// - Row 2 (Up):    `[ cos(lat)cos(lon),  cos(lat)sin(lon), sin(lat)]`
///
/// # Arguments
/// - `lat_rad`: Geodetic latitude in radians
/// - `lon_rad`: Geodetic longitude in radians
pub fn ecef_to_enu_rotation(lat_rad: f64, lon_rad: f64) -> [[f64; 3]; 3] {
    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let sin_lon = lon_rad.sin();
    let cos_lon = lon_rad.cos();

    [
        [-sin_lon, cos_lon, 0.0],
        [-sin_lat * cos_lon, -sin_lat * sin_lon, cos_lat],
        [cos_lat * cos_lon, cos_lat * sin_lon, sin_lat],
    ]
}

/// Normalize an azimuth in degrees to [0, 360).
///
/// Raw `atan2` returns values in (−180, 180]. Coverage ranges and correction-surface
/// B-spline knots are defined over [0, 360), so all pipeline azimuths must be
/// normalised before being compared or used as lookup keys.
pub fn normalize_azimuth_deg(az_deg: f64) -> f64 {
    let a = az_deg % 360.0;
    if a < 0.0 {
        a + 360.0
    } else {
        a
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

/// Rotate vector `v` by unit quaternion `q = [w, x, y, z]` (body → ECEF).
///
/// Uses the efficient formula: v' = v + 2·qv × (qv × v + w·v)
///
/// # Assumptions
///
/// This function assumes `q` is a **unit quaternion** (‖q‖ = 1). For a unit quaternion the
/// output vector has the same length as the input vector and the rotation is a pure isometry.
/// If `q` is not normalised the output vector length is **not** preserved — the result will
/// be scaled by ‖q‖² — and the rotation angle/axis will be incorrect.
///
/// Callers are responsible for normalising `q` before passing it in, or for validating it
/// upstream (e.g. via `validate_quaternion_norm` in the service layer). In particular,
/// `antenna_frame_axes` calls this function and relies on the caller having passed a valid
/// unit quaternion through `validate_gain_request` / `validate_h3_link_budget_request`.
pub fn quaternion_rotate(q: [f64; 4], v: (f64, f64, f64)) -> (f64, f64, f64) {
    let [w, x, y, z] = q;
    // v' = v + 2*qv × (qv × v + w*v), qv = (x, y, z)
    let (cx, cy, cz) = (
        y * v.2 - z * v.1 + w * v.0,
        z * v.0 - x * v.2 + w * v.1,
        x * v.1 - y * v.0 + w * v.2,
    );
    (
        v.0 + 2.0 * (y * cz - z * cy),
        v.1 + 2.0 * (z * cx - x * cz),
        v.2 + 2.0 * (x * cy - y * cx),
    )
}

/// Threshold below which body-X projection onto the boresight-perpendicular plane is treated
/// as a hard error: the azimuth reference is completely degenerate (body X ∥ boresight).
const X_MAG_HARD_ERROR_THRESHOLD: f64 = 1e-6;

/// Threshold below which body-X is *nearly* parallel to boresight.
/// `arccos(1e-2) ≈ 0.57°` — at this level the projected magnitude is < 1 % of the
/// original, meaning the azimuth reference is numerically ill-conditioned.
const X_MAG_NEAR_DEGENERATE_THRESHOLD: f64 = 1e-2;

/// Shared helper: build the antenna frame X/Y/Z unit axes from the boresight Z-axis
/// and an optional attitude quaternion.
///
/// # Arguments
/// - `z_unit`: Normalized boresight vector `(z_x, z_y, z_z)` — the antenna Z-axis.
/// - `attitude`: Optional unit quaternion `[w, x, y, z]` (body → ECEF).
///   Body axes: body +Z = boresight, body +X = azimuth-zero reference.
///
/// # Attitude-absent (None) behaviour
/// Uses a cross-product heuristic: prefers Earth Z-axis as reference, falls back to
/// East when the boresight is within arccos(0.99) ≈ 8.1° of Earth Z.
///
/// **Note**: the `None` fallback produces an azimuth-zero direction that is
/// approximate and **discontinuous** near boresight ∥ Earth-Z (|z_z| ≈ 1).
/// Callers that require a stable, deterministic azimuth reference (e.g. to match
/// a calibration E-clock reference) should supply `Some(attitude)` instead.
///
/// # Attitude-present (Some) behaviour
/// Rotates body +X = (1,0,0) into ECEF via `quaternion_rotate`, then projects
/// onto the plane ⊥ boresight. If the projection is degenerate (body X ∥
/// boresight), an error is returned.
///
/// # Returns
/// `((x_x,x_y,x_z), (y_x,y_y,y_z), (z_x,z_y,z_z))` — three orthonormal unit vectors.
#[allow(clippy::type_complexity)]
fn antenna_frame_axes(
    z_unit: (f64, f64, f64),
    attitude: Option<[f64; 4]>,
) -> Result<((f64, f64, f64), (f64, f64, f64), (f64, f64, f64))> {
    let (z_x, z_y, z_z) = z_unit;

    let (x_x, x_y, x_z) = match attitude {
        None => {
            // Reproduce the EXACT current cross-product heuristic so the None path
            // is bit-for-bit identical to the original compute_emitter_direction code.
            let (ref_x, ref_y, ref_z) = if z_z.abs() < 0.99 {
                // Use Earth Z-axis as reference (not aligned with boresight)
                (0.0_f64, 0.0_f64, 1.0_f64)
            } else {
                // Boresight nearly aligned with Earth Z → use East direction
                (0.0_f64, 1.0_f64, 0.0_f64)
            };

            // Compute X-axis: cross product of reference with Z-axis, then normalize
            let x_x_raw = ref_y * z_z - ref_z * z_y;
            let x_y_raw = ref_z * z_x - ref_x * z_z;
            let x_z_raw = ref_x * z_y - ref_y * z_x;
            let x_mag = (x_x_raw * x_x_raw + x_y_raw * x_y_raw + x_z_raw * x_z_raw).sqrt();
            (x_x_raw / x_mag, x_y_raw / x_mag, x_z_raw / x_mag)
        }
        Some(q) => {
            // Body X rotated into ECEF, projected onto the plane ⊥ boresight (Z-axis):
            let (bx, by, bz) = quaternion_rotate(q, (1.0, 0.0, 0.0));
            let dot = bx * z_x + by * z_y + bz * z_z;
            let (x_x_raw, x_y_raw, x_z_raw) = (bx - dot * z_x, by - dot * z_y, bz - dot * z_z);
            let x_mag = (x_x_raw * x_x_raw + x_y_raw * x_y_raw + x_z_raw * x_z_raw).sqrt();
            if x_mag < X_MAG_HARD_ERROR_THRESHOLD {
                return Err(AntennaModelError::InvalidCoordinate {
                    param: "vehicle_attitude".to_string(),
                    reason: "attitude body X-axis is parallel to boresight; \
                             azimuth reference is degenerate"
                        .to_string(),
                });
            }
            if x_mag < X_MAG_NEAR_DEGENERATE_THRESHOLD {
                tracing::warn!(
                    x_mag,
                    "body X-axis is nearly parallel to boresight \
                     (projected magnitude {x_mag:.2e} < {X_MAG_NEAR_DEGENERATE_THRESHOLD}); \
                     azimuth reference is poorly conditioned"
                );
            }
            (x_x_raw / x_mag, x_y_raw / x_mag, x_z_raw / x_mag)
        }
    };

    // Y-axis: cross product of Z with X (completes right-hand system)
    let y_x = z_y * x_z - z_z * x_y;
    let y_y = z_z * x_x - z_x * x_z;
    let y_z = z_x * x_y - z_y * x_x;

    Ok(((x_x, x_y, x_z), (y_x, y_y, y_z), z_unit))
}

/// Compute emitter direction (azimuth, elevation) using reflector boresight for orientation,
/// with an optional vehicle attitude quaternion.
///
/// When `attitude` is `None`, azimuth zero is derived from the Earth-Z / East cross-product
/// heuristic (see `antenna_frame_axes`).
///
/// # Arguments
/// - `emitter_pos`: Emitter position (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle position (ECEF or Geodetic)
/// - `boresight_pos`: Reflector boresight position (ECEF or Geodetic)
/// - `attitude`: Optional unit quaternion `[w, x, y, z]` body → ECEF.
///   Body +Z = boresight, body +X = azimuth-zero (E-clock zero) reference.
///
/// # Returns
/// `(azimuth_deg, elevation_deg)` in antenna frame.
/// `azimuth_deg` is normalized to [0, 360).
pub fn compute_emitter_direction_with_attitude(
    emitter_pos: &Position3D,
    vehicle_pos: &Position3D,
    boresight_pos: &Position3D,
    attitude: Option<[f64; 4]>,
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

    // Build antenna frame axes (X, Y, Z unit vectors)
    let ((x_x, x_y, x_z), (y_x, y_y, y_z), _) = antenna_frame_axes((z_x, z_y, z_z), attitude)?;

    // Transform emitter to antenna frame
    let antenna_x = emitter_dx * x_x + emitter_dy * x_y + emitter_dz * x_z;
    let antenna_y = emitter_dx * y_x + emitter_dy * y_y + emitter_dz * y_z;
    let antenna_z = emitter_dx * z_x + emitter_dy * z_y + emitter_dz * z_z;

    // Convert to spherical coordinates
    let (azimuth_deg, elevation_deg, _range) =
        antenna_frame_to_spherical(antenna_x, antenna_y, antenna_z)?;

    Ok((normalize_azimuth_deg(azimuth_deg), elevation_deg))
}

/// Compute emitter direction (azimuth, elevation) using reflector boresight for orientation.
///
/// The boresight vector (from vehicle to reflector_boresight) establishes the antenna Z-axis.
///
/// **Azimuth zero** is derived from the Earth-Z / East cross-product heuristic (no attitude
/// supplied). This is an **approximate** reference that is **discontinuous** near boresight
/// ∥ Earth-Z (|z_z| ≈ 1). For a stable, deterministic azimuth reference that matches a
/// calibration E-clock definition, supply a vehicle attitude quaternion via
/// `compute_emitter_direction_with_attitude`.
///
/// # Arguments
/// - `emitter_pos`: Emitter position (ECEF or Geodetic)
/// - `vehicle_pos`: Vehicle position (ECEF or Geodetic)
/// - `boresight_pos`: Reflector boresight position (ECEF or Geodetic)
///
/// # Returns
/// `(azimuth_deg, elevation_deg)` in antenna frame.
/// `azimuth_deg` is normalized to [0, 360) so that it is compatible with
/// correction-surface knot ranges and coverage checks, which use [0, 360) throughout.
pub fn compute_emitter_direction(
    emitter_pos: &Position3D,
    vehicle_pos: &Position3D,
    boresight_pos: &Position3D,
) -> Result<(f64, f64)> {
    compute_emitter_direction_with_attitude(emitter_pos, vehicle_pos, boresight_pos, None)
}

// ============================================================================
// Feed Position Computation from Pointing Directions
// ============================================================================

/// Beam deviation factor for a paraboloid with a laterally displaced feed.
///
/// A feed tilted by angle ψ off-axis steers the beam by only BDF·ψ. Using the
/// classical approximation (Y.T. Lo, "On the beam deviation factor of a
/// parabolic reflector", 1960):
/// ```text
/// BDF = (1 + K·(D/4f)²) / (1 + (D/4f)²),  K ≈ 0.36 for typical tapers
/// ```
/// For f/D = 0.5 this gives ≈ 0.871; BDF → 1 as f/D → ∞ (flat reflector limit).
///
/// Steering code divides the required lateral displacement by BDF so the beam
/// lands at the requested angle.
///
/// Requires `f_over_d > 0` (a physical reflector); callers obtain it from a
/// validated `ReflectorGeometry`. At `f_over_d = 0` the result is `NaN`.
pub fn beam_deviation_factor(f_over_d: f64) -> f64 {
    const K: f64 = 0.36;
    let x = 1.0 / (4.0 * f_over_d);
    (1.0 + K * x * x) / (1.0 + x * x)
}

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
/// - `reflector_diameter`: Antenna reflector diameter in meters (used to compute the
///   beam deviation factor so the steered beam peak lands at the requested angle)
/// - `attitude`: Optional unit quaternion `[w, x, y, z]` body → ECEF.
///   When `Some`, body +X defines azimuth zero for consistent E-clock reference.
///   When `None`, uses the Earth-Z heuristic (approximate, discontinuous near boresight ∥ Earth-Z).
///
/// # Returns
/// Physical feed position (x, y, z) in antenna frame relative to reflector vertex (meters)
pub fn compute_feed_position_from_pointing(
    feed_pointing_pos: &Position3D,
    reflector_pointing_pos: &Position3D,
    vehicle_pos: &Position3D,
    focal_length: f64,
    reflector_diameter: f64,
    attitude: Option<[f64; 4]>,
) -> Result<(f64, f64, f64)> {
    // Compute pointing directions for both feed and reflector
    let (feed_az, feed_el) = compute_emitter_direction_with_attitude(
        feed_pointing_pos,
        vehicle_pos,
        reflector_pointing_pos,
        attitude,
    )?;
    let (refl_az, refl_el) = compute_emitter_direction_with_attitude(
        reflector_pointing_pos,
        vehicle_pos,
        reflector_pointing_pos,
        attitude,
    )?;

    // Reflector boresight should be at (0, 0) by definition
    // The angular offset between feed and reflector is the feed displacement angle.
    // Both feed_az and refl_az are now in [0, 360) (normalized by compute_emitter_direction);
    // delta_az is therefore in (-360, 360), which EClockConeCoordinates::from_azimuth_elevation
    // accepts because clock angle is periodic and the difference is always small in practice.
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

    // Convert angular offset to physical feed position, correcting for the
    // beam deviation factor so the steered beam lands at the requested angle.
    let bdf = beam_deviation_factor(focal_length / reflector_diameter);
    let (x, y, z) = ecc.to_feed_position_with_bdf(focal_length, bdf);

    Ok((x, y, z))
}

// ============================================================================
// Beam Squint Correction
// ============================================================================

/// Apply beam squint correction for frequency-dependent beam pointing.
///
/// When the antenna is mechanically pointed at `pointing_frequency` but
/// operating at `operating_frequency`, the beam direction shifts due to
/// frequency-dependent phase effects. The squint magnitude depends on the
/// actual feed displacement from the focal point.
///
/// The squint is applied in direction-cosine (u, v) space along the
/// feed-displacement clock angle, giving correct results for all displacement
/// directions — not just elevation.
///
/// # Arguments
/// - `azimuth_deg`: Uncorrected azimuth (degrees, 0° = +X, 90° = +Y, [0,360))
/// - `elevation_deg`: Uncorrected elevation (POLAR ANGLE from boresight, degrees)
/// - `pointing_freq_mhz`: Frequency at which antenna is pointed
/// - `operating_freq_mhz`: Actual operating frequency
/// - `feed_displacement_m`: Radial feed displacement from focal point (meters)
/// - `focal_length_m`: Focal length of the reflector (meters)
/// - `displacement_clock_angle_rad`: Clock angle of the **combined** feed lateral
///   position in the antenna frame (radians). This is `atan2(feed_y, feed_x)` of
///   the total lateral offset — i.e. the vector sum of the design feed offset and
///   any steering offset — not just the steering component alone.
///
/// # Returns
/// (corrected_azimuth_deg, corrected_elevation_deg, squint_magnitude_deg)
/// - `corrected_azimuth_deg` is normalized to [0, 360)
/// - `corrected_elevation_deg` (polar angle from boresight) is always >= 0
///
/// # Physics
/// Beam squint occurs because the phase gradient across the aperture changes
/// with frequency when the feed is displaced. The squint angle is approximately:
/// ```text
/// Δθ ≈ (f_op - f_point) / f_point × (δ / f)
/// ```
/// where δ is feed displacement and f is focal length. The squint vector is
/// applied in (u, v) direction-cosine space along the displacement clock angle:
/// ```text
/// u = sin(θ)·cos(φ),  v = sin(θ)·sin(φ)
/// u' = u + Δθ·cos(clock),  v' = v + Δθ·sin(clock)
/// θ' = asin(sqrt(u'²+v'²)),  φ' = atan2(v', u')
/// ```
pub fn apply_beam_squint_correction(
    azimuth_deg: f64,
    elevation_deg: f64,
    pointing_freq_mhz: f64,
    operating_freq_mhz: f64,
    feed_displacement_m: f64,
    focal_length_m: f64,
    displacement_clock_angle_rad: f64,
) -> (f64, f64, f64) {
    debug_assert!(elevation_deg >= 0.0, "elevation must be polar angle >= 0");
    // Polar angle is non-negative by definition (acos range [0, 180]); enforce it in
    // release builds too so the direction-cosine path below never sees a negative sin(theta).
    let elevation_deg = elevation_deg.abs();

    // If frequencies are the same (within 0.1%), no correction needed
    if (pointing_freq_mhz - operating_freq_mhz).abs() / pointing_freq_mhz < 0.001 {
        return (normalize_azimuth_deg(azimuth_deg), elevation_deg.abs(), 0.0);
    }

    // If no feed displacement, no beam squint
    if feed_displacement_m < 1e-6 {
        return (normalize_azimuth_deg(azimuth_deg), elevation_deg.abs(), 0.0);
    }

    // Beam squint magnitude: Δθ ≈ (f_op - f_point) / f_point × (δ / f)
    let freq_shift_ratio = (operating_freq_mhz - pointing_freq_mhz) / pointing_freq_mhz;
    let displacement_ratio = feed_displacement_m / focal_length_m;
    let squint_rad = freq_shift_ratio * displacement_ratio;

    // Convert current (azimuth, elevation) to direction cosines (u, v).
    // elevation_deg is the POLAR ANGLE from boresight (+Z); azimuth_deg is the clock angle.
    // u = sin(θ)·cos(φ),  v = sin(θ)·sin(φ)
    let theta_rad = elevation_deg.to_radians();
    let phi_rad = azimuth_deg.to_radians();
    let sin_theta = theta_rad.sin();
    let u = sin_theta * phi_rad.cos();
    let v = sin_theta * phi_rad.sin();

    // Apply squint along the displacement clock angle in (u, v) space.
    let u_new = u + squint_rad * displacement_clock_angle_rad.cos();
    let v_new = v + squint_rad * displacement_clock_angle_rad.sin();

    // Convert back to (θ', φ').
    // θ' = asin(sqrt(u'²+v'²)), clamped to [0,1] to guard against floating-point overshoot.
    let sin_theta_new = (u_new * u_new + v_new * v_new).sqrt().min(1.0);
    let theta_new_rad = sin_theta_new.asin(); // always in [0, π/2]

    // φ' = atan2(v', u'); atan2(0,0) is 0 which is fine at/near boresight.
    let phi_new_rad = v_new.atan2(u_new);

    let corrected_elevation = theta_new_rad.to_degrees(); // >= 0 by construction
    let corrected_azimuth = normalize_azimuth_deg(phi_new_rad.to_degrees());

    (
        corrected_azimuth,
        corrected_elevation,
        squint_rad.abs().to_degrees(),
    )
}

/// Apply frequency-offset beam squint to an emitter direction.
///
/// Computes the feed radial displacement and clock angle from `(feed_x, feed_y)` and,
/// when the operating and pointing frequencies differ by more than 0.1 MHz, shifts the
/// direction in direction-cosine `(u, v)` space via [`apply_beam_squint_correction`].
/// Otherwise the input direction is returned unchanged.
///
/// This is the single source of truth shared by the `/gain` evaluator and the `/h3`
/// link budget. The `pointing_frequency_mhz.unwrap_or(operating)` defaulting is the
/// caller's responsibility; this function takes an explicit pointing frequency.
///
/// Returns `(az_deg, el_deg, squint_deg)`; `squint_deg == 0.0` when no correction applies.
/// The returned `squint_deg` magnitude depends only on the feed displacement and the
/// frequency offset, not on the input direction.
///
/// Note: this 0.1 MHz *absolute* gate preserves the original `/gain` evaluator behavior
/// and is distinct from the 0.1%-*relative* gate inside `apply_beam_squint_correction`;
/// in the target bands (1–30 GHz) the absolute gate is the stricter of the two, so the
/// delegate's relative gate never independently suppresses a correction this gate admits.
pub fn squint_corrected_direction(
    az_deg: f64,
    el_deg: f64,
    operating_freq_mhz: f64,
    pointing_freq_mhz: f64,
    feed_x: f64,
    feed_y: f64,
    focal_length_m: f64,
) -> (f64, f64, f64) {
    if (pointing_freq_mhz - operating_freq_mhz).abs() <= 0.1 {
        return (az_deg, el_deg, 0.0);
    }
    let feed_displacement_m = feed_x.hypot(feed_y);
    let displacement_clock_angle_rad = feed_y.atan2(feed_x);
    // Note: apply_beam_squint_correction takes (pointing, operating) in args 3-4 —
    // the reverse of this wrapper's (operating, pointing) parameter order.
    apply_beam_squint_correction(
        az_deg,
        el_deg,
        pointing_freq_mhz,
        operating_freq_mhz,
        feed_displacement_m,
        focal_length_m,
        displacement_clock_angle_rad,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-6;

    // ========================================================================
    // Domain-contract invariants (docs/domain-contract.md)
    // ========================================================================

    /// ENU rotation matrix must be orthogonal: R · Rᵀ = I. (Contract: Transforms.)
    #[test]
    fn test_ecef_to_enu_rotation_is_orthogonal() {
        let cases: [(f64, f64); 4] = [(0.0, 0.0), (45.0, -122.0), (89.0, 179.0), (-45.0, 30.0)];
        for (lat_deg, lon_deg) in cases {
            let r = ecef_to_enu_rotation(lat_deg.to_radians(), lon_deg.to_radians());
            for i in 0..3 {
                for j in 0..3 {
                    let dot: f64 = (0..3).map(|k| r[i][k] * r[j][k]).sum();
                    let expected = if i == j { 1.0 } else { 0.0 };
                    assert!(
                        (dot - expected).abs() < 1e-10,
                        "R*R^T[{i}][{j}] = {dot}, expected {expected} at lat={lat_deg} lon={lon_deg}"
                    );
                }
            }
        }
    }

    /// ENU→ECEF must use Rᵀ, not R (the anchor bug). A pure "Up" ENU vector must
    /// map, via Rᵀ, to the local vertical in ECEF. (Contract: ENU axis gotcha.)
    #[test]
    fn test_enu_ecef_roundtrip_uses_transpose() {
        let (lat_deg, lon_deg): (f64, f64) = (34.2, -118.1);
        let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
        let r = ecef_to_enu_rotation(lat, lon);
        // enu_up = (0, 0, 1); Rᵀ·enu_up selects row 2 of R (the Up basis vector in ECEF).
        let enu_up = (0.0, 0.0, 1.0);
        let ecef = (
            r[0][0] * enu_up.0 + r[1][0] * enu_up.1 + r[2][0] * enu_up.2,
            r[0][1] * enu_up.0 + r[1][1] * enu_up.1 + r[2][1] * enu_up.2,
            r[0][2] * enu_up.0 + r[1][2] * enu_up.1 + r[2][2] * enu_up.2,
        );
        let expected = (lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        assert!(
            (ecef.0 - expected.0).abs() < 1e-10,
            "up.x {} vs {}",
            ecef.0,
            expected.0
        );
        assert!(
            (ecef.1 - expected.1).abs() < 1e-10,
            "up.y {} vs {}",
            ecef.1,
            expected.1
        );
        assert!(
            (ecef.2 - expected.2).abs() < 1e-10,
            "up.z {} vs {}",
            ecef.2,
            expected.2
        );
    }

    /// normalize_azimuth_deg output is always in [0, 360). (Contract: Transforms.)
    #[test]
    fn test_normalize_azimuth_deg_boundaries() {
        assert_eq!(normalize_azimuth_deg(360.0), 0.0);
        assert!((0.0..360.0).contains(&normalize_azimuth_deg(-0.0001)));
        assert!((normalize_azimuth_deg(720.5) - 0.5).abs() < 1e-9);
        assert!((0.0..360.0).contains(&normalize_azimuth_deg(-720.5)));
    }

    /// The squint wrapper passes (pointing, operating) to apply_beam_squint_correction,
    /// the reverse of its own (operating, pointing) parameter order. If that internal
    /// swap is ever un-done, the two calls diverge. (Contract: squint arg-order trap.)
    #[test]
    fn test_squint_corrected_direction_frequency_argument_order() {
        // squint_corrected_direction(az, el, operating, pointing, feed_x, feed_y, focal)
        let (az, el, squint) = squint_corrected_direction(10.0, 5.0, 8500.0, 8400.0, 0.5, 0.0, 5.0);
        // Equivalent direct call: apply_beam_squint_correction(az, el, pointing, operating,
        //   feed_displacement = hypot(feed_x, feed_y), focal, clock = atan2(feed_y, feed_x)).
        let (az2, el2, squint2) =
            apply_beam_squint_correction(10.0, 5.0, 8400.0, 8500.0, 0.5, 5.0, 0.0);
        assert!((az - az2).abs() < 1e-9, "az {az} vs {az2}");
        assert!((el - el2).abs() < 1e-9, "el {el} vs {el2}");
        assert!(
            (squint - squint2).abs() < 1e-9,
            "squint {squint} vs {squint2}"
        );
        // Sanity: with a 100 MHz gap and real feed offset, a correction actually occurred.
        assert!(squint.abs() > 0.0, "expected a nonzero squint correction");
    }

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
        // Just below threshold (6400 km = 6,400,000 m)
        let below = Position3D::new(6_399_000.0, 0.0, 0.0);
        assert!(!is_ecef_coordinates(&below));

        // Just above threshold
        let above = Position3D::new(6_401_000.0, 0.0, 0.0);
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
        // Same frequency → no squint; function should still normalise the outputs.
        let (az, el, squint) = apply_beam_squint_correction(
            45.0,   // azimuth
            30.0,   // elevation (polar from boresight)
            8400.0, // pointing freq
            8400.0, // operating freq (same → no squint)
            1.0,    // feed displacement (m)
            13.6,   // focal length (m)
            0.0,    // displacement clock angle (irrelevant here)
        );

        assert!((az - 45.0).abs() < EPSILON, "az={az}");
        assert!((el - 30.0).abs() < EPSILON, "el={el}");
        assert!(squint.abs() < EPSILON, "squint={squint}");
    }

    #[test]
    fn test_beam_squint_correction_applied() {
        // Feed displaced along +x (clock = 0°), boresight pointing (az=0, el=0).
        // Squint should shift in +u direction → elevation increases, azimuth stays near 0°.
        let clock_rad = 0.0_f64; // +x direction
        let (az, el, squint) = apply_beam_squint_correction(
            0.0,    // azimuth
            0.0,    // elevation (on boresight)
            8400.0, // pointing freq
            8450.0, // operating freq (slightly higher)
            1.0,    // feed displacement (m)
            13.6,   // focal length (m)
            clock_rad,
        );

        // Squint magnitude should be non-zero
        assert!(squint > 0.0, "squint={squint}");

        // Elevation should increase (beam moves away from boresight)
        assert!(el > 0.0, "el={el}");

        // Azimuth should still be near 0° (or 360°) since clock is 0°
        let az_near_zero = az < 1.0 || az > 359.0;
        assert!(az_near_zero, "az={az} should be near 0°/360° for clock=0°");
    }

    #[test]
    fn test_beam_squint_large_frequency_difference() {
        // Feed displaced along +x (clock = 0°), 1° off boresight along azimuth 0°.
        // 10x frequency ratio gives large squint.
        let (az, el, squint) = apply_beam_squint_correction(
            0.0,    // azimuth
            1.0,    // elevation (1 degree off boresight)
            300.0,  // pointing freq (MHz)
            3000.0, // operating freq (MHz) – 10x higher
            1.0,    // feed displacement (m)
            13.6,   // focal length (m)
            0.0,    // clock = 0° (feed along +x)
        );

        // Δθ ≈ (3000-300)/300 × 1/13.6 ≈ 9 × 0.0735 ≈ 0.66 rad ≈ 38°
        assert!(
            squint > 30.0,
            "Large frequency difference should produce significant squint, got {squint}"
        );

        // Elevation should be significantly shifted
        assert!(el > 30.0, "el={el}");

        // Azimuth should remain near 0° (clock=0° → squint along u axis)
        let az_near_zero = az < 5.0 || az > 355.0;
        assert!(az_near_zero, "az={az} should be near 0°/360° for clock=0°");
    }

    #[test]
    fn test_beam_squint_no_displacement() {
        // No feed displacement means no beam squint, even with large frequency difference.
        let (az, el, squint) = apply_beam_squint_correction(
            10.0,   // azimuth
            5.0,    // elevation (polar)
            300.0,  // pointing freq
            3000.0, // operating freq
            0.0,    // no displacement
            13.6,   // focal length
            0.0,    // clock angle (irrelevant)
        );

        // Normalisation is still applied, so check output is normalised/non-negative.
        assert!((az - 10.0).abs() < EPSILON, "az={az}");
        assert!((el - 5.0).abs() < EPSILON, "el={el}");
        assert!(squint.abs() < EPSILON, "squint={squint}");
    }

    /// Squint applied along clock angle 0° (+x) shifts beam in the u direction,
    /// not the v direction.  Starting from boresight (az=0, el=0) with a positive
    /// frequency shift and +x feed displacement, the corrected beam should land at
    /// azimuth ≈ 0° (or 360°) and elevation > 0°.
    #[test]
    fn test_beam_squint_clock_angle_x_direction() {
        let (az, el, squint) = apply_beam_squint_correction(
            0.0, // boresight
            0.0, 8400.0, 8500.0, // 100 MHz above pointing frequency
            1.0,    // 1 m lateral displacement
            13.6, 0.0_f64, // clock = 0° → feed along +x axis
        );

        assert!(squint > 0.0, "squint should be non-zero");
        assert!(el >= 0.0, "elevation must be >= 0, got {el}");
        // Beam squint along +x means azimuth stays near 0° (or 360°)
        let az_near_zero = az < 5.0 || az > 355.0;
        assert!(
            az_near_zero,
            "clock=0° squint should be along u; az={az} should be ~0 or ~360"
        );
    }

    /// Squint applied along clock angle 90° (+y) shifts beam in the v direction.
    /// From boresight with feed displaced in +y, the corrected beam should land at
    /// azimuth ≈ 90° and elevation > 0°.
    #[test]
    fn test_beam_squint_clock_angle_y_direction() {
        let (az, el, squint) = apply_beam_squint_correction(
            0.0, // boresight
            0.0,
            8400.0,
            8500.0, // 100 MHz above pointing frequency
            1.0,    // 1 m lateral displacement
            13.6,
            std::f64::consts::FRAC_PI_2, // clock = 90° → feed along +y axis
        );

        assert!(squint > 0.0, "squint should be non-zero");
        assert!(el >= 0.0, "elevation must be >= 0, got {el}");
        // Beam squint along +y means azimuth stays near 90°
        assert!(
            (az - 90.0).abs() < 5.0,
            "clock=90° squint should be along v; az={az} should be ~90°"
        );
    }

    /// Elevation returned by apply_beam_squint_correction must always be >= 0
    /// (it is a polar angle from boresight).
    #[test]
    fn test_beam_squint_elevation_always_nonnegative() {
        // Drive the squint in a negative direction (lower operating freq) from a small
        // positive elevation to try to push elevation negative.
        let (_, el, _) = apply_beam_squint_correction(
            0.0, 0.5, // small positive elevation
            8400.0, 8300.0, // lower operating freq → negative squint in u
            2.0, 13.6, 0.0, // clock = 0°
        );
        assert!(el >= 0.0, "elevation must be >= 0, got {el}");
    }

    /// Azimuth returned by apply_beam_squint_correction must be in [0, 360).
    #[test]
    fn test_beam_squint_azimuth_normalised() {
        // Use a negative raw azimuth input to exercise the normalisation path.
        let (az, el, _) = apply_beam_squint_correction(
            -30.0, // raw azimuth outside [0,360) – function should normalise
            10.0, 8400.0,
            8400.0, // same freq → passthrough, but normalisation still applied
            0.0, 13.6, 0.0,
        );
        assert!(
            az >= 0.0 && az < 360.0,
            "azimuth must be in [0,360), got {az}"
        );
        assert!(el >= 0.0, "elevation must be >= 0, got {el}");
    }

    #[test]
    fn test_beam_squint_applied_along_feed_clock_angle() {
        // Feed displaced along +Y (clock = 90°). Squint must move the beam in the
        // v (sin(theta)*sin(phi)) direction, leaving u (sin(theta)*cos(phi)) unchanged.
        let (az, el, squint) = apply_beam_squint_correction(
            0.0,
            2.0, // pointing: az=0, el=2 deg polar
            8400.0,
            8800.0, // freq offset
            1.0,
            13.6,                        // displacement, focal length
            std::f64::consts::FRAC_PI_2, // feed clock angle = +Y
        );
        assert!(squint > 0.0);
        let theta = el.to_radians();
        let phi = az.to_radians();
        let u = theta.sin() * phi.cos();
        // original u = sin(2 deg)*cos(0) ~ 0.0349 - must be unchanged by a +Y squint
        assert!(
            (u - 2.0_f64.to_radians().sin()).abs() < 1e-6,
            "u changed: {u}"
        );
        assert!(el >= 0.0);
    }

    // ========================================================================
    // squint_corrected_direction
    // ========================================================================

    #[test]
    fn test_squint_corrected_direction_no_offset_passthrough() {
        let (az, el, squint) =
            squint_corrected_direction(45.0, 10.0, 8400.0, 8400.05, 0.5, 0.0, 13.6);
        assert!((az - 45.0).abs() < 1e-9);
        assert!((el - 10.0).abs() < 1e-9);
        assert_eq!(squint, 0.0);
    }

    #[test]
    fn test_squint_corrected_direction_applies_when_offset() {
        let (az, el, squint) = squint_corrected_direction(0.0, 2.0, 8400.0, 8800.0, 1.0, 0.0, 13.6);
        assert!(squint > 0.0, "expected non-zero squint, got {squint}");
        assert!(
            (el - 2.0).abs() > 1e-6 || (az - 0.0).abs() > 1e-6,
            "direction should change"
        );
    }

    #[test]
    fn test_squint_corrected_direction_zero_displacement_no_squint() {
        let (az, el, squint) =
            squint_corrected_direction(30.0, 5.0, 8400.0, 8800.0, 0.0, 0.0, 13.6);
        assert!((az - 30.0).abs() < 1e-9);
        assert!((el - 5.0).abs() < 1e-9);
        assert_eq!(squint, 0.0);
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

        // Azimuth is now normalized to [0, 360) by compute_emitter_direction
        assert!(
            azimuth >= 0.0 && azimuth < 360.0,
            "Azimuth {} out of range [0, 360)",
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

    #[test]
    fn test_emitter_azimuth_normalized_to_0_360() {
        // Raw atan2 azimuth for this geometry is ≈ -170.60° (verified empirically);
        // after normalization it must land in (180, 360), proving the wrap occurred.
        let vehicle = Position3D::new(0.0, 0.0, 42_000_000.0);
        let boresight = Position3D::new(0.0, 0.0, 0.0);
        let emitter = Position3D::new(-1.0, -1.0, 100.0);
        let (az, _el) = compute_emitter_direction(&emitter, &vehicle, &boresight).unwrap();
        assert!((0.0..360.0).contains(&az), "az={az} outside [0, 360)");
        assert!(
            az > 180.0,
            "az={az}: raw azimuth was negative, wrap should land in (180, 360)"
        );
    }

    #[test]
    fn test_ecef_to_geodetic_pole_with_altitude() {
        for lat in [90.0, -90.0] {
            let (x, y, z) = geodetic_to_ecef(0.0, lat, 1000.0).unwrap();
            let (_lon, lat2, alt2) = ecef_to_geodetic(x, y, z).unwrap();
            assert!((lat2 - lat).abs() < 1e-9, "lat {lat}: got {lat2}");
            assert!((alt2 - 1000.0).abs() < 1e-3, "lat {lat}: alt {alt2}");
        }
    }

    // ========================================================================
    // quaternion_rotate Direct Tests
    // ========================================================================

    /// Tolerance for quaternion_rotate output comparisons.
    const Q_TOL: f64 = 1e-9;

    #[test]
    fn test_quaternion_rotate_identity() {
        // Identity quaternion [1, 0, 0, 0] must leave every vector unchanged.
        let q = [1.0_f64, 0.0, 0.0, 0.0];
        let v = (1.0_f64, 2.0, 3.0);
        let (rx, ry, rz) = quaternion_rotate(q, v);
        assert!(
            (rx - v.0).abs() < Q_TOL,
            "identity rotate x: expected {}, got {}",
            v.0,
            rx
        );
        assert!(
            (ry - v.1).abs() < Q_TOL,
            "identity rotate y: expected {}, got {}",
            v.1,
            ry
        );
        assert!(
            (rz - v.2).abs() < Q_TOL,
            "identity rotate z: expected {}, got {}",
            v.2,
            rz
        );
    }

    #[test]
    fn test_quaternion_rotate_90deg_about_z() {
        // +90° rotation about Z: q = [cos(45°), 0, 0, sin(45°)] = [√½, 0, 0, √½].
        // Maps (1, 0, 0) → (0, 1, 0) by the right-hand rule.
        let s = std::f64::consts::FRAC_1_SQRT_2; // ≈ 0.70710678
        let q = [s, 0.0, 0.0, s]; // [w, x, y, z]
        let v = (1.0_f64, 0.0, 0.0);
        let (rx, ry, rz) = quaternion_rotate(q, v);
        assert!(
            (rx - 0.0).abs() < Q_TOL,
            "+90° about Z: rx expected 0, got {}",
            rx
        );
        assert!(
            (ry - 1.0).abs() < Q_TOL,
            "+90° about Z: ry expected 1, got {}",
            ry
        );
        assert!(
            (rz - 0.0).abs() < Q_TOL,
            "+90° about Z: rz expected 0, got {}",
            rz
        );
    }

    #[test]
    fn test_quaternion_rotate_180deg_about_x() {
        // 180° rotation about X: q = [cos(90°), sin(90°), 0, 0] = [0, 1, 0, 0].
        // Maps (0, 1, 0) → (0, -1, 0).
        let q = [0.0_f64, 1.0, 0.0, 0.0]; // [w, x, y, z]
        let v = (0.0_f64, 1.0, 0.0);
        let (rx, ry, rz) = quaternion_rotate(q, v);
        assert!(
            (rx - 0.0).abs() < Q_TOL,
            "180° about X: rx expected 0, got {}",
            rx
        );
        assert!(
            (ry - (-1.0)).abs() < Q_TOL,
            "180° about X: ry expected -1, got {}",
            ry
        );
        assert!(
            (rz - 0.0).abs() < Q_TOL,
            "180° about X: rz expected 0, got {}",
            rz
        );
    }

    // ========================================================================
    // Attitude Quaternion Tests
    // ========================================================================

    /// Test helper: quaternion mapping body X → `x_ecef`, body Z → `z_ecef` (orthonormal inputs).
    #[cfg(test)]
    fn quaternion_from_axes(x_ecef: (f64, f64, f64), z_ecef: (f64, f64, f64)) -> [f64; 4] {
        let (x, z) = (x_ecef, z_ecef);
        let y = (
            z.1 * x.2 - z.2 * x.1, // y = z × x completes the right-handed frame
            z.2 * x.0 - z.0 * x.2,
            z.0 * x.1 - z.1 * x.0,
        );
        // Rotation matrix with columns [x y z]; convert via Shepperd's method.
        let (m00, m01, m02) = (x.0, y.0, z.0);
        let (m10, m11, m12) = (x.1, y.1, z.1);
        let (m20, m21, m22) = (x.2, y.2, z.2);
        let trace = m00 + m11 + m22;
        if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [s / 4.0, (m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s]
        } else if m00 > m11 && m00 > m22 {
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            [(m21 - m12) / s, s / 4.0, (m01 + m10) / s, (m02 + m20) / s]
        } else if m11 > m22 {
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            [(m02 - m20) / s, (m01 + m10) / s, s / 4.0, (m12 + m21) / s]
        } else {
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            [(m10 - m01) / s, (m02 + m20) / s, (m12 + m21) / s, s / 4.0]
        }
    }

    #[test]
    fn test_attitude_defines_azimuth_zero() {
        let vehicle = Position3D::new(7_000_000.0, 0.0, 0.0);
        let boresight = Position3D::new(8_000_000.0, 0.0, 0.0);
        let emitter = Position3D::new(8_000_000.0, 50_000.0, 0.0);
        // body Z (boresight) → ECEF +X, body X → ECEF +Y:
        let q_a = quaternion_from_axes((0.0, 1.0, 0.0), (1.0, 0.0, 0.0)); // (body_x_in_ecef, body_z_in_ecef)
        let (az_a, _el) =
            compute_emitter_direction_with_attitude(&emitter, &vehicle, &boresight, Some(q_a))
                .unwrap();
        assert!(
            az_a.abs() < 1e-6 || (az_a - 360.0).abs() < 1e-6,
            "emitter on body-X: az {az_a}"
        );
        let q_b = quaternion_from_axes((0.0, 0.0, 1.0), (1.0, 0.0, 0.0)); // body X = ECEF +Z
        let (az_b, _el) =
            compute_emitter_direction_with_attitude(&emitter, &vehicle, &boresight, Some(q_b))
                .unwrap();
        assert!((az_b - 270.0).abs() < 1e-6, "rotated frame: az {az_b}");
    }

    #[test]
    fn test_attitude_none_matches_original_behaviour() {
        // Verify the None path is exactly equivalent to compute_emitter_direction.
        let emitter = Position3D::new(-1.0, -1.0, 100.0);
        let vehicle = Position3D::new(0.0, 0.0, 42_000_000.0);
        let boresight = Position3D::new(0.0, 0.0, 0.0);
        let (az1, el1) = compute_emitter_direction(&emitter, &vehicle, &boresight).unwrap();
        let (az2, el2) =
            compute_emitter_direction_with_attitude(&emitter, &vehicle, &boresight, None).unwrap();
        assert_eq!(az1, az2, "azimuth must be identical for None path");
        assert_eq!(el1, el2, "elevation must be identical for None path");
    }

    #[test]
    fn test_attitude_degenerate_body_x_parallel_to_boresight() {
        // body Z → ECEF +X, body X → ECEF +X (same as boresight!) → degenerate
        let vehicle = Position3D::new(7_000_000.0, 0.0, 0.0);
        let boresight = Position3D::new(8_000_000.0, 0.0, 0.0);
        let emitter = Position3D::new(8_000_000.0, 50_000.0, 0.0);
        // Use a valid quaternion where body X ∥ boresight: body X = ECEF +X, body Z = ECEF +Y
        // (vehicle→boresight is ECEF +X, so body X = ECEF +X → projection onto ⊥boresight = 0)
        let q_deg = quaternion_from_axes((1.0, 0.0, 0.0), (0.0, 1.0, 0.0));
        // Vehicle→boresight is ECEF +X direction, body X is also ECEF +X → degenerate
        let result =
            compute_emitter_direction_with_attitude(&emitter, &vehicle, &boresight, Some(q_deg));
        assert!(
            result.is_err(),
            "expected error for degenerate body-X ∥ boresight"
        );
    }
}
