//! Concurrent Access Integration Tests
//!
//! Tests for concurrent request handling:
//! - Multiple simultaneous clients
//! - Concurrent gain computations
//! - Batch processing under load
//! - Thread safety of calibration repository

use crate::integration::helpers::*;
use antenna_model::api::schemas::*;
use tokio::task::JoinSet;

/// Test concurrent gain computations from multiple clients
#[tokio::test]
async fn test_concurrent_gain_computations() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let num_concurrent = 10;
    let mut tasks = JoinSet::new();

    for i in 0..num_concurrent {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let mut request = builders::simple_gain_request_ecef();
            request.frequency_mhz = 8000.0 + (i as f64 * 10.0);

            let url = format!("{}/api/v1/gain", server_url);
            let response = client.post(&url).json(&request).send().await?;

            if !response.status().is_success() {
                return Err(format!("Request {} failed", i).into());
            }

            let gain_response: GainResponse = response.json().await?;
            validators::validate_gain_response(&gain_response)?;

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(gain_response)
        });
    }

    // Wait for all tasks to complete
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(response)) => results.push(response),
            Ok(Err(e)) => panic!("Task failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    // All requests should succeed
    assert_eq!(results.len(), num_concurrent);

    // All results should be valid
    for response in &results {
        validators::validate_gain_response(response).expect("Invalid response");
    }

    server.shutdown().await;
}

/// Test concurrent batch requests
#[tokio::test]
async fn test_concurrent_batch_requests() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let num_concurrent = 5;
    let batch_size = 10;
    let mut tasks = JoinSet::new();

    for i in 0..num_concurrent {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let request = builders::simple_batch_request(batch_size);

            let url = format!("{}/api/v1/gain/batch", server_url);
            let response = client.post(&url).json(&request).send().await?;

            if !response.status().is_success() {
                return Err(format!("Batch {} failed", i).into());
            }

            let batch_response: BatchGainResponse = response.json().await?;
            validators::validate_batch_response(&batch_response)?;

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(batch_response)
        });
    }

    // Wait for all tasks
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(response)) => results.push(response),
            Ok(Err(e)) => panic!("Task failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    assert_eq!(results.len(), num_concurrent);

    // Verify all batches completed successfully
    for response in &results {
        assert_eq!(response.results.len(), batch_size);
        assert_eq!(response.metadata.count, batch_size);
    }

    server.shutdown().await;
}

/// Test mixed concurrent requests (single + batch + heatmap)
#[tokio::test]
async fn test_mixed_concurrent_requests() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let mut tasks = JoinSet::new();

    // Spawn single gain requests
    for i in 0..5 {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let request = builders::simple_gain_request_ecef();
            let url = format!("{}/api/v1/gain", server_url);
            let response = client.post(&url).json(&request).send().await?;

            let gain_response: GainResponse = response.json().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(("single", i, gain_response.gain_db))
        });
    }

    // Spawn batch requests
    for i in 0..3 {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let request = builders::simple_batch_request(5);
            let url = format!("{}/api/v1/gain/batch", server_url);
            let response = client.post(&url).json(&request).send().await?;

            let batch_response: BatchGainResponse = response.json().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
                "batch",
                i,
                batch_response.results.len() as f64,
            ))
        });
    }

    // Spawn heatmap requests
    for i in 0..2 {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let request = builders::simple_heatmap_request();
            let url = format!("{}/api/v1/heatmap", server_url);
            let response = client.post(&url).json(&request).send().await?;

            let heatmap_response: HeatmapResponse = response.json().await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
                "heatmap",
                i,
                heatmap_response.metadata.points_evaluated as f64,
            ))
        });
    }

    // Wait for all tasks
    let mut success_count = 0;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(_)) => success_count += 1,
            Ok(Err(e)) => panic!("Task failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    // All requests should succeed (5 single + 3 batch + 2 heatmap = 10)
    assert_eq!(success_count, 10);

    server.shutdown().await;
}

/// Test concurrent access to same antenna (thread safety)
#[tokio::test]
async fn test_concurrent_same_antenna() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let num_concurrent = 20;
    let mut tasks = JoinSet::new();

    // All requests use the same antenna
    for i in 0..num_concurrent {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let request = builders::simple_gain_request_ecef();

            let url = format!("{}/api/v1/gain", server_url);
            let response = client.post(&url).json(&request).send().await?;

            let gain_response: GainResponse = response.json().await?;

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((i, gain_response.gain_db))
        });
    }

    // Collect results
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(response)) => results.push(response),
            Ok(Err(e)) => panic!("Task failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    assert_eq!(results.len(), num_concurrent);

    // All results should be consistent (same antenna, same request)
    let first_gain = results[0].1;
    for (i, gain) in &results {
        // Gains should be identical for identical requests
        assert!(
            (gain - first_gain).abs() < 0.01,
            "Request {} got gain {} but expected {}",
            i,
            gain,
            first_gain
        );
    }

    server.shutdown().await;
}

/// Test concurrent access to different antennas
#[tokio::test]
async fn test_concurrent_different_antennas() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let antennas = [
        ("test_simple", "primary", 8400.0),
        ("test_uncalibrated", "x_band", 8000.0),
        ("test_large", "x_band", 7200.0),
    ];

    let mut tasks = JoinSet::new();

    // Cycle through antennas
    for i in 0..15 {
        let (antenna_id, feed_id, freq) = antennas[i % antennas.len()];
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let mut request = builders::simple_gain_request_ecef();
            request.antenna_id = antenna_id.to_string();
            request.feed_id = feed_id.to_string();
            request.frequency_mhz = freq;

            let url = format!("{}/api/v1/gain", server_url);
            let response = client.post(&url).json(&request).send().await?;

            let gain_response: GainResponse = response.json().await?;

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
                antenna_id.to_string(),
                gain_response.gain_db,
            ))
        });
    }

    // Collect results
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(response)) => results.push(response),
            Ok(Err(e)) => panic!("Task failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    assert_eq!(results.len(), 15);

    // All results should be valid. The shared request steers the feed far off
    // boresight (feed near the vehicle, boresight at the satellite), so gains are
    // well below each antenna's boresight maximum.
    //
    // P10 (off-axis integrator): the feed-steering offset here is many focal lengths
    // (δ/f ≫ 1), so these route through the ray-tracing stub (D-5) whose boresight anchor
    // is the physical-optics aperture integral. The old `> 5.0` bound was calibrated to the
    // pre-P10 aliased ≈ 8.7 dBi; P10 moved it again.
    //
    // Widened 2026-07-31 (φ'-cap removal): the sentence that used to sit here — "in this
    // strongly-steered regime the mode integrator is performance-capped … so this asserts a
    // broad physical-plausibility range rather than a converged level" — described exactly
    // the defect that was removed. That cap (`MODE_PHI_STEERED_MAX`, `n_phi` clamped to 64)
    // was not a graceful degradation: measured against the 2D Simpson oracle it was wrong by
    // up to +82 dB, silently, with `converged = true`. `n_phi` is now sized from the aperture
    // function's actual azimuthal bandwidth and flagged when it cannot be met, so these values
    // ARE converged; test_large lands at −15.27 dBi.
    //
    // Still a plausibility band rather than an accuracy claim — the ray-tracing stub (D-5)
    // remains a stub, and these geometries are deep steered nulls. The lower bound must admit
    // them; the upper bound is the meaningful one.
    for (antenna_id, gain) in &results {
        assert!(
            (-40.0..60.0).contains(gain),
            "Antenna {} got invalid gain {}",
            antenna_id,
            gain
        );
    }

    server.shutdown().await;
}

/// Test concurrent health checks
#[tokio::test]
async fn test_concurrent_health_checks() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let num_concurrent = 50;
    let mut tasks = JoinSet::new();

    for _ in 0..num_concurrent {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let url = format!("{}/health", server_url);
            let response = client.get(&url).send().await?;

            let health_response: HealthResponse = response.json().await?;

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(health_response.status)
        });
    }

    // All health checks should succeed
    let mut success_count = 0;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(status)) => {
                assert_eq!(status, "healthy");
                success_count += 1;
            }
            Ok(Err(e)) => panic!("Health check failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    assert_eq!(success_count, num_concurrent);

    server.shutdown().await;
}

/// Repeated concurrent use of the real gain path (GitHub issue #73).
///
/// Three workers start together and each completes a fixed, small number of **sequential**
/// gain requests against the live server. The property under test is correctness under
/// repeated concurrency: every request returns 2xx with a well-formed, self-consistent body,
/// and every worker gets all the way round its loop more than once.
///
/// **This is not a throughput, load or soak test, and must not become one.** It has no
/// wall-clock window, no pacing sleep, and no request-count or latency threshold — every such
/// bound is a function of how much of the machine the test happens to get, which is why the
/// predecessor (`test_sustained_load`, a 2 s window with a throughput-derived floor) flaked
/// twice on CI and had to reserve every test thread to stay green. Real throughput and soak
/// numbers require a release build on a controlled machine: that measurement belongs in
/// `cargo bench` (see `antenna-model/benches/`), never in a debug-build integration test on a
/// shared runner.
#[tokio::test]
async fn test_repeated_concurrent_gain_requests() {
    /// Concurrent workers, started together on a barrier.
    const NUM_WORKERS: usize = 3;
    /// Sequential requests each worker must complete. Two is the smallest count that proves a
    /// worker went round its loop again rather than blocking forever in the first `send()`.
    const REQUESTS_PER_WORKER: usize = 2;
    /// Failure bound, not a performance budget. Six real gain evaluations in a debug build
    /// take ~1 s here and a couple of seconds on the slowest CI runner seen; two minutes is
    /// far outside any plausible scheduling variance, so tripping it means a hang or deadlock,
    /// never a slow machine.
    const COMPLETION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
    /// Two identical requests must produce the same gain. The tolerance absorbs float
    /// summation reordering in the parallel aperture integration, nothing more — it is *not*
    /// an accuracy budget, and any real cross-request state corruption moves the value by
    /// orders of magnitude more than this.
    const REPEAT_TOLERANCE_DB: f64 = 1e-9;

    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    // Every worker blocks here until all of them have been scheduled, so the requests are
    // genuinely concurrent rather than merely spawned from the same loop.
    let start_line = std::sync::Arc::new(tokio::sync::Barrier::new(NUM_WORKERS));

    let mut tasks = JoinSet::new();
    for worker_id in 0..NUM_WORKERS {
        let server_url = server.base_url.clone();
        let client = server.client.clone();
        let start_line = std::sync::Arc::clone(&start_line);

        tasks.spawn(async move {
            start_line.wait().await;

            let mut gains = Vec::with_capacity(REQUESTS_PER_WORKER);
            for request_index in 0..REQUESTS_PER_WORKER {
                // Identical request each time round, so the two responses are directly
                // comparable; workers differ from each other by frequency.
                let mut request = builders::simple_gain_request_ecef();
                request.frequency_mhz = 8000.0 + (worker_id as f64 * 10.0);

                let url = format!("{}/api/v1/gain", server_url);
                let where_ = format!("worker {worker_id} request {request_index}");

                let response = client
                    .post(&url)
                    .json(&request)
                    .send()
                    .await
                    .map_err(|e| format!("{where_}: transport error: {e}"))?;

                let status = response.status();
                if !status.is_success() {
                    let body = response.text().await.unwrap_or_default();
                    return Err(format!("{where_}: expected 2xx, got {status}: {body}"));
                }

                let gain: GainResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("{where_}: body is not a GainResponse: {e}"))?;

                validators::validate_gain_response(&gain).map_err(|e| format!("{where_}: {e}"))?;
                if gain.antenna_id != request.antenna_id || gain.feed_id != request.feed_id {
                    return Err(format!(
                        "{where_}: response is for ({}, {}), not the requested ({}, {}) \
                         — responses were crossed between concurrent requests",
                        gain.antenna_id, gain.feed_id, request.antenna_id, request.feed_id,
                    ));
                }

                gains.push(gain);
            }

            Ok::<_, String>((worker_id, gains))
        });
    }

    // Slot per worker, so a worker that never finishes is named rather than lost in a total.
    let mut completed: Vec<Option<Vec<GainResponse>>> = (0..NUM_WORKERS).map(|_| None).collect();
    let all_joined = tokio::time::timeout(COMPLETION_TIMEOUT, async {
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok((worker_id, gains))) => completed[worker_id] = Some(gains),
                Ok(Err(e)) => panic!("{e}"),
                Err(e) => panic!("worker task panicked or was cancelled: {e}"),
            }
        }
    })
    .await;
    assert!(
        all_joined.is_ok(),
        "workers did not all finish within {COMPLETION_TIMEOUT:?}; \
         completed so far: {:?}",
        completed
            .iter()
            .map(|slot| slot.as_ref().map_or(0, Vec::len))
            .collect::<Vec<_>>(),
    );

    // Assert each worker separately: an aggregate count cannot distinguish "all three looped
    // twice" from "one looped six times while two hung on their first request".
    for (worker_id, slot) in completed.iter().enumerate() {
        let gains = slot
            .as_ref()
            .unwrap_or_else(|| panic!("worker {worker_id} did not complete"));
        assert_eq!(
            gains.len(),
            REQUESTS_PER_WORKER,
            "worker {worker_id} completed {} of {REQUESTS_PER_WORKER} requests",
            gains.len(),
        );

        // Repeated identical requests must agree: the shared calibration repository and
        // evaluator carry no per-request state that concurrency can corrupt.
        let first = gains[0].gain_db;
        for (request_index, gain) in gains.iter().enumerate().skip(1) {
            assert!(
                (gain.gain_db - first).abs() <= REPEAT_TOLERANCE_DB,
                "worker {worker_id} request {request_index} returned {} dB for the same \
                 request that returned {first} dB on its first pass",
                gain.gain_db,
            );
        }
    }

    server.shutdown().await;
}

/// Test error handling under concurrent load
#[tokio::test]
async fn test_concurrent_error_handling() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let num_concurrent = 20;
    let mut tasks = JoinSet::new();

    // Mix of valid and invalid requests
    for i in 0..num_concurrent {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let mut request = builders::simple_gain_request_ecef();

            // Every 5th request is invalid
            if i % 5 == 0 {
                request.antenna_id = "invalid_antenna".to_string();
            }

            let url = format!("{}/api/v1/gain", server_url);
            let response = client.post(&url).json(&request).send().await?;

            let status = response.status();
            let success = status.is_success();

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((i, success))
        });
    }

    // Collect results
    let mut valid_count = 0;
    let mut error_count = 0;

    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok((i, success))) => {
                if success {
                    valid_count += 1;
                    // Valid requests should be those NOT divisible by 5
                    assert!(i % 5 != 0, "Request {} should have failed", i);
                } else {
                    error_count += 1;
                    // Invalid requests should be those divisible by 5
                    assert_eq!(i % 5, 0, "Request {} should have succeeded", i);
                }
            }
            Ok(Err(e)) => panic!("Task failed unexpectedly: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    // Should have both successes and errors
    assert!(valid_count > 0, "Expected some valid requests");
    assert!(error_count > 0, "Expected some error requests");
    assert_eq!(
        valid_count + error_count,
        num_concurrent,
        "Total count mismatch"
    );

    server.shutdown().await;
}
