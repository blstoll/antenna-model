//! Batch Gain Computation Service
//!
//! This module provides parallel batch processing of multiple gain computation requests.
//! It uses rayon for efficient parallel evaluation and handles partial failures gracefully.

use crate::api::schemas::{
    BatchGainRequest, BatchGainResponse, BatchMetadata, GainRequest, GainResponse,
};
use crate::data::repository::CalibrationRepository;
use crate::error::{AntennaModelError, Result};
use crate::service::evaluator::compute_gain_from_request;
use crate::service::validator::MAX_BATCH_SIZE;
use rayon::prelude::*;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Minimum batch size to benefit from parallel processing
/// Below this threshold, sequential processing may be faster due to overhead
const MIN_PARALLEL_BATCH_SIZE: usize = 5;

/// Evaluate a batch of gain computation requests in parallel.
///
/// This function processes multiple gain requests concurrently using rayon's parallel iterator.
/// It handles partial failures gracefully - individual request failures do not prevent other
/// requests from being processed.
///
/// # Arguments
/// * `request` - Batch request containing multiple gain computation requests
/// * `repository` - Calibration data repository
///
/// # Returns
/// * `Ok(BatchGainResponse)` - Response containing all results (both successes and failures)
/// * `Err(AntennaModelError)` - Only for structural errors (e.g., batch size limit exceeded)
///
/// # Performance
/// - Small batches (<5 requests) are processed sequentially to avoid parallelization overhead
/// - Large batches use rayon's work-stealing thread pool for optimal CPU utilization
/// - Target: 100 evaluations in <500ms on typical hardware
///
/// # Example
/// ```no_run
/// use antenna_model::service::batch::evaluate_batch;
/// use antenna_model::api::schemas::{BatchGainRequest, GainRequest, Position3D};
/// use antenna_model::data::repository::CalibrationRepository;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let repository = CalibrationRepository::new();
/// let request = BatchGainRequest {
///     evaluations: vec![
///         // ... gain requests ...
///     ],
/// };
/// let response = evaluate_batch(&request, &repository)?;
/// println!("Processed {} evaluations in {:.2}ms",
///     response.metadata.count,
///     response.metadata.total_computation_time_ms);
/// # Ok(())
/// # }
/// ```
pub fn evaluate_batch(
    request: &BatchGainRequest,
    repository: &CalibrationRepository,
) -> Result<BatchGainResponse> {
    let start = Instant::now();
    let num_evaluations = request.evaluations.len();

    // Validate batch size
    if num_evaluations == 0 {
        warn!("Empty batch request received");
        return Ok(BatchGainResponse {
            results: Vec::new(),
            metadata: BatchMetadata {
                total_computation_time_ms: start.elapsed().as_secs_f64() * 1000.0,
                count: 0,
                failure_count: 0,
            },
        });
    }

    if num_evaluations > MAX_BATCH_SIZE {
        warn!(
            "Batch size {} exceeds maximum allowed size {}",
            num_evaluations, MAX_BATCH_SIZE
        );
        return Err(AntennaModelError::Validation(
            crate::error::ValidationError::BatchSizeLimitExceeded {
                size: num_evaluations,
                limit: MAX_BATCH_SIZE,
            },
        ));
    }

    info!(
        "Processing batch of {} evaluations (parallel={})",
        num_evaluations,
        num_evaluations >= MIN_PARALLEL_BATCH_SIZE
    );

    // Process requests - use parallel processing for larger batches
    let results: Vec<GainResponse> = if num_evaluations >= MIN_PARALLEL_BATCH_SIZE {
        // Parallel processing using rayon
        debug!("Using parallel processing for batch of {}", num_evaluations);
        request
            .evaluations
            .par_iter()
            .enumerate()
            .map(|(idx, gain_request)| {
                match compute_gain_from_request(gain_request, repository) {
                    Ok(response) => response,
                    Err(e) => {
                        // Log the error but continue processing other requests
                        warn!(
                            "Evaluation {} failed in batch: antenna_id={}, feed_id={}, error={}",
                            idx, gain_request.antenna_id, gain_request.feed_id, e
                        );
                        // Return an error response with appropriate status
                        create_error_response(gain_request, e)
                    }
                }
            })
            .collect()
    } else {
        // Sequential processing for small batches
        debug!(
            "Using sequential processing for small batch of {}",
            num_evaluations
        );
        request
            .evaluations
            .iter()
            .enumerate()
            .map(
                |(idx, gain_request)| match compute_gain_from_request(gain_request, repository) {
                    Ok(response) => response,
                    Err(e) => {
                        warn!(
                            "Evaluation {} failed in batch: antenna_id={}, feed_id={}, error={}",
                            idx, gain_request.antenna_id, gain_request.feed_id, e
                        );
                        create_error_response(gain_request, e)
                    }
                },
            )
            .collect()
    };

    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    // Count true failures (NaN gain) and results with warnings for logging
    let failure_count = results.iter().filter(|r| r.gain_db.is_nan()).count();
    let success_count = num_evaluations - failure_count;
    let warning_count = results
        .iter()
        .filter(|r| !r.gain_db.is_nan() && !r.warnings.is_empty())
        .count();

    info!(
        "Batch processing complete: {} total, {} success, {} failures, {} with warnings, {:.2}ms total, {:.2}ms/evaluation avg",
        num_evaluations,
        success_count,
        failure_count,
        warning_count,
        elapsed,
        elapsed / num_evaluations as f64
    );

    Ok(BatchGainResponse {
        results,
        metadata: BatchMetadata {
            total_computation_time_ms: elapsed,
            count: num_evaluations,
            failure_count,
        },
    })
}

/// Create an error response for a failed gain request.
///
/// This helper function converts an error into a GainResponse with error information
/// in the warnings field, allowing batch processing to continue even when individual
/// requests fail.
fn create_error_response(request: &GainRequest, error: AntennaModelError) -> GainResponse {
    use crate::api::schemas::{ComputationMetadata, GeometryInfo, Vector3D};

    // Create a response with NaN gain to indicate error
    // Include error message in warnings
    GainResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        gain_db: f64::NAN,
        reference_gain_db: None,
        loss_db: None,
        geometry: GeometryInfo {
            feed_offset_meters: Vector3D::new(0.0, 0.0, 0.0),
            emitter_azimuth_deg: 0.0,
            emitter_elevation_deg: 0.0,
            beam_squint_deg: None,
        },
        warnings: vec![format!("Computation failed: {}", error)],
        metadata: ComputationMetadata {
            computation_time_ms: 0.0,
            coordinate_transform_ms: None,
            physics_model_ms: None,
            correction_surface_ms: None,
            extrapolated: false,
            spillover_loss_db: None,
        },
        calibration_status: None, // Will be populated in Task 6.8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schemas::{CoordinateSystem, Position3D};
    use crate::data::types::{
        AntennaCalibration, CalibrationMetadata, FeedParameters, MeshParameters,
        PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
    };

    fn create_test_repository() -> CalibrationRepository {
        let mut repo = CalibrationRepository::new();

        // Create a simple test calibration
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
                    // Feed at focal point (0, 0, focal_length) in reflector frame
                    position: (0.0, 0.0, 5.0),
                    q_factor: 8.0,
                    phase_center_offset_m: 0.0,
                    axial_defocus_m: 0.0,
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
        repo
    }

    fn create_test_request(antenna_id: &str, feed_id: &str) -> GainRequest {
        // Use ECEF coordinates for GEO satellite scenario (similar to evaluator tests).
        // Equatorial ground station (6378 km) is below the 6400 km auto-detect threshold, so
        // set explicit ECEF to preserve correct coordinate interpretation.
        let mut emitter = Position3D::new(6_378_137.0, 0.0, 0.0); // Ground station at equator
        emitter.coordinate_system = Some(CoordinateSystem::ECEF);
        GainRequest {
            antenna_id: antenna_id.to_string(),
            feed_id: feed_id.to_string(),
            vehicle_position: Position3D::new(42_164_137.0, 0.0, 0.0), // GEO at (lon=0, lat=0)
            reflector_boresight: Position3D::new(42_164_127.0, 0.0, 0.0), // 10m toward Earth
            feed_position: Position3D::new(42_164_132.0, 0.0, 0.0), // Feed at ~5m from vehicle (near focus)
            emitter_position: emitter,
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        }
    }

    #[test]
    fn test_empty_batch() {
        let repo = create_test_repository();
        let request = BatchGainRequest {
            evaluations: Vec::new(),
        };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.metadata.count, 0);
        assert_eq!(response.metadata.failure_count, 0);
    }

    #[test]
    fn test_single_evaluation_batch() {
        let repo = create_test_repository();
        let request = BatchGainRequest {
            evaluations: vec![create_test_request("test_antenna", "test_feed")],
        };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.metadata.count, 1);

        // Verify the result is valid
        let result = &response.results[0];
        assert_eq!(result.antenna_id, "test_antenna");
        assert_eq!(result.feed_id, "test_feed");
        assert!(!result.gain_db.is_nan());
    }

    #[test]
    fn test_small_batch_sequential() {
        let repo = create_test_repository();
        let request = BatchGainRequest {
            evaluations: vec![
                create_test_request("test_antenna", "test_feed"),
                create_test_request("test_antenna", "test_feed"),
                create_test_request("test_antenna", "test_feed"),
            ],
        };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 3);
        assert_eq!(response.metadata.count, 3);

        // All results should be valid
        for result in &response.results {
            assert!(!result.gain_db.is_nan());
        }
    }

    #[test]
    fn test_large_batch_parallel() {
        let repo = create_test_repository();
        let evaluations: Vec<GainRequest> = (0..20)
            .map(|_| create_test_request("test_antenna", "test_feed"))
            .collect();

        let request = BatchGainRequest { evaluations };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 20);
        assert_eq!(response.metadata.count, 20);

        // All results should be valid
        for result in &response.results {
            assert!(!result.gain_db.is_nan());
        }
    }

    #[test]
    fn test_batch_size_limit_exceeded() {
        let repo = create_test_repository();
        let evaluations: Vec<GainRequest> = (0..1001)
            .map(|_| create_test_request("test_antenna", "test_feed"))
            .collect();

        let request = BatchGainRequest { evaluations };

        let result = evaluate_batch(&request, &repo);
        assert!(result.is_err());
        match result.unwrap_err() {
            AntennaModelError::Validation(_) => {
                // Expected validation error
            }
            e => panic!("Expected Validation error, got: {:?}", e),
        }
    }

    #[test]
    fn test_partial_failures() {
        let repo = create_test_repository();
        let request = BatchGainRequest {
            evaluations: vec![
                create_test_request("test_antenna", "test_feed"), // Valid
                create_test_request("unknown_antenna", "test_feed"), // Invalid antenna
                create_test_request("test_antenna", "test_feed"), // Valid
                create_test_request("test_antenna", "unknown_feed"), // Invalid feed
            ],
        };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 4);
        assert_eq!(response.metadata.count, 4);

        // First and third should succeed
        assert!(!response.results[0].gain_db.is_nan());
        assert!(!response.results[2].gain_db.is_nan());

        // Second and fourth should fail (NaN gain with error in warnings)
        assert!(response.results[1].gain_db.is_nan());
        assert!(!response.results[1].warnings.is_empty());
        assert!(response.results[3].gain_db.is_nan());
        assert!(!response.results[3].warnings.is_empty());

        // failure_count must match the number of NaN results
        let nan_count = response
            .results
            .iter()
            .filter(|r| r.gain_db.is_nan())
            .count();
        assert_eq!(response.metadata.failure_count, nan_count);
        assert_eq!(response.metadata.failure_count, 2);
    }

    #[test]
    fn test_batch_performance_timing() {
        let repo = create_test_repository();
        let evaluations: Vec<GainRequest> = (0..10)
            .map(|_| create_test_request("test_antenna", "test_feed"))
            .collect();

        let request = BatchGainRequest { evaluations };

        let start = Instant::now();
        let response = evaluate_batch(&request, &repo).unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        assert_eq!(response.results.len(), 10);

        // Verify timing is reasonable and matches reported time
        assert!(response.metadata.total_computation_time_ms > 0.0);
        assert!(response.metadata.total_computation_time_ms <= elapsed + 10.0); // Allow 10ms margin
    }

    #[test]
    fn test_all_failures() {
        let repo = create_test_repository();
        let request = BatchGainRequest {
            evaluations: vec![
                create_test_request("unknown_antenna", "test_feed"),
                create_test_request("unknown_antenna", "test_feed"),
                create_test_request("unknown_antenna", "test_feed"),
            ],
        };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 3);

        // All should fail
        for result in &response.results {
            assert!(result.gain_db.is_nan());
            assert!(!result.warnings.is_empty());
        }
    }

    #[test]
    fn test_batch_with_reference_gain() {
        let repo = create_test_repository();
        let mut req = create_test_request("test_antenna", "test_feed");
        req.include_reference = true;

        let request = BatchGainRequest {
            evaluations: vec![req],
        };

        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 1);

        let result = &response.results[0];
        assert!(!result.gain_db.is_nan());
        assert!(result.reference_gain_db.is_some());
        assert!(result.loss_db.is_some());
    }

    #[test]
    fn test_create_error_response() {
        let request = create_test_request("test_antenna", "test_feed");
        let error = AntennaModelError::FeedNotFound {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
        };

        let response = create_error_response(&request, error);

        assert_eq!(response.antenna_id, "test_antenna");
        assert_eq!(response.feed_id, "test_feed");
        assert!(response.gain_db.is_nan());
        assert!(!response.warnings.is_empty());
        assert!(response.warnings[0].contains("Computation failed"));
    }

    #[test]
    fn test_parallel_vs_sequential_threshold() {
        let repo = create_test_repository();

        // Test just below threshold (should be sequential)
        let evaluations: Vec<GainRequest> = (0..MIN_PARALLEL_BATCH_SIZE - 1)
            .map(|_| create_test_request("test_antenna", "test_feed"))
            .collect();
        let request = BatchGainRequest { evaluations };
        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), MIN_PARALLEL_BATCH_SIZE - 1);

        // Test at threshold (should be parallel)
        let evaluations: Vec<GainRequest> = (0..MIN_PARALLEL_BATCH_SIZE)
            .map(|_| create_test_request("test_antenna", "test_feed"))
            .collect();
        let request = BatchGainRequest { evaluations };
        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), MIN_PARALLEL_BATCH_SIZE);
    }

    #[test]
    fn test_batch_metadata_accuracy() {
        let repo = create_test_repository();
        let num_evals = 15;
        let evaluations: Vec<GainRequest> = (0..num_evals)
            .map(|_| create_test_request("test_antenna", "test_feed"))
            .collect();

        let request = BatchGainRequest { evaluations };
        let response = evaluate_batch(&request, &repo).unwrap();

        assert_eq!(response.metadata.count, num_evals);
        assert!(response.metadata.total_computation_time_ms > 0.0);

        // Average time per evaluation should be reasonable (not zero, not huge)
        let avg_time = response.metadata.total_computation_time_ms / num_evals as f64;
        assert!(avg_time > 0.0);
        assert!(avg_time < 1000.0); // Should be well under 1 second per evaluation
    }

    /// Build a repository holding a single UNCALIBRATED antenna (no correction
    /// surface ⇒ `physics_is_uncorrected()` ⇒ F7 floor ON) with a nonzero surface
    /// RMS so the statistical floor is a clearly nonzero pedestal.
    fn create_uncalibrated_repository() -> CalibrationRepository {
        use crate::data::types::CalibrationStatus;
        let mut repo = CalibrationRepository::new();
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
        let calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(PhysicalAntennaConfig {
                reflector: ReflectorGeometry {
                    diameter_m: 10.0,
                    focal_length_m: 5.0,
                    f_over_d_ratio: 0.5,
                    surface_rms_mm: 1.5,
                },
                feed: FeedParameters {
                    position: (0.0, 0.0, 5.0),
                    q_factor: 8.0,
                    phase_center_offset_m: 0.0,
                    axial_defocus_m: 0.0,
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
            .calibration_status(CalibrationStatus::Uncalibrated {
                accuracy_estimate_db: 3.0,
                loss_accuracy_estimate_db: 2.0,
            })
            .build()
            .unwrap();
        assert!(calibration.correction_surface.is_none());
        repo.add_calibration(calibration);
        repo
    }

    /// A batch request whose single item points BEHIND the dish (emitter dropped to
    /// ground level at the deep-off-axis lon/lat → >90 deg off boresight, REAR
    /// hemisphere). Mirrors the evaluator `create_rear_hemisphere_request` geometry.
    fn rear_hemisphere_request() -> GainRequest {
        use crate::model::coordinates_3d::geodetic_to_ecef;
        let ecef = |lon: f64, lat: f64, alt: f64| {
            let (x, y, z) = geodetic_to_ecef(lon, lat, alt).unwrap();
            let mut p = Position3D::new(x, y, z);
            p.coordinate_system = Some(CoordinateSystem::ECEF);
            p
        };
        GainRequest {
            antenna_id: "test_antenna".to_string(),
            feed_id: "test_feed".to_string(),
            vehicle_position: ecef(-118.1234, 34.5678, 100.0),
            reflector_boresight: ecef(-117.0, 35.0, 400_000.0),
            // Feed aimed at boresight target → feed at focus → StandardPhysicalOptics.
            feed_position: ecef(-117.0, 35.0, 400_000.0),
            emitter_position: ecef(-120.0, 30.0, 0.0),
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        }
    }

    /// The batch path inherits the F7 floor from the evaluator: a rear-hemisphere
    /// item on an uncalibrated antenna returns EXACTLY the statistical floor
    /// (floor-only behind the dish). Pins that batch delegates the floor gate.
    #[test]
    fn test_batch_rear_hemisphere_uncalibrated_returns_statistical_floor() {
        use crate::model::pattern::sidelobe_floor_gain;
        use crate::model::wavelength_from_frequency;
        use crate::model::{
            AntennaConfiguration, FeedParameters as ModelFeedParams, FeedPosition,
            MeshParameters as ModelMeshParams, ReflectorGeometry as ModelReflector,
        };

        let repo = create_uncalibrated_repository();
        let request = BatchGainRequest {
            evaluations: vec![rear_hemisphere_request()],
        };
        let response = evaluate_batch(&request, &repo).unwrap();
        assert_eq!(response.results.len(), 1);
        let result = &response.results[0];

        // Frame-safety precondition: this MUST be a rear-hemisphere query, else the
        // exact-floor pin below would silently be testing the wrong seam.
        assert!(
            result.geometry.emitter_elevation_deg.abs() > 90.0,
            "rear-hemisphere premise broken: expected >90 deg off boresight, got el={}",
            result.geometry.emitter_elevation_deg
        );

        // Independent floor pedestal (surface 1.5 mm, mesh 5 mm / 0.5 mm — matching
        // the repo antenna; the floor is diameter-independent).
        let reflector = ModelReflector::new(10.0, 5.0, 0.0015).unwrap();
        let feed = ModelFeedParams::new(FeedPosition::at_focus(5.0), 8.0, 0.0, 1.0).unwrap();
        let mesh = ModelMeshParams::builder()
            .spacing(0.005)
            .wire_diameter(0.0005)
            .build()
            .unwrap();
        let floor_cfg = AntennaConfiguration::new(
            "floor_check".into(),
            "floor_check".into(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap();
        let wavelength = wavelength_from_frequency(8400.0 * 1e6);
        let floor_db = 10.0 * sidelobe_floor_gain(&floor_cfg, wavelength).log10();
        assert!(
            floor_db > -20.0,
            "floor should be a meaningful pedestal, got {floor_db} dB"
        );
        assert!(
            (result.gain_db - floor_db).abs() < 1e-6,
            "batch rear-hemisphere uncalibrated item {} dBi must be exactly the statistical \
             floor {floor_db} dBi (F7 floor ON, floor-only behind the dish)",
            result.gain_db
        );
    }
}
