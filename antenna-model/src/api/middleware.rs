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

use crate::api::schemas::ErrorResponse;
use poem::{Endpoint, IntoResponse, Middleware, Request, Response, Result};
use std::time::{Duration, Instant};
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

        // Call the endpoint. On error, convert the error into its response here
        // (rather than propagating with `?`) so the correlation header is
        // attached to error responses (413 / 504 / 4xx / 5xx) as well. Without
        // this, poem's `?` short-circuit would emit error responses with no
        // x-request-id, leaving exactly the failures operators most need to
        // trace uncorrelatable in both logs and response headers.
        let mut response = match self.ep.call(req).await {
            Ok(resp) => resp.into_response(),
            Err(err) => err.into_response(),
        };

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

/// Request size tracking and enforcement middleware
///
/// Tracks and logs the sizes of request and response bodies, and **rejects**
/// requests whose `content-length` exceeds the configured hard limit with a
/// `413 Payload Too Large` and the project's standard JSON error body.
///
/// Enforcement is keyed on the `content-length` header (the framework-blessed
/// level, matching `poem::middleware::SizeLimit`): if the header is present and
/// exceeds `max_request_size`, the request is rejected before body handling.
/// Requests without a `content-length` header fall through unchanged.
pub struct RequestSizeTracker {
    /// Reject if request size exceeds this hard limit (bytes)
    pub max_request_size: usize,
    /// Warn if request size exceeds this threshold (bytes)
    pub warn_request_size: usize,
    /// Warn if response size exceeds this threshold (bytes)
    pub warn_response_size: usize,
}

impl RequestSizeTracker {
    /// Create a new request size tracker with default thresholds
    ///
    /// The hard reject limit defaults to 10 MB, matching the
    /// `server.max_body_size_bytes` config default.
    pub fn new() -> Self {
        Self {
            max_request_size: 10_000_000,   // 10 MB (matches config default)
            warn_request_size: 1_000_000,   // 1 MB
            warn_response_size: 10_000_000, // 10 MB
        }
    }

    /// Create a new request size tracker with custom thresholds
    ///
    /// * `max_request_size` - hard reject limit for request bodies (413 when exceeded)
    /// * `warn_request_size` - soft warn threshold for request bodies
    /// * `warn_response_size` - soft warn threshold for response bodies
    pub fn with_limits(
        max_request_size: usize,
        warn_request_size: usize,
        warn_response_size: usize,
    ) -> Self {
        Self {
            max_request_size,
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
            max_request_size: self.max_request_size,
            warn_request_size: self.warn_request_size,
            warn_response_size: self.warn_response_size,
        }
    }
}

pub struct RequestSizeTrackerImpl<E> {
    ep: E,
    max_request_size: usize,
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

        // Check request size (enforce the hard limit before body handling)
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

            if size > self.max_request_size {
                warn!(
                    request_id = %request_id,
                    path = %path,
                    size_bytes = size,
                    limit_bytes = self.max_request_size,
                    "Request body exceeds the maximum allowed size; rejecting with 413"
                );

                let body = ErrorResponse::new(
                    "payload_too_large",
                    format!(
                        "Request body of {size} bytes exceeds the maximum of {} bytes",
                        self.max_request_size
                    ),
                );
                return Err(poem::Error::from_string(
                    serde_json::to_string(&body).unwrap_or_default(),
                    poem::http::StatusCode::PAYLOAD_TOO_LARGE,
                ));
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

/// Request timeout enforcement middleware
///
/// Bounds how long the wrapped endpoint may run. If the endpoint future does not
/// complete within `timeout`, the request is abandoned and the client receives a
/// `504 Gateway Timeout` with the project's standard JSON error body
/// (`request_timeout`). This makes `server.request_timeout_secs` an enforced
/// limit rather than a decorative log line.
///
/// # Why 504 (a 5xx), not 408
///
/// This deadline is a *server-side* wall-clock budget: the client sent a valid
/// request promptly, and the server then exceeded its own configured processing
/// limit. RFC 7231 §6.5.7 scopes `408 Request Timeout` to a client that was slow
/// to *send* — a 4xx (client-fault) classification that would misattribute a
/// server-side overrun and hide it from server error-rate SLOs. `504 Gateway
/// Timeout` keeps the fault on the server side (5xx) and, unlike `503`, implies
/// no transient recovery: our timeout is deterministic in the request payload
/// (the same heavy grid re-costs the same), so there is no honest `Retry-After`
/// value — the remedy is a smaller request, not waiting. The literal "gateway"
/// framing is a mild stretch (we have no upstream), accepted as the least-bad
/// standard code. Reconsider `503 + Retry-After` for S4's admission-control /
/// overload rejection, where the condition genuinely *is* transient. The machine
/// `error` code stays `request_timeout` (it names the condition, not the wire
/// status).
///
/// # Important limitation — background compute is NOT cancelled
///
/// `tokio::time::timeout` only fires while the wrapped future is `Pending`. The
/// heavy handlers (batch / heatmap / h3) therefore offload their synchronous
/// rayon compute onto `tokio::task::spawn_blocking` so the async task yields at a
/// real `.await`, letting this timeout fire. When the timeout fires we stop
/// awaiting and return a response to the client, but **the rayon work already
/// running on the blocking pool is not aborted** — dropping the join handle does
/// not stop the pool. It runs to completion (wasting CPU) until it finishes.
/// Cooperative, wall-clock-bounded compute cancellation is roadmap unit S3.
pub struct RequestTimeout {
    timeout: Duration,
}

impl RequestTimeout {
    /// Create a new request-timeout middleware with the given deadline.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl<E: Endpoint> Middleware<E> for RequestTimeout {
    type Output = RequestTimeoutImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestTimeoutImpl {
            ep,
            timeout: self.timeout,
        }
    }
}

pub struct RequestTimeoutImpl<E> {
    ep: E,
    timeout: Duration,
}

impl<E: Endpoint> Endpoint for RequestTimeoutImpl<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        let request_id = req
            .extensions()
            .get::<RequestIdExt>()
            .map(|ext| ext.0.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let path = req.uri().path().to_string();

        match tokio::time::timeout(self.timeout, self.ep.call(req)).await {
            // Endpoint completed within the deadline (Ok or Err both pass through).
            Ok(result) => result.map(IntoResponse::into_response),
            // Deadline elapsed. NOTE: this returns a response to the client but
            // does NOT cancel any rayon compute already running on the blocking
            // pool; that work runs to completion. Cooperative compute bounding
            // is S3.
            Err(_elapsed) => {
                warn!(
                    request_id = %request_id,
                    path = %path,
                    timeout_ms = self.timeout.as_millis(),
                    "Request exceeded the configured timeout; responding with 504"
                );
                let body = ErrorResponse::new(
                    "request_timeout",
                    format!(
                        "Request processing exceeded the configured timeout of {} ms",
                        self.timeout.as_millis()
                    ),
                );
                Err(poem::Error::from_string(
                    serde_json::to_string(&body).unwrap_or_default(),
                    poem::http::StatusCode::GATEWAY_TIMEOUT,
                ))
            }
        }
    }
}

/// Admission-control middleware: cap concurrently-executing heavy requests (roadmap S4)
///
/// Wraps the compute-heavy endpoints (`batch` / `heatmap` / `h3-heatmap`) and gates
/// entry on a shared [`tokio::sync::Semaphore`]. On entry it attempts a
/// **non-blocking** `try_acquire` of one permit:
/// - **permit acquired** → the permit is held for the full duration of handler
///   execution (including the awaited `spawn_blocking` compute) and released when the
///   response is produced, freeing a slot for the next request;
/// - **no permit available** → the request is rejected **immediately** (never queued)
///   with `503 Service Unavailable`, a `Retry-After` header, and the project's standard
///   JSON `ErrorResponse` body (`service_overloaded`).
///
/// One [`ConcurrencyLimit`] is cloned onto each heavy endpoint, all sharing the same
/// `Arc<Semaphore>`, so the cap bounds *total* concurrent heavy work, not per-endpoint.
///
/// # Why `503 + Retry-After` (and not S2's `504`)
///
/// Overload is genuinely **transient** — a slot frees the instant an in-flight heavy
/// request completes — so, unlike the request-timeout (`504`, deterministic in the
/// payload, no `Retry-After`; see [`RequestTimeout`]), a positive `Retry-After` backoff
/// is honest here. `503 Service Unavailable` is the standard transient-overload code.
///
/// # Disabled state
///
/// When the configured limit is `0` (unlimited — the default), the semaphore is `None`
/// and the middleware is a transparent pass-through. This keeps the wiring uniform (the
/// middleware is always applied) while engaging the limiter only when configured.
///
/// # Ordering note
///
/// This layer sits **innermost** (applied per-endpoint, inside the outer stack incl.
/// [`RequestTimeout`]). That is deliberate: if an admitted request later times out, the
/// outer `RequestTimeout` wins its `select`, drops this layer's future, and the held
/// permit is dropped with it — so a timed-out request does not leak its slot.
#[derive(Clone)]
pub struct ConcurrencyLimit {
    /// Shared permit pool; `None` = unlimited (disabled), a transparent pass-through.
    semaphore: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    /// `Retry-After` value (seconds) returned on rejection.
    retry_after_secs: u64,
    /// Configured limit, carried only for the rejection message.
    limit: usize,
}

impl ConcurrencyLimit {
    /// Build an admission-control limiter.
    ///
    /// `limit == 0` disables it (transparent pass-through); a positive `limit` creates a
    /// shared semaphore with that many permits. Prefer [`ConcurrencyLimit::shared`] when
    /// applying the *same* budget to several endpoints.
    pub fn new(limit: usize, retry_after_secs: u64) -> Self {
        let semaphore =
            (limit > 0).then(|| std::sync::Arc::new(tokio::sync::Semaphore::new(limit)));
        Self {
            semaphore,
            retry_after_secs,
            limit,
        }
    }

    /// Build a limiter around an already-constructed shared semaphore, so multiple
    /// endpoints share one budget. `None` = unlimited (disabled).
    pub fn shared(
        semaphore: Option<std::sync::Arc<tokio::sync::Semaphore>>,
        retry_after_secs: u64,
        limit: usize,
    ) -> Self {
        Self {
            semaphore,
            retry_after_secs,
            limit,
        }
    }
}

impl<E: Endpoint> Middleware<E> for ConcurrencyLimit {
    type Output = ConcurrencyLimitImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        ConcurrencyLimitImpl {
            ep,
            semaphore: self.semaphore.clone(),
            retry_after_secs: self.retry_after_secs,
            limit: self.limit,
        }
    }
}

pub struct ConcurrencyLimitImpl<E> {
    ep: E,
    semaphore: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    retry_after_secs: u64,
    limit: usize,
}

impl<E: Endpoint> Endpoint for ConcurrencyLimitImpl<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        // Unlimited (disabled): transparent pass-through, zero overhead.
        let Some(semaphore) = self.semaphore.clone() else {
            return self.ep.call(req).await.map(IntoResponse::into_response);
        };

        match semaphore.try_acquire_owned() {
            // Admitted: hold the permit for the whole handler lifetime (the binding
            // must outlive the `.await`, so it is dropped only after the response is
            // produced — releasing the slot for the next request).
            Ok(_permit) => self.ep.call(req).await.map(IntoResponse::into_response),
            // Saturated: reject immediately with 503 + Retry-After. Built as a
            // `Response` (not a `poem::Error`) because the error path has no header
            // channel and we must attach `Retry-After`. Returning `Ok(response)` lets
            // it flow back out through RequestLogger (logged as a real 503).
            Err(_) => {
                let request_id = req
                    .extensions()
                    .get::<RequestIdExt>()
                    .map(|ext| ext.0.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let path = req.uri().path().to_string();
                warn!(
                    request_id = %request_id,
                    path = %path,
                    limit = self.limit,
                    retry_after_secs = self.retry_after_secs,
                    "Heavy-request concurrency limit reached; rejecting with 503"
                );

                let body = ErrorResponse::new(
                    "service_overloaded",
                    format!(
                        "Server is at its concurrent heavy-request limit ({}); retry after {} s",
                        self.limit, self.retry_after_secs
                    ),
                );
                let response = Response::builder()
                    .status(poem::http::StatusCode::SERVICE_UNAVAILABLE)
                    .header("Retry-After", self.retry_after_secs.to_string())
                    .header("content-type", "application/json")
                    .body(serde_json::to_string(&body).unwrap_or_default());
                Ok(response)
            }
        }
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
    async fn test_request_timeout_fires_on_slow_endpoint() {
        // A handler that sleeps well past the timeout must yield a 504 with the
        // standard JSON error body.
        let app = Route::new()
            .at(
                "/slow",
                poem::endpoint::make(|_req| async {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    Ok::<_, poem::Error>("done")
                }),
            )
            .with(RequestId)
            .with(RequestTimeout::new(Duration::from_millis(20)));

        let cli = TestClient::new(app);
        let response = cli.get("/slow").send().await;
        response.assert_status(StatusCode::GATEWAY_TIMEOUT);

        let body = response.0.into_body().into_string().await.unwrap();
        let err: ErrorResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(err.error, "request_timeout");
    }

    #[tokio::test]
    async fn test_request_timeout_passes_fast_endpoint() {
        // A handler that completes comfortably within the deadline is untouched.
        let app = Route::new()
            .at("/fast", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(RequestId)
            .with(RequestTimeout::new(Duration::from_secs(5)));

        let cli = TestClient::new(app);
        let response = cli.get("/fast").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_concurrency_limit_rejects_when_saturated() {
        // Deterministic (no timing race): pre-acquire the sole permit and hold it, so
        // the semaphore is already saturated when the request arrives. The request must
        // be rejected with 503 + Retry-After and the standard JSON body.
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        let sem = Arc::new(Semaphore::new(1));
        let _held = sem.clone().try_acquire_owned().unwrap(); // occupy the only slot

        let app = Route::new()
            .at("/heavy", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(ConcurrencyLimit::shared(Some(sem.clone()), 7, 1))
            .with(RequestId);

        let cli = TestClient::new(app);
        let response = cli.get("/heavy").send().await;
        response.assert_status(StatusCode::SERVICE_UNAVAILABLE);

        let retry_after = response
            .0
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        assert_eq!(
            retry_after.as_deref(),
            Some("7"),
            "the 503 must carry a Retry-After header (the whole point of S4 vs S2's 504)"
        );

        let content_type = response
            .0
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        assert_eq!(content_type.as_deref(), Some("application/json"));

        let body = response.0.into_body().into_string().await.unwrap();
        let err: ErrorResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(err.error, "service_overloaded");
    }

    #[tokio::test]
    async fn test_concurrency_limit_admits_when_free() {
        // A free permit lets the request through unchanged.
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        let sem = Arc::new(Semaphore::new(1));
        let app = Route::new()
            .at("/heavy", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(ConcurrencyLimit::shared(Some(sem), 5, 1))
            .with(RequestId);

        let cli = TestClient::new(app);
        let response = cli.get("/heavy").send().await;
        response.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_concurrency_limit_disabled_is_passthrough() {
        // limit == 0 -> None semaphore -> transparent, even under repeated calls.
        let app = Route::new()
            .at("/heavy", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(ConcurrencyLimit::new(0, 5))
            .with(RequestId);

        let cli = TestClient::new(app);
        for _ in 0..5 {
            let response = cli.get("/heavy").send().await;
            response.assert_status_is_ok();
        }
    }

    #[tokio::test]
    async fn test_concurrency_limit_releases_permit_after_response() {
        // After a request completes, the permit is released and the next request is
        // admitted (no leak). limit=1, two sequential requests both succeed.
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        let sem = Arc::new(Semaphore::new(1));
        let app = Route::new()
            .at("/heavy", poem::endpoint::make_sync(|_req: Request| "OK"))
            .with(ConcurrencyLimit::shared(Some(sem.clone()), 5, 1))
            .with(RequestId);

        let cli = TestClient::new(app);
        cli.get("/heavy").send().await.assert_status_is_ok();
        cli.get("/heavy").send().await.assert_status_is_ok();
        // Both permits returned: the semaphore is back to full capacity.
        assert_eq!(sem.available_permits(), 1);
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
