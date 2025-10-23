//! API routing configuration
//!
//! This module defines all the API routes and combines them with middleware.

use crate::api::handlers;
use crate::api::middleware;
use crate::api::AppState;
use poem::{get, middleware::Tracing, Endpoint, EndpointExt, Route};
use std::sync::Arc;

/// Create all API routes with middleware
///
/// This function sets up the routing table for the API, including:
/// - Status endpoint at /status
/// - Tracing middleware for request logging
///
/// # Arguments
/// * `state` - Application state containing server metadata
///
/// # Returns
/// Configured Route with all endpoints and middleware
pub fn create_routes(state: Arc<AppState>) -> impl Endpoint {
    Route::new()
        // Status endpoint - basic health check
        .at("/status", get(handlers::status))
        // Add request tracing middleware
        .with(Tracing)
        // Add request logging middleware
        .with(middleware::RequestLogger)
        // Attach application state
        .data(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use poem::test::TestClient;

    #[tokio::test]
    async fn test_status_route() {
        let state = Arc::new(AppState::new());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/status").send().await;
        response.assert_status_is_ok();
        response.assert_content_type("application/json; charset=utf-8");

        let body = response.json().await;
        let json_value = body.value();

        assert_eq!(json_value.object().get("status").string(), "ok");
        // Just verify these fields exist and are the right type by accessing them
        let _version = json_value.object().get("version").string();
        let uptime = json_value.object().get("uptime_seconds").i64();
        assert!(uptime >= 0);
    }

    #[tokio::test]
    async fn test_invalid_route_returns_404() {
        let state = Arc::new(AppState::new());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/nonexistent").send().await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_status_route_multiple_calls() {
        let state = Arc::new(AppState::new());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // Call the endpoint multiple times
        for _ in 0..5 {
            let response = cli.get("/status").send().await;
            response.assert_status_is_ok();
        }
    }
}
