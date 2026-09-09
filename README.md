# Antenna Model Service

A high-performance antenna gain modeling system for parabolic dish antennas with steerable feeds. The service provides REST API access to calibrated antenna models, supporting real-time queries for G/T (Gain-to-Temperature) predictions based on antenna orientation and frequency.

## Overview

This service implements a **hybrid physical optics + correction surface model** for parabolic dish antenna performance prediction. The system combines:

1. **Physical Optics Computation Engine**: Aperture integration with phase functions (path, coma aberration, surface error, mesh effects) providing physics-based gain predictions
2. **Optional Correction Surface**: 4D B-spline interpolation (azimuth, elevation, frequency, temperature) for residual error corrections when calibration data is available

The hybrid approach enables graceful degradation from fully calibrated antennas (±1 dB accuracy) to uncalibrated antennas using design specifications only (±2 dB loss accuracy).

**Key Features:**
- **Flexible Calibration**: Support for fully calibrated, partially calibrated (boresight), and uncalibrated antennas
- **High Accuracy**: ±1 dB for fully calibrated antennas, graceful degradation for partial/uncalibrated
- **Low Latency**: 50-100ms p95 response time for single queries
- **REST API**: Comprehensive endpoints with batch processing and heatmap generation
- **3D Coordinate Support**: ECEF and Geodetic positions, each tagged with a required `coordinate_system` field (never inferred — see below)
- **Kubernetes-native**: Production-ready with health probes and structured logging
- **Multi-feed Support**: Multiple feeds per antenna with independent calibrations

## Calibration Statuses

The service supports three calibration levels with graceful accuracy degradation:

| Status | Accuracy (Absolute) | Accuracy (Loss) | Test Time | Use Case |
|--------|-------------------|-----------------|-----------|----------|
| **Fully Calibrated** | ±1 dB | ±1 dB | ~8 hours | Production operations, high-precision analysis |
| **Partially Calibrated (Boresight)** | ±1 dB @ boresight<br>±2-3 dB off-axis | ±1-2 dB | ~1 hour | Rapid commissioning, boresight verification |
| **Uncalibrated (Design Specs)** | ±3-5 dB | ±2 dB | 0 hours | Loss analysis, planning, design validation |

**Key Insight:** Loss (relative gain) has better accuracy than absolute gain for uncalibrated antennas (±2 dB vs ±3-5 dB) due to systematic error cancellation when comparing two pointing directions.

**Upgrade Path:** Antennas can be incrementally upgraded: Uncalibrated → Boresight → Fully Calibrated, with each step providing a valid calibration artifact.

**See also:** [Calibration Workflow Guide](docs/calibration-workflow-guide.md) for detailed workflows and examples.

## Project Structure

A three-crate Cargo workspace. The physics engine lives in `antenna-core` so the
CLI does not compile the web stack:

```
antenna-model/
├── antenna-core/           # Physics engine + calibration-artifact layer (no web stack)
│   └── src/
│       ├── model/          # bessel, coordinates, coordinates_3d, correction_interpolator,
│       │                   #   edge_cases, fft, geometry, illumination, integration, mesh,
│       │                   #   pattern, phase, ray_trace
│       ├── data/           # Artifact layer: types.rs + loader.rs (ANTC container)
│       ├── error.rs        # Shared error vocabulary
│       └── warnings.rs     # Shared WarningCode / ApiWarning vocabulary
│
├── antenna-model/          # REST API service binary — depends on antenna-core
│   └── src/
│       ├── api/            # REST layer (poem framework: routes, handlers, middleware)
│       ├── service/        # Business logic (evaluator, batch, cache, heatmap,
│       │                   #   h3_link_budget, validator)
│       ├── data/           # repository.rs (types/loader re-exported from antenna-core)
│       ├── config/         # Configuration system
│       └── main.rs         # Service entry point
│
├── calibrate/              # Calibration CLI tool — depends on antenna-core
│   └── src/
│       ├── parser.rs             # CSV measurement parser
│       ├── parameter_tuner.rs    # Nelder-Mead simplex optimizer
│       ├── correction_surface.rs # B-spline/RBF fitting
│       ├── validator.rs          # Cross-validation
│       ├── artifact_export.rs    # Service-loadable artifact (3D→4D bridge)
│       └── main.rs               # CLI entry point
│
├── calibration_data/       # antennas.yaml + design_specs/ (no .bin ships — see below)
├── config/                 # Runtime configuration (service.yaml)
├── docs/                   # Documentation
├── examples/               # Request/response examples, pinned by drift tests
└── scripts/                # check.sh and the worked artifact-generation path
```

## Quick Start

### Prerequisites

- Rust 2024 edition (rustc 1.75+)
- Docker (for containerized deployment)
- Kubernetes cluster (for production deployment)

### Building from Source

```bash
# Build both the service and calibration tool
cargo build --release

# Run tests (nextest runs each test in its own process, in parallel)
cargo nextest run --workspace
cargo test --doc --workspace   # nextest does not run doctests
./scripts/check.sh             # everything CI runs, including both test tiers

# Run benchmarks
cargo bench
```

### Running the Service

```bash
# Run locally with default configuration
cargo run --release --bin antenna-model

# Or run with custom configuration
CONFIG_PATH=/path/to/service.yaml cargo run --release --bin antenna-model

# Service will start on http://localhost:3000 by default
```

### What You Get on a Clean Checkout

**No `.bin` calibration artifacts ship in this repository.** A calibration
artifact is a build output — derived from measurements *plus* this codebase's own
physics model — so a committed one goes stale the moment either changes and
nothing in the build can notice. Every input is committed; the artifact is not.

The service still starts **healthy and useful**, because five antennas are
configured from design specifications alone and need no artifact:

```bash
curl -s http://localhost:3000/health   # {"status":"healthy"}
curl -s http://localhost:3000/status | jq '{antenna_count, antenna_ids}'
```

```json
{
  "antenna_count": 5,
  "antenna_ids": [
    "dsn_13m_uncalibrated",
    "dsn_34m_uncalibrated",
    "dsn_70m_uncalibrated",
    "gbt_100m_uncalibrated",
    "gs_3.7m_uncalibrated"
  ]
}
```

Those five are **uncalibrated**: absolute gain is accurate to ±3–5 dB, loss to
±2 dB, and every response says so in a `uncalibrated` warning and its
`calibration_status` block. That is the intended default, not a broken install.

`calibration_data/antennas.yaml` also holds four **disabled templates** naming
`.bin` files a clean checkout does not contain. They document the YAML shape of
the fully- and partially-calibrated levels; generate the artifact and flip
`enabled` to make one real.

`/health` reports `degraded` only when **no** antenna loads at all, and a
degraded instance never becomes ready (`/ready` stays 503), so it receives no
traffic.

### Generating a Calibration Artifact

`scripts/generate-cr159703-artifact.sh` is a complete worked example that runs
from a clean checkout with no external data:

```bash
./scripts/generate-cr159703-artifact.sh /tmp/my-calibration
```

It builds both binaries, synthesizes a real-anchored measurement grid from
digitized NASA CR-159703 data committed under
`antenna-model/tests/fixtures/`, fits the correction surface with the real
`calibrate` binary, and writes the artifact **outside the repository tree**:

```
cr159703_122m.bin            the calibration artifact the service loads
cr159703_grid.csv            the synthesized measurement grid
cr159703_grid_summary.json   fabrications, anchor table, injected residual RMS
cr159703_report.json         validation report (RMSE, cross-validation, outliers)
cr159703_metadata.json       artifact metadata sidecar
```

Do not commit what it produces. The measurement grid is **model-filled, not
measured** — only the residual at 19 digitized peak angles comes from published
measurements — and the script prints that on every run.

To serve the result, point `calibration.data_directory` at the output directory
and add an entry to `antennas.yaml` with a matching `calibration_file`.

For the general path — measurement CSV format, mode selection, and how to read
the fit quality — see
[docs/calibration-workflow-guide.md](docs/calibration-workflow-guide.md) §12.

### Using the Calibration Tool

#### Boresight Calibration (Fast Mode)

For rapid commissioning with ~1 hour test time:

```bash
# Generate boresight-calibrated artifact from frequency sweep at boresight
cargo run --release --bin calibrate -- \
  --calibration-mode boresight \
  --input measurements/boresight_xband.csv \
  --design-specs design_specs/antenna_1.yaml \
  --output /var/lib/antenna-model/antenna_1_xband_boresight.bin \
  --antenna-id antenna_1 \
  --feed-id x_band \
  --verbose
```

See [examples/README_boresight.md](examples/README_boresight.md) for measurement format and detailed usage.

#### Full Grid Calibration

For production-grade accuracy with ~8 hour test time:

```bash
# Generate fully-calibrated artifact from dense measurement grid
cargo run --release --bin calibrate -- \
  --calibration-mode full \
  --input measurements/antenna_1_full_grid.csv \
  --output /var/lib/antenna-model/antenna_1.bin \
  --antenna-id antenna_1 \
  --feed-id x_band \
  --validate
```

#### Uncalibrated Antenna (Design Specs Only)

No calibration tool needed — the design specifications live inline in
`calibration_data/antennas.yaml`, and the antenna is usable on the next restart:

```yaml
antennas:
  - id: "my_antenna"
    name: "My Ground Station"
    calibration_status: "uncalibrated"
    enabled: true

    design_specs:
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.5

      feeds:
        - id: "x_band_feed"
          name: "X-Band Feed"
          position: [0.0, 0.0, 0.0]   # offset FROM THE FOCAL POINT; on-axis = zeros
          q_factor: 2.04              # ~-11 dB edge taper at f/D 0.5
          phase_center_offset_m: 0.0
          frequency_range: [7100.0, 8500.0]

      mesh: null                      # or mesh_spacing_mm / wire_diameter_mm

    validity_ranges:
      azimuth_range: [0.0, 360.0]
      elevation_range: [0.0, 90.0]
      frequency_range: [7100.0, 8500.0]
      temperature_k: 290.0
```

Note `feeds[].position` is the feed's design offset **from the focal point**, not
a vertex-origin position — an on-axis feed is `[0, 0, 0]`. Reading it the other
way puts the feed at `z ≈ 2f` and costs ~27 dB of boresight gain; see the field's
doc comment in `antenna-core/src/data/types.rs`.

## API Usage

### Single Evaluation

The gain endpoints take **3D geometry**, not angles: you supply the vehicle's
position and attitude, where the reflector and feed are pointed, and where the
emitter is. The service derives the off-boresight angle itself. Every position
carries a **required** `coordinate_system` tag of `"ecef"` (x, y, z metres from
Earth's centre) or `"geodetic"` (lon°, lat°, alt m) — it is never inferred, and
omitting it is a 400 naming the field.

Endpoints: `POST /api/v1/gain`, `POST /api/v1/gain/batch`, `POST /api/v1/heatmap`,
`POST /api/v1/h3-heatmap`, plus `GET /api/v1/antennas[/:id[/feeds[/:feed_id]]]`.

<!-- api-example: GainRequest -->
```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "gs_3.7m_uncalibrated",
    "feed_id": "x_band_feed",
    "vehicle_position": {
      "x": -118.0, "y": 34.0, "z": 500.0,
      "coordinate_system": "geodetic"
    },
    "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
    "reflector_boresight": {
      "x": -118.0, "y": 34.0, "z": 510.0,
      "coordinate_system": "geodetic"
    },
    "feed_pointing_location": {
      "x": -118.0, "y": 34.0, "z": 505.0,
      "coordinate_system": "geodetic"
    },
    "emitter_position": {
      "x": -100.0, "y": 35.0, "z": 500000.0,
      "coordinate_system": "geodetic"
    },
    "frequency_mhz": 8200.0,
    "include_reference": false
  }'
```

Response (verified against a running service, 2026-08-16 — warning text abridged):

<!-- api-example: GainResponse -->
```json
{
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "x_band_feed",
  "gain_db": -6.7320729605282175,
  "geometry": {
    "physical_feed_offset_m": { "x": 0.05, "y": 0.0, "z": 0.0 },
    "emitter_azimuth_deg": 352.37622385040544,
    "emitter_elevation_deg": 81.31153325033851
  },
  "warnings": [
    { "code": "spillover_significant", "message": "Estimated spillover 17.1% may reduce aperture efficiency." },
    { "code": "uncalibrated", "message": "Antenna is uncalibrated (using design specifications)." },
    { "code": "off_axis_unvalidated", "message": "Query is beyond the validated main-beam region." }
  ],
  "metadata": {
    "computation_time_ms": 7.109958,
    "extrapolated": false,
    "spillover_loss_db": -0.8134714045459502
  },
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 3.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_application": "unavailable",
    "correction_applied": false,
    "parameters_source": "design_specifications"
  }
}
```

Warnings are **typed**: `code` is the contract, `message` is not — never branch
on message text. The full vocabulary is in
[docs/api-documentation.md](docs/api-documentation.md).

### Batch, Heatmap, and H3 Link Budget

These take larger payloads, so run them straight from the checked-in examples. Two
guards cover those files: one pins each against its schema, and one POSTs each to its
real endpoint and requires a fully-successful response.

```bash
curl -X POST http://localhost:3000/api/v1/gain/batch \
  -H "Content-Type: application/json" \
  -d @examples/requests/batch_request.json

curl -X POST http://localhost:3000/api/v1/heatmap \
  -H "Content-Type: application/json" \
  -d @examples/requests/heatmap_request.json

curl -X POST http://localhost:3000/api/v1/h3-heatmap \
  -H "Content-Type: application/json" \
  -d @examples/requests/h3_link_budget_request.json
```

One thing to know about batch: it returns **HTTP 200 even when individual items
fail** — each result carries either a value or a typed `error`, and
`metadata.failure_count` summarizes. Read `failure_count`, never the status code.
That is not a hypothetical caveat: `batch_request.json` itself shipped for two
months with all three of its items failing under a 200 (roadmap **D30**), because
the drift tests checked only that each example **deserializes into its schema**,
not that it *computes*. Both are now checked —
`tests/integration/example_execution_tests.rs` POSTs every committed example to
its real endpoint and requires a fully-successful response.

`/heatmap` serves rectangular grids only. The H3 grid is the separate
`/h3-heatmap` endpoint.

### Health, Readiness, and Status

```bash
curl http://localhost:3000/health   # liveness — always 200; "healthy" or "degraded"
curl -i http://localhost:3000/ready # readiness — 200 when serving, 503 otherwise
curl http://localhost:3000/status   # version, uptime, loaded antennas
```

## Docker Deployment

### Build Docker Image

```bash
docker build -t antenna-model:latest .
```

### Run with Docker

```bash
docker run -p 3000:3000 \
  -v $(pwd)/calibration_data:/app/calibration_data \
  -v $(pwd)/config:/app/config \
  -e RUST_LOG=info,antenna_model=debug \
  antenna-model:latest
```

### Docker Compose

```bash
docker-compose up
```

## Kubernetes Deployment

### Using kubectl

```bash
# Apply Kubernetes manifests
kubectl apply -f k8s/

# Check deployment status
kubectl get pods -l app=antenna-model

# View logs
kubectl logs -f deployment/antenna-model-service

# Test service
kubectl port-forward service/antenna-model-service 3000:80
curl http://localhost:3000/health
```

### Using Helm

```bash
# Install with Helm
helm install antenna-model ./helm/antenna-model \
  --namespace antenna-model \
  --create-namespace

# Upgrade release
helm upgrade antenna-model ./helm/antenna-model

# Uninstall
helm uninstall antenna-model --namespace antenna-model
```

## Configuration

### Service Configuration

Configuration is loaded from `config/service.yaml` (override the path with
`CONFIG_PATH`). Abridged — see the file for the full commentary:

```yaml
server:
  host: "127.0.0.1"
  port: 3000
  request_timeout_secs: 30
  max_body_size_bytes: 10485760   # 10 MB
  shutdown_readiness_delay_secs: 0
  shutdown_timeout_secs: 25

calibration:
  data_directory: "calibration_data"
  antenna_config_file: "calibration_data/antennas.yaml"
  fail_fast: true

logging:
  level: "info"
  format: "text"        # use "json" in production
  include_location: false

performance:
  worker_threads: 0     # 0 = auto-detect
  max_batch_size: 1000
  enable_parallel_processing: true
```

### Antenna Configuration

Antennas are configured in `calibration_data/antennas.yaml`. A calibrated entry
references an artifact by filename, resolved against `data_directory`:

```yaml
antennas:
  - id: "my_calibrated_antenna"
    name: "My Antenna - Fully Calibrated"
    calibration_status: "fully_calibrated"
    # Must exist under `calibration.data_directory` before you enable this.
    calibration_file: "my_calibrated_antenna.bin"
    enabled: true
```

**Enable such an entry only once the `.bin` exists.** No artifact ships in this
repository (see above), and the default `config/service.yaml` sets
`fail_fast: true`, so an enabled entry pointing at a missing file stops the
service from starting. That is why the four calibrated entries in the shipped
`antennas.yaml` — `dsn_34m_full` and the three partially-calibrated ones — are
`enabled: false`: they are templates, and flipping one without generating its
artifact first is exactly the failure this note exists to prevent.

An uncalibrated entry carries `design_specs` inline instead and needs no
artifact — see the design-spec example above. The shipped file contains five
enabled uncalibrated antennas and four disabled templates; read its header
before editing.

## Development

### Development Setup

```bash
# Install development dependencies
cargo install cargo-nextest cargo-watch cargo-edit cargo-tarpaulin

# Run with auto-reload
cargo watch -x run

# Run specific test
cargo nextest run test_name -- --nocapture

# Generate code coverage
cargo tarpaulin --out Html --output-dir coverage/
```

### Running Tests

```bash
# Everything CI runs, in CI's order. This is the only complete check: no single
# nextest invocation covers the doctests or the package-scoped guards.
./scripts/check.sh

# Dev inner loop (excludes the four slow calibration scenarios; see .config/nextest.toml)
cargo nextest run --workspace

# Both test tiers
cargo nextest run --workspace --profile full

# Unit tests
cargo nextest run --lib

# Integration tests
cargo nextest run --test '*'

# Doc tests (not covered by nextest)
cargo test --doc --workspace

# Performance benchmarks
cargo bench

# Load tests (requires k6)
k6 run tests/load/load_test_scenarios.js
```

### Code Quality

```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Security audit
cargo audit

# Generate documentation
cargo doc --open
```

## Performance Characteristics

| Metric | Target | Typical |
|--------|--------|---------|
| Single evaluation latency (p95) | <100ms | 50-80ms |
| Batch throughput | 1-20 req/s | 10-15 req/s |
| Startup time | <10s | 5-8s |
| Memory footprint | <512MB | 256-384MB |

## Architecture

The service follows a layered architecture:

1. **REST API Layer** (poem framework)
   - Request routing and validation
   - Serialization/deserialization
   - Middleware (logging, timing, error handling)

2. **Service/Business Logic Layer**
   - Request validation
   - Antenna configuration lookup
   - Batch processing coordination
   - Warning generation

3. **Model Computation Engine**
   - 4D B-spline interpolation
   - Extrapolation handling
   - Performance-optimized evaluation

4. **Calibration Data Repository**
   - In-memory calibration storage
   - Thread-safe concurrent access
   - Fast model coefficient lookup

For detailed architecture documentation, see [docs/architecture.md](docs/architecture.md).

## Calibration Workflow

1. **Obtain Measurement Data**
   - G/T measurements across the pattern, in spherical coordinates about boresight
   - Full-mode CSV columns: `e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k`
   - Boresight-mode CSV columns: `frequency_mhz,g_over_t_db,temperature_k`
   - A full-mode fit needs enough points to determine its coefficients — the
     shipped knot counts declare up to 960, and cross-validation trains on a
     subset, so plan on ≥1440 points. Too few is a hard `UnderdeterminedFit`
     error, not a quietly bad fit.

2. **Run Calibration Tool**
   ```bash
   calibrate --calibration-mode full \
             --input measurements.csv \
             --output my_antenna.bin \
             --antenna-id my_antenna \
             --feed-id x_band \
             --antenna-class GroundStation_13m \
             --validate --report report.json --metadata metadata.json
   ```

3. **Validate Calibration**
   - Review fit quality metrics (RMSE, R²) in the report
   - Read the **per-fold** cross-validation RMSEs, not just the mean — a mean
     alone has hidden a 100× spread between folds
   - Check the angular-resolution assessment: if the knots cannot resolve the
     antenna's lobe period, `calibrate` warns and records it in the metadata.
     In-sample RMSE structurally cannot see that limitation.

4. **Deploy Calibration**
   - Copy the `.bin` file into the directory named by `calibration.data_directory`
   - Add or enable the entry in `calibration_data/antennas.yaml` with a matching
     `calibration_file`
   - Restart the service (hot reload is not implemented)

## Monitoring and Observability

### Structured Logging

All requests are logged with structured fields:

<!-- api-example: not-a-payload a tracing log line, not an API request or response — its antenna_id is a log field -->
```json
{
  "timestamp": "2026-08-16T10:30:45Z",
  "level": "INFO",
  "target": "antenna_model::api",
  "message": "Evaluation completed",
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "x_band_feed",
  "gain_db": -6.73,
  "computation_time_ms": 7.11,
  "warnings_count": 3,
  "request_id": "uuid-1234"
}
```

### Health Probes

- **Liveness**: `GET /health` — always 200 while responsive; body reports
  `healthy`, or `degraded` when no calibration data loaded. It deliberately never
  fails, because restarting a pod cannot fix missing calibration data.
- **Readiness**: `GET /ready` — 200 when serving; 503 during startup, after a
  failed calibration load, and for the whole graceful-shutdown drain window.
- **Status**: `GET /status` — version, uptime, loaded antenna count and IDs

## Troubleshooting

### Service won't start
- Check that any `.bin` files referenced by **enabled** entries exist in
  `calibration.data_directory`. With `fail_fast: true` a missing or corrupt
  artifact stops startup; the five default antennas need no artifact at all.
- Verify `calibration_data/antennas.yaml` is valid
- Review startup logs for detailed error messages
- Ensure port 3000 is available

### Slow response times
- Check concurrent request load
- Verify calibration model sizes are reasonable
- Review logs for extrapolation warnings (slower than interpolation)
- Monitor memory usage

### Inaccurate predictions
- Verify query is within calibrated ranges (check warnings)
- Review calibration quality metrics
- Ensure measurement data covers query regions
- Re-run calibration with higher knot density

For detailed troubleshooting, see [docs/operations/troubleshooting-guide.md](docs/operations/troubleshooting-guide.md).

## Contributing

We welcome contributions! Please see [docs/development/contributing.md](docs/development/contributing.md) for guidelines.

### Code Review Checklist

- [ ] Code follows Rust idioms and best practices
- [ ] All public APIs have documentation comments
- [ ] Tests cover both happy path and error cases
- [ ] No `unwrap()` or `expect()` in production code
- [ ] Performance-critical code is benchmarked
- [ ] Logging uses structured fields

## License

[Specify your license here]

## References

- [Calibration Workflow Guide](docs/calibration-workflow-guide.md) - Complete workflows for all calibration levels
- [Design Document](docs/antenna-model-design-doc.md) - Detailed physical models and mathematical formulation
- [Architecture Document](docs/architecture.md) - System architecture and deployment
- [Implementation Plan](docs/implementation-plan.md) - Sprint-by-sprint development plan
- [Boresight Calibration Examples](examples/README_boresight.md) - Boresight calibration tool usage
- [API Examples](examples/README.md) - API request/response examples for all calibration statuses
- [API Documentation](http://localhost:3000/api/docs) - Interactive API documentation (when service is running)

## Contact

For questions, issues, or feature requests, please open an issue on the project repository.

---

**Status**: Active Development | **Version**: 0.1.0 | **Last Updated**: 2026-08-16
