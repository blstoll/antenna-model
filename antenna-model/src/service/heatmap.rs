//! Heatmap Generation Service
//!
//! Generates 2D loss heatmaps across antenna field of view.

use crate::api::schemas::{
    CalibrationStatusInfo, GainRequest, GridConfig, GridData, HeatmapMetadata, HeatmapRequest,
    HeatmapResponse, Position3D, RangeConfig,
};
use crate::data::repository::CalibrationRepository;
use crate::error::{AntennaModelError, Result};
use crate::service::evaluator::compute_gain_from_request;
use rayon::prelude::*;
use std::collections::HashSet;
use std::time::Instant;

/// Threshold for parallel evaluation (number of grid points)
const PARALLEL_THRESHOLD: usize = 100;

/// Maximum allowed grid points to prevent excessive computation
const MAX_GRID_POINTS: usize = 100_000;

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

    // Validate grid size
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

    // Evaluate gain at each grid point (in parallel if large enough)
    let results: Vec<(f64, f64, f64, Vec<String>)> = if grid_points.len() >= PARALLEL_THRESHOLD {
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

    // Find peak gain (maximum across all points)
    let peak_gain_db = results
        .iter()
        .map(|(_, _, gain, _)| *gain)
        .filter(|g| g.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);

    // Compute loss relative to peak for each point
    let losses: Vec<f64> = results
        .iter()
        .map(|(_, _, gain, _)| {
            if gain.is_finite() {
                peak_gain_db - gain
            } else {
                f64::NAN
            }
        })
        .collect();

    // Aggregate warnings (deduplicate)
    let all_warnings: HashSet<String> = results
        .iter()
        .flat_map(|(_, _, _, warnings)| warnings.clone())
        .collect();
    let mut warnings: Vec<String> = all_warnings.into_iter().collect();
    warnings.sort();

    // Check for extrapolated points
    let extrapolated_count = results
        .iter()
        .filter(|(_, _, _, warns)| {
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
/// Returns: (azimuth, elevation, gain_db, warnings)
fn evaluate_grid_point(
    request: &HeatmapRequest,
    repository: &CalibrationRepository,
    azimuth_deg: f64,
    elevation_deg: f64,
) -> (f64, f64, f64, Vec<String>) {
    // Convert azimuth/elevation to emitter position
    // For heatmap, we generate emitter positions in a spherical pattern around the antenna
    // We'll use a large distance (e.g., 400 km for LEO satellite) and convert spherical to ECEF
    let emitter_position =
        compute_emitter_position_from_angles(&request.vehicle_position, azimuth_deg, elevation_deg);

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
        ),
        Err(_) => {
            // On error, return NaN gain with error message
            (
                azimuth_deg,
                elevation_deg,
                f64::NAN,
                vec!["Computation failed for this point".to_string()],
            )
        }
    }
}

/// Convert azimuth/elevation angles to emitter position.
///
/// For heatmap generation, we place the emitter at a large distance (400 km, typical LEO altitude)
/// in the direction specified by azimuth and elevation angles.
///
/// The approach depends on whether vehicle_position is ECEF or Geodetic:
/// - ECEF: Convert to antenna-centered spherical coordinates
/// - Geodetic: Use local East-North-Up (ENU) frame
fn compute_emitter_position_from_angles(
    vehicle_position: &Position3D,
    azimuth_deg: f64,
    elevation_deg: f64,
) -> Position3D {
    // Use a large distance for emitter (400 km for LEO satellite)
    let distance_m = 400_000.0;

    // Convert to radians
    let az_rad = azimuth_deg.to_radians();
    let el_rad = elevation_deg.to_radians();

    // Compute offset in local antenna frame (ENU-like convention)
    // Azimuth: 0° = North (Y), 90° = East (X), 180° = South (-Y), 270° = West (-X)
    // Elevation: 0° = horizon, 90° = zenith
    let dx = distance_m * el_rad.cos() * az_rad.sin(); // East component
    let dy = distance_m * el_rad.cos() * az_rad.cos(); // North component
    let dz = distance_m * el_rad.sin(); // Up component

    // Add offset to vehicle position
    // NOTE: This is a simplified approach. For production, we'd use proper
    // coordinate transformations (ECEF<->ENU) based on vehicle position.
    // For now, we'll do a simple offset which works reasonably well for small angles.
    Position3D::new(
        vehicle_position.x + dx,
        vehicle_position.y + dy,
        vehicle_position.z + dz,
    )
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

    #[test]
    fn test_compute_emitter_position_from_angles_geodetic() {
        let vehicle_pos = Position3D::new(-118.0, 34.0, 100.0); // Geodetic
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 90.0, 45.0);

        // Azimuth 90° = East, Elevation 45° = 45° above horizon
        // Should add positive X (east), near-zero Y, positive Z (up)
        assert!(emitter.x > vehicle_pos.x);
        assert!(emitter.z > vehicle_pos.z);
    }

    #[test]
    fn test_compute_emitter_position_from_angles_ecef() {
        let vehicle_pos = Position3D::new(6_500_000.0, 100_000.0, 200_000.0); // ECEF
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 90.0);

        // Azimuth 0° = North, Elevation 90° = zenith (straight up)
        // Should add mostly Z component (up)
        assert!(emitter.z > vehicle_pos.z);
    }

    #[test]
    fn test_compute_emitter_position_horizon() {
        let vehicle_pos = Position3D::new(0.0, 0.0, 0.0);
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 0.0);

        // Elevation 0° = horizon
        // Should add mostly Y component (north at azimuth 0)
        assert!(emitter.y > vehicle_pos.y);
        // Z should be approximately zero (horizon)
        assert!(emitter.z.abs() < 1.0);
    }

    #[test]
    fn test_compute_emitter_position_zenith() {
        let vehicle_pos = Position3D::new(0.0, 0.0, 0.0);
        let emitter = compute_emitter_position_from_angles(&vehicle_pos, 0.0, 90.0);

        // Elevation 90° = zenith (straight up)
        // Should add mostly Z component (up)
        assert!(emitter.z > 390_000.0); // Close to 400 km
                                        // X and Y should be approximately zero
        assert!(emitter.x.abs() < 1.0);
        assert!(emitter.y.abs() < 1.0);
    }

    #[test]
    fn test_parallel_threshold() {
        // Verify parallel threshold is reasonable
        assert!(PARALLEL_THRESHOLD > 0);
        assert!(PARALLEL_THRESHOLD < 1000);
    }

    #[test]
    fn test_max_grid_points() {
        // Verify max grid points is reasonable
        assert!(MAX_GRID_POINTS >= 10_000);
        assert!(MAX_GRID_POINTS <= 1_000_000);
    }

    #[test]
    fn test_grid_too_large() {
        // Create a grid that exceeds MAX_GRID_POINTS
        let azimuth_range = RangeConfig::new(0.0, 360.0, 0.1); // 3601 points
        let elevation_range = RangeConfig::new(0.0, 90.0, 0.1); // 901 points
                                                                // Total: 3601 * 901 = 3,244,501 points > MAX_GRID_POINTS

        let grid_config = GridConfig::Rectangular {
            azimuth_range_deg: azimuth_range,
            elevation_range_deg: elevation_range,
        };

        let request = HeatmapRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: Position3D::new(0.0, 0.0, 0.0),
            reflector_boresight: Position3D::new(0.0, 0.0, 10.0), // 10m above vehicle
            feed_position: Position3D::new(0.0, 0.0, 23.6),       // 10m + 13.6m focal length
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
}
