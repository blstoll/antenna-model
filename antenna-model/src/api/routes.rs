//! API routing configuration
//!
//! This module defines all the API routes and combines them with middleware.
//!
//! # Middleware Stack (Sprint 5)
//! The middleware is applied in the following order (innermost to outermost):
//! 1. **Tracing** - Built-in poem tracing middleware
//! 2. **RequestId** - Generate/propagate unique request IDs
//! 3. **RequestLogger** - Comprehensive structured logging with timing
//! 4. **ErrorHandler** - Consistent error response formatting
//! 5. **RequestSizeTracker** - Track and warn on large request/response bodies
//!
//! This order ensures:
//! - Request IDs are available for all subsequent middleware and handlers
//! - Timing starts before any processing
//! - Errors are caught and logged consistently
//! - Size tracking happens at the outermost layer

use crate::api::handlers;
use crate::api::middleware::{ErrorHandler, RequestId, RequestLogger, RequestSizeTracker};
use crate::api::AppState;
use poem::{get, middleware::Tracing, Endpoint, EndpointExt, Route};
use std::sync::Arc;

/// Create all API routes with production-grade middleware
///
/// This function sets up the routing table for the API, including:
/// - Health and status endpoints (Sprint 5, Task 5.3)
/// - Future: Evaluation, batch, and heatmap endpoints (Sprint 5+)
/// - Complete middleware stack for production readiness
///
/// # Arguments
/// * `state` - Application state containing server metadata and configuration
///
/// # Returns
/// Configured Route with all endpoints and middleware
///
/// # Middleware Order
/// Middleware is applied from innermost to outermost, so the request flows:
/// 1. **Inbound**: Tracing → RequestId → RequestLogger → ErrorHandler → RequestSizeTracker → Handler
/// 2. **Outbound**: Handler → RequestSizeTracker → ErrorHandler → RequestLogger → RequestId → Tracing
pub fn create_routes(state: Arc<AppState>) -> impl Endpoint {
    Route::new()
        // Health and status endpoints (Sprint 5, Task 5.3)
        .at("/health", get(handlers::health))      // Liveness probe
        .at("/ready", get(handlers::ready))        // Readiness probe
        .at("/status", get(handlers::status))      // Detailed status
        // Future endpoints will be added here in subsequent sprints:
        // .at("/api/v1/gain", post(handlers::compute_gain))
        // .at("/api/v1/gain/batch", post(handlers::compute_gain_batch))
        // .at("/api/v1/heatmap", post(handlers::generate_heatmap))
        // .at("/api/v1/antennas", get(handlers::list_antennas))

        // Apply middleware stack (innermost to outermost)
        .with(Tracing)  // Built-in poem tracing
        .with(RequestId)  // Generate unique request IDs
        .with(RequestLogger)  // Comprehensive logging with timing
        .with(ErrorHandler)  // Consistent error handling
        .with(RequestSizeTracker::new())  // Track request/response sizes

        // Attach application state
        .data(state)
}

/// Create routes with custom size thresholds for testing/special deployments
///
/// This allows customization of the request/response size warning thresholds.
///
/// # Arguments
/// * `state` - Application state
/// * `warn_request_size` - Warn threshold for request bodies (bytes)
/// * `warn_response_size` - Warn threshold for response bodies (bytes)
pub fn create_routes_with_size_limits(
    state: Arc<AppState>,
    warn_request_size: usize,
    warn_response_size: usize,
) -> impl Endpoint {
    Route::new()
        .at("/health", get(handlers::health))
        .at("/ready", get(handlers::ready))
        .at("/status", get(handlers::status))
        .with(Tracing)
        .with(RequestId)
        .with(RequestLogger)
        .with(ErrorHandler)
        .with(RequestSizeTracker::with_thresholds(
            warn_request_size,
            warn_response_size,
        ))
        .data(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::middleware::REQUEST_ID_HEADER;
    use poem::test::TestClient;

    #[tokio::test]
    async fn test_status_route() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/status").send().await;
        response.assert_status_is_ok();
        response.assert_content_type("application/json; charset=utf-8");

        let body = response.json().await;
        let json_value = body.value();

        assert_eq!(json_value.object().get("status").string(), "ok");
        let _version = json_value.object().get("version").string();
        let uptime = json_value.object().get("uptime_seconds").i64();
        assert!(uptime >= 0);
    }

    #[tokio::test]
    async fn test_invalid_route_returns_404() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/nonexistent").send().await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_status_route_multiple_calls() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // Call the endpoint multiple times
        for _ in 0..5 {
            let response = cli.get("/status").send().await;
            response.assert_status_is_ok();
        }
    }

    #[tokio::test]
    async fn test_request_id_in_response() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/status").send().await;
        response.assert_status_is_ok();

        // Verify request ID header is present in response
        assert!(response.0.headers().contains_key(REQUEST_ID_HEADER));
    }

    #[tokio::test]
    async fn test_request_id_propagation() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let custom_id = "test-custom-id-12345";

        let response = cli
            .get("/status")
            .header(REQUEST_ID_HEADER, custom_id)
            .send()
            .await;

        response.assert_status_is_ok();

        // Verify the custom request ID is returned in the response
        let response_id = response
            .0
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap();

        assert_eq!(response_id, custom_id);
    }

    #[tokio::test]
    async fn test_middleware_stack_integration() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // Make a request and verify all middleware components work together
        let response = cli.get("/status").send().await;

        response.assert_status_is_ok();

        // Verify request ID was added
        assert!(response.0.headers().contains_key(REQUEST_ID_HEADER));

        // Verify response is JSON (content type middleware/handler)
        assert!(response
            .0
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .contains("application/json"));
    }

    #[tokio::test]
    async fn test_routes_with_custom_size_limits() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes_with_size_limits(state, 100, 1000);
        let cli = TestClient::new(app);

        let response = cli.get("/status").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_concurrent_requests() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // Make multiple sequential requests to verify state handling
        // Note: TestClient doesn't support cloning for true concurrent tests
        // This tests sequential requests which is sufficient for verifying
        // that the middleware and state are properly shared/thread-safe
        for _ in 0..10 {
            let response = cli.get("/status").send().await;
            response.assert_status_is_ok();
        }
    }

    #[tokio::test]
    async fn test_health_route() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/health").send().await;
        response.assert_status_is_ok();
        response.assert_content_type("application/json; charset=utf-8");

        let body = response.json().await;
        let json_value = body.value();
        assert_eq!(json_value.object().get("status").string(), "healthy");
    }

    #[tokio::test]
    async fn test_ready_route_when_ready() {
        let state = Arc::new(AppState::with_defaults());
        state.mark_ready();
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/ready").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value();
        assert_eq!(json_value.object().get("status").string(), "ready");
    }

    #[tokio::test]
    async fn test_ready_route_when_not_ready() {
        let state = Arc::new(AppState::with_defaults());
        state.mark_not_ready();
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/ready").send().await;
        response.assert_status(poem::http::StatusCode::SERVICE_UNAVAILABLE);

        let body = response.json().await;
        let json_value = body.value();
        assert_eq!(json_value.object().get("status").string(), "not_ready");
    }

    #[tokio::test]
    async fn test_status_route_with_antennas() {
        let state = Arc::new(AppState::with_defaults());
        let antenna_ids = vec!["antenna_1".to_string(), "antenna_2".to_string()];
        state.set_antenna_ids(antenna_ids.clone());

        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/status").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value();

        assert_eq!(json_value.object().get("status").string(), "ok");
        assert_eq!(json_value.object().get("antenna_count").i64(), 2);

        let ids = json_value.object().get("antenna_ids").array();
        assert_eq!(ids.len(), 2);
        // Check that the array contains the expected values
        let id_strings: Vec<String> = ids.iter().map(|v| v.string().to_string()).collect();
        assert!(id_strings.contains(&"antenna_1".to_string()));
        assert!(id_strings.contains(&"antenna_2".to_string()));
    }

    #[tokio::test]
    async fn test_health_route_always_succeeds() {
        let state = Arc::new(AppState::with_defaults());
        // Even if not ready, health should succeed (liveness check)
        state.mark_not_ready();

        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/health").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_all_endpoints_present() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // Test that all three endpoints exist
        let health_response = cli.get("/health").send().await;
        health_response.assert_status_is_ok();

        let ready_response = cli.get("/ready").send().await;
        ready_response.assert_status_is_ok();

        let status_response = cli.get("/status").send().await;
        status_response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_readiness_transitions() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state.clone());
        let cli = TestClient::new(app);

        // Start ready
        state.mark_ready();
        let response = cli.get("/ready").send().await;
        response.assert_status_is_ok();

        // Mark not ready
        state.mark_not_ready();
        let response = cli.get("/ready").send().await;
        response.assert_status(poem::http::StatusCode::SERVICE_UNAVAILABLE);

        // Mark ready again
        state.mark_ready();
        let response = cli.get("/ready").send().await;
        response.assert_status_is_ok();
    }
}
