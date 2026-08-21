# Sprints 1-4 Summary: Foundation Complete

**Status:** ✅ ALL COMPLETE (100%)
**Total Duration:** 8 weeks
**Test Coverage:** 280+ tests passing

---

## Sprint 1: Project Foundation & Core Data Types ✅

**Deliverables:**
- **Cargo Workspace:** `antenna-model` (main service) + `calibrate` (CLI tool)
- **Basic REST API:** `poem` web framework with `/status` endpoint, graceful shutdown
- **Core Data Types** (`src/data/types.rs`):
  - `AntennaCalibration` - container for antenna metadata + models
  - `BSplineModel4D` - 4D tensor (coefficients, knots, shape) for correction surfaces
  - `ValidityRanges` - physical parameter bounds (azimuth, elevation, frequency, temperature)
  - `PhysicalAntennaConfig` - reflector geometry, feed parameters, mesh properties
  - `ReflectorGeometry`, `FeedParameters`, `MeshParameters`
- **Configuration System** (`src/config/settings.rs`):
  - YAML-based config with environment variable overrides (`ANTENNA_MODEL_` prefix)
  - Server settings, calibration paths, logging, performance tuning
- **Error Framework** (`src/error.rs`):
  - `thiserror`-based error types: `DataError`, `ApiError`, `ValidationError`, `ComputationError`, `ConfigError`

**Key Architecture Decisions:**
- REST API uses `poem` framework with `tokio` async runtime
- Configuration: YAML files + env vars (12-factor app pattern)
- Error handling: `Result<T, E>` throughout, never `unwrap()` in production
- Serialization: `bincode` v2.x for calibration artifacts, JSON for API

---

## Sprint 2: Physical Optics Computation Engine ✅

**Deliverables:**
- **Antenna Geometry** (`src/model/geometry.rs`):
  - `ReflectorGeometry` - diameter, focal length, f/D ratio, surface RMS
  - `FeedParameters` - position (x,y,z), q-factor, phase center offset
  - `MeshParameters` - spacing, wire diameter, angle-dependent effects
  - Coordinate transformations: E-clock/E-cone ↔ Cartesian
- **Phase Functions** (`src/model/phase.rs`):
  - Path phase: `k·[ρ²/(4f) - ρ·sin(θ)·cos(φ-φ')]`
  - Coma aberration: `k·δ_feed·[ρ/(2f)]·[2·cos(α) - (ρ/(2f))·cos(2α-φ')]`
  - Surface errors: `(4π/λ)·ε(ρ,φ')·cos(θ_incident)` with Zernike polynomials
  - Mesh effects: `arctan[(2π·d_mesh/λ)·sin(θ_incident)]`
- **Feed Illumination** (`src/model/illumination.rs`):
  - `cos^q` pattern: q=6-8 for -25 to -30 dB edge taper
  - Feed angle calculation with accurate parabolic geometry
  - Phase center offset modeling
- **Aperture Integration** (`src/model/integration.rs`):
  - 2D Simpson's rule with adaptive refinement
  - Polar coordinates (ρ, φ') with proper Jacobian
  - Convergence monitoring, integration parameter presets (fast/default/high_accuracy)
- **Far-Field Pattern** (`src/model/pattern.rs`):
  - Gain computation: `compute_gain()`, `compute_gain_db()`
  - Ruze efficiency: `η = exp(-(4π·σ/λ)²)`
  - Mesh transparency: `T = 1/(1 + (λ₀/λ)²)`
  - G/T ratio: `compute_g_over_t()`

**Key Implementation Details:**
- Physics model pipeline: Geometry → Phase → Illumination → Integration → Pattern
- Complex arithmetic using `num_complex::Complex64`
- Adaptive refinement: 3/2 factor, max iterations with convergence check
- On-axis gain ~35 dB for 1m dish at 8.4 GHz (validated)

---

## Sprint 3: Surface Error & Mesh Reflector Models ✅

**Deliverables:**
- **Coordinate Transformations** (`src/model/coordinates.rs`):
  - ECEF (Earth-Centered Earth-Fixed) ↔ Geodetic (lon, lat, alt)
  - Antenna Frame transformations with vehicle attitude
  - Spherical coordinates (azimuth, elevation) for antenna pointing
  - Auto-detection: |x| or |y| or |z| > 6400 km → ECEF, else Geodetic
- **Edge Case Handling** (`src/model/edge_cases.rs`):
  - Large feed offsets (> 0.3f): switch to ray tracing
  - Near-boresight scenarios with feed displacement
  - Frequency-dependent mesh transparency (<1 GHz, transition region, >10 GHz)
- **Ray Tracing** (`src/model/ray_trace.rs`):
  - Direct path computation (feed → emitter without reflection)
  - Interference between direct and reflected paths
  - Spillover modeling for large offsets
- **Numerical Stability** (`src/model/numerical_stability.rs`):
  - Adaptive integration near pattern nulls
  - Minimum noise floor (-60 dB typical)
  - Kaiser windowing for sidelobe continuity

**Coordinate System Validation:**
- Geodetic singularities: poles (±90° lat), earth center
- Quaternion normalization: warn if |q| ≠ 1.0 by >0.01
- Gimbal lock handling in Euler angles (pitch = ±90°)

---

## Sprint 4: Calibration via Parameter Optimization ✅

**Deliverables:**
- **Parameter Tuning** (`calibrate/src/parameter_tuner.rs`):
  - Nelder-Mead simplex optimizer (`argmin::solver::neldermead`) *(corrected 2026-08-19, roadmap D5: the optimizer is **Nelder-Mead** — `argmin::solver::neldermead` in `calibrate/src/parameter_tuner.rs`. No differential-evolution optimizer was ever implemented under any name.)*
    The named "DE/rand/1/bin" strategy appears nowhere in the code and never did:
    `git log --all -S 'DE/rand'` returns three commits and **all three touch only
    documentation** — `b2aaaf5`, which added this line to this file, and D16's
    `e13a1e3`/`b908d1a`, which added the discussion of it to
    `docs/roadmap-2026-07-work-units.md`. No commit has ever put the string in code.
  - Tunes: surface RMS, mesh spacing, wire diameter
    *(corrected: q-factor and phase-center offset are declared design properties, not
    tuned — see `TuningMode` in `calibrate/src/parameter_tuner.rs`)*
  - Objective: a single scalar — the **weighted RMSE** of `measured_G/T − model_G/T`
    over the measurement set, with points inside 3 beamwidths of boresight weighted 3×
    *(corrected: it is not multi-objective; the main lobe is emphasised by a weight, and
    there is no separate first-sidelobe term)*
  - Convergence: `--max-tuning-iterations` (CLI default 100) with `sd_tolerance = 1e-6`
    *(corrected: a simplex has no population or generations)*
- **Correction Surface Fitting** (`calibrate/src/correction_surface.rs`):
  - Residual-based approach: measured - physics_model = correction
  - B-spline fitting: 3D (E-clock, E-cone, frequency) → correction_dB
  - Alternative: RBF (Radial Basis Functions) for scattered data
  - Knot placement: uniform grid or adaptive based on data density
- **Antenna Config Extraction** (`calibrate/src/antenna_config.rs`):
  - Builds `PhysicalAntennaConfig` from tuned parameters
  - Feed configuration with multiple feeds per antenna
  - Validity range extraction from measurement coverage
- **Calibration CLI** (`calibrate/src/main.rs`):
  - Parse CSV measurements (azimuth, elevation, frequency, G/T)
  - Parameter optimization loop with convergence monitoring
  - Correction surface fitting and validation
  - Generate binary artifacts (`AntennaCalibration` serialized with `bincode`)
- **Validation** (`calibrate/src/validator.rs`):
  - Cross-validation: split measurements into train/test sets
  - Error metrics: max error (main lobe, first sidelobe), RMSE, R²
  - Acceptance: <1 dB max error in main lobe and first sidelobe

**Calibration Workflow:**
```
Raw G/T Measurements (CSV)
  → Parse & Validate
  → Optimize Physical Parameters (DE)
  → Fit Correction Surface (B-spline/RBF)
  → Generate AntennaCalibration artifact (.bin)
  → Validate against test set
```

**Artifact Format:**
- Binary serialization (`bincode` v2.x)
- Contains: `PhysicalAntennaConfig` + `BSplineModel4D` (correction) + `ValidityRanges` + metadata
- File size: <20 MB per antenna (typical)

---

## Available Modules for Sprint 5+

> **Module map audited 2026-08-19 (roadmap D5): this listing is historical.** It
> predates the three-crate split (roadmap **D4**, 2026-08-13), so the physics engine and
> artifact data layer now live in **`antenna-core/src/`**, not `antenna-model/src/`, and
> `antenna-model` keeps only the web/service layers (`antenna-model` re-exports the moved
> paths, so the names below still resolve in Rust). `model/numerical_stability.rs`,
> listed below, does not exist at all any more — it and `model/surface.rs` were deleted as
> dead code in `a6dac0c`. For the current layout see `docs/architecture.md` §11, which is
> checked against `ls`. Three more entries below are stale in the same way:
> `calibrate/src/serializer.rs` was renamed to `sidecar.rs` when the legacy JSON
> serializer was retired (roadmap **D1**, 2026-07-29) and the binary writer moved to
> `artifact_export.rs`; `integration.rs` is no longer "adaptive Simpson's" on the served
> path but the Hankel / azimuthal-mode (Jₘ) integrator (roadmap **P10**, 2026-07-15), the
> 2D Simpson quadrature surviving only as a `#[cfg(test)]` reference oracle; and
> `phase.rs`'s surface-error function is present but unused by the aperture integrand
> (see the design doc §2.2).

**Antenna Model Service (`antenna-model/`):**
- `src/api/` - REST API (poem), handlers, routes, middleware, schemas
- `src/config/` - Configuration system (YAML + env vars)
- `src/data/` - Data types (AntennaCalibration, BSplineModel4D, etc.)
- `src/error.rs` - Error types
- `src/model/` - **Complete physics computation pipeline:**
  - `coordinates.rs` - ECEF/Geodetic/Antenna Frame/Spherical transforms
  - `geometry.rs` - Reflector, feed, mesh structures
  - `phase.rs` - All phase functions (path, coma, surface, mesh)
  - `illumination.rs` - Feed patterns (cos^q)
  - `integration.rs` - Aperture integration (adaptive Simpson's)
  - `pattern.rs` - Far-field gain, Ruze, mesh effects
  - `edge_cases.rs` - Large offsets, ray tracing
  - `numerical_stability.rs` - Stability helpers

**Calibration Tool (`calibrate/`):**
- `src/parser.rs` - CSV measurement parsing
- `src/parameter_tuner.rs` - Nelder-Mead simplex optimizer *(corrected 2026-08-19)*
- `src/correction_surface.rs` - B-spline/RBF fitting
- `src/antenna_config.rs` - Config extraction
- `src/validator.rs` - Cross-validation, error metrics
- `src/serializer.rs` - Binary artifact generation

---

## Key Technical Constraints for Future Sprints

1. **Calibration Data Repository** (Sprint 5.4):
   - Must load `AntennaCalibration` artifacts from `calibration_data/*.bin`
   - Use `antenna_model::data::types` structures (already defined)
   - Thread-safe access (use `Arc<HashMap<String, AntennaCalibration>>`)

2. **Gain Computation Endpoint** (Sprint 5.5):
   - Pipeline: Parse 3D coords → Transform to antenna frame → Physics model → Correction surface → Gain
   - Use `antenna_model::model::coordinates` for ECEF/Geodetic auto-detection
   - Use `antenna_model::model::pattern::compute_gain()` for physics
   - Interpolate correction surface from `BSplineModel4D` in calibration artifact
   - Combine: `corrected_gain = physics_gain + correction_db`

3. **Performance Targets:**
   - Single evaluation: <100ms p95 (including coordinate transforms)
   - Batch throughput: 1-20 req/s per instance
   - Heatmap (3312 points): <2 seconds
   - Memory: <512 MB per instance

4. **Interpolation Engine** (needed for correction surfaces):
   - 4D B-spline interpolation: (azimuth, elevation, frequency, temperature) → correction_dB
   - Use Cox-de Boor algorithm for basis functions
   - Local coefficient extraction (only relevant subset)
   - Sprint 5 will need to implement: `src/model/bspline.rs`, `src/model/interpolation.rs`

5. **Multi-Feed Support:**
   - Each antenna can have multiple feeds (e.g., S-band, X-band, Ka-band)
   - Composite identifier: `(antenna_id, feed_id)`
   - Different feeds → different correction surfaces
   - Feed positions stored in `FeedParameters` within `PhysicalAntennaConfig`

---

## Dependencies (from Sprints 1-4)

```toml
[dependencies]
# Web framework
poem = { version = "3.1.12", features = ["test"] }
tokio = { version = "1.48.0", features = ["full"] }
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.145"

# Numerics
ndarray = "0.16.1"
num-complex = "0.4.6"

# Configuration
config = "0.15.18"
serde_yaml = "0.9.34"
toml = "0.9.8"

# Error handling
anyhow = "1.0.100"
thiserror = "2.0.17"

# Logging
tracing = "0.1.41"
tracing-subscriber = { version = "0.3.20", features = ["env-filter", "json"] }

# Serialization
bincode = "2.0.1"

# Parallelization
rayon = "1.11.0"

# Request IDs (added in Sprint 5.1)
uuid = { version = "1.11.0", features = ["v4", "serde"] }
```

---

## Testing Foundation

**Total Tests:** 280+ passing
- Unit tests in each module (inline `#[cfg(test)]`)
- Integration tests in `tests/`
- Doc tests for public APIs
- No compiler warnings, no clippy warnings

**Test Patterns Established:**
- Physics validation: compare to hand calculations, known patterns
- Serialization: round-trip tests (JSON, bincode)
- Error handling: verify error types and messages
- Edge cases: extreme parameters, boundary conditions
- Performance: benchmarks in `benches/` (using `criterion`)

---

## References

- `docs/antenna-model-design-doc.md` - Mathematical foundations, equations
- `docs/architecture.md` - System architecture, API specs, deployment
- `config/service.yaml` - Example service configuration
- `calibration_data/antennas.yaml` - Example antenna registry

---

**Next:** Sprint 5 - REST API Core Endpoints (production middleware ✅, schemas, health, gain evaluation, repository)
## Sprint 5: REST API - Core Endpoints ✅

**Status:** ✅ COMPLETE - 7/7 tasks complete (100%)  
**Duration:** ~4 weeks  
**Test Coverage:** 80%+ tests passing

### Deliverables

**Production-Grade API Infrastructure:**
- Middleware stack: RequestId, RequestLogger, ErrorHandler, RequestSizeTracker
- Health endpoints: `/health` (liveness), `/ready` (readiness), `/status` (detailed)
- Comprehensive structured logging with request IDs and timing
- Thread-safe calibration repository with multi-feed support

**3D Coordinate-Based API:**
- Position3D with auto-detection (ECEF vs Geodetic based on magnitude >6400km)
- Full coordinate transformation pipeline: ECEF ↔ Geodetic ↔ Antenna Frame ↔ Spherical
- Beam squint correction for frequency-dependent pointing
- GeometryInfo in responses (feed offset, emitter direction)

**Complete Gain Computation Pipeline:**
- `POST /api/v1/gain` - Single gain evaluation
- Coordinate transforms → Physics model → **B-spline correction surface** → Final gain
- Optional reference gain and loss calculation
- Comprehensive input validation (positions, attitudes, frequencies)

**Key Implementation:**
- `src/api/schemas.rs` (1214 lines) - All request/response types
- `src/data/repository.rs` (367 lines) - Calibration artifact management
- `src/service/evaluator.rs` - 7-step gain computation pipeline
- `src/model/correction_interpolator.rs` (462 lines) - B-spline interpolation **FULLY INTEGRATED**
- `src/model/coordinates_3d.rs` - WGS84 transformations
- `src/service/validator.rs` (924 lines) - Comprehensive validation

### Key Files Created

- `src/api/middleware.rs` - 4 middleware components
- `src/api/schemas.rs` - 30+ schema types
- `src/data/loader.rs`, `src/data/repository.rs` - Calibration management
- `src/service/evaluator.rs`, `src/service/validator.rs` - Business logic
- `src/model/correction_interpolator.rs` - B-spline evaluation (Cox-de Boor)
- `src/model/coordinates_3d.rs` - 3D transformations

### Test Coverage

- **Middleware:** 23 tests (execution order, request IDs, timing, error handling)
- **Schemas:** 34 tests (serialization, coordinate detection, validation)
- **Repository:** 34 tests (loading, multi-feed, concurrent access)
- **Validation:** 32 tests (coordinates, attitudes, frequencies, batch limits)
- **Total:** 120+ new tests, all passing

### Architecture Highlights

**Calibration Repository Structure:**
```
HashMap<antenna_id, HashMap<feed_id, AntennaCalibration>>
├── physical_config (reflector, feed, mesh)
├── correction_surface (Optional<BSplineModel4D>)  // ✅ WORKING
└── validity_ranges (frequency, spatial, temperature)
```

**Gain Computation Flow:**
```
3D Positions (ECEF/Geodetic)
  → Auto-detect coordinate system
  → Transform to antenna frame (vehicle position + attitude)
  → Compute feed offset & emitter direction
  → Load calibration (antenna + feed config + correction surface)
  → Physics model (aperture integration, phase functions, Ruze)
  → B-spline correction surface evaluation  // ✅ INTEGRATED
  → Combine: Gain_final = Gain_physics + Correction
  → Optional: Reference gain & loss calculation
```

**Coordinate Auto-Detection:**
- If `|x| > 6400km OR |y| > 6400km OR |z| > 6400km` → ECEF
- Else → Geodetic (lon°, lat°, alt meters)

---

## Sprint 6: REST API - Advanced Endpoints & Partial Calibration ✅

**Status:** ✅ COMPLETE - 10/10 tasks complete (100%)  
**Duration:** ~4 weeks  
**Test Coverage:** 80%+ (468 total tests passing)

### Deliverables

**Advanced API Endpoints:**
- `POST /api/v1/gain/batch` - Batch processing (max 1000, parallel for ≥5 requests)
- `POST /api/v1/heatmap` - Loss heatmap generation (rectangular grids, H3 deferred)
- `GET /api/v1/antennas` - List all antennas with feeds
- `GET /api/v1/antennas/{id}` - Antenna details
- `GET /api/v1/antennas/{id}/feeds` - List feeds for antenna
- `GET /api/v1/antennas/{id}/feeds/{feed_id}` - Feed details

**Partial Calibration Phase 1 (Tasks 6.4-6.9):**
- **Data Model:** `CalibrationStatus` enum (Fully/Partially/Uncalibrated)
- **Configuration:** Parse design specs for uncalibrated antennas
- **Repository:** Load uncalibrated antennas from design specs (no .bin file)
- **API Schemas:** `CalibrationStatusInfo` in all responses
- **Service Layer:** Handle all calibration statuses with appropriate warnings
- **Use Case:** Loss analysis for uncalibrated antennas using physics model only

**API Documentation:**
- Complete OpenAPI 3.0 specification (47 schemas, 10 endpoints)
- Examples for all calibration statuses
- Coordinate system auto-detection documented

### Key Implementation

**Batch Processing** (`src/service/batch.rs`, 457 lines):
- Parallel processing using `rayon` for batches ≥5 requests
- Partial failures return NaN with error in warnings
- Max 1000 evaluations per batch
- 12 comprehensive tests

**Heatmap Generation** (`src/service/heatmap.rs`, 496 lines):
- Rectangular grid generation (azimuth × elevation)
- Parallel processing for grids ≥100 points
- Loss = peak_gain - gain at each grid point
- Max 100,000 grid points
- H3 hexagonal grids deferred (returns NotImplemented)
- 12 comprehensive tests

**Partial Calibration Support:**

**CalibrationStatus Enum:**
```rust
enum CalibrationStatus {
    FullyCalibrated { 
        accuracy_estimate_db: f64  // ±1 dB typical
    },
    PartiallyCalibrated { 
        accuracy_estimate_db: f64,  // ±1-3 dB
        coverage: CalibrationCoverage 
    },
    Uncalibrated { 
        accuracy_estimate_db: f64,        // ±3-5 dB absolute
        loss_accuracy_estimate_db: f64    // ±2 dB loss
    }
}
```

**Uncalibrated Antenna Loading:**
- Design specs in `antennas.yaml` (no .bin file required)
- Builds `PhysicalAntennaConfig` from specs
- Physics model only (no correction surface)
- Useful for loss analysis relative to ideal gain

**Example Configuration:**
```yaml
antennas:
  - antenna_id: "test_antenna"
    enabled: true
    calibration_status: "uncalibrated"
    design_specs:
      reflector:
        diameter_m: 1.2
        focal_length_m: 0.48
        surface_rms_mm: 0.5
      feeds:
        - feed_id: "x_band"
          position: { x: 0.0, y: 0.0, z: 0.48 }
          q_factor: 8.0
          frequency_range: { min_mhz: 7000, max_mhz: 8500 }
```

### Test Coverage

- **Batch:** 12 tests (empty, partial failures, size limits, parallel threshold)
- **Heatmap:** 12 tests (grids, H3 NotImplemented, parallel, emitter positions)
- **Antenna Endpoints:** 11 tests (list, details, 404 errors, multi-feed)
- **Partial Calibration:** 81 tests across 6 tasks
  - Data models: 18 tests
  - Config parsing: 14 tests
  - Loading: 12 tests
  - Schemas: 11 tests
  - Service layer: 17 tests
  - Integration: 4 tests
- **Total Sprint 6:** 116+ new tests, all passing

### Architecture Highlights

**Service Layer Coverage Check:**
```rust
fn is_in_coverage(
    coverage: &CalibrationCoverage,
    az: f64, el: f64, freq: f64
) -> bool {
    // Check if query point within measured coverage
    // If out of coverage → physics model only, no correction
}
```

**Calibration Warnings:**
```rust
fn generate_calibration_warnings(status: &CalibrationStatus) -> Vec<String> {
    match status {
        Uncalibrated => vec!["Using uncalibrated antenna - physics model only, no correction surface"],
        PartiallyCalibrated if !in_coverage => vec!["Query outside calibrated coverage region"],
        _ => vec![]
    }
}
```

**Parallel Processing Strategy:**
- Batch: Sequential <5 requests, parallel ≥5 requests
- Heatmap: Sequential <100 points, parallel ≥100 points
- Uses `rayon` for CPU-bound parallel evaluation

### Files Created

- `src/service/batch.rs` (457 lines)
- `src/service/heatmap.rs` (496 lines)
- Updated `src/data/types.rs` (+328 lines for calibration status types)
- Updated `src/config/settings.rs` (+704 lines for design specs parsing)
- Updated `src/data/repository.rs` (+169 lines for uncalibrated loading)
- Updated `src/api/schemas.rs` (+250 lines for calibration status info)
- Updated `src/service/evaluator.rs` (+437 lines for status handling)
- `openapi.yaml` (comprehensive API documentation)

### Performance

- Batch: 10-20 req/s for small batches (parallel)
- Heatmap: ~3312 points (72×46) in <2 seconds expected
- Single evaluation: <100ms p95 latency
- Parallel threshold tuning for optimal performance

---

**Next:** Sprint 7 - Boresight Calibration Tool & Testing (36% complete, 4/11 tasks)

