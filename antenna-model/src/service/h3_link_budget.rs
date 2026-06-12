//! H3 Link Budget Service
//!
//! Computes per-cell antenna gain, free-space path loss, and total path loss
//! for an H3 hexagonal grid centered on the feed pointing location.
//!
//! # Pipeline
//!
//! 1. Resolve H3 resolution from request (or derive from frequency)
//! 2. Find center H3 cell from feed position lat/lon
//! 3. Generate grid disk of N rings around center cell
//! 4. Build antenna configuration from calibration data
//! 5. For each cell (parallel): compute az/el, look up gain via cache, compute FSPL and path loss
//! 6. Return H3LinkBudgetResponse with per-cell results and metadata

use crate::api::schemas::{
    CalibrationStatusInfo, CoordinateSystem, H3CellResult, H3LinkBudgetRequest,
    H3LinkBudgetResponse, HeatmapMetadata, Position3D,
};
use crate::data::types::AntennaCalibration;
use crate::error::{AntennaModelError, Result};
use crate::model::compute_gain_db;
use crate::model::{
    compute_emitter_direction, compute_feed_position_from_pointing, ecef_to_geodetic,
    geodetic_to_ecef, AntennaConfiguration, FeedParameters as ModelFeedParams, FeedPosition,
    IntegrationParams, MeshParameters as ModelMeshParams, ReflectorGeometry as ModelReflector,
};
use crate::service::{GainCache, GainCacheKey};
use rayon::prelude::*;
use std::collections::HashSet;

/// Select H3 resolution from frequency (MHz):
/// - < 2000 MHz → 6
/// - 2000–8000 MHz → 7
/// - 8000–20000 MHz → 8
/// - > 20000 MHz → 9
pub fn h3_resolution_from_frequency(frequency_mhz: f64) -> u8 {
    match frequency_mhz {
        f if f < 2_000.0 => 6,
        f if f < 8_000.0 => 7,
        f if f < 20_000.0 => 8,
        _ => 9,
    }
}

/// Compute free-space path loss in dB: 20·log10(4π·d·f/c)
///
/// # Arguments
/// - `d_m`: Distance in meters
/// - `freq_hz`: Frequency in Hz
pub fn free_space_path_loss_db(d_m: f64, freq_hz: f64) -> f64 {
    const C: f64 = 299_792_458.0;
    20.0 * (4.0 * std::f64::consts::PI * d_m * freq_hz / C).log10()
}

/// Convert a Position3D to ECEF (x, y, z) in meters.
///
/// If geodetic (auto-detected by magnitude threshold), converts via `geodetic_to_ecef`.
/// Otherwise returns coordinates directly.
fn pos_to_ecef(pos: &Position3D) -> Result<(f64, f64, f64)> {
    if pos.is_ecef() {
        Ok((pos.x, pos.y, pos.z))
    } else {
        geodetic_to_ecef(pos.x, pos.y, pos.z)
    }
}

/// Build the antenna configuration from calibration data and feed pointing.
///
/// Returns `(AntennaConfiguration, feed_x, feed_y, feed_z)` where feed_xyz are
/// the physical feed position used for cache keying.
fn build_antenna_config(
    calibration: &AntennaCalibration,
    request: &H3LinkBudgetRequest,
) -> Result<(AntennaConfiguration, f64, f64, f64)> {
    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let diameter_m = calibration.physical_config.reflector.diameter_m;

    let reflector = ModelReflector::builder()
        .diameter(diameter_m)
        .focal_length(focal_length_m)
        .surface_rms(calibration.physical_config.reflector.surface_rms_mm / 1000.0)
        .build()
        .map_err(|e| AntennaModelError::Generic(format!("Failed to build reflector: {}", e)))?;

    // Compute physical feed position from pointing target
    let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
        &request.feed_position,
        &request.reflector_boresight,
        &request.vehicle_position,
        focal_length_m,
    )?;

    let design_pos = &calibration.physical_config.feed.position;
    let feed_x = steer_x + design_pos.0;
    let feed_y = steer_y + design_pos.1;
    let feed_z = steer_z + design_pos.2;
    let feed_position = FeedPosition::new(feed_x, feed_y, feed_z);

    let feed = ModelFeedParams::builder()
        .position(feed_position)
        .q_factor(calibration.physical_config.feed.q_factor)
        .phase_center_offset(calibration.physical_config.feed.phase_center_offset_m)
        .build()
        .map_err(|e| AntennaModelError::Generic(format!("Failed to build feed: {}", e)))?;

    let mut config_builder = AntennaConfiguration::builder()
        .id(&calibration.antenna_id)
        .name(&calibration.metadata.antenna_name)
        .reflector(reflector)
        .feed(feed);

    if let Some(ref mesh_data) = calibration.physical_config.mesh {
        let mesh = ModelMeshParams::builder()
            .spacing(mesh_data.mesh_spacing_mm / 1000.0)
            .wire_diameter(mesh_data.wire_diameter_mm / 1000.0)
            .build()
            .map_err(|e| AntennaModelError::Generic(format!("Failed to build mesh: {}", e)))?;
        config_builder = config_builder.mesh(mesh);
    }

    let antenna_config = config_builder.build().map_err(|e| {
        AntennaModelError::Generic(format!("Failed to build antenna configuration: {}", e))
    })?;

    Ok((antenna_config, feed_x, feed_y, feed_z))
}

/// Compute the gain (dB) for a cell position using the cache.
///
/// Cell center is provided as ECEF for consistent az/el derivation.
/// Returns `(gain_db, az_deg, el_deg, warnings)` so the caller can use the
/// az/el values directly for reporting without a second `compute_emitter_direction` call.
///
/// The `GainCache` stores only the scalar gain value, not the full
/// `GainComputationResult`. To avoid losing extrapolation warnings, this function
/// checks the cache first and only calls the physics engine on a miss, allowing
/// us to capture `result.warnings` from the fresh computation.  On a cache hit
/// the warnings are not re-surfaced (they were already returned on the first
/// call that populated the entry).
#[allow(clippy::too_many_arguments)]
fn compute_cell_gain(
    cell_ecef: (f64, f64, f64),
    request: &H3LinkBudgetRequest,
    antenna_config: &AntennaConfiguration,
    feed_x: f64,
    feed_y: f64,
    feed_z: f64,
    cache: &GainCache,
    integration_params: &IntegrationParams,
    frequency_hz: f64,
) -> Result<(f64, f64, f64, Vec<String>)> {
    // Create a Position3D for the cell center (ECEF). Earth-surface ECEF values are
    // typically 2–6 Mm which is below the 6400 km auto-detect threshold, so set
    // an explicit tag to prevent misclassification as Geodetic.
    let mut cell_pos = Position3D::new(cell_ecef.0, cell_ecef.1, cell_ecef.2);
    cell_pos.coordinate_system = Some(CoordinateSystem::ECEF);

    // Compute az/el once; the result is returned to the caller so that
    // `compute_cell_result` does not need to call `compute_emitter_direction` again.
    let (az_deg, el_deg) = compute_emitter_direction(
        &cell_pos,
        &request.vehicle_position,
        &request.reflector_boresight,
    )?;

    let cache_key = GainCacheKey::new(
        az_deg,
        el_deg,
        request.frequency_mhz,
        feed_x,
        feed_y,
        feed_z,
    );

    let theta_rad = el_deg.to_radians();
    let phi_rad = az_deg.to_radians();

    // Use get_or_compute to handle the cache hit path (no warnings re-surfaced on
    // a hit — acceptable since warnings were returned when the entry was first
    // computed).  On a cache miss the closure runs compute_gain_db; we capture
    // warnings by running compute_gain_db directly when the cache is disabled or
    // misses.  To avoid double computation we use a cell to smuggle warnings out
    // of the closure.
    let mut captured_warnings: Vec<String> = Vec::new();
    let gain_db = cache.get_or_compute(&request.antenna_id, &request.feed_id, cache_key, || {
        let result = compute_gain_db(
            theta_rad,
            phi_rad,
            antenna_config,
            frequency_hz,
            integration_params,
        )?;
        captured_warnings = result.warnings;
        Ok(result.gain)
    })?;

    Ok((gain_db, az_deg, el_deg, captured_warnings))
}

/// Compute H3 link budget for a request.
///
/// Generates a hexagonal grid of H3 cells centered on the feed pointing location,
/// computes antenna gain for each cell, and returns per-cell path loss, FSPL, and G/T.
pub fn compute_h3_link_budget(
    request: &H3LinkBudgetRequest,
    calibration: &AntennaCalibration,
    cache: &GainCache,
    start_time: std::time::Instant,
) -> Result<H3LinkBudgetResponse> {
    // 1. Resolve H3 resolution
    let resolution = request
        .h3_resolution
        .unwrap_or_else(|| h3_resolution_from_frequency(request.frequency_mhz));

    let h3_res = h3o::Resolution::try_from(resolution).map_err(|e| {
        AntennaModelError::Generic(format!("Invalid H3 resolution {}: {}", resolution, e))
    })?;

    // 2. Find center cell from feed position
    // Use feed_position to determine where on Earth we're centering the grid
    let (feed_ex, feed_ey, feed_ez) = pos_to_ecef(&request.feed_position)?;
    let (feed_lon_deg, feed_lat_deg, _) = ecef_to_geodetic(feed_ex, feed_ey, feed_ez)?;

    let center_latlng = h3o::LatLng::new(feed_lat_deg, feed_lon_deg).map_err(|e| {
        AntennaModelError::Generic(format!(
            "Invalid lat/lon for H3 cell ({}, {}): {}",
            feed_lat_deg, feed_lon_deg, e
        ))
    })?;

    let center_cell = center_latlng.to_cell(h3_res);
    let center_cell_id = format!("{}", center_cell);

    // 3. Generate grid disk
    let cells: Vec<h3o::CellIndex> = center_cell.grid_disk(request.n_rings);

    // 4. Build antenna configuration
    let integration_params = IntegrationParams::fast();
    let frequency_hz = request.frequency_mhz * 1e6;

    let (antenna_config, feed_x, feed_y, feed_z) = build_antenna_config(calibration, request)?;

    // 5. Compute vehicle ECEF for distance calculations
    let (vehicle_ex, vehicle_ey, vehicle_ez) = pos_to_ecef(&request.vehicle_position)?;

    // 6. Compute boresight gain (center cell) as reference peak for loss_db
    let center_latlng_cell = h3o::LatLng::from(center_cell);
    let center_lat = center_latlng_cell.lat();
    let center_lon = center_latlng_cell.lng();
    let (center_ex, center_ey, center_ez) = geodetic_to_ecef(center_lon, center_lat, 0.0)?;

    let boresight_gain_db = {
        // Earth-surface ECEF values are ~2–6 Mm, below the 6400 km auto-detect
        // threshold; set explicit ECEF to prevent misclassification as Geodetic.
        let mut cell_pos = Position3D::new(center_ex, center_ey, center_ez);
        cell_pos.coordinate_system = Some(CoordinateSystem::ECEF);
        let (az_deg, el_deg) = compute_emitter_direction(
            &cell_pos,
            &request.vehicle_position,
            &request.reflector_boresight,
        )?;
        let cache_key = GainCacheKey::new(
            az_deg,
            el_deg,
            request.frequency_mhz,
            feed_x,
            feed_y,
            feed_z,
        );
        let theta_rad = el_deg.to_radians();
        let phi_rad = az_deg.to_radians();
        cache.get_or_compute(&request.antenna_id, &request.feed_id, cache_key, || {
            let result = compute_gain_db(
                theta_rad,
                phi_rad,
                &antenna_config,
                frequency_hz,
                &integration_params,
            )?;
            Ok(result.gain)
        })?
    };

    // 7. Process each cell in parallel
    const PARALLEL_THRESHOLD: usize = 20;

    let results: Vec<Result<(H3CellResult, Vec<String>)>> = if cells.len() >= PARALLEL_THRESHOLD {
        cells
            .par_iter()
            .map(|&cell| {
                compute_cell_result(
                    cell,
                    request,
                    &antenna_config,
                    feed_x,
                    feed_y,
                    feed_z,
                    cache,
                    &integration_params,
                    frequency_hz,
                    vehicle_ex,
                    vehicle_ey,
                    vehicle_ez,
                    boresight_gain_db,
                )
            })
            .collect()
    } else {
        cells
            .iter()
            .map(|&cell| {
                compute_cell_result(
                    cell,
                    request,
                    &antenna_config,
                    feed_x,
                    feed_y,
                    feed_z,
                    cache,
                    &integration_params,
                    frequency_hz,
                    vehicle_ex,
                    vehicle_ey,
                    vehicle_ez,
                    boresight_gain_db,
                )
            })
            .collect()
    };

    // 8. Separate successes and failures
    let mut cell_results: Vec<H3CellResult> = Vec::with_capacity(cells.len());
    let mut warnings_set: HashSet<String> = HashSet::new();
    let mut failed_count = 0usize;

    for result in results {
        match result {
            Ok((cell_result, cell_warnings)) => {
                cell_results.push(cell_result);
                for w in cell_warnings {
                    warnings_set.insert(w);
                }
            }
            Err(e) => {
                failed_count += 1;
                warnings_set.insert(format!("Cell computation failed: {}", e));
            }
        }
    }

    // Compute peak gain across all cells
    let peak_gain_db = cell_results
        .iter()
        .map(|c| c.gain_db)
        .filter(|g| g.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);

    let peak_gain_db = if peak_gain_db.is_finite() {
        peak_gain_db
    } else {
        boresight_gain_db
    };

    let mut warnings: Vec<String> = warnings_set.into_iter().collect();
    warnings.sort();

    let computation_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
    let points_evaluated = cells.len();

    // Build calibration status info
    let calibration_status = calibration.calibration_status.as_ref().map(|status| {
        let mut info = CalibrationStatusInfo::from(status);
        info.correction_applied = calibration.correction_surface.is_some();
        info
    });

    Ok(H3LinkBudgetResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        frequency_mhz: request.frequency_mhz,
        center_cell_id,
        h3_resolution: resolution,
        cells: cell_results,
        warnings,
        metadata: HeatmapMetadata {
            points_evaluated,
            computation_time_ms,
            peak_gain_db,
            failed_points: failed_count,
        },
        calibration_status,
    })
}

/// Compute the link budget result for a single H3 cell.
#[allow(clippy::too_many_arguments)]
fn compute_cell_result(
    cell: h3o::CellIndex,
    request: &H3LinkBudgetRequest,
    antenna_config: &AntennaConfiguration,
    feed_x: f64,
    feed_y: f64,
    feed_z: f64,
    cache: &GainCache,
    integration_params: &IntegrationParams,
    frequency_hz: f64,
    vehicle_ex: f64,
    vehicle_ey: f64,
    vehicle_ez: f64,
    boresight_gain_db: f64,
) -> Result<(H3CellResult, Vec<String>)> {
    // Get cell center lat/lon
    let latlng = h3o::LatLng::from(cell);
    let lat_deg = latlng.lat();
    let lon_deg = latlng.lng();

    // Convert cell center to ECEF at altitude 0m
    let (cell_ex, cell_ey, cell_ez) = geodetic_to_ecef(lon_deg, lat_deg, 0.0)?;

    // Distance from vehicle to cell center
    let dx = cell_ex - vehicle_ex;
    let dy = cell_ey - vehicle_ey;
    let dz = cell_ez - vehicle_ez;
    let distance_m = (dx * dx + dy * dy + dz * dz).sqrt();
    let distance_km = distance_m / 1000.0;

    // Compute gain together with az/el; az/el are returned directly so we
    // avoid a redundant second call to `compute_emitter_direction` for reporting.
    let (gain_db, azimuth_deg, elevation_deg, cell_warnings) = compute_cell_gain(
        (cell_ex, cell_ey, cell_ez),
        request,
        antenna_config,
        feed_x,
        feed_y,
        feed_z,
        cache,
        integration_params,
        frequency_hz,
    )?;

    // Compute losses
    let loss_db = boresight_gain_db - gain_db;
    let fspl = free_space_path_loss_db(distance_m, frequency_hz);
    let total_path_loss_db = loss_db + fspl;

    // G/T computation (if temperature provided)
    let g_over_t_db = request.temperature_k.map(|t| gain_db - 10.0 * t.log10());

    Ok((
        H3CellResult {
            cell_id: format!("{}", cell),
            center_lon: lon_deg,
            center_lat: lat_deg,
            azimuth_deg,
            elevation_deg,
            distance_km,
            gain_db,
            loss_db,
            free_space_path_loss_db: fspl,
            total_path_loss_db,
            g_over_t_db,
        },
        cell_warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h3_resolution_l_band() {
        assert_eq!(h3_resolution_from_frequency(1500.0), 6);
    }

    #[test]
    fn test_h3_resolution_sc_band() {
        assert_eq!(h3_resolution_from_frequency(5000.0), 7);
    }

    #[test]
    fn test_h3_resolution_xku_band() {
        assert_eq!(h3_resolution_from_frequency(12000.0), 8);
    }

    #[test]
    fn test_h3_resolution_ka_band() {
        assert_eq!(h3_resolution_from_frequency(30000.0), 9);
    }

    #[test]
    fn test_cell_counts() {
        use h3o::{LatLng, Resolution};
        let center = LatLng::new(37.0, -122.0)
            .unwrap()
            .to_cell(Resolution::Seven);
        assert_eq!(center.grid_disk::<Vec<_>>(0).len(), 1);
        assert_eq!(center.grid_disk::<Vec<_>>(1).len(), 7);
        assert_eq!(center.grid_disk::<Vec<_>>(2).len(), 19);
    }

    #[test]
    fn test_fspl_known_value() {
        // At 100 km and 12 GHz: FSPL = 20*log10(4π * 100000 * 12e9 / 299792458)
        // = 20*log10(4π * 1.2e15 / 2.998e8) = 20*log10(5.03e7) ≈ 154.0 dB
        let fspl = free_space_path_loss_db(100_000.0, 12e9);
        assert!((fspl - 154.0).abs() < 1.0, "FSPL={}", fspl);
    }
}
