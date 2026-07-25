//! The single construction point for HTTP error responses (roadmap unit C4).
//!
//! Every error the service returns is an [`ErrorResponse`] serialized as JSON.
//! Before C4 each site built one by hand with
//! `poem::Error::from_string(serde_json::to_string(&body)?, status)`. That
//! produces the right bytes with **no `Content-Type` header at all** — poem's
//! `from_string` has no way to know the string is JSON, and does not guess.
//! Clients received a correct body and no indication of how to read it.
//!
//! [`json_error`] is the replacement. It emits byte-identical bodies with the
//! correct content type, and it is the only place in the crate that turns an
//! `ErrorResponse` into a `poem::Error` — a new error site cannot reintroduce
//! the `text/plain` bug without going around this module.
//!
//! The error-code vocabulary itself lives in [`crate::api::schemas::error_codes`]
//! (roadmap unit C3); this module owns the transport, not the taxonomy.

use crate::api::schemas::ErrorResponse;
use poem::http::StatusCode;
use poem::Response;

/// Body used when serializing an [`ErrorResponse`] fails.
///
/// Unreachable in practice: `ErrorResponse` is four owned `String` fields, and
/// `serde_json` cannot fail on those. It exists because the alternative to a
/// fallback is `unwrap()` (banned in production code) or `unwrap_or_default()`,
/// which would emit an **empty** body under `Content-Type: application/json` —
/// a parse error on the client for what should be a readable failure. The
/// pre-C4 sites all used `unwrap_or_default()`.
const SERIALIZATION_FALLBACK: &str =
    r#"{"error":"internal_error","message":"failed to serialize the error response"}"#;

/// Build a `poem::Error` carrying `body` as a JSON response with `status`.
///
/// The returned error serializes `body` exactly as the hand-rolled sites did,
/// so response bytes are unchanged; only the `Content-Type` header differs.
///
/// The error's `Display` is set to the same JSON payload. That is deliberate:
/// `poem::Error::from_response` leaves the message empty and falls back to
/// printing the bare status, which would have silently reduced every
/// `error = %err` log line in the request logger and error handler from the
/// full error body to the string `"422 Unprocessable Entity"`.
pub fn json_error(status: StatusCode, body: &ErrorResponse) -> poem::Error {
    let payload =
        serde_json::to_string(body).unwrap_or_else(|_| SERIALIZATION_FALLBACK.to_string());

    let response = Response::builder()
        .status(status)
        .content_type("application/json")
        .body(payload.clone());

    let mut error = poem::Error::from_response(response);
    error.set_error_message(payload);
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schemas::error_codes;

    #[tokio::test]
    async fn carries_the_json_content_type_and_the_exact_serialized_body() {
        let body = ErrorResponse::new(error_codes::VALIDATION_ERROR, "frequency out of range")
            .with_field("frequency_mhz");
        let expected = serde_json::to_string(&body).expect("ErrorResponse serializes");

        let response = json_error(StatusCode::UNPROCESSABLE_ENTITY, &body).into_response();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );

        let bytes = response.into_body().into_bytes().await.expect("body");
        assert_eq!(String::from_utf8_lossy(&bytes), expected);
    }

    #[test]
    fn preserves_the_status_for_middleware_that_inspects_it() {
        let body = ErrorResponse::new(error_codes::INTERNAL_ERROR, "boom");
        assert_eq!(
            json_error(StatusCode::INTERNAL_SERVER_ERROR, &body).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// The request logger and error handler log `error = %err`; an empty
    /// `Display` would degrade those lines to the bare status text.
    #[test]
    fn display_is_the_json_payload_not_the_bare_status() {
        let body = ErrorResponse::new(error_codes::ANTENNA_NOT_FOUND, "Antenna 'x' not found");
        let rendered = json_error(StatusCode::NOT_FOUND, &body).to_string();

        assert!(
            rendered.contains(error_codes::ANTENNA_NOT_FOUND),
            "expected the payload in Display, got {rendered:?}"
        );
        assert!(
            rendered.contains("Antenna 'x' not found"),
            "expected the message in Display, got {rendered:?}"
        );
    }
}
