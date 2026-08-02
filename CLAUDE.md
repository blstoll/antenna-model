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

# Run all tests — dev inner loop (~86 s, 980 tests). The default nextest
# profile excludes the slow tier: three heavy physics pins + the calibrate
# full-mode e2e binary. See .config/nextest.toml and roadmap unit D18.
# (P10-perf returned six pins to this tier on 2026-08-01 by making the mode
# integrator 2.4–7.4× cheaper — the list is meant to shrink, not ratchet.)
cargo nextest run --workspace

# Run BOTH tiers — what scripts/check.sh and CI run
cargo nextest run --workspace --profile full

# Run specific workspace member tests
cargo nextest run -p antenna-model
cargo nextest run -p calibrate

# Run single test with output. Use --profile full for a slow-tier test — the
# default profile filters it out and reports 0 tests run.
cargo nextest run --profile full --no-capture test_name

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

# Run all checks exactly as CI does (fmt --check, clippy --workspace
# --all-targets -D warnings, full workspace tests, cargo audit) — single
# entrypoint. Sets RUST_MIN_STACK to match CI; the ad-hoc one-liners above
# do not, and calibrate's 3D→4D round-trip overflows the default stack.
./scripts/check.sh
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
│       ├── parameter_tuner.rs    # Nelder-Mead simplex optimizer
│       ├── correction_surface.rs # B-spline/RBF fitting
│       ├── validator.rs          # Cross-validation
│       ├── artifact_export.rs    # Service-loadable AntennaCalibration (3D→4D bridge)
│       └── sidecar.rs            # Optional JSON metadata/report sidecars only
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
   - Parse request with 3D positions (ECEF or Geodetic, per each position's required `coordinate_system` tag)
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
- **`integration.rs`** - Aperture integration via the **Hankel / azimuthal-mode (Jₘ) integrator** (roadmap P10, landed 2026-07-15): the φ' integral is collapsed analytically (Jacobi–Anger), radial density is derived adaptively from `(D/λ, θ)` at ~2× Nyquist, and runtime self-checks flag non-convergence (surfaced as a response warning). **Both branches now verify BOTH axes** (roadmap **P12**, landed 2026-07-31, `PHYSICS_MODEL_VERSION` 6). Until P12 the asymmetric (azimuthal-mode) branch sized `n_rho` once and self-checked only mode truncation `I(M)` vs `I(M+1)`, so on a laterally-offset or `asymmetry_factor != 1.0` feed — **five of the enabled feeds** — `converged = true` asserted nothing about the radial quadrature. Measured silent errors: 0.82 dB (`gs_3.7m` X-band, θ=5°), 1.17 dB (`dsn_34m` X-band, θ=0.10°), **7.08 dB** (D12's UHF fixture, θ=16°, φ=0); all now within 0.013 dB. The mechanism was **not** a missing term in the cycle budget (measured bandwidth 7–8 cycles vs the budget's 10.5) and **not** a too-coarse samples-per-cycle constant (the symmetric branch is 0.043 dB accurate at the same density) — it was that the mode path returned the coarse leg and never checked, while the answer is a residue of mode integrals that cancel 59–111×, so per-mode errors of ~1% become ~10% of the result. The mode path now returns the **fine (2N) leg** and refines until converged (`MAX_RADIAL_REFINEMENTS`). P12 also put a cheap `{0,1}`-mode pre-gate in front of that loop on expensive geometries; **`PHYSICS_MODEL_VERSION` 8 (P13, 2026-08-01) deleted it**, along with `RADIAL_PROBE_MODES`, `RADIAL_PRE_GATE_SAFETY`, `FULL_RADIAL_CHECK_WORK_LIMIT` and `radial_probe_field`, so there is now exactly one radial shape for every geometry. Two independent reasons, and the first is the one to remember: the safety factor **stopped bounding its quantity because of a change with no physics content** — P10-perf's `next_fast_len` φ' resizing (512 → 270) moved the worst *passing* probe-to-total ratio from 26× to **43.5×** against a constant of 32, on `dsn_34m` Ka θ=90°, with nothing in the build able to notice. Second, post-FFT the pre-gate was strictly dominated: 2.33× baseline returning the *coarse* leg, where computing at 2N and returning the *fine* leg costs 2.00×. Deleting it made the affected Ka geometries **16× more accurate** (+0.0126 → +0.0008 dB) for +28 % work, and made `dsn_34m` X θ=45° **31 % cheaper**. **Do not reintroduce a fitted numeric guard on this path without a test that asserts its margin** — that absence is what let this one rot silently. See `docs/findings-2026-08-01-p13-pre-gate-retirement.md` and `docs/findings-2026-07-31-p12-mode-path-radial-budget.md`. **The φ' cap is fixed too** (same unit, `PHYSICS_MODEL_VERSION` 7). `MODE_PHI_STEERED_MAX` used to clamp `n_phi` to 64 on steered feeds (`δ/f > 0.05`), aliasing high modes into `g₀` — measured **+82 dB** wrong against the 2D oracle on a routine ~5° beam steer, with `converged = true`, because neither existing check can see φ' aliasing (both operate on the already-corrupted `gₘ`). `n_phi` is now sized from the azimuthal bandwidth `B = k·δ·(R/f)`, rounded up only to the next even 5-smooth FFT length (**P10-perf**, 2026-08-01 — before that it was not rounded at all, because the transform was a naive DFT), `MODE_PHI_MAX` = 2048, and `ModeSizing::azimuthally_resolved` gates `converged` when the ceiling binds. An effort ceiling *does* remain, but keyed to `SEVERE_OFFSET_THRESHOLD` (0.5f) — the model's own PO scope boundary, where it already warns and routes to the ray-tracing stub — instead of the old arbitrary 0.05, and it announces itself instead of being silent. Removing it outright over-corrected: the integration suite's 3.06f-steered fixture went from 5.5 s to 66 s converging a number the model had already disclaimed. Its sibling `MODE_RADIAL_CYCLE_CAP` was re-keyed the same way (now `BEYOND_SCOPE_COMA_CYCLE_CAP`): inside scope it was strictly harmful, making the same geometry *both* 0.34 dB worse and 2× more expensive (a refinement loop started below the physics discards every wasted leg); outside scope it is kept, and P12's radial check reports if it costs accuracy. **The rule both caps now follow: size from the physics inside the model's scope, cap effort outside it, never be silent about which.** The old constants capped *inside* scope and did it silently — the threshold, not the mechanism, was the defect. That in turn exposed `m_theta = k·R·sinθ + 6`: `Jₘ` has an Airy turning point at `m = x` with transition width `~x^(1/3)`, so a flat `+6` truncated live spectrum (+0.49 dB at θ=3°); it is now `x + 4·x^(1/3) + 6`. **Cost:** a steered geometry is ~69× more expensive than the capped version, which briefly made it a *coverage* item — it could hit S3's wall-clock budget and serve a 504 rather than return an aliased number. **Closed by P10-perf, 2026-08-01** (no physics change, `PHYSICS_MODEL_VERSION` still 7): the φ' transform is now an in-house mixed-radix FFT (`model/fft.rs`, `O(n_φ log n_φ)` instead of `O(n_φ·M)`), every `Jₘ` order at a given argument comes from one recurrence sweep (`bessel_jn_array`, `O(M)` instead of `O(M²)`), and the aperture-plane function `g(ρ,φ')` — which those two changes left as ~79% of a sweep — lost its `acos`→`cos` round trip and its per-radial-sample recomputation of φ'-invariant trigonometry. Net **2.4–7.4×**: the ~5° steer went 500 → 67 ms (its test 22.3 s → 4.0 s), `dsn_34m` Ka θ=90° 2135 → 559 ms. The φ' axis has exactly one automatic guard, `served_n_phi_sizing_is_sufficient_on_every_asymmetric_geometry`; do not weaken it. The legacy 2D Simpson quadrature survives only as a `#[cfg(test)]` reference oracle. The `IntegrationParams` presets (`fast()`, `high_accuracy()`) no longer gate served correctness — the served path uses `adaptive()` and most preset fields are inert (see the docstrings in `integration.rs`).
- **`bessel.rs`** - In-house Bessel Jₘ (pure Rust), pinned by tests in every branch, across the **turning point** `m ≈ x`, and — since **P14** (2026-08-01, `PHYSICS_MODEL_VERSION` 9) — against an **independent oracle**: a compensated trapezoidal quadrature of `Jₘ(x) = (1/2π)∫₀^{2π} cos(mτ − x sinτ)dτ`, which shares no machinery with the recurrences. Add that oracle to any Bessel change: the module's other graders are recurrence identities, and *an identity is scale-invariant* — a uniformly mis-normalized Miller result satisfies it exactly, which is the one way Miller's algorithm actually fails. `bessel_jn_array` returns every order `J_0…J_{m_max}` from a single sweep and is what the mode integrator uses — do not mix it with per-order `bessel_jn` calls on the same path, since the two select their recurrence direction from different orders. P14 closed the accuracy cliff at `m ≈ x` (was 2e-8 at x=255, 9e-3 at x=10⁴, **growing without bound in x**; now ~3e-16 flat) by making the Miller start offset scale with the turning-point width, `12·x^(1/3)`, where the 12 is **derived** from an Airy decay requirement rather than fitted — and, per P13's lesson, `miller_start_offset_has_real_margin` asserts that constant's margin *directly*, by re-running the shipped recurrence at 3× the offset and requiring the answer not to move, with a negative control proving the check has power. **Two accuracy floors remain, both deliberate and pinned:** `bessel_j0`/`bessel_j1` above |x| = 8 are still the Numerical Recipes rational fit at **~3e-9 absolute** (below |x| = 8 P14 replaced it with the convergent series, ~1e-14 and exactly 1 at the origin), and that ceiling propagates to every order the *upward* branch produces; and a renormalized downward sweep is accurate to ~ε·(largest Jₘ in the sweep) in **absolute** terms, so orders well below the turning-point peak are relatively less accurate by exactly that ratio — chasing either one relatively is asking a normalized recurrence for something it cannot give.
- **`fft.rs`** - Mixed-radix (2/3/5) forward FFT backing the integrator's φ' transform (P10-perf). Crate-internal, forward-only, deliberately not a general FFT crate. `next_fast_len` rounds a requested length up to the next even 5-smooth number — **not** a power of two, because the padding is paid in aperture-plane evaluations (536 → 540 costs 0.7%; 536 → 1024 would cost 91%). Validated against a literal DFT transcription at every fast length the integrator can ask for, not spot-checked.
- **`pattern.rs`** - Far-field pattern computation with Ruze efficiency and the Huygens obliquity factor `(1+cosθ)/2` (F7, 2026-07-16, `absolute_gain_from_integral`)
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

**No separate higher-order aberration mode (roadmap P2, 2026-07):** because
`phase_feed_displacement` is the *exact* geometric path difference, it already carries the
complete low-order aberration content (astigmatism, field curvature, distortion, trefoil) as
an exact function of the displacement. The former `HigherOrderAberrations` computation mode
(0.3f–0.5f band) added heuristic Seidel terms *on top of* that exact phase — a double-count,
and worse, with wrong-sign/wrong-scale/wrong-pupil-power coefficients (e.g. it coded ρ³
distortion where the exact model and classical theory give leading ρ¹). It was removed;
0.3f–0.5f offsets now route through `StandardPhysicalOptics`, whose exact coma phase covers
them. The completeness pin
`edge_cases::exact_feed_displacement_phase_contains_all_low_order_aberrations` proves the
exact phase's full content against an independent closed form. Offsets >0.5f still route to
the ray-tracing stub (roadmap P3). This is why `PHYSICS_MODEL_VERSION` is 4.

### Calibration Workflow

The `calibrate` tool processes measurement data:

1. **Parse CSV** (`parser.rs`) - Read G/T measurements (azimuth, elevation, frequency, temperature, g_over_t_db)
2. **Tune Parameters** (`parameter_tuner.rs`) - Nelder-Mead simplex optimizer adjusts physical parameters (surface RMS, mesh spacing, wire diameter). Search bounds come from `ParameterBounds::from_class` — a multiplicative bracket around each antenna class's own nominal, **not** a fixed global range (D16, 2026-07-31). The objective must be evaluated under the same `IntegrationParams` as `main.rs::compute_model_predictions` (`default()`), or the tuner optimizes against integrator discretisation error rather than the physics — see `docs/findings-2026-07-30-full-mode-parameter-tuning-broken.md` defect 4.
3. **Fit Correction Surface** (`correction_surface.rs`) - B-spline/RBF fitted to residuals (measured - physics). **The data requirement is the coefficient count `∏(placed_knots_axis + order)`, not the `(spline_order+1)³ = 125` pre-check** (roadmap D20, 2026-08-02): an underdetermined fit is now a hard `UnderdeterminedFit` error, checked after knot generation because the knot counts in `CorrectionSurfaceParams` are a *request* that interior-only placement and minimum-spacing can reduce. Full mode's shipped 4/6/8 counts declare up to 960 coefficients, so a full-mode dataset needs ≥960 points — and ≥1440 if a 3-fold cross-validation has to pass, since the training split is what must cover them. Switching this check on failed 24 tests that had been fitting underdetermined surfaces and reporting excellent RMSE: such a fit interpolates its own data points almost exactly while oscillating between them. Related, same day: adaptive knots are now **strictly interior** (D19) — a knot equal to an axis bound became multiplicity `order+1` after clamping, giving that basis function zero-width support, so 37.5% of the shipped configuration's coefficients were attached to functions that were identically zero. `validate_knot_vector` enforces end multiplicity `== order` and interior `<= order-1`. **Sizing a test fixture here means sizing it to the coefficient count of the params it fits, and for the tightest CV fold it runs** — not to 125.
4. **Validate** (`validator.rs`) - Cross-validation, ensure <1 dB error in main lobe/first sidelobe
5. **Serialize** (`artifact_export::write_calibration_artifact` — the tool's **only** artifact writer, shared by full and boresight mode since D2, 2026-07-30) - Generate binary `.bin` artifact: an `AntennaCalibration` encoded with **postcard** (documented, versioned wire format), wrapped in the ANTC header (magic + version + CRC32 + length). Migrated off the unmaintained `bincode` crate 2026-07-18. An artifact carries **two** version axes and the loader enforces both: the ANTC header `u32` (`ANTC_ARTIFACT_VERSION` = 2) is the *container* axis, readable before the decode; `metadata.format_version` (`CALIBRATION_SCHEMA_VERSION` = "2.0") is the *schema* axis, readable only after it, and a foreign MAJOR is a hard error. See `data/loader.rs`'s module docs and `docs/calibration-workflow-guide.md` §10.5.1 before touching either. Do NOT add `#[serde(skip_serializing_if)]`/`skip`/`flatten` to any serialized calibration type — postcard is positional and non-self-describing, so those attributes silently corrupt the format (see the note atop `data/types.rs`).

### Configuration System

- **Service config**: `config/service.toml` or environment variables
- **Antenna configs**: `calibration_data/antennas.yaml` - lists available antennas
- **Calibration data**: Binary `.bin` artifacts referenced by `antennas.yaml` (generated locally; none committed — see D9)
- Uses `config` crate for hierarchical config (file + env vars)

## Important Design Constraints

### Physics Model Implementation

1. **Coordinate Systems Are Declared, Never Inferred** (`api/schemas.rs`)
   - `Position3D.coordinate_system` is **required**: `"ecef"` (x,y,z meters from Earth's
     centre) or `"geodetic"` (lon°, lat°, alt m). Omitting it is a 400 naming the field.
   - Construct in Rust with `Position3D::ecef(...)` / `Position3D::geodetic(...)`; there is
     no `new()` that picks a frame for you.
   - The former magnitude heuristic (>6400 km → ECEF) was removed by roadmap unit C8 stage 2
     (2026-07-27). **Do not reintroduce a default or a fallback** — it could not tell a
     geodetic GEO satellite from an ECEF point, and silently returned a wrong gain when it
     guessed. See `docs/domain-contract.md`, "Resolved by design 2026-07-27".

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
- Generate warnings (not errors) for extrapolation or edge cases. Response warnings are
  **typed**: `ApiWarning { code: WarningCode, message: String }` (roadmap C8 stage 3,
  2026-07-27). `WarningCode` is a **closed** enum in `src/warnings.rs` — a peer of
  `error.rs`, since the model layer produces warnings too. Adding a producer means adding
  a variant, updating `WarningCode::ALL` and `docs/api-documentation.md`, then regenerating
  `openapi.yaml` (`cargo run -p antenna-model --bin generate_openapi` — the spec is
  generated since C7, never hand-edited); `tests/warning_code_vocabulary.rs` fails
  otherwise. The error-code vocabulary is the closed `ErrorCode` enum in `api/schemas.rs`
  (promoted from `&str` consts by C7) with the same procedure via
  `tests/error_code_vocabulary.rs`. **`code` is the contract, `message`
  is not** — never branch on message text (the substring test that C8 stage 3 deleted from
  `service/heatmap.rs` is why). Heatmap/H3 aggregation dedupes on `(code, message)`, so a
  warning meant to appear once per response must keep its message constant across grid
  points.

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
  partial-calibration statuses, multi-feed support. `/heatmap` serves **rectangular grids
  only** — the `h3` grid type was a `not_implemented` stub, removed 2026-07-28 (roadmap C8
  stage 4); the real H3 grid is the separate `/h3-heatmap` endpoint. A merge of the two is
  tracked as feature **F5**, not yet decided.
- The **4D B-spline correction surface is implemented and live** (`model/correction_interpolator.rs`,
  applied at `service/evaluator.rs:265-287`).
- The **P10 off-axis integrator landed 2026-07-15**: served off-axis gain is numerically
  converged at all angles (the pre-P10 aliasing that returned gain 20–35 dB too high beyond a
  few degrees is fixed). Served values on uncalibrated antennas are *idealised* physical optics
  (no blockage/strut/edge-diffraction), stated honestly by the off-axis warning.
- The **F7 sidelobe-floor redesign landed 2026-07-16/17** (`PHYSICS_MODEL_VERSION` 5): Huygens
  obliquity factor `(1+cosθ)/2` on the far-field conversion, plus the statistical Ruze sidelobe
  floor on uncorrected-physics antennas (power sum forward, floor-only rear). Calibrated
  antennas unaffected — see `docs/domain-contract.md`.

Active hardening and debt work is tracked in `docs/roadmap-2026-07.md` and
`docs/roadmap-2026-07-work-units.md`.

## Common Pitfalls

1. **Coordinate System Confusion**: See `docs/domain-contract.md` for the frame table and known gotchas (ENU axis direction, the removed GEO-altitude auto-detection, antenna-frame origin, `feed_pointing_location` = pointing target not physical offset) before touching coordinate transforms.

2. **A wrong oscillatory integrator is not obviously wrong** — it returns a plausible number. Any change to `integration.rs` or `bessel.rs` must be cross-checked at angles whose answers are independently known, spanning the full θ range **and both Bessel branches** (small-argument and asymptotic): a P10-era spike was confidently wrong by 22 dB at θ=0 while looking flawless at θ=90°, because special-function bugs fail branch-locally. The validation protocol lives in `antenna-model/tests/reference_validation.rs` (anchors, independent Hankel oracle, physicality sweeps, and since P12 the mode-path radial-convergence anchors + symmetric control) — run it, and never validate at a single angle. **Cross-check against a method that is not the one you are changing**: P12's `p2_moderate_offset` pin moved 2.3 dB and only the 2D Simpson oracle could show that *both* the old and new values were ~29 dB wrong for an unrelated reason. Performance note: the integrator is O(D/λ) per point, cheap near boresight; the remaining hot case is wide-angle Ka on offset-feed (coma) antennas (~559 ms at θ=90° after P10-perf, down from 2135 ms). **Never buy speed by reducing sample density** — P10-perf got 2.4–7.4× without touching a single sample count, by making each sample cheaper (FFT φ' transform, one-sweep `Jₘ` ladder, hoisted φ'-invariant trigonometry). The remaining cost is ~85% aperture-plane function evaluation, so that is where the next win is, not in the quadrature. Counter-intuitively, cost and convergence are **anti-correlated** here: every geometry measured with a radial error was sub-millisecond, while the 300 ms–3.7 s Ka cases were already accurate to ±0.02 dB.

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
- **Numerical Integration & Special Functions**: Jacobi–Anger expansion / Hankel transforms for the azimuthal collapse; composite Simpson's rule for the radial quadrature; mixed-radix Cooley–Tukey FFT for the φ' Fourier coefficients; Bessel Jₘ rational approximations and recurrences, including Miller's downward recurrence for the whole order ladder (Press et al. "Numerical Recipes"; Abramowitz & Stegun for reference values)
