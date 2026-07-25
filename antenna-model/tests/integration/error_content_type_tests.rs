//! Every error response is `application/json` (roadmap unit C4).
//!
//! Before C4 the service built error responses with
//! `poem::Error::from_string(serde_json::to_string(&body)?, status)`. The bodies
//! were correct JSON, but `from_string` sets **no `Content-Type` at all** —
//! verified by reverting the helper and watching all four of these tests report
//! `got "<missing>"`. (The roadmap predicted `text/plain`; the reality was
//! worse.) Clients were left to guess the media type.
//!
//! These tests observe the wire through a real HTTP client, and cover the three
//! distinct construction paths — a handler pre-check, a handler mapping a
//! service-layer error, and a middleware rejection that never reaches a handler
//! — because C4's helper has to be the only builder for all of them.
//!
//! `assert_json_error` also pins the body *bytes* to exactly
//! `serde_json::to_string(&ErrorResponse)`: C4 was a header fix, not a payload
//! change.

use crate::integration::error_tests::small_body_limit_config;
use crate::integration::helpers::*;
use antenna_model::api::schemas::*;

/// Assert an error response is JSON, and return its parsed body.
///
/// Checks the media type rather than the raw header so a future charset
/// parameter (`application/json; charset=utf-8`) does not fail the test for a
/// reason nobody cares about.
async fn assert_json_error(response: reqwest::Response, expected_code: &str) -> ErrorResponse {
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("<missing>")
        .to_string();

    assert!(
        content_type.starts_with("application/json"),
        "{status} response must be application/json, got {content_type:?}"
    );

    let raw = response.text().await.expect("readable body");
    let parsed: ErrorResponse = serde_json::from_str(&raw).unwrap_or_else(|e| {
        panic!("{status} body did not parse as ErrorResponse: {e}; got {raw:?}")
    });

    assert_eq!(
        parsed.error, expected_code,
        "unexpected error code in {status} response"
    );

    // C4 changed the header, not the payload: the body must still be exactly
    // `serde_json::to_string(&ErrorResponse)` with nothing added around it.
    let reserialized = serde_json::to_string(&parsed).expect("ErrorResponse re-serializes");
    assert_eq!(
        raw, reserialized,
        "body is not exactly the serialized ErrorResponse"
    );

    parsed
}

/// Handler pre-check path: `validator::validate_gain_request` rejects before any
/// physics runs, and the handler builds the 422 itself.
#[tokio::test]
async fn validation_failure_422_is_json() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.antenna_id = "no_such_antenna".to_string();

    let response = server
        .client
        .post(format!("{}/api/v1/gain", server.base_url))
        .json(&request)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 422);
    let body = assert_json_error(response, "validation_error").await;
    assert!(
        body.message.contains("no_such_antenna"),
        "message should name the offending antenna, got {:?}",
        body.message
    );

    server.shutdown().await;
}

/// Handler-maps-service-error path: the batch endpoint has no pre-check, so the
/// size-limit rejection surfaces from `evaluate_batch` as an
/// `AntennaModelError::Validation` and is mapped to 400 in the handler.
///
/// (That 400-vs-422 split is exactly what roadmap unit C2 unifies; this test
/// pins the *media type*, and deliberately does not bless the status.)
#[tokio::test]
async fn service_layer_400_is_json() {
    let server = TestServer::start().await.unwrap();

    let oversized = BatchGainRequest {
        evaluations: vec![builders::simple_gain_request_ecef(); 1001],
    };

    let response = server
        .client
        .post(format!("{}/api/v1/gain/batch", server.base_url))
        .json(&oversized)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    assert_json_error(response, "validation_error").await;

    server.shutdown().await;
}

/// A 404 built by a GET handler with no request body involved.
#[tokio::test]
async fn antenna_not_found_404_is_json() {
    let server = TestServer::start().await.unwrap();

    let response = server
        .client
        .get(format!(
            "{}/api/v1/antennas/no_such_antenna",
            server.base_url
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
    assert_json_error(response, "antenna_not_found").await;

    server.shutdown().await;
}

/// Framework path: poem's `Json` extractor rejects an unparseable body before
/// the handler runs, with its own bare `text/plain` 400 and no error code.
/// `ErrorHandler` normalizes it (roadmap C4).
///
/// The useful part of poem's message — the line and column of the syntax error —
/// must survive the rewrite.
#[tokio::test]
async fn malformed_body_400_is_normalized_to_json() {
    let server = TestServer::start().await.unwrap();

    let response = server
        .client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("content-type", "application/json")
        .body("{ not json at all ")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body = assert_json_error(response, "invalid_request_body").await;
    assert!(
        body.message.contains("line") && body.message.contains("column"),
        "the parse location from poem's message must survive normalization, got {:?}",
        body.message
    );

    server.shutdown().await;
}

/// The normalization above must not touch errors the service built itself.
///
/// `service_layer_400_is_json` is the live guard — it asserts a handler-built 400
/// still reports `validation_error`, which would read `invalid_request_body` if
/// the `is_from_response` discriminator ever stopped working. This test states
/// the invariant explicitly so the coupling is not accidental.
#[tokio::test]
async fn normalization_does_not_rewrite_our_own_400() {
    let server = TestServer::start().await.unwrap();

    let oversized = BatchGainRequest {
        evaluations: vec![builders::simple_gain_request_ecef(); 1001],
    };

    let response = server
        .client
        .post(format!("{}/api/v1/gain/batch", server.base_url))
        .json(&oversized)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body: ErrorResponse = response.json().await.unwrap();
    assert_eq!(
        body.error, "validation_error",
        "a handler-built 400 must keep its own code, not be rewritten as a body-parse failure"
    );

    server.shutdown().await;
}

/// Middleware path: `RequestSizeTracker` rejects before any handler runs, so it
/// builds its own error. It has to go through the same helper — this is the
/// site most likely to drift back to `text/plain`, since it lives nowhere near
/// the handlers.
#[tokio::test]
async fn middleware_413_is_json() {
    let server = TestServer::start_with_config(Some(small_body_limit_config(256)))
        .await
        .unwrap();

    let request = builders::simple_gain_request_ecef();
    let body = serde_json::to_string(&request).unwrap();
    assert!(
        body.len() > 256,
        "test precondition: a gain request ({} bytes) must exceed the 256-byte limit",
        body.len()
    );

    let response = server
        .client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 413);
    assert_json_error(response, "payload_too_large").await;

    server.shutdown().await;
}
