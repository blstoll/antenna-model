//! API middleware
//!
//! This module provides production-grade middleware for request logging, timing,
//! error handling, and other cross-cutting concerns.
//!
//! # Middleware Components
//!
//! - **RequestId**: Generates and propagates unique request IDs for tracing
//! - **RequestLogger**: Comprehensive structured logging with timing metrics
//! - **ErrorHandler**: Consistent error response formatting
//! - **RequestSizeTracker**: Tracks request and response body sizes

use poem::{Endpoint, IntoResponse, Middleware, Request, Response, Result};
use std::time::Instant;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Request ID header name
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Extension key for request ID
#[derive(Clone, Debug)]
pub struct RequestIdExt(pub String);

/// Extension key for request start time
#[derive(Clone, Debug)]
pub struct RequestStartTime(pub Instant);

/// Request ID middleware - generates unique ID for each request
///
/// If the client provides an x-request-id header, it will be used.
/// Otherwise, a new UUID v4 will be generated.
pub struct RequestId;

impl<E: Endpoint> Middleware<E> for RequestId {
    type Output = RequestIdImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestIdImpl { ep }
    }
}

pub struct RequestIdImpl<E> {
    ep: E,
}

impl<E: Endpoint> Endpoint for RequestIdImpl<E> {
    type Output = Response;

    async fn call(&self, mut req: Request) -> Result<Self::Output> {
        // Extract or generate request ID
        let request_id = req
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // Store request ID in extensions for downstream handlers
        req.extensions_mut()
            .insert(RequestIdExt(request_id.clone()));

        // Call the endpoint
        let mut response = self.ep.call(req).await.map(IntoResponse::into_response)?;

        // Add request ID to response headers
        // Note: request_id is a valid UUID string, but we handle the parse error defensively
        if let Ok(header_value) = request_id.parse() {
            response
                .headers_mut()
                .insert(REQUEST_ID_HEADER, header_value);
        } else {
            // Log the error but don't fail the request
            warn!(request_id = %request_id, "Failed to parse request ID as header value");
        }

        Ok(response)
    }
}

/// Request logging middleware with comprehensive metrics
///
/// Logs:
/// - Request method, path, and ID
/// - Request completion time
/// - Response status code
/// - Request/response sizes (if available)
/// - Errors with context
pub struct RequestLogger;

impl<E: Endpoint> Middleware<E> for RequestLogger {
    type Output = RequestLoggerImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestLoggerImpl { ep }
    }
}

pub struct RequestLoggerImpl<E> {
    ep: E,
}

impl<E: Endpoint> Endpoint for RequestLoggerImpl<E> {
    type Output = Response;

    async fn call(&self, mut req: Request) -> Result<Self::Output> {
        let start_time = Instant::now();
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let query = req.uri().query().map(|q| q.to_string());

        // Get request ID from extensions (set by RequestId middleware)
        let request_id = req
            .extensions()
            .get::<RequestIdExt>()
            .map(|ext| ext.0.clone())
            .unwrap_or_else(|| "unknown".to_string());

        // Get request body size if available
        let request_size = req
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok());

        // Store start time in extensions for potential use by handlers
        req.extensions_mut().insert(RequestStartTime(start_time));

        // Log incoming request
        info!(
            request_id = %request_id,
            method = %method,
            path = %path,
            query = ?query,
            request_size_bytes = ?request_size,
            "Incoming request"
        );

        // Call the endpoint
        let result = self.ep.call(req).await;

        // Calculate elapsed time
        let elapsed_ms = start_time.elapsed().as_secs_f64() * 1000.0;

        match result {
            Ok(response) => {
                let response = response.into_response();
                let status = response.status();

                // Get response body size if available
                let response_size = response
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<usize>().ok());

                if status.is_success() {
                    info!(
                        request_id = %request_id,
                        method = %method,
                        path = %path,
                        status = status.as_u16(),
                        elapsed_ms = format!("{:.2}", elapsed_ms),
                        response_size_bytes = ?response_size,
                        "Request completed successfully"
                    );
                } else if status.is_client_error() {
                    warn!(
                        request_id = %request_id,
                        method = %method,
                        path = %path,
                        status = status.as_u16(),
                        elapsed_ms = format!("{:.2}", elapsed_ms),
                        "Request completed with client error"
                    );
                } else if status.is_server_error() {
                    error!(
                        request_id = %request_id,
                        method = %method,
                        path = %path,
                        status = status.as_u16(),
                        elapsed_ms = format!("{:.2}", elapsed_ms),
                        "Request completed with server error"
                    );
                }

                Ok(response)
            }
            Err(err) => {
                error!(
                    request_id = %request_id,
                    method = %method,
                    path = %path,
                    error = %err,
                    elapsed_ms = format!("{:.2}", elapsed_ms),
                    "Request failed with error"
                );
                Err(err)
            }
        }
    }
}

/// Error handling middleware
///
/// Ensures all errors are formatted consistently and logged appropriately.
/// This middleware catches any errors that weren't already handled by
/// endpoint-specific error handlers.
pub struct ErrorHandler;

impl<E: Endpoint> Middleware<E> for ErrorHandler {
    type Output = ErrorHandlerImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        ErrorHandlerImpl { ep }
    }
}

pub struct ErrorHandlerImpl<E> {
    ep: E,
}

impl<E: Endpoint> Endpoint for ErrorHandlerImpl<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        let request_id = req
            .extensions()
            .get::<RequestIdExt>()
            .map(|ext| ext.0.clone())
            .unwrap_or_else(|| "unknown".to_string());

        match self.ep.call(req).await {
            Ok(response) => Ok(response.into_response()),
            Err(err) => {
                // Log the error with request context
                error!(
                    request_id = %request_id,
                    error = %err,
                    "Request error caught by error handler"
                );

                // Return the error response (poem handles error to response conversion)
                Err(err)
            }
        }
    }
}

/// Request size tracking middleware
///
/// Tracks and logs the sizes of request and response bodies.
/// Useful for monitoring API usage patterns and identifying
/// potentially problematic large requests.
pub struct RequestSizeTracker {
    /// Warn if request size exceeds this threshold (bytes)
    pub warn_request_size: usize,
    /// Warn if response size exceeds this threshold (bytes)
    pub warn_response_size: usize,
}

impl RequestSizeTracker {
    /// Create a new request size tracker with default thresholds
    pub fn new() -> Self {
        Self {
            warn_request_size: 1_000_000,   // 1 MB
            warn_response_size: 10_000_000, // 10 MB
        }
    }

    /// Create a new request size tracker with custom thresholds
    pub fn with_thresholds(warn_request_size: usize, warn_response_size: usize) -> Self {
        Self {
            warn_request_size,
            warn_response_size,
        }
    }
}

impl Default for RequestSizeTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Endpoint> Middleware<E> for RequestSizeTracker {
    type Output = RequestSizeTrackerImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestSizeTrackerImpl {
            ep,
            warn_request_size: self.warn_request_size,
            warn_response_size: self.warn_response_size,
        }
    }
}

pub struct RequestSizeTrackerImpl<E> {
    ep: E,
    warn_request_size: usize,
    warn_response_size: usize,
}

impl<E: Endpoint> Endpoint for RequestSizeTrackerImpl<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        let request_id = req
            .extensions()
            .get::<RequestIdExt>()
            .map(|ext| ext.0.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let path = req.uri().path().to_string();

        // Check request size
        if let Some(size) = req
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
        {
            if size > self.warn_request_size {
                warn!(
                    request_id = %request_id,
                    path = %path,
                    size_bytes = size,
                    threshold_bytes = self.warn_request_size,
                    "Large request body detected"
                );
            }
        }

        let response = self.ep.call(req).await.map(IntoResponse::into_response)?;

        // Check response size
        if let Some(size) = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
        {
            if size > self.warn_response_size {
                warn!(
                    request_id = %request_id,
                    path = %path,
                    size_bytes = size,
                    threshold_bytes = self.warn_response_size,
                    "Large response body detected"
                );
            }
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poem::{http::StatusCode, test::TestClient, EndpointExt, Route};

    #[tokio::test]
    async fn test_request_id_generation() {
        let app = Route::new()
            .at(
                "/test",
                poem::endpoint::make_sync(|req: Request| {
                    let ext = req.extensions().get::<RequestIdExt>().unwrap();
                    ext.0.clone()
                }),
            )
            .with(RequestId);

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status_is_ok();

        // Check that response has request ID header
        assert!(response.0.headers().contains_key(REQUEST_ID_HEADER));
    }

    #[tokio::test]
    async fn test_request_id_propagation() {
        let test_id = "test-request-id-12345";

        let app = Route::new()
            .at(
                "/test",
                poem::endpoint::make_sync(|req: Request| {
                    let ext = req.extensions().get::<RequestIdExt>().unwrap();
                    ext.0.clone()
                }),
            )
            .with(RequestId);

        let cli = TestClient::new(app);
        let response = cli
            .get("/test")
            .header(REQUEST_ID_HEADER, test_id)
            .send()
            .await;

        response.assert_status_is_ok();

        // Check that the same request ID was used
        let body = response.0.into_body().into_string().await.unwrap();
        assert_eq!(body, test_id);
    }

    #[tokio::test]
    async fn test_request_logger_success() {
        let app = Route::new()
            .at("/test", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(RequestId)
            .with(RequestLogger);

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_request_logger_error() {
        let app = Route::new()
            .at(
                "/test",
                poem::endpoint::make_sync(|_req: Request| {
                    Err::<String, _>(poem::Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))
                }),
            )
            .with(RequestId)
            .with(RequestLogger);

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status(StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_error_handler() {
        let app = Route::new()
            .at(
                "/test",
                poem::endpoint::make_sync(|_req: Request| {
                    Err::<String, _>(poem::Error::from_string(
                        "Test error",
                        StatusCode::BAD_REQUEST,
                    ))
                }),
            )
            .with(RequestId)
            .with(ErrorHandler);

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status(StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_request_size_tracker() {
        let app = Route::new()
            .at("/test", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(RequestId)
            .with(RequestSizeTracker::new());

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_middleware_chain_order() {
        // Test that middleware executes in the correct order
        let app = Route::new()
            .at(
                "/test",
                poem::endpoint::make_sync(|req: Request| {
                    // Verify request ID is available (set by RequestId middleware)
                    assert!(req.extensions().get::<RequestIdExt>().is_some());
                    // Verify start time is available (set by RequestLogger middleware)
                    assert!(req.extensions().get::<RequestStartTime>().is_some());
                    "OK"
                }),
            )
            .with(RequestId)
            .with(RequestLogger)
            .with(ErrorHandler);

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_timing_measurement() {
        // Test that RequestLogger adds timing information to request extensions
        let app = Route::new()
            .at(
                "/test",
                poem::endpoint::make_sync(|req: Request| {
                    // Verify that start time was recorded by RequestLogger middleware
                    let start_time = req.extensions().get::<RequestStartTime>();
                    assert!(
                        start_time.is_some(),
                        "RequestStartTime should be set by RequestLogger"
                    );
                    "OK"
                }),
            )
            .with(RequestId)
            .with(RequestLogger);

        let cli = TestClient::new(app);
        let response = cli.get("/test").send().await;
        response.assert_status_is_ok();
    }
}
