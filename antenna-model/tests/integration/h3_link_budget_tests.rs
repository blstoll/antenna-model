//! End-to-End Integration Tests for the H3 Heatmap Endpoint
//!
//! Tests cover:
//! - Cell count for various n_rings values
//! - loss_db is referenced to the grid peak: never negative, exactly 0.0 at the peak cell,
//!   and computed by the same rule as `/api/v1/heatmap` (roadmap C9)
//! - Link budget arithmetic consistency (total = loss + fspl)
//! - Unknown antenna returns HTTP 404
//! - n_rings > 10 returns HTTP 422
//! - calibration_status presence in response
//! - Cache consistency across identical requests
//! - Auto-resolution selection from frequency

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;

// ---------------------------------------------------------------------------
// Helper: build a valid H3LinkBudgetRequest using the same antenna/feed/coords
// as the existing integration tests (test_simple / primary, geodetic).
// ---------------------------------------------------------------------------
fn base_h3_request() -> H3LinkBudgetRequest {
    H3LinkBudgetRequest {
        antenna_id: "test_simple".to_string(),
        feed_id: "primary".to_string(),
        // Vehicle at Los Angeles area, 100 m altitude (geodetic)
        vehicle_position: Position3D::geodetic(-118.1234, 34.5678, 100.0),
        // Reflector boresight: slightly north and up (establishes pointing direction)
        reflector_boresight: Position3D::geodetic(-118.1234, 34.5679, 110.0),
        // feed_pointing_location is the H3 center cell location (same area as vehicle)
        feed_pointing_location: Position3D::geodetic(-118.124, 34.568, 105.0),
        frequency_mhz: 8400.0,
        pointing_frequency_mhz: None,
        n_rings: 2,
        h3_resolution: Some(7),
        temperature_k: None,
        vehicle_attitude: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: n_rings=0 returns exactly 1 cell
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_n_rings_0_returns_1_cell() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = base_h3_request();
    request.n_rings = 0;

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    assert_eq!(
        response.cells.len(),
        1,
        "n_rings=0 should produce exactly 1 cell, got {}",
        response.cells.len()
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 2: n_rings=2 returns exactly 19 cells (1 + 6 + 12)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_n_rings_2_returns_19_cells() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request(); // already n_rings=2

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    assert_eq!(
        response.cells.len(),
        19,
        "n_rings=2 should produce exactly 19 cells, got {}",
        response.cells.len()
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 3: loss_db is referenced to the grid PEAK (roadmap C9)
//
// Replaces the pre-C9 "centre cell is the zero" test. The centre cell is merely
// the cell nearest `feed_pointing_location`; the beam peak generally lies elsewhere, and
// referencing loss to the centre made every stronger cell report a negative
// loss_db. The reference is now `metadata.peak_gain_db` — max gain over the cells
// actually evaluated, the same rule `/api/v1/heatmap` applies.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_peak_cell_is_the_zero_loss_reference() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request();

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    let peak = response.metadata.peak_gain_db;
    assert!(
        peak.is_finite(),
        "peak_gain_db must be finite, got {}",
        peak
    );

    let mut zero_loss_cells = 0usize;
    for cell in &response.cells {
        // The client-visible symptom of the old reference: negative losses.
        assert!(
            cell.loss_db >= 0.0,
            "Cell {}: loss_db must never be negative under a peak reference, got {}",
            cell.cell_id,
            cell.loss_db
        );
        // …and its knock-on: a total path loss below free space.
        assert!(
            cell.total_path_loss_db >= cell.free_space_path_loss_db,
            "Cell {}: total_path_loss_db ({}) fell below free_space_path_loss_db ({})",
            cell.cell_id,
            cell.total_path_loss_db,
            cell.free_space_path_loss_db
        );
        // The response is internally re-derivable from the values it reports.
        assert!(
            (cell.loss_db - (peak - cell.gain_db)).abs() < 1e-9,
            "Cell {}: loss_db ({}) != peak_gain_db ({}) − gain_db ({})",
            cell.cell_id,
            cell.loss_db,
            peak,
            cell.gain_db
        );
        if cell.loss_db == 0.0 {
            zero_loss_cells += 1;
            assert!(
                (cell.gain_db - peak).abs() < 1e-12,
                "Cell {}: the zero-loss cell must be the peak cell",
                cell.cell_id
            );
        }
    }

    assert_eq!(
        zero_loss_cells, 1,
        "Exactly one cell — the peak — should carry loss_db == 0.0"
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 3b: /heatmap and /h3-heatmap reference loss by the same rule (C9)
//
// Both endpoints report `loss_db` relative to the peak gain over the points they
// actually evaluated, so on both the minimum loss over the grid is exactly 0.0 and
// no value is negative. This is the drift guard: the two endpoints gave the same
// field two meanings before C9.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_heatmap_and_h3_heatmap_reference_loss_by_the_same_rule() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let h3: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &base_h3_request())
        .await
        .expect("H3 heatmap computation failed");

    let heatmap: HeatmapResponse = server
        .post("/api/v1/heatmap", &builders::simple_heatmap_request())
        .await
        .expect("Heatmap computation failed");

    let h3_losses: Vec<f64> = h3.cells.iter().map(|c| c.loss_db).collect();
    let GridData::Rectangular { loss_db, .. } = &heatmap.grid;
    let rect_losses: Vec<f64> = loss_db.iter().flatten().copied().collect();

    for (endpoint, losses) in [("/h3-heatmap", &h3_losses), ("/heatmap", &rect_losses)] {
        assert!(!losses.is_empty(), "{endpoint} returned no grid values");
        let min = losses.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            min == 0.0,
            "{endpoint}: loss is peak-referenced, so the minimum over the grid must be \
             exactly 0.0 (the peak point), got {min}"
        );
        assert!(
            losses.iter().all(|l| *l >= 0.0),
            "{endpoint}: peak-referenced loss must never be negative"
        );
    }

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 4: total_path_loss_db == loss_db + free_space_path_loss_db for every cell
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_total_equals_loss_plus_fspl() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request();

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    for cell in &response.cells {
        let diff = (cell.total_path_loss_db - cell.loss_db - cell.free_space_path_loss_db).abs();
        assert!(
            diff < 0.001,
            "Cell {}: total_path_loss_db ({}) != loss_db ({}) + free_space_path_loss_db ({}) — diff={}",
            cell.cell_id,
            cell.total_path_loss_db,
            cell.loss_db,
            cell.free_space_path_loss_db,
            diff
        );
    }

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 5: Unknown antenna returns HTTP 404
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_unknown_antenna_404() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = base_h3_request();
    request.antenna_id = "does_not_exist".to_string();

    let url = format!("{}/api/v1/h3-heatmap", server.base_url);
    let raw = server
        .client
        .post(&url)
        .json(&request)
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(
        raw.status(),
        404,
        "Unknown antenna should return HTTP 404, got {}",
        raw.status()
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 6: n_rings=11 (> max 10) returns HTTP 422
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_n_rings_too_large_422() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut request = base_h3_request();
    request.n_rings = 11;

    let url = format!("{}/api/v1/h3-heatmap", server.base_url);
    let raw = server
        .client
        .post(&url)
        .json(&request)
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(
        raw.status(),
        422,
        "n_rings=11 should return HTTP 422, got {}",
        raw.status()
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 7: calibration_status is present in a valid response
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_calibration_status_present() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request();

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    assert!(
        response.calibration_status.is_some(),
        "calibration_status should be present in H3 heatmap response"
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 8: Identical requests return identical gain_db values (cache consistency)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_cache_consistency() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request();

    let response1: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("First H3 heatmap computation failed");

    let response2: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("Second H3 heatmap computation failed");

    assert_eq!(
        response1.cells.len(),
        response2.cells.len(),
        "Both responses should have the same number of cells"
    );

    // Sort both by cell_id so comparison is order-independent
    let mut cells1 = response1.cells.clone();
    let mut cells2 = response2.cells.clone();
    cells1.sort_by(|a, b| a.cell_id.cmp(&b.cell_id));
    cells2.sort_by(|a, b| a.cell_id.cmp(&b.cell_id));

    for (c1, c2) in cells1.iter().zip(cells2.iter()) {
        assert_eq!(
            c1.cell_id, c2.cell_id,
            "Cell IDs should match between requests"
        );
        assert_eq!(
            c1.gain_db, c2.gain_db,
            "gain_db for cell {} should be identical across requests (got {} vs {})",
            c1.cell_id, c1.gain_db, c2.gain_db
        );
    }

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 9: No h3_resolution field + frequency_mhz=12000.0 → h3_resolution==8
// (8000–20000 MHz maps to resolution 8 per h3_resolution_from_frequency)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_auto_resolution_from_frequency() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Use test_uncalibrated which supports a wider frequency range,
    // or test_simple (warnings are OK — the response should still succeed).
    let mut request = base_h3_request();
    // Remove explicit h3_resolution so the service auto-selects based on frequency
    request.h3_resolution = None;
    // 12000 MHz is in the 8000–20000 range → should auto-select resolution 8
    request.frequency_mhz = 12000.0;
    // Use test_uncalibrated / x_band which covers 7100–8500 MHz; to avoid
    // a hard validation failure we keep test_simple (warnings generated for
    // out-of-range frequency are acceptable — the endpoint still returns 200).
    // Alternatively use n_rings=0 to keep computation fast.
    request.n_rings = 0;

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    assert_eq!(
        response.h3_resolution, 8,
        "frequency_mhz=12000 should auto-select h3_resolution=8, got {}",
        response.h3_resolution
    );

    server.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 10: an identical repeated request returns an identical warning set
//
// Regression pin for roadmap C10. `/h3-heatmap` is the only endpoint that reads
// through `GainCache::get_or_compute`, and the model's warnings used to be
// captured only inside the cache-MISS closure. A second identical request
// therefore came back with a shorter `warnings` array than the first — worst
// case, silently dropping INTEGRATION_NONCONVERGENCE_WARNING from a value that
// a non-converged integration produced, breaking the "never silent" property
// the P10 self-check is supposed to guarantee.
//
// Both requests go through one server, so the second is served entirely from a
// warm cache.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_warnings_stable_across_cache_hits() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request();

    let first: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("first H3 heatmap computation failed");
    let second: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("second H3 heatmap computation failed");

    // Guard against a vacuous pass: if this geometry stopped producing warnings
    // the equality below would hold trivially and pin nothing.
    assert!(
        !first.warnings.is_empty(),
        "test geometry must produce at least one warning for this test to mean \
         anything; got none on the first (cold-cache) call"
    );

    let mut cold = first.warnings.clone();
    cold.sort();
    let mut warm = second.warnings.clone();
    warm.sort();

    assert_eq!(
        cold, warm,
        "warm-cache warnings must equal cold-cache warnings; a repeated identical \
         request must not lose warnings.\n  cold: {cold:#?}\n  warm: {warm:#?}"
    );

    server.shutdown().await;
}
