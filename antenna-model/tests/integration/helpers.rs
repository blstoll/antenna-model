//! Integration Test Helpers
//!
//! Utilities for integration testing the Antenna Model Service:
//! - Test server management (start/stop)
//! - HTTP client for API requests
//! - Response validation helpers
//! - Test data generation

use antenna_model::api::schemas::*;
use antenna_model::api::AppState;
use antenna_model::config::ServiceConfig;
use antenna_model::data::repository::CalibrationRepository;
use poem::listener::TcpAcceptor;
use poem::Server;
use serde::de::DeserializeOwned;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Test server handle for managing a test API instance
pub struct TestServer {
    pub base_url: String,
    pub client: reqwest::Client,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TestServer {
    /// Start a new test server on an available port
    ///
    /// This starts the full API server with test configuration and returns
    /// a handle for making requests and shutting down the server.
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_with_config(None).await
    }

    /// Start test server with custom configuration
    pub async fn start_with_config(
        config: Option<ServiceConfig>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Use provided config or create test config
        let config = config.unwrap_or_else(|| {
            let mut cfg = ServiceConfig::with_defaults();

            // Override with test-specific settings
            cfg.server.host = "127.0.0.1".to_string();
            cfg.server.port = 0; // Let OS assign port
            cfg.server.max_body_size_bytes = 10485760; // 10MB
            cfg.server.request_timeout_secs = 30;

            // Use CARGO_MANIFEST_DIR to get absolute path to test fixtures
            let manifest_dir =
                std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
            let fixtures_dir = PathBuf::from(&manifest_dir).join("tests/fixtures");

            cfg.calibration.data_directory = fixtures_dir.clone();
            cfg.calibration.antenna_config_file = fixtures_dir.join("test_antennas.yaml");
            cfg.calibration.fail_fast = false; // Don't fail on uncalibrated antennas

            cfg.performance.worker_threads = 2;
            cfg.performance.max_batch_size = 1000;
            cfg.performance.enable_parallel_processing = true;

            cfg
        });

        // Default: derive the request timeout from config (whole seconds).
        let timeout = Duration::from_secs(config.server.request_timeout_secs);
        Self::start_inner(config, timeout).await
    }

    /// Start test server with an explicit request-timeout `Duration`.
    ///
    /// The public `request_timeout_secs` config is whole seconds; this lets a
    /// test exercise the 504 timeout path with a **sub-second** deadline, so the
    /// timeout fires with a large margin over real compute instead of coupling
    /// the assertion to multi-second wall-clock timing.
    pub async fn start_with_config_and_timeout(
        config: ServiceConfig,
        timeout: Duration,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::start_inner(config, timeout).await
    }

    /// Shared server bring-up: load the repository, build the app with the given
    /// request-timeout `Duration`, bind an ephemeral port, and wait for health.
    async fn start_inner(
        config: ServiceConfig,
        timeout: Duration,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Load calibration repository
        let repository = CalibrationRepository::load_from_config(&config.calibration)
            .expect("Failed to load calibration data for tests");

        let state = Arc::new(AppState::new(config.clone(), repository));

        // Mirror the production startup sequence (roadmap S5): the repository above loaded
        // successfully, so publish the loaded set and mark the service ready. Without these,
        // /ready would 503 and /status would report zero antennas for the whole test run.
        state.set_antenna_ids(state.repository.list_antennas());
        state.mark_ready();

        // Build routes with the resolved timeout (create_routes is exactly this
        // with timeout = from_secs(config.request_timeout_secs)).
        let app = antenna_model::api::routes::create_routes_with_timeout(state, timeout);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Bind to port 0 to let OS assign an available port (avoids conflicts in parallel tests)

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("creating tcp server");
        let actual_addr = listener
            .local_addr()
            .expect("getting local address")
            .to_string();

        let acceptor = TcpAcceptor::from_tokio(listener).expect("creating expector");
        let base_url = format!("http://{}", actual_addr);

        // Spawn server task
        tokio::spawn(async move {
            let _ = Server::new_with_acceptor(acceptor)
                .run_with_graceful_shutdown(
                    app,
                    async move {
                        shutdown_rx.await.ok();
                    },
                    Some(Duration::from_secs(5)),
                )
                .await;
        });

        // Wait for server to be ready
        sleep(Duration::from_millis(50)).await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        // Verify server is responding
        let test_server = TestServer {
            base_url: base_url.clone(),
            client: client.clone(),
            shutdown_tx: Some(shutdown_tx),
        };

        // Wait for health endpoint
        for _ in 0..50 {
            if test_server.get::<HealthResponse>("/health").await.is_ok() {
                return Ok(test_server);
            }
            sleep(Duration::from_millis(100)).await;
        }

        Err("Server failed to start within timeout".into())
    }

    /// Make GET request to endpoint
    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, Box<dyn std::error::Error>> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(format!("Request failed with status: {}", response.status()).into());
        }

        let body = response.json::<T>().await?;
        Ok(body)
    }

    /// Make POST request to endpoint
    pub async fn post<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, Box<dyn std::error::Error>> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.post(&url).json(body).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("Request failed with status {}: {}", status, error_body).into());
        }

        let body = response.json::<T>().await?;
        Ok(body)
    }

    /// Shutdown the test server
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            // Give server time to shutdown gracefully
            sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Build the full middleware-wrapped app **in process** — no TCP listener, no
/// background server task, no HTTP client.
///
/// Call it with `poem::Endpoint::call`, which drives the identical middleware
/// stack (`RequestId` → `RequestTimeout` → … → handler) that `TestServer` serves
/// over a socket.
///
/// This exists for tests that need to control *scheduling order*. Over a socket,
/// the request reaching the handler depends on real loopback I/O, which no amount
/// of `tokio::time` control can order — under a paused clock the mocked time jumps
/// past the deadline in microseconds of real time, before the request has arrived,
/// so the timeout registers after the jump and never fires. In process, the
/// request reaches the handler synchronously within the first poll of the task,
/// which makes the ordering deterministic.
pub fn build_in_process_app(
    config: ServiceConfig,
    timeout: Duration,
) -> Result<impl poem::Endpoint, Box<dyn std::error::Error>> {
    let repository = CalibrationRepository::load_from_config(&config.calibration)?;
    let state = Arc::new(AppState::new(config, repository));

    // Mirror the production startup sequence (roadmap S5), as `TestServer` does.
    state.set_antenna_ids(state.repository.list_antennas());
    state.mark_ready();

    Ok(antenna_model::api::routes::create_routes_with_timeout(
        state, timeout,
    ))
}

/// POST `body` as JSON to `path` on an in-process app, returning
/// `(status, body_bytes)`.
///
/// Uses `Endpoint::get_response`, which folds both the `Ok` and `Err` arms into a
/// response — so a middleware rejection (413 / 504 / 503) is observed the way a
/// client would observe it rather than surfacing as a bare `poem::Error`.
pub async fn call_json<E: poem::Endpoint, B: serde::Serialize>(
    app: &E,
    path: &str,
    body: &B,
) -> (poem::http::StatusCode, Vec<u8>) {
    let request = poem::Request::builder()
        .method(poem::http::Method::POST)
        .uri(path.parse().expect("valid test URI"))
        .header("content-type", "application/json")
        .body(serde_json::to_vec(body).expect("serializable request body"));

    let response = app.get_response(request).await;

    let status = response.status();
    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .expect("readable response body")
        .to_vec();

    (status, bytes)
}

/// Test data builders for creating realistic API requests
pub mod builders {
    use super::*;

    /// Create a simple gain request for testing (ECEF coordinates)
    pub fn simple_gain_request_ecef() -> GainRequest {
        // Use geodetic_to_ecef to generate valid ECEF coordinates
        // Vehicle at Los Angeles area (same as geodetic test) at 100m altitude
        use antenna_model::model::coordinates_3d::geodetic_to_ecef;
        let (veh_x, veh_y, veh_z) = geodetic_to_ecef(-118.1234, 34.5678, 100.0).unwrap();

        // Emitter at 400km altitude (satellite)
        let (emit_x, emit_y, emit_z) = geodetic_to_ecef(-117.0, 35.0, 400_000.0).unwrap();

        // Reflector boresight pointing at emitter (same as emitter position)
        // Feed position close to vehicle
        let (feed_x, feed_y, feed_z) = geodetic_to_ecef(-118.124, 34.568, 105.0).unwrap();

        GainRequest {
            antenna_id: "test_simple".to_string(),
            feed_id: "primary".to_string(),
            vehicle_position: Position3D {
                x: veh_x,
                y: veh_y,
                z: veh_z,
                // Earth-surface ECEF values are ~2-6 Mm, below 6400 km threshold;
                // set explicit tag so they are not misclassified as Geodetic.
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            reflector_boresight: Position3D {
                x: emit_x,
                y: emit_y,
                z: emit_z,
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            feed_position: Position3D {
                x: feed_x,
                y: feed_y,
                z: feed_z,
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            emitter_position: Position3D {
                x: emit_x,
                y: emit_y,
                z: emit_z,
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            include_reference: false,
            vehicle_attitude: None,
        }
    }

    /// Create a gain request with Geodetic coordinates
    pub fn simple_gain_request_geodetic() -> GainRequest {
        GainRequest {
            antenna_id: "test_simple".to_string(),
            feed_id: "primary".to_string(),
            vehicle_position: Position3D {
                x: -118.1234,
                y: 34.5678,
                z: 100.0,
                coordinate_system: None,
            },
            reflector_boresight: Position3D {
                x: -117.0,
                y: 35.0,
                z: 400000.0,
                coordinate_system: None,
            },
            feed_position: Position3D {
                x: -118.124,
                y: 34.568,
                z: 105.0,
                coordinate_system: None,
            },
            emitter_position: Position3D {
                x: -117.0,
                y: 35.0,
                z: 400000.0,
                coordinate_system: None,
            },
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: Some(8450.0),
            include_reference: true,
            vehicle_attitude: None,
        }
    }

    /// Create batch gain request
    pub fn simple_batch_request(count: usize) -> BatchGainRequest {
        let mut evaluations = Vec::new();
        for i in 0..count {
            let mut req = simple_gain_request_ecef();
            req.frequency_mhz = 8000.0 + (i as f64 * 10.0);
            evaluations.push(req);
        }
        BatchGainRequest { evaluations }
    }

    /// Create heatmap request
    pub fn simple_heatmap_request() -> HeatmapRequest {
        // Use geodetic_to_ecef to generate valid ECEF coordinates
        use antenna_model::model::coordinates_3d::geodetic_to_ecef;
        let (veh_x, veh_y, veh_z) = geodetic_to_ecef(-118.1234, 34.5678, 100.0).unwrap();
        let (feed_x, feed_y, feed_z) = geodetic_to_ecef(-118.124, 34.568, 105.0).unwrap();

        HeatmapRequest {
            antenna_id: "test_simple".to_string(),
            feed_id: "primary".to_string(),
            vehicle_position: Position3D {
                x: veh_x,
                y: veh_y,
                z: veh_z,
                // Earth-surface ECEF values are ~2-6 Mm, below 6400 km threshold;
                // set explicit tag so they are not misclassified as Geodetic.
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            reflector_boresight: Position3D {
                x: veh_x + 100.0, // Slightly offset for boresight direction
                y: veh_y + 100.0,
                z: veh_z + 1000.0,
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            feed_position: Position3D {
                x: feed_x,
                y: feed_y,
                z: feed_z,
                coordinate_system: Some(CoordinateSystem::ECEF),
            },
            frequency_mhz: 8400.0,
            pointing_frequency_mhz: None,
            grid_config: GridConfig::Rectangular {
                azimuth_range_deg: RangeConfig {
                    min: 0.0,
                    max: 10.0,
                    step: 5.0,
                },
                elevation_range_deg: RangeConfig {
                    min: 0.0,
                    max: 10.0,
                    step: 5.0,
                },
            },
        }
    }

    /// Create request for uncalibrated antenna
    pub fn uncalibrated_antenna_request() -> GainRequest {
        let mut req = simple_gain_request_ecef();
        req.antenna_id = "test_uncalibrated".to_string();
        req.feed_id = "x_band".to_string();
        req.frequency_mhz = 8000.0;
        req
    }

    /// Create request for multi-feed antenna
    pub fn multi_feed_request(feed_id: &str, freq_mhz: f64) -> GainRequest {
        let mut req = simple_gain_request_ecef();
        req.antenna_id = "test_large".to_string();
        req.feed_id = feed_id.to_string();
        req.frequency_mhz = freq_mhz;
        req
    }
}

/// Response validation helpers
pub mod validators {
    use super::*;

    /// Validate GainResponse has required fields
    pub fn validate_gain_response(response: &GainResponse) -> Result<(), String> {
        if response.antenna_id.is_empty() {
            return Err("antenna_id is empty".to_string());
        }
        if response.feed_id.is_empty() {
            return Err("feed_id is empty".to_string());
        }
        if response.gain_db.is_nan() || response.gain_db.is_infinite() {
            return Err(format!("Invalid gain_db: {}", response.gain_db));
        }
        if response.metadata.computation_time_ms < 0.0 {
            return Err("Invalid computation time".to_string());
        }
        Ok(())
    }

    /// Validate BatchGainResponse has required fields
    pub fn validate_batch_response(response: &BatchGainResponse) -> Result<(), String> {
        if response.results.is_empty() {
            return Err("No results in batch response".to_string());
        }
        for result in &response.results {
            validate_gain_response(result)?;
        }
        if response.metadata.total_computation_time_ms < 0.0 {
            return Err("Invalid total computation time".to_string());
        }
        Ok(())
    }

    /// Validate HeatmapResponse has required fields
    pub fn validate_heatmap_response(response: &HeatmapResponse) -> Result<(), String> {
        if response.antenna_id.is_empty() {
            return Err("antenna_id is empty".to_string());
        }
        if response.metadata.points_evaluated == 0 {
            return Err("No points evaluated".to_string());
        }
        if response.metadata.computation_time_ms < 0.0 {
            return Err("Invalid computation time".to_string());
        }
        match &response.grid {
            GridData::Rectangular {
                azimuth_values,
                elevation_values,
                loss_db,
            } => {
                if azimuth_values.is_empty() || elevation_values.is_empty() {
                    return Err("Empty grid axes".to_string());
                }
                if loss_db.len() != elevation_values.len() {
                    return Err("Loss matrix row count mismatch".to_string());
                }
                for row in loss_db {
                    if row.len() != azimuth_values.len() {
                        return Err("Loss matrix column count mismatch".to_string());
                    }
                }
            }
            GridData::H3 { .. } => {
                // H3 validation
            }
        }
        Ok(())
    }

    /// Validate calibration status is present and valid
    #[allow(dead_code)]
    pub fn validate_calibration_status(
        status: &Option<CalibrationStatusInfo>,
    ) -> Result<(), String> {
        match status {
            Some(s) => {
                if s.accuracy_estimate_db < 0.0 {
                    return Err("Invalid accuracy estimate".to_string());
                }
                Ok(())
            }
            None => Err("Calibration status missing".to_string()),
        }
    }
}

/// Raw HTTP/1.1 client for wire-level cases `reqwest` will not produce.
///
/// Specifically: sending a request body with `Transfer-Encoding: chunked` and
/// **no** `content-length` header (roadmap S1b). `reqwest` only emits chunked
/// bodies via its optional `stream` feature, and the point of these tests is the
/// exact wire framing, so we speak HTTP directly instead of adding a dev-dep.
pub mod raw_http {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    /// A parsed raw HTTP response.
    pub struct RawResponse {
        pub status: u16,
        /// Response body with any `Transfer-Encoding: chunked` framing removed.
        pub body: String,
    }

    impl RawResponse {
        /// Deserialize the body as JSON.
        pub fn json<T: serde::de::DeserializeOwned>(&self) -> T {
            serde_json::from_str(&self.body)
                .unwrap_or_else(|e| panic!("response body is not valid JSON ({e}): {}", self.body))
        }
    }

    /// POST `body` to `base_url + path` framed as `Transfer-Encoding: chunked`,
    /// deliberately omitting `content-length`.
    ///
    /// The body is sent as a single chunk. Keep test bodies small (a few KB): the
    /// server may reject and stop draining before we finish writing, and a body
    /// larger than the socket buffers would then deadlock the write.
    pub async fn post_chunked(
        base_url: &str,
        path: &str,
        body: &str,
        request_id: &str,
    ) -> Result<RawResponse, Box<dyn std::error::Error>> {
        let authority = base_url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(authority).await?;

        let head = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: {authority}\r\n\
             Content-Type: application/json\r\n\
             Transfer-Encoding: chunked\r\n\
             x-request-id: {request_id}\r\n\
             Connection: close\r\n\
             \r\n"
        );
        // Single chunk: <hex-length> CRLF <data> CRLF, then the zero-length terminator.
        let framed = format!("{head}{:x}\r\n{body}\r\n0\r\n\r\n", body.len());

        // Ignore write errors: on the over-limit path the server answers and closes
        // mid-write, which is the behavior under test, not a failure.
        let _ = stream.write_all(framed.as_bytes()).await;
        let _ = stream.flush().await;

        read_response(&mut stream).await
    }

    /// Send a bodyless `GET` (no `content-length`, no `transfer-encoding`).
    pub async fn get(
        base_url: &str,
        path: &str,
    ) -> Result<RawResponse, Box<dyn std::error::Error>> {
        let authority = base_url.trim_start_matches("http://");
        let mut stream = TcpStream::connect(authority).await?;

        let req = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;
        stream.flush().await?;

        read_response(&mut stream).await
    }

    /// Read to EOF (we always send `Connection: close`), then split head from body
    /// and undo chunked framing if the server used it.
    async fn read_response(
        stream: &mut TcpStream,
    ) -> Result<RawResponse, Box<dyn std::error::Error>> {
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await?;
        let text = String::from_utf8_lossy(&raw).to_string();

        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or("malformed HTTP response: no header/body separator")?;

        let status: u16 = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or("malformed HTTP response: no status line")?
            .parse()?;

        let chunked = head.lines().skip(1).any(|l| {
            l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.contains("chunked")
        });

        let body = if chunked {
            decode_chunked(body)
        } else {
            body.to_string()
        };

        Ok(RawResponse { status, body })
    }

    /// Minimal chunked-transfer decoder: `<hex-len> CRLF <data> CRLF …  0 CRLF CRLF`.
    fn decode_chunked(body: &str) -> String {
        let mut out = String::new();
        let mut rest = body;

        while let Some((size_line, after)) = rest.split_once("\r\n") {
            // Chunk extensions (`;ext=val`) are legal after the size; ignore them.
            let size_hex = size_line.split(';').next().unwrap_or("").trim();
            let size = match usize::from_str_radix(size_hex, 16) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if after.len() < size {
                break;
            }
            out.push_str(&after[..size]);
            rest = after[size..].trim_start_matches("\r\n");
        }

        out
    }
}
