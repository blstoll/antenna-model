//! API middleware
//!
//! This module provides middleware for request logging, timing, and other
//! cross-cutting concerns. More sophisticated middleware will be added in Sprint 5.

use poem::{Endpoint, IntoResponse, Middleware, Request, Response, Result};
use tracing::info;

/// Request logging middleware
///
/// Logs basic information about each incoming request including method and path.
/// This is a simple implementation for Sprint 1. More detailed logging with
/// request IDs and timing will be added in Sprint 5.
pub struct RequestLogger;

impl<E: Endpoint> Middleware<E> for RequestLogger {
    type Output = RequestLoggerImpl<E>;

    fn transform(&self, ep: E) -> Self::Output {
        RequestLoggerImpl { ep }
    }
}

/// Implementation of request logging middleware
pub struct RequestLoggerImpl<E> {
    ep: E,
}

impl<E: Endpoint> Endpoint for RequestLoggerImpl<E> {
    type Output = Response;

    async fn call(&self, req: Request) -> Result<Self::Output> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        info!(
            method = %method,
            path = %path,
            "Incoming request"
        );

        let response = self.ep.call(req).await;

        match &response {
            Ok(_) => {
                info!(
                    method = %method,
                    path = %path,
                    "Request completed"
                );
            }
            Err(err) => {
                info!(
                    method = %method,
                    path = %path,
                    error = %err,
                    "Request failed"
                );
            }
        }

        response.map(IntoResponse::into_response)
    }
}
