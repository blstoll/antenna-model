//! Gain Computation Service - Core Pipeline
//!
//! This module implements the end-to-end gain computation workflow, orchestrating
//! coordinate transformations, physics modeling, and correction surface evaluation.
//!
//! # Pipeline Overview
//!
//! ```text
//! ┌─────────────────┐
//! │ GainRequest     │  Input: 3D positions (ECEF/Geodetic), frequencies, antenna ID
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 1: Load Calibration Data                                       │
//! │ - Retrieve antenna configuration (reflector, feed, mesh)            │
//! │ - Load correction surface (B-spline) if calibrated                  │
//! │ - Get validity ranges and calibration status                        │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 2: Coordinate Transformations                                  │
//! │ - Convert emitter/feed/vehicle positions to antenna frame           │
//! │ - Compute azimuth/elevation angles (θ, φ)                           │
//! │ - Apply beam squint correction for frequency offset                 │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 3: Physics Model Computation                                   │
//! │ - Aperture integration (physical optics) or ray tracing             │
//! │ - Phase accumulation: path + coma + surface + mesh                  │
//! │ - Apply Ruze efficiency (surface RMS) and mesh transparency         │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 4: Correction Surface Evaluation                               │
//! │ - Interpolate B-spline correction (if calibrated)                   │
//! │ - Add correction to physics model: Gain_final = Gain_phys + ΔG      │
//! │ - Generate warnings for extrapolation outside calibrated range      │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 5: G/T Computation (if temperature provided)                   │
//! │ - Compute G/T ratio from gain and system temperature                │
//! │ - Apply overall efficiency (Ruze × mesh)                            │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Step 6: Loss Calculation (if reference gain provided)               │
//! │ - Compute loss = reference_gain - actual_gain (dB)                  │
//! │ - Used for link budget analysis                                     │
//! └────────┬────────────────────────────────────────────────────────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │ GainResponse    │  Output: gain_db, g_over_t_db, loss, warnings, metadata
//! └─────────────────┘
//! ```
//!
//! # Error Handling
//! - Missing antenna/feed → FeedNotFound error
//! - Invalid coordinates → ValidationError
//! - Computation failures → ComputationError with context
//! - Out-of-range queries → Warning (not error)

use crate::api::schemas::{
    CalibrationStatusInfo, ComputationMetadata, GainRequest, GainResponse, GeometryInfo,
};
use crate::data::repository::CalibrationRepository;
use crate::data::types::{CalibrationCoverage, CalibrationStatus};
use crate::error::{AntennaModelError, Result};
use crate::model::{
    compute_emitter_direction_with_attitude, compute_feed_position_from_pointing, compute_gain_db,
    evaluate_correction, squint_corrected_direction, AntennaConfiguration, IntegrationParams,
};
use crate::service::validator::coordinate_ambiguity_warnings;
use std::time::Instant;

/// Compute antenna gain from a gain request
///
/// This is the main entry point for gain computation, transforming 3D positions
/// into antenna frame coordinates and evaluating the physics model.
///
/// # Arguments
///
/// * `request` - The gain request containing vehicle position, reflector boresight, and feed position
/// * `repository` - The calibration data repository
///
/// # Returns
///
/// A `GainResponse` containing the computed gain and metadata
pub fn compute_gain_from_request(
    request: &GainRequest,
    repository: &CalibrationRepository,
) -> Result<GainResponse> {
    let start = Instant::now();
    let mut warnings = Vec::new();

    // Emit ambiguity warnings for positions that may be misclassified by auto-detection
    warnings.extend(coordinate_ambiguity_warnings(request));

    let (emitter_az, emitter_el) = compute_emitter_direction_with_attitude(
        &request.emitter_position,
        &request.vehicle_position,
        &request.reflector_boresight,
        request.vehicle_attitude,
    )?;

    let calibration = repository
        .get_calibration(&request.antenna_id, &request.feed_id)
        .ok_or_else(|| AntennaModelError::FeedNotFound {
            antenna_id: request.antenna_id.clone(),
            feed_id: request.feed_id.clone(),
        })?;

    // Build AntennaConfiguration from calibration data
    // Convert data types to model geometry types
    use crate::model::{
        FeedParameters as ModelFeedParams, FeedPosition, MeshParameters as ModelMeshParams,
        ReflectorGeometry as ModelReflector,
    };

    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let diameter_m = calibration.physical_config.reflector.diameter_m;

    let reflector = ModelReflector::builder()
        .diameter(diameter_m)
        .focal_length(focal_length_m)
        .surface_rms(calibration.physical_config.reflector.surface_rms_mm / 1000.0) // mm to m
        .build()
        .map_err(|e| AntennaModelError::Generic(format!("Failed to build reflector: {}", e)))?;

    // Compute physical feed position from API steering parameters
    // The API's feed_position specifies where the feed is aimed (Earth target location)
    // This computes the corresponding physical feed position in the reflector frame
    let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
        &request.feed_position,
        &request.reflector_boresight,
        &request.vehicle_position,
        focal_length_m,
        diameter_m,
        request.vehicle_attitude,
    )?;

    // Combine steering-induced position with design feed offset
    // The design position represents the physical offset of this feed from the optical axis
    // (e.g., multi-feed antennas have feeds at different physical locations)
    let design_pos = &calibration.physical_config.feed.position;
    let feed_x = steer_x + design_pos.0;
    let feed_y = steer_y + design_pos.1;
    let feed_z = steer_z + design_pos.2;
    let feed_position = FeedPosition::new(feed_x, feed_y, feed_z);

    // Physical feed offset from the focal point in the antenna frame (meters).
    // feed_z is the z-position relative to the reflector vertex; subtracting focal_length_m
    // gives the displacement from the focal point. For an on-axis feed (zero steering offset),
    // this is (0, 0, 0). For a steered feed, x/y are the lateral displacement and z is
    // the (small, second-order) defocus component.
    let feed_offset = crate::api::schemas::Vector3D::new(feed_x, feed_y, feed_z - focal_length_m);

    // Apply beam squint correction if pointing frequency differs from operating frequency.
    // Must be done AFTER computing feed position since squint depends on actual displacement.
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);

    let (corrected_az, corrected_el, squint_magnitude_deg) = squint_corrected_direction(
        emitter_az,
        emitter_el,
        request.frequency_mhz,
        pointing_freq,
        feed_x,
        feed_y,
        focal_length_m,
    );

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

    // Add mesh if present
    if let Some(ref mesh_data) = calibration.physical_config.mesh {
        let mesh = ModelMeshParams::builder()
            .spacing(mesh_data.mesh_spacing_mm / 1000.0) // mm to m
            .wire_diameter(mesh_data.wire_diameter_mm / 1000.0) // mm to m
            .build()
            .map_err(|e| AntennaModelError::Generic(format!("Failed to build mesh: {}", e)))?;
        config_builder = config_builder.mesh(mesh);
    }

    let antenna_config = config_builder.build().map_err(|e| {
        AntennaModelError::Generic(format!("Failed to build antenna configuration: {}", e))
    })?;

    // Use fast integration parameters for <100ms target
    let mut integration_params = IntegrationParams::fast();
    // Double-counting gate: physical spillover only when NO correction surface exists
    // (the surface otherwise absorbs it). Whole-antenna gate — never per query. Note the
    // model layer further restricts spillover to StandardPhysicalOptics mode, so a large
    // feed offset may leave this flag on yet apply no spillover. The ideal-reference
    // computation below tracks the ACTUAL result's spillover state (not this raw flag) so
    // that base spillover cancels in loss_db without introducing a one-sided bias.
    integration_params.apply_spillover = calibration.correction_surface.is_none();

    // Convert frequency from MHz to Hz for physics model
    let frequency_hz = request.frequency_mhz * 1e6;

    // PHYSICS MODEL: Compute gain using full aperture integration
    // Note: theta and phi are in radians in spherical coordinates
    // Our corrected_el uses the convention: elevation = 0° at boresight (Z-axis alignment)
    // Physics model theta uses: theta = 0° at boresight (standard spherical coordinates)
    // These conventions match, so direct conversion:
    tracing::debug!(
        emitter_az = %emitter_az,
        emitter_el = %emitter_el,
        "Computed emitter direction in antenna frame"
    );
    tracing::debug!(
        corrected_az = %corrected_az,
        corrected_el = %corrected_el,
        "Emitter direction after beam squint correction"
    );

    let theta_rad = corrected_el.to_radians();
    let phi_rad = corrected_az.to_radians();

    tracing::debug!(
        theta_rad = %theta_rad,
        phi_rad = %phi_rad,
        feed_x = %feed_x,
        feed_y = %feed_y,
        feed_z = %feed_z,
        "Physics model inputs"
    );

    let result = compute_gain_db(
        theta_rad,
        phi_rad,
        &antenna_config,
        frequency_hz,
        &integration_params,
    )?; // ComputationError automatically converts via #[from]
    let gain_physics = result.gain;

    // Collect edge case warnings from physics computation
    warnings.extend(result.warnings);

    // Apply correction surface (if available and in coverage)
    // Use corrected angles for coverage check and interpolation
    let mut correction_extrapolated = false;
    let in_coverage = calibration.correction_surface.is_some()
        && is_in_coverage(
            &calibration.calibration_coverage,
            corrected_az,
            corrected_el,
            request.frequency_mhz,
        );
    let (correction_db, correction_applied) = match &calibration.correction_surface {
        Some(correction) if in_coverage => {
            let result = evaluate_correction(
                correction,
                corrected_az,
                corrected_el,
                request.frequency_mhz,
                calibration.validity_ranges.temperature_const,
            )?;
            correction_extrapolated = result.extrapolated;
            warnings.extend(result.warnings);
            (result.correction_db, true)
        }
        _ => (0.0, false),
    };

    // Determine whether this result was extrapolated:
    // - Correction surface was applied but the query was outside its B-spline knot range, OR
    // - Correction surface exists but was not applied because the query is outside coverage.
    let out_of_coverage = calibration.correction_surface.is_some() && !correction_applied;
    let extrapolated = correction_extrapolated || out_of_coverage;

    let final_gain_db = gain_physics + correction_db;

    // Compute reference gain if requested.
    //
    // The reference is the boresight gain of an IDEAL version of this antenna (feed at
    // the focal point, perfect surface), evaluated through the SAME `compute_gain_db`
    // pipeline as the actual gain. Because both numbers come from the identical
    // aperture-directivity formula, `loss_db` has no built-in offset: it is purely the
    // pointing/aberration loss (≈0 dB at boresight with a focused feed).
    let (reference_gain_db, loss_db) = if request.include_reference {
        let ideal_reflector = ModelReflector::new(diameter_m, focal_length_m, 0.0)
            .map_err(|e| AntennaModelError::Generic(format!("ideal reflector: {e}")))?;
        let ideal_feed = ModelFeedParams::new(
            FeedPosition::at_focus(focal_length_m),
            calibration.physical_config.feed.q_factor,
            calibration.physical_config.feed.phase_center_offset_m,
            1.0,
        )
        .map_err(|e| AntennaModelError::Generic(format!("ideal feed: {e}")))?;
        let ideal_config = AntennaConfiguration::new(
            format!("{}_ideal", calibration.antenna_id),
            "ideal".into(),
            ideal_reflector,
            ideal_feed,
            antenna_config.mesh.clone(),
        )
        .map_err(|e| AntennaModelError::Generic(format!("ideal config: {e}")))?;
        // Match the reference's spillover to the ACTUAL path: if the actual was in a mode
        // where spillover was folded in (StandardPhysicalOptics), apply it to the ideal
        // reference too so the base spillover cancels in loss_db; if the actual did NOT get
        // spillover (large offset / non-standard mode, or calibrated), the reference must
        // not either, keeping loss_db free of a one-sided spillover bias.
        let mut reference_params = integration_params.clone();
        reference_params.apply_spillover = result.spillover_loss_db.is_some();
        let reference = compute_gain_db(0.0, 0.0, &ideal_config, frequency_hz, &reference_params)?;

        // Loss is reference minus actual gain (final gain, including the
        // correction surface when it was applied).
        (Some(reference.gain), Some(reference.gain - final_gain_db))
    } else {
        (None, None)
    };

    // Generate warnings based on calibration status (use corrected angles)
    let calibration_warnings =
        generate_calibration_warnings(&calibration, corrected_az, corrected_el, correction_applied);
    warnings.extend(calibration_warnings);

    // Build calibration status info
    let calibration_status_info = calibration.calibration_status.as_ref().map(|status| {
        let mut info = CalibrationStatusInfo::from(status);
        info.correction_applied = correction_applied;
        info
    });

    Ok(GainResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        gain_db: final_gain_db,
        reference_gain_db,
        loss_db,
        geometry: GeometryInfo {
            feed_offset_meters: feed_offset,
            emitter_azimuth_deg: corrected_az,
            emitter_elevation_deg: corrected_el,
            beam_squint_deg: if squint_magnitude_deg > 0.001 {
                Some(squint_magnitude_deg)
            } else {
                None
            },
        },
        warnings,
        metadata: ComputationMetadata {
            computation_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            coordinate_transform_ms: None,
            physics_model_ms: None,
            correction_surface_ms: None,
            extrapolated,
            spillover_loss_db: result.spillover_loss_db,
        },
        calibration_status: calibration_status_info,
    })
}

/// Check if the query is within the calibrated coverage region.
///
/// When no coverage restriction is recorded (`None`) the correction surface is
/// treated as valid everywhere it has data — the query is considered in-coverage.
/// Actual correction application is still gated separately on
/// `correction_surface.is_some()`, so returning `true` here for `None` is safe.
///
/// When a `CalibrationCoverage` is present (partially calibrated artifact), the
/// query must fall within the specified azimuth, elevation, and frequency ranges.
pub(crate) fn is_in_coverage(
    coverage: &Option<CalibrationCoverage>,
    azimuth_deg: f64,
    elevation_deg: f64,
    frequency_mhz: f64,
) -> bool {
    match coverage {
        Some(cov) => {
            azimuth_deg >= cov.azimuth_range.0
                && azimuth_deg <= cov.azimuth_range.1
                && elevation_deg >= cov.elevation_range.0
                && elevation_deg <= cov.elevation_range.1
                && frequency_mhz >= cov.frequency_range.0
                && frequency_mhz <= cov.frequency_range.1
        }
        // No coverage restriction recorded (fully calibrated artifact):
        // the correction surface applies everywhere it has data.
        None => true,
    }
}

/// Generate warnings based on calibration status and query parameters.
///
/// Returns a vector of warning messages to be included in the response.
fn generate_calibration_warnings(
    calibration: &crate::data::types::AntennaCalibration,
    azimuth_deg: f64,
    elevation_deg: f64,
    correction_applied: bool,
) -> Vec<String> {
    let mut warnings = Vec::new();

    // Get calibration status (default to FullyCalibrated if not specified for backward compatibility)
    let status = calibration.calibration_status.as_ref();

    match status {
        Some(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db,
            loss_accuracy_estimate_db,
        }) => {
            warnings.push(format!(
                "Antenna '{}' is uncalibrated (using design specifications). \
                 Absolute gain accuracy: ±{:.1} dB, Loss accuracy: ±{:.1} dB",
                calibration.antenna_id, accuracy_estimate_db, loss_accuracy_estimate_db
            ));
        }
        Some(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db,
            coverage,
        }) => {
            warnings.push(format!(
                "Antenna '{}' is partially calibrated. Accuracy estimate: ±{:.1} dB",
                calibration.antenna_id, accuracy_estimate_db
            ));

            // Check if query is outside calibrated spatial region (azimuth/elevation)
            let in_spatial_coverage = azimuth_deg >= coverage.azimuth_range.0
                && azimuth_deg <= coverage.azimuth_range.1
                && elevation_deg >= coverage.elevation_range.0
                && elevation_deg <= coverage.elevation_range.1;

            if !in_spatial_coverage {
                warnings.push(
                    "Query is outside calibrated region - using physics model extrapolation"
                        .to_string(),
                );
            }
        }
        Some(CalibrationStatus::FullyCalibrated { .. }) | None => {
            // No calibration warnings for fully calibrated antennas
        }
    }

    // Warn if correction surface exists but wasn't applied
    if !correction_applied && calibration.correction_surface.is_some() {
        warnings.push("Correction surface not applied (out of coverage)".to_string());
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schemas::{CoordinateSystem, Position3D};
    use crate::data::types::{
        AntennaCalibration, CalibrationCoverage, CalibrationMetadata, CalibrationStatus,
        FeedParameters, MeshParameters, PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
    };

    fn create_test_calibration(status: CalibrationStatus) -> AntennaCalibration {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-01T00:00:00Z")
            .format_version("2.0")
            .data_source("test")
            .rmse_db(0.5)
            .r_squared(0.99)
            .num_measurements(1000)
            .build()
            .unwrap();

        let mut builder = AntennaCalibration::builder()
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
                    // Feed at focal point - zero offset from optical axis
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
            });

        builder = builder.calibration_status(status.clone());

        // Add coverage for partially calibrated
        if let CalibrationStatus::PartiallyCalibrated { ref coverage, .. } = status {
            builder = builder.calibration_coverage(coverage.clone());
        }

        builder.build().unwrap()
    }

    fn create_test_request() -> GainRequest {
        // Emitter is a LEO satellite at 400 km geodetic altitude. Set explicit
        // coordinate_system to prevent ambiguity warnings in tests that assert
        // response.warnings.is_empty().
        let mut emitter = Position3D::new(-117.0, 35.0, 400_000.0);
        emitter.coordinate_system = Some(CoordinateSystem::Geodetic);
        GainRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: Position3D::new(-118.0, 34.0, 100.0),
            reflector_boresight: Position3D::new(-117.99, 34.01, 110.0), // 10m from vehicle
            feed_position: Position3D::new(-117.99, 34.01, 123.6),       // Feed at focal point
            emitter_position: emitter,
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        }
    }

    #[test]
    fn test_is_in_coverage_fully_covered() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, 90.0)
                .frequency_range(8000.0, 9000.0)
                .num_measurements(1000)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_outside_azimuth() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 90.0)
                .elevation_range(0.0, 90.0)
                .frequency_range(8000.0, 9000.0)
                .num_measurements(100)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(!is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_outside_elevation() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, 30.0)
                .frequency_range(8000.0, 9000.0)
                .num_measurements(100)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(!is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_outside_frequency() {
        let coverage = Some(
            CalibrationCoverage::builder()
                .azimuth_range(0.0, 360.0)
                .elevation_range(0.0, 90.0)
                .frequency_range(9000.0, 10000.0)
                .num_measurements(100)
                .has_correction_surface(true)
                .build()
                .unwrap(),
        );

        assert!(!is_in_coverage(&coverage, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_is_in_coverage_none_means_unrestricted() {
        // No coverage restriction recorded (fully calibrated artifact) → always in coverage.
        assert!(is_in_coverage(&None, 180.0, 45.0, 8400.0));
    }

    #[test]
    fn test_generate_warnings_uncalibrated() {
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, false);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("uncalibrated"));
        assert!(warnings[0].contains("±3.0 dB"));
        assert!(warnings[0].contains("±2.0 dB"));
    }

    #[test]
    fn test_generate_warnings_partially_calibrated_in_coverage() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(500)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, true);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("partially calibrated"));
        assert!(warnings[0].contains("±1.5 dB"));
    }

    #[test]
    fn test_generate_warnings_partially_calibrated_out_of_coverage() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 90.0)
            .elevation_range(0.0, 30.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });

        // Add a dummy correction surface to trigger the "not applied" warning
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 360.0],
            knots_elevation: vec![0.0, 90.0],
            knots_frequency: vec![8000.0, 9000.0],
            knots_temperature: vec![290.0],
            spline_order: 3,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, false);

        assert_eq!(warnings.len(), 3);
        assert!(warnings[0].contains("partially calibrated"));
        assert!(warnings[1].contains("outside calibrated region"));
        assert!(warnings[2].contains("Correction surface not applied"));
    }

    #[test]
    fn test_generate_warnings_fully_calibrated() {
        let calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, true);

        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_generate_warnings_correction_not_applied() {
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });

        // Add a dummy correction surface
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 360.0],
            knots_elevation: vec![0.0, 90.0],
            knots_frequency: vec![8000.0, 9000.0],
            knots_temperature: vec![290.0],
            spline_order: 3,
        });

        let warnings = generate_calibration_warnings(&calibration, 180.0, 45.0, false);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Correction surface not applied"));
    }

    /// The correction surface must be evaluated at the calibration's
    /// temperature_const, not a hardcoded 290 K. This artifact is calibrated
    /// at 300 K; with the old hardcoded 290 K the temperature dimension
    /// extrapolated and emitted a warning.
    #[test]
    fn test_correction_uses_calibration_temperature() {
        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.validity_ranges.temperature_const = 300.0;
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![300.0, 300.0, 300.0, 300.0, 300.0, 300.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            !response.warnings.iter().any(|w| w.contains("temperature")),
            "no temperature extrapolation warning expected, got: {:?}",
            response.warnings
        );
        assert!(!response.metadata.extrapolated);
    }

    #[test]
    fn test_compute_gain_uncalibrated_antenna() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have gain computed (physics model)
        assert!(!response.gain_db.is_nan());

        // Should have calibration status
        assert!(response.calibration_status.is_some());
        let status = response.calibration_status.unwrap();
        assert_eq!(status.status, "uncalibrated");
        assert_eq!(status.accuracy_estimate_db, 3.0);
        assert_eq!(status.loss_accuracy_estimate_db, Some(2.0));
        assert!(!status.correction_applied);

        // Should have warning about uncalibrated
        assert!(!response.warnings.is_empty());
        assert!(response.warnings.iter().any(|w| w.contains("uncalibrated")));
    }

    #[test]
    fn test_loss_near_zero_for_boresight_focused_feed() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration(
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            },
        ));
        let mut request = create_test_request();
        // Aim emitter along the boresight direction (on-axis) and feed at boresight (focused):
        request.emitter_position = request.reflector_boresight.clone();
        request.feed_position = request.reflector_boresight.clone();
        request.include_reference = true;
        let response = compute_gain_from_request(&request, &repo).unwrap();
        let loss = response.loss_db.expect("reference requested");
        assert!(
            loss.abs() < 0.6,
            "boresight focused-feed loss should be ~0 dB, got {loss}"
        );
    }

    #[test]
    fn test_compute_gain_uncalibrated_with_reference() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        repo.add_calibration(calibration);

        let mut request = create_test_request();
        request.include_reference = true;

        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have reference gain and loss
        assert!(response.reference_gain_db.is_some());
        assert!(response.loss_db.is_some());

        // Loss should be positive (gain < reference)
        let loss = response.loss_db.unwrap();
        assert!(loss >= 0.0);
    }

    /// Uncalibrated antennas (no correction surface) should have physical
    /// spillover folded in: `apply_spillover` gated on, and the applied loss
    /// (small negative dB, per P1 magnitude finding) surfaced on metadata.
    #[test]
    fn test_spillover_applied_for_uncalibrated_antenna() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.correction_surface.is_none());
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        let spillover = response
            .metadata
            .spillover_loss_db
            .expect("spillover should be applied and reported for an uncalibrated antenna");
        assert!(
            spillover < 0.0,
            "spillover loss must be negative, got {spillover}"
        );
    }

    /// Calibrated antennas (correction surface present) must NOT have physical
    /// spillover folded in — the surface already absorbs it empirically. The
    /// flag must be off, so `spillover_loss_db` is `None`.
    #[test]
    fn test_spillover_not_applied_for_calibrated_antenna() {
        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        // Valid order-3 B-spline knot vectors (>=4 knots each), matching the pattern used by
        // `test_correction_uses_calibration_temperature`, so the surface actually evaluates.
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            response.metadata.spillover_loss_db.is_none(),
            "calibrated antenna must not report spillover_loss_db, got {:?}",
            response.metadata.spillover_loss_db
        );
    }

    /// Reference invariant: the ideal reference computation shares the same
    /// `integration_params` (and thus the same `apply_spillover` gate) as the
    /// actual gain computation, so the *base* spillover applied to both should
    /// cancel out of `loss_db` entirely, leaving only the physics baseline
    /// delta that already exists for the calibrated path (surface-RMS Ruze
    /// loss on the real antenna vs. a perfect ideal reference — see the sibling
    /// `test_loss_near_zero_for_boresight_focused_feed`, tolerance 0.6 dB).
    ///
    /// We assert this directly by comparing the uncalibrated (spillover ON)
    /// loss against the calibrated (spillover OFF) loss for the *same*
    /// physical geometry: if spillover truly cancels, the two losses must be
    /// numerically identical (not just both "small").
    #[test]
    fn test_loss_near_zero_for_boresight_focused_feed_uncalibrated() {
        let mut boresight_request = create_test_request();
        // Aim emitter along the boresight direction (on-axis) and feed at boresight (focused):
        boresight_request.emitter_position = boresight_request.reflector_boresight.clone();
        boresight_request.feed_position = boresight_request.reflector_boresight.clone();
        boresight_request.include_reference = true;

        // Baseline: calibrated (correction surface present) -> apply_spillover is off.
        // `correction_surface` is a distinct field from `calibration_status`, so it must
        // be attached explicitly even for `FullyCalibrated` (see other tests in this module).
        let mut repo_calibrated = CalibrationRepository::new();
        let mut calibration_with_surface =
            create_test_calibration(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            });
        calibration_with_surface.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        assert!(calibration_with_surface.correction_surface.is_some());
        repo_calibrated.add_calibration(calibration_with_surface);
        let response_calibrated =
            compute_gain_from_request(&boresight_request, &repo_calibrated).unwrap();
        assert!(response_calibrated.metadata.spillover_loss_db.is_none());
        let loss_calibrated = response_calibrated
            .loss_db
            .expect("reference requested (calibrated baseline)");

        // Uncalibrated: no correction surface -> apply_spillover is on for both
        // the actual and ideal-reference computations.
        let mut repo_uncalibrated = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.correction_surface.is_none());
        repo_uncalibrated.add_calibration(calibration);
        let response_uncalibrated =
            compute_gain_from_request(&boresight_request, &repo_uncalibrated).unwrap();

        // Confirm spillover was actually applied on this path (otherwise the
        // invariant would hold trivially and prove nothing about cancellation).
        let spillover = response_uncalibrated
            .metadata
            .spillover_loss_db
            .expect("spillover should be applied on the uncalibrated path");
        assert!(spillover < 0.0);

        let loss_uncalibrated = response_uncalibrated
            .loss_db
            .expect("reference requested (uncalibrated)");

        // The base spillover efficiency applies identically to the actual and
        // ideal-reference computations (same q-factor, f/D, and on-axis/zero
        // feed offset in both), so it must cancel exactly out of loss_db.
        assert!(
            (loss_uncalibrated - loss_calibrated).abs() < 1e-6,
            "spillover should cancel out of loss_db entirely: calibrated (no spillover) = {loss_calibrated}, \
             uncalibrated (spillover applied to both actual and reference) = {loss_uncalibrated}"
        );

        // Sanity: still within the same loose bound as the calibrated sibling test.
        assert!(
            loss_uncalibrated.abs() < 0.6,
            "boresight focused-feed loss should be ~0 dB, got {loss_uncalibrated}"
        );
    }

    /// Build a request whose feed is steered far off boresight, so the actual
    /// gain routes to a non-StandardPhysicalOptics mode (large feed offset) and
    /// the model layer applies no spillover. Mirrors the ECEF geometry the
    /// integration tests use for their large-offset cases (feed near the vehicle,
    /// boresight/emitter at a 400 km satellite ~96° away).
    fn create_large_offset_request() -> GainRequest {
        use crate::model::coordinates_3d::geodetic_to_ecef;
        let (veh_x, veh_y, veh_z) = geodetic_to_ecef(-118.1234, 34.5678, 100.0).unwrap();
        let (emit_x, emit_y, emit_z) = geodetic_to_ecef(-117.0, 35.0, 400_000.0).unwrap();
        let (feed_x, feed_y, feed_z) = geodetic_to_ecef(-118.124, 34.568, 105.0).unwrap();

        let ecef = |x: f64, y: f64, z: f64| {
            let mut p = Position3D::new(x, y, z);
            p.coordinate_system = Some(CoordinateSystem::ECEF);
            p
        };

        GainRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: ecef(veh_x, veh_y, veh_z),
            reflector_boresight: ecef(emit_x, emit_y, emit_z),
            feed_position: ecef(feed_x, feed_y, feed_z),
            emitter_position: ecef(emit_x, emit_y, emit_z),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: true,
            vehicle_attitude: None,
        }
    }

    /// Regression test for the reference/actual spillover asymmetry: for an
    /// uncalibrated antenna at a LARGE feed offset, the actual gain routes to a
    /// non-standard-PO mode and gets NO spillover. The ideal reference (always a
    /// focused feed → standard PO) must therefore also skip spillover, tracking
    /// the actual's state — otherwise `loss_db` carries a one-sided spillover bias
    /// while `metadata.spillover_loss_db` reports `None`.
    ///
    /// We prove consistency by comparing `loss_db` against a CALIBRATED antenna
    /// (correction surface → no spillover on either side) for the SAME geometry:
    /// with the fix they must be numerically identical.
    #[test]
    fn test_large_offset_uncalibrated_reference_has_no_spillover_bias() {
        let request = create_large_offset_request();

        // Uncalibrated: apply_spillover flag on, but actual is large-offset → no spillover.
        let mut repo_uncalibrated = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        assert!(calibration.correction_surface.is_none());
        repo_uncalibrated.add_calibration(calibration);
        let response_uncalibrated =
            compute_gain_from_request(&request, &repo_uncalibrated).unwrap();

        // The actual got no spillover (non-standard-PO mode gate in the model layer).
        assert!(
            response_uncalibrated.metadata.spillover_loss_db.is_none(),
            "large-offset actual should not report spillover, got {:?}",
            response_uncalibrated.metadata.spillover_loss_db
        );
        let loss_uncalibrated = response_uncalibrated
            .loss_db
            .expect("reference requested (uncalibrated)");

        // Calibrated baseline: correction surface present → apply_spillover off on both sides.
        let mut repo_calibrated = CalibrationRepository::new();
        let mut calibration_with_surface =
            create_test_calibration(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            });
        calibration_with_surface.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        repo_calibrated.add_calibration(calibration_with_surface);
        let response_calibrated = compute_gain_from_request(&request, &repo_calibrated).unwrap();
        assert!(response_calibrated.metadata.spillover_loss_db.is_none());
        let loss_calibrated = response_calibrated
            .loss_db
            .expect("reference requested (calibrated baseline)");

        // With the reference tracking the actual's (absent) spillover, loss_db must match
        // the no-spillover calibrated baseline exactly — no one-sided bias.
        assert!(
            (loss_uncalibrated - loss_calibrated).abs() < 1e-6,
            "reference spillover must track the actual: uncalibrated large-offset loss = \
             {loss_uncalibrated}, calibrated (no spillover) loss = {loss_calibrated}"
        );
    }

    /// Spillover keys on correction-surface *presence* (whole-antenna gate), NOT
    /// on per-query coverage. An antenna WITH a surface whose coverage excludes the
    /// query (so `correction_applied` is false but `correction_surface.is_some()`)
    /// must still get NO spillover.
    #[test]
    fn test_spillover_not_applied_when_surface_present_but_out_of_coverage() {
        let mut repo = CalibrationRepository::new();

        // Coverage restricted to a narrow region the default request falls outside.
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 5.0)
            .elevation_range(0.0, 5.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();
        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        });
        // Attach a valid surface so `correction_surface.is_some()` even though the query
        // is out of coverage (→ correction not applied).
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 2 * 2 * 2],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 5.0, 5.0, 5.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 5.0, 5.0, 5.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Surface exists but wasn't applied here (out of coverage → extrapolated).
        assert!(response.metadata.extrapolated);
        // Whole-antenna gate: presence of a surface suppresses spillover regardless.
        assert!(
            response.metadata.spillover_loss_db.is_none(),
            "spillover must key on surface presence, not coverage; got {:?}",
            response.metadata.spillover_loss_db
        );
    }

    #[test]
    fn test_compute_gain_fully_calibrated() {
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have gain computed
        assert!(!response.gain_db.is_nan());

        // Should have calibration status
        assert!(response.calibration_status.is_some());
        let status = response.calibration_status.unwrap();
        assert_eq!(status.status, "fully_calibrated");
        assert_eq!(status.accuracy_estimate_db, 1.0);

        // Should NOT have calibration-related warnings (fully calibrated).
        // Integration convergence warnings are acceptable and unrelated to
        // calibration status.
        let calibration_warnings: Vec<_> = response
            .warnings
            .iter()
            .filter(|w| !w.contains("did not converge") && !w.contains("aperture integration"))
            .collect();
        assert!(
            calibration_warnings.is_empty(),
            "Unexpected calibration warnings: {:?}",
            calibration_warnings
        );
    }

    #[test]
    fn test_compute_gain_partially_calibrated_in_coverage() {
        let mut repo = CalibrationRepository::new();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(500)
            .has_correction_surface(false)
            .build()
            .unwrap();

        let calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should have gain computed
        assert!(!response.gain_db.is_nan());

        // Should have calibration status
        assert!(response.calibration_status.is_some());
        let status = response.calibration_status.unwrap();
        assert_eq!(status.status, "partially_calibrated");
        assert_eq!(status.accuracy_estimate_db, 1.5);

        // Should have warning about partial calibration
        assert!(!response.warnings.is_empty());
        assert!(response
            .warnings
            .iter()
            .any(|w| w.contains("partially calibrated")));
    }

    #[test]
    fn test_compute_gain_antenna_not_found() {
        let repo = CalibrationRepository::new();
        let request = create_test_request();
        let result = compute_gain_from_request(&request, &repo);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AntennaModelError::FeedNotFound { .. }
        ));
    }

    #[test]
    fn test_extrapolated_flag_out_of_coverage() {
        // When a correction surface exists but the query is outside coverage,
        // the extrapolated flag should be set to true.
        let mut repo = CalibrationRepository::new();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 10.0) // narrow range
            .elevation_range(0.0, 10.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(100)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let mut calibration = create_test_calibration(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        });
        // Add a correction surface so out-of-coverage detection triggers
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 10.0],
            knots_elevation: vec![0.0, 10.0],
            knots_frequency: vec![8000.0, 9000.0],
            knots_temperature: vec![290.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        // Use the standard test request — emitter direction lands outside the narrow coverage
        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Since the correction surface exists but coverage doesn't include the emitter direction,
        // the extrapolated flag should be set.
        assert!(
            response.metadata.extrapolated,
            "Expected extrapolated=true when correction surface exists but query is out-of-coverage"
        );
    }

    #[test]
    fn test_extrapolated_flag_no_correction_surface() {
        // When there's no correction surface, extrapolated should be false
        let mut repo = CalibrationRepository::new();
        let calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            !response.metadata.extrapolated,
            "Expected extrapolated=false when no correction surface"
        );
    }

    /// Test that `feed_offset_meters` is ~zero when the feed is aimed at the same Earth
    /// target as the reflector boresight (focused/on-axis configuration).
    ///
    /// When `feed_position == reflector_boresight`, `compute_feed_position_from_pointing`
    /// returns (0, 0, focal_length), so the physical offset from the focal point is
    /// (0, 0, focal_length - focal_length) = (0, 0, 0).
    ///
    /// The OLD (angular) code also returned ~(0, 0, 0) for this case because
    /// `feed_az - refl_az ≈ 0` — so this test alone does not discriminate.
    /// See `test_feed_offset_is_meters_not_degrees` for the discriminating case.
    #[test]
    fn test_feed_offset_reported_in_meters_zero_for_boresight() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration(
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            },
        ));
        let mut request = create_test_request();
        // Aim the feed at the same Earth point as the reflector boresight → on-axis feed,
        // so the physical feed offset from the focal point should be ~zero.
        request.feed_position = request.reflector_boresight.clone();
        let response = compute_gain_from_request(&request, &repo).unwrap();
        let off = &response.geometry.feed_offset_meters;
        assert!(
            off.x.abs() < 0.05 && off.y.abs() < 0.05 && off.z.abs() < 0.05,
            "expected ~zero physical offset in meters for boresight-aimed feed, got ({}, {}, {})",
            off.x,
            off.y,
            off.z
        );
    }

    /// Discriminating test: verifies `feed_offset_meters` contains physical meters,
    /// not angular degrees.
    ///
    /// Strategy: call `compute_feed_position_from_pointing` directly to get the
    /// expected physical feed position (x, y, z) in the antenna frame, then assert
    /// that `response.geometry.feed_offset_meters` equals (x, y, z - focal_length_m).
    ///
    /// The default `create_test_request()` has `feed_position` at a different altitude
    /// than `reflector_boresight` (123.6 m vs 110.0 m at the same lon/lat), giving a
    /// non-zero angular offset and therefore a non-zero physical feed displacement.
    ///
    /// The OLD code stored angular degrees (feed_az - refl_az, feed_el - refl_el),
    /// which for this geometry differ from the physical meters values — so this test
    /// WOULD HAVE FAILED against the old implementation.
    #[test]
    fn test_feed_offset_is_meters_not_degrees() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration(
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            },
        ));
        let request = create_test_request();

        // Compute the expected physical feed position directly using the same helper
        // the evaluator uses. focal_length_m = 5.0 (from create_test_calibration).
        let focal_length_m = 5.0_f64;
        let diameter_m = 10.0_f64;
        let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
            &request.feed_position,
            &request.reflector_boresight,
            &request.vehicle_position,
            focal_length_m,
            diameter_m,
            None,
        )
        .expect("compute_feed_position_from_pointing failed in test");
        // Design offset from create_test_calibration is (0, 0, 0), so total = steer
        let expected_x = steer_x;
        let expected_y = steer_y;
        let expected_z_offset = steer_z - focal_length_m;

        let response = compute_gain_from_request(&request, &repo).unwrap();
        let off = &response.geometry.feed_offset_meters;

        assert!(
            (off.x - expected_x).abs() < 1e-9,
            "feed_offset_meters.x should be {expected_x} m (physical), got {}",
            off.x
        );
        assert!(
            (off.y - expected_y).abs() < 1e-9,
            "feed_offset_meters.y should be {expected_y} m (physical), got {}",
            off.y
        );
        assert!(
            (off.z - expected_z_offset).abs() < 1e-9,
            "feed_offset_meters.z should be {expected_z_offset} m (z - focal_length), got {}",
            off.z
        );

        // Also verify the magnitude is physically plausible (sub-meter for the small
        // angular offset in this test geometry, certainly < focal_length = 5 m).
        let mag = (off.x * off.x + off.y * off.y + off.z * off.z).sqrt();
        assert!(
            mag.is_finite() && mag < focal_length_m,
            "feed offset magnitude {mag} m is not physically plausible (should be < focal_length {focal_length_m} m)"
        );
    }

    #[test]
    fn test_backward_compatibility_no_calibration_status() {
        let mut repo = CalibrationRepository::new();

        // Create calibration without calibration_status (old format)
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-01T00:00:00Z")
            .format_version("1.0")
            .data_source("test")
            .rmse_db(0.5)
            .r_squared(0.99)
            .num_measurements(1000)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
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
                    // Feed at focal point - zero offset from optical axis
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
            .build()
            .unwrap();

        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        // Should still compute gain
        assert!(!response.gain_db.is_nan());

        // calibration_status should be None for backward compatibility
        assert!(response.calibration_status.is_none());

        // Should not have calibration warnings (treated as fully calibrated).
        // Integration convergence warnings are acceptable and unrelated to
        // calibration status.
        let calibration_warnings: Vec<_> = response
            .warnings
            .iter()
            .filter(|w| !w.contains("did not converge") && !w.contains("aperture integration"))
            .collect();
        assert!(
            calibration_warnings.is_empty(),
            "Unexpected calibration warnings: {:?}",
            calibration_warnings
        );
    }
}
