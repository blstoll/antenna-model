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
    apply_beam_squint_correction, compute_emitter_direction, compute_feed_position_from_pointing,
    compute_gain_db, evaluate_correction, overall_efficiency, theoretical_max_gain,
    wavelength_from_frequency, AntennaConfiguration, IntegrationParams,
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

    // Compute feed offset for reporting
    // Note: This represents the angular offset converted to physical displacement,
    // not the distance between Earth positions
    let (feed_az, feed_el) = compute_emitter_direction(
        &request.feed_position,
        &request.vehicle_position,
        &request.reflector_boresight,
    )?;
    let (refl_az, refl_el) = compute_emitter_direction(
        &request.reflector_boresight,
        &request.vehicle_position,
        &request.reflector_boresight,
    )?;

    // The feed offset is the angular separation from boresight
    let feed_offset_az = feed_az - refl_az;
    let feed_offset_el = feed_el - refl_el;

    // For reporting purposes, convert to a simple vector representation
    // This is an approximate Cartesian representation of the angular offset
    let feed_offset = crate::api::schemas::Vector3D::new(
        feed_offset_az,
        feed_offset_el,
        (feed_offset_az * feed_offset_az + feed_offset_el * feed_offset_el).sqrt(),
    );

    let (emitter_az, emitter_el) = compute_emitter_direction(
        &request.emitter_position,
        &request.vehicle_position,
        &request.reflector_boresight,
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
    )?;

    // Combine steering-induced position with design feed offset
    // The design position represents the physical offset of this feed from the optical axis
    // (e.g., multi-feed antennas have feeds at different physical locations)
    let design_pos = &calibration.physical_config.feed.position;
    let feed_x = steer_x + design_pos.0;
    let feed_y = steer_y + design_pos.1;
    let feed_z = steer_z + design_pos.2;
    let feed_position = FeedPosition::new(feed_x, feed_y, feed_z);

    // Calculate radial feed displacement for beam squint calculation
    let feed_displacement_m = (feed_x.powi(2) + feed_y.powi(2)).sqrt();

    // Apply beam squint correction if pointing frequency differs from operating frequency
    // Must be done AFTER computing feed position since squint depends on actual displacement
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);

    let (corrected_az, corrected_el, squint_magnitude_deg) =
        if (pointing_freq - request.frequency_mhz).abs() > 0.1 {
            apply_beam_squint_correction(
                emitter_az,
                emitter_el,
                pointing_freq,
                request.frequency_mhz,
                feed_displacement_m,
                focal_length_m,
            )
        } else {
            (emitter_az, emitter_el, 0.0)
        };

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
    let integration_params = IntegrationParams::fast();

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
                290.0,
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

    // Compute reference gain if requested
    let (reference_gain_db, loss_db) = if request.include_reference {
        // Reference gain is theoretical maximum with efficiency factors (no pointing loss)
        let wavelength_m = wavelength_from_frequency(frequency_hz);
        let aperture_efficiency = overall_efficiency(&antenna_config, wavelength_m);
        let theoretical_gain_linear =
            theoretical_max_gain(diameter_m, wavelength_m, aperture_efficiency);
        let reference_db = 10.0 * theoretical_gain_linear.log10();

        // Loss is difference between reference and actual gain (without correction surface)
        (Some(reference_db), Some(reference_db - final_gain_db))
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
        },
        calibration_status: calibration_status_info,
    })
}

/// Check if the query is within the calibrated coverage region.
///
/// For fully calibrated antennas (no coverage specified), always returns true.
/// For partially calibrated antennas, checks if azimuth, elevation, and frequency
/// are within the specified coverage ranges.
/// For uncalibrated antennas (no coverage), returns false.
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
        None => false,
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
    fn test_is_in_coverage_none() {
        assert!(!is_in_coverage(&None, 180.0, 45.0, 8400.0));
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

        // Should NOT have warnings (fully calibrated)
        assert!(response.warnings.is_empty());
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

        // Should not have calibration warnings (treated as fully calibrated)
        assert!(response.warnings.is_empty());
    }
}
