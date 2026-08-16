//! Contract tests for the integration-test helpers themselves (roadmap D29 item 2).
//!
//! A test helper is not exempt from having its behaviour pinned — it is *less*
//! exempt, because a helper that silently does nothing turns every test built on it
//! into a test that asserts nothing while reporting a pass. That is what
//! [`call_json_with_headers`] used to do with a header pair it could not parse, and
//! it is exactly the rot P13 records: a guard whose power nothing asserts.
//!
//! These drive a **local echo endpoint** rather than the service. What is under test
//! is the helper's construction of the request, so involving the real app would only
//! add ways for the assertion to be satisfied by something else.

use crate::integration::helpers::*;
use poem::{handler, post, web::Json, Route};
use std::collections::BTreeMap;

/// Echoes the request's headers as `{name: [value, ...]}`, preserving duplicates —
/// which is the whole point: append-vs-replace is invisible unless duplicates are
/// observable.
#[handler]
async fn echo_headers(request: &poem::Request) -> Json<BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in request.headers().keys() {
        let values = request
            .headers()
            .get_all(name)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        out.insert(name.as_str().to_string(), values);
    }
    Json(out)
}

fn echo_app() -> impl poem::Endpoint {
    Route::new().at("/echo", post(echo_headers))
}

async fn echoed_headers(headers: &[(&str, &str)]) -> BTreeMap<String, Vec<String>> {
    let (status, _response_headers, body) =
        call_json_with_headers(&echo_app(), "/echo", &serde_json::json!({}), headers).await;
    assert_eq!(status, 200, "echo endpoint must accept the request");
    serde_json::from_slice(&body).expect("echo endpoint returns a header map")
}

/// A caller-supplied header must arrive, exactly once, with the given value.
#[tokio::test]
async fn a_supplied_header_arrives_once() {
    let headers = echoed_headers(&[("x-request-id", "abc-123")]).await;

    assert_eq!(
        headers.get("x-request-id"),
        Some(&vec!["abc-123".to_string()]),
        "the helper must deliver the header it was given"
    );
}

/// **The D29 item 2 defect.** Supplying a header the helper already sets must
/// *override* it, not append a second copy.
///
/// Under the previous implementation (`RequestBuilder::header` per pair, which
/// appends) this returned two `content-type` values — so a test meaning to send
/// `text/plain` in fact sent `application/json` as well, and which one the endpoint
/// honored was left to poem. This assertion fails under that implementation, which
/// is what makes it a guard rather than a description.
#[tokio::test]
async fn a_supplied_header_replaces_the_helpers_own() {
    let headers = echoed_headers(&[("content-type", "text/plain")]).await;

    assert_eq!(
        headers.get("content-type"),
        Some(&vec!["text/plain".to_string()]),
        "overriding content-type must replace the helper's own value, not append to it"
    );
}

/// A header *name* the HTTP grammar rejects must fail the test loudly.
///
/// Before D29 this pair was silently dropped: the request went out without it, and a
/// test asserting the service's response to a malformed header name passed while
/// sending nothing malformed at all.
#[tokio::test]
#[should_panic(expected = "malformed header name")]
async fn a_malformed_header_name_panics() {
    // A space is not a valid `token` character in a field name (RFC 9110 §5.1).
    let _ = echoed_headers(&[("not a header name", "value")]).await;
}

/// As above, for a header *value*: control characters are not permitted, and a
/// dropped value is indistinguishable from a header the service chose to ignore.
#[tokio::test]
#[should_panic(expected = "malformed header value")]
async fn a_malformed_header_value_panics() {
    let _ = echoed_headers(&[("x-request-id", "line-one\nline-two")]).await;
}
