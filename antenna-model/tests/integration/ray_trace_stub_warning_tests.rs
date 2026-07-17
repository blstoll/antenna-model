//! Ray-Tracing Stub Degraded-Accuracy Warning Tests (roadmap unit P3,
//! maintainer decision 2026-07-16: document + flag).
//!
//! Feed offsets beyond the severe threshold (> 0.5·f) route through the
//! acknowledged ray-tracing stub (`model/ray_trace.rs`; real ray tracing is
//! gated as feature F2). Results there carry an honest "not fully implemented;
//! gain accuracy may be degraded" warning. P3 pins that this warning reaches
//! ALL FOUR compute endpoints (`/gain`, `/gain/batch`, `/heatmap`,
//! `/h3-heatmap`), and — via `evaluator::ray_trace_stub_warning` re-emitted
//! outside the H3 gain-cache closure — that it survives H3 cache HITS, not just
//! the first (cold-cache) request.
//!
//! The large-offset geometry reuses the `test_uncalibrated` request builder
//! (`builders::uncalibrated_antenna_request`): the feed is aimed at a ground
//! point beside the vehicle while the reflector boresight points at a 400 km
//! satellite, giving a ~90° feed/boresight angular gap and a feed displacement
//! of ~3·f — well beyond the 0.5·f ray-tracing threshold. The offset ratio is
//! geometry-driven (≈ 2·tan(cone/2)/bdf) and effectively antenna-independent,
//! so the same pointing geometry routes any enabled antenna through the stub.
//! The negative control re-aims the feed at the boresight target, collapsing the
//! offset to ≈ 0 (StandardPhysicalOptics, no stub).

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;

/// Stable substring of the ray-tracing stub warning
/// (`model::pattern::RAY_TRACING_STUB_WARNING`). "not fully implemented" is the
/// load-bearing honesty flag and is specific to this warning (distinct from the
/// advisory "Ray tracing recommended" edge-case warning).
const RAY_TRACE_STUB_MARKER: &str = "not fully implemented";

fn has_ray_trace_stub_warning(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains(RAY_TRACE_STUB_MARKER))
}

/// A request whose feed is aimed far off the reflector boresight, producing a
/// feed offset ≈ 3·f (> 0.5·f), which routes through the ray-tracing stub.
fn large_feed_offset_gain_request() -> GainRequest {
    builders::uncalibrated_antenna_request()
}

/// Negative control: the feed aims at the SAME target as the reflector
/// boresight, so the feed offset is ≈ 0 (well under 0.5·f) — no ray-tracing stub.
fn small_feed_offset_gain_request() -> GainRequest {
    let mut req = builders::uncalibrated_antenna_request();
    req.feed_position = req.reflector_boresight.clone();
    req
}

fn large_feed_offset_heatmap_request() -> HeatmapRequest {
    let g = large_feed_offset_gain_request();
    HeatmapRequest {
        antenna_id: g.antenna_id,
        feed_id: g.feed_id,
        vehicle_position: g.vehicle_position,
        reflector_boresight: g.reflector_boresight,
        feed_position: g.feed_position,
        frequency_mhz: g.frequency_mhz,
        pointing_frequency_mhz: None,
        grid_config: GridConfig::Rectangular {
            azimuth_range_deg: RangeConfig {
                min: 0.0,
                max: 4.0,
                step: 2.0,
            },
            elevation_range_deg: RangeConfig {
                min: 0.0,
                max: 4.0,
                step: 2.0,
            },
        },
    }
}

fn large_feed_offset_h3_request() -> H3LinkBudgetRequest {
    let g = large_feed_offset_gain_request();
    H3LinkBudgetRequest {
        antenna_id: g.antenna_id,
        feed_id: g.feed_id,
        vehicle_position: g.vehicle_position,
        reflector_boresight: g.reflector_boresight,
        feed_position: g.feed_position,
        frequency_mhz: g.frequency_mhz,
        pointing_frequency_mhz: None,
        n_rings: 1,
        h3_resolution: Some(7),
        temperature_k: None,
        vehicle_attitude: None,
    }
}

/// Gain endpoint: a > 0.5·f request carries the ray-tracing stub warning.
#[tokio::test]
async fn test_gain_large_feed_offset_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = large_feed_offset_gain_request();
    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    assert!(
        has_ray_trace_stub_warning(&response.warnings),
        "expected ray-tracing stub warning, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}

/// Negative control: a boresight-aimed request on the same antenna (feed offset
/// ≈ 0) does NOT carry the ray-tracing stub warning.
#[tokio::test]
async fn test_gain_small_feed_offset_does_not_warn() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = small_feed_offset_gain_request();
    let response: GainResponse = server
        .post("/api/v1/gain", &request)
        .await
        .expect("Gain computation failed");

    assert!(
        !has_ray_trace_stub_warning(&response.warnings),
        "ray-tracing stub warning must not fire for a small feed offset, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}

/// Batch endpoint: the per-item warning surfaces on the large-offset item.
#[tokio::test]
async fn test_batch_large_feed_offset_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = BatchGainRequest {
        evaluations: vec![large_feed_offset_gain_request()],
    };
    let response: BatchGainResponse = server
        .post("/api/v1/gain/batch", &request)
        .await
        .expect("Batch computation failed");

    assert_eq!(response.results.len(), 1);
    assert!(
        has_ray_trace_stub_warning(&response.results[0].warnings),
        "expected ray-tracing stub warning in batch item, got: {:?}",
        response.results[0].warnings
    );

    server.shutdown().await;
}

/// Heatmap endpoint: every grid point shares the (constant) large feed offset,
/// so the warning appears in the aggregated list, deduplicated to one entry.
#[tokio::test]
async fn test_heatmap_large_feed_offset_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = large_feed_offset_heatmap_request();
    let response: HeatmapResponse = server
        .post("/api/v1/heatmap", &request)
        .await
        .expect("Heatmap generation failed");

    assert!(
        has_ray_trace_stub_warning(&response.warnings),
        "expected ray-tracing stub warning in heatmap warnings, got: {:?}",
        response.warnings
    );
    // Constant per antenna config, so aggregation must deduplicate to one entry.
    assert_eq!(
        response
            .warnings
            .iter()
            .filter(|w| w.contains(RAY_TRACE_STUB_MARKER))
            .count(),
        1,
        "ray-tracing stub warning must deduplicate across grid points, got: {:?}",
        response.warnings
    );
}

/// H3 link-budget endpoint: the warning appears in the aggregated list.
#[tokio::test]
async fn test_h3_heatmap_large_feed_offset_warns() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = large_feed_offset_h3_request();
    let response: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed");

    assert!(
        has_ray_trace_stub_warning(&response.warnings),
        "expected ray-tracing stub warning in h3-heatmap warnings, got: {:?}",
        response.warnings
    );

    server.shutdown().await;
}

/// H3 cache-hit robustness (the P3 fix): the H3 gain cache stores physics-only
/// gain and only runs the model — which pushes the stub warning — on a cache
/// MISS. A second identical request is served entirely from the warm cache;
/// `evaluator::ray_trace_stub_warning`, emitted outside the cache closure, must
/// keep the warning present on that warm-cache response too.
#[tokio::test]
async fn test_h3_heatmap_large_feed_offset_warns_on_cache_hit() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let request = large_feed_offset_h3_request();

    // First request populates the shared gain cache (cache miss on every cell).
    let first: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed (cold cache)");
    assert!(
        has_ray_trace_stub_warning(&first.warnings),
        "expected ray-tracing stub warning on cold cache, got: {:?}",
        first.warnings
    );

    // Second identical request is served from the warm cache (cache hit on every
    // cell) — the model is not re-run, so the warning must come from the
    // service-layer re-emission.
    let second: H3LinkBudgetResponse = server
        .post("/api/v1/h3-heatmap", &request)
        .await
        .expect("H3 heatmap computation failed (warm cache)");
    assert!(
        has_ray_trace_stub_warning(&second.warnings),
        "ray-tracing stub warning must survive H3 cache hits, got: {:?}",
        second.warnings
    );

    server.shutdown().await;
}
