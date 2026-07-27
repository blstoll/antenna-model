//! Guards that every example response in `examples/responses/` deserializes
//! into its documented schema type — the response-side sibling of G3's
//! request guard (`example_requests_deserialize.rs`).
//!
//! Landed BEFORE C8 stage 1's response-field renames
//! (`GeometryInfo::feed_offset_meters` → `physical_feed_offset_m`,
//! `FeedInfo::position_offset` → `design_feed_offset_m`) so this guard
//! catches that pass's misses rather than merely ratifying whatever survived
//! it — same reasoning the repo used to land C11 ahead of C8.
//!
//! The check is symmetric, not just "does it deserialize": missing required
//! fields fail loudly (serde does that for free), but serde silently accepts
//! extra, undeclared keys — which is exactly the shape of bug a half-applied
//! field rename produces (new name added, old name never deleted). So after
//! deserializing, each example is re-serialized back to a `Value` and every
//! non-null key in the source is required to still be present in the
//! round-trip.
//!
//! Null sources are exempt, and the exemption is load-bearing rather than a
//! convenience: once a JSON `null` has been deserialized into an `Option<T>`,
//! "absent" and "present-as-null" are the same state, so a field carrying
//! `#[serde(skip_serializing_if = "Option::is_none")]` legitimately vanishes
//! on the way out. Asserting on it reports a *declared* field as undeclared —
//! which already cost this repo one correct example, deleted because this
//! test said so. The bug class the guard exists for always leaves a stale key
//! with a real value, never `null`, so nothing is lost by the exemption.

use antenna_model::api::schemas::{
    AntennaDetailsResponse, AntennaListResponse, BatchGainResponse, ErrorResponse, GainResponse,
    HealthResponse, HeatmapResponse, StatusResponse,
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

fn assert_parses<T: serde::de::DeserializeOwned + serde::Serialize>(path: &Path) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let source: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
    let type_name = std::any::type_name::<T>();
    let value: T = serde_json::from_value(source.clone()).unwrap_or_else(|e| {
        panic!(
            "{} did not deserialize into {type_name}: {e}",
            path.display()
        )
    });
    let roundtrip = serde_json::to_value(&value).unwrap_or_else(|e| {
        panic!(
            "{} ({type_name}) failed to re-serialize: {e}",
            path.display()
        )
    });
    assert_no_undeclared_keys(&source, &roundtrip, "", type_name);
}

#[test]
fn every_example_response_deserializes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/responses");
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&dir).expect("examples/responses must exist") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        match name.as_str() {
            "gain_response.json" => assert_parses::<GainResponse>(&path),
            "batch_response.json" => assert_parses::<BatchGainResponse>(&path),
            "heatmap_response.json" => assert_parses::<HeatmapResponse>(&path),
            "antenna_list_response.json" => assert_parses::<AntennaListResponse>(&path),
            "antenna_details_response.json" => assert_parses::<AntennaDetailsResponse>(&path),
            "health_response.json" => assert_parses::<HealthResponse>(&path),
            "status_response.json" => assert_parses::<StatusResponse>(&path),
            "error_response.json" => assert_parses::<ErrorResponse>(&path),
            other => panic!(
                "no schema mapping for examples/responses/{other} — \
                 add it to every_example_response_deserializes"
            ),
        }
        checked += 1;
    }

    assert!(
        checked >= 8,
        "expected to check all example responses, only saw {checked}"
    );
}
