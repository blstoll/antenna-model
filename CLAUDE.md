## Project Overview

Antenna Model Service is a high-performance REST API for parabolic dish antenna gain modeling using **physical optics computation** with calibrated correction surfaces. The system computes G/T (Gain-to-Temperature) predictions based on 3D geometry, supporting real-time queries with <100ms p95 latency.

**Key Architecture:** Hybrid physics-based model combining:
1. **Physical optics computation** - Aperture integration with phase functions (path, coma, surface error via the statistical Ruze efficiency, mesh effects)
2. **Correction surface** - B-spline interpolation for residual error corrections (measured - physics model)

## Commands

### Build and Test
```bash
cargo build --release          # both binaries

# Dev inner loop. The default nextest profile EXCLUDES the slow tier (four named
# calibration scenarios) — see .config/nextest.toml. Both heavy physics pins rejoined
# this tier on 2026-09-06 (#74) when antenna-core became an optimized compile.
cargo nextest run --workspace

# Both tiers — what scripts/check.sh and CI run. `calibrate` dominates the wall clock.
cargo nextest run --workspace --profile full

# Doctests are NOT run by nextest — they need their own command.
cargo test --doc --workspace

cargo nextest run -p antenna-core          # single workspace member
cargo nextest run --profile full --no-capture test_name   # single slow-tier test
cargo bench
```

**Tripwire:** the default tier runs in ~8 s and the full profile in ~18 s on an idle
8-core machine (measured 2026-09-06; they were ~20 s and ~110 s before #74). If your run
takes *minutes*, something has regressed — check two things first. One: `antenna-core` must
still be compiled at `opt-level = 3` in the debug and test builds
(`[profile.dev.package.antenna-core]` in the root `Cargo.toml`, GitHub issue #74).
Everything still passes without it, ~6x slower, which is why
`scripts/assert-numerical-certification.sh` asserts it. Two: `reqwest` in
`antenna-model/Cargo.toml` must still have `default-features = false`. Its default
`system-proxy` feature makes `reqwest::Client::builder().build()` cost ~11.8 s on
macOS via a serialized `configd` query, paid once per test that starts a `TestServer`
(848 s vs 33 s on the `antenna-model` suite alone). See
`docs/findings-2026-08-15-test-suite-execution-time.md`.

### Run Service
```bash
cargo run --release --bin antenna-model                          # http://localhost:3000
CONFIG_PATH=/path/to/service.yaml cargo run --release --bin antenna-model
```

### Calibration Tool
```bash
cargo run --release --bin calibrate -- \
  --input measurements/antenna_1.csv \
  --output calibration_data/antenna_1.bin \
  --antenna-id antenna_1 \
  --validate
```

### Code Quality
```bash
cargo fmt
cargo clippy -- -D warnings
cargo audit
./scripts/check.sh    # runs all checks exactly as CI does — single entrypoint
```

`check.sh` also runs **package-scoped** checks that no workspace-scoped command can perform,
because a workspace build unifies features ON across members and hides the properties
they test (roadmap D4):

- `cargo clippy -p antenna-core --all-targets` — the only compile of `antenna-core` with
  its `openapi` feature OFF, the configuration the CLI actually builds under.
- `scripts/assert-dep-graphs.sh` — `cargo build -p calibrate` is the only build using
  calibrate's normal deps alone. `clippy -p calibrate --all-targets` does NOT substitute:
  `--all-targets` pulls the dev-dependency `antenna-model` back in and re-unifies features.
  It asserts the dependency-graph invariants — each with a live control, since a guard
  whose power nothing asserts is exactly the rot roadmap P13 records. **Do not weaken the
  controls**; the script's header says what each invariant is and why.

It also runs `scripts/assert-numerical-certification.sh` (GitHub issue #74), which is
workspace-scoped but asserts two things no test result can show: the numerical certification
named in `.config/numerical-certification-manifest.txt` is still *selected* by the profiles
that are supposed to run it, and `antenna-core` is still compiled optimized so it stays
affordable there. A filter that drops a test and a deleted profile override both leave a
green suite — one with less in it, one several times slower. Same live-control discipline as
the dep-graph script.

## Repo Etiquette

Work may originate from either **roadmap units** with IDs (`D21`, `P13`, `C15`, `F6`, `S3`)
defined in `docs/roadmap-2026-07-work-units.md` or tickets in this repository's issue tracker.
Code comments, docs and commit subjects cite whichever source governs the change (for example,
`D21` or `#60`). Never commit to `main` — branch first. For implementing a roadmap unit and
opening its PR, use the `roadmap-unit` skill.

## Architecture

### Workspace Structure

Three members (roadmap D4). The split is an enforced boundary, not just layout —
`scripts/assert-dep-graphs.sh` fails the build if it erodes:

- **`antenna-core`** — physics engine and the calibration-artifact layer, plus the shared
  error/warning vocabularies. **No web stack**, and it stays under `CORE_MAX_DEPS` packages
  so it does not drift into a general-purpose crate.
- **`antenna-model`** — the REST service binary (poem).
- **`calibrate`** — the CLI tool. `antenna-core` is its only production dependency.

Two things about this that the code will not tell you:

- `antenna-model` glob-re-exports what moved to core, so both paths compile forever and
  nothing will ever flag the older one. **The canonical home of a physics or artifact type
  is `antenna_core::…` — prefer that path in new code.**
- **`calibrate` keeps `antenna-model` as a *dev*-dependency, and it must stay one.** It looks
  unused: do not remove it. `calibrate/tests/**` serve generated artifacts through the real
  service path (`service::compute_gain_from_request`), which is what caught the 27.3 dB C13
  defect — and it must stay dev-only so the shipped CLI compiles no web stack.

### Data Flow: API Request → Response

```
3D Positions → Coordinate Transforms → Physics Model → Correction Surface → Final Gain
```

`service/evaluator.rs` orchestrates this and its module docs carry the authoritative
step-by-step diagram (including beam squint, G/T, and loss) — read those rather than a copy.
`AntennaCalibration` (`antenna-core/src/data/types.rs`) is loaded at startup from the `.bin`
artifacts `antennas.yaml` names; see `.claude/rules/calibration.md` before touching it.

### Configuration System

- **Service config**: `config/service.yaml` (override the path with `CONFIG_PATH`). There is
  no `service.toml` — the loader reads YAML, and README/CLAUDE both claimed `.toml` until
  D9 checked (2026-08-16).
- **Antenna configs**: `calibration_data/antennas.yaml`. **No `.bin` artifacts ship in-repo**
  (roadmap D9) — the entries that reference one are `enabled: false` templates; the
  uncalibrated design-spec antennas are `enabled: true` and load with no artifact, so a
  clean checkout starts healthy. Counts drift — count them, don't quote a doc.

## Important Design Constraints

### Coordinate Systems Are Declared, Never Inferred

(`antenna-core/src/model/coordinates_3d.rs`, re-exported by `antenna-model/src/api/schemas.rs`)

- `Position3D.coordinate_system` is **required**: `"ecef"` (x,y,z meters from Earth's centre)
  or `"geodetic"` (lon°, lat°, alt m). Omitting it is a 400 naming the field.
- Construct in Rust with `Position3D::ecef(...)` / `Position3D::geodetic(...)`; there is no
  `new()` that picks a frame for you.
- **Do not reintroduce a default or a fallback.** The magnitude heuristic that C8 stage 2
  removed could not tell a geodetic GEO satellite from an ECEF point, and returned a
  silently wrong gain when it guessed.

### Other Constraints

- **Multi-feed:** antennas can have multiple feeds; the composite identifier is
  `(antenna_id, feed_id)`. Each feed has its own position, pattern, correction surface.
- **Performance targets:** single evaluation <100 ms p95; batch 1–20 req/s per instance;
  <512 MB memory; <10 s startup.
- **Accuracy:** <1 dB error in main lobe and first sidelobe; warnings (not errors) for
  extrapolated queries.

### Error Handling

- **Never use `unwrap()` or `expect()` in production code** — use proper error propagation.
- Use `thiserror` for error types (`antenna-core/src/error.rs`).
- Return actionable error messages specifying which field/parameter failed.
- Generate warnings (not errors) for extrapolation or edge cases. The typed warning and
  error vocabularies have a required change procedure — see `.claude/rules/api-contract.md`.

### Testing

- **Property tests** (roadmap D7) live in `antenna-core/tests/property_tests.rs`. Generators
  are constrained to the *validated physical domain* so they exercise the physics rather than
  rediscovering inputs upstream validation already rejects.
- **A new test costing >10 s either gets faster or joins the slow tier** in
  `.config/nextest.toml` — it stays CI-blocking, it just leaves the inner loop.

### Logging

`tracing` with structured fields, never format strings; include the request ID for
correlation. DEBUG for physics detail, INFO for requests, WARN for extrapolation.

## References

- **Domain Contract**: `docs/domain-contract.md` — coordinate frames, parameter meanings, and
  invariants. **Read this before touching anything in `model/coordinates*.rs`,
  `service/heatmap.rs`, or any API field named `*position*`/`*boresight*`.** Frame or
  parameter-meaning ambiguity has caused real, expensive bugs in this codebase before.
- **Architecture**: `docs/architecture.md`
- **Design Doc**: `docs/antenna-model-design-doc.md` — physical models and formulation
- **Calibration Guide**: `docs/calibration-workflow-guide.md`

## Agent skills

### Issue tracker

Issues are tracked in this repository's GitHub Issues using the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Triage roles use the default label vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

Domain documentation uses the single-context layout. See `docs/agents/domain.md`.
