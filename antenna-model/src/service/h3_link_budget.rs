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
    compute_emitter_direction_with_attitude, compute_feed_position_from_pointing, ecef_to_geodetic,
    evaluate_correction, geodetic_to_ecef, squint_corrected_direction, AntennaConfiguration,
    FeedParameters as ModelFeedParams, FeedPosition, IntegrationParams,
    MeshParameters as ModelMeshParams, ReflectorGeometry as ModelReflector,
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
        diameter_m,
        request.vehicle_attitude,
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
/// Returns `(gain_db, az_deg, el_deg, warnings, correction_applied)` so the caller can use the
/// az/el values directly for reporting without a second `compute_emitter_direction` call.
///
/// The `GainCache` stores only the **physics-only** scalar gain value, not the
/// correction-adjusted value. The correction surface is applied AFTER the cache
/// lookup so that the cache key space remains consistent regardless of whether a
/// correction surface is present.
///
/// On a cache hit, physics warnings are not re-surfaced (they were already returned
/// on the first call that populated the entry). Correction-surface warnings ARE
/// always re-surfaced because they are computed fresh on every call.
#[allow(clippy::too_many_arguments)]
fn compute_cell_gain(
    cell_ecef: (f64, f64, f64),
    request: &H3LinkBudgetRequest,
    calibration: &AntennaCalibration,
    antenna_config: &AntennaConfiguration,
    feed_x: f64,
    feed_y: f64,
    feed_z: f64,
    cache: &GainCache,
    integration_params: &IntegrationParams,
    frequency_hz: f64,
) -> Result<(f64, f64, f64, Vec<String>, bool)> {
    // Create a Position3D for the cell center (ECEF). Earth-surface ECEF values are
    // typically 2–6 Mm which is below the 6400 km auto-detect threshold, so set
    // an explicit tag to prevent misclassification as Geodetic.
    let mut cell_pos = Position3D::new(cell_ecef.0, cell_ecef.1, cell_ecef.2);
    cell_pos.coordinate_system = Some(CoordinateSystem::ECEF);

    // Compute az/el once; the result is returned to the caller so that
    // `compute_cell_result` does not need to call `compute_emitter_direction` again.
    let (az_deg, el_deg) = compute_emitter_direction_with_attitude(
        &cell_pos,
        &request.vehicle_position,
        &request.reflector_boresight,
        request.vehicle_attitude,
    )?;

    // Apply beam squint (honors pointing_frequency_mhz). Corrected angles are used for
    // BOTH the cache key and the gain evaluation so cached values match the angle used.
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);
    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let (az_deg, el_deg, _squint_deg) = squint_corrected_direction(
        az_deg,
        el_deg,
        request.frequency_mhz,
        pointing_freq,
        feed_x,
        feed_y,
        focal_length_m,
    );

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
    //
    // IMPORTANT: the cache stores PHYSICS-ONLY gain. The correction surface must
    // be applied after this call, never inside the closure.
    let mut captured_warnings: Vec<String> = Vec::new();
    let physics_gain_db =
        cache.get_or_compute(&request.antenna_id, &request.feed_id, cache_key, || {
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

    // Apply correction surface (post-cache). Uses the same gating logic as
    // `service::evaluator::compute_gain_from_request` (290.0 K temperature constant,
    // `is_in_coverage` for optional-coverage gating).
    let mut correction_applied = false;
    let mut gain_db = physics_gain_db;
    if let Some(ref surface) = calibration.correction_surface {
        if crate::service::evaluator::is_in_coverage(
            &calibration.calibration_coverage,
            az_deg,
            el_deg,
            request.frequency_mhz,
        ) {
            let corr = evaluate_correction(surface, az_deg, el_deg, request.frequency_mhz, 290.0)?;
            gain_db += corr.correction_db;
            captured_warnings.extend(corr.warnings);
            correction_applied = true;
        }
    }

    Ok((
        gain_db,
        az_deg,
        el_deg,
        captured_warnings,
        correction_applied,
    ))
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

    // Squint magnitude is constant per request: it is the freq-shift ratio times the
    // feed-displacement ratio, independent of which (az, el) the squint is applied to.
    // Evaluate at (0.0, 0.0) to extract that magnitude without a real direction, once,
    // for the response field.
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);
    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let (_, _, squint_magnitude_deg) = squint_corrected_direction(
        0.0,
        0.0,
        request.frequency_mhz,
        pointing_freq,
        feed_x,
        feed_y,
        focal_length_m,
    );
    let beam_squint_deg = if squint_magnitude_deg > 0.001 {
        Some(squint_magnitude_deg)
    } else {
        None
    };

    // 5. Compute vehicle ECEF for distance calculations
    let (vehicle_ex, vehicle_ey, vehicle_ez) = pos_to_ecef(&request.vehicle_position)?;

    // 6. Compute boresight gain (center cell) as reference peak for loss_db.
    //    The correction surface is applied here so that loss_db = boresight_gain_db - cell_gain_db
    //    is computed on a consistent basis (both corrected, or both physics-only).
    let center_latlng_cell = h3o::LatLng::from(center_cell);
    let center_lat = center_latlng_cell.lat();
    let center_lon = center_latlng_cell.lng();
    let (center_ex, center_ey, center_ez) = geodetic_to_ecef(center_lon, center_lat, 0.0)?;

    let boresight_gain_db = {
        // Earth-surface ECEF values are ~2–6 Mm, below the 6400 km auto-detect
        // threshold; set explicit ECEF to prevent misclassification as Geodetic.
        let mut cell_pos = Position3D::new(center_ex, center_ey, center_ez);
        cell_pos.coordinate_system = Some(CoordinateSystem::ECEF);
        let (az_deg, el_deg) = compute_emitter_direction_with_attitude(
            &cell_pos,
            &request.vehicle_position,
            &request.reflector_boresight,
            request.vehicle_attitude,
        )?;

        // Apply beam squint (honors pointing_frequency_mhz). Corrected angles are used for
        // BOTH the cache key and the gain evaluation so cached values match the angle used.
        let (az_deg, el_deg, _squint_deg) = squint_corrected_direction(
            az_deg,
            el_deg,
            request.frequency_mhz,
            pointing_freq,
            feed_x,
            feed_y,
            focal_length_m,
        );

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
        // Cache stores physics-only gain; correction is applied below.
        let physics_gain =
            cache.get_or_compute(&request.antenna_id, &request.feed_id, cache_key, || {
                let result = compute_gain_db(
                    theta_rad,
                    phi_rad,
                    &antenna_config,
                    frequency_hz,
                    &integration_params,
                )?;
                Ok(result.gain)
            })?;
        // Apply correction surface to boresight reference for consistent loss_db.
        if let Some(ref surface) = calibration.correction_surface {
            if crate::service::evaluator::is_in_coverage(
                &calibration.calibration_coverage,
                az_deg,
                el_deg,
                request.frequency_mhz,
            ) {
                let corr =
                    evaluate_correction(surface, az_deg, el_deg, request.frequency_mhz, 290.0)?;
                physics_gain + corr.correction_db
            } else {
                physics_gain
            }
        } else {
            physics_gain
        }
    };

    // 7. Process each cell in parallel
    const PARALLEL_THRESHOLD: usize = 20;

    let results: Vec<Result<(H3CellResult, Vec<String>, bool)>> =
        if cells.len() >= PARALLEL_THRESHOLD {
            cells
                .par_iter()
                .map(|&cell| {
                    compute_cell_result(
                        cell,
                        request,
                        calibration,
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
                        calibration,
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

    // 8. Separate successes and failures; track whether correction was applied to any cell.
    let mut cell_results: Vec<H3CellResult> = Vec::with_capacity(cells.len());
    let mut warnings_set: HashSet<String> = HashSet::new();
    let mut failed_count = 0usize;
    let mut any_correction_applied = false;

    for result in results {
        match result {
            Ok((cell_result, cell_warnings, correction_applied)) => {
                cell_results.push(cell_result);
                for w in cell_warnings {
                    warnings_set.insert(w);
                }
                any_correction_applied |= correction_applied;
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

    // Build calibration status info.
    // `correction_applied` reflects whether the correction surface was actually
    // applied to at least one cell (gated on coverage), not merely whether a surface
    // exists — matching the truthful reporting in `service::evaluator`.
    let calibration_status = calibration.calibration_status.as_ref().map(|status| {
        let mut info = CalibrationStatusInfo::from(status);
        info.correction_applied = any_correction_applied;
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
        beam_squint_deg,
    })
}

/// Compute the link budget result for a single H3 cell.
#[allow(clippy::too_many_arguments)]
fn compute_cell_result(
    cell: h3o::CellIndex,
    request: &H3LinkBudgetRequest,
    calibration: &AntennaCalibration,
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
) -> Result<(H3CellResult, Vec<String>, bool)> {
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
    // `correction_applied` indicates whether the correction surface was applied.
    let (gain_db, azimuth_deg, elevation_deg, cell_warnings, correction_applied) =
        compute_cell_gain(
            (cell_ex, cell_ey, cell_ez),
            request,
            calibration,
            antenna_config,
            feed_x,
            feed_y,
            feed_z,
            cache,
            integration_params,
            frequency_hz,
        )?;

    // Compute losses.
    // loss_db = boresight_gain_db - gain_db, where both values are on the same
    // basis (both physics+correction, or both physics-only). The boresight reference
    // computed in step 6 of `compute_h3_link_budget` uses the same correction gating,
    // so loss_db is a consistent relative measure.
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
        correction_applied,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::types::{
        AntennaCalibration, BSplineModel4D, CalibrationMetadata, CalibrationStatus, FeedParameters,
        MeshParameters, PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
    };
    use crate::model::evaluate_correction;

    /// Build a minimal `AntennaCalibration` suitable for H3 link-budget tests.
    ///
    /// The geometry matches the evaluator.rs `create_test_calibration`:
    /// 10 m dish, f/D=0.5, no design feed offset, mesh present.
    fn make_h3_test_calibration() -> AntennaCalibration {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("H3 Test Antenna")
            .calibration_date("2025-01-01T00:00:00Z")
            .format_version("2.0")
            .data_source("test")
            .rmse_db(0.5)
            .r_squared(0.99)
            .num_measurements(1000)
            .build()
            .unwrap();

        AntennaCalibration::builder()
            .antenna_id("h3_test_antenna")
            .feed_id("h3_test_feed")
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
            .unwrap()
    }

    /// Build an H3 link-budget request centered near San Francisco with 0 rings
    /// (a single cell) so tests finish fast.  The vehicle is at 400 km altitude
    /// which keeps the geometry realistic without requiring real ECEF coordinates.
    fn make_h3_test_request() -> H3LinkBudgetRequest {
        use crate::api::schemas::CoordinateSystem;
        // Vehicle: geodetic (lon, lat, alt_m)
        let mut vehicle = Position3D::new(-122.0, 37.5, 400_000.0);
        vehicle.coordinate_system = Some(CoordinateSystem::Geodetic);
        // Reflector boresight: aimed a tiny bit away from nadir so there is a
        // well-defined boresight direction.
        let mut boresight = Position3D::new(-122.01, 37.49, 0.0);
        boresight.coordinate_system = Some(CoordinateSystem::Geodetic);
        // Feed position: same as boresight (on-axis) for simplicity.
        let mut feed = Position3D::new(-122.01, 37.49, 0.0);
        feed.coordinate_system = Some(CoordinateSystem::Geodetic);

        H3LinkBudgetRequest {
            antenna_id: "h3_test_antenna".to_string(),
            feed_id: "h3_test_feed".to_string(),
            vehicle_position: vehicle,
            reflector_boresight: boresight,
            feed_position: feed,
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            n_rings: 0, // single center cell only — fast
            h3_resolution: Some(7),
            temperature_k: None,
            vehicle_attitude: None,
        }
    }

    /// Build a constant-valued 4D B-spline correction surface.
    ///
    /// Uses order-2 (linear), shape [2, 2, 2, 2] (4 control points per axis means
    /// 16 coefficients total).  The knot vectors are clamped and wide enough to
    /// cover the test request's az/el/freq/temp values.
    ///
    /// For a B-spline with all equal coefficients `c`, the partition-of-unity
    /// property guarantees that the interpolant evaluates to exactly `c` everywhere
    /// in range.
    fn constant_surface_db(value: f64) -> BSplineModel4D {
        // Order 2, shape [2,2,2,2]: knot vectors need length >= n + order = 2 + 2 = 4.
        // Clamped knots for order 2: [lo, lo, hi, hi].
        let surface = BSplineModel4D {
            // 2×2×2×2 = 16 coefficients, all equal to `value`.
            coefficients: vec![value; 16],
            shape: [2, 2, 2, 2],
            // Wide ranges that encompass any az/el from the test geometry.
            knots_azimuth: vec![0.0, 0.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 90.0, 90.0],
            // Cover the test frequency (8400 MHz).
            knots_frequency: vec![8000.0, 8000.0, 9000.0, 9000.0],
            // Cover the temperature constant used by the evaluator (290 K).
            knots_temperature: vec![280.0, 280.0, 300.0, 300.0],
            spline_order: 2,
        };
        // Verify the model passes structural validation before returning.
        surface
            .validate()
            .expect("constant_surface_db: BSplineModel4D failed validate()");
        surface
    }

    /// Verify the constant surface helper actually evaluates to the expected constant.
    ///
    /// This is a pre-flight check ensuring the test fixture is non-vacuous before
    /// using it in `test_h3_applies_correction_surface`.
    #[test]
    fn test_constant_surface_evaluates_to_constant() {
        let surface = constant_surface_db(2.0);
        let result = evaluate_correction(&surface, 45.0, 30.0, 8400.0, 290.0)
            .expect("evaluate_correction failed on constant surface");
        assert!(
            !result.extrapolated,
            "query (45°, 30°, 8400 MHz, 290 K) should be in range"
        );
        assert!(
            (result.correction_db - 2.0).abs() < 1e-9,
            "constant surface should evaluate to 2.0 dB everywhere, got {}",
            result.correction_db
        );
    }

    /// Core correctness test: a constant +2 dB correction surface must shift
    /// every cell's gain by exactly +2 dB relative to the physics-only run.
    ///
    /// Also checks that `correction_applied` is truthfully reported.
    #[test]
    fn test_h3_applies_correction_surface() {
        let request = make_h3_test_request();

        // Physics-only calibration (no correction surface).
        let cal_no_corr = make_h3_test_calibration();

        // Same calibration but with a +2 dB correction surface and unrestricted coverage.
        let mut cal_corr = cal_no_corr.clone();
        cal_corr.correction_surface = Some(constant_surface_db(2.0));
        cal_corr.calibration_coverage = None; // unrestricted → correction applies everywhere

        // Disable the cache so each run computes fresh (avoids cross-test key collisions).
        let cache1 = GainCache::new(false, 1);
        let base =
            compute_h3_link_budget(&request, &cal_no_corr, &cache1, std::time::Instant::now())
                .expect("physics-only H3 run failed");

        let cache2 = GainCache::new(false, 1);
        let corrected =
            compute_h3_link_budget(&request, &cal_corr, &cache2, std::time::Instant::now())
                .expect("corrected H3 run failed");

        assert!(
            !base.cells.is_empty(),
            "expected at least one cell in result"
        );
        assert_eq!(
            base.cells.len(),
            corrected.cells.len(),
            "cell count must match between runs"
        );

        for (a, b) in base.cells.iter().zip(corrected.cells.iter()) {
            assert!(
                (b.gain_db - a.gain_db - 2.0).abs() < 1e-6,
                "cell {}: corrected gain {:.6} - base gain {:.6} = {:.6}, expected +2.0 dB",
                a.cell_id,
                b.gain_db,
                a.gain_db,
                b.gain_db - a.gain_db
            );
        }

        // correction_applied must be true when surface was applied.
        assert!(
            corrected
                .calibration_status
                .as_ref()
                .map(|s| s.correction_applied)
                .unwrap_or(false),
            "correction_applied should be true when correction surface was applied"
        );

        // correction_applied must be false when no surface present.
        assert!(
            !base
                .calibration_status
                .as_ref()
                .map(|s| s.correction_applied)
                .unwrap_or(true),
            "correction_applied should be false when no correction surface"
        );
    }

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

    #[test]
    fn test_h3_squint_changes_cell_gains_with_pointing_offset() {
        let calibration = make_h3_test_calibration();

        let mut req_baseline = make_h3_test_request();
        req_baseline.pointing_frequency_mhz = None;

        let mut req_squint = make_h3_test_request();
        // Steer the feed off boresight so feed displacement (hence squint) is non-zero.
        req_squint.feed_position = Position3D::new(
            req_squint.reflector_boresight.x + 0.05,
            req_squint.reflector_boresight.y,
            req_squint.reflector_boresight.z,
        );
        // Explicit tag (matches make_h3_test_request); don't rely on auto-detection.
        req_squint.feed_position.coordinate_system = Some(CoordinateSystem::Geodetic);
        req_squint.pointing_frequency_mhz = Some(req_squint.frequency_mhz * 1.4);
        req_baseline.feed_position = req_squint.feed_position.clone();

        let cache1 = GainCache::new(false, 1);
        let resp_baseline =
            compute_h3_link_budget(&req_baseline, &calibration, &cache1, std::time::Instant::now())
                .unwrap();
        let cache2 = GainCache::new(false, 1);
        let resp_squint =
            compute_h3_link_budget(&req_squint, &calibration, &cache2, std::time::Instant::now())
                .unwrap();

        let gains_baseline: Vec<f64> = resp_baseline.cells.iter().map(|c| c.gain_db).collect();
        let gains_squint: Vec<f64> = resp_squint.cells.iter().map(|c| c.gain_db).collect();
        assert_eq!(gains_baseline.len(), gains_squint.len());
        assert_ne!(
            gains_baseline,
            gains_squint,
            "a large pointing-frequency offset with a steered feed must change cell gains"
        );
    }

    #[test]
    fn test_h3_no_pointing_offset_is_unchanged() {
        let calibration = make_h3_test_calibration();
        let mut req_none = make_h3_test_request();
        req_none.pointing_frequency_mhz = None;
        let mut req_equal = make_h3_test_request();
        req_equal.pointing_frequency_mhz = Some(req_equal.frequency_mhz);
        let cache1 = GainCache::new(false, 1);
        let resp_none =
            compute_h3_link_budget(&req_none, &calibration, &cache1, std::time::Instant::now())
                .unwrap();
        let cache2 = GainCache::new(false, 1);
        let resp_equal =
            compute_h3_link_budget(&req_equal, &calibration, &cache2, std::time::Instant::now())
                .unwrap();
        let gains_none: Vec<f64> = resp_none.cells.iter().map(|c| c.gain_db).collect();
        let gains_equal: Vec<f64> = resp_equal.cells.iter().map(|c| c.gain_db).collect();
        assert_eq!(
            gains_none, gains_equal,
            "pointing == operating must not change gains"
        );
    }

    #[test]
    fn test_h3_reports_beam_squint_deg() {
        let calibration = make_h3_test_calibration();

        let mut req = make_h3_test_request();
        req.feed_position = Position3D::new(
            req.reflector_boresight.x + 0.05,
            req.reflector_boresight.y,
            req.reflector_boresight.z,
        );
        req.feed_position.coordinate_system = Some(CoordinateSystem::Geodetic);
        req.pointing_frequency_mhz = Some(req.frequency_mhz * 1.4);
        let cache = GainCache::new(false, 1);
        let resp = compute_h3_link_budget(&req, &calibration, &cache, std::time::Instant::now())
            .unwrap();
        assert!(
            resp.beam_squint_deg.is_some_and(|s| s > 0.0),
            "expected Some(squint>0), got {:?}",
            resp.beam_squint_deg
        );

        // Displace the feed too, so this asserts None comes from pointing == None — not
        // merely from zero feed displacement.
        let mut req_none = make_h3_test_request();
        req_none.feed_position = Position3D::new(
            req_none.reflector_boresight.x + 0.05,
            req_none.reflector_boresight.y,
            req_none.reflector_boresight.z,
        );
        req_none.feed_position.coordinate_system = Some(CoordinateSystem::Geodetic);
        req_none.pointing_frequency_mhz = None;
        let cache_none = GainCache::new(false, 1);
        let resp_none =
            compute_h3_link_budget(&req_none, &calibration, &cache_none, std::time::Instant::now())
                .unwrap();
        assert!(
            resp_none.beam_squint_deg.is_none(),
            "no offset -> None"
        );
    }
}
