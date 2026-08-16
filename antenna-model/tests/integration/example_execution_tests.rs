//! Guards that every example request in `examples/requests/` **computes** —
//! not merely that it deserializes (roadmap unit D30).
//!
//! # Why this exists beside `example_requests_deserialize.rs`
//!
//! G3's guard (`tests/example_requests_deserialize.rs`) checks that each example
//! parses into its schema type. That is a real check and it caught a real break
//! — four examples carrying an object-form `vehicle_attitude` after the schema
//! moved to `[w, x, y, z]`. But it pins **shape**, and nothing pinned
//! **semantics**: `gain_request.json`, the file the README, `examples/README.md`
//! and `examples/TESTING.md` all told a new user to run first, deserialized
//! perfectly and returned **HTTP 422** for two months underneath a green suite.
//! Its ECEF geometry put the attitude's body X-axis parallel to boresight, which
//! makes the azimuth reference degenerate — and the validator is right to reject
//! it.
//!
//! So: shape is checked there, behaviour is checked here. Both are cheap now
//! only because D18 removed reqwest's `system-proxy` feature; before that this
//! guard would have cost ~12 s per `TestServer` in client construction alone.
//!
//! # Two things to preserve when editing this file
//!
//! **It serves the shipped configuration.** `TestServer::start()` loads
//! `antenna-model/tests/fixtures/test_antennas.yaml`, whose antennas
//! (`test_simple`, `test_large`, …) are not the ones the examples name. These
//! examples are what we hand an operator, so they are executed against
//! `calibration_data/antennas.yaml` — which also makes this a standing check
//! that every antenna/feed id an example names is still present and `enabled` in
//! the config we ship.
//!
//! **A 200 is not the assertion for batch.** `/api/v1/gain/batch` returns 200
//! even when every item fails, each carrying a typed `error` and a null gain, so
//! a status-code check calls a fully-failed batch healthy — which is exactly how
//! `batch_request.json` hid. `failure_count` is what must be zero.

use super::helpers::TestServer;
use antenna_model::api::schemas::{
    BatchGainResponse, GainResponse, H3LinkBudgetResponse, HeatmapResponse,
};
use antenna_model::config::ServiceConfig;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Repository root — this crate's manifest dir is `<root>/antenna-model`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// A server loading the **shipped** `calibration_data/antennas.yaml`, so the
/// example payloads run against the antennas an operator actually gets.
async fn shipped_config_server() -> TestServer {
    let mut cfg = ServiceConfig::with_defaults();
    cfg.server.host = "127.0.0.1".to_string();
    cfg.server.port = 0;
    cfg.calibration.data_directory = repo_root().join("calibration_data");
    cfg.calibration.antenna_config_file = repo_root().join("calibration_data/antennas.yaml");
    // The shipped config's enabled entries are uncalibrated design-spec antennas
    // that load without a `.bin` artifact (roadmap D9); the disabled template
    // entries reference files that are deliberately absent.
    cfg.calibration.fail_fast = false;

    TestServer::start_with_config(Some(cfg))
        .await
        .expect("test server with the shipped calibration config")
}

/// The endpoint an example file is meant to be POSTed to.
///
/// Mirrors the name → schema mapping in `example_requests_deserialize.rs`. The
/// `other` arm is a hard error for the same reason it is there: a new example
/// file must force a decision rather than be silently uncovered.
fn endpoint_for(file_name: &str) -> &'static str {
    match file_name {
        "batch_request.json" => "/api/v1/gain/batch",
        "heatmap_request.json" => "/api/v1/heatmap",
        "h3_link_budget_request.json" => "/api/v1/h3-heatmap",
        n if n.starts_with("gain_request") || n.starts_with("geo_") => "/api/v1/gain",
        other => {
            panic!("no endpoint mapping for examples/requests/{other} — add it to endpoint_for")
        }
    }
}

/// Every `examples/requests/*.json`, sorted so failures report in a stable order.
fn example_files() -> Vec<PathBuf> {
    let dir = repo_root().join("examples/requests");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("examples/requests must exist")
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
}

#[tokio::test]
async fn every_example_request_computes_against_the_shipped_config() {
    let server = shipped_config_server().await;
    let files = example_files();
    assert!(
        files.len() >= 10,
        "expected to execute all example requests, only found {}",
        files.len()
    );

    let mut failures: Vec<String> = Vec::new();

    for path in &files {
        let name = path.file_name().unwrap().to_str().unwrap();
        let endpoint = endpoint_for(name);
        let body: Value = serde_json::from_str(
            &std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {name}: {e}")),
        )
        .unwrap_or_else(|e| panic!("{name} is not valid JSON: {e}"));

        let response = server
            .client
            .post(format!("{}{endpoint}", server.base_url))
            .json(&body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST {endpoint} for {name}: {e}"));

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            failures.push(format!(
                "{name} -> POST {endpoint} returned {status}: {}",
                text.trim()
            ));
            continue;
        }

        // A 2xx is where the old guard would have stopped. Look inside.
        if let Some(detail) = per_item_failure(name, endpoint, &text) {
            failures.push(detail);
        }
    }

    // Shut down BEFORE asserting: a panic here would otherwise leak the listener,
    // and the diagnostics are already collected.
    server.shutdown().await;

    assert!(
        failures.is_empty(),
        "these committed examples do not compute against the shipped configuration \
         (they still deserialize, which is all the G3 guard checks):\n  {}",
        failures.join("\n  ")
    );
}

/// The committed **response** examples must describe the request they sit beside,
/// and must carry a physically possible gain.
///
/// Found while fixing the requests (roadmap D30, scope extension): `example_responses_deserialize.rs`
/// pins their shape, so `examples/responses/gain_response.json` shipped
/// `"gain_db": -3370985.117` — three and a half *million* dB — beside a feed offset
/// of −5.0 m on a 34 m dish, which is ten times the ray-tracing scope boundary. Its
/// batch sibling paired that same impossible offset with a peak-gain value. Both
/// predate the P10 integrator and everything since. Shape was checked; nothing
/// asked whether the numbers could exist.
///
/// **What this does not promise.** It is deliberately not a physics pin: the
/// tolerance is "could this number come from an antenna at all", not "is this the
/// value this build computes". Pinning the exact gains would make every legitimate
/// `PHYSICS_MODEL_VERSION` bump edit the documentation, and the docs would then be
/// updated mechanically rather than read. Regenerate them from a running service
/// when they drift far enough to mislead:
///
/// ```bash
/// curl -s -X POST localhost:3000/api/v1/gain \
///   -H 'Content-Type: application/json' \
///   -d @examples/requests/gain_request.json | jq . > examples/responses/gain_response.json
/// ```
#[test]
fn every_response_example_matches_its_request_and_is_physically_possible() {
    // No dish in this tree can plausibly exceed ~90 dBi (GBT 100 m at Q-band is
    // ~89), and nothing real sits below −100 dBi. This is a sanity envelope, not
    // an accuracy claim.
    const MIN_PLAUSIBLE_DBI: f64 = -100.0;
    const MAX_PLAUSIBLE_DBI: f64 = 100.0;

    let read = |rel: &str| -> Value {
        let path = repo_root().join(rel);
        serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}")),
        )
        .unwrap_or_else(|e| panic!("{rel} is not valid JSON: {e}"))
    };

    let check_gain = |what: &str, value: &Value| {
        let gain = value["gain_db"]
            .as_f64()
            .unwrap_or_else(|| panic!("{what}: gain_db is missing or not a number"));
        assert!(
            gain.is_finite() && (MIN_PLAUSIBLE_DBI..=MAX_PLAUSIBLE_DBI).contains(&gain),
            "{what}: gain_db {gain} is not a physically possible antenna gain \
             (expected {MIN_PLAUSIBLE_DBI}..={MAX_PLAUSIBLE_DBI} dBi)"
        );
    };

    // Single gain: identity must match the request it documents.
    let request = read("examples/requests/gain_request.json");
    let response = read("examples/responses/gain_response.json");
    for field in ["antenna_id", "feed_id"] {
        assert_eq!(
            request[field], response[field],
            "gain_response.json documents a different {field} than gain_request.json — \
             a reader following the pair is being shown two different antennas"
        );
    }
    check_gain("gain_response.json", &response);

    // Batch: same, item by item, in order.
    let request = read("examples/requests/batch_request.json");
    let response = read("examples/responses/batch_response.json");
    let items = request["evaluations"]
        .as_array()
        .expect("evaluations array");
    let results = response["results"].as_array().expect("results array");
    assert_eq!(
        items.len(),
        results.len(),
        "batch_response.json has {} results for {} requested evaluations",
        results.len(),
        items.len()
    );
    for (i, (item, result)) in items.iter().zip(results).enumerate() {
        for field in ["antenna_id", "feed_id"] {
            assert_eq!(
                item[field], result[field],
                "batch_response.json item {i} documents a different {field} than \
                 batch_request.json item {i}"
            );
        }
        check_gain(&format!("batch_response.json item {i}"), result);
    }

    // Heatmap: identity only — its grid carries loss, not gain.
    let request = read("examples/requests/heatmap_request.json");
    let response = read("examples/responses/heatmap_response.json");
    for field in ["antenna_id", "feed_id"] {
        assert_eq!(
            request[field], response[field],
            "heatmap_response.json documents a different {field} than heatmap_request.json"
        );
    }
}

/// Negative control: put D30's exact defect back into the committed batch example
/// and require the guard above to catch it.
///
/// Without this, `every_example_request_computes_…` is a test that passes and
/// asserts nothing observable — the failure mode P13 records. It matters most for
/// batch, where the hazard is that HTTP 200 is *not* the signal: this reproduces a
/// fully-failed batch and pins both halves, that the service really does answer 200
/// and that `per_item_failure` really does reject it.
///
/// The defect is re-injected from the live example rather than hand-written, so this
/// control cannot drift away from the file it protects.
#[tokio::test]
async fn a_degenerate_batch_returns_200_and_the_guard_still_rejects_it() {
    let server = shipped_config_server().await;

    let path = repo_root().join("examples/requests/batch_request.json");
    let mut body: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read batch example"))
            .expect("batch example is valid JSON");

    // The identity quaternion against these files' `+X` boresight is precisely what
    // made the azimuth reference degenerate before D30.
    let items = body["evaluations"]
        .as_array_mut()
        .expect("batch example has an evaluations array");
    assert_eq!(items.len(), 3, "control assumes the committed 3-item batch");
    for item in items.iter_mut() {
        item["vehicle_attitude"] = serde_json::json!([1.0, 0.0, 0.0, 0.0]);
    }

    let response = server
        .client
        .post(format!("{}/api/v1/gain/batch", server.base_url))
        .json(&body)
        .send()
        .await
        .expect("POST the degenerate batch");

    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    assert!(
        status.is_success(),
        "the hazard this guard exists for is a *successful* status on a failed batch; \
         if batch has started rejecting this at the HTTP layer, that is a behaviour \
         change to record, not a control to delete (got {status})"
    );

    let caught = per_item_failure("batch_request.json", "/api/v1/gain/batch", &text);
    assert!(
        caught.is_some(),
        "per_item_failure accepted a batch whose every item failed — the guard has no \
         power and would not have caught D30"
    );

    server.shutdown().await;
}

/// Negative control for the `/heatmap` arm, the sibling of the batch control above.
///
/// `/heatmap` does not report a failed grid with a status either: every failed point
/// becomes a `FAILED_POINT_LOSS_DB` sentinel inside a 200, and `failed_points` is the
/// only field that says so.
///
/// **Why this one feeds the checker directly instead of going over HTTP.** The batch
/// control can re-inject its defect through the real service, but the routes to an
/// all-failed grid are not reachable that way: C2 made an unknown `antenna_id`/`feed_id`
/// a **404** at the validator, before `generate_heatmap` runs (measured — an unknown feed
/// returns 404, not a 200 full of sentinels), and the remaining route is per-point budget
/// exhaustion, which is a timing race. `heatmap.rs`'s own unit test reaches the state by
/// calling `generate_heatmap` directly for exactly this reason. So the control asserts
/// what it can actually assert — that the **checker** rejects the shape — and does not
/// pretend to prove the service produces it.
///
/// It also pins *why* the first version of this arm was vacuous, which a bare "the guard
/// rejects this" assertion would not: `points_evaluated` stays **nonzero** on a fully
/// failed grid, because `heatmap.rs` sets it to `grid_points.len()` unconditionally. A
/// control that only checked rejection would have passed against the broken arm too.
#[test]
fn an_all_failed_heatmap_body_is_rejected_by_the_guard() {
    // The shape `generate_heatmap` produces when every point fails: sentinel losses,
    // `failed_points == points_evaluated`, both nonzero. Pinned by
    // `heatmap::tests` (all-failed → `failed_points == points_evaluated`).
    let all_failed = serde_json::json!({
        "antenna_id": "gs_3.7m_uncalibrated",
        "feed_id": "x_band_feed",
        "frequency_mhz": 8250.0,
        "grid": {
            "grid_type": "rectangular",
            "azimuth_values": [0.0, 1.0],
            "elevation_values": [0.0, 1.0],
            "loss_db": [[-999999.0, -999999.0], [-999999.0, -999999.0]]
        },
        "warnings": [],
        "metadata": {
            "points_evaluated": 4,
            "computation_time_ms": 1.0,
            "peak_gain_db": -999999.0,
            "failed_points": 4
        },
        "calibration_status": null
    })
    .to_string();

    let parsed: HeatmapResponse =
        serde_json::from_str(&all_failed).expect("the control body must match the real schema");
    assert!(
        parsed.metadata.points_evaluated > 0,
        "the trap being pinned: points_evaluated stays NONZERO on a fully failed grid, \
         which is why it cannot be the success signal"
    );
    assert_eq!(
        parsed.metadata.failed_points, parsed.metadata.points_evaluated,
        "control assumes every grid point failed"
    );

    let caught = per_item_failure("heatmap_request.json", "/api/v1/heatmap", &all_failed);
    assert!(
        caught.is_some(),
        "per_item_failure accepted a heatmap whose every point failed — the guard has \
         no power over this endpoint"
    );
}

/// Inspect a 2xx body for the failure modes a status code cannot show.
///
/// Returns `Some(description)` when the response is a success by status and a
/// failure in substance.
fn per_item_failure(name: &str, endpoint: &str, text: &str) -> Option<String> {
    match endpoint {
        "/api/v1/gain/batch" => {
            let parsed: BatchGainResponse = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("{name}: batch response did not parse: {e}"));

            if parsed.metadata.failure_count > 0 {
                let reasons: Vec<String> = parsed
                    .results
                    .iter()
                    .enumerate()
                    .filter_map(|(i, r)| {
                        r.error
                            .as_ref()
                            .map(|e| format!("item {i}: {:?} {}", e.code, e.message))
                    })
                    .collect();
                return Some(format!(
                    "{name} -> HTTP 200 with failure_count {} of {} — {}",
                    parsed.metadata.failure_count,
                    parsed.metadata.count,
                    reasons.join("; ")
                ));
            }
            // failure_count counts NaN gains; assert the items agree with it.
            for (i, result) in parsed.results.iter().enumerate() {
                if !result.gain_db.is_finite() {
                    return Some(format!(
                        "{name} -> HTTP 200, failure_count 0, but item {i} gain_db is not finite"
                    ));
                }
            }
            None
        }
        "/api/v1/gain" => {
            let parsed: GainResponse = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("{name}: gain response did not parse: {e}"));
            (!parsed.gain_db.is_finite())
                .then(|| format!("{name} -> HTTP 200 with a non-finite gain_db"))
        }
        "/api/v1/h3-heatmap" => {
            let parsed: H3LinkBudgetResponse = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("{name}: h3 response did not parse: {e}"));
            parsed
                .cells
                .is_empty()
                .then(|| format!("{name} -> HTTP 200 with no cells"))
        }
        // `/heatmap`'s per-point failures surface as the FAILED_POINT_LOSS_DB
        // sentinel, never as a status — so `failed_points` is this endpoint's
        // `failure_count`, and it is the only field worth reading.
        //
        // Do NOT go back to `points_evaluated`: `heatmap.rs` sets it to
        // `grid_points.len()` unconditionally, so it is a grid-size readout and
        // not an outcome at all. An all-failed heatmap reports
        // `failed_points == points_evaluated`, both nonzero (pinned by that
        // module's own unit test), so a `points_evaluated > 0` check passes a
        // grid of nothing but sentinels — reproducing the batch hazard this
        // guard exists for, one arm over. It did exactly that until D30's review
        // caught it.
        "/api/v1/heatmap" => {
            let parsed: HeatmapResponse = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("{name}: heatmap response did not parse: {e}"));
            (parsed.metadata.failed_points > 0).then(|| {
                format!(
                    "{name} -> HTTP 200 with {} of {} grid points failed",
                    parsed.metadata.failed_points, parsed.metadata.points_evaluated
                )
            })
        }
        other => panic!("no response check for {other} — add one to per_item_failure"),
    }
}
