//! Guards that every example response in `examples/responses/` deserializes
//! into its documented schema type — the response-side sibling of G3's
//! request guard (`example_requests_deserialize.rs`).
//!
//! Landed BEFORE C8 stage 1's response-field renames
//! (`GeometryInfo::feed_offset_meters` → `physical_feed_offset_m`,
//! `FeedInfo::position_offset` → `design_feed_offset_m`) so this guard
//! catches that pass's misses rather than merely ratifying whatever survived
//! it — same reasoning the repo used to land C11 ahead of C8.

use antenna_model::api::schemas::{
    AntennaDetailsResponse, AntennaListResponse, BatchGainResponse, ErrorResponse, GainResponse,
    HealthResponse, HeatmapResponse, StatusResponse,
};
use std::path::Path;

fn assert_parses<T: serde::de::DeserializeOwned>(path: &Path) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    if let Err(e) = serde_json::from_str::<T>(&text) {
        panic!(
            "{} did not deserialize into {}: {e}",
            path.display(),
            std::any::type_name::<T>()
        );
    }
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
