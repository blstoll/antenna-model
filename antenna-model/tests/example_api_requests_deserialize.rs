//! Guards that every example in `examples/api_requests.json` deserializes
//! into its documented schema type (roadmap unit C15).
//!
//! `examples/api_requests.json` is the largest client-visible surface in this
//! repo and, until this guard, nothing ever read it. During C8 stage 1 it
//! drifted silently three separate times — a renamed field that appeared in
//! no task inventory, a missing required `failed_points`/`failure_count`
//! pair, and an undeclared `vehicle_attitude` on two `HeatmapRequest` bodies
//! — and each was caught only because a human reviewer happened to go
//! looking. C8 stage 4 edited the file again (removing two H3 heatmap
//! examples) with no test watching either. This guard exists so unit C7
//! freezes a contract that has actually been checked.
//!
//! The file has the shape
//! `{"examples": {"<name>": {"description": ..., "request"|"response": {...}}}}`.
//! Each named example is deserialized into its documented schema type
//! (missing required fields fail loudly — serde does that for free).
//!
//! This is the sibling of `example_responses_deserialize.rs`, and duplicates
//! (rather than shares) that file's `assert_no_undeclared_keys` helper: the
//! task that introduced this guard was scoped to add a new test file only,
//! not to restructure `example_responses_deserialize.rs` into a shared
//! module, so sharing would mean touching a file explicitly out of scope.
//! The logic below — and the reasoning for it — is copied from there
//! verbatim.
//!
//! The check is symmetric, not just "does it deserialize": missing required
//! fields fail loudly (serde does that for free), but serde silently accepts
//! extra, undeclared keys — which is exactly the shape of bug a half-applied
//! field rename produces (new name added, old name never deleted). So after
//! deserializing, each example body is re-serialized back to a `Value` and
//! every non-null key in the source is required to still be present in the
//! round-trip.
//!
//! Null sources are exempt, and the exemption is load-bearing rather than a
//! convenience: once a JSON `null` has been deserialized into an `Option<T>`,
//! "absent" and "present-as-null" are the same state, so a field carrying
//! `#[serde(skip_serializing_if = "Option::is_none")]` legitimately vanishes
//! on the way out. Asserting on it reports a *declared* field as undeclared —
//! which already cost this repo one correct example, deleted because that
//! test said so (see `example_responses_deserialize.rs`). The bug class the
//! guard exists for always leaves a stale key with a real value, never
//! `null`, so nothing is lost by the exemption.
//!
//! An example name with no schema mapping panics naming the example, rather
//! than being silently skipped — a silent skip is exactly how this file
//! drifted three times without anything noticing.

use antenna_model::api::schemas::{
    AntennaDetailsResponse, AntennaListResponse, BatchGainRequest, BatchGainResponse,
    ErrorResponse, GainRequest, GainResponse, HealthResponse, HeatmapRequest, HeatmapResponse,
    StatusResponse,
};
use serde_json::Value;
use std::path::Path;

/// Recursively asserts that every non-null key present in `source` is still
/// present in `roundtrip` (the same value after deserialize -> re-serialize).
/// A non-null key that disappears was silently accepted by serde on the way in
/// but is not declared by any field of `type_name` on the way out — e.g. a
/// stale key left behind by a half-applied rename, or a field that was never
/// real.
///
/// Keys whose source value is `null` are skipped: see the module doc — a
/// `null` cannot be distinguished from an absent optional after
/// deserialization, so the round-trip has no evidence either way.
fn assert_no_undeclared_keys(
    source: &Value,
    roundtrip: &Value,
    dotted_path: &str,
    type_name: &str,
) {
    match (source, roundtrip) {
        (Value::Object(src_map), Value::Object(rt_map)) => {
            for (key, src_val) in src_map {
                let child_path = if dotted_path.is_empty() {
                    key.clone()
                } else {
                    format!("{dotted_path}.{key}")
                };
                match rt_map.get(key) {
                    Some(rt_val) => {
                        assert_no_undeclared_keys(src_val, rt_val, &child_path, type_name)
                    }
                    // A null source is unfalsifiable by round-trip (module doc).
                    None if src_val.is_null() => {}
                    None => panic!(
                        "{child_path} is present in the example with a non-null value but is \
                         not declared by any field of {type_name} (it disappeared on round-trip \
                         serialization — likely a stale/renamed key)"
                    ),
                }
            }
        }
        (Value::Array(src_arr), Value::Array(rt_arr)) => {
            // Nothing in `schemas.rs` today deserializes to a shorter sequence than
            // it was given, but zip() would silently truncate if something did, and
            // this contract is actively being changed. Fail instead of shortening.
            assert_eq!(
                src_arr.len(),
                rt_arr.len(),
                "{dotted_path} changed length on round-trip for {type_name} \
                 ({} in the example, {} after re-serialization)",
                src_arr.len(),
                rt_arr.len()
            );
            for (i, (s, r)) in src_arr.iter().zip(rt_arr.iter()).enumerate() {
                assert_no_undeclared_keys(s, r, &format!("{dotted_path}[{i}]"), type_name);
            }
        }
        _ => {}
    }
}

/// Deserializes `body` (the `request` or `response` value of one named
/// example) into `T`, then round-trips it through `assert_no_undeclared_keys`.
/// `example_name` is only used to make panic messages actionable.
fn assert_parses<T: serde::de::DeserializeOwned + serde::Serialize>(
    body: &Value,
    example_name: &str,
) {
    let type_name = std::any::type_name::<T>();
    let value: T = serde_json::from_value(body.clone()).unwrap_or_else(|e| {
        panic!("example \"{example_name}\" did not deserialize into {type_name}: {e}")
    });
    let roundtrip = serde_json::to_value(&value).unwrap_or_else(|e| {
        panic!("example \"{example_name}\" ({type_name}) failed to re-serialize: {e}")
    });
    assert_no_undeclared_keys(body, &roundtrip, "", type_name);
}

#[test]
fn every_example_in_api_requests_json_deserializes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/api_requests.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let root: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
    let examples = root
        .get("examples")
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("{} has no top-level \"examples\" object", path.display()));

    // Must not vacuously pass if the file is emptied or the top-level key is
    // renamed out from under this test.
    assert!(
        !examples.is_empty(),
        "{} \"examples\" map is empty — nothing was checked",
        path.display()
    );

    let mut checked = 0usize;
    for (name, example) in examples {
        let body = if let Some(v) = example.get("request") {
            v
        } else if let Some(v) = example.get("response") {
            v
        } else {
            panic!(
                "example \"{name}\" in {} has neither a \"request\" nor a \"response\" key",
                path.display()
            );
        };

        match name.as_str() {
            "gain_request_ecef_quaternion" | "gain_request_geodetic_quaternion" => {
                assert_parses::<GainRequest>(body, name)
            }
            "gain_response" => assert_parses::<GainResponse>(body, name),
            "batch_request" => assert_parses::<BatchGainRequest>(body, name),
            "batch_response" => assert_parses::<BatchGainResponse>(body, name),
            "heatmap_request_rectangular" => assert_parses::<HeatmapRequest>(body, name),
            "heatmap_response_rectangular" => assert_parses::<HeatmapResponse>(body, name),
            "antenna_list_response" => assert_parses::<AntennaListResponse>(body, name),
            "antenna_details_response" => assert_parses::<AntennaDetailsResponse>(body, name),
            "health_response" => assert_parses::<HealthResponse>(body, name),
            "status_response" => assert_parses::<StatusResponse>(body, name),
            // The error_response_* family all share one schema type; matching
            // on the prefix is fine per the task's own carve-out, since the
            // fallback arm below still panics for anything genuinely unmapped.
            n if n.starts_with("error_response_") => assert_parses::<ErrorResponse>(body, name),
            other => panic!(
                "no schema mapping for example \"{other}\" in {} — add it to \
                 every_example_in_api_requests_json_deserializes",
                path.display()
            ),
        }
        checked += 1;
    }

    assert_eq!(
        checked,
        examples.len(),
        "checked {checked} examples but the file has {} — every example must go through the \
         match above",
        examples.len()
    );
    assert!(
        checked >= 16,
        "expected to check all 16 documented examples, only saw {checked}"
    );
}
