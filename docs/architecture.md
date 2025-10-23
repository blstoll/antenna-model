# Antenna Model Service - Application Architecture

## 1. Executive Summary

The Antenna Model Service is a high-accuracy antenna loss modeling system deployed as a Kubernetes service. It provides REST API access to calibrated antenna models, supporting real-time queries for G/T (Gain-to-Temperature) predictions based on antenna orientation and frequency. The system emphasizes accuracy through sophisticated interpolation of measured calibration data while maintaining sub-100ms response latencies.

### Key Characteristics
- **Language**: Rust
- **Deployment**: Kubernetes (on-premise)
- **API**: REST (poem framework), future gRPC support
- **Performance**: 1-20 requests/second per instance, 50-100ms latency
- **Calibration**: Offline, pre-deployment process
- **Data Volume**: <20MB calibration data per deployment

## 2. System Context

### 2.1 External Actors

```
┌─────────────────┐
│  Analytical     │
│     Tools       │──┐
└─────────────────┘  │
                     │
┌─────────────────┐  │    ┌──────────────────────┐
│   Real-time     │  │    │                      │
│   Processors    │──┼───▶│  Antenna Model API   │
└─────────────────┘  │    │                      │
                     │    └──────────────────────┘
┌─────────────────┐  │              │
│   Engineers     │──┘              │
│  (Calibration)  │                 ▼
└─────────────────┘         ┌──────────────┐
                            │   S3 Storage │
                            │  (Raw G/T    │
                            │   Tables)    │
                            └──────────────┘
```

### 2.2 Client Interaction Patterns

**Analytical Tools**
- Batch queries for loss heatmap generation
- Tolerance to higher latencies (seconds)
- Require detailed error information and warnings

**Real-time Processors**
- Single-point queries during operation
- Latency-sensitive (<100ms)
- Need fast failure modes

## 3. System Architecture

### 3.1 High-Level Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    Kubernetes Cluster                    │
│                                                          │
│  ┌─────────────────────────────────────────────────────┐ │
│  │           Antenna Model Service Pod(s)              │ │
│  │                                                     │ │
│  │  ┌────────────────────────────────────────────────┐ │ │
│  │  │              REST API Layer                    │ │ │
│  │  │              (poem framework)                  │ │ │
│  │  └──────────────┬─────────────────────────────────┘ │ │
│  │                 │                                   │ │
│  │  ┌──────────────▼─────────────────────────────────┐ │ │
│  │  │         Service/Business Logic                 │ │ │
│  │  │  - Request validation                          │ │ │
│  │  │  - Antenna configuration selection             │ │ │
│  │  │  - Batch processing coordination               │ │ │
│  │  └──────────────┬─────────────────────────────────┘ │ │
│  │                 │                                   │ │
│  │  ┌──────────────▼─────────────────────────────────┐ │ │
│  │  │         Model Computation Engine               │ │ │
│  │  │  - 4D interpolation (az, el, freq, [temp])     │ │ │
│  │  │  - B-spline evaluation                         │ │ │
│  │  │  - Extrapolation handling                      │ │ │
│  │  │  - [Future: GPU acceleration]                  │ │ │
│  │  └──────────────┬─────────────────────────────────┘ │ │
│  │                 │                                   │ │
│  │  ┌──────────────▼─────────────────────────────────┐ │ │
│  │  │       Calibration Data Repository              │ │ │
│  │  │  - In-memory calibration data                  │ │ │
│  │  │  - Model coefficients                          │ │ │
│  │  │  - Antenna configurations                      │ │ │
│  │  │  - Validity ranges                             │ │ │
│  │  └────────────────────────────────────────────────┘ │ │
│  │                                                     │ │
│  │  Observability:                                     │ │
│  │  - tokio-tracing (structured logging)               │ │
│  │  - [Future: Prometheus metrics]                     │ │
│  └─────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘

        ┌────────────────────────────────────┐
        │    Calibration Tool (CLI)          │
        │    (Development/Build-time)        │
        │                                    │
        │  - Reads G/T measurements          │
        │  - Fits B-spline models            │
        │  - Generates calibration artifacts │
        │  - Validates input data            │
        └────────────────────────────────────┘
                       │
                       ▼
              ┌─────────────────┐
              │ Calibration     │
              │ Artifacts       │
              │ (Source Control)│
              └─────────────────┘
```

### 3.2 Component Descriptions

#### 3.2.1 REST API Layer
**Responsibilities:**
- HTTP request handling and routing
- Request/response serialization (JSON)
- Authentication/authorization (if required)
- Rate limiting (future)
- Health and readiness endpoints

**Technology:**
- `poem` web framework
- `serde`/`serde_json` for serialization
- `tokio` async runtime

**Key Endpoints:**
- `GET /health` - Health check for K8s probes
- `GET /status` - Service status and loaded antennas
- `POST /api/v1/loss` - Single antenna loss computation
- `POST /api/v1/loss/batch` - Batch loss computation
- `POST /api/v1/heatmap` - Generate loss heatmap grid
- `GET /api/v1/antennas` - List available antennas
- `GET /api/v1/antennas/{id}` - Antenna configuration details

#### 3.2.2 Service/Business Logic Layer
**Responsibilities:**
- Input validation (range checking, parameter validation)
- Antenna configuration lookup
- Coordinate system transformations (if needed)
- Batch request orchestration
- Warning generation for out-of-range queries
- Error handling and response formatting

**Key Operations:**
- Validate request parameters against antenna calibration ranges
- Route requests to appropriate antenna models
- Coordinate parallel processing for batch requests
- Generate structured warnings for extrapolated values

#### 3.2.3 Model Computation Engine
**Responsibilities:**
- 4D B-spline interpolation (azimuth, elevation, frequency, temperature)
- Efficient coefficient evaluation
- Extrapolation for out-of-range queries
- Performance-critical path optimization

**Design Considerations:**
- **Initial Implementation**: CPU-based computation
  - Optimize for single-threaded performance
  - Use SIMD where applicable
- **Future Enhancement**: GPU acceleration
  - Design abstractions to support compute backend switching
  - Use trait-based interfaces for CPU/GPU implementations

**Libraries:**
- Custom interpolation implementation or `ndarray` for multi-dimensional operations
- `rayon` for CPU parallelization (batch processing)
- Future: CUDA/ROCm bindings or compute shaders

#### 3.2.4 Calibration Data Repository
**Responsibilities:**
- Load calibration data at startup
- Provide fast access to model coefficients
- Store metadata (validity ranges, antenna properties)
- Support multiple antenna configurations

**Data Structures:**
- In-memory hash map: `antenna_id -> AntennaModel`
- Each `AntennaModel` contains:
  - B-spline coefficients (4D tensor)
  - Knot vectors for each dimension
  - Validity ranges (min/max for az, el, freq)
  - Metadata (antenna name, calibration date, etc.)

**Data Loading:**
- Load from local filesystem at startup
- Fail-fast if calibration data is missing or corrupted
- Log loaded antenna configurations

## 4. Data Architecture

### 4.1 Calibration Data Flow

```
┌─────────────────┐
│  S3 Storage     │
│  Raw G/T Tables │
└────────┬────────┘
         │
         │ (Engineer downloads)
         ▼
┌─────────────────────────┐
│  Local Filesystem       │
│  measurements/          │
│  ├── antenna_1.csv      │
│  ├── antenna_2.csv      │
│  └── ...                │
└────────┬────────────────┘
         │
         │ (Calibration CLI)
         ▼
┌─────────────────────────┐
│  Calibration Process    │
│  - Parse measurements   │
│  - Fit B-spline models  │
│  - Validate fit quality │
│  - Serialize models     │
└────────┬────────────────┘
         │
         │ (Artifacts committed)
         ▼
┌─────────────────────────┐
│  Source Control         │
│  calibration_data/      │
│  ├── antenna_1.bin      │
│  ├── antenna_2.bin      │
│  └── antennas.yaml      │
└────────┬────────────────┘
         │
         │ (Docker build)
         ▼
┌─────────────────────────┐
│  Container Image        │
│  /app/calibration_data/ │
└────────┬────────────────┘
         │
         │ (Runtime load)
         ▼
┌─────────────────────────┐
│  Runtime Memory         │
│  In-memory models       │
└─────────────────────────┘
```

### 4.2 Data Formats

#### Measurement Data Format (Input to Calibration Tool)
CSV format with columns:
```
azimuth_deg, elevation_deg, frequency_mhz, temperature_k, g_over_t_db
```

#### Calibration Artifact Format
Binary format (MessagePack, bincode, or custom):
```rust
struct AntennaCalibration {
    antenna_id: String,
    metadata: CalibrationMetadata,
    model: BSplineModel4D,
    validity_ranges: ValidityRanges,
}

struct BSplineModel4D {
    coefficients: Vec<f64>,          // Flattened 4D array
    shape: [usize; 4],               // Dimensions
    knots_azimuth: Vec<f64>,
    knots_elevation: Vec<f64>,
    knots_frequency: Vec<f64>,
    knots_temperature: Vec<f64>,
    spline_order: u8,                // Typically 3 for cubic
}

struct ValidityRanges {
    azimuth_min_max: (f64, f64),
    elevation_min_max: (f64, f64),
    frequency_min_max: (f64, f64),
    temperature_const: f64,
}
```

#### Configuration File (antennas.yaml)
```yaml
antennas:
  configs:
    - id: "antenna_1"
      name: "Deep Space Network 34m"
      calibration_file: "antenna_1.bin"
      enabled: true

    - id: "antenna_2"
      name: "Ground Station Array Element"
      calibration_file: "antenna_2.bin"
      enabled: true
```

### 4.3 API Request/Response Schemas

#### Single Evaluation Request
```json
{
  "antenna_id": "antenna_1",
  "azimuth_deg": 45.0,
  "elevation_deg": 30.0,
  "frequency_mhz": 8400.0
}
```

#### Single Evaluation Response
```json
{
  "antenna_id": "antenna_1",
  "g_over_t_db": 41.2,
  "warnings": [
    "frequency_mhz outside calibrated range [8000, 8300]"
  ],
  "metadata": {
    "computation_time_ms": 1.2,
    "extrapolated": true
  }
}
```

#### Batch Request
```json
{
  "evaluations": [
    {
      "antenna_id": "antenna_1",
      "azimuth_deg": 45.0,
      "elevation_deg": 30.0,
      "frequency_mhz": 8400.0
    },
    {
      "antenna_id": "antenna_2",
      "azimuth_deg": 180.0,
      "elevation_deg": 15.0,
      "frequency_mhz": 2200.0
    }
  ]
}
```

#### Batch Response
```json
{
  "results": [
    {
      "antenna_id": "antenna_1",
      "g_over_t_db": 41.2,
      "warnings": [],
      "metadata": {
        "extrapolated": false
      }
    },
    {
      "antenna_id": "antenna_2",
      "g_over_t_db": 38.7,
      "warnings": [],
      "metadata": {
        "extrapolated": false
      }
    }
  ],
  "metadata": {
    "total_computation_time_ms": 15.3,
    "count": 2
  }
}
```

#### Heatmap Request
```json
{
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
}
```

#### Heatmap Response
```json
{
  "antenna_id": "antenna_1",
  "frequency_mhz": 8400.0,
  "grid": {
    "azimuth_values": [0.0, 5.0, 10.0, ...],
    "elevation_values": [0.0, 2.0, 4.0, ...],
    "g_over_t_db": [
      [41.2, 41.5, 41.8, ...],  // Row for elevation 0
      [40.8, 41.1, 41.4, ...],  // Row for elevation 2
      ...
    ]
  },
  "warnings": ["Some points extrapolated"],
  "metadata": {
    "points_evaluated": 3276,
    "computation_time_ms": 245.6
  }
}
```

## 5. Deployment Architecture

### 5.1 Container Structure

```
Dockerfile
├── Build Stage
│   ├── Rust toolchain
│   ├── Compile calibration tool
│   └── Compile API service
│
└── Runtime Stage
    ├── Minimal base image (distroless or alpine)
    ├── API service binary
    ├── /app/calibration_data/
    │   ├── antenna_1.bin
    │   ├── antenna_2.bin
    │   └── antennas.yaml
    └── /app/config/
        └── service.yaml
```

### 5.2 Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: antenna-model-service
spec:
  replicas: 3  # Adjust based on load
  selector:
    matchLabels:
      app: antenna-model
  template:
    metadata:
      labels:
        app: antenna-model
    spec:
      containers:
      - name: antenna-model
        image: antenna-model:latest
        ports:
        - containerPort: 3000
        resources:
          requests:
            cpu: "500m"
            memory: "256Mi"
          limits:
            cpu: "2000m"
            memory: "512Mi"
        livenessProbe:
          httpGet:
            path: /health
            port: 3000
          initialDelaySeconds: 5
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health
            port: 3000
          initialDelaySeconds: 5
          periodSeconds: 5
        env:
        - name: RUST_LOG
          value: "info,antenna_model=debug"
        - name: CONFIG_PATH
          value: "/app/config/service.yaml"
```

### 5.3 Service Configuration

```yaml
apiVersion: v1
kind: Service
metadata:
  name: antenna-model-service
spec:
  selector:
    app: antenna-model
  ports:
  - protocol: TCP
    port: 80
    targetPort: 3000
  type: ClusterIP  # Or LoadBalancer if external access needed
```

## 6. Operational Considerations

### 6.1 Logging Strategy

**Log Levels:**
- `ERROR`: System failures, configuration errors
- `WARN`: Out-of-range queries, degraded performance
- `INFO`: Request completions, startup/shutdown events
- `DEBUG`: Model evaluation details, interpolation steps
- `TRACE`: Detailed computation traces (development only)

**Structured Log Fields:**
```rust
// Example log entry
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
  "extrapolated": true,
  "request_id": "uuid-1234"
}
```

### 6.2 Monitoring (Future)

**Prometheus Metrics:**
- `antenna_model_requests_total` - Counter by antenna, status
- `antenna_model_request_duration_seconds` - Histogram
- `antenna_model_extrapolation_total` - Counter by dimension
- `antenna_model_errors_total` - Counter by error type
- `antenna_model_loaded_antennas` - Gauge

### 6.3 Error Handling

**Error Categories:**
```rust
enum ApiError {
    AntennaNotFound { antenna_id: String },
    InvalidParameter { param: String, reason: String },
    ComputationError { details: String },
    InternalError { details: String },
}
```

**HTTP Status Mappings:**
- 200: Success
- 400: Invalid parameters
- 404: Antenna not found
- 500: Internal computation/system error
- 503: Service unavailable (startup, shutdown)

### 6.4 Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Single evaluation latency | 50-100ms | p95 |
| Batch throughput | 1-20 req/s | Per instance |
| Startup time | <10s | Including data load |
| Memory footprint | <512MB | With all antennas |
| Calibration data size | <20MB | All antennas combined |

## 7. Development Workflow

### 7.1 Calibration Process

```bash
# Engineer workflow
# 1. Download measurements from S3
aws s3 cp s3://bucket/antenna_1_measurements.csv ./measurements/

# 2. Run calibration tool
./calibrate \
  --input ./measurements/antenna_1_measurements.csv \
  --output ./calibration_data/antenna_1.bin \
  --antenna-id antenna_1 \
  --validate

# 3. Update configuration
# Edit calibration_data/antennas.yaml to add new antenna

# 4. Commit artifacts
git add calibration_data/
git commit -m "Add calibration for antenna_1"
```

### 7.2 Build Process

```bash
# Local build
cargo build --release

# Docker build
docker build -t antenna-model:latest .

# Push to registry
docker tag antenna-model:latest registry.local/antenna-model:v1.2.3
docker push registry.local/antenna-model:v1.2.3
```

### 7.3 Testing Strategy

**Unit Tests:**
- Interpolation accuracy tests
- Input validation tests
- Configuration loading tests

**Integration Tests:**
- API endpoint tests
- Multi-antenna scenarios
- Error handling paths

**Performance Tests:**
- Latency benchmarks
- Throughput tests
- Memory profiling

**Validation Tests:**
- Compare against known measurement data
- Verify extrapolation behavior
- Check warning generation

## 8. Security Considerations

### 8.1 Current Scope (Internal Use)
- No authentication/authorization required initially
- Network-level access control via K8s NetworkPolicy
- No sensitive data in calibration artifacts

### 8.2 Future Enhancements
- API key authentication
- Rate limiting per client
- Audit logging for compliance
- Encrypted calibration data (if needed)

## 9. Future Enhancements

### 9.1 GPU Acceleration
**Phase 1: Design for extensibility**
- Abstract computation engine behind trait
- Implement CPU backend first
- Design data structures for GPU transfer

**Phase 2: GPU implementation**
- Evaluate CUDA vs compute shaders vs portable solutions
- Implement GPU backend for interpolation
- Add backend selection configuration
- Benchmark and optimize

### 9.2 gRPC API
- Define protobuf schemas matching REST API
- Implement gRPC server alongside REST
- Support streaming for large batch requests
- Benchmark performance vs REST

### 9.3 CI/CD Calibration
- Automated calibration pipeline
- S3 → Calibration → Build → Deploy
- Version control for calibration data
- A/B testing for model updates

### 9.4 Advanced Features
- Multi-temperature support (full 4D)
- Uncertainty quantification
- Real-time model updates
- Federated antenna configurations

## 10. Dependencies

### 10.1 Core Dependencies
```toml
[dependencies]
# Web framework
poem = "2.0"
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Numerics
ndarray = "0.15"
# or custom implementation for interpolation

# Configuration
serde_yaml = "0.9"
config = "0.13"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Error handling
anyhow = "1.0"
thiserror = "1.0"

# Serialization (for calibration artifacts)
bincode = "1.3"
# or serde_messagepack = "0.15"

# Optional: Parallelization
rayon = "1.8"

# CLI tool
clap = { version = "4.4", features = ["derive"] }
```

## 11. Repository Structure

```
antenna-model/
├── Cargo.toml
├── Dockerfile
├── README.md
├── ARCHITECTURE.md              # This document
├── antenna-model-design-doc.md  # Original design doc
│
├── src/
│   ├── main.rs                  # API service entry point
│   ├── lib.rs                   # Shared library code
│   │
│   ├── api/                     # REST API layer
│   │   ├── mod.rs
│   │   ├── routes.rs            # Endpoint definitions
│   │   ├── handlers.rs          # Request handlers
│   │   ├── schemas.rs           # Request/response types
│   │   └── middleware.rs        # Logging, etc.
│   │
│   ├── service/                 # Business logic
│   │   ├── mod.rs
│   │   ├── evaluator.rs         # Evaluation orchestration
│   │   ├── validator.rs         # Input validation
│   │   └── batch.rs             # Batch processing
│   │
│   ├── model/                   # Computation engine
│   │   ├── mod.rs
│   │   ├── interpolation.rs     # 4D B-spline interpolation
│   │   ├── bspline.rs           # B-spline primitives
│   │   └── extrapolation.rs     # Out-of-range handling
│   │
│   ├── data/                    # Data management
│   │   ├── mod.rs
│   │   ├── repository.rs        # Calibration data access
│   │   ├── loader.rs            # Load artifacts at startup
│   │   └── types.rs             # Data structures
│   │
│   └── config/                  # Configuration
│       ├── mod.rs
│       └── settings.rs          # Settings types
│
├── calibrate/                   # Calibration CLI tool
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs              # CLI entry point
│   │   ├── parser.rs            # Parse measurement CSV
│   │   ├── fitter.rs            # B-spline fitting
│   │   ├── validator.rs         # Validation logic
│   │   └── serializer.rs        # Write artifacts
│   └── README.md
│
├── calibration_data/            # Committed artifacts
│   ├── antennas.yaml            # Antenna configuration
│   ├── antenna_1.bin
│   └── antenna_2.bin
│
├── config/                      # Runtime configuration
│   └── service.yaml
│
├── helm/antenna-model           # Kubernetes Helm chart
│
└── tests/
    ├── integration/
    ├── performance/
    └── fixtures/
```

## 12. Acceptance Criteria

### 12.1 Functional Requirements
- ✓ Load multiple antenna calibrations at startup
- ✓ REST API with health, status, evaluate, batch, heatmap endpoints
- ✓ 4D B-spline interpolation (az, el, freq, const temp)
- ✓ Out-of-range queries with warnings
- ✓ Input validation with clear error messages
- ✓ JSON request/response format

### 12.2 Non-Functional Requirements
- ✓ 50-100ms p95 latency for single evaluations
- ✓ Support 1-20 requests/second per instance
- ✓ <512MB memory footprint
- ✓ <10s startup time
- ✓ Structured logging with tokio-tracing
- ✓ Kubernetes deployment with health probes

### 12.3 Calibration Tool Requirements
- ✓ CLI tool accepting local files or S3 URLs
- ✓ Parse CSV measurement data
- ✓ Fit B-spline models
- ✓ Serialize to binary artifacts
- ✓ Basic input validation

## 13. Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| B-spline fitting accuracy | High | Extensive validation against measurements |
| Extrapolation errors | Medium | Clear warnings, conservative extrapolation |
| Memory growth with antennas | Medium | Monitor footprint, optimize data structures |
| CPU performance insufficient | Medium | Design for GPU migration, optimize hot paths |
| Calibration data corruption | High | Checksums, validation at load time |
| K8s pod crashes | Low | Health probes, automatic restarts |

## 14. Success Metrics

- API response time p95 < 100ms
- Zero errors for in-range queries
- Successful deployment to K8s cluster
- Support for 5+ antenna configurations
- Accurate warning generation for extrapolation
- Clean structured logs for debugging
- Successful calibration workflow documentation

---

**Document Version:** 1.0
**Last Updated:** 2025-01-15
**Authors:** System Architect
**Reviewers:** [TBD]
