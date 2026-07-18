//! API request handlers
//!
//! This module implements HTTP request handlers for all API endpoints.

use crate::api::schemas::{
    BatchGainRequest, BatchGainResponse, CalibrationStatusInfo, ErrorResponse, GainRequest,
    GainResponse, H3LinkBudgetRequest, H3LinkBudgetResponse, HealthResponse, HeatmapRequest,
    HeatmapResponse, StatusResponse,
};
use crate::api::AppState;
use crate::service::{
    compute_gain_from_request, compute_h3_link_budget, evaluate_batch, generate_heatmap, validator,
};
use poem::{
    handler,
    http::StatusCode,
    web::{Data, Json},
    Response,
};
use std::sync::Arc;
use tracing::{error, info, warn};

/// GET /health - Liveness probe endpoint
///
/// Returns the current health status of the service.
/// This endpoint always returns 200 OK if the server is responsive,
/// indicating that the service is alive (not deadlocked or crashed).
///
/// For Kubernetes liveness probes - the service is alive if it can respond.
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - status: "healthy" when service is responsive
///
/// # Example Response
/// ```json
/// {
///   "status": "healthy"
/// }
/// ```
#[handler]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse::healthy())
}

/// GET /ready - Readiness probe endpoint
///
/// Returns the current readiness status of the service.
/// This endpoint returns 200 OK when the service is ready to accept requests,
/// or 503 Service Unavailable during startup or if initialization fails.
///
/// For Kubernetes readiness probes - the service is ready if:
/// - Calibration data is loaded (when available)
/// - All initialization is complete
///
/// # Response
/// Returns HTTP 200 when ready, 503 when not ready
///
/// # Example Response (Ready)
/// ```json
/// {
///   "status": "ready"
/// }
/// ```
///
/// # Example Response (Not Ready)
/// ```json
/// {
///   "status": "not_ready"
/// }
/// ```
#[handler]
pub async fn ready(state: Data<&Arc<AppState>>) -> Response {
    let is_ready = state.is_ready();

    if is_ready {
        Response::builder()
            .status(StatusCode::OK)
            .body(serde_json::json!({"status": "ready"}).to_string())
    } else {
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(serde_json::json!({"status": "not_ready"}).to_string())
    }
}

/// GET /status - Service status endpoint
///
/// Returns the current status of the service including version, uptime,
/// loaded antennas, and memory usage.
/// This endpoint provides detailed operational information for monitoring.
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - status: "ok" when service is operational
/// - version: Application version from Cargo.toml
/// - uptime_seconds: Seconds since server started
/// - antenna_count: Number of loaded antennas (when available)
/// - antenna_ids: List of loaded antenna IDs (when available)
/// - memory_bytes: Memory usage in bytes (when available, Linux only)
///
/// # Example Response
/// ```json
/// {
///   "status": "ok",
///   "version": "0.1.0",
///   "uptime_seconds": 3600,
///   "antenna_count": 2,
///   "antenna_ids": ["antenna_1", "antenna_2"],
///   "memory_bytes": 134217728
/// }
/// ```
#[handler]
pub async fn status(state: Data<&Arc<AppState>>) -> Json<StatusResponse> {
    let uptime = state.uptime_seconds();
    let version = state.version.to_string();
    let antenna_ids = state.get_antenna_ids();
    let memory_bytes = state.get_memory_usage();

    info!(
        version = version,
        uptime_seconds = uptime,
        antenna_count = antenna_ids.len(),
        memory_bytes = ?memory_bytes,
        "Status endpoint called"
    );

    let mut response = StatusResponse::ok(version, uptime);

    // Add antenna information if any antennas are loaded
    if !antenna_ids.is_empty() {
        response = response.with_antennas(antenna_ids);
    }

    // Add memory usage if available
    if let Some(mem) = memory_bytes {
        response = response.with_memory(mem);
    }

    Json(response)
}

/// POST /api/v1/gain - Compute antenna gain
///
/// Computes antenna gain based on 3D positions.
/// This is the main computation endpoint combining coordinate transformations,
/// physics-based modeling, and correction surface interpolation.
///
/// # Request Body
/// JSON object containing:
/// - antenna_id: Antenna identifier
/// - feed_id: Feed identifier (for multi-feed antennas)
/// - vehicle_position: Vehicle position (ECEF or Geodetic, auto-detected)
/// - reflector_boresight: Reflector boresight position (ECEF or Geodetic)
/// - feed_position: Feed position (ECEF or Geodetic)
/// - emitter_position: Emitter position (ECEF or Geodetic)
/// - frequency_mhz: Operating frequency in MHz
/// - include_reference: Whether to include reference gain in response
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - gain_db: Computed gain in dB
/// - geometry: Geometric information (feed offset, emitter direction)
/// - warnings: Any warnings generated during computation
/// - metadata: Computation timing metadata
///
/// Returns HTTP 400 for invalid input or HTTP 404 if antenna/feed not found
///
/// # Example Request
/// ```json
/// {
///   "antenna_id": "antenna_1",
///   "feed_id": "feed_1",
///   "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "reflector_boresight": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "feed_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "emitter_position": {"x": 42164000.0, "y": 0.0, "z": 0.0},
///   "frequency_mhz": 11450.0,
///   "include_reference": true
/// }
/// ```
#[handler]
pub async fn compute_gain(
    state: Data<&Arc<AppState>>,
    Json(request): Json<GainRequest>,
) -> poem::Result<Json<GainResponse>> {
    info!(
        antenna_id = %request.antenna_id,
        feed_id = %request.feed_id,
        frequency_mhz = request.frequency_mhz,
        "Gain computation request received"
    );

    // Validate the request
    if let Err(validation_err) = validator::validate_gain_request(&request, &state.repository) {
        warn!(
            antenna_id = %request.antenna_id,
            feed_id = %request.feed_id,
            error = %validation_err,
            "Request validation failed"
        );
        let error_response = ErrorResponse::new("validation_error", validation_err.to_string());
        return Err(poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    // Compute gain using the service layer
    match compute_gain_from_request(&request, &state.repository) {
        Ok(response) => {
            info!(
                antenna_id = %request.antenna_id,
                feed_id = %request.feed_id,
                gain_db = response.gain_db,
                computation_time_ms = response.metadata.computation_time_ms,
                warnings_count = response.warnings.len(),
                "Gain computation successful"
            );
            Ok(Json(response))
        }
        Err(e) => {
            error!(
                antenna_id = %request.antenna_id,
                feed_id = %request.feed_id,
                error = %e,
                "Gain computation failed"
            );

            // Map errors to appropriate HTTP status codes and create error response
            let (status_code, error_type) = match &e {
                crate::error::AntennaModelError::FeedNotFound { .. } => {
                    (StatusCode::NOT_FOUND, "feed_not_found")
                }
                crate::error::AntennaModelError::InvalidCoordinate { .. } => {
                    (StatusCode::BAD_REQUEST, "invalid_coordinate")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };

            let error_response = ErrorResponse::new(error_type, e.to_string());

            Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                status_code,
            ))
        }
    }
}

/// POST /api/v1/gain/batch - Batch gain computation
///
/// Processes multiple gain computation requests in parallel for improved throughput.
/// This endpoint is optimized for analytical workloads that need to evaluate many
/// configurations efficiently.
///
/// # Request Body
/// JSON object containing:
/// - evaluations: Array of GainRequest objects (max 1000)
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - results: Array of GainResponse objects (one per request)
/// - metadata: Aggregate metadata (total time, count)
///
/// Returns HTTP 400 if batch size exceeds limit
///
/// # Error Handling
/// Individual request failures do not prevent other requests from being processed.
/// Failed requests will have NaN gain_db and error message in warnings field.
///
/// # Performance
/// - Small batches (<5 requests): Processed sequentially
/// - Large batches (≥5 requests): Processed in parallel using rayon
/// - Target: 100 evaluations in <500ms
///
/// # Example Request
/// ```json
/// {
///   "evaluations": [
///     {
///       "antenna_id": "antenna_1",
///       "feed_id": "feed_1",
///       "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///       "reflector_boresight": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///       "feed_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///       "emitter_position": {"x": 42164000.0, "y": 0.0, "z": 0.0},
///       "frequency_mhz": 11450.0,
///       "include_reference": false
///     },
///     // ... more requests ...
///   ]
/// }
/// ```
#[handler]
pub async fn compute_gain_batch(
    state: Data<&Arc<AppState>>,
    Json(request): Json<BatchGainRequest>,
) -> poem::Result<Json<BatchGainResponse>> {
    let num_evaluations = request.evaluations.len();

    info!(
        num_evaluations = num_evaluations,
        "Batch gain computation request received"
    );

    // Note: Individual request validation is performed within evaluate_batch
    // Batch-level validation (size limit) is also handled there

    // Process the batch using the service layer. The service runs rayon
    // synchronously (CPU-bound), which would otherwise block the async worker
    // thread and defeat the RequestTimeout middleware. Offload it to the blocking
    // pool so the async task yields at the join `.await`, letting the timeout
    // fire. (The rayon work is not cancelled on timeout — see RequestTimeout.)
    let state = state.0.clone();
    let result = tokio::task::spawn_blocking(move || evaluate_batch(&request, &state.repository))
        .await
        .map_err(|join_err| {
            error!(error = %join_err, "Batch compute task failed to join");
            let error_response = ErrorResponse::new(
                "internal_error",
                format!("Batch computation task failed: {join_err}"),
            );
            poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

    match result {
        Ok(response) => {
            let success_count = response
                .results
                .iter()
                .filter(|r| !r.gain_db.is_nan())
                .count();
            let failure_count = num_evaluations - success_count;

            info!(
                num_evaluations = num_evaluations,
                success_count = success_count,
                failure_count = failure_count,
                total_time_ms = response.metadata.total_computation_time_ms,
                avg_time_ms = response.metadata.total_computation_time_ms / num_evaluations as f64,
                "Batch gain computation completed"
            );

            Ok(Json(response))
        }
        Err(e) => {
            error!(
                num_evaluations = num_evaluations,
                error = %e,
                "Batch gain computation failed"
            );

            // Map errors to appropriate HTTP status codes
            let (status_code, error_type) = match &e {
                crate::error::AntennaModelError::Validation(_) => {
                    (StatusCode::BAD_REQUEST, "validation_error")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };

            let error_response = ErrorResponse::new(error_type, e.to_string());

            Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                status_code,
            ))
        }
    }
}

/// POST /api/v1/heatmap - Generate loss heatmap
///
/// Generates a 2D loss heatmap across the antenna field of view by evaluating
/// gain at a grid of emitter positions. Loss is computed relative to peak gain.
///
/// # Request Body
/// JSON object containing:
/// - antenna_id: Antenna identifier
/// - feed_id: Feed identifier
/// - vehicle_position: 3D position (ECEF or Geodetic)
/// - reflector_boresight: 3D position (ECEF or Geodetic)
/// - feed_position: 3D position (ECEF or Geodetic)
/// - frequency_mhz: Operating frequency
/// - pointing_frequency_mhz: Optional pointing frequency for beam squint
/// - grid_config: Grid configuration (rectangular or H3)
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - antenna_id: Antenna identifier
/// - feed_id: Feed identifier
/// - frequency_mhz: Operating frequency
/// - grid: Grid data (azimuth/elevation values and loss matrix)
/// - warnings: List of warnings (e.g., extrapolation)
/// - metadata: Computation metadata (points evaluated, time, peak gain)
///
/// Returns HTTP 400 for invalid requests
/// Returns HTTP 404 if antenna or feed not found
/// Returns HTTP 422 if grid configuration is invalid or feature not implemented
///
/// # Performance
/// - 72x46 rectangular grid (~3312 points): Target <2 seconds
/// - Grid points evaluated in parallel using rayon for large grids
///
/// # Example Request (Rectangular Grid)
/// ```json
/// {
///   "antenna_id": "antenna_1",
///   "feed_id": "x_band_feed",
///   "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "reflector_boresight": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "feed_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "frequency_mhz": 8400.0,
///   "grid_config": {
///     "grid_type": "rectangular",
///     "azimuth_range_deg": {"min": 0.0, "max": 360.0, "step": 5.0},
///     "elevation_range_deg": {"min": 0.0, "max": 90.0, "step": 2.0}
///   }
/// }
/// ```
#[handler]
pub async fn generate_heatmap_endpoint(
    state: Data<&Arc<AppState>>,
    Json(request): Json<HeatmapRequest>,
) -> poem::Result<Json<HeatmapResponse>> {
    info!(
        antenna_id = %request.antenna_id,
        feed_id = %request.feed_id,
        frequency_mhz = request.frequency_mhz,
        "Heatmap generation request received"
    );

    // Validate the request
    if let Err(validation_err) = validator::validate_heatmap_request(&request, &state.repository) {
        warn!(
            antenna_id = %request.antenna_id,
            feed_id = %request.feed_id,
            error = %validation_err,
            "Heatmap request validation failed"
        );
        let error_response = ErrorResponse::new("validation_error", validation_err.to_string());
        return Err(poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    // Generate heatmap using the service layer. The service runs rayon
    // synchronously (CPU-bound); offload it to the blocking pool so the async
    // task yields at the join `.await` and the RequestTimeout middleware can
    // fire. (The rayon work is not cancelled on timeout — see RequestTimeout.)
    // Pre-extract only the two small fields post-compute logging needs, then
    // MOVE `request` into the closure — avoids deep-cloning the whole
    // HeatmapRequest (grid config + three 3D positions) on every heavy call.
    let compute_state = state.0.clone();
    let antenna_id = request.antenna_id.clone();
    let feed_id = request.feed_id.clone();
    let result =
        tokio::task::spawn_blocking(move || generate_heatmap(&request, &compute_state.repository))
            .await
            .map_err(|join_err| {
                error!(error = %join_err, "Heatmap compute task failed to join");
                let error_response = ErrorResponse::new(
                    "internal_error",
                    format!("Heatmap computation task failed: {join_err}"),
                );
                poem::Error::from_string(
                    serde_json::to_string(&error_response).unwrap_or_default(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?;

    match result {
        Ok(response) => {
            info!(
                antenna_id = %antenna_id,
                feed_id = %feed_id,
                points_evaluated = response.metadata.points_evaluated,
                computation_time_ms = response.metadata.computation_time_ms,
                peak_gain_db = response.metadata.peak_gain_db,
                warnings_count = response.warnings.len(),
                "Heatmap generation successful"
            );
            Ok(Json(response))
        }
        Err(e) => {
            error!(
                antenna_id = %antenna_id,
                feed_id = %feed_id,
                error = %e,
                "Heatmap generation failed"
            );

            // Map errors to appropriate HTTP status codes
            let (status_code, error_type) = match &e {
                crate::error::AntennaModelError::FeedNotFound { .. } => {
                    (StatusCode::NOT_FOUND, "feed_not_found")
                }
                crate::error::AntennaModelError::NotImplemented { .. } => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "not_implemented")
                }
                crate::error::AntennaModelError::Validation(_) => {
                    (StatusCode::BAD_REQUEST, "validation_error")
                }
                crate::error::AntennaModelError::InvalidCoordinate { .. } => {
                    (StatusCode::BAD_REQUEST, "invalid_coordinate")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };

            let error_response = ErrorResponse::new(error_type, e.to_string());

            Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                status_code,
            ))
        }
    }
}

/// GET /api/v1/antennas - List all available antennas
///
/// Returns a list of all loaded antennas with basic metadata including available feeds.
/// Results are sorted alphabetically by antenna ID.
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - antennas: Array of antenna information objects
///   - id: Antenna identifier
///   - name: Human-readable antenna name
///   - enabled: Whether antenna is enabled
///   - feed_count: Number of available feeds
///   - feed_ids: List of available feed IDs
///
/// # Example Response
/// ```json
/// {
///   "antennas": [
///     {
///       "id": "antenna_1",
///       "name": "Deep Space Network 34m",
///       "enabled": true,
///       "feed_count": 2,
///       "feed_ids": ["s_band", "x_band"]
///     }
///   ]
/// }
/// ```
#[handler]
pub async fn list_antennas(
    state: Data<&Arc<AppState>>,
) -> poem::Result<Json<crate::api::schemas::AntennaListResponse>> {
    info!("Antenna list request received");

    let antenna_ids = state.repository.list_antennas();
    let mut antennas = Vec::new();

    for antenna_id in antenna_ids {
        let feed_ids = state.repository.list_feeds(&antenna_id);

        // Get metadata from first feed (name is antenna-level, not feed-specific)
        if let Some(feed_id) = feed_ids.first() {
            if let Some(calibration) = state.repository.get_calibration(&antenna_id, feed_id) {
                antennas.push(crate::api::schemas::AntennaInfo {
                    id: antenna_id.clone(),
                    name: calibration.metadata.antenna_name,
                    enabled: true, // If loaded, it's enabled
                    feed_count: feed_ids.len(),
                    feed_ids: feed_ids.clone(),
                });
            }
        }
    }

    info!(
        antenna_count = antennas.len(),
        "Antenna list request successful"
    );
    Ok(Json(crate::api::schemas::AntennaListResponse { antennas }))
}

/// GET /api/v1/antennas/{id} - Get detailed antenna information
///
/// Returns comprehensive information about a specific antenna including all feeds,
/// validity ranges, calibration metadata, and physical parameters.
///
/// # Path Parameters
/// - id: Antenna identifier
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - id: Antenna identifier
/// - name: Human-readable antenna name
/// - enabled: Whether antenna is enabled
/// - feeds: Array of feed information
/// - validity_ranges: Valid parameter ranges
/// - calibration: Calibration metadata
/// - physical_parameters: Physical antenna parameters
///
/// Returns HTTP 404 if antenna not found
///
/// # Example Response
/// ```json
/// {
///   "id": "antenna_1",
///   "name": "Deep Space Network 34m",
///   "enabled": true,
///   "feeds": [...],
///   "validity_ranges": {...},
///   "calibration": {...},
///   "physical_parameters": {...}
/// }
/// ```
#[handler]
pub async fn get_antenna_details(
    state: Data<&Arc<AppState>>,
    antenna_id: poem::web::Path<String>,
) -> poem::Result<Json<crate::api::schemas::AntennaDetailsResponse>> {
    let antenna_id = antenna_id.0;
    info!(antenna_id = %antenna_id, "Antenna details request received");

    let feed_ids = state.repository.list_feeds(&antenna_id);

    if feed_ids.is_empty() {
        warn!(antenna_id = %antenna_id, "Antenna not found");
        let error_response = ErrorResponse::new(
            "antenna_not_found",
            format!("Antenna '{}' not found", antenna_id),
        );
        return Err(poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            StatusCode::NOT_FOUND,
        ));
    }

    // Use first feed to get antenna-level information
    let first_feed_id = &feed_ids[0];
    let calibration = state
        .repository
        .get_calibration(&antenna_id, first_feed_id)
        .ok_or_else(|| {
            poem::Error::from_string(
                format!("Antenna {}/{} not found", antenna_id, first_feed_id),
                StatusCode::NOT_FOUND,
            )
        })?;

    // Build feed information for all feeds
    let mut feeds = Vec::new();
    for feed_id in &feed_ids {
        if let Some(cal) = state.repository.get_calibration(&antenna_id, feed_id) {
            feeds.push(crate::api::schemas::FeedInfo {
                id: feed_id.clone(),
                position_offset: crate::api::schemas::Vector3D {
                    x: cal.physical_config.feed.position.0,
                    y: cal.physical_config.feed.position.1,
                    z: cal.physical_config.feed.position.2,
                },
                frequency_range_mhz: cal.validity_ranges.frequency_min_max,
                q_factor: cal.physical_config.feed.q_factor,
            });
        }
    }

    // Build validity ranges from first feed (should be consistent across feeds)
    let validity_ranges = crate::api::schemas::ValidityRangesInfo {
        azimuth_deg: calibration.validity_ranges.azimuth_min_max,
        elevation_deg: calibration.validity_ranges.elevation_min_max,
        frequency_mhz: calibration.validity_ranges.frequency_min_max,
        temperature_k: calibration.validity_ranges.temperature_const,
    };

    // Build calibration info
    let calibration_info = crate::api::schemas::CalibrationInfo {
        date: calibration.metadata.calibration_date.clone(),
        version: calibration.metadata.format_version.clone(),
        source: calibration.metadata.data_source.clone(),
        rmse_db: Some(calibration.metadata.rmse_db),
        r_squared: Some(calibration.metadata.r_squared),
        num_measurements: calibration.metadata.num_measurements,
    };

    // Build physical parameters
    let mesh_info =
        calibration
            .physical_config
            .mesh
            .as_ref()
            .map(|mesh| crate::api::schemas::MeshInfo {
                mesh_spacing_mm: mesh.mesh_spacing_mm,
                wire_diameter_mm: mesh.wire_diameter_mm,
            });

    let physical_parameters = crate::api::schemas::PhysicalParametersInfo {
        diameter_m: calibration.physical_config.reflector.diameter_m,
        focal_length_m: calibration.physical_config.reflector.focal_length_m,
        f_over_d_ratio: calibration.physical_config.reflector.f_over_d_ratio,
        surface_rms_mm: calibration.physical_config.reflector.surface_rms_mm,
        mesh: mesh_info,
    };

    // Build calibration status info
    let calibration_status_info = calibration.calibration_status.as_ref().map(|cal_status| {
        let mut info = CalibrationStatusInfo::from(cal_status);
        // For antenna details, indicate if correction surface is available
        info.correction_applied = calibration.correction_surface.is_some();
        info
    });

    let response = crate::api::schemas::AntennaDetailsResponse {
        id: antenna_id.clone(),
        name: calibration.metadata.antenna_name,
        enabled: true,
        feeds,
        validity_ranges,
        calibration: calibration_info,
        physical_parameters,
        calibration_status: calibration_status_info,
    };

    info!(
        antenna_id = %antenna_id,
        feed_count = response.feeds.len(),
        "Antenna details request successful"
    );
    Ok(Json(response))
}

/// GET /api/v1/antennas/{id}/feeds - List feeds for an antenna
///
/// Returns a list of all feeds available for a specific antenna.
///
/// # Path Parameters
/// - id: Antenna identifier
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - feeds: Array of feed information objects
///
/// Returns HTTP 404 if antenna not found
///
/// # Example Response
/// ```json
/// {
///   "feeds": [
///     {
///       "id": "x_band",
///       "position_offset": {"x": 0.05, "y": 0.02, "z": 0.01},
///       "frequency_range_mhz": [7100.0, 8500.0],
///       "q_factor": 8.0
///     }
///   ]
/// }
/// ```
#[handler]
pub async fn list_antenna_feeds(
    state: Data<&Arc<AppState>>,
    antenna_id: poem::web::Path<String>,
) -> poem::Result<Json<serde_json::Value>> {
    let antenna_id = antenna_id.0;
    info!(antenna_id = %antenna_id, "Antenna feeds list request received");

    let feed_ids = state.repository.list_feeds(&antenna_id);

    if feed_ids.is_empty() {
        warn!(antenna_id = %antenna_id, "Antenna not found");
        let error_response = ErrorResponse::new(
            "antenna_not_found",
            format!("Antenna '{}' not found", antenna_id),
        );
        return Err(poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            StatusCode::NOT_FOUND,
        ));
    }

    // Build feed information
    let mut feeds = Vec::new();
    for feed_id in &feed_ids {
        if let Some(cal) = state.repository.get_calibration(&antenna_id, feed_id) {
            feeds.push(crate::api::schemas::FeedInfo {
                id: feed_id.clone(),
                position_offset: crate::api::schemas::Vector3D {
                    x: cal.physical_config.feed.position.0,
                    y: cal.physical_config.feed.position.1,
                    z: cal.physical_config.feed.position.2,
                },
                frequency_range_mhz: cal.validity_ranges.frequency_min_max,
                q_factor: cal.physical_config.feed.q_factor,
            });
        }
    }

    info!(
        antenna_id = %antenna_id,
        feed_count = feeds.len(),
        "Antenna feeds list request successful"
    );
    Ok(Json(serde_json::json!({ "feeds": feeds })))
}

/// GET /api/v1/antennas/{id}/feeds/{feed_id} - Get feed details
///
/// Returns detailed information about a specific feed including position,
/// pattern parameters, and frequency range.
///
/// # Path Parameters
/// - id: Antenna identifier
/// - feed_id: Feed identifier
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - id: Feed identifier
/// - position_offset: Feed position offset from focal point (meters)
/// - frequency_range_mhz: Valid frequency range [min, max] in MHz
/// - q_factor: Feed pattern q-factor
///
/// Returns HTTP 404 if antenna or feed not found
///
/// # Example Response
/// ```json
/// {
///   "id": "x_band",
///   "position_offset": {"x": 0.05, "y": 0.02, "z": 0.01},
///   "frequency_range_mhz": [7100.0, 8500.0],
///   "q_factor": 8.0
/// }
/// ```
#[handler]
pub async fn get_feed_details(
    state: Data<&Arc<AppState>>,
    path: poem::web::Path<(String, String)>,
) -> poem::Result<Json<crate::api::schemas::FeedInfo>> {
    let (antenna_id, feed_id) = path.0;
    info!(
        antenna_id = %antenna_id,
        feed_id = %feed_id,
        "Feed details request received"
    );

    match state.repository.get_calibration(&antenna_id, &feed_id) {
        Some(cal) => {
            let feed_info = crate::api::schemas::FeedInfo {
                id: feed_id.clone(),
                position_offset: crate::api::schemas::Vector3D {
                    x: cal.physical_config.feed.position.0,
                    y: cal.physical_config.feed.position.1,
                    z: cal.physical_config.feed.position.2,
                },
                frequency_range_mhz: cal.validity_ranges.frequency_min_max,
                q_factor: cal.physical_config.feed.q_factor,
            };

            info!(
                antenna_id = %antenna_id,
                feed_id = %feed_id,
                "Feed details request successful"
            );
            Ok(Json(feed_info))
        }
        None => {
            // Check if antenna exists
            let antenna_exists = !state.repository.list_feeds(&antenna_id).is_empty();

            let (error_type, error_msg) = if antenna_exists {
                (
                    "feed_not_found",
                    format!("Feed '{}' not found for antenna '{}'", feed_id, antenna_id),
                )
            } else {
                (
                    "antenna_not_found",
                    format!("Antenna '{}' not found", antenna_id),
                )
            };

            warn!(
                antenna_id = %antenna_id,
                feed_id = %feed_id,
                error = %error_msg,
                "Feed details request failed"
            );

            let error_response = ErrorResponse::new(error_type, error_msg);
            Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                StatusCode::NOT_FOUND,
            ))
        }
    }
}

/// POST /api/v1/h3-heatmap - Compute H3 hexagonal link budget
///
/// Generates per-cell link budget values across an H3 hexagonal grid centered
/// on the feed pointing location. Each cell includes antenna gain, free-space
/// path loss, total path loss, and optional G/T.
///
/// # Request Body
/// JSON object containing:
/// - antenna_id: Antenna identifier
/// - feed_id: Feed identifier
/// - vehicle_position: 3D position (ECEF or Geodetic)
/// - reflector_boresight: 3D position (ECEF or Geodetic)
/// - feed_position: 3D position (ECEF or Geodetic)
/// - frequency_mhz: Operating frequency in MHz (must be positive)
/// - n_rings: Number of H3 rings around center cell (max 10)
/// - h3_resolution: Optional H3 resolution (0-15); derived from frequency when absent
/// - temperature_k: Optional system noise temperature for G/T computation
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - cells: Per-cell link budget results
/// - metadata: Computation metadata (points evaluated, time, peak gain)
/// - warnings: Any warnings generated during computation
///
/// Returns HTTP 422 for validation errors (e.g., n_rings > 10, invalid positions, out-of-range
/// frequency), HTTP 404 if antenna or feed not found, HTTP 500 for internal errors.
#[handler]
pub async fn h3_link_budget(
    state: Data<&Arc<AppState>>,
    Json(request): Json<H3LinkBudgetRequest>,
) -> poem::Result<Json<H3LinkBudgetResponse>> {
    let start_time = std::time::Instant::now();

    info!(
        antenna_id = %request.antenna_id,
        feed_id = %request.feed_id,
        frequency_mhz = request.frequency_mhz,
        n_rings = request.n_rings,
        "H3 link budget request received"
    );

    // Validate the request
    if let Err(validation_err) = validator::validate_h3_link_budget_request(&request) {
        warn!(
            antenna_id = %request.antenna_id,
            feed_id = %request.feed_id,
            error = %validation_err,
            "H3 link budget request validation failed"
        );
        let error_response = ErrorResponse::new("validation_error", validation_err.to_string());
        return Err(poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ));
    }

    // Look up antenna/feed calibration
    let calibration = match state
        .repository
        .get_calibration(&request.antenna_id, &request.feed_id)
    {
        Some(cal) => cal,
        None => {
            // Distinguish missing antenna from missing feed
            let antenna_exists = !state.repository.list_feeds(&request.antenna_id).is_empty();
            let (error_type, error_msg) = if antenna_exists {
                (
                    "feed_not_found",
                    format!(
                        "Feed '{}' not found for antenna '{}'",
                        request.feed_id, request.antenna_id
                    ),
                )
            } else {
                (
                    "antenna_not_found",
                    format!("Antenna '{}' not found", request.antenna_id),
                )
            };
            warn!(
                antenna_id = %request.antenna_id,
                feed_id = %request.feed_id,
                error = %error_msg,
                "H3 link budget antenna/feed lookup failed"
            );
            let error_response = ErrorResponse::new(error_type, error_msg);
            return Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                StatusCode::NOT_FOUND,
            ));
        }
    };

    // Delegate to service layer. The service runs rayon synchronously
    // (CPU-bound); offload it to the blocking pool so the async task yields at
    // the join `.await` and the RequestTimeout middleware can fire. (The rayon
    // work is not cancelled on timeout — see RequestTimeout.) Pre-extract only
    // the two small fields post-compute logging needs, then MOVE `request` into
    // the closure — avoids deep-cloning the whole H3LinkBudgetRequest; the
    // looked-up `calibration` (owned) and cache `Arc` move in alongside it.
    let compute_cache = state.cache.clone();
    let antenna_id = request.antenna_id.clone();
    let feed_id = request.feed_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        compute_h3_link_budget(&request, &calibration, &compute_cache, start_time)
    })
    .await
    .map_err(|join_err| {
        error!(error = %join_err, "H3 link budget compute task failed to join");
        let error_response = ErrorResponse::new(
            "internal_error",
            format!("H3 link budget computation task failed: {join_err}"),
        );
        poem::Error::from_string(
            serde_json::to_string(&error_response).unwrap_or_default(),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
    })?;

    match result {
        Ok(response) => {
            info!(
                antenna_id = %antenna_id,
                feed_id = %feed_id,
                cells_computed = response.cells.len(),
                computation_time_ms = response.metadata.computation_time_ms,
                peak_gain_db = response.metadata.peak_gain_db,
                warnings_count = response.warnings.len(),
                "H3 link budget computation successful"
            );
            Ok(Json(response))
        }
        Err(e) => {
            error!(
                antenna_id = %antenna_id,
                feed_id = %feed_id,
                error = %e,
                "H3 link budget computation failed"
            );

            let (status_code, error_type) = match &e {
                crate::error::AntennaModelError::FeedNotFound { .. } => {
                    (StatusCode::NOT_FOUND, "feed_not_found")
                }
                crate::error::AntennaModelError::InvalidCoordinate { .. } => {
                    (StatusCode::BAD_REQUEST, "invalid_coordinate")
                }
                crate::error::AntennaModelError::Validation(_) => {
                    (StatusCode::BAD_REQUEST, "validation_error")
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };

            let error_response = ErrorResponse::new(error_type, e.to_string());
            Err(poem::Error::from_string(
                serde_json::to_string(&error_response).unwrap_or_default(),
                status_code,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_app_state_uptime() {
        let state = AppState::with_defaults();

        // Get initial uptime
        let uptime1 = state.uptime_seconds();

        // Wait a bit
        sleep(Duration::from_millis(100)).await;

        // Get uptime again
        let uptime2 = state.uptime_seconds();

        // Uptime should have increased (or at least not decreased)
        assert!(uptime2 >= uptime1);
    }

    #[test]
    fn test_app_state_version() {
        let state = AppState::with_defaults();
        assert_eq!(state.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_app_state_initial_uptime() {
        let state = AppState::with_defaults();
        let uptime = state.uptime_seconds();
        // Should be very close to 0 when just created
        assert!(uptime <= 1);
    }

    #[test]
    fn test_app_state_readiness() {
        let state = AppState::with_defaults();

        // Should be ready by default
        assert!(state.is_ready());

        // Mark not ready
        state.mark_not_ready();
        assert!(!state.is_ready());

        // Mark ready again
        state.mark_ready();
        assert!(state.is_ready());
    }

    #[test]
    fn test_app_state_antenna_ids() {
        let state = AppState::with_defaults();

        // Should start empty
        assert_eq!(state.get_antenna_ids(), Vec::<String>::new());

        // Set some antenna IDs
        let ids = vec!["antenna_1".to_string(), "antenna_2".to_string()];
        state.set_antenna_ids(ids.clone());

        // Should match what we set
        assert_eq!(state.get_antenna_ids(), ids);
    }

    #[test]
    fn test_app_state_memory_usage() {
        let state = AppState::with_defaults();
        let memory = state.get_memory_usage();

        // On Linux, we should get a value
        #[cfg(target_os = "linux")]
        {
            // Memory might be None if /proc/self/statm is not available
            // but in most cases it should be Some
            if let Some(mem) = memory {
                assert!(mem > 0);
            }
        }

        // On non-Linux, should be None
        #[cfg(not(target_os = "linux"))]
        {
            assert!(memory.is_none());
        }
    }

    // Note: Handler function tests are in routes.rs module tests
    // since poem #[handler] macro creates wrapper types that must be tested via routes
}
