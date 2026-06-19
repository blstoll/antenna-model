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
    // well below each antenna's boresight maximum. With the aperture-directivity
    // formula (no hardcoded 0.55 efficiency) the smallest of these is ≈ 8.7 dBi.
    for (antenna_id, gain) in &results {
        assert!(
            *gain > 5.0 && *gain < 60.0,
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

/// Test load with sustained concurrent requests
#[tokio::test]
async fn test_sustained_load() {
    let server = TestServer::start()
        .await
        .expect("Failed to start test server");

    let duration_ms = 2000; // 2 seconds
    let request_interval_ms = 50; // 20 req/s per task
    let num_workers = 3;

    let mut tasks = JoinSet::new();

    for worker_id in 0..num_workers {
        let server_url = server.base_url.clone();
        let client = server.client.clone();

        tasks.spawn(async move {
            let start = std::time::Instant::now();
            let mut request_count = 0;

            while start.elapsed().as_millis() < duration_ms as u128 {
                let mut request = builders::simple_gain_request_ecef();
                request.frequency_mhz = 8000.0 + (worker_id as f64 * 10.0);

                let url = format!("{}/api/v1/gain", server_url);
                let response = client.post(&url).json(&request).send().await;

                if response.is_ok() {
                    request_count += 1;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(request_interval_ms)).await;
            }

            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(request_count)
        });
    }

    // Collect results
    let mut total_requests = 0;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(count)) => total_requests += count,
            Ok(Err(e)) => panic!("Worker failed: {}", e),
            Err(e) => panic!("Join error: {}", e),
        }
    }

    // Should have processed many requests successfully
    // Expected: ~3 workers * 20 req/s * 2s = ~120 requests
    assert!(
        total_requests > 50,
        "Expected >50 requests under load, got {}",
        total_requests
    );

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
