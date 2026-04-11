//! End-to-End Integration Tests for the H3 Heatmap Endpoint
//!
//! Tests cover:
//! - Cell count for various n_rings values
//! - Center cell loss is minimum and approximately 0.0
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
        // Vehicle at Los Angeles area, 100 m altitude (geodetic, auto-detected as small)
        vehicle_position: Position3D {
            x: -118.1234,
            y: 34.5678,
            z: 100.0,
        },
        // Reflector boresight: slightly north and up (establishes pointing direction)
        reflector_boresight: Position3D {
            x: -118.1234,
            y: 34.5679,
            z: 110.0,
        },
        // feed_position is the H3 center cell location (same area as vehicle)
        feed_position: Position3D {
            x: -118.124,
            y: 34.568,
            z: 105.0,
        },
        frequency_mhz: 8400.0,
        pointing_frequency_mhz: None,
        n_rings: 2,
        h3_resolution: Some(7),
        temperature_k: None,
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
// Test 3: Center cell has loss_db ≈ 0.0 and is the minimum-loss cell
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_h3_center_cell_minimum_loss() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = base_h3_request();

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    // Find the center cell
    let center_cell = response
        .cells
        .iter()
        .find(|c| c.cell_id == response.center_cell_id)
        .expect("Center cell must be present in cells list");

    // Center cell loss should be exactly 0.0 dB: it is the boresight reference
    // (the service computes boresight_gain_db from the center cell, so
    // loss_db = boresight_gain_db − gain_db = 0 for the center by definition).
    assert!(
        center_cell.loss_db.abs() < 0.01,
        "Center cell loss_db should be ≈ 0.0, got {}",
        center_cell.loss_db
    );

    // The center cell is the reference (loss_db = 0).  Off-axis cells generally
    // have loss_db > 0, but coma lobes can produce negative values for some
    // geometries.  Verify that no other cell has a *larger* absolute loss than
    // would be reasonable, and that the center cell has the minimum absolute
    // loss (i.e. it is closest to zero).
    let min_abs_loss = response
        .cells
        .iter()
        .map(|c| c.loss_db.abs())
        .fold(f64::INFINITY, f64::min);

    assert!(
        center_cell.loss_db.abs() <= min_abs_loss + 0.001,
        "Center cell |loss_db| ({}) should be the minimum absolute loss (min_abs={})",
        center_cell.loss_db.abs(),
        min_abs_loss
    );

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
