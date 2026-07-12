//! Off-Axis Honesty Warning Tests (roadmap unit P8)
//!
//! The physics model's off-axis (sidelobe) gain is systematically optimistic
//! (~8–13 dB below the ITU-R S.580 mask — see docs/domain-contract.md,
//! "Off-axis pattern / sidelobe fidelity"). Queries on UNCALIBRATED antennas
//! beyond the validated main-beam region (3× the first-null angle ≈ 1.6·λ/D)
//! must carry an explicit warning on every compute endpoint.
//!
//! Calibrated / partially-calibrated antennas are excluded: out-of-coverage
//! queries there already receive the extrapolation warning (no stacking) —
//! that gate is pinned by unit tests in `service::evaluator`.

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use antenna_model::model::coordinates_3d::geodetic_to_ecef;

/// Stable substring of the P8 warning message asserted by every test here.
/// (C8 stage 3 will convert this string warning to typed code
/// `off_axis_unvalidated`; update these tests to assert the code then.)
const OFF_AXIS_WARNING_MARKER: &str = "beyond the validated main-beam region";

/// A gain request on the uncalibrated 3.7 m test antenna whose emitter sits
/// tens of degrees off boresight — far beyond the ~2.8° warning threshold
/// (3 × 1.6·λ/D at 8000 MHz for D = 3.7 m).
fn off_axis_uncalibrated_gain_request() -> GainRequest {
    let mut req = builders::uncalibrated_antenna_request();
    // Boresight stays aimed at the original satellite (-117, 35, 400 km);
    // move the emitter to a different satellite tens of degrees away.
    let (x, y, z) = geodetic_to_ecef(-120.0, 30.0, 400_000.0).unwrap();
    req.emitter_position = Position3D {
        x,
        y,
        z,
        // Earth-orbit ECEF magnitudes here exceed nothing; tag explicitly to
        // avoid geodetic misclassification (matches the builder convention).
        coordinate_system: Some(CoordinateSystem::ECEF),
    };
    req
}

fn has_off_axis_warning(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains(OFF_AXIS_WARNING_MARKER))
}

/// Gain endpoint: large-θ query on an uncalibrated antenna warns.
#[tokio::test]
async fn test_gain_off_axis_uncalibrated_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = off_axis_uncalibrated_gain_request();
    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    assert!(
        has_off_axis_warning(&response.warnings),
        "expected off-axis honesty warning, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}

/// Gain endpoint: boresight query on the same uncalibrated antenna does NOT
/// carry the off-axis warning (main beam is the validated region).
#[tokio::test]
async fn test_gain_boresight_uncalibrated_does_not_warn_off_axis() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Builder default: emitter position == boresight target (θ ≈ 0).
    let request = builders::uncalibrated_antenna_request();
    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    assert!(
        !has_off_axis_warning(&response.warnings),
        "off-axis warning must not fire inside the main beam, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}

/// Batch endpoint: the per-item warning surfaces on the off-axis item.
#[tokio::test]
async fn test_batch_off_axis_uncalibrated_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = BatchGainRequest {
        evaluations: vec![off_axis_uncalibrated_gain_request()],
    };
    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &request)
        .await
        .expect("Batch computation failed");

    assert_eq!(response.results.len(), 1);
    assert!(
        has_off_axis_warning(&response.results[0].warnings),
        "expected off-axis honesty warning in batch item, got: {:?}",
        response.results[0].warnings
    );

    server.shutdown().await;
}

/// Heatmap endpoint: a grid extending to 10° off boresight on an uncalibrated
/// antenna carries the (deduplicated) warning in the aggregated list.
#[tokio::test]
async fn test_heatmap_off_axis_uncalibrated_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // simple_heatmap_request uses test_simple (uncalibrated, D = 5 m) with a
    // 0–10° az/el grid; the warning threshold at 8400 MHz is ~2.0°, so outer
    // grid points are well beyond it.
    let request = builders::simple_heatmap_request();
    let response: HeatmapResponse = server
        .post("/api/v1/heatmap", &request)
        .await
        .expect("Heatmap generation failed");

    assert!(
        has_off_axis_warning(&response.warnings),
        "expected off-axis honesty warning in heatmap warnings, got: {:?}",
        response.warnings
    );
    // The message is constant per (antenna, frequency), so aggregation must
    // deduplicate it to a single entry even though many points triggered it.
    assert_eq!(
        response
            .warnings
            .iter()
            .filter(|w| w.contains(OFF_AXIS_WARNING_MARKER))
            .count(),
        1,
        "off-axis warning must deduplicate across grid points, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}

/// H3 link-budget endpoint: ground cells tens of degrees off boresight on an
/// uncalibrated antenna carry the warning in the aggregated list.
#[tokio::test]
async fn test_h3_heatmap_off_axis_uncalibrated_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Mirrors h3_link_budget_tests::base_h3_request: test_simple (uncalibrated),
    // boresight steeply elevated while ring cells lie near the horizon —
    // cell directions sit tens of degrees off boresight.
    let request = H3LinkBudgetRequest {
        antenna_id: "test_simple".to_string(),
        feed_id: "primary".to_string(),
        vehicle_position: Position3D {
            x: -118.1234,
            y: 34.5678,
            z: 100.0,
            coordinate_system: None,
        },
        reflector_boresight: Position3D {
            x: -118.1234,
            y: 34.5679,
            z: 110.0,
            coordinate_system: None,
        },
        feed_position: Position3D {
            x: -118.124,
            y: 34.568,
            z: 105.0,
            coordinate_system: None,
        },
        frequency_mhz: 8400.0,
        pointing_frequency_mhz: None,
        n_rings: 2,
        h3_resolution: Some(7),
        temperature_k: None,
        vehicle_attitude: None,
    };

    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    assert!(
        has_off_axis_warning(&response.warnings),
        "expected off-axis honesty warning in h3-heatmap warnings, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}
