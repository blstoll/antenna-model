//! API request handlers
//!
//! This module implements HTTP request handlers for all API endpoints.

use crate::api::schemas::StatusResponse;
use crate::api::AppState;
use poem::{handler, web::Data, web::Json};
use std::sync::Arc;
use tracing::info;

/// GET /status - Service status endpoint
///
/// Returns the current status of the service including version and uptime.
/// This endpoint is used for health checks and monitoring.
///
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - status: "ok" when service is operational
/// - version: Application version from Cargo.toml
/// - uptime_seconds: Seconds since server started
///
/// # Example Response
/// ```json
/// {
///   "status": "ok",
///   "version": "0.1.0",
///   "uptime_seconds": 3600
/// }
/// ```
#[handler]
pub async fn status(state: Data<&Arc<AppState>>) -> Json<StatusResponse> {
    let uptime = state.uptime_seconds();
    let version = state.version.to_string();

    info!(
        version = version,
        uptime_seconds = uptime,
        "Status endpoint called"
    );

    Json(StatusResponse::ok(version, uptime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_app_state_uptime() {
        let state = AppState::new();

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
        let state = AppState::new();
        assert_eq!(state.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_app_state_initial_uptime() {
        let state = AppState::new();
        let uptime = state.uptime_seconds();
        // Should be very close to 0 when just created
        assert!(uptime <= 1);
    }
}
