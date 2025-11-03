# Antenna Model Service - Implementation Plan

## Document Overview

**Version:** 2.0 (Revised for Physical Optics Model)
**Created:** 2025-10-22
**Last Revised:** 2025-10-26
**Target Timeline:** 8 sprints (16 weeks)
**Scope:** MVP with physical optics computation engine, REST API, calibration tool, and Kubernetes deployment

**Key Change from v1.0:** This plan now implements a full **physical optics antenna model** (Ruze equation, coma aberration, mesh effects) rather than a simple interpolation service. The calibration tool optimizes physical parameters to match measurements instead of fitting B-splines.

This implementation plan breaks down the Antenna Model Service into manageable sprints, each containing tasks scoped for a mid-level engineer to complete within a 2-week period.

---

## Sprint Overview

| Sprint | Focus Area | Duration | Status | Key Deliverables |
|--------|-----------|----------|--------|-----------------|
| Sprint 1 | Project Foundation & Core Data Types | 2 weeks | ✅ Complete | Repository structure, basic REST API with /status endpoint, core data types, basic tests |
| Sprint 2 | Physical Optics Computation Engine | 2 weeks | ✅ Complete | Aperture integration, phase functions (path, coma, surface, mesh), far-field pattern computation |
| Sprint 3 | Surface Error & Mesh Reflector Models | 2 weeks | ✅ Complete | Ruze equation, mesh transparency, coordinate transformations, edge case handling |
| Sprint 4 | Calibration via Parameter Optimization | 2 weeks | ✅ Complete | Physical parameter fitting, differential evolution optimizer, Zernike polynomials, correction surfaces, CLI tool |
| Sprint 5 | REST API - Core Endpoints | 2 weeks | 📋 Pending | Production middleware, enhanced health checks, single evaluation endpoints |
| Sprint 6 | REST API - Advanced Endpoints | 2 weeks | 📋 Pending | Batch processing, heatmap generation |
| Sprint 7 | Integration & Performance Testing | 2 weeks | 📋 Pending | End-to-end tests, performance benchmarks, validation against measurements |
| Sprint 8 | Deployment & Documentation | 2 weeks | 📋 Pending | Docker, Kubernetes, operational docs |

---

## Sprints 1-4: Foundation Complete ✅

**Status:** ALL COMPLETE (100%)
**Duration:** 8 weeks
**Tests:** 280+ passing

### Summary

**Sprint 1 - Project Foundation:**
- Cargo workspace: `antenna-model` (service) + `calibrate` (CLI tool)
- Basic REST API with `/status` endpoint (poem framework)
- Core data types: `AntennaCalibration`, `BSplineModel4D`, `ValidityRanges`, `PhysicalAntennaConfig`
- Configuration system (YAML + env vars)
- Error framework (`thiserror`-based)

**Sprint 2 - Physical Optics Engine:**
- Antenna geometry structures (`ReflectorGeometry`, `FeedParameters`, `MeshParameters`)
- Phase functions: path, coma, surface (Zernike), mesh
- Feed illumination (cos^q pattern with q-factor)
- Aperture integration (adaptive Simpson's rule)
- Far-field pattern with Ruze efficiency and mesh transparency

**Sprint 3 - Coordinate Systems & Edge Cases:**
- Coordinate transforms: ECEF ↔ Geodetic ↔ Antenna Frame ↔ Spherical
- Auto-detection: |x,y,z| > 6400km → ECEF, else Geodetic
- Edge case handling (large feed offsets, ray tracing, direct paths)
- Numerical stability improvements

**Sprint 4 - Calibration Tool:**
- Parameter optimization via differential evolution
- Correction surface fitting (B-spline/RBF): residual = measured - physics_model
- CLI tool: measurements CSV → parameter tuning → correction surface → binary artifact
- Validation: <1 dB max error in main lobe and first sidelobe

### Available Modules

**Complete physics pipeline** in `antenna-model/src/model/`:
- `coordinates.rs` - All coordinate transformations
- `geometry.rs` - Reflector, feed, mesh structures
- `phase.rs` - Path, coma, surface error, mesh phases
- `illumination.rs` - Feed patterns
- `integration.rs` - Aperture integration
- `pattern.rs` - Gain computation with Ruze/mesh
- `edge_cases.rs` - Large offsets, ray tracing
- `numerical_stability.rs` - Stability helpers

**Calibration pipeline** in `calibrate/src/`:
- `parser.rs` - CSV measurement parsing
- `parameter_tuner.rs` - Differential evolution
- `correction_surface.rs` - B-spline/RBF fitting
- `antenna_config.rs` - Config extraction
- `validator.rs` - Cross-validation
- `serializer.rs` - Binary artifact generation

### Key Technical Context for Sprint 5+

1. **Calibration Artifacts:** Binary files (`*.bin`) contain `AntennaCalibration` with `PhysicalAntennaConfig` + `BSplineModel4D` correction surface
2. **Gain Computation Pipeline:** 3D coords → transform to antenna frame → physics model → correction interpolation → final gain
3. **Multi-Feed Support:** Composite `(antenna_id, feed_id)` identifiers, different corrections per feed
4. **Performance Targets:** <100ms single eval, 1-20 req/s batch, <2s for 3312-point heatmap
5. **Interpolation Needed:** 4D B-spline for correction surfaces (not yet implemented - Sprint 5 task)

📄 **Detailed summary:** See `docs/implementation-plan-sprints-1-4-summary.md`

---

## Sprint 5: REST API - Core Endpoints

**Goal:** Enhance REST API with production middleware, comprehensive health checks, and core evaluation endpoints

**Status:** 🔄 IN PROGRESS - 6/7 tasks complete (86%)

**Note:** Basic REST API server and status endpoint were established in Sprint 1. This sprint focuses on production-grade enhancements and evaluation functionality.

### Tasks

#### 5.1 API Server Enhancement & Middleware (3-4 days) ✅ COMPLETE
**Objective:** Enhance existing API server with production-grade middleware

**Note:** Basic API server and status endpoint were established in Sprint 1, Task 1.2. This task builds upon that foundation.

**Steps:**
- ✅ Enhance `src/api/mod.rs` with production features:
  - ✅ Integrate with configuration system for advanced settings
  - ✅ Add connection pooling and resource management (state management)
  - ✅ Implement proper state management
- ✅ Implement middleware in `src/api/middleware.rs`:
  - ✅ Request ID generation and propagation (UUID v4 with header support)
  - ✅ Comprehensive structured logging (using `tracing` with timing)
  - ✅ Request/response timing and metrics (elapsed time in ms)
  - ✅ Error handling middleware (consistent error logging)
  - ✅ CORS support (not implemented, not needed for initial deployment)
- ✅ Enhance startup and shutdown sequences with detailed logging
- ✅ Add request/response size tracking (with configurable warning thresholds)

**Acceptance Criteria:**
- ✅ Server uses configuration from settings file (ServiceConfig with YAML and env vars)
- ✅ All requests get unique request IDs in logs (x-request-id header)
- ✅ Request/response logs are structured JSON with timing (configurable format)
- ✅ Middleware chain executes in correct order (Tracing → RequestId → RequestLogger → ErrorHandler → RequestSizeTracker)
- ✅ Error responses are consistently formatted (via ErrorHandler middleware)

**Files Updated/Created:**
- ✅ Updated `src/api/mod.rs` with enhanced features (configuration integration, start_server_with_config)
- ✅ Updated `src/api/middleware.rs` with production middleware (RequestId, RequestLogger, ErrorHandler, RequestSizeTracker)
- ✅ Updated `src/main.rs` with middleware integration (uses start_server_with_config)
- ✅ Updated `src/api/routes.rs` with middleware stack
- ✅ Added `uuid` dependency to Cargo.toml

**Test Coverage:**
- ✅ Middleware execution order (test_middleware_chain_order)
- ✅ Request ID generation and propagation (test_request_id_generation, test_request_id_propagation)
- ✅ Timing measurement accuracy (test_timing_measurement)
- ✅ Error middleware handling (test_error_handler)
- ✅ Request size tracking (test_request_size_tracker)
- ✅ Request logger success and error cases
- ✅ **Total: 8 middleware tests + 9 route tests + 6 mod tests = 23 new tests, all passing**

---

#### 5.2 Request/Response Schemas (4-5 days) ✅ COMPLETE
**Objective:** Define API contract with typed schemas supporting 3D coordinate-based queries

**Steps:**
- ✅ Create `src/api/schemas.rs` with:
  - ✅ `Position3D` - 3D position (auto-detects ECEF vs Geodetic based on magnitude)
  - ✅ `Quaternion` and `EulerAngles` - vehicle attitude representation
  - ✅ `GainRequest` - gain computation input (replaces EvaluationRequest)
  - ✅ `GainResponse` - gain computation output with optional reference gain
  - ✅ `HeatmapRequest` - 2D heatmap generation input
  - ✅ `HeatmapResponse` - 2D heatmap output
  - ✅ `GeometryInfo` - computed geometry details (feed offset, emitter direction)
  - ✅ `GridConfig` - grid configuration (rectangular or H3 hexagonal)
  - ✅ `ErrorResponse` - standardized error format
  - ✅ `HealthResponse` - health check response
  - ✅ `StatusResponse` - enhanced service status
  - ✅ `AntennaInfo` - antenna metadata including feed configurations
  - ✅ Additional types: `Vector3D`, `Attitude` enum, `BatchGainRequest`, `BatchGainResponse`, `AntennaListResponse`, `AntennaDetailsResponse`, `FeedInfo`, `ValidityRangesInfo`, `CalibrationInfo`, `PhysicalParametersInfo`, `MeshInfo`, `ComputationMetadata`, `BatchMetadata`, `HeatmapMetadata`, `RangeConfig`, `GridData`
- ✅ Implement `serde` serialization with proper field naming (snake_case)
- ✅ Coordinate system auto-detection logic in Position3D implemented:
  - ✅ If `abs(x) > 6400e3 OR abs(y) > 6400e3 OR abs(z) > 6400e3` → ECEF
  - ✅ Otherwise → Geodetic (lon degrees, lat degrees, alt meters)
- ✅ Write schema documentation with coordinate system examples

**Acceptance Criteria:**
- ✅ All schemas serialize/deserialize correctly
- ✅ JSON field names match API spec (section 4.3 of architecture doc) - snake_case
- ✅ Position3D auto-detection works reliably (tested with threshold checks)
- ✅ Composite `(antenna_id, feed_id)` identifiers supported
- ✅ Schema documentation is clear with coordinate examples
- ✅ Example JSON payloads are valid

**Files Created:**
- ✅ `src/api/schemas.rs` (1214 lines, comprehensive schema definitions)
- ✅ `examples/api_requests.json` (example payloads with ECEF and Geodetic)

**Test Coverage:**
- ✅ Serialization/deserialization round-trips (all major types)
- ✅ Position3D coordinate system auto-detection (ECEF vs Geodetic, boundary cases)
- ✅ Field naming conventions (snake_case verification)
- ✅ Quaternion validation (magnitude, normalization checks)
- ✅ EulerAngles validation
- ✅ Vector3D operations
- ✅ RangeConfig calculations
- ✅ GridConfig serialization (Rectangular and H3)
- ✅ ErrorResponse helper methods
- ✅ StatusResponse and HealthResponse
- ✅ **Total: 34 schema tests, all passing**

**Example Schema:**
```rust
#[derive(Serialize, Deserialize)]
pub struct Position3D {
    pub x: f64,  // ECEF X (meters) OR longitude (degrees)
    pub y: f64,  // ECEF Y (meters) OR latitude (degrees)
    pub z: f64,  // ECEF Z (meters) OR altitude (meters)
}

#[derive(Serialize, Deserialize)]
pub struct GainRequest {
    pub antenna_id: String,
    pub feed_id: String,
    pub vehicle_position: Position3D,
    pub vehicle_attitude: Quaternion,
    pub reflector_boresight: Position3D,
    pub feed_position: Position3D,
    pub emitter_position: Position3D,
    pub frequency_mhz: f64,
    pub pointing_frequency_mhz: Option<f64>,  // For beam squint correction
    pub include_reference: bool,  // Include ideal gain for loss calculation
}

#[derive(Serialize, Deserialize)]
pub struct GainResponse {
    pub antenna_id: String,
    pub feed_id: String,
    pub gain_db: f64,
    pub reference_gain_db: Option<f64>,  // If include_reference=true
    pub loss_db: Option<f64>,  // reference - actual (if reference computed)
    pub geometry: GeometryInfo,
    pub warnings: Vec<String>,
    pub metadata: ComputationMetadata,
}
```

---

#### 5.3 Enhanced Health & Status Endpoints (2-3 days) ✅ COMPLETE
**Objective:** Enhance operational endpoints with comprehensive service information

**Note:** Basic `/status` endpoint was created in Sprint 1, Task 1.2. This task expands it with calibration-aware health checks.

**Steps:**
- ✅ Enhance `src/api/handlers.rs` with:
  - ✅ `GET /health` - liveness probe (returns 200 if responsive)
  - ✅ `GET /ready` - readiness probe (returns 200/503 based on ready state)
  - ✅ Enhance `GET /status` - add detailed service status
- ✅ Enhanced status endpoint returns:
  - ✅ Server uptime (already implemented)
  - ✅ Build version/commit (already implemented)
  - ✅ Loaded antenna count and IDs (new)
  - ✅ Memory usage (new, Linux only via /proc/self/statm)
  - ✅ Calibration data status (via antenna_ids in AppState)
- ✅ Add readiness check logic:
  - ✅ Readiness state tracking via AtomicBool in AppState
  - ✅ Antenna IDs tracked in AppState (ready for Task 5.4)
- ✅ Implement separate liveness check (verify service is responsive)
- ✅ Differentiate between `/health` (liveness) and `/ready` (readiness)

**Acceptance Criteria:**
- ✅ `/health` returns 200 when service is responsive (liveness)
- ✅ `/ready` returns 200 when ready, 503 during startup or if data fails to load (readiness)
- ✅ `/status` returns comprehensive service information including antenna count
- ✅ Endpoints respond quickly (<10ms verified in tests)

**Files Updated/Created:**
- ✅ Updated `src/api/handlers.rs` with health, ready, and enhanced status handlers
- ✅ Updated `src/api/routes.rs` with /health, /ready, /status routes
- ✅ Updated `src/api/mod.rs` with readiness state and antenna ID tracking
- ✅ Added `parking_lot` dependency to Cargo.toml for RwLock

**Test Coverage:**
- ✅ Health endpoint (always succeeds as liveness check)
- ✅ Ready endpoint (200 when ready, 503 when not ready)
- ✅ Readiness state transitions (testing mark_ready/mark_not_ready)
- ✅ Enhanced status endpoint with antenna information
- ✅ Memory usage tracking (platform-specific)
- ✅ All endpoints present and functional
- ✅ **Total: 11 new handler tests + 8 new route tests = 19 new tests, all passing**

**Implementation Notes:**
- Memory usage tracking implemented for Linux via /proc/self/statm (RSS)
- Returns None on non-Linux platforms (can be enhanced later with platform-specific code)
- AppState now includes:
  - `ready: Arc<AtomicBool>` for readiness tracking
  - `antenna_ids: Arc<parking_lot::RwLock<Vec<String>>>` for antenna tracking
- Ready for integration with Task 5.4 (Calibration Repository)

---

#### 5.4 Calibration Data Repository (3-4 days) ✅ COMPLETE
**Objective:** Implement loading and management of calibration artifacts (antenna configs + correction surfaces + feed configurations)

**Implementation:**
- ✅ Extended `AntennaCalibration` structure in `src/data/types.rs`:
  - Added `feed_id` field for multi-feed support
  - Updated builder and validation logic
  - Each calibration artifact represents one antenna-feed combination
- ✅ Created `src/data/loader.rs`:
  - `load_calibration_artifact(path)` - deserialize and validate binary artifacts
  - `validate_calibration()` - deep validation beyond basic checks
  - Comprehensive logging of loaded data (physical config, correction surface, validity ranges)
  - Warnings for old format versions, poor quality metrics
- ✅ Created `src/data/repository.rs`:
  - `CalibrationRepository` with nested `HashMap<antenna_id, HashMap<feed_id, AntennaCalibration>>`
  - `load_from_config()` - loads all enabled antennas from configuration
  - `get_calibration(antenna_id, feed_id)` - retrieve full calibration
  - `get_antenna_config()` - retrieve physical antenna configuration
  - `get_correction_surface()` - retrieve optional B-spline correction
  - `get_validity_ranges()` - retrieve valid parameter ranges
  - `list_antennas()` - return all antenna IDs (sorted)
  - `list_feeds(antenna_id)` - return all feeds for antenna (sorted)
  - `has_calibration()` - check if antenna-feed exists
  - Thread-safe using `Arc<RwLock<>>` with parking_lot
- ✅ Updated `src/data/mod.rs` to export new modules
- ✅ Updated `src/error.rs` with new DataError variants:
  - `LoadError` - general loading failures
  - `ValidationError` - validation failures
  - `ConfigurationError` - configuration issues
  - Conversion from `ConfigError` to `DataError`

**Acceptance Criteria:** ✅ ALL MET
- ✅ Repository loads all configured antennas with their feeds at startup
- ✅ Three components accessible: antenna config + correction surface + validity ranges
- ✅ Composite `(antenna_id, feed_id)` lookups work correctly
- ✅ Thread-safe concurrent access (using parking_lot RwLock)
- ✅ Clear logging of loaded antennas and feeds
- ✅ Fail-fast on corrupted or missing artifacts (configurable)
- ✅ Feed configurations stored in physical_config.feed

**Files Created/Modified:**
- ✅ Created `src/data/repository.rs` (367 lines, 15 public methods, 13 tests)
- ✅ Created `src/data/loader.rs` (239 lines, 2 public functions, 5 tests)
- ✅ Updated `src/data/mod.rs` to export loader and repository
- ✅ Updated `src/data/types.rs` to add `feed_id` field to AntennaCalibration
- ✅ Updated `src/error.rs` with new error variants

**Test Coverage:** ✅ COMPREHENSIVE (34 new tests, all passing)
- ✅ Loading calibration artifacts (success, file not found, invalid data)
- ✅ Validation (success, invalid elevation range, with correction surface)
- ✅ Repository operations (add, get, list antennas/feeds, concurrent access)
- ✅ Composite (antenna_id, feed_id) lookup (found and not found)
- ✅ Artifact deserialization (bincode round-trip)
- ✅ Configuration integration (load from config, fail-fast, disabled antennas)
- ✅ Thread safety (clone, concurrent access)

**Multi-Feed Support:**
- Each .bin file represents one antenna-feed combination
- Repository aggregates multiple files with same antenna_id
- Supports lookups like `get_calibration("antenna_1", "x_band")`
- Feed parameters stored in `physical_config.feed` (position, q_factor, phase_center_offset)

---

#### 5.5 Gain Computation Endpoint with Coordinate Transforms (5-6 days) ✅ COMPLETE
**Objective:** Implement gain computation endpoint with 3D coordinate transformations, physics model, and correction surface

**Steps:**
- Create service layer in `src/service/evaluator.rs`:
  - `compute_gain(request: GainRequest)` - orchestrate gain computation from 3D positions
  - **Step 1: Transform coordinates**:
    - Auto-detect coordinate system (ECEF vs Geodetic) for all positions
    - Convert all positions to antenna frame using vehicle position and attitude
    - Compute feed offset vector from reflector boresight
    - Compute emitter direction (azimuth, elevation) in antenna frame
    - Apply beam squint correction if pointing_frequency ≠ operating_frequency
  - **Step 2: Load antenna config, feed config, and correction surface** from repository using composite (antenna_id, feed_id)
  - **Step 3: Compute base prediction** using physical optics model (Sprint 2-3) with:
    - Antenna configuration parameters
    - Feed offset from boresight
    - Emitter direction
    - Operating frequency
  - **Step 4: Evaluate correction surface** using B-spline interpolation
  - **Step 5: Combine**: `Gain_final = Gain_physics + Correction`
  - **Step 6 (if include_reference=true)**: Compute reference gain (ideal: feed at focus, pointing at emitter)
  - **Step 7 (if reference computed)**: Calculate loss = reference_gain - actual_gain
  - Generate warnings for:
    - Out-of-range queries (extrapolated regions in correction surface)
    - Physical model edge cases (large feed offsets, etc.)
    - Coordinate transformation issues (singularities, large uncertainties)
    - Beam squint correction applied
  - Track computation time for each step
- Create `src/model/correction_interpolator.rs`:
  - `evaluate_correction(correction_surface, freq, cone, clock)` - B-spline interpolation
  - Reuse B-spline evaluation code from Sprint 1 data types (repurposed)
  - Handle out-of-range gracefully (return warning, use nearest or zero)
- Add handler in `src/api/handlers.rs`:
  - `POST /api/v1/gain`
  - Request validation and parsing
  - Error handling and response formatting
  - Return GeometryInfo with computed geometry details
  - Logging with structured fields (include coordinates, geometry, physics, and correction values)
- Integrate with calibration repository (Task 5.4) using composite identifiers
- Integrate with coordinate transformation module (Task 5.7)
- Implement detailed error responses

**Acceptance Criteria:**
- Endpoint returns correct gain values combining physics model + corrections
- Coordinate transformations work for both ECEF and Geodetic inputs
- Auto-detection correctly identifies coordinate systems
- Response includes GeometryInfo with feed offset and emitter direction
- Optional reference gain and loss calculation works correctly
- Beam squint correction applied when pointing_frequency differs
- Out-of-range queries include appropriate warnings
- Correction surface evaluation works correctly
- Error responses follow standard format
- Response time <150ms (p95) for typical queries (includes coordinate transforms)
- Comprehensive logging for debugging with geometry details

**Files to Create:**
- `src/service/evaluator.rs`
- `src/model/correction_interpolator.rs` (B-spline evaluation for corrections)
- `src/service/mod.rs`
- Update `src/api/handlers.rs` and `src/api/routes.rs`

**Test Coverage:**
- Valid gain requests with ECEF coordinates (physics + correction + reference)
- Valid gain requests with Geodetic coordinates
- Coordinate system auto-detection accuracy
- Coordinate transformation accuracy
- Beam squint correction application
- Loss calculation (reference vs actual)
- Correction surface interpolation accuracy
- Combined model output validation
- Antenna or feed not found errors
- Out-of-range parameter warnings
- Invalid parameter errors (bad coordinates, attitudes)
- Response format validation including GeometryInfo
- Integration tests with real calibration data

**Note:** This is where the complete system comes together: `CoordinateTransform → PhysicsModel + CorrectionSurface = Final Gain`. Replaces the original simple evaluation endpoint with full 3D geometric modeling.

---

#### 5.6 Input Validation Layer (3-4 days)
**Objective:** Implement comprehensive input validation for 3D coordinate-based requests

**Steps:**
- Create `src/service/validator.rs` with:
  - `validate_gain_request()` - check all gain computation parameters
  - `validate_heatmap_request()` - check all heatmap parameters
  - Position validation:
    - ECEF coordinates reasonable (|x|, |y|, |z| < 10000 km)
    - Geodetic coordinates reasonable (lon: -180 to 180, lat: -90 to 90, alt < 1000 km)
    - Detect obviously invalid coordinates (e.g., NaN, Inf)
  - Attitude validation:
    - Quaternion normalization check (magnitude ≈ 1)
    - Euler angle ranges
  - Composite identifier validation:
    - `(antenna_id, feed_id)` exists in repository
  - Frequency range validation
  - Generate specific error messages per field
- Add validation to all API handlers
- Implement custom validation error types including coordinate errors

**Acceptance Criteria:**
- All invalid inputs are caught before processing
- Error messages specify which field failed and why
- Coordinate validation catches common errors
- Attitude validation ensures valid rotations
- Composite identifier validation works correctly
- Validation logic is reusable across endpoints
- Tests cover all validation rules

**Files to Create:**
- `src/service/validator.rs`
- Update error types to include validation errors and coordinate-specific errors

**Test Coverage:**
- Each validation rule individually
- Position validation (ECEF and Geodetic edge cases)
- Attitude validation (invalid quaternions, out-of-range Euler angles)
- Composite identifier validation
- Multiple validation failures
- Edge cases (boundary values, NaN, Inf, special characters)

---

#### 5.7 Coordinate Transformation Module (4-5 days) ✅ COMPLETE
**Objective:** Implement comprehensive 3D coordinate transformations for antenna gain computation

**Steps:**
- Create `src/model/coordinates_3d.rs` with:
  - **Coordinate System Detection**:
    - `detect_coordinate_system(pos: Position3D)` - auto-detect ECEF vs Geodetic
    - Detection logic: if `abs(x) > 6400e3 OR abs(y) > 6400e3 OR abs(z) > 6400e3` → ECEF
  - **ECEF ↔ Geodetic Transformations**:
    - `geodetic_to_ecef(lon, lat, alt)` - WGS84 conversion
    - `ecef_to_geodetic(x, y, z)` - inverse conversion (Bowring's method or iterative)
  - **ECEF → Antenna Frame Transformation**:
    - `ecef_to_antenna_frame(ecef_pos, vehicle_pos, vehicle_attitude)` - transform to antenna-centered coordinates
    - Apply vehicle attitude (quaternion or Euler angles)
    - Compute East-North-Up (ENU) frame at vehicle location
    - Transform to antenna mounting frame
  - **Antenna Frame → Spherical Coordinates**:
    - `antenna_frame_to_spherical(x, y, z)` - convert to (azimuth, elevation, range)
    - Handle singularities at zenith/nadir
  - **Geometric Computations**:
    - `compute_feed_offset(feed_pos, boresight_pos, antenna_frame)` - feed displacement from boresight
    - `compute_emitter_direction(emitter_pos, antenna_frame)` - (azimuth, elevation) to emitter
  - **Beam Squint Correction**:
    - `apply_beam_squint(direction, pointing_freq, operating_freq, antenna_params)` - frequency-dependent pointing offset
- Implement WGS84 ellipsoid parameters as constants
- Add comprehensive error handling for:
  - Invalid coordinates (singularities, out-of-bounds)
  - Gimbal lock in attitude transformations
  - Numerical precision issues
- Document coordinate conventions (right-hand rule, angle definitions)

**Acceptance Criteria:**
- Auto-detection correctly identifies ECEF vs Geodetic coordinates
- ECEF ↔ Geodetic transformations accurate to <1 meter
- Attitude transformations preserve vector magnitudes
- Spherical coordinate computation handles all quadrants correctly
- Beam squint correction applies frequency-dependent offsets correctly
- Singularities handled gracefully with clear error messages
- Comprehensive unit tests with known reference transformations

**Files to Create:**
- `src/model/coordinates_3d.rs`
- Update `src/model/mod.rs` to export coordinate functions

**Test Coverage:**
- Coordinate system auto-detection (ECEF, Geodetic, edge cases)
- ECEF ↔ Geodetic round-trip accuracy (multiple reference points)
- Attitude transformations (quaternion and Euler angles)
- ECEF → Antenna frame → Spherical (full pipeline)
- Feed offset computation
- Emitter direction computation
- Beam squint correction at different frequency ratios
- Singularity handling (poles, gimbal lock)
- Edge cases (coordinates near Earth center, very high altitudes)

**Reference Data for Testing:**
- Use published WGS84 test vectors
- NASA/JPL reference frames
- Known antenna locations and orientations

**Note:** This module is critical infrastructure for the new API. All geometric computations depend on correct coordinate transformations.

---

### Sprint 5 Deliverables

- Production-grade REST API with middleware (built on Sprint 1 foundation)
- Enhanced health and status endpoints for K8s probes
- **3D coordinate-based API schemas** with auto-detection (ECEF/Geodetic)
- **Coordinate transformation module** (ECEF ↔ Geodetic ↔ Antenna Frame ↔ Spherical)
- **Calibration data repository** loading antenna configs + correction surfaces + feed configurations
- **Gain computation endpoint** with full geometric pipeline:
  - Coordinate transformations (3D positions → antenna frame)
  - Physics model evaluation
  - Correction surface interpolation
  - Optional reference gain and loss calculation
  - Beam squint correction
- B-spline interpolation for correction surfaces
- Complete pipeline: `3D Coordinates → Transform → PhysicsModel + CorrectionSurface = Gain`
- Composite `(antenna_id, feed_id)` identifier support
- Comprehensive input validation for coordinates and attitudes
- Comprehensive error handling and response formatting
- Advanced structured logging with request IDs, timing, and geometry details
- Integration tests with calibration data and coordinate transforms
- 80%+ test coverage

---

## Sprint 6: REST API - Advanced Endpoints

**Goal:** Implement batch processing, heatmap generation, and antenna listing endpoints

### Tasks

#### 6.1 Batch Evaluation Endpoint (4-5 days)
**Objective:** Support multiple evaluations in a single request

**Steps:**
- Create `src/service/batch.rs` with:
  - `evaluate_batch()` - process multiple evaluation requests
  - Parallel processing using `rayon` for independent evaluations
  - Result aggregation and error collection
  - Overall timing metrics
- Add handler for `POST /api/v1/evaluate/batch`
- Implement request size limits (e.g., max 1000 evaluations per batch)
- Add progress tracking for large batches
- Generate aggregate statistics

**Acceptance Criteria:**
- Batch processing faster than sequential single evaluations
- Partial failures handled gracefully (some succeed, some fail)
- Response includes both results and errors
- Request size limits enforced
- Throughput meets 1-20 req/s target per instance

**Files to Create:**
- `src/service/batch.rs`
- Update `src/api/handlers.rs` and `src/api/routes.rs`
- Update `src/api/schemas.rs` with batch request/response types

**Test Coverage:**
- Small batch (5 evaluations)
- Large batch (100+ evaluations)
- Mixed valid/invalid requests
- Performance benchmarks (throughput, latency distribution)
- Parallel execution verification

**Performance Target:**
- Batch of 100 evaluations completes in <500ms

---

#### 6.2 Heatmap Generation Endpoint (6-7 days)
**Objective:** Generate 2D loss heatmaps across antenna field of view using rectangular or H3 hexagonal grids

**Steps:**
- Create heatmap generation logic in `src/service/evaluator.rs`:
  - `generate_heatmap(request: HeatmapRequest)` - evaluate grid of emitter positions
  - **Grid Generation**:
    - **Rectangular Grid** (default):
      - Generate azimuth/elevation grid from range specifications
      - Convert grid points to emitter positions in 3D space
    - **H3 Hexagonal Grid** (optional):
      - Integrate `h3o` or `h3ron` crate for H3 cell generation
      - Generate H3 cells at specified resolution covering field of view
      - Convert H3 cell centers to emitter positions
  - **For each grid point**:
    - Compute emitter position in 3D space (antenna frame or ECEF)
    - Call gain computation (same as Task 5.5)
    - Compute loss relative to peak gain (reference = ideal pointing)
  - Efficient parallel evaluation using `rayon`
  - Handle extrapolation warnings for grid points
  - Clip grid based on antenna beamwidth at operating frequency (optional optimization)
- Add handler for `POST /api/v1/heatmap`
- Implement response optimization:
  - Optional data compression
  - Configurable output resolution
  - Streaming response for large grids (future enhancement)
- Add heatmap-specific validation:
  - Reasonable grid sizes (warn if > 10,000 points)
  - Valid azimuth/elevation ranges or H3 resolution
  - Beamwidth clipping parameters

**Acceptance Criteria:**
- Heatmaps generated for specified azimuth/elevation ranges (rectangular grid)
- H3 hexagonal grid option works correctly
- Grid spacing/resolution configurable via API
- Loss values computed relative to peak gain
- Performance acceptable for typical grids (72x46 = 3312 points rectangular, or H3 res 6-8)
- Warnings aggregated for out-of-range regions
- Response size reasonable (<1MB for typical heatmaps)
- Coordinate transformations work correctly for all grid points
- Optional beamwidth clipping reduces computation for off-axis points

**Files to Create:**
- Update `src/service/evaluator.rs` with heatmap logic
- Update `src/api/schemas.rs` with heatmap request/response types
- Add heatmap handler to `src/api/handlers.rs`
- Optional: `src/model/h3_grid.rs` for H3 grid generation

**Dependencies:**
- Add `h3o` or `h3ron` crate (if H3 support included)

**Test Coverage:**
- Small rectangular grid (10x10)
- Large rectangular grid (100x100)
- H3 grid at different resolutions (res 6, 7, 8)
- Partial out-of-range grid (some extrapolated points)
- Loss calculation accuracy (verify relative to peak)
- Coordinate transformation for grid points
- Performance benchmarks (rectangular vs H3)
- Response format validation (both grid types)

**Performance Target:**
- 72x46 rectangular grid (3312 points) completes in <2 seconds
- H3 resolution 7 (~5000 cells) completes in <3 seconds

**Note:** H3 grid support is optional for MVP. Can be deferred to future sprint if time-constrained. Rectangular azimuth/elevation grid is sufficient for initial deployment.

---

#### 6.3 Antenna Listing & Details Endpoints (3-4 days)
**Objective:** Allow clients to query available antennas, feeds, and their properties

**Steps:**
- Add `GET /api/v1/antennas` endpoint:
  - List all loaded antenna IDs with available feeds
  - Include basic metadata (name, enabled status, feed count)
  - Sort alphabetically
- Add `GET /api/v1/antennas/{id}` endpoint:
  - Return detailed antenna information
  - List of available feeds with their configurations
  - Validity ranges for all dimensions
  - Calibration metadata (date, version, etc.)
  - Model statistics (knot counts, coefficient counts)
  - Physical parameters (diameter, f/D ratio, etc.)
- Add `GET /api/v1/antennas/{id}/feeds` endpoint:
  - List all feeds for a specific antenna
  - Include feed positions and frequency ranges
- Add `GET /api/v1/antennas/{id}/feeds/{feed_id}` endpoint:
  - Return detailed feed configuration
  - Feed position (offset from focal point)
  - Feed pattern parameters
  - Frequency range and beamwidth
- Implement caching for antenna/feed lists (static after startup)

**Acceptance Criteria:**
- Antenna list returns all configured antennas with feed counts
- Antenna details include all relevant metadata and feeds
- Feed listing works for multi-feed antennas
- Feed details include position and pattern parameters
- 404 error for unknown antenna or feed IDs
- Response times <50ms for all endpoints
- Composite `(antenna_id, feed_id)` pairs are discoverable

**Files to Create:**
- Update `src/api/handlers.rs` with antenna/feed list/details handlers
- Update `src/api/schemas.rs` with antenna and feed info types

**Test Coverage:**
- List all antennas (with feeds)
- Get details for existing antenna (with feeds list)
- List feeds for specific antenna
- Get details for specific feed
- Get details for non-existent antenna (404)
- Get details for non-existent feed (404)
- Metadata accuracy for antennas and feeds
- Multi-feed antenna support

---

#### 6.4 API Documentation & OpenAPI Spec (2-3 days)
**Objective:** Generate API documentation for clients

**Steps:**
- Integrate `poem-openapi` or manually create OpenAPI 3.0 spec
- Document all endpoints with:
  - Request/response schemas
  - Example payloads
  - Error responses
  - Parameter descriptions
- Generate interactive API documentation (Swagger UI or similar)
- Host documentation at `/api/docs` endpoint

**Acceptance Criteria:**
- OpenAPI spec is valid and complete
- All endpoints documented with examples
- Interactive docs accessible via browser
- Examples can be executed directly from docs

**Files to Create:**
- `openapi.yaml` or generated via `poem-openapi`
- `src/api/docs.rs` (if needed for doc endpoint)

**Test Coverage:**
- OpenAPI spec validation
- Example payload validation

---

#### 6.5 Rate Limiting & Throttling (Optional - 2-3 days)
**Objective:** Protect service from overload

**Steps:**
- Implement token bucket or leaky bucket rate limiting
- Add rate limit middleware to poem
- Configure limits per endpoint type:
  - Higher limits for single evaluations
  - Lower limits for batch/heatmap (more expensive)
- Return 429 (Too Many Requests) when limit exceeded
- Add rate limit headers to responses

**Acceptance Criteria:**
- Rate limits enforced per IP or API key
- Clear error messages when rate limited
- Configuration allows adjusting limits without code changes
- Legitimate usage patterns not impacted

**Files to Create:**
- `src/api/rate_limit.rs`
- Update middleware configuration

**Test Coverage:**
- Rate limit enforcement
- Limit reset after time window
- Different limits per endpoint

**Note:** This task is optional for MVP but recommended for production

---

### Sprint 6 Deliverables

- ✅ Batch evaluation endpoint with parallelization
- ✅ Heatmap generation endpoint
- ✅ Antenna listing and details endpoints
- ✅ Complete API documentation (OpenAPI spec)
- ✅ Optional: Rate limiting
- ✅ Full API test suite
- ✅ 80%+ test coverage for new code

---

## Sprint 7: Integration & Performance Testing

**Goal:** Comprehensive testing, optimization, and quality assurance

### Tasks

#### 7.1 End-to-End Integration Tests (4-5 days)
**Objective:** Test complete workflows from API to physical optics computation

**Steps:**
- Create `tests/integration/` test suite:
  - Full API request/response cycles
  - Multi-antenna scenarios
  - Concurrent request handling
  - Error recovery paths
- Generate realistic test calibration data:
  - 2-3 complete antenna models with physical parameters
  - Various antenna geometries (different f/D, sizes)
  - Various calibrated parameters (mesh, surface, feed)
  - Edge cases (large feed offsets, extreme frequencies)
- Test startup/shutdown sequences
- Test configuration variations
- **Validation against measured patterns:**
  - Compare computed patterns to actual measurements
  - Verify <1 dB accuracy in main lobe
  - Verify <1 dB accuracy in first sidelobe
  - Check coma lobe positions for offset feeds

**Acceptance Criteria:**
- Integration tests run against real server instance
- All API endpoints covered by integration tests
- Concurrent access patterns tested
- Tests use realistic physical antenna models
- **Model predictions match measurements within 1 dB**
- All tests pass consistently

**Files to Create:**
- `tests/integration/api_tests.rs`
- `tests/integration/concurrent_tests.rs`
- `tests/fixtures/` with realistic calibration artifacts
- `tests/integration/helpers.rs` (test utilities)

**Test Coverage:**
- Single evaluation workflow
- Batch evaluation workflow
- Heatmap generation workflow
- Error handling paths
- Concurrent multi-client scenarios
- Startup with various configurations

---

#### 7.2 Performance Benchmarking Suite (3-4 days)
**Objective:** Establish performance baseline and identify bottlenecks

**Steps:**
- Create comprehensive benchmark suite:
  - Single evaluation latency (p50, p95, p99)
  - Aperture integration convergence time
  - Pattern computation at various frequencies
  - Batch throughput (requests/second)
  - Heatmap generation time vs. grid size
  - Memory usage over time
  - Concurrent load testing
- Use `criterion` for statistical benchmarking
- Set up automated performance tracking
- Profile with `flamegraph` and `perf`
- Identify and optimize bottlenecks:
  - **Aperture integration** (likely hottest path)
  - Phase function calculations
  - Coordinate transformations
  - Feed pattern evaluations

**Acceptance Criteria:**
- Single evaluation p95 latency <100ms (physical optics computation)
- Batch throughput >10 req/s for small batches
- Heatmap generation meets performance targets
- Memory usage stable under load
- No performance regressions in CI
- **Aperture integration converges accurately within time budget**

**Files to Create:**
- `benches/api_benchmarks.rs`
- `benches/physics_engine_benchmarks.rs`
- `benches/aperture_integration_benchmarks.rs`
- `benches/load_test.rs`
- `docs/performance-results.md`

**Benchmarks:**
- Single evaluation: various antenna positions
- Batch: 10, 50, 100, 500 evaluations
- Heatmap: 10x10, 50x50, 100x100 grids
- Concurrent: 1, 5, 10, 20 simultaneous clients
- Memory: sustained load over 10 minutes

**Performance Targets (from architecture doc):**
- Single evaluation latency: 50-100ms (p95)
- Batch throughput: 1-20 req/s per instance
- Memory footprint: <512MB
- Startup time: <10s

---

#### 7.3 Error Handling & Resilience Testing (3-4 days)
**Objective:** Verify robust error handling and recovery

**Steps:**
- Test failure scenarios:
  - Missing calibration files
  - Corrupted calibration data
  - Invalid configuration
  - Out-of-memory conditions
  - Malformed API requests
  - Extreme parameter values
- Verify error messages are clear and actionable
- Test graceful degradation
- Verify no panics under any input
- Test recovery from transient failures

**Acceptance Criteria:**
- All error conditions produce clear error messages
- No panics or crashes under any input
- Partial failures (e.g., one antenna load fails) handled gracefully
- Service recovers from transient errors
- Error logs contain sufficient debugging information

**Files to Create:**
- `tests/integration/error_tests.rs`
- `tests/integration/resilience_tests.rs`

**Test Coverage:**
- Startup failures (missing files, invalid config)
- Runtime errors (invalid requests, out-of-range)
- Resource exhaustion (large requests, memory limits)
- Data corruption scenarios
- Concurrent error conditions

---

#### 7.4 Load Testing & Scalability Analysis (3-4 days)
**Objective:** Validate service behavior under production load

**Steps:**
- Set up load testing infrastructure:
  - Use `wrk`, `k6`, or similar load testing tool
  - Define realistic usage patterns
  - Simulate multiple client types (single, batch, heatmap)
- Run load tests at various levels:
  - Normal load (1-5 req/s)
  - Peak load (10-20 req/s)
  - Stress test (>20 req/s)
- Monitor resource usage during tests:
  - CPU utilization
  - Memory consumption
  - Response time distribution
  - Error rates
- Analyze results and document findings

**Acceptance Criteria:**
- Service handles target load (1-20 req/s) without degradation
- Response times remain within targets under normal load
- Graceful degradation under overload (no crashes)
- Resource usage documented for capacity planning
- Scalability characteristics understood

**Files to Create:**
- `tests/load/load_test_scenarios.js` (k6 scripts)
- `tests/load/README.md` (how to run load tests)
- `docs/scalability-analysis.md`

**Load Test Scenarios:**
- Sustained 10 req/s for 5 minutes (single evaluations)
- Burst to 20 req/s for 1 minute
- Mixed workload (70% single, 20% batch, 10% heatmap)
- Gradual ramp-up to failure point

---

#### 7.5 Code Quality & Documentation Review (2-3 days)
**Objective:** Ensure code quality and completeness

**Steps:**
- Run static analysis tools:
  - `cargo clippy` with strict lints
  - `cargo fmt` for formatting
  - `cargo audit` for security vulnerabilities
- Review and improve documentation:
  - Public API documentation (`cargo doc`)
  - Inline code comments for complex logic
  - Module-level documentation
  - README files for each component
- Conduct code review:
  - Check for code smells
  - Verify error handling patterns
  - Review test coverage gaps
  - Ensure consistency in coding style

**Acceptance Criteria:**
- Zero clippy warnings with strict lints
- All code formatted with `cargo fmt`
- No known security vulnerabilities
- Public APIs fully documented
- Code review feedback addressed
- Test coverage >80% overall

**Files to Create:**
- `.clippy.toml` (clippy configuration)
- Update documentation throughout codebase
- `docs/code-review-checklist.md`

---

### Sprint 7 Deliverables

- ✅ Comprehensive integration test suite
- ✅ Performance benchmark suite with baseline results
- ✅ Error handling and resilience tests
- ✅ Load testing infrastructure and results
- ✅ Code quality improvements and documentation
- ✅ Performance meeting all targets
- ✅ >85% test coverage overall

---

## Sprint 8: Deployment & Documentation

**Goal:** Production-ready deployment artifacts and operational documentation

### Tasks

#### 8.1 Docker Image Creation (3-4 days)
**Objective:** Build optimized Docker image for deployment

**Steps:**
- Create multi-stage `Dockerfile`:
  - Build stage: compile Rust binaries
  - Runtime stage: minimal base image (distroless or alpine)
  - Include calibration data in image
  - Set up proper permissions and user
- Optimize image size:
  - Strip debug symbols
  - Use release profile with LTO
  - Minimize runtime dependencies
- Add health check in Dockerfile
- Create `.dockerignore` file
- Build and test image locally

**Acceptance Criteria:**
- Docker image builds successfully
- Image size <100MB (excluding calibration data)
- Image runs with non-root user
- Health check works correctly
- Image tagged with version

**Files to Create:**
- `Dockerfile`
- `.dockerignore`
- `docker-compose.yml` (for local testing)
- `scripts/build-docker.sh`

**Dockerfile Structure:**
```dockerfile
# Build stage
FROM rust:1.75 as builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked

# Runtime stage
FROM debian:bookworm-slim
COPY --from=builder /build/target/release/antenna-model /app/
COPY calibration_data/ /app/calibration_data/
COPY config/ /app/config/
USER 1000
HEALTHCHECK CMD curl -f http://localhost:3000/health || exit 1
CMD ["/app/antenna-model"]
```

---

#### 8.2 Kubernetes Deployment Configuration (4-5 days)
**Objective:** Create complete Kubernetes deployment manifests

**Steps:**
- Create Kubernetes resources:
  - Deployment with replica configuration
  - Service (ClusterIP or LoadBalancer)
  - ConfigMap for service configuration
  - Resource limits and requests
  - Liveness and readiness probes
  - Pod disruption budget
- Create Helm chart (optional but recommended):
  - Chart.yaml with metadata
  - values.yaml with configurable parameters
  - Templates for all K8s resources
  - Support for multiple environments (dev, staging, prod)
- Add deployment documentation
- Test deployment in local Kubernetes (minikube or kind)

**Acceptance Criteria:**
- All K8s manifests are valid
- Deployment succeeds in local cluster
- Health probes work correctly
- Service is accessible within cluster
- Helm chart (if created) installs successfully
- Rolling updates work without downtime

**Files to Create:**
- `k8s/deployment.yaml`
- `k8s/service.yaml`
- `k8s/configmap.yaml`
- `k8s/pdb.yaml` (pod disruption budget)
- `helm/antenna-model/Chart.yaml`
- `helm/antenna-model/values.yaml`
- `helm/antenna-model/templates/` (various templates)
- `docs/kubernetes-deployment.md`

**Test in Local Cluster:**
```bash
# Using minikube or kind
kind create cluster
kubectl apply -f k8s/
kubectl get pods
kubectl logs <pod-name>
curl http://<service-ip>/health
```

---

#### 8.3 Operational Documentation (3-4 days)
**Objective:** Create comprehensive operational guides

**Steps:**
- Write operational runbooks:
  - Deployment procedures
  - Configuration management
  - Monitoring and alerting setup
  - Troubleshooting guide
  - Common issues and solutions
  - Rollback procedures
- Create calibration workflow documentation:
  - Step-by-step calibration process
  - Data preparation guidelines
  - Quality validation procedures
  - Artifact management
- Document logging and monitoring:
  - Log format and fields
  - Important log patterns
  - Metrics to monitor
  - Alert thresholds
- Create disaster recovery procedures

**Acceptance Criteria:**
- Runbooks cover all operational scenarios
- Documentation tested by someone unfamiliar with system
- Troubleshooting guide addresses common issues
- Calibration workflow can be followed independently
- All documentation reviewed and approved

**Files to Create:**
- `docs/operations/deployment-runbook.md`
- `docs/operations/troubleshooting-guide.md`
- `docs/operations/monitoring-and-alerting.md`
- `docs/operations/calibration-workflow.md`
- `docs/operations/disaster-recovery.md`
- `docs/operations/configuration-reference.md`

---

#### 8.4 Developer Documentation (2-3 days)
**Objective:** Enable future developers to contribute

**Steps:**
- Create developer guides:
  - Architecture overview
  - Code structure walkthrough
  - Development environment setup
  - Testing guide
  - Contribution guidelines
  - Code review checklist
- Update README.md with:
  - Project overview
  - Quick start guide
  - Build instructions
  - Testing instructions
  - Links to detailed docs
- Create API usage examples:
  - Client code examples (Python, JavaScript)
  - cURL examples for all endpoints
  - Common usage patterns
- Add architectural decision records (ADRs) for key decisions

**Acceptance Criteria:**
- New developer can set up environment from docs
- README provides clear project overview
- API examples work and cover common use cases
- ADRs document key design decisions
- Contributing guide is clear and comprehensive

**Files to Create:**
- Update `README.md`
- `docs/development/getting-started.md`
- `docs/development/architecture-overview.md`
- `docs/development/testing-guide.md`
- `docs/development/contributing.md`
- `docs/api-examples/` (examples in various languages)
- `docs/adr/` (architectural decision records)

---

#### 8.5 Release Preparation & Final Testing (2-3 days)
**Objective:** Prepare for production release

**Steps:**
- Final quality checks:
  - Run full test suite (unit, integration, load)
  - Verify all documentation is up to date
  - Check for any remaining TODOs or FIXMEs
  - Security scan of dependencies
  - License compliance check
- Create release artifacts:
  - Tag release version in git
  - Build and tag Docker image with version
  - Generate release notes
  - Create deployment checklist
- Conduct final deployment dry-run:
  - Deploy to staging environment
  - Run smoke tests
  - Verify monitoring and logging
  - Test rollback procedure
- Prepare rollout plan

**Acceptance Criteria:**
- All tests pass (100% success rate)
- Documentation complete and reviewed
- Release artifacts generated and tagged
- Successful deployment to staging
- Smoke tests pass in staging
- Rollback tested and verified
- Rollout plan approved

**Files to Create:**
- `CHANGELOG.md`
- `RELEASE_NOTES.md`
- `docs/deployment-checklist.md`
- `docs/rollout-plan.md`

**Final Checks:**
```bash
# Run all tests
cargo test --all --all-features
cargo bench

# Security audit
cargo audit

# Build release
cargo build --release

# Build Docker image
docker build -t antenna-model:v1.0.0 .

# Deploy to staging
helm upgrade --install antenna-model ./helm/antenna-model \
  --namespace staging --create-namespace
```

---

### Sprint 8 Deliverables

- ✅ Optimized Docker image
- ✅ Complete Kubernetes deployment manifests
- ✅ Operational runbooks and procedures
- ✅ Comprehensive developer documentation
- ✅ Release artifacts and deployment plan
- ✅ Successful staging deployment
- ✅ Production readiness review complete

---

## Post-MVP Roadmap

### Future Enhancements (Post-Sprint 8)

#### GPU Acceleration (2-3 sprints)
**Priority: High** - Physical optics integration is compute-intensive
- Design trait-based compute backend abstraction
- Implement CUDA or compute shader backend for aperture integration
- Parallelize phase calculations across GPU
- Benchmark performance improvements (target: 10-100x speedup)
- Add configuration for backend selection (CPU/GPU)

#### B-Spline Interpolation Cache (1-2 sprints)
**Priority: Medium** - Performance optimization for repeated queries
- Pre-compute patterns at grid points using physical model
- Fit B-splines to pre-computed patterns
- Use interpolation for fast lookup between grid points
- Fall back to physical model for accuracy-critical queries
- Hybrid mode: interpolation with physical model validation

#### gRPC API (1-2 sprints)
- Define protobuf schemas
- Implement gRPC server alongside REST
- Add streaming support for large batches
- Performance comparison and optimization

#### Advanced Monitoring (1 sprint)
- Prometheus metrics integration
- Grafana dashboards
- Custom alerting rules
- Distributed tracing with OpenTelemetry

#### Temperature Modeling (2-3 sprints)
- Add thermal expansion effects on surface RMS
- Temperature-dependent mesh properties
- Feed pattern changes with temperature
- Extend calibration to include temperature data

#### Uncertainty Quantification (2-3 sprints)
- Add confidence intervals to predictions
- Parameter uncertainty propagation from calibration
- Monte Carlo analysis with parameter distributions
- Update API to return uncertainty estimates
- Visualization of uncertainty regions

---

## Risk Management

### High-Priority Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Physical optics model accuracy insufficient | Medium | High | Early validation against measured patterns in Sprint 2-3; detailed comparison to literature |
| Aperture integration too slow (>100ms) | Medium | High | Early benchmarking in Sprint 2; adaptive quadrature optimization; GPU acceleration roadmap |
| Parameter optimization doesn't converge | Medium | High | Multiple optimizer algorithms; good initial guesses; synthetic test cases |
| Numerical instability (pattern nulls, edge cases) | Medium | Medium | Careful integration schemes; minimum noise floor; extensive edge case testing |
| Calibration complexity underestimated | Medium | Medium | Start with simple test cases; iterative refinement; differential evolution proven for this |
| Kubernetes deployment issues | Low | Medium | Test in local cluster early; staging environment validation |

### Mitigation Strategies

1. **Weekly Progress Reviews**: Check sprint progress weekly to identify blockers early
2. **Technical Spike Time**: Allocate 10% of each sprint for technical investigation
3. **Continuous Integration**: Ensure CI runs on every commit to catch regressions
4. **Pair Programming**: Complex tasks (interpolation, fitting) benefit from collaboration
5. **Incremental Delivery**: Each sprint produces working, tested code

---

## Success Criteria

### MVP Acceptance Criteria

The project is considered successfully completed when:

1. **Functional Requirements**
   - ✅ REST API with all specified endpoints operational
   - ✅ **Physical optics computation engine** (aperture integration, phase functions, Ruze, mesh effects)
   - ✅ **Coma lobe modeling** for off-axis feed positions
   - ✅ Calibration CLI tool with **parameter optimization** (differential evolution)
   - ✅ Support for multiple antenna configurations
   - ✅ **Model accuracy within 1 dB** for main lobe and first sidelobe (validated against measurements)
   - ✅ Proper warning generation for extrapolated queries

2. **Performance Requirements**
   - ✅ Single evaluation p95 latency <100ms (physical optics computation)
   - ✅ Batch throughput >10 req/s per instance
   - ✅ Startup time <10s
   - ✅ Memory footprint <512MB

3. **Quality Requirements**
   - ✅ >85% test coverage overall
   - ✅ **Physics model validated against measured antenna patterns**
   - ✅ Zero critical bugs in production
   - ✅ All documentation complete and reviewed
   - ✅ Successful deployment to production environment

4. **Operational Requirements**
   - ✅ Kubernetes deployment with health probes
   - ✅ Structured logging for all requests
   - ✅ Operational runbooks complete
   - ✅ On-call team trained on troubleshooting

5. **Scientific Validation Requirements** (New)
   - ✅ **Ruze efficiency** matches published values for various surface RMS
   - ✅ **Zernike polynomials** correctly model aberrations
   - ✅ **Mesh transparency** model validated across frequency range (100 MHz - 50 GHz)
   - ✅ **Coma lobes** appear at correct angular positions for feed displacement
   - ✅ Edge case handling (large feed offsets, near-boresight scenarios)

---

## Sprint Planning Guidelines

### Sprint Ceremonies

**Sprint Planning (First day of sprint)**
- Review sprint goals and deliverables
- Break down tasks into daily work items
- Identify dependencies and blockers
- Assign tasks based on engineer strengths

**Daily Standups (15 minutes)**
- What was completed yesterday
- What will be worked on today
- Any blockers or concerns

**Sprint Review (Last day of sprint)**
- Demo completed functionality
- Review test results and coverage
- Gather feedback from stakeholders

**Sprint Retrospective (Last day of sprint)**
- What went well
- What could be improved
- Action items for next sprint

### Recommended Work Distribution

For a mid-level engineer working 40 hours/week:

- **Development**: 60-70% (24-28 hours)
- **Testing**: 15-20% (6-8 hours)
- **Documentation**: 10-15% (4-6 hours)
- **Code Review/Planning**: 5-10% (2-4 hours)

### Buffer Time

Each sprint includes ~15-20% buffer time for:
- Unexpected complexity
- Bug fixes
- Technical debt
- Learning new libraries/concepts

---

## Dependencies & Prerequisites

### Before Sprint 1

- Rust development environment set up (rustc, cargo)
- Git repository created and accessible
- CI/CD platform configured (GitHub Actions, GitLab CI, etc.)
- Access to documentation and design specs
- Development workstation with adequate resources

### Before Sprint 2

- **Access to antenna physics references** (see Appendix A)
- Understanding of physical optics and electromagnetic theory
- Familiarity with numerical integration techniques

### Before Sprint 4

- Sample measurement data (CSV files) with G/T measurements for testing calibration tool
- Understanding of optimization algorithms (differential evolution)
- Access to reference antenna patterns for validation

### Before Sprint 8

- Kubernetes cluster access (local or cloud)
- Docker registry for image storage
- Staging environment for deployment testing

---

## Appendices

### Appendix A: Recommended Reading

**Antenna Physics & Physical Optics:**
- **"Antenna Theory: Analysis and Design" by Balanis** - Chapters on reflector antennas and physical optics
- **Ruze, J. "Antenna Tolerance Theory"** (1966) - Classic paper on surface error effects
- **"Reflector Antennas" by Love (ed.)** - IEEE Press, comprehensive reflector antenna theory
- Silver, S. "Microwave Antenna Theory and Design" - Radiation Laboratory Series
- Design doc: `docs/antenna-model-design-doc.md` - Mathematical formulations specific to this project

**Numerical Methods:**
- **Numerical integration techniques** - Gaussian quadrature, adaptive Simpson's rule
- **Optimization algorithms** - Differential evolution, Nelder-Mead, gradient-free methods
- Press et al. "Numerical Recipes" - Chapters on integration and optimization

**Zernike Polynomials & Aberrations:**
- **Noll, R.J. "Zernike Polynomials and Atmospheric Turbulence"** - Standard reference for Zernike ordering
- Born & Wolf "Principles of Optics" - Aberration theory
- Hopkins, H.H. "Wave Theory of Aberrations" - Coma and other Seidel aberrations

**Mesh Reflectors:**
- **Wire mesh antenna literature** - Frequency-dependent transparency
- EM scattering theory for periodic structures

**Rust Web Development:**
- Poem framework documentation
- Tokio async runtime guide
- Rust API guidelines

**Kubernetes:**
- Kubernetes documentation - Deployments, Services, ConfigMaps
- Helm documentation (if using Helm)

**Optional (for future enhancements):**
- "A Practical Guide to Splines" by Carl de Boor - If implementing B-spline interpolation cache

### Appendix B: Useful Tools

**Development:**
- `cargo-watch` - auto-rebuild on file changes
- `cargo-expand` - expand macros for debugging
- `cargo-edit` - manage dependencies from CLI

**Testing:**
- `cargo-tarpaulin` - code coverage
- `criterion` - benchmarking
- `proptest` - property-based testing

**Performance:**
- `flamegraph` - CPU profiling
- `heaptrack` - memory profiling
- `valgrind` - memory debugging

**API Testing:**
- `curl` - command-line HTTP client
- `httpie` - user-friendly HTTP client
- `k6` - load testing
- Postman or Insomnia - interactive API testing

### Appendix C: Code Review Checklist

- [ ] Code follows Rust idioms and best practices
- [ ] All public APIs have documentation comments
- [ ] Error handling is comprehensive
- [ ] Tests cover happy path and error cases
- [ ] No unwrap() or expect() in production code (use proper error handling)
- [ ] Performance-critical code is benchmarked
- [ ] Logging uses structured fields
- [ ] Security considerations addressed (input validation, etc.)
- [ ] Breaking changes documented
- [ ] Backward compatibility maintained (if applicable)

### Appendix D: Troubleshooting Common Issues

**Compilation Issues:**
- Clear target directory: `cargo clean`
- Update dependencies: `cargo update`
- Check Rust version: `rustc --version`

**Test Failures:**
- Run tests with output: `cargo test -- --nocapture`
- Run specific test: `cargo test test_name`
- Check test fixtures are up to date

**Performance Issues:**
- Profile with flamegraph: `cargo flamegraph`
- Check for unnecessary allocations
- Review algorithm complexity
- Consider parallelization opportunities

**Docker Build Issues:**
- Clear Docker cache: `docker build --no-cache`
- Check .dockerignore file
- Verify file paths in COPY commands

---

**End of Implementation Plan**

This plan provides a roadmap for implementing the Antenna Model Service over 16 weeks (8 two-week sprints). Each sprint contains well-scoped tasks appropriate for a mid-level engineer, with clear acceptance criteria, deliverables, and success metrics. The plan balances feature development with testing, documentation, and operational readiness to ensure a production-quality system at the end of Sprint 8.
