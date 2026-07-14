# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Antenna Model Service is a high-performance REST API for parabolic dish antenna gain modeling using **physical optics computation** with calibrated correction surfaces. The system computes G/T (Gain-to-Temperature) predictions based on 3D geometry, supporting real-time queries with <100ms p95 latency.

**Key Architecture:** Hybrid physics-based model combining:
1. **Physical optics computation** - Aperture integration with phase functions (path, coma, surface error via the statistical Ruze efficiency, mesh effects)
2. **Correction surface** - B-spline interpolation for residual error corrections (measured - physics model)

Sprints 1–7 of 8 are complete (see `docs/implementation-plan.md`): physics engine, calibration tool, core + advanced REST endpoints, partial-calibration support, and boresight calibration are all built and tested.

## Commands

### Build and Test
```bash
# Build both service and calibration tool
cargo build --release

# Run all tests
cargo test --all

# Run specific workspace member tests
cargo test -p antenna-model
cargo test -p calibrate

# Run single test with output
cargo test test_name -- --nocapture

# Run benchmarks
cargo bench
```

### Run Service
```bash
# Run service locally (default: http://localhost:3000)
cargo run --release --bin antenna-model

# With custom config
CONFIG_PATH=/path/to/config.toml cargo run --release --bin antenna-model
```

### Calibration Tool
```bash
# Generate calibration artifacts from measurement CSV
cargo run --release --bin calibrate -- \
  --input measurements/antenna_1.csv \
  --output calibration_data/antenna_1.bin \
  --antenna-id antenna_1 \
  --validate
```

### Code Quality
```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Security audit
cargo audit

# Generate docs
cargo doc --open
```

## Architecture

### Workspace Structure
```
antenna-model/           # Cargo workspace root
├── antenna-model/      # REST API service binary
│   └── src/
│       ├── api/        # REST layer (poem framework)
│       ├── service/    # Business logic (evaluator, batch, validator)
│       ├── model/      # Physics engine (coordinates, geometry, phase, pattern)
│       ├── data/       # Calibration data types
│       └── config/     # Configuration system
├── calibrate/          # CLI calibration tool binary
│   └── src/
│       ├── parser.rs             # CSV measurement parsing
│       ├── parameter_tuner.rs    # Differential evolution optimizer
│       ├── correction_surface.rs # B-spline/RBF fitting
│       ├── validator.rs          # Cross-validation
│       └── serializer.rs         # Binary artifact generation
└── calibration_data/   # Calibration config (antennas.yaml) + generated *.bin artifacts (none checked in; see roadmap D9)
```

### Data Flow: API Request → Response

1. **API Layer** (`src/api/`) - poem framework routes and handlers
   - Middleware: RequestId, RequestLogger, ErrorHandler, RequestSizeTracker
   - Schema validation via `schemas.rs`

2. **Service Layer** (`src/service/`) - Business logic orchestration
   - `evaluator.rs` - Main gain computation pipeline
   - `validator.rs` - Input validation
   - `batch.rs` - Parallel batch processing

3. **Gain Computation Pipeline** (Service → Model layers):
   ```
   3D Positions → Coordinate Transforms → Physics Model → Correction Surface → Final Gain
   ```

   **Step-by-step:**
   - Parse request with 3D positions (ECEF or Geodetic, auto-detected)
   - Transform to antenna frame using vehicle position/attitude (`model/coordinates.rs`)
   - Compute emitter direction (azimuth, elevation) from geometry
   - Evaluate **physics model** (`model/pattern.rs`):
     - Aperture integration over reflector surface (`model/integration.rs`)
     - Phase accumulation: path + coma + mesh (`model/phase.rs`); surface error is applied statistically as a Ruze efficiency in `model/pattern.rs`, not as a per-point aperture phase
     - Feed illumination pattern (`model/illumination.rs`)
     - Apply Ruze efficiency and mesh transparency
   - Interpolate **correction surface** (4D B-spline — implemented and live in `model/correction_interpolator.rs`, applied in `service/evaluator.rs`)
   - Combine: `Gain_final = Gain_physics + Correction`
   - Generate warnings for out-of-range queries

4. **Data Layer** (`src/data/types.rs`) - `AntennaCalibration` structure
   - `physical_config: PhysicalAntennaConfig` - reflector geometry, feed parameters
   - `correction_surface: Option<BSplineModel4D>` - residual corrections
   - Loaded at startup from `.bin` artifacts referenced by `antennas.yaml`. **No `.bin` artifacts ship in-repo: the four `antennas.yaml` entries that reference a `.bin` calibration file are `enabled: false`, while the four uncalibrated design-spec antennas are `enabled: true` and load from `calibration_data/design_specs/` — see roadmap unit D9.**

### Key Physics Modules (`antenna-model/src/model/`)

- **`coordinates.rs`** - ECEF ↔ Geodetic ↔ Antenna Frame ↔ Spherical transforms
- **`geometry.rs`** - `ReflectorGeometry`, `FeedParameters`, `MeshParameters`
- **`phase.rs`** - Phase functions: path length, coma (full path-length model), surface error (statistical Ruze model; per-point Zernike maps are not implemented — the aperture integrand uses `surface_error = 0.0` and the calibration correction surface absorbs systematic surface deviations), mesh
- **`illumination.rs`** - Feed pattern: cos^q with q-factor
- **`integration.rs`** - Adaptive Simpson's rule aperture integration
- **`pattern.rs`** - Far-field pattern computation with Ruze efficiency
- **`coordinates_3d.rs`** - 3D position → antenna-frame direction transforms (ECEF/geodetic vehicle geometry)
- **`correction_interpolator.rs`** - 4D B-spline evaluation of the residual correction surface
- **`edge_cases.rs`, `ray_trace.rs`** - Special case / large-feed-offset handling
- **`mesh.rs`** - Mesh transparency (wire-mesh reflection efficiency). Surface RMS / Ruze efficiency lives in `pattern.rs`.

### Coma Aberration Model

The coma aberration (feed displacement) uses a **full path-length model** that computes the exact geometric path difference between:
- Path from ideal focal point to each aperture point on the parabolic surface
- Path from displaced feed position to each aperture point

This naturally includes all orders of aberration:
- **First order (linear)**: Beam steering (θ ≈ δ/f)
- **Second order**: Defocus/astigmatism effects
- **Third order**: True coma with asymmetric sidelobes
- **Higher orders**: Additional aberrations for large displacements

The model is more accurate than simplified linear approximations, especially for:
- Large feed offsets (>0.1f)
- Predicting gain loss at boresight when feed is displaced
- Computing asymmetric sidelobe patterns (coma lobes)

### Calibration Workflow

The `calibrate` tool processes measurement data:

1. **Parse CSV** (`parser.rs`) - Read G/T measurements (azimuth, elevation, frequency, temperature, g_over_t_db)
2. **Tune Parameters** (`parameter_tuner.rs`) - Differential evolution optimizer adjusts physical parameters
3. **Fit Correction Surface** (`correction_surface.rs`) - B-spline/RBF fitted to residuals (measured - physics)
4. **Validate** (`validator.rs`) - Cross-validation, ensure <1 dB error in main lobe/first sidelobe
5. **Serialize** (`serializer.rs`) - Generate binary `.bin` artifact with `AntennaCalibration` structure

### Configuration System

- **Service config**: `config/service.toml` or environment variables
- **Antenna configs**: `calibration_data/antennas.yaml` - lists available antennas
- **Calibration data**: Binary `.bin` artifacts referenced by `antennas.yaml` (generated locally; none committed — see D9)
- Uses `config` crate for hierarchical config (file + env vars)

## Important Design Constraints

### Physics Model Implementation

1. **Coordinate System Auto-Detection** (`model/coordinates.rs`)
   - If `|x|, |y|, or |z| > 6400 km` → ECEF coordinates
   - Otherwise → Geodetic (lon, lat degrees; alt meters)

2. **Multi-Feed Support**
   - Antennas can have multiple feeds
   - Use composite identifier: `(antenna_id, feed_id)`
   - Each feed has unique position, pattern, correction surface

3. **Performance Targets**
   - Single evaluation: <100ms p95 latency (physics computation is expensive)
   - Batch throughput: 1-20 req/s per instance
   - Memory: <512MB footprint
   - Startup: <10s

4. **Accuracy Requirements**
   - <1 dB error in main lobe (validated against measurements)
   - <1 dB error in first sidelobe
   - Warnings for extrapolated queries (out of calibrated range)

### Error Handling

- **Never use `unwrap()` or `expect()` in production code** - use proper error propagation
- Use `thiserror` for error types (`src/error.rs`)
- Return actionable error messages specifying which field/parameter failed
- Generate warnings (not errors) for extrapolation or edge cases

### Testing Philosophy

- Unit tests for all physics functions (with known reference values)
- Integration tests with realistic calibration data
- Property-based tests for coordinate transforms (round-trip accuracy) — *planned; not yet implemented, see roadmap unit D7*
- Benchmarks for performance-critical paths (aperture integration is hottest)
- Target: >80% test coverage

### Logging

- Use `tracing` with structured fields (not format strings)
- Include request IDs for correlation
- Log at appropriate levels: DEBUG for physics details, INFO for requests, WARN for extrapolation
- JSON format in production for structured parsing

## Project Status

Per `docs/implementation-plan.md`, Sprints 1–7 are complete:
- Physics engine (aperture integration, phase functions, far-field pattern, Ruze/mesh efficiency).
- Calibration tool (parameter tuning, correction-surface fitting, boresight calibration).
- REST API: single gain, batch, rectangular heatmap, H3 link budget, antenna/feed listing,
  partial-calibration statuses, multi-feed support.
- The **4D B-spline correction surface is implemented and live** (`model/correction_interpolator.rs`,
  applied at `service/evaluator.rs:265-287`).

Active hardening and debt work is tracked in `docs/roadmap-2026-07.md` and
`docs/roadmap-2026-07-work-units.md`.

## Common Pitfalls

1. **Coordinate System Confusion**: See `docs/domain-contract.md` for the frame table and known gotchas (ENU axis direction, GEO-altitude auto-detection, antenna-frame origin, `feed_position` = pointing target not physical offset) before touching coordinate transforms.

2. **Aperture Integration Performance**: This is the computational bottleneck. The adaptive Simpson's rule must converge accurately within time budget.

3. **Phase Wrapping**: Phase functions must handle 2π wrapping correctly (see the phase accumulation in `model/phase.rs`).

4. **Feed Offset Sign Conventions**: Coma lobe direction depends on feed displacement sign; follow right-hand rule.

5. **Correction Surface vs Physics Model**: Correction surface is *residual* (measured - physics), not absolute gain.

6. **Validity Ranges**: Queries outside calibrated ranges should generate warnings but still return values (extrapolated).

7. **No system BLAS — the build is pure Rust**: `cargo build` / `cargo test` need no environment variables, no Homebrew packages, and no system libraries on any platform. Do not add `LDFLAGS`/`CPPFLAGS`, and do not reintroduce `ndarray-linalg`/OpenBLAS. The correction-surface fit (`correction_surface.rs`) exploits the B-spline's local support to accumulate the normal equations `(BᵀB + λI)` directly from the `order³` non-zero basis values per data point, then solves the SPD system with an in-house Cholesky factorization. This is both dependency-free and substantially cheaper than the dense `BᵀB` product it replaced.

## References

- **Implementation Plan**: `docs/implementation-plan.md` - Sprint-by-sprint development plan (8 sprints)
- **Architecture Doc**: `docs/architecture.md` - System architecture and deployment
- **Design Doc**: `docs/antenna-model-design-doc.md` - Physical models and mathematical formulation
- **Sprint 1-4 Summary**: `docs/implementation-plan-sprints-1-4-summary.md` - Foundation work completed
- **Domain Contract**: `docs/domain-contract.md` — coordinate frames, parameter meanings, and invariants. Read this before touching anything in `model/coordinates*.rs`, `service/heatmap.rs`, or any API field named `*position*`/`*boresight*`. Frame or parameter-meaning ambiguity has caused real, expensive bugs in this codebase before.

## Physics References (for Physical Model Work)

- **Antenna Theory**: Balanis - reflector antenna chapters
- **Ruze Equation**: J. Ruze "Antenna Tolerance Theory" (1966) - surface error effects
- **Zernike Polynomials**: Noll "Zernike Polynomials and Atmospheric Turbulence" - standard ordering
- **Mesh Reflectors**: Wire mesh EM scattering literature
- **Numerical Integration**: Gaussian quadrature, adaptive Simpson's rule (Press et al. "Numerical Recipes")
