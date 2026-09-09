//! H3 Link Budget Service
//!
//! Computes per-cell antenna gain, free-space path loss, and total path loss
//! for an H3 hexagonal grid centered on the feed pointing location.
//!
//! # Pipeline
//!
//! 1. Resolve H3 resolution from request (or derive from frequency)
//! 2. Find center H3 cell from feed pointing location lat/lon
//! 3. Generate grid disk of N rings around center cell
//! 4. Prepare one immutable served-gain evaluator from calibration and request geometry
//! 5. For each cell (sequentially or in parallel): adapt its geometry, evaluate served gain
//!    via cache, compute FSPL
//! 6. Take the peak gain over the cells actually evaluated, then fill each cell's
//!    `loss_db` / `total_path_loss_db` relative to it (roadmap C9 — the same rule
//!    `service::heatmap` applies)
//! 7. Return H3LinkBudgetResponse with per-cell results and metadata

use crate::api::schemas::{
    CalibrationStatusInfo, H3CellResult, H3LinkBudgetRequest, H3LinkBudgetResponse,
    HeatmapMetadata, Position3D,
};
use crate::error::{AntennaModelError, Result};
use crate::model::integration::DEFAULT_INTEGRATION_BUDGET;
use crate::model::{
    compute_emitter_direction_with_attitude, compute_feed_position_from_pointing, ecef_to_geodetic,
    geodetic_to_ecef,
};
use crate::service::served_gain::{
    FeedSteering, PreSquintDirection, PreparedServedGain, ReferenceGainRequest, ServedFrequencies,
};
use crate::service::GainCache;
use crate::warnings::{ApiWarning, WarningCode};
use antenna_core::data::types::AntennaCalibration;
use rayon::prelude::*;
use std::collections::HashSet;
use std::time::Duration;

/// Grids at or above this size use Rayon; smaller grids avoid parallel overhead.
const PARALLEL_THRESHOLD: usize = 20;

#[derive(Debug, Clone, Copy)]
enum CellTraversal {
    Automatic,
    #[cfg(test)]
    Sequential,
}

impl CellTraversal {
    fn is_parallel(self, cell_count: usize) -> bool {
        match self {
            Self::Automatic => cell_count >= PARALLEL_THRESHOLD,
            #[cfg(test)]
            Self::Sequential => false,
        }
    }
}

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
/// A position declared geodetic is converted via `geodetic_to_ecef`; one declared ECEF
/// is returned directly.
fn pos_to_ecef(pos: &Position3D) -> Result<(f64, f64, f64)> {
    if pos.is_ecef() {
        Ok((pos.x, pos.y, pos.z))
    } else {
        geodetic_to_ecef(pos.x, pos.y, pos.z)
    }
}

/// Adapt one H3 cell centre into the prepared served-gain operation.
///
/// H3 owns only the cell geometry: the prepared value owns squint, cache identity,
/// physical optics, correction disposition, and directional warnings (#63).
fn compute_cell_gain(
    cell_ecef: (f64, f64, f64),
    request: &H3LinkBudgetRequest,
    prepared: &PreparedServedGain,
    cache: &GainCache,
) -> Result<(f64, f64, f64, Vec<ApiWarning>, bool)> {
    let cell_pos = Position3D::ecef(cell_ecef.0, cell_ecef.1, cell_ecef.2);
    let (e_clock_deg, e_cone_deg) = compute_emitter_direction_with_attitude(
        &cell_pos,
        &request.vehicle_position,
        &request.reflector_boresight,
        request.vehicle_attitude,
    )?;

    let served = prepared.evaluate_cached(
        PreSquintDirection::new(e_clock_deg, e_cone_deg),
        ReferenceGainRequest::Omit,
        cache,
    )?;

    Ok((
        served.gain_db,
        served.direction.e_clock_deg,
        served.direction.e_cone_deg,
        served.warnings,
        served.correction.applied(),
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
    // Thin wrapper passing the generous model-layer default; the endpoint calls
    // `compute_h3_link_budget_with_budget` with the configured value.
    compute_h3_link_budget_with_budget(
        request,
        calibration,
        cache,
        start_time,
        DEFAULT_INTEGRATION_BUDGET,
    )
}

/// Compute the H3 link budget, bounding each per-cell aperture integration to `time_budget`
/// (roadmap S3). The served endpoint threads `performance.integration_budget_ms` here.
///
/// Note (honest limitation): the budget caps each per-cell integration, not the whole grid.
/// The whole-request wall-clock is S2's `RequestTimeout`; total fan-out CPU is S4.
pub fn compute_h3_link_budget_with_budget(
    request: &H3LinkBudgetRequest,
    calibration: &AntennaCalibration,
    cache: &GainCache,
    start_time: std::time::Instant,
    time_budget: Duration,
) -> Result<H3LinkBudgetResponse> {
    compute_h3_link_budget_with_traversal(
        request,
        calibration,
        cache,
        start_time,
        time_budget,
        CellTraversal::Automatic,
    )
}

fn compute_h3_link_budget_with_traversal(
    request: &H3LinkBudgetRequest,
    calibration: &AntennaCalibration,
    cache: &GainCache,
    start_time: std::time::Instant,
    time_budget: Duration,
    traversal: CellTraversal,
) -> Result<H3LinkBudgetResponse> {
    // 1. Resolve H3 resolution
    let resolution = request
        .h3_resolution
        .unwrap_or_else(|| h3_resolution_from_frequency(request.frequency_mhz));

    let h3_res = h3o::Resolution::try_from(resolution).map_err(|e| {
        AntennaModelError::Generic(format!("Invalid H3 resolution {}: {}", resolution, e))
    })?;

    // 2. Find center cell from the feed pointing location
    // Use feed_pointing_location to determine where on Earth we're centering the grid
    let (feed_ex, feed_ey, feed_ez) = pos_to_ecef(&request.feed_pointing_location)?;
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

    // 4. Prepare the immutable served-gain value once, before either traversal branch.
    // Rayon workers borrow this one `Sync` value; every direction-dependent operation then
    // goes through `evaluate_cached` rather than being reconstructed in H3 (#63).
    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let diameter_m = calibration.physical_config.reflector.diameter_m;
    let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
        &request.feed_pointing_location,
        &request.reflector_boresight,
        &request.vehicle_position,
        focal_length_m,
        diameter_m,
        request.vehicle_attitude,
    )?;
    let prepared = PreparedServedGain::prepare(
        calibration.clone(),
        FeedSteering::new(steer_x, steer_y, steer_z),
        ServedFrequencies::new(request.frequency_mhz, request.pointing_frequency_mhz),
        time_budget,
    )?;
    let beam_squint_deg = prepared.reported_beam_squint_deg();
    let frequency_hz = request.frequency_mhz * 1e6;

    // 5. Compute vehicle ECEF for distance calculations
    let (vehicle_ex, vehicle_ey, vehicle_ez) = pos_to_ecef(&request.vehicle_position)?;

    // 6. Process each cell in parallel.
    //    Pass 1 computes everything that does not need the grid peak: az/el, gain, distance,
    //    FSPL, G/T. `loss_db` / `total_path_loss_db` are filled in pass 2 (step 8), once the
    //    peak over the evaluated cells is known (roadmap C9). There is deliberately no
    //    separate boresight reference evaluation any more: the reference is one of the cells.
    let results: Vec<Result<(CellGain, Vec<ApiWarning>, bool)>> =
        if traversal.is_parallel(cells.len()) {
            cells
                .par_iter()
                .map(|&cell| {
                    compute_cell_result(
                        cell,
                        request,
                        &prepared,
                        cache,
                        frequency_hz,
                        vehicle_ex,
                        vehicle_ey,
                        vehicle_ez,
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
                        &prepared,
                        cache,
                        frequency_hz,
                        vehicle_ex,
                        vehicle_ey,
                        vehicle_ez,
                    )
                })
                .collect()
        };

    // 7. Separate successes and failures; track whether correction was applied to any cell.
    let mut cell_gains: Vec<CellGain> = Vec::with_capacity(cells.len());
    // Seed the aggregate with preparation-time advisories. Successful cell results carry
    // the same whole objects and deduplicate into this set; seeding also preserves the
    // pre-#63 endpoint behavior when every directional evaluation fails.
    let mut warnings_set: HashSet<ApiWarning> =
        prepared.configuration_warnings().iter().cloned().collect();
    let mut failed_count = 0usize;
    let mut any_correction_applied = false;

    for result in results {
        match result {
            Ok((cell_gain, cell_warnings, correction_applied)) => {
                cell_gains.push(cell_gain);
                for w in cell_warnings {
                    warnings_set.insert(w);
                }
                any_correction_applied |= correction_applied;
            }
            Err(e) => {
                failed_count += 1;
                warnings_set.insert(
                    WarningCode::PointComputationFailed
                        .with(format!("Cell computation failed: {}", e)),
                );
            }
        }
    }

    // 8. Pass 2 (roadmap C9): the loss reference is the peak gain over the cells actually
    //    evaluated — the rule `service::heatmap` already applies, so the two heatmap
    //    endpoints give `loss_db` one meaning. Every cell's loss is therefore ≥ 0, the peak
    //    cell's is exactly 0, and the response is internally re-derivable
    //    (`loss_db == metadata.peak_gain_db − gain_db`).
    //
    //    Basis note: the reference is one of the cells, so both sides of the subtraction
    //    share a basis by construction. A grid can still straddle two bases — cells outside
    //    calibration coverage are uncorrected while in-coverage cells are corrected —
    //    exactly as `/heatmap` already does.
    let peak_gain_db = cell_gains
        .iter()
        .map(|c| c.gain_db)
        .filter(|g| g.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);

    // No cell yielded a finite gain (every cell failed, or every gain was non-finite):
    // there is no peak to reference. Report the finite sentinel rather than -inf, which
    // would serialize to `null` for a field the schema declares as a number.
    let peak_gain_db = if peak_gain_db.is_finite() {
        peak_gain_db
    } else {
        crate::service::heatmap::NO_PEAK_GAIN_DB
    };

    let cell_results: Vec<H3CellResult> = cell_gains
        .into_iter()
        .map(|c| {
            // A non-finite cell gain has no meaningful loss; use the same sentinel
            // `/heatmap` reports for a failed grid point rather than emitting NaN/-inf.
            let loss_db = if c.gain_db.is_finite() && peak_gain_db.is_finite() {
                peak_gain_db - c.gain_db
            } else {
                crate::service::heatmap::FAILED_POINT_LOSS_DB
            };
            H3CellResult {
                cell_id: c.cell_id,
                center_lon: c.center_lon,
                center_lat: c.center_lat,
                azimuth_deg: c.azimuth_deg,
                elevation_deg: c.elevation_deg,
                distance_km: c.distance_km,
                gain_db: c.gain_db,
                loss_db,
                free_space_path_loss_db: c.free_space_path_loss_db,
                total_path_loss_db: loss_db + c.free_space_path_loss_db,
                g_over_t_db: c.g_over_t_db,
            }
        })
        .collect();

    let mut warnings: Vec<ApiWarning> = warnings_set.into_iter().collect();
    warnings.sort();

    let computation_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
    let points_evaluated = cells.len();

    // Build calibration status info.
    // `correction_applied` reflects whether the correction surface was actually
    // applied to at least one cell (gated on coverage), not merely whether a surface
    // exists — matching the truthful reporting in `service::evaluator`.
    let calibration_status = prepared.calibration_status().map(|status| {
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

/// A cell's per-cell quantities, computed before the grid peak is known.
///
/// `loss_db` and `total_path_loss_db` are deliberately absent: since roadmap C9 they are
/// referenced to the peak gain over the whole grid, which cannot be known until every cell
/// has been evaluated. Keeping them out of this struct makes the second pass mandatory —
/// `H3CellResult` is constructed only there, so a cell cannot escape with an unfilled loss.
struct CellGain {
    cell_id: String,
    center_lon: f64,
    center_lat: f64,
    azimuth_deg: f64,
    elevation_deg: f64,
    distance_km: f64,
    gain_db: f64,
    free_space_path_loss_db: f64,
    g_over_t_db: Option<f64>,
}

/// Compute the peak-independent link budget quantities for a single H3 cell.
#[allow(clippy::too_many_arguments)]
fn compute_cell_result(
    cell: h3o::CellIndex,
    request: &H3LinkBudgetRequest,
    prepared: &PreparedServedGain,
    cache: &GainCache,
    frequency_hz: f64,
    vehicle_ex: f64,
    vehicle_ey: f64,
    vehicle_ez: f64,
) -> Result<(CellGain, Vec<ApiWarning>, bool)> {
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
        compute_cell_gain((cell_ex, cell_ey, cell_ez), request, prepared, cache)?;

    // Free-space path loss is peak-independent, so it is computed here. `loss_db` and
    // `total_path_loss_db` are filled by the caller's second pass, against the grid peak.
    let fspl = free_space_path_loss_db(distance_m, frequency_hz);

    // G/T computation (if temperature provided) — shared formula, see
    // `pattern::g_over_t_from_gain_db`. T is a user-supplied passthrough (F4).
    let g_over_t_db = request
        .temperature_k
        .map(|t| crate::model::pattern::g_over_t_from_gain_db(gain_db, t));

    Ok((
        CellGain {
            cell_id: format!("{}", cell),
            center_lon: lon_deg,
            center_lat: lat_deg,
            azimuth_deg,
            elevation_deg,
            distance_km,
            gain_db,
            free_space_path_loss_db: fspl,
            g_over_t_db,
        },
        cell_warnings,
        correction_applied,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use antenna_core::data::types::{
        AntennaCalibration, BSplineModel4D, CalibrationCoverage, CalibrationMetadata,
        CalibrationStatus, FeedParameters, MeshParameters, PhysicalAntennaConfig,
        ReflectorGeometry, ValidityRanges,
    };

    use crate::api::schemas::GainRequest;
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
                    axial_defocus_m: 0.0,
                    asymmetry_factor: 1.0,
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
        // Vehicle: geodetic (lon, lat, alt_m)
        let vehicle = Position3D::geodetic(-122.0, 37.5, 400_000.0);
        // Reflector boresight: aimed a tiny bit away from nadir so there is a
        // well-defined boresight direction.
        let boresight = Position3D::geodetic(-122.01, 37.49, 0.0);
        // Feed position: same as boresight (on-axis) for simplicity.
        let feed = Position3D::geodetic(-122.01, 37.49, 0.0);

        H3LinkBudgetRequest {
            antenna_id: "h3_test_antenna".to_string(),
            feed_id: "h3_test_feed".to_string(),
            vehicle_position: vehicle,
            reflector_boresight: boresight,
            feed_pointing_location: feed,
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            n_rings: 0, // single center cell only — fast
            h3_resolution: Some(7),
            temperature_k: None,
            vehicle_attitude: None,
        }
    }

    /// Reconstruct the exact H3 centre-cell emitter position for a matching `/gain` call.
    fn center_cell_position(request: &H3LinkBudgetRequest) -> Position3D {
        let resolution = request
            .h3_resolution
            .unwrap_or_else(|| h3_resolution_from_frequency(request.frequency_mhz));
        let h3_res = h3o::Resolution::try_from(resolution).unwrap();
        let (feed_x, feed_y, feed_z) = pos_to_ecef(&request.feed_pointing_location).unwrap();
        let (feed_lon, feed_lat, _) = ecef_to_geodetic(feed_x, feed_y, feed_z).unwrap();
        let center = h3o::LatLng::new(feed_lat, feed_lon)
            .unwrap()
            .to_cell(h3_res);
        let center_latlng = h3o::LatLng::from(center);
        let (cell_x, cell_y, cell_z) =
            geodetic_to_ecef(center_latlng.lng(), center_latlng.lat(), 0.0).unwrap();
        Position3D::ecef(cell_x, cell_y, cell_z)
    }

    fn center_cell_gain_request(request: &H3LinkBudgetRequest) -> GainRequest {
        GainRequest {
            antenna_id: request.antenna_id.clone(),
            feed_id: request.feed_id.clone(),
            vehicle_position: request.vehicle_position.clone(),
            reflector_boresight: request.reflector_boresight.clone(),
            feed_pointing_location: request.feed_pointing_location.clone(),
            emitter_position: center_cell_position(request),
            frequency_mhz: request.frequency_mhz,
            pointing_frequency_mhz: request.pointing_frequency_mhz,
            include_reference: false,
            vehicle_attitude: request.vehicle_attitude,
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
    /// every cell's gain by exactly +2 dB relative to a "base" run.
    ///
    /// The base fixture uses a 0 dB constant correction surface rather than no
    /// surface at all. This is deliberate (P1 follow-up): since `correction_surface`
    /// presence also gates `apply_spillover` off (a real correction surface
    /// empirically absorbs physical spillover — see
    /// `service::evaluator::compute_gain_from_request`), comparing against a truly
    /// uncalibrated (no-surface) baseline would leak the small physical-spillover
    /// delta into the +2 dB comparison. Giving both fixtures *a* surface (0 dB vs.
    /// 2 dB) holds `apply_spillover` constant across the comparison, isolating the
    /// correction-surface delta exactly.
    ///
    /// `correction_applied` truthfulness (true/false) is checked separately below
    /// against a genuinely uncalibrated fixture.
    #[test]
    fn test_h3_applies_correction_surface() {
        let request = make_h3_test_request();

        // "Base": a 0 dB (no-op) constant correction surface, so apply_spillover
        // is OFF — same as the +2 dB fixture below, isolating the surface delta.
        let mut cal_base = make_h3_test_calibration();
        cal_base.correction_surface = Some(constant_surface_db(0.0));
        cal_base.calibration_coverage = None; // unrestricted → correction applies everywhere

        // Same calibration but with a +2 dB correction surface and unrestricted coverage.
        let mut cal_corr = cal_base.clone();
        cal_corr.correction_surface = Some(constant_surface_db(2.0));

        // Disable the cache so each run computes fresh (avoids cross-test key collisions).
        let cache1 = GainCache::new(false, 1);
        let base = compute_h3_link_budget(&request, &cal_base, &cache1, std::time::Instant::now())
            .expect("base (0 dB surface) H3 run failed");

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

        // correction_applied must be true when a (non-degenerate) surface was applied.
        assert!(
            corrected
                .calibration_status
                .as_ref()
                .map(|s| s.correction_applied)
                .unwrap_or(false),
            "correction_applied should be true when correction surface was applied"
        );

        // correction_applied must be false when no surface is present at all
        // (checked against a genuinely uncalibrated fixture, distinct from `base`
        // above which has a 0 dB surface purely to control the spillover gate).
        let cal_no_corr = make_h3_test_calibration();
        assert!(cal_no_corr.correction_surface.is_none());
        let cache3 = GainCache::new(false, 1);
        let uncalibrated =
            compute_h3_link_budget(&request, &cal_no_corr, &cache3, std::time::Instant::now())
                .expect("uncalibrated H3 run failed");
        assert!(
            !uncalibrated
                .calibration_status
                .as_ref()
                .map(|s| s.correction_applied)
                .unwrap_or(true),
            "correction_applied should be false when no correction surface"
        );
    }

    /// Pin the h3 G/T output (P5): for a known temperature, every cell's
    /// `g_over_t_db` must equal `gain_db − 10·log₁₀(T)` — the same formula as
    /// `pattern::compute_g_over_t` — and must be absent when no temperature is
    /// provided. Written against the pre-unification inline expression and kept
    /// green across the P5 consolidation onto `pattern::g_over_t_from_gain_db`.
    #[test]
    fn test_h3_g_over_t_matches_gain_minus_10log10_t() {
        let mut request = make_h3_test_request();
        request.temperature_k = Some(150.0);
        let cal = make_h3_test_calibration();

        let cache = GainCache::new(false, 1);
        let result = compute_h3_link_budget(&request, &cal, &cache, std::time::Instant::now())
            .expect("H3 run with temperature failed");

        assert!(!result.cells.is_empty(), "expected at least one cell");
        for cell in &result.cells {
            let g_over_t = cell
                .g_over_t_db
                .expect("temperature_k provided → g_over_t_db must be present");
            let expected = cell.gain_db - 10.0 * 150.0_f64.log10();
            assert!(
                (g_over_t - expected).abs() < 1e-9,
                "cell {}: g_over_t_db {:.9} != gain_db {:.9} − 10·log10(150) = {:.9}",
                cell.cell_id,
                g_over_t,
                cell.gain_db,
                expected
            );
        }

        // Without a temperature, G/T must be absent.
        let request_no_t = make_h3_test_request();
        let cache2 = GainCache::new(false, 1);
        let result_no_t =
            compute_h3_link_budget(&request_no_t, &cal, &cache2, std::time::Instant::now())
                .expect("H3 run without temperature failed");
        assert!(result_no_t.cells.iter().all(|c| c.g_over_t_db.is_none()));
    }

    /// SERVED h3 PATH — F7 floor ON (redesign 2026-07-16). The h3 per-cell path
    /// sets `apply_sidelobe_floor = calibration.physics_is_uncorrected()` — the
    /// same P11 predicate as the evaluator — so for an uncalibrated antenna every
    /// off-axis ring cell carries the statistical floor: forward cells report the
    /// incoherent power sum (raw PO + floor), which can never dip BELOW the floor,
    /// and any rear cell reports the floor alone. This pins that the floor IS
    /// applied on the h3 served path: no uncalibrated cell falls below the pedestal.
    ///
    /// Geometry mirrors the P8 off-axis h3 test: a low-altitude vehicle so the
    /// n_rings=2 ground cells fan out tens of degrees off boresight, deep in the
    /// sidelobes. The Ruze floor is diameter-independent, but it DOES scale with
    /// mesh transmission, so `floor_cfg` mirrors the antenna's mesh to reproduce
    /// `floor_db` exactly.
    #[test]
    fn test_h3_sidelobe_floor_on_for_uncorrected_physics() {
        use crate::model::{
            wavelength_from_frequency, AntennaConfiguration, FeedParameters as ModelFeedParams,
            FeedPosition, MeshParameters as ModelMeshParams, ReflectorGeometry as ModelReflector,
        };

        let geo = |lon: f64, lat: f64, alt: f64| Position3D::geodetic(lon, lat, alt);
        let request = H3LinkBudgetRequest {
            antenna_id: "h3_test_antenna".to_string(),
            feed_id: "h3_test_feed".to_string(),
            vehicle_position: geo(-118.1234, 34.5678, 100.0),
            reflector_boresight: geo(-118.1234, 34.5679, 110.0),
            feed_pointing_location: geo(-118.1234, 34.5679, 110.0), // feed aimed at boresight → at focus
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            n_rings: 2,
            h3_resolution: Some(7),
            temperature_k: None,
            vehicle_attitude: None,
        };

        // High surface RMS (3 mm) so the Ruze pedestal (which saturates at
        // 4π/Ω_scatter ≈ +8 dBi as η_ruze → 0) is a clearly nonzero pedestal to
        // compare against. The floor is diameter-independent, but scales with mesh
        // transmission, so `floor_cfg` carries the same 3 mm RMS AND the same mesh
        // (5 mm / 0.5 mm) as `make_h3_test_calibration` to reproduce `floor_db`
        // exactly.
        const SURFACE_RMS_MM: f64 = 3.0;
        let wavelength = wavelength_from_frequency(8400.0e6);
        let floor_cfg = AntennaConfiguration::new(
            "floor_ref".into(),
            "floor_ref".into(),
            ModelReflector::new(10.0, 5.0, SURFACE_RMS_MM / 1000.0).unwrap(),
            ModelFeedParams::new(FeedPosition::at_focus(5.0), 8.0, 0.0, 1.0).unwrap(),
            Some(
                ModelMeshParams::builder()
                    .spacing(0.005)
                    .wire_diameter(0.0005)
                    .build()
                    .unwrap(),
            ),
        )
        .unwrap();
        let floor_db =
            10.0 * crate::model::pattern::sidelobe_floor_gain(&floor_cfg, wavelength).log10();

        // Uncalibrated: no correction surface → floor ON. Every off-axis ring cell
        // is the power sum (raw PO + floor), so none can fall below the pedestal.
        let mut cal_unc = make_h3_test_calibration();
        cal_unc.physical_config.reflector.surface_rms_mm = SURFACE_RMS_MM;
        assert!(cal_unc.correction_surface.is_none());
        let cache_unc = GainCache::new(false, 1);
        let uncal =
            compute_h3_link_budget(&request, &cal_unc, &cache_unc, std::time::Instant::now())
                .expect("uncalibrated h3 run failed");
        assert!(
            uncal.cells.len() > 1,
            "need ring cells to exercise the off-axis path"
        );

        let min_uncal = uncal
            .cells
            .iter()
            .map(|c| c.gain_db)
            .fold(f64::INFINITY, f64::min);
        assert!(
            min_uncal >= floor_db - 1e-6,
            "F7 floor ON: no uncalibrated off-axis cell may fall below the Ruze floor \
             {floor_db} dB (power sum forward / floor-only rear), got min {min_uncal}"
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
        req_squint.feed_pointing_location = Position3D::geodetic(
            req_squint.reflector_boresight.x + 0.05,
            req_squint.reflector_boresight.y,
            req_squint.reflector_boresight.z,
        );
        req_squint.pointing_frequency_mhz = Some(req_squint.frequency_mhz * 1.4);
        req_baseline.feed_pointing_location = req_squint.feed_pointing_location.clone();

        let cache1 = GainCache::new(false, 1);
        let resp_baseline = compute_h3_link_budget(
            &req_baseline,
            &calibration,
            &cache1,
            std::time::Instant::now(),
        )
        .unwrap();
        let cache2 = GainCache::new(false, 1);
        let resp_squint = compute_h3_link_budget(
            &req_squint,
            &calibration,
            &cache2,
            std::time::Instant::now(),
        )
        .unwrap();

        let gains_baseline: Vec<f64> = resp_baseline.cells.iter().map(|c| c.gain_db).collect();
        let gains_squint: Vec<f64> = resp_squint.cells.iter().map(|c| c.gain_db).collect();
        assert_eq!(gains_baseline.len(), gains_squint.len());
        assert_ne!(
            gains_baseline, gains_squint,
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
        req.feed_pointing_location = Position3D::geodetic(
            req.reflector_boresight.x + 0.05,
            req.reflector_boresight.y,
            req.reflector_boresight.z,
        );
        req.pointing_frequency_mhz = Some(req.frequency_mhz * 1.4);
        let cache = GainCache::new(false, 1);
        let resp =
            compute_h3_link_budget(&req, &calibration, &cache, std::time::Instant::now()).unwrap();
        assert!(
            resp.beam_squint_deg.is_some_and(|s| s > 0.0),
            "expected Some(squint>0), got {:?}",
            resp.beam_squint_deg
        );

        // Displace the feed too, so this asserts None comes from pointing == None — not
        // merely from zero feed displacement.
        let mut req_none = make_h3_test_request();
        req_none.feed_pointing_location = Position3D::geodetic(
            req_none.reflector_boresight.x + 0.05,
            req_none.reflector_boresight.y,
            req_none.reflector_boresight.z,
        );
        req_none.pointing_frequency_mhz = None;
        let cache_none = GainCache::new(false, 1);
        let resp_none = compute_h3_link_budget(
            &req_none,
            &calibration,
            &cache_none,
            std::time::Instant::now(),
        )
        .unwrap();
        assert!(resp_none.beam_squint_deg.is_none(), "no offset -> None");
    }

    /// P1 cross-endpoint consistency: for an uncalibrated antenna (no correction
    /// surface), the h3 endpoint must apply the same physical spillover reduction
    /// as the `/gain` endpoint for the identical geometry.
    ///
    /// `make_h3_test_request` uses `n_rings: 0`, so `compute_h3_link_budget` yields
    /// exactly one cell (the center cell), which is therefore also the grid peak — so
    /// `loss_db` is 0 and `gain_db` is the raw (spillover-gated) physics+correction
    /// gain for that single point. We reconstruct the identical emitter geometry
    /// (feed lat/lon -> H3 center cell -> ECEF at alt 0, same resolution-selection
    /// logic) and feed it through `compute_gain_from_request` (the `/gain` path,
    /// already spillover-wired by P1 task 2) to check both endpoints agree to
    /// within numerical noise.
    #[test]
    fn test_h3_consistent_with_gain_endpoint_for_uncalibrated_antenna() {
        use crate::data::repository::CalibrationRepository;
        use crate::service::evaluator::compute_gain_from_request;

        let calibration = make_h3_test_calibration();
        assert!(
            calibration.correction_surface.is_none(),
            "fixture must be uncalibrated (no correction surface) for this test to be meaningful"
        );

        let request = make_h3_test_request();
        assert_eq!(request.n_rings, 0, "test relies on a single-cell grid");

        let cache = GainCache::new(false, 1);
        let h3_response =
            compute_h3_link_budget(&request, &calibration, &cache, std::time::Instant::now())
                .expect("h3 link budget computation failed");
        assert_eq!(
            h3_response.cells.len(),
            1,
            "n_rings=0 must yield exactly one cell"
        );
        let cell = &h3_response.cells[0];

        let gain_request = center_cell_gain_request(&request);

        let mut repo = CalibrationRepository::new();
        repo.add_calibration(calibration);
        let gain_response = compute_gain_from_request(&gain_request, &repo)
            .expect("/gain computation failed for the reconstructed geometry");

        // Sanity: confirm spillover was actually applied on the /gain path, so this
        // comparison is non-vacuous (both endpoints reduce gain, not both skip it).
        assert!(
            gain_response.metadata.spillover_loss_db.is_some(),
            "expected spillover to be applied on the /gain path for an uncalibrated antenna"
        );

        assert!(
            (cell.gain_db - gain_response.gain_db).abs() < 1e-6,
            "h3 cell gain {:.9} dB should match /gain endpoint gain {:.9} dB for the same \
             geometry (both spillover-gated identically for an uncalibrated antenna)",
            cell.gain_db,
            gain_response.gain_db
        );
    }

    /// Issue #63: the H3 adapter and `/gain` must consume the same final served result for
    /// geometrically identical one-cell requests. The scenarios cover the three calibration
    /// states, a real beam-squint offset, correction application, and full-coverage rejection
    /// at an uncalibrated frequency. Warning comparison is set-wise because H3 deliberately
    /// sorts its grid aggregate while `/gain` preserves served-law assembly order.
    #[test]
    fn one_cell_h3_matches_gain_for_all_calibration_states() {
        use crate::data::repository::CalibrationRepository;
        use crate::service::evaluator::compute_gain_from_request;

        let full_coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 180.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(1000)
            .has_correction_surface(true)
            .build()
            .unwrap();
        let partial_coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 180.0)
            .frequency_range(8000.0, 8300.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();
        let spatially_disjoint_coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(80.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let mut uncalibrated = make_h3_test_calibration();
        uncalibrated.calibration_status = Some(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        let mut calibrated = make_h3_test_calibration();
        calibrated.correction_surface = Some(constant_surface_db(2.0));
        calibrated.calibration_coverage = Some(full_coverage);

        let mut partial = make_h3_test_calibration();
        partial.calibration_status = Some(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: partial_coverage.clone(),
        });
        partial.correction_surface = Some(constant_surface_db(2.0));
        partial.calibration_coverage = Some(partial_coverage);

        let mut spatially_outside = make_h3_test_calibration();
        spatially_outside.calibration_status = Some(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: spatially_disjoint_coverage.clone(),
        });
        spatially_outside.correction_surface = Some(constant_surface_db(2.0));
        spatially_outside.calibration_coverage = Some(spatially_disjoint_coverage);

        for (name, calibration, with_squint, correction_applied, warning_codes) in [
            (
                "uncalibrated",
                uncalibrated,
                false,
                false,
                vec![WarningCode::Uncalibrated],
            ),
            ("calibrated", calibrated, true, true, vec![]),
            (
                "partially calibrated outside frequency coverage",
                partial,
                true,
                false,
                vec![
                    WarningCode::PartiallyCalibrated,
                    WarningCode::CorrectionNotApplied,
                ],
            ),
            (
                "partially calibrated outside spatial coverage",
                spatially_outside,
                true,
                false,
                vec![
                    WarningCode::PartiallyCalibrated,
                    WarningCode::OutOfCoverage,
                    WarningCode::CorrectionNotApplied,
                ],
            ),
        ] {
            let mut request = make_h3_test_request();
            if with_squint {
                request.feed_pointing_location = Position3D::geodetic(
                    request.reflector_boresight.x + 0.05,
                    request.reflector_boresight.y,
                    request.reflector_boresight.z,
                );
                request.pointing_frequency_mhz = Some(request.frequency_mhz * 1.4);
            }

            let cache = GainCache::new(true, 100);
            let h3 =
                compute_h3_link_budget(&request, &calibration, &cache, std::time::Instant::now())
                    .unwrap_or_else(|error| panic!("{name}: H3 failed: {error}"));
            assert_eq!(h3.cells.len(), 1, "{name}: expected one H3 cell");

            let gain_request = center_cell_gain_request(&request);
            let mut repository = CalibrationRepository::new();
            repository.add_calibration(calibration);
            let gain = compute_gain_from_request(&gain_request, &repository)
                .unwrap_or_else(|error| panic!("{name}: /gain failed: {error}"));

            assert_eq!(h3.cells[0].gain_db, gain.gain_db, "{name}: final gain");
            assert_eq!(
                h3.cells[0].azimuth_deg, gain.geometry.emitter_azimuth_deg,
                "{name}: corrected E-clock"
            );
            assert_eq!(
                h3.cells[0].elevation_deg, gain.geometry.emitter_elevation_deg,
                "{name}: corrected E-cone"
            );
            assert_eq!(
                h3.beam_squint_deg, gain.geometry.beam_squint_deg,
                "{name}: squint"
            );
            assert_eq!(
                h3.calibration_status
                    .as_ref()
                    .map(|status| status.correction_applied),
                gain.calibration_status
                    .as_ref()
                    .map(|status| status.correction_applied),
                "{name}: correction evidence"
            );
            assert_eq!(
                h3.calibration_status
                    .as_ref()
                    .map(|status| status.correction_applied),
                Some(correction_applied),
                "{name}: expected correction disposition"
            );
            for code in warning_codes {
                assert!(
                    h3.warnings.iter().any(|warning| warning.is(code)),
                    "{name}: missing {code:?} advisory"
                );
            }

            let mut gain_warnings = gain.warnings;
            gain_warnings.sort();
            assert_eq!(h3.warnings, gain_warnings, "{name}: served warnings");

            let hot = compute_h3_link_budget(
                &request,
                &repository
                    .get_calibration(&request.antenna_id, &request.feed_id)
                    .unwrap(),
                &cache,
                std::time::Instant::now(),
            )
            .unwrap_or_else(|error| panic!("{name}: hot-cache H3 failed: {error}"));
            assert_eq!(hot.cells, h3.cells, "{name}: hot-cache cell results");
            assert_eq!(hot.warnings, h3.warnings, "{name}: hot-cache warnings");
            assert_eq!(
                hot.calibration_status, h3.calibration_status,
                "{name}: hot-cache correction evidence"
            );
            assert_eq!(
                hot.beam_squint_deg, h3.beam_squint_deg,
                "{name}: hot-cache squint"
            );
        }
    }

    /// Issue #63: sequential and automatic parallel traversal are observationally
    /// identical for the same 37-cell grid. The sequential pass fills the shared cache;
    /// the zero-budget automatic pass can succeed only by taking concurrent cache hits.
    /// Only wall-clock timing is normalized; cells, sorted warnings, correction evidence,
    /// peak-relative losses, path loss, G/T, and failure counts compare exactly.
    #[test]
    fn sequential_and_parallel_traversals_are_equivalent() {
        let mut calibration = make_h3_test_calibration();
        calibration.correction_surface = Some(constant_surface_db(2.0));

        let mut request = make_h3_test_request();
        request.n_rings = 3;
        request.temperature_k = Some(290.0);
        let cache = GainCache::new(true, 100);

        let mut sequential = compute_h3_link_budget_with_traversal(
            &request,
            &calibration,
            &cache,
            std::time::Instant::now(),
            DEFAULT_INTEGRATION_BUDGET,
            CellTraversal::Sequential,
        )
        .unwrap();
        let mut parallel = compute_h3_link_budget_with_traversal(
            &request,
            &calibration,
            &cache,
            std::time::Instant::now(),
            Duration::ZERO,
            CellTraversal::Automatic,
        )
        .unwrap();

        assert_eq!(parallel.cells.len(), 37);
        sequential.metadata.computation_time_ms = 0.0;
        parallel.metadata.computation_time_ms = 0.0;
        assert_eq!(sequential, parallel);
        assert!(parallel.warnings.windows(2).all(|pair| pair[0] < pair[1]));

        // Failure-count parity is non-vacuous: an empty cache and zero budget make every
        // cell fail under both traversals, including the automatic Rayon branch.
        let mut sequential_failures = compute_h3_link_budget_with_traversal(
            &request,
            &calibration,
            &GainCache::new(false, 1),
            std::time::Instant::now(),
            Duration::ZERO,
            CellTraversal::Sequential,
        )
        .unwrap();
        let mut parallel_failures = compute_h3_link_budget_with_traversal(
            &request,
            &calibration,
            &GainCache::new(false, 1),
            std::time::Instant::now(),
            Duration::ZERO,
            CellTraversal::Automatic,
        )
        .unwrap();

        assert_eq!(parallel_failures.metadata.failed_points, 37);
        sequential_failures.metadata.computation_time_ms = 0.0;
        parallel_failures.metadata.computation_time_ms = 0.0;
        assert_eq!(sequential_failures, parallel_failures);
    }

    /// Issue #63: warnings intended to occur once per grid retain one constant message
    /// across cells, so whole-object deduplication yields one entry for each warning code.
    #[test]
    fn grid_wide_advisory_messages_deduplicate_across_cells() {
        let mut calibration = make_h3_test_calibration();
        calibration.calibration_status = Some(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        calibration.physical_config.feed.position = (3.0, 0.0, 0.0);

        let mut request = make_h3_test_request();
        request.n_rings = 1;
        let response = compute_h3_link_budget(
            &request,
            &calibration,
            &GainCache::new(true, 100),
            std::time::Instant::now(),
        )
        .unwrap();

        assert_eq!(response.cells.len(), 7);
        for code in [
            WarningCode::Uncalibrated,
            WarningCode::SevereFeedOffset,
            WarningCode::RayTraceDegraded,
        ] {
            assert_eq!(
                response
                    .warnings
                    .iter()
                    .filter(|warning| warning.is(code))
                    .count(),
                1,
                "{code:?} must have one constant message across all cells"
            );
        }
    }

    /// Issue #63 regression: a severe feed offset normally selects the ray-tracing stub,
    /// but an uncorrected rear-hemisphere direction takes the floor-only shortcut before
    /// that dispatch. H3 must consume the served warning set and therefore must not restore
    /// the ray-trace warning that the skipped operation never earned.
    #[test]
    fn h3_rear_floor_shortcut_does_not_invent_ray_trace_warning() {
        use crate::data::repository::CalibrationRepository;
        use crate::service::evaluator::compute_gain_from_request;

        let mut calibration = make_h3_test_calibration();
        calibration.calibration_status = Some(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        let mut request = make_h3_test_request();
        request.vehicle_position = Position3D::geodetic(0.0, 0.0, 400_000.0);
        request.reflector_boresight = Position3D::geodetic(20.0, 0.0, 0.0);
        request.feed_pointing_location = Position3D::geodetic(-20.0, 0.0, 0.0);

        let cache = GainCache::new(true, 100);
        let h3 = compute_h3_link_budget(&request, &calibration, &cache, std::time::Instant::now())
            .unwrap();
        assert!(h3.cells[0].elevation_deg > 90.0);
        assert!(h3
            .warnings
            .iter()
            .any(|warning| warning.is(WarningCode::RearHemisphereInvalid)));
        assert!(h3
            .warnings
            .iter()
            .any(|warning| warning.is(WarningCode::SevereFeedOffset)));
        assert!(!h3
            .warnings
            .iter()
            .any(|warning| warning.is(WarningCode::RayTraceDegraded)));

        let mut repository = CalibrationRepository::new();
        repository.add_calibration(calibration);
        let gain_request = center_cell_gain_request(&request);
        let gain = compute_gain_from_request(&gain_request, &repository).unwrap();
        let mut gain_warnings = gain.warnings;
        gain_warnings.sort();
        assert_eq!(h3.warnings, gain_warnings);

        let forward = compute_gain_from_request(
            &GainRequest {
                emitter_position: request.reflector_boresight.clone(),
                ..gain_request
            },
            &repository,
        )
        .unwrap();
        assert!(
            forward
                .warnings
                .iter()
                .any(|warning| warning.is(WarningCode::RayTraceDegraded)),
            "the forward direction proves this request's feed offset is severe"
        );

        // A forward H3 request whose artifact supplies the severe offset reaches the stub;
        // its warning is reconstructed on the second request's cache hit.
        let mut forward_calibration = make_h3_test_calibration();
        forward_calibration.physical_config.feed.position = (3.0, 0.0, 0.0);
        let forward_request = make_h3_test_request();
        let forward_cache = GainCache::new(true, 100);
        for pass in ["cold", "hot"] {
            let response = compute_h3_link_budget(
                &forward_request,
                &forward_calibration,
                &forward_cache,
                std::time::Instant::now(),
            )
            .unwrap_or_else(|error| panic!("{pass} forward H3 failed: {error}"));
            assert!(
                response
                    .warnings
                    .iter()
                    .any(|warning| warning.is(WarningCode::RayTraceDegraded)),
                "{pass} forward H3 must retain ray_trace_degraded"
            );
        }

        // A corrected rear direction does not take the uncorrected floor-only shortcut,
        // so it also reaches the stub and retains the warning on a cache hit.
        let mut corrected_rear = make_h3_test_calibration();
        corrected_rear.correction_surface = Some(constant_surface_db(2.0));
        let corrected_cache = GainCache::new(true, 100);
        for pass in ["cold", "hot"] {
            let response = compute_h3_link_budget(
                &request,
                &corrected_rear,
                &corrected_cache,
                std::time::Instant::now(),
            )
            .unwrap_or_else(|error| panic!("{pass} corrected-rear H3 failed: {error}"));
            assert!(
                response
                    .warnings
                    .iter()
                    .any(|warning| warning.is(WarningCode::RayTraceDegraded)),
                "{pass} corrected-rear H3 must retain ray_trace_degraded"
            );
        }
    }

    /// C9 regression: `loss_db` is referenced to the **grid peak**, not the grid centre.
    ///
    /// The design feed is displaced laterally by 0.3 m on a 10 m / f=5 m dish (0.06·f), which
    /// steers the beam well off the pointing target, so the peak cell is emphatically *not*
    /// the centre cell. Under the pre-C9 centre-cell reference this geometry produced
    /// `loss_db == 0` at the centre and **negative** losses at the stronger cells; the
    /// assertions below fail outright on that code.
    #[test]
    fn h3_loss_is_referenced_to_the_grid_peak_not_the_centre_cell() {
        let mut calibration = make_h3_test_calibration();
        // Lateral design-feed displacement → steered beam → peak away from grid centre.
        calibration.physical_config.feed.position = (0.3, 0.0, 0.0);

        let mut request = make_h3_test_request();
        request.n_rings = 2; // 19 cells

        let cache = GainCache::new(false, 1);
        let response =
            compute_h3_link_budget(&request, &calibration, &cache, std::time::Instant::now())
                .expect("h3 link budget computation failed");
        assert_eq!(response.cells.len(), 19, "n_rings=2 must yield 19 cells");
        assert_eq!(response.metadata.failed_points, 0, "no cell should fail");

        // Non-vacuous: the steered beam must actually put the peak off the centre cell,
        // otherwise this test would pass under the old rule too.
        let centre = response
            .cells
            .iter()
            .find(|c| c.cell_id == response.center_cell_id)
            .expect("centre cell must be present");
        assert!(
            centre.gain_db < response.metadata.peak_gain_db - 0.5,
            "test geometry is vacuous: centre cell gain {:.3} dB is (near) the grid peak \
             {:.3} dB — the steered beam should have moved the peak off centre",
            centre.gain_db,
            response.metadata.peak_gain_db
        );
        assert!(
            centre.loss_db > 0.5,
            "the centre cell is no longer the reference, so its loss must be positive; got {}",
            centre.loss_db
        );

        let mut zero_loss_cells = 0usize;
        for cell in &response.cells {
            assert!(
                cell.loss_db >= 0.0,
                "cell {}: loss_db must never be negative under a peak reference, got {}",
                cell.cell_id,
                cell.loss_db
            );
            // Internally re-derivable from the values the response itself reports.
            assert!(
                (cell.loss_db - (response.metadata.peak_gain_db - cell.gain_db)).abs() < 1e-9,
                "cell {}: loss_db {} != peak_gain_db {} - gain_db {}",
                cell.cell_id,
                cell.loss_db,
                response.metadata.peak_gain_db,
                cell.gain_db
            );
            assert!(
                cell.total_path_loss_db >= cell.free_space_path_loss_db,
                "cell {}: total_path_loss_db {} must not fall below free-space {}",
                cell.cell_id,
                cell.total_path_loss_db,
                cell.free_space_path_loss_db
            );
            if cell.loss_db == 0.0 {
                zero_loss_cells += 1;
                assert!(
                    (cell.gain_db - response.metadata.peak_gain_db).abs() < 1e-12,
                    "the zero-loss cell must be the peak cell"
                );
            }
        }
        assert_eq!(
            zero_loss_cells, 1,
            "exactly one cell (the peak) should have loss_db == 0.0"
        );
    }

    /// Issue #63: once every cell's physics term is cached, a zero integration budget
    /// cannot make the repeated request fail. This proves H3 cache hits skip integration;
    /// the neighbouring all-miss test proves the same budget still applies independently
    /// to every cache miss.
    #[test]
    fn h3_hot_cache_skips_integration_even_with_zero_budget() {
        let calibration = make_h3_test_calibration();
        let mut request = make_h3_test_request();
        request.n_rings = 1;
        let cache = GainCache::new(true, 100);

        let cold =
            compute_h3_link_budget(&request, &calibration, &cache, std::time::Instant::now())
                .unwrap();
        assert_eq!(cold.metadata.failed_points, 0);

        let hot = compute_h3_link_budget_with_budget(
            &request,
            &calibration,
            &cache,
            std::time::Instant::now(),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(hot.metadata.failed_points, 0);
        assert_eq!(hot.cells, cold.cells);
        assert_eq!(hot.warnings, cold.warnings);
        assert_eq!(hot.calibration_status, cold.calibration_status);
    }

    /// C9 degenerate case: when *no* cell yields a finite gain there is no peak to
    /// reference. The pre-C9 code fell back to the separate boresight evaluation, which no
    /// longer exists. Assert the response stays finite and self-describing rather than
    /// serializing `peak_gain_db` as JSON `null`.
    #[test]
    fn h3_all_cells_failed_reports_a_finite_peak_sentinel() {
        let mut calibration = make_h3_test_calibration();
        calibration.physical_config.feed.position = (3.0, 0.0, 0.0);
        let mut request = make_h3_test_request();
        request.n_rings = 1; // 7 cells

        let cache = GainCache::new(false, 1);
        // A zero wall-clock budget makes every per-cell aperture integration abort (S3).
        let response = compute_h3_link_budget_with_budget(
            &request,
            &calibration,
            &cache,
            std::time::Instant::now(),
            Duration::ZERO,
        )
        .expect("the request itself must still succeed when every cell fails");

        assert!(response.cells.is_empty(), "every cell should have failed");
        assert_eq!(response.metadata.failed_points, 7);
        assert_eq!(
            response.metadata.points_evaluated, 7,
            "failed_points == points_evaluated is how a client detects the degenerate case"
        );
        assert!(
            response.metadata.peak_gain_db.is_finite(),
            "peak_gain_db must stay finite (serde emits null for -inf), got {}",
            response.metadata.peak_gain_db
        );
        assert_eq!(
            response.metadata.peak_gain_db,
            crate::service::heatmap::NO_PEAK_GAIN_DB
        );
        assert!(
            response
                .warnings
                .iter()
                .any(|warning| warning.is(WarningCode::SevereFeedOffset)),
            "configuration advisories must survive when every directional evaluation fails"
        );
    }
}
