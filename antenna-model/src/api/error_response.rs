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
//! The error-code vocabulary itself lives in [`crate::api::schemas::ErrorCode`]
//! (roadmap unit C3; promoted from `&str` consts to a closed enum by C7); this
//! module owns the transport, not the taxonomy.
//!
//! # Status policy (roadmap unit C2)
//!
//! [`validation_status`] and [`service_status`] are the **only** two places that
//! decide which HTTP status a rejection gets. Before C2 each of the four compute
//! handlers carried its own hand-written `match` over the service error type, and
//! they had drifted: `/gain`'s had no `Validation(_)` arm at all and fell through to
//! `500`, while `/heatmap`'s and `/h3-heatmap`'s answered `400`. Centralizing the
//! decision means a handler cannot disagree with its siblings, and adding an error
//! variant is one edit rather than four.

use crate::api::schemas::{ErrorCode, ErrorResponse};
use crate::error::{
    AntennaModelError, BatchValidationError, ComputationError, DataError, ValidationError,
};
use poem::http::StatusCode;
use poem::Response;

/// Body used when serializing an [`ErrorResponse`] fails.
///
/// Unreachable in practice: `ErrorResponse` is a unit enum plus owned `String`
/// fields, and `serde_json` cannot fail on those. It exists because the alternative to a
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

/// Status and error code for a request rejected by validation (roadmap unit C2).
///
/// Two outcomes:
///
/// - **404** when the request names an antenna or feed that does not exist. Absence
///   is not invalidity: no change to the request body will make a missing antenna
///   appear, so `422 Unprocessable Entity` would send the client looking for a
///   parameter to fix.
/// - **422** for everything else — the body parsed, but a value is out of range,
///   non-finite, or otherwise unusable.
///
/// Never `400`. Under C2 a `400` means only that the body could not be parsed, which
/// is decided before any validator runs.
pub fn validation_status(err: &ValidationError) -> (StatusCode, ErrorCode) {
    match err {
        ValidationError::AntennaNotFound { .. } => {
            (StatusCode::NOT_FOUND, ErrorCode::AntennaNotFound)
        }
        ValidationError::FeedNotFound { .. } => (StatusCode::NOT_FOUND, ErrorCode::FeedNotFound),
        _ => (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::ValidationError),
    }
}

/// Status and error code for an error that surfaced from the service layer.
///
/// The same policy as [`validation_status`], applied one layer deeper: a semantic
/// failure the pre-check could not have caught is still a **422**, and a resource
/// that turns out to be absent is still a **404**. A client should not be able to
/// tell which layer noticed.
///
/// Notable mappings:
///
/// - `Validation(_)` delegates to [`validation_status`], so an unknown antenna named
///   in a nested request member gets the same 404 it would get at the top level.
///   Before C2 `/gain` had no arm for this case at all and served it as a **500**.
/// - `InvalidCoordinate` is **422**, not the pre-C2 **400**. It is raised for bodies
///   that parsed perfectly — the reachable case is a `reflector_boresight`
///   coincident with `vehicle_position`, which leaves the antenna Z-axis undefined.
///   Calling that "bad request syntax" was simply wrong.
/// - `CoordinateTransformError` stays **500**: unlike `InvalidCoordinate` it signals
///   a transform that failed on input already accepted as valid, which is our bug,
///   not the caller's.
/// - `TimeBudgetExceeded` keeps its **504**; see the `RequestTimeout` middleware for
///   the 504-vs-408-vs-503 reasoning.
pub fn service_status(err: &AntennaModelError) -> (StatusCode, ErrorCode) {
    match err {
        AntennaModelError::Validation(inner) => validation_status(inner),

        AntennaModelError::FeedNotFound { .. } => (StatusCode::NOT_FOUND, ErrorCode::FeedNotFound),
        AntennaModelError::Data(DataError::AntennaNotFound { .. }) => {
            (StatusCode::NOT_FOUND, ErrorCode::AntennaNotFound)
        }

        AntennaModelError::InvalidCoordinate { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::InvalidCoordinate,
        ),
        AntennaModelError::Computation(ComputationError::TimeBudgetExceeded { .. }) => (
            StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::ComputationBudgetExceeded,
        ),

        _ => (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError),
    }
}

/// Build the HTTP rejection for a validation failure, optionally naming the field.
///
/// `field` is the JSON path of the offending request member when one can be named —
/// `"evaluations[3]"` for a batch item. It is omitted when the failure is not
/// attributable to a single member; the message already carries the parameter name in
/// that case.
pub fn validation_error(err: &ValidationError, field: Option<&str>) -> poem::Error {
    let (status, code) = validation_status(err);
    let mut body = ErrorResponse::new(code, err.to_string());
    if let Some(field) = field {
        body = body.with_field(field);
    }
    json_error(status, &body)
}

/// Build the HTTP rejection for a batch pre-check failure (roadmap unit C2, call C).
///
/// Classified on the *inner* error, so an absent antenna named inside
/// `evaluations[3]` is a 404 exactly as it would be at the top level of
/// `/api/v1/gain`. The message keeps the `evaluations[3]:` prefix and `field` carries
/// the same path — one for a human reading the response, one for a client that wants
/// to locate the item without parsing prose.
pub fn batch_validation_error(err: &BatchValidationError) -> poem::Error {
    let (status, code) = validation_status(err.inner());
    json_error(
        status,
        &ErrorResponse::new(code, err.to_string()).with_field(err.field()),
    )
}

/// Build the HTTP rejection for an error that surfaced from the service layer.
pub fn service_error(err: &AntennaModelError) -> poem::Error {
    let (status, code) = service_status(err);
    json_error(status, &ErrorResponse::new(code, err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn carries_the_json_content_type_and_the_exact_serialized_body() {
        let body = ErrorResponse::new(ErrorCode::ValidationError, "frequency out of range")
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
        let body = ErrorResponse::new(ErrorCode::InternalError, "boom");
        assert_eq!(
            json_error(StatusCode::INTERNAL_SERVER_ERROR, &body).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // ========================================================================
    // Status policy (roadmap unit C2)
    // ========================================================================

    /// The whole C2 policy as one table.
    ///
    /// This is the pin that makes the policy auditable: every rejection class the
    /// service can produce, with the status and code it must carry. The four
    /// handlers used to encode this four times with their own `match` arms, and had
    /// drifted apart — `/gain` served a service-layer `Validation(_)` as 500 because
    /// it simply had no arm for it.
    #[test]
    fn service_status_policy_table() {
        use crate::error::ValidationError;

        let cases: Vec<(&str, AntennaModelError, StatusCode, ErrorCode)> = vec![
            (
                // The 500-fallthrough that C2 fixes: no `Validation(_)` arm existed
                // on `/gain`, so this class fell through to INTERNAL_SERVER_ERROR.
                "service-layer validation failure",
                AntennaModelError::Validation(ValidationError::FrequencyOutOfRange {
                    frequency_mhz: 0.0,
                }),
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::ValidationError,
            ),
            (
                // Absence stays absence even when it surfaces one layer down.
                "service-layer unknown antenna",
                AntennaModelError::Validation(ValidationError::AntennaNotFound {
                    antenna_id: "nope".into(),
                }),
                StatusCode::NOT_FOUND,
                ErrorCode::AntennaNotFound,
            ),
            (
                "service-layer unknown feed",
                AntennaModelError::Validation(ValidationError::FeedNotFound {
                    antenna_id: "a".into(),
                    feed_id: "nope".into(),
                    available: vec!["primary".into()],
                }),
                StatusCode::NOT_FOUND,
                ErrorCode::FeedNotFound,
            ),
            (
                "evaluator feed lookup miss",
                AntennaModelError::FeedNotFound {
                    antenna_id: "a".into(),
                    feed_id: "f".into(),
                },
                StatusCode::NOT_FOUND,
                ErrorCode::FeedNotFound,
            ),
            (
                "repository antenna miss",
                AntennaModelError::Data(DataError::AntennaNotFound {
                    antenna_id: "a".into(),
                }),
                StatusCode::NOT_FOUND,
                ErrorCode::AntennaNotFound,
            ),
            (
                // Was 400 before C2, on a body that parsed perfectly.
                "degenerate geometry from the coordinate transform",
                AntennaModelError::InvalidCoordinate {
                    param: "reflector_boresight".into(),
                    reason: "too close to vehicle_position".into(),
                },
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::InvalidCoordinate,
            ),
            (
                "integration over budget",
                AntennaModelError::Computation(ComputationError::TimeBudgetExceeded {
                    operation: "azimuthal_mode_field".into(),
                    elapsed_ms: 31_000.0,
                    budget_ms: 30_000,
                }),
                StatusCode::GATEWAY_TIMEOUT,
                ErrorCode::ComputationBudgetExceeded,
            ),
            (
                // Distinct from InvalidCoordinate: the input was already accepted as
                // valid, so a failure here is ours.
                "coordinate transform failure",
                AntennaModelError::CoordinateTransformError {
                    details: "singular".into(),
                },
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalError,
            ),
            (
                "numerical instability",
                AntennaModelError::Computation(ComputationError::NumericalInstability {
                    operation: "integration".into(),
                    reason: "overflow".into(),
                }),
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalError,
            ),
        ];

        for (case, err, expected_status, expected_code) in cases {
            let (status, code) = service_status(&err);
            assert_eq!(status, expected_status, "wrong status for {case}: {err}");
            assert_eq!(code, expected_code, "wrong error code for {case}: {err}");
        }
    }

    /// No rejection the service builds may be a 400. Under C2 that status means
    /// exactly one thing — an unparseable body — and it is decided by the `Json`
    /// extractor before any of this code runs.
    #[test]
    fn the_service_never_answers_400_from_a_typed_error() {
        use crate::error::ValidationError;

        let errors = [
            AntennaModelError::Validation(ValidationError::FrequencyOutOfRange {
                frequency_mhz: 1e9,
            }),
            AntennaModelError::InvalidCoordinate {
                param: "p".into(),
                reason: "r".into(),
            },
            AntennaModelError::CoordinateTransformError {
                details: "d".into(),
            },
            AntennaModelError::Generic("boom".into()),
        ];

        for err in errors {
            let (status, _) = service_status(&err);
            assert_ne!(
                status,
                StatusCode::BAD_REQUEST,
                "400 is reserved for unparseable bodies, but {err} mapped to it"
            );
        }
    }

    /// Anything parameter-shaped defaults to 422, including variants added later
    /// without a deliberate decision in `validation_status`.
    #[test]
    fn validation_status_defaults_to_422_and_singles_out_absence() {
        use crate::error::ValidationError;

        assert_eq!(
            validation_status(&ValidationError::BatchSizeLimitExceeded {
                size: 1001,
                limit: 1000
            }),
            (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::ValidationError)
        );
        assert_eq!(
            validation_status(&ValidationError::AntennaNotFound {
                antenna_id: "a".into()
            }),
            (StatusCode::NOT_FOUND, ErrorCode::AntennaNotFound)
        );
    }

    /// `field` reaches the wire, so a batch client can locate the item to fix.
    #[test]
    fn validation_error_carries_the_field_path() {
        use crate::error::ValidationError;

        let err = validation_error(
            &ValidationError::AntennaNotFound {
                antenna_id: "nope".into(),
            },
            Some("evaluations[3]"),
        );
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert!(
            err.to_string().contains("evaluations[3]"),
            "expected the field path in the body, got {err}"
        );
    }

    /// The request logger and error handler log `error = %err`; an empty
    /// `Display` would degrade those lines to the bare status text.
    #[test]
    fn display_is_the_json_payload_not_the_bare_status() {
        let body = ErrorResponse::new(ErrorCode::AntennaNotFound, "Antenna 'x' not found");
        let rendered = json_error(StatusCode::NOT_FOUND, &body).to_string();

        assert!(
            rendered.contains(ErrorCode::AntennaNotFound.as_str()),
            "expected the payload in Display, got {rendered:?}"
        );
        assert!(
            rendered.contains("Antenna 'x' not found"),
            "expected the message in Display, got {rendered:?}"
        );
    }
}
