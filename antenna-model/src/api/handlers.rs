//! API request handlers
//!
//! This module implements HTTP request handlers for all API endpoints.

use crate::api::schemas::{HealthResponse, StatusResponse};
use crate::api::AppState;
use poem::{
    handler,
    http::StatusCode,
    web::{Data, Json},
    Response,
};
use std::sync::Arc;
use tracing::info;

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
