//! Heatmap Generation Service
//!
//! Generates 2D loss heatmaps across antenna field of view.

use crate::api::schemas::{
    CalibrationStatusInfo, GainRequest, GridConfig, GridData, HeatmapMetadata, HeatmapRequest,
    HeatmapResponse, Position3D, RangeConfig,
};
use crate::data::repository::CalibrationRepository;
use crate::error::{AntennaModelError, Result};
use crate::model::coordinates_3d::{ecef_to_enu_rotation, ecef_to_geodetic, geodetic_to_ecef};
use crate::service::evaluator::compute_gain_from_request;
use rayon::prelude::*;
use std::collections::HashSet;
use std::time::Instant;

/// Threshold for parallel evaluation (number of grid points)
const PARALLEL_THRESHOLD: usize = 100;

/// Sentinel loss value used when a grid point computation fails.
/// Using a large finite value avoids NaN serialization issues.
const FAILED_POINT_LOSS_DB: f64 = 999_999.0;

/// Type alias for grid generation results.
///
/// Contains:
/// - Vector of (azimuth, elevation) grid points in degrees
/// - Tuple of (azimuth_values, elevation_values) for rectangular grids
type GridPoints = (Vec<(f64, f64)>, (Vec<f64>, Vec<f64>));

/// Generate a heatmap for the given request.
///
/// This function:
/// 1. Generates a grid of emitter positions (rectangular or H3 hexagonal)
/// 2. Evaluates gain at each grid point
/// 3. Computes peak gain (reference)
/// 4. Calculates loss relative to peak for each point
/// 5. Returns formatted heatmap response
///
/// # Performance
///
/// Grid points are evaluated in parallel using rayon when grid size exceeds
/// PARALLEL_THRESHOLD (100 points). Expected performance:
/// - 72x46 rectangular grid (~3312 points): <2 seconds
/// - H3 resolution 7 (~5000 cells): <3 seconds
pub fn generate_heatmap(
    request: &HeatmapRequest,
    repository: &CalibrationRepository,
) -> Result<HeatmapResponse> {
    let start = Instant::now();

    // Generate grid points based on configuration
    let (grid_points, grid_coords) = generate_grid_points(&request.grid_config)?;

    // Note: the validator pre-filters requests exceeding MAX_HEATMAP_POINTS before this
    // function is called. The following is a defence-in-depth check matching that limit.
    const MAX_GRID_POINTS: usize = 100_000;
    if grid_points.len() > MAX_GRID_POINTS {
        return Err(AntennaModelError::Validation(
            crate::error::ValidationError::InvalidValue {
                param: "grid_config".to_string(),
                reason: format!(
                    "Grid size {} exceeds maximum allowed {} points",
                    grid_points.len(),
                    MAX_GRID_POINTS
                ),
            },
        ));
    }

    // Evaluate gain at each grid point (in parallel if large enough).
    // Returns (az, el, gain_db, warnings, is_failed).
    // Failed points use f64::NEG_INFINITY as gain (never NaN to avoid serialization issues).
    let results: Vec<(f64, f64, f64, Vec<String>, bool)> =
        if grid_points.len() >= PARALLEL_THRESHOLD {
            grid_points
                .par_iter()
                .map(|(az, el)| evaluate_grid_point(request, repository, *az, *el))
                .collect()
        } else {
            grid_points
                .iter()
                .map(|(az, el)| evaluate_grid_point(request, repository, *az, *el))
                .collect()
        };

    // Count failed points
    let failed_count = results
        .iter()
        .filter(|(_, _, _, _, failed)| *failed)
        .count();

    // Find peak gain (maximum across all successful points only)
    let peak_gain_db = results
        .iter()
        .filter(|(_, _, _, _, failed)| !failed)
        .map(|(_, _, gain, _, _)| *gain)
        .filter(|g| g.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);

    // Compute loss relative to peak for each point.
    // Failed points receive FAILED_POINT_LOSS_DB sentinel — never NaN.
    let losses: Vec<f64> = results
        .iter()
        .map(|(_, _, gain, _, failed)| {
            if *failed || !gain.is_finite() {
                FAILED_POINT_LOSS_DB
            } else {
                peak_gain_db - gain
            }
        })
        .collect();

    // Aggregate warnings (deduplicate)
    let all_warnings: HashSet<String> = results
        .iter()
        .flat_map(|(_, _, _, warnings, _)| warnings.clone())
        .collect();
    let mut warnings: Vec<String> = all_warnings.into_iter().collect();
    warnings.sort();

    // Check for extrapolated points
    let extrapolated_count = results
        .iter()
        .filter(|(_, _, _, warns, _)| {
            warns
                .iter()
                .any(|w| w.contains("extrapolat") || w.contains("out of range"))
        })
        .count();
    if extrapolated_count > 0 {
        warnings.insert(
            0,
            format!(
                "{} out of {} points were extrapolated",
                extrapolated_count,
                grid_points.len()
            ),
        );
    }

    // Format grid data based on grid type
    let grid_data = match &request.grid_config {
        GridConfig::Rectangular {
            azimuth_range_deg: _,
            elevation_range_deg: _,
        } => {
            let azimuth_values = grid_coords.0.clone();
            let elevation_values = grid_coords.1.clone();

            // Reshape losses into 2D array (rows = elevation, columns = azimuth)
            let num_az = azimuth_values.len();
            let num_el = elevation_values.len();
            let loss_db: Vec<Vec<f64>> = (0..num_el)
                .map(|el_idx| {
                    (0..num_az)
                        .map(|az_idx| losses[el_idx * num_az + az_idx])
                        .collect()
                })
                .collect();

            GridData::Rectangular {
                azimuth_values,
                elevation_values,
                loss_db,
            }
        }
        GridConfig::H3 { .. } => {
            // H3 support: for now, return error indicating it's not implemented
            return Err(AntennaModelError::NotImplemented {
                feature: "H3 hexagonal grid".to_string(),
            });
        }
    };

    let computation_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Get calibration status info from repository
    let calibration_status_info = repository
        .get_calibration(&request.antenna_id, &request.feed_id)
        .and_then(|cal| {
            cal.calibration_status.as_ref().map(|status| {
                // For heatmap, we don't track per-point correction application
                // Set correction_applied to true if any correction surface exists
                let mut info = CalibrationStatusInfo::from(status);
                info.correction_applied = cal.correction_surface.is_some();
                info
            })
        });

    Ok(HeatmapResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        frequency_mhz: request.frequency_mhz,
        grid: grid_data,
        warnings,
        metadata: HeatmapMetadata {
            points_evaluated: grid_points.len(),
            computation_time_ms,
            peak_gain_db,
            failed_points: failed_count,
        },
        calibration_status: calibration_status_info,
    })
}

/// Generate grid points based on grid configuration.
///
/// Returns:
/// - Vector of (azimuth, elevation) tuples in degrees
/// - Grid coordinates (azimuth_values, elevation_values) for rectangular grids
fn generate_grid_points(grid_config: &GridConfig) -> Result<GridPoints> {
    match grid_config {
        GridConfig::Rectangular {
            azimuth_range_deg,
            elevation_range_deg,
        } => generate_rectangular_grid(azimuth_range_deg, elevation_range_deg),
        GridConfig::H3 { .. } => Err(AntennaModelError::NotImplemented {
            feature: "H3 hexagonal grid".to_string(),
        }),
    }
}

/// Generate rectangular azimuth/elevation grid.
fn generate_rectangular_grid(
    azimuth_range: &RangeConfig,
    elevation_range: &RangeConfig,
) -> Result<GridPoints> {
    // Generate azimuth values
    let mut azimuth_values = Vec::new();
    let mut az = azimuth_range.min;
    while az <= azimuth_range.max + 1e-9 {
        // small tolerance for floating point
        azimuth_values.push(az);
        az += azimuth_range.step;
    }

    // Generate elevation values
    let mut elevation_values = Vec::new();
    let mut el = elevation_range.min;
    while el <= elevation_range.max + 1e-9 {
        elevation_values.push(el);
        el += elevation_range.step;
    }

    // Generate all combinations (row-major: elevation varies fastest in outer loop)
    let mut grid_points = Vec::new();
    for &el_val in &elevation_values {
        for &az_val in &azimuth_values {
            grid_points.push((az_val, el_val));
        }
    }

    Ok((grid_points, (azimuth_values, elevation_values)))
}

/// Evaluate gain at a single grid point.
///
/// Returns: `(azimuth, elevation, gain_db, warnings, is_failed)`.
/// On failure, `gain_db` is `f64::NEG_INFINITY` (never NaN) and `is_failed` is `true`.
fn evaluate_grid_point(
    request: &HeatmapRequest,
    repository: &CalibrationRepository,
    azimuth_deg: f64,
    elevation_deg: f64,
) -> (f64, f64, f64, Vec<String>, bool) {
    // Convert azimuth/elevation to emitter position using proper ECEF/ENU transformation
    let emitter_position = match compute_emitter_position_from_angles(
        &request.vehicle_position,
        azimuth_deg,
        elevation_deg,
    ) {
        Ok(pos) => pos,
        Err(_) => {
            return (
                azimuth_deg,
                elevation_deg,
                f64::NEG_INFINITY,
                vec!["Failed to compute emitter position for this point".to_string()],
                true,
            )
        }
    };

    // Create a GainRequest for this grid point
    let gain_request = GainRequest {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        vehicle_position: request.vehicle_position.clone(),
        reflector_boresight: request.reflector_boresight.clone(),
        feed_position: request.feed_position.clone(),
        emitter_position,
        frequency_mhz: request.frequency_mhz,
        pointing_frequency_mhz: request.pointing_frequency_mhz,
        include_reference: false, // Don't need reference for heatmap
    };

    // Evaluate gain at this point
    match compute_gain_from_request(&gain_request, repository) {
        Ok(response) => (
            azimuth_deg,
            elevation_deg,
            response.gain_db,
            response.warnings,
            false,
        ),
        Err(_) => (
            azimuth_deg,
            elevation_deg,
            f64::NEG_INFINITY,
            vec!["Computation failed for this point".to_string()],
            true,
        ),
    }
}

/// Convert azimuth/elevation angles to an emitter position in ECEF.
///
/// Places the emitter at 400 km distance from the vehicle in the direction
/// specified by azimuth/elevation in the local East-North-Up (ENU) frame.
///
/// Azimuth convention: 0° = North, 90° = East, 180° = South, 270° = West.
/// Elevation convention: 0° = horizon, 90° = zenith.
///
/// The vehicle position (ECEF or Geodetic) is converted to ECEF, and the ENU
/// rotation matrix at the vehicle's geodetic location is used to transform the
/// ENU offset into an ECEF offset. The result is always returned as an ECEF
/// `Position3D`.
fn compute_emitter_position_from_angles(
    vehicle_position: &Position3D,
    azimuth_deg: f64,
    elevation_deg: f64,
) -> Result<Position3D> {
    // Distance to emitter (400 km, typical LEO altitude)
    let distance_m = 400_000.0;

    // ENU offset from vehicle towards (az, el)
    let az_rad = azimuth_deg.to_radians();
    let el_rad = elevation_deg.to_radians();
    let e = distance_m * el_rad.cos() * az_rad.sin(); // East
    let n = distance_m * el_rad.cos() * az_rad.cos(); // North
    let u = distance_m * el_rad.sin(); // Up

    // Convert vehicle position to ECEF
    let (vx, vy, vz) = if vehicle_position.is_ecef() {
        (vehicle_position.x, vehicle_position.y, vehicle_position.z)
    } else {
        geodetic_to_ecef(vehicle_position.x, vehicle_position.y, vehicle_position.z)?
    };

    // Get geodetic lat/lon for ENU rotation matrix
    let (lon_deg, lat_deg, _) = ecef_to_geodetic(vx, vy, vz)?;
    let lat_rad = lat_deg.to_radians();
    let lon_rad = lon_deg.to_radians();

    // ENU-to-ECEF rotation matrix (columns are East, North, Up vectors in ECEF)
    let rot = ecef_to_enu_rotation(lat_rad, lon_rad);
    // rot maps ECEF → ENU; rot^T maps ENU → ECEF
    let dx = rot[0][0] * e + rot[1][0] * n + rot[2][0] * u;
    let dy = rot[0][1] * e + rot[1][1] * n + rot[2][1] * u;
    let dz = rot[0][2] * e + rot[1][2] * n + rot[2][2] * u;

    Ok(Position3D::new(vx + dx, vy + dy, vz + dz))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::repository::CalibrationRepository;

    #[test]
    fn test_generate_rectangular_grid_small() {
        let azimuth_range = RangeConfig::new(0.0, 10.0, 5.0);
        let elevation_range = RangeConfig::new(0.0, 10.0, 5.0);

        let (points, (az_vals, el_vals)) =
            generate_rectangular_grid(&azimuth_range, &elevation_range).unwrap();

        assert_eq!(az_vals, vec![0.0, 5.0, 10.0]);
        assert_eq!(el_vals, vec![0.0, 5.0, 10.0]);
        assert_eq!(points.len(), 9); // 3x3 grid

        // Check first few points (row-major: el varies in outer loop)
        assert_eq!(points[0], (0.0, 0.0)); // el=0, az=0
        assert_eq!(points[1], (5.0, 0.0)); // el=0, az=5
        assert_eq!(points[2], (10.0, 0.0)); // el=0, az=10
        assert_eq!(points[3], (0.0, 5.0)); // el=5, az=0
    }

    #[test]
    fn test_generate_rectangular_grid_typical() {
        let azimuth_range = RangeConfig::new(0.0, 360.0, 5.0);
        let elevation_range = RangeConfig::new(0.0, 90.0, 2.0);

        let (points, (az_vals, el_vals)) =
            generate_rectangular_grid(&azimuth_range, &elevation_range).unwrap();

        let expected_az = ((360.0_f64 - 0.0_f64) / 5.0_f64).ceil() as usize + 1;
        let expected_el = ((90.0_f64 - 0.0_f64) / 2.0_f64).ceil() as usize + 1;

        assert_eq!(az_vals.len(), expected_az);
        assert_eq!(el_vals.len(), expected_el);
        assert_eq!(points.len(), expected_az * expected_el);

        // Check bounds
        assert_eq!(az_vals[0], 0.0);
        assert!(az_vals.last().unwrap() >= &360.0);
        assert_eq!(el_vals[0], 0.0);
        assert!(el_vals.last().unwrap() >= &90.0);
    }

    #[test]
    fn test_generate_rectangular_grid_single_point() {
        let azimuth_range = RangeConfig::new(45.0, 45.0, 1.0);
        let elevation_range = RangeConfig::new(30.0, 30.0, 1.0);

        let (points, (az_vals, el_vals)) =
            generate_rectangular_grid(&azimuth_range, &elevation_range).unwrap();

        assert_eq!(az_vals, vec![45.0]);
        assert_eq!(el_vals, vec![30.0]);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], (45.0, 30.0));
    }

    /// Vehicle at north pole (geodetic), zenith (el=90°) should produce an emitter
    /// mostly in the ECEF +Z direction (towards North Pole zenith).
    #[test]
    fn test_compute_emitter_position_zenith_north_pole() {
        // North Pole: lon=0, lat=90, alt=0
        let vehicle_pos = Position3D::new(0.0, 90.0, 0.0);
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 90.0).unwrap();

        // Zenith at north pole points in +Z direction (ECEF).
        // Vehicle ECEF z ≈ 6356752, emitter z ≈ 6356752 + 400000 ≈ 6756752
        assert!(
            emitter.z > 6_700_000.0,
            "Expected emitter z > 6.7M, got z={}",
            emitter.z
        );
        // x and y should be near zero
        assert!(
            emitter.x.abs() < 1.0,
            "Expected emitter x ≈ 0, got {}",
            emitter.x
        );
        assert!(
            emitter.y.abs() < 1.0,
            "Expected emitter y ≈ 0, got {}",
            emitter.y
        );
    }

    /// Vehicle at equator, prime meridian (geodetic). Zenith (el=90°) should produce
    /// an emitter mostly in the ECEF +X direction.
    #[test]
    fn test_compute_emitter_position_zenith_equator() {
        let vehicle_pos = Position3D::new(0.0, 0.0, 0.0); // lon=0, lat=0, alt=0
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 90.0).unwrap();

        // Zenith at (lon=0, lat=0) points in +X direction (ECEF).
        // Vehicle ECEF x ≈ 6378137, emitter x ≈ 6378137 + 400000 ≈ 6778137
        assert!(
            emitter.x > 6_700_000.0,
            "Expected emitter x > 6.7M, got x={}",
            emitter.x
        );
        assert!(
            emitter.y.abs() < 1.0,
            "Expected emitter y ≈ 0, got {}",
            emitter.y
        );
        assert!(
            emitter.z.abs() < 1.0,
            "Expected emitter z ≈ 0, got {}",
            emitter.z
        );
    }

    /// Vehicle at equator, prime meridian. North (az=0, el=0) should produce an emitter
    /// in the ECEF +Z direction (north is up in ECEF at equator, prime meridian).
    #[test]
    fn test_compute_emitter_position_north_at_equator() {
        let vehicle_pos = Position3D::new(0.0, 0.0, 0.0);
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 0.0).unwrap();

        // North at (lon=0, lat=0) in ENU → +Z in ECEF
        // emitter.z should be ≈ 400000 above vehicle's z (≈ 0)
        assert!(
            emitter.z > 390_000.0,
            "Expected emitter z > 390km, got z={}",
            emitter.z
        );
    }

    /// ECEF vehicle position: zenith should move emitter in the "up" direction
    /// (away from Earth's center).
    #[test]
    fn test_compute_emitter_position_ecef_vehicle_zenith() {
        // GEO satellite at equator, prime meridian: (42164137, 0, 0)
        let vehicle_pos = Position3D::new(42164137.0, 0.0, 0.0);
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 90.0).unwrap();

        // Zenith at equator prime meridian → +X in ECEF
        assert!(
            emitter.x > vehicle_pos.x,
            "Emitter x should be beyond vehicle x"
        );
        assert!(
            (emitter.x - vehicle_pos.x - 400_000.0).abs() < 1.0,
            "Emitter should be ~400km above in x"
        );
    }

    /// Emitter result is always in ECEF (detectable by magnitude > 6.4M for Earth-surface orbit).
    #[test]
    fn test_compute_emitter_position_returns_ecef() {
        // Geodetic vehicle at 0 altitude, equator: ECEF radius ≈ 6378137
        let vehicle_pos = Position3D::new(0.0, 0.0, 0.0);
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 90.0).unwrap();

        // Result magnitude should be Earth radius + 400km ≈ 6778137 > 6.4M → ECEF
        let magnitude = (emitter.x.powi(2) + emitter.y.powi(2) + emitter.z.powi(2)).sqrt();
        assert!(
            magnitude > 6_400_000.0,
            "Expected ECEF magnitude > 6.4M, got {}",
            magnitude
        );
    }

    #[test]
    fn test_parallel_threshold() {
        // Verify parallel threshold is reasonable
        assert!(PARALLEL_THRESHOLD > 0);
        assert!(PARALLEL_THRESHOLD < 1000);
    }

    #[test]
    fn test_grid_too_large() {
        // Create a grid that exceeds the internal limit (100,000 points)
        // 317 × 317 = 100,489 points > 100,000 limit
        let azimuth_range = RangeConfig::new(0.0, 316.0, 1.0); // 317 points
        let elevation_range = RangeConfig::new(0.0, 316.0, 1.0); // 317 points

        let grid_config = GridConfig::Rectangular {
            azimuth_range_deg: azimuth_range,
            elevation_range_deg: elevation_range,
        };

        let request = HeatmapRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: Position3D::new(0.0, 0.0, 0.0),
            reflector_boresight: Position3D::new(0.0, 0.0, 10.0),
            feed_position: Position3D::new(0.0, 0.0, 23.6),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            grid_config,
        };

        let repository = CalibrationRepository::new();
        let result = generate_heatmap(&request, &repository);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AntennaModelError::Validation(_)));
    }

    #[test]
    fn test_h3_grid_not_implemented() {
        let grid_config = GridConfig::H3 {
            h3_resolution: 7,
            center_azimuth_deg: 180.0,
            center_elevation_deg: 45.0,
            field_of_view_deg: 30.0,
        };

        let result = generate_grid_points(&grid_config);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AntennaModelError::NotImplemented { .. }));
    }

    #[test]
    fn test_grid_generation_boundary_values() {
        // Test with fractional steps that don't divide evenly
        let azimuth_range = RangeConfig::new(0.0, 10.0, 3.0);
        let elevation_range = RangeConfig::new(0.0, 10.0, 4.0);

        let (points, (az_vals, el_vals)) =
            generate_rectangular_grid(&azimuth_range, &elevation_range).unwrap();

        // Should have 0, 3, 6, 9 for azimuth (step 3)
        // and 0, 4, 8 for elevation (step 4)
        assert!(az_vals.len() >= 4);
        assert!(el_vals.len() >= 3);
        assert_eq!(points.len(), az_vals.len() * el_vals.len());

        // Check that the max values are included (within tolerance)
        assert!(*az_vals.last().unwrap() >= 9.0);
        assert!(*el_vals.last().unwrap() >= 8.0);
    }

    /// Verify that a grid with partial failures returns a valid response without NaN values.
    #[test]
    fn test_partial_failures_no_nan_in_response() {
        use crate::data::types::{
            AntennaCalibration, CalibrationMetadata, CalibrationStatus, FeedParameters,
            MeshParameters, PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
        };

        // Build a minimal repository with a working antenna
        let mut repository = CalibrationRepository::new();
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-01T00:00:00Z")
            .format_version("2.0")
            .data_source("test")
            .rmse_db(0.5)
            .r_squared(0.99)
            .num_measurements(10)
            .build()
            .unwrap();
        let cal = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(PhysicalAntennaConfig {
                reflector: ReflectorGeometry {
                    diameter_m: 10.0,
                    focal_length_m: 5.0,
                    f_over_d_ratio: 0.5,
                    surface_rms_mm: 0.5,
                },
                feed: FeedParameters {
                    position: (0.0, 0.0, 0.0),
                    q_factor: 8.0,
                    phase_center_offset_m: 0.0,
                },
                mesh: Some(MeshParameters {
                    mesh_spacing_mm: 5.0,
                    wire_diameter_mm: 0.5,
                }),
            })
            .validity_ranges(ValidityRanges {
                azimuth_min_max: (0.0, 360.0),
                elevation_min_max: (0.0, 90.0),
                frequency_min_max: (1000.0, 10000.0),
                temperature_const: 290.0,
            })
            .calibration_status(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            })
            .build()
            .unwrap();
        repository.add_calibration(cal);

        // Request with antenna that doesn't exist → all points fail
        let grid_config = GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig::new(0.0, 10.0, 5.0),
            elevation_range_deg: RangeConfig::new(0.0, 10.0, 5.0),
        };
        let request = HeatmapRequest {
            antenna_id: "nonexistent_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: Position3D::new(0.0, 0.0, 0.0),
            reflector_boresight: Position3D::new(0.0, 0.0, 10.0),
            feed_position: Position3D::new(0.0, 0.0, 23.6),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            grid_config,
        };

        let result = generate_heatmap(&request, &repository).unwrap();

        // All points failed → failed_points == total points
        assert_eq!(
            result.metadata.failed_points,
            result.metadata.points_evaluated
        );

        // Verify no NaN values appear in the loss grid (JSON-serializable)
        let json = serde_json::to_string(&result).expect("Response must serialize without error");
        assert!(!json.contains("NaN"), "Response JSON must not contain NaN");

        // Failed points should use sentinel value
        if let GridData::Rectangular { ref loss_db, .. } = result.grid {
            for row in loss_db {
                for &val in row {
                    assert!(val.is_finite(), "Loss values must be finite, got {}", val);
                }
            }
        }
    }
}
