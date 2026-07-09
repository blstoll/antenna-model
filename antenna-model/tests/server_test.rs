//! Integration tests for server startup and status endpoint
//!
//! These tests verify that the server can start, respond to requests,
//! and shut down gracefully.

use std::time::Duration;
use tokio::time::timeout;

/// Test that the server can start and is accessible
///
/// This test verifies:
/// - Server binds to the configured port
/// - Status endpoint returns 200 OK
/// - Response contains expected JSON fields
#[tokio::test]
async fn test_server_startup_and_status() {
    // Start the server in a background task
    let server_handle = tokio::spawn(async {
        antenna_model::api::start_server("127.0.0.1", 3001)
            .await
            .expect("Failed to start server");
    });

    // Give the server a moment to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Make a request to the status endpoint
    let client = reqwest::Client::new();
    let response = timeout(
        Duration::from_secs(5),
        client.get("http://127.0.0.1:3001/status").send(),
    )
    .await
    .expect("Request timed out")
    .expect("Request failed");

    // Verify response
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json; charset=utf-8"
    );

    // Parse JSON body
    let json: serde_json::Value = response.json().await.expect("Failed to parse JSON");

    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
    assert!(json["uptime_seconds"].is_u64());

    // Abort the server task
    server_handle.abort();
}

/// Test that uptime increases over time
#[tokio::test]
async fn test_status_uptime_increases() {
    // Start the server
    let server_handle = tokio::spawn(async {
        antenna_model::api::start_server("127.0.0.1", 3002)
            .await
            .expect("Failed to start server");
    });

    // Wait for server to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    let client = reqwest::Client::new();

    // First request
    let response1 = client
        .get("http://127.0.0.1:3002/status")
        .send()
        .await
        .expect("First request failed");
    let json1: serde_json::Value = response1.json().await.expect("Failed to parse JSON");
    let uptime1 = json1["uptime_seconds"].as_u64().expect("Invalid uptime");

    // Wait a bit
    tokio::time::sleep(Duration::from_millis(1100)).await;

    // Second request
    let response2 = client
        .get("http://127.0.0.1:3002/status")
        .send()
        .await
        .expect("Second request failed");
    let json2: serde_json::Value = response2.json().await.expect("Failed to parse JSON");
    let uptime2 = json2["uptime_seconds"].as_u64().expect("Invalid uptime");

    // Uptime should have increased by at least 1 second
    assert!(uptime2 > uptime1);

    server_handle.abort();
}
