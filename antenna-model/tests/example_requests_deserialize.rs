//! Guards that every example request in `examples/requests/` deserializes into
//! its documented schema type — prevents doc/example drift (roadmap unit G3).

use antenna_model::api::schemas::{BatchGainRequest, GainRequest, HeatmapRequest};
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
fn every_example_request_deserializes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/requests");
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&dir).expect("examples/requests must exist") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        match name.as_str() {
            "batch_request.json" => assert_parses::<BatchGainRequest>(&path),
            "heatmap_request.json" => assert_parses::<HeatmapRequest>(&path),
            // All single-gain examples, including every geo_*.json fixture.
            n if n.starts_with("gain_request") || n.starts_with("geo_") => {
                assert_parses::<GainRequest>(&path)
            }
            other => panic!(
                "no schema mapping for examples/requests/{other} — \
                 add it to every_example_request_deserializes"
            ),
        }
        checked += 1;
    }

    assert!(
        checked >= 9,
        "expected to check all example requests, only saw {checked}"
    );
}
