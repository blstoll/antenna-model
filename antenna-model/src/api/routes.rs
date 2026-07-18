//! API routing configuration
//!
//! This module defines all the API routes and combines them with middleware.
//!
//! # Middleware Stack
//!
//! Execution order, **outermost → innermost → handler**:
//! 1. **Tracing** - Built-in poem tracing middleware (outermost)
//! 2. **RequestId** - Generate/propagate unique request IDs
//! 3. **RequestLogger** - Comprehensive structured logging with timing
//! 4. **ErrorHandler** - Consistent error response formatting
//! 5. **RequestSizeTracker** - Enforce/track request & response body sizes (413)
//! 6. **RequestTimeout** - Enforce the per-request processing deadline (504, innermost)
//!
//! This order ensures:
//! - `RequestId` populates the request-id extension **before** any other layer
//!   reads it, so every log line and every error path (incl. the 413 and 504
//!   rejections) carries the real id rather than `"unknown"`.
//! - `RequestLogger` times and logs completion for **every** outcome, including a
//!   timeout — because the 504 error propagates back out through it.
//! - Body-size (413) rejection sits **outside** the timeout window (instant), and
//!   the timeout wraps only handler execution (innermost).
//!
//! ## poem ordering caveat (why the `.with(...)` chain is written in reverse)
//!
//! `EndpointExt::with(m)` returns `m.transform(self)` — the middleware **wraps**
//! the current endpoint — so the **last** `.with(...)` becomes the **outermost**
//! layer. The chain in [`build_app`] is therefore written handler-inward-first
//! (`RequestTimeout` applied first = innermost, `Tracing` last = outermost). A
//! prior revision listed the layers in source order and got the runtime order
//! exactly backwards; do not "tidy" the chain into declaration order.

use crate::api::handlers;
use crate::api::middleware::{
    ErrorHandler, RequestId, RequestLogger, RequestSizeTracker, RequestTimeout,
};
use crate::api::AppState;
use poem::{get, middleware::Tracing, post, Endpoint, EndpointExt, Route};
use std::sync::Arc;
use std::time::Duration;

/// Default soft warn thresholds for request / response body sizes (bytes).
const DEFAULT_WARN_REQUEST_SIZE: usize = 1_000_000;
const DEFAULT_WARN_RESPONSE_SIZE: usize = 10_000_000;

/// Assemble the full route table and production middleware stack.
///
/// All public builders delegate here so the route table and middleware ordering
/// live in exactly one place. See the module docs for the execution order and
/// the poem `.with(...)` ordering caveat: the chain below is written
/// handler-inward-first on purpose (the **last** `.with` is the **outermost**
/// layer). Do not reorder it into declaration order.
fn build_app(
    state: Arc<AppState>,
    max_request_size: usize,
    warn_request_size: usize,
    warn_response_size: usize,
    timeout: Duration,
) -> impl Endpoint {
    Route::new()
        // Health and status endpoints (Sprint 5, Task 5.3)
        .at("/health", get(handlers::health)) // Liveness probe
        .at("/ready", get(handlers::ready)) // Readiness probe
        .at("/status", get(handlers::status)) // Detailed status
        // Gain computation endpoints (Sprint 5, Task 5.5; Sprint 6, Task 6.1)
        .at("/api/v1/gain", post(handlers::compute_gain))
        .at("/api/v1/gain/batch", post(handlers::compute_gain_batch))
        // Heatmap endpoint (Sprint 6, Task 6.2)
        .at("/api/v1/heatmap", post(handlers::generate_heatmap_endpoint))
        // H3 link budget endpoint (Sprint 6, Task 6.x)
        .at("/api/v1/h3-heatmap", post(handlers::h3_link_budget))
        // Antenna listing and details endpoints (Sprint 6, Task 6.3)
        .at("/api/v1/antennas", get(handlers::list_antennas))
        .at("/api/v1/antennas/:id", get(handlers::get_antenna_details))
        .at(
            "/api/v1/antennas/:id/feeds",
            get(handlers::list_antenna_feeds),
        )
        .at(
            "/api/v1/antennas/:id/feeds/:feed_id",
            get(handlers::get_feed_details),
        )
        // Middleware — written handler-inward-first because poem's LAST `.with`
        // is the OUTERMOST layer (see module docs). Runtime order is therefore
        // Tracing → RequestId → RequestLogger → ErrorHandler → RequestSizeTracker
        // → RequestTimeout → handler.
        .with(RequestTimeout::new(timeout)) // innermost: bounds handler execution only
        .with(RequestSizeTracker::with_limits(
            max_request_size,
            warn_request_size,
            warn_response_size,
        )) // 413 reject + size tracking, outside the timeout window
        .with(ErrorHandler) // Consistent error handling
        .with(RequestLogger) // Comprehensive logging with timing
        .with(RequestId) // Generate/propagate request IDs (must precede all readers)
        .with(Tracing) // outermost: built-in poem tracing span
        // Attach application state (available to every layer + handler)
        .data(state)
}

/// Create all API routes with production-grade middleware.
///
/// Body-size limit and request timeout are read from `state.config.server`.
/// See the module docs for the middleware stack and execution order.
pub fn create_routes(state: Arc<AppState>) -> impl Endpoint {
    let max_body = state.config.server.max_body_size_bytes;
    let timeout = Duration::from_secs(state.config.server.request_timeout_secs);
    build_app(
        state,
        max_body,
        DEFAULT_WARN_REQUEST_SIZE,
        DEFAULT_WARN_RESPONSE_SIZE,
        timeout,
    )
}

/// Create routes with custom size thresholds for testing/special deployments.
///
/// The request timeout is still read from `state.config.server.request_timeout_secs`.
///
/// # Arguments
/// * `state` - Application state
/// * `max_request_size` - Hard reject limit for request bodies (bytes); 413 when exceeded
/// * `warn_request_size` - Warn threshold for request bodies (bytes)
/// * `warn_response_size` - Warn threshold for response bodies (bytes)
pub fn create_routes_with_size_limits(
    state: Arc<AppState>,
    max_request_size: usize,
    warn_request_size: usize,
    warn_response_size: usize,
) -> impl Endpoint {
    let timeout = Duration::from_secs(state.config.server.request_timeout_secs);
    build_app(
        state,
        max_request_size,
        warn_request_size,
        warn_response_size,
        timeout,
    )
}

/// Create routes with an explicit request-timeout `Duration`.
///
/// The public `server.request_timeout_secs` config is whole seconds; this
/// builder lets tests (and special deployments) set a sub-second deadline so the
/// 504 timeout path can be exercised quickly, with a large margin over real
/// compute rather than depending on multi-second wall-clock timing. Body-size
/// limit still comes from config.
pub fn create_routes_with_timeout(state: Arc<AppState>, timeout: Duration) -> impl Endpoint {
    let max_body = state.config.server.max_body_size_bytes;
    build_app(
        state,
        max_body,
        DEFAULT_WARN_REQUEST_SIZE,
        DEFAULT_WARN_RESPONSE_SIZE,
        timeout,
    )
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

    /// Regression pin for the middleware-ordering fix: RequestId is the
    /// outermost application-layer (only Tracing is outer) and attaches the
    /// correlation header even on the error path. Before the fix RequestId sat
    /// near the innermost position and used `?`, so error responses carried no
    /// x-request-id — the failures operators most need to trace were
    /// uncorrelatable. A client-supplied id must be echoed back on the error.
    #[tokio::test]
    async fn test_error_response_carries_request_id() {
        let state = Arc::new(AppState::with_defaults());
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let custom_id = "error-correlation-id-42";
        let response = cli
            .get("/nonexistent")
            .header(REQUEST_ID_HEADER, custom_id)
            .send()
            .await;

        response.assert_status(poem::http::StatusCode::NOT_FOUND);

        let echoed = response
            .0
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok());
        assert_eq!(
            echoed,
            Some(custom_id),
            "error responses must carry the x-request-id correlation header"
        );
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
        let app = create_routes_with_size_limits(state, 10_000, 100, 1000);
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

    // ========== Task 6.3: Antenna Listing & Details Tests ==========

    /// Helper function to create a test calibration repository with sample data
    fn create_test_repository() -> crate::data::repository::CalibrationRepository {
        use crate::data::types::{
            AntennaCalibration, CalibrationMetadata, FeedParameters, MeshParameters,
            PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
        };

        let mut repo = crate::data::repository::CalibrationRepository::new();

        // Create antenna_1 with two feeds (x_band and s_band)
        let metadata1 = CalibrationMetadata::builder()
            .antenna_name("Deep Space Network 34m".to_string())
            .calibration_date("2025-01-15T00:00:00Z".to_string())
            .data_source("test_data.csv".to_string())
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let reflector1 = ReflectorGeometry::builder()
            .diameter_m(34.0)
            .focal_length_m(13.6)
            .f_over_d_ratio(0.4)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let mesh1 = MeshParameters::builder()
            .mesh_spacing_mm(5.0)
            .wire_diameter_mm(0.5)
            .build()
            .unwrap();

        let feed_x = FeedParameters::builder()
            .position(0.05, 0.02, 0.01)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let feed_s = FeedParameters::builder()
            .position(0.0, 0.0, 0.0)
            .q_factor(7.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config_x = PhysicalAntennaConfig::builder()
            .reflector(reflector1.clone())
            .feed(feed_x)
            .mesh(mesh1.clone())
            .build()
            .unwrap();

        let physical_config_s = PhysicalAntennaConfig::builder()
            .reflector(reflector1)
            .feed(feed_s)
            .mesh(mesh1)
            .build()
            .unwrap();

        let validity1 = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(5.0, 85.0)
            .frequency_range(7100.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let validity2 = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(5.0, 85.0)
            .frequency_range(2000.0, 2300.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let cal1_x = AntennaCalibration::builder()
            .antenna_id("antenna_1".to_string())
            .feed_id("x_band".to_string())
            .metadata(metadata1.clone())
            .physical_config(physical_config_x)
            .validity_ranges(validity1)
            .build()
            .unwrap();

        let cal1_s = AntennaCalibration::builder()
            .antenna_id("antenna_1".to_string())
            .feed_id("s_band".to_string())
            .metadata(metadata1)
            .physical_config(physical_config_s)
            .validity_ranges(validity2)
            .build()
            .unwrap();

        repo.add_calibration(cal1_x);
        repo.add_calibration(cal1_s);

        // Create antenna_2 with one feed
        let metadata2 = CalibrationMetadata::builder()
            .antenna_name("Ground Station 12m".to_string())
            .calibration_date("2025-01-20T00:00:00Z".to_string())
            .data_source("test_data_2.csv".to_string())
            .rmse_db(0.3)
            .r_squared(0.99)
            .num_measurements(500)
            .build()
            .unwrap();

        let reflector2 = ReflectorGeometry::builder()
            .diameter_m(12.0)
            .focal_length_m(4.8)
            .f_over_d_ratio(0.4)
            .surface_rms_mm(0.3)
            .build()
            .unwrap();

        let feed2 = FeedParameters::builder()
            .position(0.0, 0.0, 0.0)
            .q_factor(9.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config2 = PhysicalAntennaConfig::builder()
            .reflector(reflector2)
            .feed(feed2)
            .build()
            .unwrap();

        let validity3 = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(10.0, 80.0)
            .frequency_range(10000.0, 12000.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let cal2 = AntennaCalibration::builder()
            .antenna_id("antenna_2".to_string())
            .feed_id("ku_band".to_string())
            .metadata(metadata2)
            .physical_config(physical_config2)
            .validity_ranges(validity3)
            .build()
            .unwrap();

        repo.add_calibration(cal2);

        repo
    }

    #[tokio::test]
    async fn test_list_antennas() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas").send().await;
        response.assert_status_is_ok();
        response.assert_content_type("application/json; charset=utf-8");

        let body = response.json().await;
        let json_value = body.value();

        let antennas = json_value.object().get("antennas").array();
        assert_eq!(antennas.len(), 2);

        // Check first antenna (should be antenna_1 - alphabetically sorted)
        let ant1 = antennas.get(0).object();
        assert_eq!(ant1.get("id").string(), "antenna_1");
        assert_eq!(ant1.get("name").string(), "Deep Space Network 34m");
        assert!(ant1.get("enabled").bool());
        assert_eq!(ant1.get("feed_count").i64(), 2);

        let feed_ids = ant1.get("feed_ids").array();
        assert_eq!(feed_ids.len(), 2);
        // Feed IDs should be sorted
        assert_eq!(feed_ids.get(0).string(), "s_band");
        assert_eq!(feed_ids.get(1).string(), "x_band");

        // Check second antenna
        let ant2 = antennas.get(1).object();
        assert_eq!(ant2.get("id").string(), "antenna_2");
        assert_eq!(ant2.get("name").string(), "Ground Station 12m");
        assert_eq!(ant2.get("feed_count").i64(), 1);
    }

    #[tokio::test]
    async fn test_get_antenna_details() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas/antenna_1").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        assert_eq!(json_value.get("id").string(), "antenna_1");
        assert_eq!(json_value.get("name").string(), "Deep Space Network 34m");
        assert!(json_value.get("enabled").bool());

        // Check feeds
        let feeds = json_value.get("feeds").array();
        assert_eq!(feeds.len(), 2);

        // Check validity ranges
        let validity = json_value.get("validity_ranges").object();
        let azimuth = validity.get("azimuth_deg").array();
        assert_eq!(azimuth.get(0).f64(), 0.0);
        assert_eq!(azimuth.get(1).f64(), 360.0);

        // Check calibration info
        let calibration = json_value.get("calibration").object();
        assert_eq!(calibration.get("version").string(), "2.0");
        assert_eq!(calibration.get("rmse_db").f64(), 0.5);

        // Check physical parameters
        let physical = json_value.get("physical_parameters").object();
        assert_eq!(physical.get("diameter_m").f64(), 34.0);
        assert_eq!(physical.get("focal_length_m").f64(), 13.6);

        // Check mesh parameters (should exist for antenna_1)
        let mesh = physical.get("mesh").object();
        assert_eq!(mesh.get("mesh_spacing_mm").f64(), 5.0);
    }

    #[tokio::test]
    async fn test_get_antenna_details_not_found() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas/nonexistent").send().await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);

        let body = response.json().await;
        let json_value = body.value().object();
        assert_eq!(json_value.get("error").string(), "antenna_not_found");
    }

    #[tokio::test]
    async fn test_list_antenna_feeds() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas/antenna_1/feeds").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        let feeds = json_value.get("feeds").array();
        assert_eq!(feeds.len(), 2);

        // Check first feed (sorted alphabetically: s_band)
        let feed1 = feeds.get(0).object();
        assert_eq!(feed1.get("id").string(), "s_band");
        assert_eq!(feed1.get("q_factor").f64(), 7.0);

        let pos1 = feed1.get("position_offset").object();
        assert_eq!(pos1.get("x").f64(), 0.0);
        assert_eq!(pos1.get("y").f64(), 0.0);
        assert_eq!(pos1.get("z").f64(), 0.0);

        let freq1 = feed1.get("frequency_range_mhz").array();
        assert_eq!(freq1.get(0).f64(), 2000.0);
        assert_eq!(freq1.get(1).f64(), 2300.0);

        // Check second feed (x_band)
        let feed2 = feeds.get(1).object();
        assert_eq!(feed2.get("id").string(), "x_band");
        assert_eq!(feed2.get("q_factor").f64(), 8.0);

        let pos2 = feed2.get("position_offset").object();
        assert_eq!(pos2.get("x").f64(), 0.05);
        assert_eq!(pos2.get("y").f64(), 0.02);
        assert_eq!(pos2.get("z").f64(), 0.01);
    }

    #[tokio::test]
    async fn test_list_antenna_feeds_not_found() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas/nonexistent/feeds").send().await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_feed_details() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli
            .get("/api/v1/antennas/antenna_1/feeds/x_band")
            .send()
            .await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        assert_eq!(json_value.get("id").string(), "x_band");
        assert_eq!(json_value.get("q_factor").f64(), 8.0);

        let position = json_value.get("position_offset").object();
        assert_eq!(position.get("x").f64(), 0.05);
        assert_eq!(position.get("y").f64(), 0.02);
        assert_eq!(position.get("z").f64(), 0.01);

        let freq_range = json_value.get("frequency_range_mhz").array();
        assert_eq!(freq_range.get(0).f64(), 7100.0);
        assert_eq!(freq_range.get(1).f64(), 8500.0);
    }

    #[tokio::test]
    async fn test_get_feed_details_feed_not_found() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli
            .get("/api/v1/antennas/antenna_1/feeds/nonexistent")
            .send()
            .await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);

        let body = response.json().await;
        let json_value = body.value().object();
        assert_eq!(json_value.get("error").string(), "feed_not_found");
        assert!(json_value
            .get("message")
            .string()
            .contains("Feed 'nonexistent' not found for antenna 'antenna_1'"));
    }

    #[tokio::test]
    async fn test_get_feed_details_antenna_not_found() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli
            .get("/api/v1/antennas/nonexistent/feeds/x_band")
            .send()
            .await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);

        let body = response.json().await;
        let json_value = body.value().object();
        assert_eq!(json_value.get("error").string(), "antenna_not_found");
    }

    #[tokio::test]
    async fn test_antenna_without_mesh() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas/antenna_2").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        let physical = json_value.get("physical_parameters").object();
        // antenna_2 has no mesh, so the mesh field should be omitted
        // (serde with skip_serializing_if = None omits the field)
        // We just verify the other fields are present
        assert_eq!(physical.get("diameter_m").f64(), 12.0);
        assert_eq!(physical.get("surface_rms_mm").f64(), 0.3);
        // The mesh field is not present in the JSON, which is correct behavior
    }

    #[tokio::test]
    async fn test_multi_feed_antenna_support() {
        use crate::config::ServiceConfig;
        let repo = create_test_repository();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // Get antenna_1 which has multiple feeds
        let response = cli.get("/api/v1/antennas/antenna_1").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        let feeds = json_value.get("feeds").array();
        assert_eq!(feeds.len(), 2);

        // Verify we can get details for each feed individually
        for i in 0..feeds.len() {
            let feed = feeds.get(i);
            let feed_id = feed.object().get("id").string();
            let feed_response = cli
                .get(format!("/api/v1/antennas/antenna_1/feeds/{}", feed_id))
                .send()
                .await;
            feed_response.assert_status_is_ok();
        }
    }

    #[tokio::test]
    async fn test_antenna_endpoints_with_empty_repository() {
        use crate::config::ServiceConfig;
        let repo = crate::data::repository::CalibrationRepository::new();
        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        // List should return empty array
        let response = cli.get("/api/v1/antennas").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();
        let antennas = json_value.get("antennas").array();
        assert_eq!(antennas.len(), 0);

        // Details should return 404
        let response = cli.get("/api/v1/antennas/antenna_1").send().await;
        response.assert_status(poem::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_antenna_details_with_uncalibrated_status() {
        use crate::config::ServiceConfig;
        use crate::data::types::{
            AntennaCalibration, CalibrationMetadata, CalibrationStatus, FeedParameters,
            PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
        };

        let mut repo = crate::data::repository::CalibrationRepository::new();

        // Create an uncalibrated antenna
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Uncalibrated Antenna")
            .calibration_date("N/A")
            .format_version("2.0")
            .data_source("design_specifications")
            .rmse_db(f64::NAN)
            .r_squared(f64::NAN)
            .num_measurements(0)
            .build()
            .unwrap();

        let reflector = ReflectorGeometry::builder()
            .diameter_m(10.0)
            .focal_length_m(5.0)
            .f_over_d_ratio(0.5)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.0)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let validity = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("uncalibrated_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(validity)
            .calibration_status(CalibrationStatus::Uncalibrated {
                accuracy_estimate_db: 3.0,
                loss_accuracy_estimate_db: 2.0,
            })
            .build()
            .unwrap();

        repo.add_calibration(calibration);

        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli
            .get("/api/v1/antennas/uncalibrated_antenna")
            .send()
            .await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        // Check basic antenna info
        assert_eq!(json_value.get("id").string(), "uncalibrated_antenna");
        assert_eq!(json_value.get("name").string(), "Test Uncalibrated Antenna");

        // Check calibration status
        let calibration_status = json_value.get("calibration_status").object();
        assert_eq!(calibration_status.get("status").string(), "uncalibrated");
        assert_eq!(calibration_status.get("accuracy_estimate_db").f64(), 3.0);
        assert_eq!(
            calibration_status.get("loss_accuracy_estimate_db").f64(),
            2.0
        );
        assert!(!calibration_status.get("correction_applied").bool());
        assert_eq!(
            calibration_status.get("parameters_source").string(),
            "design_specifications"
        );
    }

    #[tokio::test]
    async fn test_antenna_details_with_partially_calibrated_status() {
        use crate::config::ServiceConfig;
        use crate::data::types::{
            AntennaCalibration, CalibrationCoverage, CalibrationMetadata, CalibrationStatus,
            FeedParameters, PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
        };

        let mut repo = crate::data::repository::CalibrationRepository::new();

        // Create a partially calibrated antenna
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Partially Calibrated Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .format_version("2.0")
            .data_source("boresight_measurements")
            .rmse_db(1.5)
            .r_squared(0.95)
            .num_measurements(50)
            .build()
            .unwrap();

        let reflector = ReflectorGeometry::builder()
            .diameter_m(10.0)
            .focal_length_m(5.0)
            .f_over_d_ratio(0.5)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.0)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let validity = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 0.0)
            .elevation_range(0.0, 0.0)
            .frequency_range(8000.0, 9000.0)
            .num_measurements(50)
            .has_correction_surface(false)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("partially_calibrated_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(validity)
            .calibration_status(CalibrationStatus::PartiallyCalibrated {
                accuracy_estimate_db: 1.5,
                coverage: coverage.clone(),
            })
            .calibration_coverage(coverage)
            .build()
            .unwrap();

        repo.add_calibration(calibration);

        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli
            .get("/api/v1/antennas/partially_calibrated_antenna")
            .send()
            .await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        // Check basic antenna info
        assert_eq!(
            json_value.get("id").string(),
            "partially_calibrated_antenna"
        );
        assert_eq!(
            json_value.get("name").string(),
            "Test Partially Calibrated Antenna"
        );

        // Check calibration status
        let calibration_status = json_value.get("calibration_status").object();
        assert_eq!(
            calibration_status.get("status").string(),
            "partially_calibrated"
        );
        assert_eq!(calibration_status.get("accuracy_estimate_db").f64(), 1.5);
        assert!(!calibration_status.get("correction_applied").bool());
        assert_eq!(
            calibration_status.get("parameters_source").string(),
            "measurement_tuned"
        );

        // Check coverage info
        let coverage = calibration_status.get("coverage").object();
        assert!(coverage.get("is_boresight_only").bool());
        let freq_range = coverage.get("frequency_range_mhz").array();
        assert_eq!(freq_range.get(0).f64(), 8000.0);
        assert_eq!(freq_range.get(1).f64(), 9000.0);
    }

    #[tokio::test]
    async fn test_antenna_details_with_fully_calibrated_status() {
        use crate::config::ServiceConfig;
        use crate::data::types::{
            AntennaCalibration, CalibrationMetadata, CalibrationStatus, FeedParameters,
            PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
        };

        let mut repo = crate::data::repository::CalibrationRepository::new();

        // Create a fully calibrated antenna
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Fully Calibrated Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .format_version("2.0")
            .data_source("full_grid_measurements")
            .rmse_db(0.8)
            .r_squared(0.99)
            .num_measurements(5000)
            .build()
            .unwrap();

        let reflector = ReflectorGeometry::builder()
            .diameter_m(10.0)
            .focal_length_m(5.0)
            .f_over_d_ratio(0.5)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.0)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let validity = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("fully_calibrated_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(validity)
            .calibration_status(CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db: 1.0,
            })
            .build()
            .unwrap();

        repo.add_calibration(calibration);

        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli
            .get("/api/v1/antennas/fully_calibrated_antenna")
            .send()
            .await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        // Check basic antenna info
        assert_eq!(json_value.get("id").string(), "fully_calibrated_antenna");
        assert_eq!(
            json_value.get("name").string(),
            "Test Fully Calibrated Antenna"
        );

        // Check calibration status
        let calibration_status = json_value.get("calibration_status").object();
        assert_eq!(
            calibration_status.get("status").string(),
            "fully_calibrated"
        );
        assert_eq!(calibration_status.get("accuracy_estimate_db").f64(), 1.0);
        assert!(!calibration_status.get("correction_applied").bool());
        assert_eq!(
            calibration_status.get("parameters_source").string(),
            "measurement_tuned"
        );
    }

    #[tokio::test]
    async fn test_antenna_details_backward_compatibility_without_status() {
        use crate::config::ServiceConfig;
        use crate::data::types::{
            AntennaCalibration, CalibrationMetadata, FeedParameters, PhysicalAntennaConfig,
            ReflectorGeometry, ValidityRanges,
        };

        let mut repo = crate::data::repository::CalibrationRepository::new();

        // Create antenna without calibration_status (old format)
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Legacy Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .format_version("1.0")
            .data_source("legacy_measurements")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let reflector = ReflectorGeometry::builder()
            .diameter_m(10.0)
            .focal_length_m(5.0)
            .f_over_d_ratio(0.5)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.0)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let validity = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 9000.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("legacy_antenna")
            .feed_id("test_feed")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(validity)
            // Note: NOT setting calibration_status
            .build()
            .unwrap();

        repo.add_calibration(calibration);

        let state = Arc::new(AppState::new(ServiceConfig::with_defaults(), repo));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/api/v1/antennas/legacy_antenna").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        let json_value = body.value().object();

        // Check basic antenna info
        assert_eq!(json_value.get("id").string(), "legacy_antenna");
        assert_eq!(json_value.get("name").string(), "Legacy Antenna");

        // calibration_status field is not present (backward compatibility)
        // The schema uses Option<CalibrationStatusInfo> with skip_serializing_if = "Option::is_none"
        // which means None values are not serialized to JSON
        // We verify backward compatibility by ensuring the response is still valid without it
        assert!(json_value.get("enabled").bool());
        let feeds = json_value.get("feeds").array();
        assert_eq!(feeds.len(), 1);
    }
}
