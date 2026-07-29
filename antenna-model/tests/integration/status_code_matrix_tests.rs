//! One status-code policy for validation failures, on every compute endpoint
//! (roadmap unit C2).
//!
//! The policy, decided 2026-07-24:
//!
//! - **400** — the request body could not be parsed. Nothing else.
//! - **422** — the body parsed but is semantically invalid, whether the pre-check
//!   catches it or it surfaces from the service layer.
//! - **404** — the body names an antenna or feed that does not exist.
//!
//! Before C2 the same four inputs produced six different answers across the four
//! endpoints: `/gain` and `/heatmap` answered **422** for an unknown antenna where
//! `/h3-heatmap` answered **404**; a service-layer `InvalidCoordinate` was **400**
//! on all three; and `/gain/batch` rejected nothing at all — it returned **200**
//! with `"gain_db": null` per bad item.
//!
//! This file is the net. The matrix below runs every (endpoint × case) pair, so a
//! fix applied to three handlers and forgotten on the fourth fails here rather
//! than shipping.
//!
//! Media type and body shape are C4's contract and are pinned in
//! `error_content_type_tests`; this file asserts only the status and the machine
//! `error` code.

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use serde_json::{json, Value};

/// A compute endpoint, with a valid request body to mutate.
struct Endpoint {
    path: &'static str,
    valid: Value,
    /// `/gain/batch` nests the antenna/feed/frequency fields one level down, inside
    /// `evaluations[0]`, so mutations have to descend.
    nested: bool,
}

impl Endpoint {
    /// The valid body with one field replaced — inside `evaluations[0]` for batch.
    fn with(&self, key: &str, value: Value) -> Value {
        let mut body = self.valid.clone();
        if self.nested {
            body["evaluations"][0][key] = value;
        } else {
            body[key] = value;
        }
        body
    }
}

/// All four compute endpoints, each with a body that succeeds unmodified.
///
/// Built from the shared typed builders so a schema change breaks compilation here
/// rather than silently turning every case into a body-parse failure — which would
/// make the whole matrix pass for the wrong reason.
fn compute_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            path: "/api/v1/gain",
            valid: serde_json::to_value(builders::simple_gain_request_ecef()).unwrap(),
            nested: false,
        },
        Endpoint {
            path: "/api/v1/gain/batch",
            valid: serde_json::to_value(builders::simple_batch_request(2)).unwrap(),
            nested: true,
        },
        Endpoint {
            path: "/api/v1/heatmap",
            valid: serde_json::to_value(builders::simple_heatmap_request()).unwrap(),
            nested: false,
        },
        Endpoint {
            path: "/api/v1/h3-heatmap",
            valid: json!({
                "antenna_id": "test_simple",
                "feed_id": "primary",
                "vehicle_position": {"x": -118.1234, "y": 34.5678, "z": 100.0, "coordinate_system": "geodetic"},
                "reflector_boresight": {"x": -118.1234, "y": 34.5679, "z": 110.0, "coordinate_system": "geodetic"},
                "feed_pointing_location": {"x": -118.124, "y": 34.568, "z": 105.0, "coordinate_system": "geodetic"},
                "frequency_mhz": 8400.0,
                "n_rings": 1,
                "h3_resolution": 7
            }),
            nested: false,
        },
    ]
}

/// POST a raw body and assert the status and `error` code, naming the endpoint and
/// case in every failure message — with four endpoints and four cases, a bare
/// `assertion failed` would not say which cell broke.
///
/// Returns the parsed [`ErrorResponse`] so a caller that also cares about the
/// *message* (not just the code) can assert on the specific field it means, without
/// posting the request twice. Deliberately not the raw body: a `raw.contains(..)`
/// check matches anywhere in the serialized JSON, so it would keep passing if the
/// text it looks for drifted from `message` into `field` or `details`.
async fn assert_rejected(
    server: &TestServer,
    path: &str,
    case: &str,
    body: String,
    expected_status: u16,
    expected_code: &str,
) -> ErrorResponse {
    let response = server
        .client
        .post(format!("{}{}", server.base_url, path))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    let status = response.status().as_u16();
    let raw = response.text().await.expect("readable body");

    assert_eq!(
        status, expected_status,
        "{path} [{case}]: expected {expected_status}, got {status}; body: {raw}"
    );

    let parsed: ErrorResponse = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{path} [{case}]: body is not an ErrorResponse ({e}): {raw}"));
    assert_eq!(
        parsed.error, expected_code,
        "{path} [{case}]: wrong error code; body: {raw}"
    );

    parsed
}

// ============================================================================
// The matrix: endpoint × {malformed, invalid, unknown antenna, unknown feed}
// ============================================================================

/// An unparseable body is the *only* thing that earns a 400.
#[tokio::test]
async fn malformed_body_is_400_everywhere() {
    let server = TestServer::start().await.unwrap();

    for endpoint in compute_endpoints() {
        assert_rejected(
            &server,
            endpoint.path,
            "malformed",
            "{ not json at all ".to_string(),
            400,
            "invalid_request_body",
        )
        .await;
    }

    server.shutdown().await;
}

/// A body that parses but carries an out-of-range value is 422 — not 400.
#[tokio::test]
async fn semantically_invalid_body_is_422_everywhere() {
    let server = TestServer::start().await.unwrap();

    for endpoint in compute_endpoints() {
        let body = endpoint.with("frequency_mhz", json!(0.0));
        assert_rejected(
            &server,
            endpoint.path,
            "frequency_mhz = 0",
            body.to_string(),
            422,
            "validation_error",
        )
        .await;
    }

    server.shutdown().await;
}

/// A named antenna that does not exist is absent, not invalid: 404.
#[tokio::test]
async fn unknown_antenna_is_404_everywhere() {
    let server = TestServer::start().await.unwrap();

    for endpoint in compute_endpoints() {
        let body = endpoint.with("antenna_id", json!("no_such_antenna"));
        assert_rejected(
            &server,
            endpoint.path,
            "unknown antenna_id",
            body.to_string(),
            404,
            "antenna_not_found",
        )
        .await;
    }

    server.shutdown().await;
}

/// The antenna exists but the feed does not: also 404, with the narrower code.
#[tokio::test]
async fn unknown_feed_is_404_everywhere() {
    let server = TestServer::start().await.unwrap();

    for endpoint in compute_endpoints() {
        let body = endpoint.with("feed_id", json!("no_such_feed"));
        assert_rejected(
            &server,
            endpoint.path,
            "unknown feed_id",
            body.to_string(),
            404,
            "feed_not_found",
        )
        .await;
    }

    server.shutdown().await;
}

// ============================================================================
// Service-layer semantic failures take the same 422 as the pre-check
// ============================================================================

/// A boresight coincident with the vehicle position (< 1 mm separation) leaves the
/// antenna Z-axis undefined. The pre-check cannot see it — each position is
/// individually well-formed, and only their *relationship* is degenerate — so it
/// surfaces from the coordinate transform as `InvalidCoordinate`.
///
/// This is the reachable half of C2's call (A): before the fix this was **400**,
/// which said "your body was unreadable" about a body that read perfectly.
#[tokio::test]
async fn degenerate_boresight_from_the_service_layer_is_422() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    // Boresight exactly at the vehicle: zero-length pointing vector.
    request.reflector_boresight = request.vehicle_position.clone();

    assert_rejected(
        &server,
        "/api/v1/gain",
        "boresight == vehicle_position",
        serde_json::to_string(&request).unwrap(),
        422,
        "invalid_coordinate",
    )
    .await;

    server.shutdown().await;
}

/// The request-side mirror of the null-gain hazard, and the one case where a
/// semantically-invalid *value* legitimately produces 400 rather than 422.
///
/// JSON has no encoding for `NaN` or `±Infinity`, and `serde_json` emits `null` for
/// them rather than failing. So a client that puts a non-finite float in a request
/// body sends `"x": null`, which cannot deserialize into the non-optional `f64` the
/// schema declares — the body is genuinely unparseable, and 400 is the honest answer.
///
/// The validator's own non-finite checks are therefore unreachable over HTTP; they
/// still guard the in-process service API, which is not restricted to JSON.
#[tokio::test]
async fn non_finite_request_value_is_a_parse_failure() {
    let server = TestServer::start().await.unwrap();

    let mut request = builders::simple_gain_request_ecef();
    request.emitter_position.x = f64::INFINITY;

    assert_rejected(
        &server,
        "/api/v1/gain",
        "emitter_position.x = inf",
        serde_json::to_string(&request).unwrap(),
        400,
        "invalid_request_body",
    )
    .await;

    server.shutdown().await;
}

// ============================================================================
// Batch: whole-request rejection, no null gains for validation-class failures
// ============================================================================

/// C2 call (C): a batch with one bad item is rejected whole, and the response says
/// *which* item — a client cannot fix `evaluations[1]` it cannot locate.
#[tokio::test]
async fn batch_rejection_names_the_failing_item_index() {
    let server = TestServer::start().await.unwrap();

    let mut batch = builders::simple_batch_request(3);
    batch.evaluations[1].antenna_id = "no_such_antenna".to_string();

    let response = server
        .client
        .post(format!("{}/api/v1/gain/batch", server.base_url))
        .json(&batch)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
    let body: ErrorResponse = response.json().await.unwrap();
    assert_eq!(body.error, "antenna_not_found");
    assert_eq!(
        body.field.as_deref(),
        Some("evaluations[1]"),
        "the rejection must name the offending item, got field={:?} message={:?}",
        body.field,
        body.message
    );
    assert!(
        body.message.contains("no_such_antenna"),
        "the message must name the offending antenna, got {:?}",
        body.message
    );

    server.shutdown().await;
}

/// The silent-null hazard, stated as a test.
///
/// `GainResponse::gain_db` is `f64`, and `serde_json` renders `f64::NAN` as
/// **`null`** rather than failing. So before C2 a batch item that referenced a
/// nonexistent antenna came back under **HTTP 200** as `"gain_db": null` with the
/// reason buried in a per-item `warnings` string — a client checking the status
/// code saw success, and one deserializing into a non-optional float got a parse
/// error it had no way to anticipate.
///
/// Validation-class failures must now be rejected before compute, so no `null`
/// can reach the wire for them. Per-item degradation survives only for
/// *compute*-class failures (over-budget, non-convergence), which C8 stage 3
/// reshapes into an explicit `{code, message}`.
#[tokio::test]
async fn batch_never_returns_a_null_gain_for_a_validation_failure() {
    let server = TestServer::start().await.unwrap();

    let cases: Vec<(&str, BatchGainRequest)> = vec![
        ("unknown antenna", {
            let mut b = builders::simple_batch_request(2);
            b.evaluations[0].antenna_id = "no_such_antenna".to_string();
            b
        }),
        ("unknown feed", {
            let mut b = builders::simple_batch_request(2);
            b.evaluations[1].feed_id = "no_such_feed".to_string();
            b
        }),
        ("invalid frequency", {
            let mut b = builders::simple_batch_request(2);
            b.evaluations[0].frequency_mhz = 0.0;
            b
        }),
        ("out-of-range coordinate", {
            let mut b = builders::simple_batch_request(2);
            // Beyond the 400,000 km ECEF ceiling. Note this has to be a *finite*
            // out-of-range value: see `non_finite_request_value_is_a_parse_failure`
            // for why an infinity cannot reach the validator at all.
            b.evaluations[1].emitter_position.x = 5.0e8;
            b
        }),
    ];

    for (case, batch) in cases {
        let response = server
            .client
            .post(format!("{}/api/v1/gain/batch", server.base_url))
            .json(&batch)
            .send()
            .await
            .unwrap();

        let status = response.status().as_u16();
        let raw = response.text().await.expect("readable body");

        assert!(
            status == 404 || status == 422,
            "batch [{case}]: a validation-class failure must be rejected \
             (404/422), got {status}; body: {raw}"
        );
        assert!(
            !raw.contains("\"gain_db\":null"),
            "batch [{case}]: response carries a null gain, which is a NaN \
             serialized as JSON null; body: {raw}"
        );
    }

    server.shutdown().await;
}

/// Control: batch-level constraints (empty, over the 1000-item limit) are
/// semantically invalid, not unparseable, so they are 422 like everything else in
/// that class. Before C2 both were 400.
#[tokio::test]
async fn batch_level_constraints_are_422() {
    let server = TestServer::start().await.unwrap();

    for (case, batch) in [
        (
            "empty",
            BatchGainRequest {
                evaluations: vec![],
            },
        ),
        ("oversized", builders::simple_batch_request(1001)),
    ] {
        let response = server
            .client
            .post(format!("{}/api/v1/gain/batch", server.base_url))
            .json(&batch)
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            422,
            "batch [{case}] should be 422; body: {}",
            response.text().await.unwrap_or_default()
        );
    }

    server.shutdown().await;
}

/// Control: a valid batch is untouched by the new pre-check. Cheap, and it is the
/// assertion that would catch a pre-check whose existence lookup disagrees with the
/// one the evaluator performs — which would reject every batch as 404.
#[tokio::test]
async fn valid_batch_still_succeeds() {
    let server = TestServer::start().await.unwrap();

    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &builders::simple_batch_request(3))
        .await
        .expect("a valid batch must still be served");

    assert_eq!(response.results.len(), 3);
    for (i, result) in response.results.iter().enumerate() {
        assert!(
            result.gain_db.is_finite(),
            "item {i} should have a real gain, got {}",
            result.gain_db
        );
    }

    server.shutdown().await;
}

// ============================================================================
// C8 stage 1: the aim-point rename is a clean break, on every request type
// ============================================================================

/// C8 stage 1 renamed the aim-point field `feed_position` → `feed_pointing_location`
/// as a **clean break** — no serde alias, no deprecation shim. A body using the old
/// key is therefore missing a required field, i.e. unparseable, i.e. 400 under C2's
/// policy, with a message naming the field the client should have sent instead.
///
/// Run across the whole matrix, not just `/gain`, because the field is declared
/// independently on `GainRequest`, `HeatmapRequest` and `H3LinkBudgetRequest`. A
/// `#[serde(alias = "feed_position")]` added to any one of them is precisely the
/// well-meaning reintroduction of backwards compatibility that the C8 decision
/// rejected, and a single-endpoint guard would wave two thirds of it through.
///
/// Each body is the endpoint's own *valid* body with that one key renamed, per the
/// rule at the top of this file: the legacy key must be the only difference. A
/// hand-rolled literal would assert 400 + `invalid_request_body` against a body that
/// could equally have gone stale for an unrelated reason — the right answer for the
/// wrong cause, and the failure mode this file exists to avoid.
#[tokio::test]
async fn legacy_feed_position_key_is_rejected_with_400() {
    let server = TestServer::start().await.unwrap();

    for endpoint in compute_endpoints() {
        let mut body = endpoint.valid.clone();
        let target = if endpoint.nested {
            &mut body["evaluations"][0]
        } else {
            &mut body
        };
        let fields = target
            .as_object_mut()
            .unwrap_or_else(|| panic!("{}: request body must be a JSON object", endpoint.path));
        let aim = fields.remove("feed_pointing_location").unwrap_or_else(|| {
            panic!(
                "{}: the valid body has no `feed_pointing_location` key — the aim-point \
                 field was renamed again and this guard is now testing nothing",
                endpoint.path
            )
        });
        fields.insert("feed_position".to_string(), aim);

        let error = assert_rejected(
            &server,
            endpoint.path,
            "legacy feed_position key",
            body.to_string(),
            400,
            "invalid_request_body",
        )
        .await;

        assert!(
            error.message.contains("feed_pointing_location"),
            "{}: the 400 must name the field the client should send, got message: {}",
            endpoint.path,
            error.message
        );
    }

    server.shutdown().await;
}

/// Every `Position3D` field on the request body, per endpoint. `/gain` is the only
/// one carrying `emitter_position`; the other three derive their emitters from the
/// grid.
fn position_fields(path: &str) -> &'static [&'static str] {
    match path {
        "/api/v1/gain" | "/api/v1/gain/batch" => &[
            "vehicle_position",
            "reflector_boresight",
            "feed_pointing_location",
            "emitter_position",
        ],
        _ => &[
            "vehicle_position",
            "reflector_boresight",
            "feed_pointing_location",
        ],
    }
}

/// C8 stage 2 made `Position3D.coordinate_system` **required**, deleting the
/// magnitude-based auto-detection that guessed the frame. An untagged position is
/// therefore a missing required field — unparseable, so 400 under C2's policy, with a
/// message naming the field.
///
/// Stripped one position at a time, on every endpoint, because each request type
/// declares its positions independently: a `#[serde(default)]` or a re-added
/// heuristic on any single field would restore the exact silent-misparse hazard this
/// unit removed, and a one-field guard would wave the rest through.
#[tokio::test]
async fn a_position_without_coordinate_system_is_rejected_with_400() {
    let server = TestServer::start().await.unwrap();

    for endpoint in compute_endpoints() {
        for field in position_fields(endpoint.path) {
            let mut body = endpoint.valid.clone();
            let target = if endpoint.nested {
                &mut body["evaluations"][0]
            } else {
                &mut body
            };
            let position = target
                .get_mut(field)
                .and_then(Value::as_object_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "{}: the valid body has no `{field}` object — this guard is \
                         testing nothing",
                        endpoint.path
                    )
                });
            assert!(
                position.remove("coordinate_system").is_some(),
                "{}: the valid body's `{field}` carries no `coordinate_system`, so \
                 removing it changes nothing — the tag stopped being serialized",
                endpoint.path
            );

            let error = assert_rejected(
                &server,
                endpoint.path,
                &format!("{field} without coordinate_system"),
                body.to_string(),
                400,
                "invalid_request_body",
            )
            .await;

            assert!(
                error.message.contains("coordinate_system"),
                "{}: the 400 for an untagged `{field}` must name the missing field, \
                 got message: {}",
                endpoint.path,
                error.message
            );
        }
    }

    server.shutdown().await;
}

/// The reason the tag was made required: a GEO satellite's geodetic altitude
/// (~35,786 km) exceeded the old 6400 km ECEF threshold, so an untagged GEO emitter
/// silently misparsed as a near-Earth-centre ECEF point and returned a confidently
/// wrong gain under HTTP 200. Tagged `geodetic`, the same numbers are now read as
/// intended and the request succeeds.
///
/// This is the acceptance half of the pair above: without it, "reject everything
/// untagged" would pass just as well if tagged GEO input were broken too.
#[tokio::test]
async fn geo_altitude_geodetic_emitter_is_accepted_when_tagged() {
    let server = TestServer::start().await.unwrap();

    let mut body = serde_json::to_value(builders::simple_gain_request_geodetic()).unwrap();
    body["emitter_position"] = json!({
        "x": -118.0,
        "y": 0.0,
        "z": 35_786_000.0,
        "coordinate_system": "geodetic"
    });

    let response = server
        .client
        .post(format!("{}/api/v1/gain", server.base_url))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();

    let status = response.status().as_u16();
    let raw = response.text().await.expect("readable body");
    assert_eq!(
        status, 200,
        "a tagged GEO-altitude geodetic emitter must be accepted, got {status}: {raw}"
    );

    let gain: GainResponse = serde_json::from_str(&raw).expect("a GainResponse");
    assert!(
        gain.gain_db.is_finite(),
        "expected a finite gain for a tagged GEO emitter, got {}",
        gain.gain_db
    );

    server.shutdown().await;
}

// ============================================================================
// C8 stage 4: /heatmap's `h3` grid_type stub is gone
// ============================================================================

/// C8 stage 4 deleted the `GridConfig` variant that used to parse and validate an
/// `h3` grid_type on `POST /api/v1/heatmap` and then fail with `not_implemented` — a
/// stub, never an alternative to the separate, fully-implemented
/// `POST /api/v1/h3-heatmap` endpoint. An `h3` tag is now an unknown serde variant,
/// i.e. an unparseable body: 400 `invalid_request_body` under C2's policy, not the
/// 422/`not_implemented` the stub used to return.
#[tokio::test]
async fn h3_grid_type_on_heatmap_is_rejected_with_400() {
    let server = TestServer::start().await.unwrap();

    let mut body = serde_json::to_value(builders::simple_heatmap_request()).unwrap();
    body["grid_config"] = json!({
        "grid_type": "h3",
        "h3_resolution": 7,
        "center_azimuth_deg": 180.0,
        "center_elevation_deg": 45.0,
        "field_of_view_deg": 30.0
    });

    assert_rejected(
        &server,
        "/api/v1/heatmap",
        "h3 grid_type",
        body.to_string(),
        400,
        "invalid_request_body",
    )
    .await;

    server.shutdown().await;
}
