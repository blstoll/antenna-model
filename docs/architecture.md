# Antenna Model Service - Application Architecture

## 1. Executive Summary

The Antenna Model Service is a high-accuracy antenna loss modeling system deployed as a Kubernetes service. It provides REST API access to antenna models with flexible calibration statuses, supporting real-time queries for G/T (Gain-to-Temperature) predictions based on antenna orientation and frequency. The system supports graceful degradation from fully calibrated to uncalibrated antennas, prioritizing **loss accuracy** where systematic errors cancel.

### Key Characteristics
- **Language**: Rust
- **Deployment**: Kubernetes (on-premise)
- **API**: REST (poem framework), future gRPC support
- **Performance**: 1-20 requests/second per instance, 50-100ms latency
- **Calibration**: Offline process with multiple modes (full, boresight, uncalibrated)
- **Data Volume**: <20MB calibration data per deployment
- **Calibration Statuses**:
  - Fully Calibrated: ±1 dB (main lobe/first sidelobe)
  - Partially Calibrated (Boresight): ±1 dB at boresight, ±1-2 dB loss
  - Partially Calibrated (Limited Coverage): ±1-1.5 dB in-coverage
  - Uncalibrated: ±3-5 dB absolute, ±2-3 dB loss (design specs only)

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
- Batch queries for loss heatmap generation across antenna field of view
- 3D coordinate-based queries (ECEF or Geodetic positions)
- Tolerance to higher latencies (seconds)
- Require detailed error information and warnings
- Support for rectangular or H3 hexagonal grid generation

**Real-time Processors**
- Single-point gain queries during operation
- 3D position-based queries (vehicle, antenna boresight, feed, emitter)
- Latency-sensitive (<150ms including coordinate transforms)
- Need fast failure modes
- Support for beam squint correction with pointing frequency
- Require calibration status information in responses

**Calibration Engineers** (NEW)
- Add uncalibrated antennas with design specs only
- Perform boresight calibration (~1 hour test time)
- Upgrade antennas from uncalibrated → partial → fully calibrated
- Monitor calibration quality and accuracy estimates

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
        │  - Fits B-spline models (full)     │
        │  - Tunes parameters (boresight)    │
        │  - Loads design specifications     │
        │  - Generates calibration artifacts │
        │  - Validates input data            │
        │  - Supports: full, boresight modes │
        └────────────────────────────────────┘
                       │
                       ▼
              ┌─────────────────┐
              │ Calibration     │
              │ Artifacts       │
              │ (Source Control)│
              │  - .bin files   │
              │  - antennas.yaml│
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
- `POST /api/v1/gain` - Single antenna gain computation from 3D positions
- `POST /api/v1/gain/batch` - Batch gain computation
- `POST /api/v1/heatmap` - Generate loss heatmap grid (rectangular or H3 hexagonal)
- `GET /api/v1/antennas` - List available antennas with feeds
- `GET /api/v1/antennas/{id}` - Antenna configuration details
- `GET /api/v1/antennas/{id}/feeds` - List feeds for antenna
- `GET /api/v1/antennas/{id}/feeds/{feed_id}` - Feed configuration details

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
- Load calibration data at startup (all calibration statuses)
- Provide fast access to model coefficients and calibration status
- Store metadata (validity ranges, antenna properties, calibration coverage)
- Support multiple antenna configurations with multi-feed support
- Load uncalibrated antennas from design specifications

**Data Structures:**
- In-memory nested hash map: `antenna_id -> feed_id -> AntennaCalibration`
- Each `AntennaCalibration` contains:
  - Physical antenna configuration (reflector, feed parameters)
  - Optional B-spline correction surface (4D tensor) - if calibrated
  - Knot vectors for each dimension (if correction surface present)
  - Validity ranges (min/max for az, el, freq)
  - Calibration status (Fully/Partially/Uncalibrated)
  - Optional calibration coverage metadata (for partial calibration)
  - Metadata (antenna name, calibration date, parameters source, etc.)

**Data Loading:**
- Load from local filesystem at startup (`.bin` files)
- **Uncalibrated antennas**: Construct in-memory from design specs in `antennas.yaml`
- Fail-fast if calibration data is missing or corrupted (configurable)
- Log loaded antenna configurations with calibration status
- Support composite `(antenna_id, feed_id)` lookups

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
Binary format (bincode):
```rust
struct AntennaCalibration {
    antenna_id: String,
    feed_id: String,                 // Multi-feed support
    metadata: CalibrationMetadata,
    physical_config: PhysicalAntennaConfig,  // Always present
    correction_surface: Option<BSplineModel4D>,  // Optional for partial/uncalibrated
    validity_ranges: ValidityRanges,

    // NEW: Calibration status and coverage
    calibration_status: Option<CalibrationStatus>,  // Optional for backward compatibility
    calibration_coverage: Option<CalibrationCoverage>,
}

enum CalibrationStatus {
    FullyCalibrated { accuracy_estimate_db: f64 },
    PartiallyCalibrated {
        accuracy_estimate_db: f64,
        coverage: CalibrationCoverage,
    },
    Uncalibrated {
        accuracy_estimate_db: f64,
        loss_accuracy_estimate_db: f64,
    },
}

struct CalibrationCoverage {
    azimuth_range: (f64, f64),       // Degrees
    elevation_range: (f64, f64),     // Degrees
    frequency_range: (f64, f64),     // MHz
    num_measurements: usize,
    has_correction_surface: bool,
}

struct PhysicalAntennaConfig {
    reflector: ReflectorGeometry,
    feed: FeedParameters,
    mesh: Option<MeshParameters>,
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
    # Fully calibrated antenna (existing workflow)
    - id: "antenna_1"
      name: "Deep Space Network 34m"
      calibration_file: "antenna_1.bin"
      calibration_status: "fully_calibrated"  # Default if omitted
      enabled: true

    # Partially calibrated antenna (boresight only)
    - id: "antenna_2"
      name: "Ground Station - Boresight Calibrated"
      calibration_file: "antenna_2_boresight.bin"
      calibration_status: "partially_calibrated"
      calibration_coverage:
        azimuth_range: [0.0, 0.0]      # Single point
        elevation_range: [0.0, 0.0]    # Single point
        frequency_range: [7100.0, 8500.0]
        num_measurements: 25
      enabled: true

    # Uncalibrated antenna (design specs only, no .bin file)
    - id: "antenna_3"
      name: "Ground Station - Uncalibrated"
      calibration_status: "uncalibrated"
      # No calibration_file - constructed from design_specs
      design_specs:
        diameter_m: 3.7
        focal_length_m: 1.85
        f_over_d_ratio: 0.5
        surface_rms_mm: 1.5
        feeds:
          - id: "x_band_feed"
            name: "X-Band Primary Feed"
            position: [0.0, 0.0, 0.0]
            q_factor: 8.0
            phase_center_offset_m: 0.0
            frequency_range: [7100.0, 8500.0]
          - id: "s_band_feed"
            name: "S-Band Feed"
            position: [0.05, 0.0, 0.0]
            q_factor: 7.0
            phase_center_offset_m: 0.0
            frequency_range: [2000.0, 2300.0]
        mesh:
          mesh_spacing_mm: 5.0
          wire_diameter_mm: 0.5
      validity_ranges:
        azimuth_range: [0.0, 360.0]
        elevation_range: [0.0, 90.0]
        frequency_range: [2000.0, 8500.0]
        temperature_k: 290.0
      enabled: true
```

### 4.3 API Request/Response Schemas

**Note on Coordinate Systems:**
All 3D positions support automatic detection of coordinate system based on magnitude:
- **ECEF**: If `abs(x) > 6400e3 OR abs(y) > 6400e3 OR abs(z) > 6400e3` (meters)
- **Geodetic**: Otherwise (x=longitude degrees, y=latitude degrees, z=altitude meters)

#### Gain Computation Request
```json
{
  "antenna_id": "antenna_1",
  "feed_id": "x_band_feed",
  "vehicle_position": {
    "x": 4510731.123,
    "y": 4510731.456,
    "z": 3488865.789
  },
  "vehicle_attitude": {
    "w": 1.0,
    "x": 0.0,
    "y": 0.0,
    "z": 0.0
  },
  "reflector_boresight": {
    "x": 4510732.0,
    "y": 4510732.0,
    "z": 3488950.0
  },
  "feed_position": {
    "x": 4510731.5,
    "y": 4510731.5,
    "z": 3488870.0
  },
  "emitter_position": {
    "x": 4520000.0,
    "y": 4520000.0,
    "z": 3500000.0
  },
  "frequency_mhz": 8400.0,
  "pointing_frequency_mhz": 8450.0,
  "include_reference": true
}
```

**Alternative with Geodetic coordinates:**
```json
{
  "antenna_id": "antenna_1",
  "feed_id": "x_band_feed",
  "vehicle_position": {
    "x": -118.1234,
    "y": 34.5678,
    "z": 100.0
  },
  "vehicle_attitude": {
    "roll_deg": 0.0,
    "pitch_deg": 5.0,
    "yaw_deg": 180.0
  },
  "reflector_boresight": {
    "x": -117.0,
    "y": 35.0,
    "z": 400000.0
  },
  "feed_position": {
    "x": -118.124,
    "y": 34.568,
    "z": 105.0
  },
  "emitter_position": {
    "x": -117.0,
    "y": 35.0,
    "z": 400000.0
  },
  "frequency_mhz": 8400.0,
  "pointing_frequency_mhz": 8450.0,
  "include_reference": true
}
```

#### Gain Computation Response
```json
{
  "antenna_id": "antenna_1",
  "feed_id": "x_band_feed",
  "gain_db": 41.2,
  "reference_gain_db": 43.5,
  "loss_db": 2.3,
  "geometry": {
    "feed_offset_meters": {
      "x": 0.05,
      "y": 0.02,
      "z": 0.01
    },
    "emitter_azimuth_deg": 185.5,
    "emitter_elevation_deg": 32.1,
    "beam_squint_deg": 0.15
  },
  "warnings": [
    "Beam squint correction applied (pointing_freq != operating_freq)"
  ],
  "metadata": {
    "computation_time_ms": 2.8,
    "coordinate_transform_ms": 0.3,
    "physics_model_ms": 1.8,
    "correction_surface_ms": 0.5,
    "extrapolated": false
  },
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 1.5,
    "coverage": {
      "azimuth_range_deg": [0.0, 0.0],
      "elevation_range_deg": [0.0, 0.0],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 25,
      "is_boresight_only": true
    },
    "correction_applied": false,
    "parameters_source": "boresight_tuning"
  }
}
```

**Response for Uncalibrated Antenna:**
```json
{
  "antenna_id": "antenna_3",
  "feed_id": "x_band_feed",
  "gain_db": 40.5,
  "reference_gain_db": 43.8,
  "loss_db": 3.3,
  "geometry": { ... },
  "warnings": [
    "Antenna 'antenna_3' is uncalibrated (using design specifications). Absolute gain accuracy: ±3.0 dB, Loss accuracy: ±2.0 dB"
  ],
  "metadata": {
    "computation_time_ms": 1.5,
    "extrapolated": false
  },
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 3.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_applied": false,
    "parameters_source": "design_specifications"
  }
}
```

#### Batch Request
```json
{
  "evaluations": [
    {
      "antenna_id": "antenna_1",
      "feed_id": "x_band_feed",
      "vehicle_position": {"x": 4510731.123, "y": 4510731.456, "z": 3488865.789},
      "vehicle_attitude": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
      "reflector_boresight": {"x": 4510732.0, "y": 4510732.0, "z": 3488950.0},
      "feed_position": {"x": 4510731.5, "y": 4510731.5, "z": 3488870.0},
      "emitter_position": {"x": 4520000.0, "y": 4520000.0, "z": 3500000.0},
      "frequency_mhz": 8400.0,
      "include_reference": false
    },
    {
      "antenna_id": "antenna_2",
      "feed_id": "s_band_feed",
      "vehicle_position": {"x": -118.1234, "y": 34.5678, "z": 100.0},
      "vehicle_attitude": {"roll_deg": 0.0, "pitch_deg": 0.0, "yaw_deg": 0.0},
      "reflector_boresight": {"x": -117.0, "y": 35.0, "z": 400000.0},
      "feed_position": {"x": -118.124, "y": 34.568, "z": 105.0},
      "emitter_position": {"x": -117.0, "y": 35.0, "z": 400000.0},
      "frequency_mhz": 2200.0,
      "include_reference": false
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
      "feed_id": "x_band_feed",
      "gain_db": 41.2,
      "geometry": {
        "feed_offset_meters": {"x": 0.05, "y": 0.02, "z": 0.01},
        "emitter_azimuth_deg": 185.5,
        "emitter_elevation_deg": 32.1
      },
      "warnings": [],
      "metadata": {
        "extrapolated": false
      }
    },
    {
      "antenna_id": "antenna_2",
      "feed_id": "s_band_feed",
      "gain_db": 38.7,
      "geometry": {
        "feed_offset_meters": {"x": 0.0, "y": 0.0, "z": 0.0},
        "emitter_azimuth_deg": 45.2,
        "emitter_elevation_deg": 78.9
      },
      "warnings": [],
      "metadata": {
        "extrapolated": false
      }
    }
  ],
  "metadata": {
    "total_computation_time_ms": 25.3,
    "count": 2
  }
}
```

#### Heatmap Request (Rectangular Grid)
```json
{
  "antenna_id": "antenna_1",
  "feed_id": "x_band_feed",
  "vehicle_position": {
    "x": 4510731.123,
    "y": 4510731.456,
    "z": 3488865.789
  },
  "vehicle_attitude": {
    "w": 1.0,
    "x": 0.0,
    "y": 0.0,
    "z": 0.0
  },
  "reflector_boresight": {
    "x": 4510732.0,
    "y": 4510732.0,
    "z": 3488950.0
  },
  "feed_position": {
    "x": 4510731.5,
    "y": 4510731.5,
    "z": 3488870.0
  },
  "frequency_mhz": 8400.0,
  "pointing_frequency_mhz": 8450.0,
  "grid_config": {
    "grid_type": "rectangular",
    "azimuth_range_deg": {
      "min": 0.0,
      "max": 360.0,
      "step": 5.0
    },
    "elevation_range_deg": {
      "min": 0.0,
      "max": 90.0,
      "step": 2.0
    }
  }
}
```

#### Heatmap Request (H3 Hexagonal Grid)
```json
{
  "antenna_id": "antenna_1",
  "feed_id": "x_band_feed",
  "vehicle_position": {
    "x": 4510731.123,
    "y": 4510731.456,
    "z": 3488865.789
  },
  "vehicle_attitude": {
    "w": 1.0,
    "x": 0.0,
    "y": 0.0,
    "z": 0.0
  },
  "reflector_boresight": {
    "x": 4510732.0,
    "y": 4510732.0,
    "z": 3488950.0
  },
  "feed_position": {
    "x": 4510731.5,
    "y": 4510731.5,
    "z": 3488870.0
  },
  "frequency_mhz": 8400.0,
  "grid_config": {
    "grid_type": "h3",
    "h3_resolution": 7,
    "center_azimuth_deg": 180.0,
    "center_elevation_deg": 45.0,
    "field_of_view_deg": 30.0
  }
}
```

#### Heatmap Response
```json
{
  "antenna_id": "antenna_1",
  "feed_id": "x_band_feed",
  "frequency_mhz": 8400.0,
  "grid": {
    "grid_type": "rectangular",
    "azimuth_values": [0.0, 5.0, 10.0, ...],
    "elevation_values": [0.0, 2.0, 4.0, ...],
    "loss_db": [
      [2.1, 2.3, 2.8, ...],  // Row for elevation 0
      [1.8, 2.0, 2.5, ...],  // Row for elevation 2
      ...
    ]
  },
  "warnings": ["Some points extrapolated"],
  "metadata": {
    "points_evaluated": 3276,
    "computation_time_ms": 1245.6,
    "peak_gain_db": 43.5
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
    FeedNotFound { antenna_id: String, feed_id: String },
    InvalidParameter { param: String, reason: String },
    InvalidCoordinate { param: String, reason: String },
    InvalidAttitude { reason: String },
    CoordinateTransformError { details: String },
    ComputationError { details: String },
    InternalError { details: String },
}
```

**HTTP Status Mappings:**
- 200: Success
- 400: Invalid parameters, coordinates, or attitude
- 404: Antenna or feed not found
- 500: Internal computation/system error (including coordinate transform failures)
- 503: Service unavailable (startup, shutdown)

**Common Error Examples:**
- Invalid ECEF coordinates: `{"error": "InvalidCoordinate", "param": "vehicle_position", "reason": "Coordinates exceed Earth radius (|pos| > 10000 km)"}`
- Invalid Geodetic coordinates: `{"error": "InvalidCoordinate", "param": "emitter_position", "reason": "Latitude out of range [-90, 90] degrees"}`
- Invalid attitude: `{"error": "InvalidAttitude", "reason": "Quaternion not normalized (|q| = 1.15)"}`
- Feed not found: `{"error": "FeedNotFound", "antenna_id": "antenna_1", "feed_id": "invalid_feed"}`
- Coordinate singularity: `{"error": "CoordinateTransformError", "details": "Gimbal lock at zenith (elevation = 90 degrees)"}`

### 6.4 Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Single gain computation latency | 100-150ms | p95, includes coordinate transforms |
| Coordinate transform overhead | <10ms | Per request |
| Physics model computation | 50-80ms | Per evaluation |
| Correction surface interpolation | 5-10ms | Per evaluation |
| Batch throughput | 1-20 req/s | Per instance |
| Heatmap generation (3312 points) | <2s | Rectangular grid |
| Startup time | <10s | Including data load |
| Memory footprint | <512MB | With all antennas and feeds |
| Calibration data size | <20MB | All antennas combined |

## 7. Development Workflow

### 7.1 Calibration Process

#### Full Calibration Workflow (Existing)
```bash
# Engineer workflow for fully calibrated antenna
# 1. Download measurements from S3
aws s3 cp s3://bucket/antenna_1_full_grid.csv ./measurements/

# 2. Run calibration tool (full mode)
./calibrate \
  --input ./measurements/antenna_1_full_grid.csv \
  --output ./calibration_data/antenna_1.bin \
  --antenna-id antenna_1 \
  --feed-id x_band_feed \
  --calibration-mode full \
  --validate

# 3. Update configuration
# Edit calibration_data/antennas.yaml to add new antenna

# 4. Commit artifacts
git add calibration_data/
git commit -m "Add full calibration for antenna_1"
```

#### Boresight Calibration Workflow (NEW - Sprint 7)
```bash
# Engineer workflow for boresight-only calibration
# 1. Download boresight measurements
aws s3 cp s3://bucket/antenna_2_boresight.csv ./measurements/

# 2. Run calibration tool (boresight mode)
./calibrate \
  --input ./measurements/antenna_2_boresight.csv \
  --output ./calibration_data/antenna_2_boresight.bin \
  --antenna-id antenna_2 \
  --feed-id x_band_feed \
  --calibration-mode boresight \
  --design-specs ./design_specs/antenna_2.yaml \
  --validate

# 3. Update configuration
# Edit calibration_data/antennas.yaml:
#   - Set calibration_status: "partially_calibrated"
#   - Add calibration_coverage metadata

# 4. Commit artifacts
git add calibration_data/
git commit -m "Add boresight calibration for antenna_2"
```

#### Uncalibrated Antenna Workflow (NEW - Sprint 6)
```bash
# Engineer workflow for uncalibrated antenna (no measurements)
# 1. Create design specs file
# Create design_specs/antenna_3.yaml with reflector geometry and feed parameters

# 2. Update configuration
# Edit calibration_data/antennas.yaml:
#   - Set calibration_status: "uncalibrated"
#   - Add design_specs section (inline or reference)
#   - NO calibration_file needed

# 3. Commit configuration only
git add calibration_data/antennas.yaml
git commit -m "Add uncalibrated antenna_3 with design specs"

# Note: Service will construct AntennaCalibration in-memory at startup
```

#### Calibration Upgrade Path
```bash
# 1. Deploy uncalibrated antenna (design specs only)
# → Service provides ±3-5 dB absolute, ±2-3 dB loss

# 2. Collect boresight measurements (~1 hour test time)
# 3. Run boresight calibration (as shown above)
# → Service provides ±1 dB boresight, ±2-3 dB off-axis, ±1-2 dB loss

# 4. Collect full grid measurements (~8 hours test time)
# 5. Run full calibration
# → Service provides ±1 dB full FOV
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
- ✅ Load multiple antenna calibrations at startup (all statuses)
- ✅ REST API with health, status, gain, batch, heatmap, antenna/feed endpoints
- ✅ Support all calibration statuses (fully/partially/uncalibrated)
- ✅ Load uncalibrated antennas from design specs (no .bin file required)
- ✅ 4D B-spline interpolation for correction surfaces (optional)
- ✅ Physics-based model always computed
- ✅ Out-of-range queries with warnings
- ✅ Calibration status information in API responses
- ✅ Input validation with clear error messages
- ✅ JSON request/response format
- ✅ Multi-feed support with composite (antenna_id, feed_id) identifiers
- 📋 Boresight calibration mode (Sprint 7)

### 12.2 Non-Functional Requirements
- ✅ 50-100ms p95 latency for single evaluations
- ✅ Support 1-20 requests/second per instance
- ✅ <512MB memory footprint
- ✅ <10s startup time
- ✅ Structured logging with tokio-tracing
- ✅ Kubernetes deployment with health probes
- ✅ Backward compatibility maintained (optional calibration_status field)

### 12.3 Calibration Tool Requirements (Phase 1 ✅, Phase 2 📋)
- ✅ CLI tool accepting local files (Phase 1)
- ✅ Parse CSV measurement data (Phase 1)
- ✅ Fit B-spline models for full calibration (Phase 1)
- ✅ Serialize to binary artifacts (Phase 1)
- ✅ Input validation (Phase 1)
- 📋 Boresight calibration mode with parameter tuning (Phase 2 - Sprint 7)
- 📋 Load design specifications from YAML (Phase 2 - Sprint 7)
- 📋 Optional frequency-only correction surface (Phase 2 - Sprint 7)

### 12.4 Calibration Status Support (NEW - Sprint 6 ✅)
- ✅ Data model extensions (CalibrationStatus enum, CalibrationCoverage struct)
- ✅ Configuration parsing (design_specs, calibration_coverage)
- ✅ Uncalibrated antenna loading from design specs
- ✅ Service layer handling all calibration statuses
- ✅ API schemas with calibration_status field
- ✅ Antenna details endpoint enhancement
- ✅ Warning generation for uncalibrated/partially calibrated antennas
- ✅ Coverage-aware correction surface application
- ✅ 81+ comprehensive tests (all passing, 468 total tests)

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

### Core Service Metrics
- ✅ API response time p95 < 100ms (achieved for all calibration statuses)
- ✅ Zero errors for in-range queries
- ✅ Successful deployment to K8s cluster
- ✅ Support for 5+ antenna configurations (with multiple feeds)
- ✅ Accurate warning generation for extrapolation and calibration status
- ✅ Clean structured logs for debugging
- ✅ Successful calibration workflow documentation

### Partial Calibration Metrics (NEW - Sprint 6)
- ✅ Service supports all calibration statuses (fully/partially/uncalibrated)
- ✅ Uncalibrated antennas load from design specs without .bin files
- ✅ API responses include calibration_status field with accuracy estimates
- ✅ Warning generation for uncalibrated antennas (accuracy expectations)
- ✅ Loss accuracy prioritization for partial calibration (±1-2 dB)
- ✅ 468 total tests passing (100% backward compatibility)
- ✅ Zero regressions from partial calibration implementation
- 📋 Boresight calibration tool functional (Sprint 7 target)
- 📋 Parameter tuning from boresight measurements (Sprint 7 target)
- 📋 Calibration upgrade path validated (uncalibrated → boresight → full)

---

**Document Version:** 1.1 (Updated for Partial Calibration Support)
**Last Updated:** 2025-01-15
**Authors:** System Architect
**Reviewers:** [TBD]

**Changes in v1.1:**
- Added support for multiple calibration statuses (fully/partially/uncalibrated)
- Added design specs configuration for uncalibrated antennas
- Updated data structures with CalibrationStatus enum and CalibrationCoverage
- Extended API responses with calibration_status field
- Added boresight calibration workflow (Sprint 7)
- Updated acceptance criteria and success metrics
- 100% backward compatibility maintained
