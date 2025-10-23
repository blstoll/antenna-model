# Antenna Model Service - Implementation Plan

## Document Overview

**Version:** 1.0
**Created:** 2025-10-22
**Target Timeline:** 8 sprints (16 weeks)
**Scope:** MVP with full REST API, calibration tool, and Kubernetes deployment

This implementation plan breaks down the Antenna Model Service into manageable sprints, each containing tasks scoped for a mid-level engineer to complete within a 2-week period.

---

## Sprint Overview

| Sprint | Focus Area | Duration | Key Deliverables |
|--------|-----------|----------|-----------------|
| Sprint 1 | Project Foundation & Core Data Types | 2 weeks | Repository structure, basic REST API with /status endpoint, core data types, basic tests |
| Sprint 2 | B-Spline Interpolation Engine | 2 weeks | 4D interpolation, extrapolation, unit tests |
| Sprint 3 | Calibration Data Management | 2 weeks | Data loader, repository, configuration system |
| Sprint 4 | Calibration CLI Tool | 2 weeks | CSV parser, B-spline fitter, artifact serializer |
| Sprint 5 | REST API - Core Endpoints | 2 weeks | Production middleware, enhanced health checks, single evaluation endpoints |
| Sprint 6 | REST API - Advanced Endpoints | 2 weeks | Batch processing, heatmap generation |
| Sprint 7 | Integration & Performance Testing | 2 weeks | End-to-end tests, performance benchmarks |
| Sprint 8 | Deployment & Documentation | 2 weeks | Docker, Kubernetes, operational docs |

---

## Sprint 1: Project Foundation & Core Data Types

**Goal:** Establish project structure, dependencies, and foundational data types

### Tasks

#### 1.1 Repository & Build Setup (3-4 days) ✅ COMPLETE
**Objective:** Initialize Rust project with proper workspace structure

**Steps:**
- ✅ Create Cargo workspace with two crates: `antenna-model` (main service) and `calibrate` (CLI tool)
- ✅ Set up `Cargo.toml` with dependencies from architecture doc section 10.1
- ✅ Configure Rust edition 2024 and basic compiler settings
- ✅ Add `.gitignore` for Rust projects
- ✅ Create basic directory structure matching architecture doc section 11

**Acceptance Criteria:**
- ✅ `cargo build` succeeds for both crates
- ✅ CI pipeline runs successfully on commit
- ✅ Directory structure matches architecture specification

**Files Created:**
- ✅ `Cargo.toml` (workspace)
- ✅ `antenna-model/Cargo.toml`
- ✅ `calibrate/Cargo.toml`
- ✅ `.gitignore`

---

#### 1.2 Basic REST API & Status Endpoint (2-3 days)
**Objective:** Set up minimal REST API server with status endpoint for health checks

**Steps:**
- Create basic `poem` web server in `src/api/mod.rs` and `src/main.rs`:
  - Initialize tokio runtime
  - Create simple server with graceful shutdown
  - Configure port from environment or default to 3000
- Implement `GET /status` endpoint in `src/api/handlers.rs`:
  - Return application version (from Cargo.toml)
  - Return uptime since server start
  - Return simple "ok" status
  - HTTP 200 response
- Add basic request logging with `tracing`
- Test server startup and endpoint functionality

**Acceptance Criteria:**
- Server starts and responds on configured port
- `/status` endpoint returns JSON with version, uptime, and status
- Suitable for Kubernetes liveness/readiness probes
- Graceful shutdown on SIGTERM/SIGINT
- Basic structured logging for requests

**Files to Create:**
- `src/api/mod.rs`
- `src/api/handlers.rs`
- Update `src/main.rs` with server initialization

**Test Coverage:**
- Server startup
- Status endpoint response format
- Uptime calculation
- Graceful shutdown

**Example Response:**
```json
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_seconds": 3600
}
```

---

#### 1.3 Core Data Types Implementation (4-5 days)
**Objective:** Implement foundational data structures for calibration and antenna models

**Steps:**
- Create `src/data/types.rs` with core structures:
  - `AntennaCalibration` - holds antenna metadata and model
  - `BSplineModel4D` - stores coefficients, knots, and shape
  - `ValidityRanges` - min/max ranges for each dimension
  - `CalibrationMetadata` - antenna name, calibration date, etc.
- Implement `serde` serialization/deserialization for all types
- Add builder patterns for ergonomic construction
- Write unit tests for serialization round-trips

**Acceptance Criteria:**
- All data structures compile with proper serialization attributes
- Unit tests verify serialization/deserialization with `bincode`
- Builder patterns allow easy construction of test fixtures
- Documentation comments on all public types

**Files to Create:**
- `src/data/types.rs`
- `src/data/mod.rs`
- `src/lib.rs` (expose data module)

**Test Coverage:**
- Serialization round-trip tests
- Builder pattern tests
- Validation of field constraints

---

#### 1.4 Configuration System (3-4 days)
**Objective:** Implement configuration loading for service and antenna management

**Steps:**
- Create `src/config/settings.rs` with service configuration:
  - Server port, host binding
  - Calibration data directory path
  - Logging configuration
  - Performance tuning parameters
- Implement TOML-based antenna configuration loading
- Add environment variable override support
- Write tests for configuration parsing

**Acceptance Criteria:**
- Service configuration loads from `config/service.toml`
- Antenna configuration loads from `calibration_data/antennas.toml`
- Environment variables override file-based config
- Clear error messages for malformed configuration

**Files to Create:**
- `src/config/settings.rs`
- `src/config/mod.rs`
- `config/service.toml` (example)
- `calibration_data/antennas.toml` (example)

**Test Coverage:**
- Valid configuration parsing
- Invalid configuration error handling
- Environment variable override tests

---

#### 1.5 Error Handling Framework (2-3 days)
**Objective:** Define error types and handling strategy

**Steps:**
- Create custom error types using `thiserror`:
  - `DataError` - calibration data issues
  - `ApiError` - HTTP/API errors
  - `ValidationError` - input validation failures
  - `ComputationError` - interpolation/math errors
- Implement `From` conversions for common error types
- Add error context helpers
- Write error formatting tests

**Acceptance Criteria:**
- All error types implement proper `Display` and `Debug`
- Error chains preserve context information
- Conversion traits allow ergonomic error propagation
- Tests verify error message formatting

**Files to Create:**
- `src/error.rs`
- `src/lib.rs` (export error module)

**Test Coverage:**
- Error creation and formatting
- Error chain preservation
- Conversion trait tests

---

### Sprint 1 Deliverables

- ✅ Working Rust workspace with two crates
- ✅ Basic REST API server with status endpoint for health checks
- ✅ Core data structures with serialization
- ✅ Configuration system with TOML support
- ✅ Error handling framework
- ✅ Basic CI pipeline
- ✅ 80%+ test coverage for implemented code

---

## Sprint 2: B-Spline Interpolation Engine

**Goal:** Implement the core computation engine for 4D B-spline interpolation

### Tasks

#### 2.1 1D B-Spline Primitives (4-5 days)
**Objective:** Implement fundamental B-spline basis function evaluation

**Steps:**
- Create `src/model/bspline.rs` with:
  - `find_knot_interval()` - binary search for knot span
  - `basis_functions()` - Cox-de Boor recursive algorithm
  - `basis_derivatives()` - derivative computation (for future use)
- Implement efficient caching for repeated evaluations
- Add comprehensive unit tests against known B-spline values
- Benchmark performance and optimize hot paths

**Acceptance Criteria:**
- Basis function evaluation matches reference implementations
- Binary search completes in O(log n) time
- Unit tests cover edge cases (boundaries, repeated knots)
- Performance benchmarks show <1μs per evaluation

**Files to Create:**
- `src/model/bspline.rs`
- `src/model/mod.rs`
- `benches/bspline_bench.rs`

**Test Coverage:**
- Known B-spline values (compare to published tables)
- Boundary conditions
- Repeated knot handling
- Performance benchmarks

**Reference:**
- "A Practical Guide to Splines" by de Boor
- Verify against scipy.interpolate results

---

#### 2.2 4D Tensor Interpolation (5-6 days)
**Objective:** Extend 1D B-splines to 4D tensor interpolation

**Steps:**
- Create `src/model/interpolation.rs` with:
  - `evaluate_4d()` - main interpolation entry point
  - `tensor_product()` - combines 1D basis functions
  - `extract_local_coefficients()` - retrieves relevant coefficient subset
- Implement dimension ordering (azimuth, elevation, frequency, temperature)
- Optimize memory access patterns for cache efficiency
- Add interpolation accuracy tests

**Acceptance Criteria:**
- 4D interpolation produces continuous, smooth results
- Accuracy within floating-point precision for synthetic data
- Memory access patterns minimize cache misses
- Tests verify C2 continuity (for cubic splines)

**Files to Create:**
- `src/model/interpolation.rs`
- `tests/integration/interpolation_tests.rs`

**Test Coverage:**
- Synthetic 4D function interpolation (polynomial, trigonometric)
- Boundary point evaluation
- Continuity verification
- Derivative continuity (if implemented)

**Test Data:**
- Create synthetic 4D datasets with known analytical forms
- Example: `f(az, el, freq, temp) = sin(az) * cos(el) * log(freq)`

---

#### 2.3 Extrapolation Handling (3-4 days)
**Objective:** Implement safe out-of-range query handling

**Steps:**
- Create `src/model/extrapolation.rs` with:
  - `ExtrapolationStrategy` enum (Linear, Constant, Nearest)
  - `check_bounds()` - determine if query is in-range
  - `extrapolate_4d()` - apply strategy per dimension
  - Warning generation for out-of-range queries
- Implement conservative default strategy (nearest neighbor)
- Add configuration for extrapolation behavior
- Test extrapolation accuracy and stability

**Acceptance Criteria:**
- Out-of-range queries return valid results (no panics)
- Warnings correctly identify which dimensions are extrapolated
- Extrapolated values are physically reasonable
- Tests cover all dimension combinations

**Files to Create:**
- `src/model/extrapolation.rs`
- Update `src/model/interpolation.rs` to integrate extrapolation

**Test Coverage:**
- Each dimension out-of-range individually
- Multiple dimensions out-of-range simultaneously
- Extrapolation strategy variations
- Warning message generation

---

#### 2.4 Performance Optimization (2-3 days)
**Objective:** Optimize interpolation to meet <1ms evaluation target

**Steps:**
- Profile interpolation code with `cargo flamegraph`
- Optimize hot paths identified in profiling:
  - Pre-compute knot interval searches where possible
  - Use SIMD for basis function evaluation (if beneficial)
  - Optimize coefficient indexing and memory layout
- Add performance benchmarks for various model sizes
- Document performance characteristics

**Acceptance Criteria:**
- Single evaluation completes in <1ms (p95) for typical model
- Benchmark suite tracks performance across model sizes
- No performance regressions in CI
- Documentation explains performance trade-offs

**Files to Create:**
- `benches/interpolation_bench.rs`
- `docs/performance-characteristics.md`

**Benchmarks:**
- Various knot grid sizes (10x10x10x1, 50x50x20x1, etc.)
- Different spline orders (linear, cubic)
- Memory usage tracking

---

### Sprint 2 Deliverables

- ✅ Working 4D B-spline interpolation engine
- ✅ Extrapolation handling with warnings
- ✅ Performance meeting <1ms evaluation target
- ✅ Comprehensive unit and integration tests
- ✅ Performance benchmark suite
- ✅ 85%+ test coverage

---

## Sprint 3: Calibration Data Management

**Goal:** Implement data loading, repository, and in-memory management

### Tasks

#### 3.1 Binary Artifact Serialization (3-4 days)
**Objective:** Implement efficient binary format for calibration data

**Steps:**
- Create `src/data/serializer.rs` with:
  - `serialize_calibration()` - write calibration to binary
  - `deserialize_calibration()` - read from binary
  - Checksum/CRC validation for data integrity
  - Version header for format evolution
- Choose serialization format (bincode recommended for performance)
- Add compression option for larger datasets
- Test serialization round-trips and compatibility

**Acceptance Criteria:**
- Serialization preserves all calibration data accurately
- Checksums detect corrupted files
- Version header allows format migration
- Compressed files reduce size by >50% (if enabled)

**Files to Create:**
- `src/data/serializer.rs`
- Update `src/data/types.rs` with serialization metadata

**Test Coverage:**
- Round-trip serialization
- Checksum validation (both valid and corrupted data)
- Version compatibility
- Compression effectiveness

---

#### 3.2 Calibration Data Loader (4-5 days)
**Objective:** Load calibration artifacts from filesystem

**Steps:**
- Create `src/data/loader.rs` with:
  - `load_calibration()` - load single calibration file
  - `load_all_calibrations()` - load from directory
  - Parallel loading for multiple files (using `rayon`)
  - Detailed error reporting for load failures
- Integrate with configuration system for paths
- Add validation checks on loaded data
- Implement fail-fast strategy for corrupted data

**Acceptance Criteria:**
- Successfully loads valid calibration files
- Clear error messages for missing/corrupted files
- Parallel loading speeds up multi-antenna scenarios
- Validates data structure integrity after loading

**Files to Create:**
- `src/data/loader.rs`
- `tests/fixtures/` directory with sample calibration files

**Test Coverage:**
- Valid file loading
- Missing file error handling
- Corrupted file detection
- Parallel loading performance
- Fixture data for testing

**Test Fixtures:**
- Create 2-3 minimal but valid calibration files
- One intentionally corrupted file for error testing

---

#### 3.3 Calibration Repository (4-5 days)
**Objective:** Implement in-memory repository for antenna models

**Steps:**
- Create `src/data/repository.rs` with:
  - `CalibrationRepository` struct with `HashMap<String, AntennaCalibration>`
  - `load_from_config()` - initialize from configuration
  - `get_calibration()` - retrieve by antenna ID
  - `list_antennas()` - return all loaded antenna IDs
  - `get_validity_ranges()` - query bounds for an antenna
- Implement thread-safe access (using `Arc` and `RwLock` if needed)
- Add startup validation and logging
- Create repository builder for testing

**Acceptance Criteria:**
- Repository loads all configured antennas at startup
- Thread-safe concurrent access
- Clear logging of loaded antennas
- Tests use builder pattern for easy fixture creation

**Files to Create:**
- `src/data/repository.rs`
- Update `src/data/mod.rs` to export repository

**Test Coverage:**
- Loading multiple antennas
- Antenna ID lookup (found and not found)
- Concurrent access patterns
- Builder pattern for test fixtures

---

#### 3.4 Startup Sequence & Validation (2-3 days)
**Objective:** Implement robust startup with data loading and validation

**Steps:**
- Create initialization sequence:
  - Load service configuration
  - Load antenna configuration
  - Load all calibration artifacts
  - Validate loaded data
  - Log startup summary
- Add structured logging for each startup phase
- Implement graceful failure for missing/invalid data
- Add startup time tracking

**Acceptance Criteria:**
- Startup completes in <10s for 5 antennas
- Each startup phase is clearly logged
- Validation catches common data issues
- Service fails fast with clear error messages if data is invalid

**Files to Create:**
- Update `src/main.rs` with startup logic
- Create `src/startup.rs` helper module

**Test Coverage:**
- Successful startup sequence
- Missing calibration file handling
- Invalid calibration data detection
- Startup time benchmarks

---

### Sprint 3 Deliverables

- ✅ Binary serialization with checksums
- ✅ Calibration data loader with parallel support
- ✅ Thread-safe calibration repository
- ✅ Robust startup sequence with validation
- ✅ Test fixtures for integration testing
- ✅ 80%+ test coverage

---

## Sprint 4: Calibration CLI Tool

**Goal:** Build command-line tool to generate calibration artifacts from measurement data

### Tasks

#### 4.1 CLI Framework & Argument Parsing (2-3 days)
**Objective:** Set up CLI structure and command-line interface

**Steps:**
- Create `calibrate/src/main.rs` with `clap` argument parsing:
  - `--input <path>` - measurement CSV file or S3 URL
  - `--output <path>` - output binary file path
  - `--antenna-id <id>` - antenna identifier
  - `--validate` - run validation after fitting
  - `--verbose` - detailed logging
- Implement help text and usage examples
- Add version information
- Set up logging with `tracing`

**Acceptance Criteria:**
- `calibrate --help` shows clear usage information
- All arguments parse correctly
- Validation flags work as expected
- Version information displays correctly

**Files to Create:**
- `calibrate/src/main.rs`
- `calibrate/Cargo.toml` (update with dependencies)
- `calibrate/README.md`

**Test Coverage:**
- Argument parsing tests (using clap's built-in testing)
- Help text verification

---

#### 4.2 CSV Measurement Parser (3-4 days)
**Objective:** Parse and validate measurement CSV files

**Steps:**
- Create `calibrate/src/parser.rs` with:
  - `parse_measurements()` - read CSV into structured data
  - `MeasurementPoint` struct (azimuth, elevation, frequency, temperature, g_over_t)
  - Input validation (range checks, missing data handling)
  - Statistics computation (data coverage, density)
- Support both local files and S3 URLs (using `aws-sdk-s3`)
- Add data quality checks:
  - Check for required column headers
  - Validate numeric ranges
  - Identify gaps in coverage
- Generate parsing report

**Acceptance Criteria:**
- Successfully parses valid CSV files
- Clear error messages for malformed CSV
- Data quality report identifies coverage gaps
- S3 download works (if AWS credentials available)

**Files to Create:**
- `calibrate/src/parser.rs`
- `calibrate/tests/fixtures/sample_measurements.csv`

**Test Coverage:**
- Valid CSV parsing
- Missing columns detection
- Invalid numeric values
- Coverage statistics
- Sample fixture data

**CSV Format Example:**
```csv
azimuth_deg,elevation_deg,frequency_mhz,temperature_k,g_over_t_db
0.0,45.0,8200.0,290.0,41.5
5.0,45.0,8200.0,290.0,41.3
...
```

---

#### 4.3 B-Spline Fitting Engine (5-6 days)
**Objective:** Fit B-spline models to measurement data

**Steps:**
- Create `calibrate/src/fitter.rs` with:
  - `fit_bspline_4d()` - main fitting function
  - Knot placement strategy (uniform or data-adaptive)
  - Least-squares coefficient solver (using `ndarray-linalg`)
  - Residual analysis and quality metrics
- Implement fitting algorithm:
  - Generate basis function matrix
  - Set up normal equations
  - Solve for coefficients
  - Compute fit quality (RMSE, R²)
- Add progress reporting for large datasets
- Implement automatic knot selection based on data density

**Acceptance Criteria:**
- Fitting produces smooth interpolants
- RMSE and R² metrics are computed correctly
- Automatic knot selection works for sparse/dense data
- Progress updates for datasets >1000 points

**Files to Create:**
- `calibrate/src/fitter.rs`
- Add `ndarray-linalg` dependency

**Test Coverage:**
- Synthetic data fitting (known functions)
- Fit quality metrics verification
- Knot placement strategies
- Performance on various dataset sizes

**References:**
- Implement least-squares B-spline fitting as per de Boor
- Consider using existing crates like `ndarray-linalg` for matrix operations

---

#### 4.4 Validation & Artifact Generation (3-4 days)
**Objective:** Validate fitted models and generate calibration artifacts

**Steps:**
- Create `calibrate/src/validator.rs` with:
  - Cross-validation (k-fold or leave-one-out)
  - Error distribution analysis
  - Extrapolation behavior checks
  - Comparison to measurement data
- Update `calibrate/src/serializer.rs`:
  - Write fitted model to binary artifact
  - Include metadata (fit date, quality metrics, data source)
  - Generate human-readable summary report
- Create detailed validation report output

**Acceptance Criteria:**
- Validation identifies overfitting or poor fits
- Cross-validation RMSE within 1 dB (per requirements)
- Binary artifacts load correctly in main service
- Summary report is human-readable

**Files to Create:**
- `calibrate/src/validator.rs`
- `calibrate/src/serializer.rs`
- `calibrate/src/report.rs` (for generating summary)

**Test Coverage:**
- Validation metrics computation
- Cross-validation implementation
- Artifact serialization
- Report generation

---

### Sprint 4 Deliverables

- ✅ Working calibration CLI tool
- ✅ CSV parsing with data quality checks
- ✅ B-spline fitting with automatic knot selection
- ✅ Validation suite with quality metrics
- ✅ Binary artifact generation
- ✅ Sample calibration artifacts for testing
- ✅ 75%+ test coverage

---

## Sprint 5: REST API - Core Endpoints

**Goal:** Enhance REST API with production middleware, comprehensive health checks, and core evaluation endpoints

**Note:** Basic REST API server and status endpoint were established in Sprint 1. This sprint focuses on production-grade enhancements and evaluation functionality.

### Tasks

#### 5.1 API Server Enhancement & Middleware (3-4 days)
**Objective:** Enhance existing API server with production-grade middleware

**Note:** Basic API server and status endpoint were established in Sprint 1, Task 1.2. This task builds upon that foundation.

**Steps:**
- Enhance `src/api/mod.rs` with production features:
  - Integrate with configuration system for advanced settings
  - Add connection pooling and resource management
  - Implement proper state management
- Implement middleware in `src/api/middleware.rs`:
  - Request ID generation and propagation
  - Comprehensive structured logging (using `tracing`)
  - Request/response timing and metrics
  - Error handling middleware
  - CORS support (if needed)
- Enhance startup and shutdown sequences with detailed logging
- Add request/response size tracking

**Acceptance Criteria:**
- Server uses configuration from settings file
- All requests get unique request IDs in logs
- Request/response logs are structured JSON with timing
- Middleware chain executes in correct order
- Error responses are consistently formatted

**Files to Update/Create:**
- Update `src/api/mod.rs` with enhanced features
- Create `src/api/middleware.rs`
- Update `src/main.rs` with middleware integration

**Test Coverage:**
- Middleware execution order
- Request ID generation and propagation
- Timing measurement accuracy
- Error middleware handling

---

#### 5.2 Request/Response Schemas (3-4 days)
**Objective:** Define API contract with typed schemas

**Steps:**
- Create `src/api/schemas.rs` with:
  - `EvaluationRequest` - single evaluation input
  - `EvaluationResponse` - single evaluation output
  - `ErrorResponse` - standardized error format
  - `HealthResponse` - health check response
  - `StatusResponse` - service status
  - `AntennaInfo` - antenna metadata
- Implement `serde` serialization with proper field naming (snake_case)
- Add JSON schema annotations (using `poem-openapi` if desired)
- Write schema documentation

**Acceptance Criteria:**
- All schemas serialize/deserialize correctly
- JSON field names match API spec (section 4.3 of architecture doc)
- Schema documentation is clear
- Example JSON payloads are valid

**Files to Create:**
- `src/api/schemas.rs`
- `examples/api_requests.json` (example payloads)

**Test Coverage:**
- Serialization/deserialization round-trips
- Field naming conventions
- Validation edge cases

**Example Schema:**
```rust
#[derive(Serialize, Deserialize)]
pub struct EvaluationRequest {
    pub antenna_id: String,
    pub azimuth_deg: f64,
    pub elevation_deg: f64,
    pub frequency_mhz: f64,
}
```

---

#### 5.3 Enhanced Health & Status Endpoints (2-3 days)
**Objective:** Enhance operational endpoints with comprehensive service information

**Note:** Basic `/status` endpoint was created in Sprint 1, Task 1.2. This task expands it with calibration-aware health checks.

**Steps:**
- Enhance `src/api/handlers.rs` with:
  - `GET /health` - readiness/liveness probe (returns 200 if ready)
  - Enhance `GET /status` - add detailed service status
- Enhanced status endpoint returns:
  - Server uptime (already implemented)
  - Build version/commit (already implemented)
  - Loaded antenna count and IDs (new)
  - Memory usage (new, if available)
  - Calibration data status (new)
- Add readiness check logic:
  - Verify calibration data loaded successfully
  - Verify interpolation engine initialized
- Implement separate liveness check (verify service is responsive)
- Differentiate between `/health` (liveness) and `/ready` (readiness) if needed

**Acceptance Criteria:**
- `/health` returns 200 when service is responsive (liveness)
- `/health` or `/ready` returns 503 during startup or if data fails to load (readiness)
- `/status` returns comprehensive service information including antenna count
- Endpoints respond in <10ms

**Files to Update/Create:**
- Update `src/api/handlers.rs` with enhanced endpoints
- Create `src/api/routes.rs` (route definitions)

**Test Coverage:**
- Health check during startup, running, and shutdown states
- Readiness check with and without calibration data
- Enhanced status endpoint data accuracy
- Response time benchmarks

---

#### 5.4 Single Evaluation Endpoint (4-5 days)
**Objective:** Implement core antenna evaluation endpoint

**Steps:**
- Create service layer in `src/service/evaluator.rs`:
  - `evaluate_antenna()` - orchestrate single evaluation
  - Input validation against antenna validity ranges
  - Call interpolation engine
  - Generate warnings for out-of-range queries
  - Track computation time
- Add handler in `src/api/handlers.rs`:
  - `POST /api/v1/evaluate`
  - Request validation and parsing
  - Error handling and response formatting
  - Logging with structured fields
- Integrate with calibration repository
- Implement detailed error responses

**Acceptance Criteria:**
- Endpoint returns correct G/T values for in-range queries
- Out-of-range queries include appropriate warnings
- Error responses follow standard format
- Response time <100ms (p95) for typical queries
- Comprehensive logging for debugging

**Files to Create:**
- `src/service/evaluator.rs`
- `src/service/mod.rs`
- Update `src/api/handlers.rs` and `src/api/routes.rs`

**Test Coverage:**
- Valid evaluation requests
- Antenna not found errors
- Out-of-range parameter warnings
- Invalid parameter errors
- Response format validation
- Integration tests with real calibration data

---

#### 5.5 Input Validation Layer (2-3 days)
**Objective:** Implement comprehensive input validation

**Steps:**
- Create `src/service/validator.rs` with:
  - `validate_evaluation_request()` - check all parameters
  - Range validation (azimuth 0-360, elevation 0-90, etc.)
  - Antenna ID validation (exists in repository)
  - Frequency range validation
  - Generate specific error messages per field
- Add validation to all API handlers
- Implement custom validation error types

**Acceptance Criteria:**
- All invalid inputs are caught before processing
- Error messages specify which field failed and why
- Validation logic is reusable across endpoints
- Tests cover all validation rules

**Files to Create:**
- `src/service/validator.rs`
- Update error types to include validation errors

**Test Coverage:**
- Each validation rule individually
- Multiple validation failures
- Edge cases (boundary values, special characters)

---

### Sprint 5 Deliverables

- ✅ Production-grade REST API with middleware (built on Sprint 1 foundation)
- ✅ Enhanced health and status endpoints for K8s probes
- ✅ Single evaluation endpoint with full validation
- ✅ Comprehensive error handling and response formatting
- ✅ Advanced structured logging with request IDs and timing
- ✅ Integration tests with calibration data
- ✅ 80%+ test coverage

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

#### 6.2 Heatmap Generation Endpoint (5-6 days)
**Objective:** Generate 2D heatmaps of G/T across azimuth/elevation

**Steps:**
- Create heatmap generation logic in `src/service/evaluator.rs`:
  - `generate_heatmap()` - evaluate grid of points
  - Grid generation from range specifications
  - Efficient parallel evaluation
  - Handle extrapolation warnings for grid points
- Add handler for `POST /api/v1/heatmap`
- Implement response optimization:
  - Optional data compression
  - Configurable output resolution
  - Streaming response for large grids (future enhancement)
- Add heatmap-specific validation (reasonable grid sizes)

**Acceptance Criteria:**
- Heatmaps generated for specified azimuth/elevation ranges
- Grid spacing configurable via API
- Performance acceptable for typical grids (72x46 = 3312 points)
- Warnings aggregated for out-of-range regions
- Response size reasonable (<1MB for typical heatmaps)

**Files to Create:**
- Update `src/service/evaluator.rs` with heatmap logic
- Update `src/api/schemas.rs` with heatmap request/response types
- Add heatmap handler to `src/api/handlers.rs`

**Test Coverage:**
- Small grid (10x10)
- Large grid (100x100)
- Partial out-of-range grid (some extrapolated points)
- Performance benchmarks
- Response format validation

**Performance Target:**
- 72x46 grid (3312 points) completes in <2 seconds

---

#### 6.3 Antenna Listing & Details Endpoints (2-3 days)
**Objective:** Allow clients to query available antennas and their properties

**Steps:**
- Add `GET /api/v1/antennas` endpoint:
  - List all loaded antenna IDs
  - Include basic metadata (name, enabled status)
  - Sort alphabetically
- Add `GET /api/v1/antennas/{id}` endpoint:
  - Return detailed antenna information
  - Validity ranges for all dimensions
  - Calibration metadata (date, version, etc.)
  - Model statistics (knot counts, coefficient counts)
- Implement caching for antenna list (static after startup)

**Acceptance Criteria:**
- Antenna list returns all configured antennas
- Antenna details include all relevant metadata
- 404 error for unknown antenna IDs
- Response times <50ms

**Files to Create:**
- Update `src/api/handlers.rs` with antenna list/details handlers
- Update `src/api/schemas.rs` with antenna info types

**Test Coverage:**
- List all antennas
- Get details for existing antenna
- Get details for non-existent antenna (404)
- Metadata accuracy

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
**Objective:** Test complete workflows from API to interpolation

**Steps:**
- Create `tests/integration/` test suite:
  - Full API request/response cycles
  - Multi-antenna scenarios
  - Concurrent request handling
  - Error recovery paths
- Generate realistic test calibration data:
  - 2-3 complete antenna models
  - Various coverage patterns
  - Edge cases (sparse data, boundary regions)
- Test startup/shutdown sequences
- Test configuration variations

**Acceptance Criteria:**
- Integration tests run against real server instance
- All API endpoints covered by integration tests
- Concurrent access patterns tested
- Tests use realistic calibration data
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
  - Batch throughput (requests/second)
  - Heatmap generation time vs. grid size
  - Memory usage over time
  - Concurrent load testing
- Use `criterion` for statistical benchmarking
- Set up automated performance tracking
- Profile with `flamegraph` and `perf`
- Identify and optimize bottlenecks

**Acceptance Criteria:**
- Single evaluation p95 latency <100ms
- Batch throughput >10 req/s for small batches
- Heatmap generation meets performance targets
- Memory usage stable under load
- No performance regressions in CI

**Files to Create:**
- `benches/api_benchmarks.rs`
- `benches/interpolation_benchmarks.rs`
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
- Design trait-based compute backend abstraction
- Implement CUDA or compute shader backend
- Benchmark performance improvements
- Add configuration for backend selection

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

#### Multi-Temperature Support (2-3 sprints)
- Extend to full 4D interpolation (temperature dimension)
- Update calibration tool for temperature data
- API changes for temperature parameter
- Backward compatibility with 3D models

#### Uncertainty Quantification (2-3 sprints)
- Add confidence intervals to predictions
- Implement bootstrapping or Bayesian approaches
- Update API to return uncertainty estimates
- Visualization of uncertainty regions

---

## Risk Management

### High-Priority Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| B-spline fitting accuracy insufficient | Medium | High | Early validation against known data; fallback to higher-order splines |
| Performance targets not met | Medium | High | Early benchmarking in Sprint 2; plan for GPU acceleration |
| Calibration data format evolution | Low | Medium | Version headers in binary format; migration tools |
| Integration complexity underestimated | Medium | Medium | Buffer time in Sprint 7; daily standups to catch issues early |
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
   - ✅ Calibration CLI tool generates valid artifacts
   - ✅ Support for multiple antenna configurations
   - ✅ Interpolation accuracy within 1 dB for in-range queries
   - ✅ Proper warning generation for extrapolated queries

2. **Performance Requirements**
   - ✅ Single evaluation p95 latency <100ms
   - ✅ Batch throughput >10 req/s per instance
   - ✅ Startup time <10s
   - ✅ Memory footprint <512MB

3. **Quality Requirements**
   - ✅ >85% test coverage overall
   - ✅ Zero critical bugs in production
   - ✅ All documentation complete and reviewed
   - ✅ Successful deployment to production environment

4. **Operational Requirements**
   - ✅ Kubernetes deployment with health probes
   - ✅ Structured logging for all requests
   - ✅ Operational runbooks complete
   - ✅ On-call team trained on troubleshooting

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

### Before Sprint 4

- Sample measurement data (CSV files) for testing calibration tool
- Understanding of B-spline mathematics (recommend reading materials)

### Before Sprint 8

- Kubernetes cluster access (local or cloud)
- Docker registry for image storage
- Staging environment for deployment testing

---

## Appendices

### Appendix A: Recommended Reading

**B-Splines and Interpolation:**
- "A Practical Guide to Splines" by Carl de Boor
- "Geometric Modeling with Splines" by Cohen et al.
- SciPy interpolation documentation (for reference implementations)

**Rust Web Development:**
- Poem framework documentation
- Tokio async runtime guide
- Rust API guidelines

**Kubernetes:**
- Kubernetes documentation - Deployments, Services, ConfigMaps
- Helm documentation (if using Helm)

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
