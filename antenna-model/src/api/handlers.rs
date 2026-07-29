//! API request handlers
//!
//! This module implements HTTP request handlers for all API endpoints.

use crate::api::error_response::{
    batch_validation_error, json_error, service_error, validation_error,
};
use crate::api::schemas::{
    AntennaDetailsResponse, AntennaListResponse, BatchGainRequest, BatchGainResponse,
    CalibrationStatusInfo, ErrorCode, ErrorResponse, GainRequest, GainResponse,
    H3LinkBudgetRequest, H3LinkBudgetResponse, HealthResponse, HeatmapRequest, HeatmapResponse,
    StatusResponse,
};
use crate::api::AppState;
use crate::service::{
    compute_gain_from_request_with_budget, compute_h3_link_budget_with_budget,
    evaluate_batch_with_budget, generate_heatmap_with_budget, validator,
};
use poem::{
    handler,
    http::StatusCode,
    web::{Data, Json},
    Response,
};
use std::sync::Arc;
use std::time::Duration;
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
/// - status: "healthy" when calibration data is loaded
/// - status: "degraded" when the service is responsive but has no calibration data loaded
///
/// Always returns 200 — a non-200 liveness response would restart the pod, which cannot
/// fix missing calibration data. Use `/ready` to keep traffic away (roadmap S5).
///
/// # Example Response
/// ```json
/// {
///   "status": "healthy"
/// }
/// ```
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    operation_id = "getHealth",
    summary = "Health check (liveness probe)",
    description = "Always returns 200 while the process is responsive. Used for the Kubernetes liveness
probe, so it deliberately never returns a failure status: a restart cannot fix
missing calibration data.

The `status` field reports `healthy` when calibration data is loaded, or `degraded`
when the service is running with an empty repository (calibration load failed and
`calibration.fail_fast` was off). A degraded instance never becomes ready, so it
receives no traffic — see `/ready`.",
    responses(
        (status = 200, description = r#"Service is responsive ("healthy", or "degraded" with no data loaded)"#, body = HealthResponse)
    )
)]
#[handler]
pub async fn health(state: Data<&Arc<AppState>>) -> Json<HealthResponse> {
    // Always HTTP 200 — this is the liveness probe (see HealthResponse::degraded). An
    // empty repository means no antenna can be evaluated, which is degraded but alive.
    if state.repository.antenna_count() == 0 {
        Json(HealthResponse::degraded())
    } else {
        Json(HealthResponse::healthy())
    }
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
#[utoipa::path(
    get,
    path = "/ready",
    tag = "health",
    operation_id = "getReady",
    summary = "Readiness check (readiness probe)",
    description = "Returns 200 only once the calibration load has completed successfully. Returns 503
during startup, when the calibration load failed, and for the whole graceful-shutdown
drain window.",
    responses(
        (status = 200, description = r#"Service is ready to serve requests ({"status": "ready"})"#, body = HealthResponse),
        (status = 503, description = r#"Not ready — startup, failed calibration load, or shutting down
({"status": "not_ready"})."#, body = HealthResponse)
    )
)]
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
/// - antenna_count: Number of loaded antennas (always present, 0 on a degraded start)
/// - antenna_ids: List of loaded antenna IDs (always present, `[]` on a degraded start)
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
#[utoipa::path(
    get,
    path = "/status",
    tag = "health",
    operation_id = "getStatus",
    summary = "Service status and metadata",
    description = "Returns comprehensive service information including loaded antennas, uptime, and version.",
    responses(
        (status = 200, description = "Service status information", body = StatusResponse)
    )
)]
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

    // Always report antenna_count/antenna_ids, even when empty (roadmap S5): a degraded
    // start (empty repository) must be visible to monitoring as "0 antennas", not silently
    // omitted, which would be indistinguishable from "field not implemented".
    let mut response = StatusResponse::ok(version, uptime).with_antennas(antenna_ids);

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
/// - vehicle_position: Vehicle position (ECEF or Geodetic, per its `coordinate_system`)
/// - reflector_boresight: Reflector boresight position (ECEF or Geodetic)
/// - feed_pointing_location: Earth location the feed's beam is aimed at (ECEF or Geodetic)
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
/// Returns HTTP 422 if the body parsed but is semantically invalid, HTTP 404 if the
/// named antenna/feed does not exist, HTTP 400 only if the body could not be parsed.
///
/// # Example Request
/// ```json
/// {
///   "antenna_id": "antenna_1",
///   "feed_id": "feed_1",
///   "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "reflector_boresight": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "feed_pointing_location": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "emitter_position": {"x": 42164000.0, "y": 0.0, "z": 0.0},
///   "frequency_mhz": 11450.0,
///   "include_reference": true
/// }
/// ```
#[utoipa::path(
    post,
    path = "/api/v1/gain",
    tag = "gain",
    operation_id = "computeGain",
    summary = "Compute antenna gain from 3D geometric configuration",
    description = "Computes antenna gain for a specific geometric configuration including vehicle position,
antenna orientation, feed position, and emitter location. Each position declares
its own frame — ECEF or Geodetic — in its required `coordinate_system` field.

Optionally computes reference gain (ideal case) and loss (reference - actual).",
    request_body(content = GainRequest, examples(
        ("ecef_coordinates" = (summary = "ECEF coordinates example", value = json!({
            "antenna_id": "antenna_1",
            "feed_id": "x_band_feed",
            "vehicle_position": {"x": 4510731.123, "y": 4510731.456, "z": 3488865.789, "coordinate_system": "ecef"},
            "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
            "reflector_boresight": {"x": 4510732.0, "y": 4510732.0, "z": 3488950.0, "coordinate_system": "ecef"},
            "feed_pointing_location": {"x": 4510731.5, "y": 4510731.5, "z": 3488870.0, "coordinate_system": "ecef"},
            "emitter_position": {"x": 4520000.0, "y": 4520000.0, "z": 3500000.0, "coordinate_system": "ecef"},
            "frequency_mhz": 8400.0,
            "include_reference": true
        }))),
        ("geodetic_coordinates" = (summary = "Geodetic coordinates example", value = json!({
            "antenna_id": "antenna_2",
            "feed_id": "s_band_feed",
            "vehicle_position": {"x": -118.1234, "y": 34.5678, "z": 100.0, "coordinate_system": "geodetic"},
            "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
            "reflector_boresight": {"x": -117.0, "y": 35.0, "z": 400000.0, "coordinate_system": "geodetic"},
            "feed_pointing_location": {"x": -118.124, "y": 34.568, "z": 105.0, "coordinate_system": "geodetic"},
            "emitter_position": {"x": -117.0, "y": 35.0, "z": 400000.0, "coordinate_system": "geodetic"},
            "frequency_mhz": 2200.0,
            "include_reference": false
        })))
    )),
    responses(
        (status = 200, description = "Gain computation successful", body = GainResponse, examples(
            ("partially_calibrated" = (summary = "Partially calibrated antenna response", value = json!({
                "antenna_id": "antenna_2",
                "feed_id": "x_band_feed",
                "gain_db": 41.2,
                "reference_gain_db": 43.5,
                "loss_db": 2.3,
                "geometry": {
                    "physical_feed_offset_m": {"x": 0.05, "y": 0.02, "z": 0.01},
                    "emitter_azimuth_deg": 185.5,
                    "emitter_elevation_deg": 32.1,
                    "beam_squint_deg": 0.15
                },
                "warnings": [
                    {"code": "partially_calibrated", "message": "Antenna 'antenna_2' is partially calibrated. Accuracy estimate: ±1.5 dB"},
                    {"code": "out_of_coverage", "message": "Query is outside calibrated region - using physics model extrapolation"}
                ],
                "metadata": {"computation_time_ms": 2.8, "extrapolated": true},
                "calibration_status": {
                    "status": "partially_calibrated",
                    "accuracy_estimate_db": 1.5,
                    "coverage": {
                        "azimuth_range_deg": [0.0, 0.0],
                        "elevation_range_deg": [0.0, 0.0],
                        "frequency_range_mhz": [7100.0, 8500.0],
                        "num_measurements": 25,
                        "is_boresight_only": true
                    },
                    "correction_applied": false,
                    "parameters_source": "boresight_tuning"
                }
            }))),
            ("uncalibrated" = (summary = "Uncalibrated antenna response", value = json!({
                "antenna_id": "antenna_3",
                "feed_id": "x_band_feed",
                "gain_db": 40.5,
                "reference_gain_db": 43.8,
                "loss_db": 3.3,
                "geometry": {
                    "physical_feed_offset_m": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "emitter_azimuth_deg": 45.2,
                    "emitter_elevation_deg": 78.9
                },
                "warnings": [
                    {"code": "uncalibrated", "message": "Antenna 'antenna_3' is uncalibrated (using design specifications). Absolute gain accuracy: ±3.0 dB, Loss accuracy: ±2.0 dB"}
                ],
                "metadata": {"computation_time_ms": 1.5, "extrapolated": false},
                "calibration_status": {
                    "status": "uncalibrated",
                    "accuracy_estimate_db": 3.0,
                    "loss_accuracy_estimate_db": 2.0,
                    "correction_applied": false,
                    "parameters_source": "design_specifications"
                }
            })))
        )),
        (status = 400, description = "The request body could not be parsed. This is the only condition that
returns 400 (roadmap C2) — a body that parses but is invalid returns 422.

Note that a non-finite number lands here rather than in 422: JSON cannot
encode `NaN` or `Infinity`, so serializers emit `null`, which does not
deserialize into the numeric fields this schema declares.", body = ErrorResponse, examples(
            ("unparseable_body" = (summary = "Malformed JSON", value = json!({
                "error": "invalid_request_body",
                "message": "parse error: expected value at line 1 column 3"
            })))
        )),
        (status = 422, description = "The body parsed but is semantically invalid. Applies whether the failure is
caught by the request pre-check or surfaces from the service layer — the
client cannot tell, and should not have to.", body = ErrorResponse, examples(
            ("invalid_frequency" = (summary = "Frequency out of range", value = json!({
                "error": "validation_error",
                "message": "frequency 0 MHz is outside supported range [100, 50000] MHz",
                "field": "frequency_mhz"
            }))),
            ("degenerate_geometry" = (summary = "Boresight coincident with vehicle position", value = json!({
                "error": "invalid_coordinate",
                "message": "invalid coordinate for 'reflector_boresight': Boresight position too close to vehicle position (< 1mm separation)"
            })))
        )),
        (status = 404, description = "The request names an antenna or feed that does not exist. Absence is not
invalidity: no change to the body will make a missing antenna appear, so
this is a 404 rather than a 422 (roadmap C2).", body = ErrorResponse, examples(
            ("antenna_not_found" = (summary = "Antenna not found", value = json!({
                "error": "antenna_not_found",
                "message": "Antenna 'invalid_antenna' not found",
                "field": "antenna_id"
            }))),
            ("feed_not_found" = (summary = "Feed not found", value = json!({
                "error": "feed_not_found",
                "message": "Feed 'invalid_feed' not found for antenna 'antenna_1'",
                "field": "feed_id"
            })))
        )),
        (status = 413, description = include_str!("openapi_descriptions/resp_413_payload_too_large.md"), body = ErrorResponse, examples(
            ("payload_too_large" = (summary = "Payload too large", value = json!({
                "error": "payload_too_large",
                "message": "Request body of 12000000 bytes exceeds the maximum of 10485760 bytes"
            })))
        )),
        (status = 504, description = include_str!("openapi_descriptions/resp_504_budgets.md"), body = ErrorResponse, examples(
            ("request_timeout" = (summary = "Whole-request timeout (S2)", value = json!({
                "error": "request_timeout",
                "message": "Request processing exceeded the configured timeout of 30000 ms"
            }))),
            ("computation_budget_exceeded" = (summary = "Single integration over budget (S3)", value = json!({
                "error": "computation_budget_exceeded",
                "message": "computation exceeded time budget in azimuthal_mode_field: 31000 ms > 30000 ms budget"
            })))
        )),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
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

    // Validate the request. The status is chosen centrally (roadmap C2): 404 when the
    // named antenna/feed does not exist, 422 for anything else.
    if let Err(validation_err) = validator::validate_gain_request(&request, &state.repository) {
        warn!(
            antenna_id = %request.antenna_id,
            feed_id = %request.feed_id,
            error = %validation_err,
            "Request validation failed"
        );
        return Err(validation_error(&validation_err, None));
    }

    // Compute gain using the service layer, bounding each aperture integration to the
    // configured per-integration wall-clock budget (S3).
    //
    // The physics runs synchronously and CPU-bound, which would otherwise block the
    // async worker thread and defeat the RequestTimeout middleware — a future that
    // never yields is never preempted. Offload it to the blocking pool so the async
    // task yields at the join `.await`, letting the timeout fire (roadmap S2b, matching
    // the batch/heatmap/h3 handlers). The compute itself is not cancelled on timeout;
    // `performance.integration_budget_ms` is what bounds the work.
    let compute_state = state.0.clone();
    let compute_request = request.clone();
    let budget = Duration::from_millis(state.config.performance.integration_budget_ms);
    let result = tokio::task::spawn_blocking(move || {
        compute_gain_from_request_with_budget(&compute_request, &compute_state.repository, budget)
    })
    .await
    .map_err(|join_err| {
        error!(error = %join_err, "Gain compute task failed to join");
        let error_response = ErrorResponse::new(
            ErrorCode::InternalError,
            format!("Gain computation task failed: {join_err}"),
        );
        json_error(StatusCode::INTERNAL_SERVER_ERROR, &error_response)
    })?;

    match result {
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

            // Status and code come from the shared policy (roadmap C2). This handler's
            // own `match` had no `Validation(_)` arm, so that class fell through to a
            // 500; it also served `InvalidCoordinate` as 400 for a body that parsed.
            Err(service_error(&e))
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
/// Returns HTTP 422 if the batch is empty, exceeds the size limit, or contains an
/// invalid evaluation; HTTP 404 if an evaluation names a missing antenna/feed.
///
/// # Error Handling
/// Every item is validated before any physics runs, and the first failure rejects the
/// whole batch — 404 for an antenna or feed that does not exist, 422 otherwise — with
/// the offending index in `field` (roadmap C2).
///
/// Per-item degradation survives only for *compute*-class failures, which cannot be
/// predicted in advance: such an item carries a NaN `gain_db` (rendered as JSON `null`)
/// and the reason in its typed `error` object — `{code, message}`, drawn from the same
/// vocabulary as `ErrorResponse.error` — and does not stop the other items. Its
/// `warnings` array is empty; before C8 stage 3 the reason was a prose string in there.
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
///       "feed_pointing_location": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///       "emitter_position": {"x": 42164000.0, "y": 0.0, "z": 0.0},
///       "frequency_mhz": 11450.0,
///       "include_reference": false
///     },
///     // ... more requests ...
///   ]
/// }
/// ```
#[utoipa::path(
    post,
    path = "/api/v1/gain/batch",
    tag = "gain",
    operation_id = "computeGainBatch",
    summary = "Batch gain computation",
    description = "Processes multiple gain computation requests in a single API call.
Automatically uses parallel processing for batches ≥5 requests.

Maximum batch size: 1000 evaluations.",
    request_body(content = BatchGainRequest),
    responses(
        (status = 200, description = "Batch computation successful.

May still include per-item failures, but only *compute*-class ones — an
integration over budget, or one that did not converge — which cannot be
predicted before running the integral. Such an item carries a `null`
`gain_db` (a NaN, as JSON has no NaN literal) and the reason in its
`warnings`. Validation-class failures never reach this response: they reject
the whole batch (roadmap C2).", body = BatchGainResponse),
        (status = 400, description = include_str!("openapi_descriptions/resp_400_see_gain.md"), body = ErrorResponse, examples(
            ("unparseable_body" = (summary = "Malformed JSON", value = json!({
                "error": "invalid_request_body",
                "message": "parse error: expected value at line 1 column 3"
            })))
        )),
        (status = 422, description = "The batch parsed but is semantically invalid — empty, over the 1000-item
limit, or containing an invalid evaluation.

Every item is validated before any physics runs, and the first failure
rejects the whole request. `field` carries the offending item's path
(`evaluations[3]`) so a client can locate it without parsing the message.", body = ErrorResponse, examples(
            ("empty_batch" = (summary = "Empty evaluations array", value = json!({
                "error": "validation_error",
                "message": "invalid value for parameter 'evaluations': batch must contain at least one evaluation",
                "field": "evaluations"
            }))),
            ("size_limit" = (summary = "Over the batch size limit", value = json!({
                "error": "validation_error",
                "message": "batch size 1001 exceeds limit of 1000",
                "field": "evaluations"
            }))),
            ("invalid_item" = (summary = "One evaluation is invalid", value = json!({
                "error": "validation_error",
                "message": "evaluations[3]: frequency 0 MHz is outside supported range [100, 50000] MHz",
                "field": "evaluations[3]"
            })))
        )),
        (status = 404, description = "An evaluation names an antenna or feed that does not exist. Rejects the whole
batch, with the offending item's path in `field`.", body = ErrorResponse, examples(
            ("antenna_not_found" = (summary = "Unknown antenna in one item", value = json!({
                "error": "antenna_not_found",
                "message": "evaluations[3]: antenna 'invalid_antenna' not found",
                "field": "evaluations[3]"
            })))
        )),
        (status = 413, description = include_str!("openapi_descriptions/resp_413_payload_too_large.md"), body = ErrorResponse, examples(
            ("payload_too_large" = (summary = "Payload too large", value = json!({
                "error": "payload_too_large",
                "message": "Request body of 12000000 bytes exceeds the maximum of 10485760 bytes"
            })))
        )),
        (status = 504, description = include_str!("openapi_descriptions/resp_504_budgets.md"), body = ErrorResponse, examples(
            ("request_timeout" = (summary = "Whole-request timeout (S2)", value = json!({
                "error": "request_timeout",
                "message": "Request processing exceeded the configured timeout of 30000 ms"
            }))),
            ("computation_budget_exceeded" = (summary = "Single integration over budget (S3)", value = json!({
                "error": "computation_budget_exceeded",
                "message": "computation exceeded time budget in azimuthal_mode_field: 31000 ms > 30000 ms budget"
            })))
        )),
        (status = 503, description = include_str!("openapi_descriptions/resp_503_overloaded.md"), body = ErrorResponse,
         headers(("Retry-After" = i32, description = "Seconds to wait before retrying (performance.admission_retry_after_secs).")),
         examples(
            ("service_overloaded" = (summary = "Heavy-request concurrency limit reached (S4)", value = json!({
                "error": "service_overloaded",
                "message": "Server is at its concurrent heavy-request limit (8); retry after 5 s"
            })))
        ))
    )
)]
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

    // Pre-validate the whole batch before any physics runs (roadmap C2, call C).
    //
    // Before C2 nothing validated items: a bad item reached the evaluator, failed
    // there, and came back inside an HTTP 200 as `"gain_db": null` — a `f64::NAN`
    // that `serde_json` renders as `null` — with the reason in that item's
    // `warnings`. A client checking the status code saw success, and one
    // deserializing `gain_db` into a non-optional float got a parse error instead of
    // an error response.
    //
    // The rejection names the failing item (`field: "evaluations[3]"`) and keeps its
    // class, so an absent antenna is 404 here exactly as it is on `/api/v1/gain`.
    if let Err(batch_err) = validator::validate_batch_gain_request(&request, &state.repository) {
        warn!(
            num_evaluations = num_evaluations,
            field = %batch_err.field(),
            error = %batch_err,
            "Batch request validation failed"
        );
        return Err(batch_validation_error(&batch_err));
    }

    // Process the batch using the service layer. The service runs rayon
    // synchronously (CPU-bound), which would otherwise block the async worker
    // thread and defeat the RequestTimeout middleware. Offload it to the blocking
    // pool so the async task yields at the join `.await`, letting the timeout
    // fire. (The rayon work is not cancelled on timeout — see RequestTimeout.)
    let state = state.0.clone();
    let budget = Duration::from_millis(state.config.performance.integration_budget_ms);
    let result = tokio::task::spawn_blocking(move || {
        evaluate_batch_with_budget(&request, &state.repository, budget)
    })
    .await
    .map_err(|join_err| {
        error!(error = %join_err, "Batch compute task failed to join");
        let error_response = ErrorResponse::new(
            ErrorCode::InternalError,
            format!("Batch computation task failed: {join_err}"),
        );
        json_error(StatusCode::INTERNAL_SERVER_ERROR, &error_response)
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

            // Shared status policy (roadmap C2). Per-item over-budget failures become
            // error results inside the batch; this arm covers whole-batch failures,
            // which after the pre-check above should only be compute-class.
            Err(service_error(&e))
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
/// - feed_pointing_location: Earth location the feed's beam is aimed at (ECEF or Geodetic)
/// - frequency_mhz: Operating frequency
/// - pointing_frequency_mhz: Optional pointing frequency for beam squint
/// - grid_config: Grid configuration (rectangular)
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
/// Returns HTTP 422 if the request is semantically invalid (bad grid configuration or
/// out-of-range value), HTTP 404 if the named antenna/feed does not exist, HTTP 400 if
/// the body could not be parsed — including an `h3` grid_type, which is an unknown
/// `grid_config` enum variant rather than a validated grid.
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
///   "feed_pointing_location": {"x": 6500000.0, "y": 0.0, "z": 0.0},
///   "frequency_mhz": 8400.0,
///   "grid_config": {
///     "grid_type": "rectangular",
///     "azimuth_range_deg": {"min": 0.0, "max": 360.0, "step": 5.0},
///     "elevation_range_deg": {"min": 0.0, "max": 90.0, "step": 2.0}
///   }
/// }
/// ```
#[utoipa::path(
    post,
    path = "/api/v1/heatmap",
    tag = "heatmap",
    operation_id = "generateHeatmap",
    summary = "Generate loss heatmap",
    description = "Generates a 2D loss heatmap across antenna field of view.
Supports rectangular azimuth/elevation grids.

Loss is computed relative to peak gain for each grid point.
Maximum grid size: 100,000 points.",
    request_body(content = HeatmapRequest, examples(
        ("rectangular_grid" = (summary = "Rectangular grid (73x46 = 3358 points)", value = json!({
            "antenna_id": "antenna_1",
            "feed_id": "x_band_feed",
            "vehicle_position": {"x": 4510731.123, "y": 4510731.456, "z": 3488865.789, "coordinate_system": "ecef"},
            "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
            "reflector_boresight": {"x": 4510732.0, "y": 4510732.0, "z": 3488950.0, "coordinate_system": "ecef"},
            "feed_pointing_location": {"x": 4510731.5, "y": 4510731.5, "z": 3488870.0, "coordinate_system": "ecef"},
            "frequency_mhz": 8400.0,
            "grid_config": {
                "grid_type": "rectangular",
                "azimuth_range_deg": {"min": 0.0, "max": 360.0, "step": 5.0},
                "elevation_range_deg": {"min": 0.0, "max": 90.0, "step": 2.0}
            }
        })))
    )),
    responses(
        (status = 200, description = "Heatmap generation successful", body = HeatmapResponse),
        (status = 400, description = include_str!("openapi_descriptions/resp_400_see_gain.md"), body = ErrorResponse, examples(
            ("unparseable_body" = (summary = "Malformed JSON", value = json!({
                "error": "invalid_request_body",
                "message": "parse error: expected value at line 1 column 3"
            })))
        )),
        (status = 422, description = "The body parsed but is semantically invalid — an out-of-range value, a
degenerate geometry, or a grid exceeding 100,000 points.", body = ErrorResponse, examples(
            ("invalid_grid" = (summary = "Grid too large", value = json!({
                "error": "validation_error",
                "message": "invalid grid specification for rectangular: total grid points 324000 exceeds maximum 100000 (1800x180 grid)"
            })))
        )),
        (status = 404, description = "The request names an antenna or feed that does not exist (roadmap C2).", body = ErrorResponse, examples(
            ("antenna_not_found" = (summary = "Antenna not found", value = json!({
                "error": "antenna_not_found",
                "message": "antenna 'invalid_antenna' not found"
            })))
        )),
        (status = 413, description = include_str!("openapi_descriptions/resp_413_payload_too_large.md"), body = ErrorResponse, examples(
            ("payload_too_large" = (summary = "Payload too large", value = json!({
                "error": "payload_too_large",
                "message": "Request body of 12000000 bytes exceeds the maximum of 10485760 bytes"
            })))
        )),
        (status = 504, description = include_str!("openapi_descriptions/resp_504_budgets.md"), body = ErrorResponse, examples(
            ("request_timeout" = (summary = "Whole-request timeout (S2)", value = json!({
                "error": "request_timeout",
                "message": "Request processing exceeded the configured timeout of 30000 ms"
            }))),
            ("computation_budget_exceeded" = (summary = "Single integration over budget (S3)", value = json!({
                "error": "computation_budget_exceeded",
                "message": "computation exceeded time budget in azimuthal_mode_field: 31000 ms > 30000 ms budget"
            })))
        )),
        (status = 503, description = include_str!("openapi_descriptions/resp_503_overloaded.md"), body = ErrorResponse,
         headers(("Retry-After" = i32, description = "Seconds to wait before retrying (performance.admission_retry_after_secs).")),
         examples(
            ("service_overloaded" = (summary = "Heavy-request concurrency limit reached (S4)", value = json!({
                "error": "service_overloaded",
                "message": "Server is at its concurrent heavy-request limit (8); retry after 5 s"
            })))
        ))
    )
)]
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
        return Err(validation_error(&validation_err, None));
    }

    // Generate heatmap using the service layer. The service runs rayon
    // synchronously (CPU-bound); offload it to the blocking pool so the async
    // task yields at the join `.await` and the RequestTimeout middleware can
    // fire. (The rayon work is not cancelled on timeout — see RequestTimeout.)
    // Pre-extract only the two small fields post-compute logging needs, then
    // MOVE `request` into the closure — avoids deep-cloning the whole
    // HeatmapRequest (grid config + three 3D positions) on every heavy call.
    let compute_state = state.0.clone();
    let budget = Duration::from_millis(compute_state.config.performance.integration_budget_ms);
    let antenna_id = request.antenna_id.clone();
    let feed_id = request.feed_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        generate_heatmap_with_budget(&request, &compute_state.repository, budget)
    })
    .await
    .map_err(|join_err| {
        error!(error = %join_err, "Heatmap compute task failed to join");
        let error_response = ErrorResponse::new(
            ErrorCode::InternalError,
            format!("Heatmap computation task failed: {join_err}"),
        );
        json_error(StatusCode::INTERNAL_SERVER_ERROR, &error_response)
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

            // Shared status policy (roadmap C2).
            Err(service_error(&e))
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
#[utoipa::path(
    get,
    path = "/api/v1/antennas",
    tag = "antennas",
    operation_id = "listAntennas",
    summary = "List all available antennas",
    description = "Returns list of all loaded antenna configurations with basic metadata.",
    responses(
        (status = 200, description = "List of antennas", body = AntennaListResponse)
    )
)]
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
#[utoipa::path(
    get,
    path = "/api/v1/antennas/{id}",
    tag = "antennas",
    operation_id = "getAntennaDetails",
    summary = "Get antenna details",
    description = "Returns detailed information about a specific antenna including feeds, calibration status, and physical parameters.",
    params(
        ("id" = String, Path, description = "Antenna ID", example = "antenna_1")
    ),
    responses(
        (status = 200, description = "Antenna details", body = AntennaDetailsResponse),
        (status = 404, description = "Antenna not found", body = ErrorResponse)
    )
)]
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
            ErrorCode::AntennaNotFound,
            format!("Antenna '{}' not found", antenna_id),
        );
        return Err(json_error(StatusCode::NOT_FOUND, &error_response));
    }

    // Use first feed to get antenna-level information
    let first_feed_id = &feed_ids[0];
    let calibration = state
        .repository
        .get_calibration(&antenna_id, first_feed_id)
        .ok_or_else(|| {
            // Reachable only if the repository lists a feed it cannot then resolve.
            // Before C4 this site returned a bare string body with no error code at
            // all — the one error path in the service that carried nothing
            // machine-readable.
            error!(
                antenna_id = %antenna_id,
                feed_id = %first_feed_id,
                "Repository listed a feed with no retrievable calibration"
            );
            json_error(
                StatusCode::NOT_FOUND,
                &ErrorResponse::new(
                    ErrorCode::FeedNotFound,
                    format!("Feed '{first_feed_id}' not found for antenna '{antenna_id}'"),
                )
                .with_field("feed_id"),
            )
        })?;

    // Build feed information for all feeds
    let mut feeds = Vec::new();
    for feed_id in &feed_ids {
        if let Some(cal) = state.repository.get_calibration(&antenna_id, feed_id) {
            feeds.push(crate::api::schemas::FeedInfo {
                id: feed_id.clone(),
                design_feed_offset_m: crate::api::schemas::Vector3D {
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
        rmse_db: calibration.metadata.rmse_db,
        r_squared: calibration.metadata.r_squared,
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
///       "design_feed_offset_m": {"x": 0.05, "y": 0.02, "z": 0.01},
///       "frequency_range_mhz": [7100.0, 8500.0],
///       "q_factor": 8.0
///     }
///   ]
/// }
/// ```
#[utoipa::path(
    get,
    path = "/api/v1/antennas/{id}/feeds",
    tag = "antennas",
    operation_id = "listFeeds",
    summary = "List feeds for antenna",
    description = "Returns list of all feeds available for the specified antenna.",
    params(
        ("id" = String, Path, description = "Antenna ID", example = "antenna_1")
    ),
    responses(
        (status = 200, description = "List of feeds", body = crate::api::schemas::FeedListResponse),
        (status = 404, description = "Antenna not found", body = ErrorResponse)
    )
)]
#[handler]
pub async fn list_antenna_feeds(
    state: Data<&Arc<AppState>>,
    antenna_id: poem::web::Path<String>,
) -> poem::Result<Json<crate::api::schemas::FeedListResponse>> {
    let antenna_id = antenna_id.0;
    info!(antenna_id = %antenna_id, "Antenna feeds list request received");

    let feed_ids = state.repository.list_feeds(&antenna_id);

    if feed_ids.is_empty() {
        warn!(antenna_id = %antenna_id, "Antenna not found");
        let error_response = ErrorResponse::new(
            ErrorCode::AntennaNotFound,
            format!("Antenna '{}' not found", antenna_id),
        );
        return Err(json_error(StatusCode::NOT_FOUND, &error_response));
    }

    // Build feed information
    let mut feeds = Vec::new();
    for feed_id in &feed_ids {
        if let Some(cal) = state.repository.get_calibration(&antenna_id, feed_id) {
            feeds.push(crate::api::schemas::FeedInfo {
                id: feed_id.clone(),
                design_feed_offset_m: crate::api::schemas::Vector3D {
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
    Ok(Json(crate::api::schemas::FeedListResponse { feeds }))
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
/// - design_feed_offset_m: Feed design offset from focal point (meters)
/// - frequency_range_mhz: Valid frequency range [min, max] in MHz
/// - q_factor: Feed pattern q-factor
///
/// Returns HTTP 404 if antenna or feed not found
///
/// # Example Response
/// ```json
/// {
///   "id": "x_band",
///   "design_feed_offset_m": {"x": 0.05, "y": 0.02, "z": 0.01},
///   "frequency_range_mhz": [7100.0, 8500.0],
///   "q_factor": 8.0
/// }
/// ```
#[utoipa::path(
    get,
    path = "/api/v1/antennas/{id}/feeds/{feed_id}",
    tag = "antennas",
    operation_id = "getFeedDetails",
    summary = "Get feed details",
    description = "Returns detailed information about a specific feed.",
    params(
        ("id" = String, Path, description = "Antenna ID", example = "antenna_1"),
        ("feed_id" = String, Path, description = "Feed ID", example = "x_band_feed")
    ),
    responses(
        (status = 200, description = "Feed details", body = crate::api::schemas::FeedInfo),
        (status = 404, description = "Antenna or feed not found", body = ErrorResponse)
    )
)]
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
                design_feed_offset_m: crate::api::schemas::Vector3D {
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
                    ErrorCode::FeedNotFound,
                    format!("Feed '{}' not found for antenna '{}'", feed_id, antenna_id),
                )
            } else {
                (
                    ErrorCode::AntennaNotFound,
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
            Err(json_error(StatusCode::NOT_FOUND, &error_response))
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
/// - feed_pointing_location: Earth location the feed's beam is aimed at (ECEF or Geodetic)
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
#[utoipa::path(
    post,
    path = "/api/v1/h3-heatmap",
    tag = "heatmap",
    operation_id = "computeH3LinkBudget",
    summary = "Compute a per-cell link budget over an H3 hexagonal grid",
    description = include_str!("openapi_descriptions/op_h3_link_budget.md"),
    request_body(content = H3LinkBudgetRequest, examples(
        ("ground_station_coverage" = (summary = "3.7 m ground station, 2 rings at resolution 7 (19 cells)", value = json!({
            "antenna_id": "gs_3.7m_uncalibrated",
            "feed_id": "s_band_feed",
            "vehicle_position": {"x": -116.889, "y": 35.4267, "z": 1036.0, "coordinate_system": "geodetic"},
            "reflector_boresight": {"x": -116.45, "y": 35.4267, "z": 800.0, "coordinate_system": "geodetic"},
            "feed_pointing_location": {"x": -116.45, "y": 35.4267, "z": 800.0, "coordinate_system": "geodetic"},
            "frequency_mhz": 2200.0,
            "n_rings": 2,
            "h3_resolution": 7,
            "temperature_k": 150.0
        })))
    )),
    responses(
        (status = 200, description = "Link budget computed for every cell in the grid", body = H3LinkBudgetResponse),
        (status = 400, description = include_str!("openapi_descriptions/resp_400_see_gain.md"), body = ErrorResponse, examples(
            ("unparseable_body" = (summary = "Malformed JSON", value = json!({
                "error": "invalid_request_body",
                "message": "parse error: expected value at line 1 column 3"
            })))
        )),
        (status = 422, description = "The body parsed but is semantically invalid — an out-of-range value
(`n_rings` above 10, `h3_resolution` outside 0-15, a non-positive
`temperature_k`, a frequency outside 100-50000 MHz), or a degenerate geometry.", body = ErrorResponse, examples(
            ("too_many_rings" = (summary = "n_rings above the maximum", value = json!({
                "error": "validation_error",
                "message": "invalid value for parameter 'n_rings': n_rings must be ≤ 10"
            }))),
            ("out_of_range_resolution" = (summary = "h3_resolution outside 0-15", value = json!({
                "error": "validation_error",
                "message": "parameter 'h3_resolution' value 20 is out of valid range [0, 15]"
            })))
        )),
        (status = 404, description = "The request names an antenna or feed that does not exist (roadmap C2).", body = ErrorResponse, examples(
            ("antenna_not_found" = (summary = "Antenna not found", value = json!({
                "error": "antenna_not_found",
                "message": "antenna 'invalid_antenna' not found"
            })))
        )),
        (status = 413, description = "Request body exceeds the configured maximum size
(`server.max_body_size_bytes`, default 10 MB). Enforced on both the
`content-length` and `Transfer-Encoding: chunked` framings — see
`/api/v1/heatmap` for the full description.", body = ErrorResponse, examples(
            ("payload_too_large" = (summary = "Payload too large", value = json!({
                "error": "payload_too_large",
                "message": "Request body of 12000000 bytes exceeds the maximum of 10485760 bytes"
            })))
        )),
        (status = 504, description = "A server-side wall-clock budget was exceeded — either the whole request
(`request_timeout`, `server.request_timeout_secs`) or a single aperture
integration (`computation_budget_exceeded`,
`performance.integration_budget_ms`). See `/api/v1/heatmap` for how the two
budgets differ; the fan-out caveat applies here too, since one request
integrates once per cell.", body = ErrorResponse, examples(
            ("request_timeout" = (summary = "Whole-request timeout (S2)", value = json!({
                "error": "request_timeout",
                "message": "Request processing exceeded the configured timeout of 30000 ms"
            }))),
            ("computation_budget_exceeded" = (summary = "Single integration over budget (S3)", value = json!({
                "error": "computation_budget_exceeded",
                "message": "computation exceeded time budget in azimuthal_mode_field: 31000 ms > 30000 ms budget"
            })))
        )),
        (status = 503, description = "Admission control (roadmap S4): the server is already running the maximum
number of concurrent heavy requests (batch / heatmap / h3-heatmap), configured
by `performance.max_concurrent_heavy_requests` and shared across those three
endpoints. The request was rejected immediately (never queued), so a
`Retry-After` header is included. Never returned when the limit is 0 (the
default; admission control disabled).", body = ErrorResponse,
         headers(("Retry-After" = i32, description = "Seconds to wait before retrying (performance.admission_retry_after_secs).")),
         examples(
            ("service_overloaded" = (summary = "Heavy-request concurrency limit reached (S4)", value = json!({
                "error": "service_overloaded",
                "message": "Server is at its concurrent heavy-request limit (8); retry after 5 s"
            })))
        )),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
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
        return Err(validation_error(&validation_err, None));
    }

    // Existence is checked through the shared validator rather than hand-rolled from
    // the lookup miss (roadmap C2): this endpoint's request validator does not take
    // the repository, and before C2 the bespoke rejection here was the reason the same
    // unknown antenna produced 404 on `/h3-heatmap` and 422 on `/gain`.
    if let Err(lookup_err) = validator::validate_antenna_feed_exists(
        &request.antenna_id,
        &request.feed_id,
        &state.repository,
    ) {
        warn!(
            antenna_id = %request.antenna_id,
            feed_id = %request.feed_id,
            error = %lookup_err,
            "H3 link budget antenna/feed lookup failed"
        );
        return Err(validation_error(&lookup_err, None));
    }

    let calibration = match state
        .repository
        .get_calibration(&request.antenna_id, &request.feed_id)
    {
        Some(cal) => cal,
        None => {
            // Unreachable: the existence check above passed, so the pair resolves.
            // Reachable only if the repository were mutated between the two calls,
            // which it is not — it is immutable after startup.
            error!(
                antenna_id = %request.antenna_id,
                feed_id = %request.feed_id,
                "Calibration vanished between the existence check and the lookup"
            );
            return Err(json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &ErrorResponse::new(
                    ErrorCode::InternalError,
                    "calibration lookup failed after a successful existence check",
                ),
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
    let budget = Duration::from_millis(state.config.performance.integration_budget_ms);
    let antenna_id = request.antenna_id.clone();
    let feed_id = request.feed_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        compute_h3_link_budget_with_budget(
            &request,
            &calibration,
            &compute_cache,
            start_time,
            budget,
        )
    })
    .await
    .map_err(|join_err| {
        error!(error = %join_err, "H3 link budget compute task failed to join");
        let error_response = ErrorResponse::new(
            ErrorCode::InternalError,
            format!("H3 link budget computation task failed: {join_err}"),
        );
        json_error(StatusCode::INTERNAL_SERVER_ERROR, &error_response)
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

            // Shared status policy (roadmap C2).
            Err(service_error(&e))
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
        // The not-ready-by-default invariant is pinned by
        // `api::tests::test_app_state_starts_not_ready` (roadmap S5). This test covers only
        // the toggle round-trip.
        let state = AppState::with_defaults();

        state.mark_ready();
        assert!(state.is_ready());

        state.mark_not_ready();
        assert!(!state.is_ready());
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
