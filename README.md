# Antenna Model Service

A high-performance antenna gain modeling system for parabolic dish antennas with steerable feeds. The service provides REST API access to calibrated antenna models, supporting real-time queries for G/T (Gain-to-Temperature) predictions based on antenna orientation and frequency.

## Overview

This service implements a sophisticated 4D B-spline interpolation engine to model antenna performance across:
- Azimuth (0-360 degrees)
- Elevation (0-90 degrees)
- Frequency (100 MHz - 50 GHz)
- Temperature (constant in current implementation)

**Key Features:**
- High accuracy: ±1 dB for main lobe and first sidelobe
- Low latency: 50-100ms p95 response time
- REST API with batch processing and heatmap generation
- Kubernetes-native deployment
- Production-ready with comprehensive monitoring and observability

## Project Structure

```
antenna-model/
├── antenna-model/           # Main service (REST API)
│   └── src/
│       ├── api/            # REST API layer (routes, handlers, middleware)
│       ├── service/        # Business logic (evaluator, validator, batch)
│       ├── model/          # Computation engine (interpolation, B-spline)
│       ├── data/           # Data management (repository, loader, types)
│       ├── config/         # Configuration system
│       └── main.rs         # Service entry point
│
├── calibrate/              # Calibration CLI tool
│   └── src/
│       ├── parser.rs       # CSV measurement parser
│       ├── fitter.rs       # B-spline fitting
│       ├── validator.rs    # Validation logic
│       └── main.rs         # CLI entry point
│
├── calibration_data/       # Pre-computed calibration artifacts
│   └── antennas.toml      # Antenna configuration
│
├── config/                 # Runtime configuration
├── docs/                   # Documentation
└── tests/                  # Integration and performance tests
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

# Run tests
cargo test --all

# Run benchmarks
cargo bench
```

### Running the Service

```bash
# Run locally with default configuration
cargo run --release --bin antenna-model

# Or run with custom configuration
CONFIG_PATH=/path/to/config.toml cargo run --release --bin antenna-model

# Service will start on http://localhost:3000 by default
```

### Using the Calibration Tool

```bash
# Generate calibration artifacts from measurement data
cargo run --release --bin calibrate -- \
  --input measurements/antenna_1.csv \
  --output calibration_data/antenna_1.bin \
  --antenna-id antenna_1 \
  --validate
```

## API Usage

### Single Evaluation

```bash
curl -X POST http://localhost:3000/api/v1/evaluate \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "azimuth_deg": 45.0,
    "elevation_deg": 30.0,
    "frequency_mhz": 8400.0
  }'
```

Response:
```json
{
  "antenna_id": "antenna_1",
  "g_over_t_db": 41.2,
  "warnings": [],
  "metadata": {
    "computation_time_ms": 1.2,
    "extrapolated": false
  }
}
```

### Batch Evaluation

```bash
curl -X POST http://localhost:3000/api/v1/evaluate/batch \
  -H "Content-Type: application/json" \
  -d '{
    "evaluations": [
      {
        "antenna_id": "antenna_1",
        "azimuth_deg": 45.0,
        "elevation_deg": 30.0,
        "frequency_mhz": 8400.0
      },
      {
        "antenna_id": "antenna_1",
        "azimuth_deg": 180.0,
        "elevation_deg": 15.0,
        "frequency_mhz": 2200.0
      }
    ]
  }'
```

### Heatmap Generation

```bash
curl -X POST http://localhost:3000/api/v1/heatmap \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "frequency_mhz": 8400.0,
    "azimuth_range": {
      "min": 0.0,
      "max": 360.0,
      "step": 5.0
    },
    "elevation_range": {
      "min": 0.0,
      "max": 90.0,
      "step": 2.0
    }
  }'
```

### Health Check

```bash
curl http://localhost:3000/health
```

### Service Status

```bash
curl http://localhost:3000/status
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

Configuration is loaded from `config/service.toml`:

```toml
[server]
host = "0.0.0.0"
port = 3000

[calibration]
data_dir = "/app/calibration_data"
config_file = "antennas.toml"

[logging]
level = "info"
format = "json"
```

### Antenna Configuration

Antennas are configured in `calibration_data/antennas.toml`:

```toml
[[antennas.configs]]
id = "antenna_1"
name = "Deep Space Network 34m"
calibration_file = "antenna_1.bin"
enabled = true

[[antennas.configs]]
id = "antenna_2"
name = "Ground Station Array Element"
calibration_file = "antenna_2.bin"
enabled = true
```

## Development

### Development Setup

```bash
# Install development dependencies
cargo install cargo-watch cargo-edit cargo-tarpaulin

# Run with auto-reload
cargo watch -x run

# Run specific test
cargo test test_name -- --nocapture

# Generate code coverage
cargo tarpaulin --out Html --output-dir coverage/
```

### Running Tests

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test --test '*'

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
   - G/T measurements across azimuth, elevation, and frequency
   - CSV format: `azimuth_deg,elevation_deg,frequency_mhz,temperature_k,g_over_t_db`

2. **Run Calibration Tool**
   ```bash
   calibrate --input measurements.csv \
             --output calibration.bin \
             --antenna-id my_antenna \
             --validate
   ```

3. **Validate Calibration**
   - Review fit quality metrics (RMSE, R²)
   - Check interpolation accuracy
   - Verify extrapolation behavior

4. **Deploy Calibration**
   - Copy `.bin` file to `calibration_data/`
   - Update `antennas.toml` configuration
   - Rebuild and deploy service

## Monitoring and Observability

### Structured Logging

All requests are logged with structured fields:
```json
{
  "timestamp": "2025-01-15T10:30:45Z",
  "level": "INFO",
  "target": "antenna_model::api",
  "message": "Evaluation completed",
  "antenna_id": "antenna_1",
  "azimuth_deg": 45.0,
  "elevation_deg": 30.0,
  "frequency_mhz": 8400.0,
  "g_over_t_db": 41.2,
  "computation_time_ms": 1.2,
  "extrapolated": false,
  "request_id": "uuid-1234"
}
```

### Health Probes

- **Liveness**: `GET /health` - Service is running
- **Readiness**: `GET /health` - Service is ready (calibration data loaded)
- **Status**: `GET /status` - Detailed service information

## Troubleshooting

### Service won't start
- Check calibration data files exist in configured directory
- Verify `antennas.toml` configuration is valid
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

- [Design Document](docs/antenna-model-design-doc.md) - Detailed physical models and mathematical formulation
- [Architecture Document](docs/architecture.md) - System architecture and deployment
- [Implementation Plan](docs/implementation-plan.md) - Sprint-by-sprint development plan
- [API Documentation](http://localhost:3000/api/docs) - Interactive API documentation (when service is running)

## Contact

For questions, issues, or feature requests, please open an issue on the project repository.

---

**Status**: Active Development | **Version**: 0.1.0 | **Last Updated**: 2025-10-22
