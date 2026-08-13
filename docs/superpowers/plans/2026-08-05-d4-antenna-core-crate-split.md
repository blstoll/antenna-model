# D4 — Extract `antenna-core` Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the workspace into three crates so `calibrate` stops compiling the web stack: `antenna-core` (physics engine + calibration artifact types + shared error/warning vocabulary), `antenna-model` (REST service), `calibrate` (CLI).

**Architecture:** Pure mechanical move (roadmap D4's gotcha: "`git mv` files, fix `use` paths, change nothing else — if any test value changes, the move went wrong"). `antenna-core` receives `antenna-model/src/model/` (all 14 files), `data/types.rs`, `data/loader.rs`, `error.rs`, `warnings.rs`, and `Position3D`/`CoordinateSystem` out of `api/schemas.rs`. `antenna-model` re-exports every moved module at its old path, so **zero** import changes are needed in `api/`, `service/`, `config/`, tests, or benches. `calibrate`'s normal dependency becomes `antenna-core`; `antenna-model` drops to a dev-dependency (its e2e tests deliberately serve artifacts through the real service loader/evaluator — that path is what caught C13 and must not be weakened). The one non-move edit: `utoipa::ToSchema` derives on the four moved API-visible types go behind an `openapi` cargo feature that only `antenna-model` enables, so utoipa never enters calibrate's tree.

**Tech Stack:** Rust workspace (cargo, nextest, clippy), postcard/crc32fast (moved with loader), feature-gated utoipa =5.5.0.

**User decisions (already made):**
- Maintainer decision (roadmap decision register, D4 row): **Split** — mechanical move, after Phases 1–3 (Phases 1–3 landed; C7/C8 complete as of 2026-07-28).
- "Attempt ndarray unification during the split": **already satisfied** — both crates are on ndarray 0.16.1 and `ndarray-linalg` was removed by the no-BLAS work. `ndarray` is now a *dead* dependency of antenna-model (zero hits in src/tests/benches) and gets pruned here.
- `calibrate/src/mod.rs` orphan: roadmap hygiene note says "delete there or on the next touch of the crate" — this is that touch (Task 3).
- Test-only invariant: no test value may change; the existing ~980-test suite is the oracle. No new tests are required by D4's exit criteria.

**Exit criteria (from `docs/roadmap-2026-07-work-units.md` D4):** three-crate workspace; `cargo tree -p calibrate` shows no poem/h3o/tokio-web deps; all tests pass; CI green; CLAUDE.md + architecture.md module maps updated.

**Measured baseline (2026-08-05, main @ 6f42799):** `cargo tree -p calibrate -e normal | grep -cE 'poem|h3o|utoipa|dashmap|lru'` → **9**. After Task 3 this must be **0**.

---

## Dependency map (verified 2026-08-05, drives the task ordering)

What the moving set references outside itself (all verified by grep):

| From | To | Resolution |
|---|---|---|
| `model/{pattern,edge_cases,correction_interpolator}.rs` | `crate::warnings::{ApiWarning, WarningCode}` | `warnings.rs` moves to core (Task 1) |
| `model/*`, `data/{types,loader}.rs` | `crate::error::*` | `error.rs` moves to core (Task 1) |
| `model/coordinates_3d.rs` | `crate::api::schemas::Position3D` | `Position3D` + `CoordinateSystem` move INTO `coordinates_3d.rs` (Task 2) |
| `data/loader.rs` | `crate::model::PHYSICS_MODEL_VERSION`; `data/types.rs` → `crate::model::geometry` | model/ and data/{types,loader} must move **in the same commit** (Task 2) |
| `data/repository.rs` | `crate::config::*`, `parking_lot` | repository.rs **stays** in antenna-model (service-side; loads antennas.yaml via the service config system) |
| `warnings.rs`, `Position3D`, `CoordinateSystem` | `utoipa::ToSchema` derives | feature-gated `openapi` feature in core, enabled only by antenna-model |

External crates used by the moving set (become antenna-core deps): serde, serde_json, thiserror, tracing, num-complex, rayon, postcard, crc32fast; dev: tempfile (loader.rs inline tests). NOT needed: anyhow, parking_lot, ndarray, poem, h3o, tokio, config, dashmap, lru.

Infrastructure verified to need **no** changes: `.config/nextest.toml` (slow-tier filters are `test(name)` / `binary_id(calibrate::…)` — name-based and package-agnostic; `mode_path_reports_a_radial_error_even_when_the_density_cap_binds` lives in `integration.rs` and moves to core, but `test()` filters match across packages), `.github/workflows/ci.yml` (everything is `--workspace`), `scripts/check.sh` (workspace-wide).

## Out of scope (do not do)

- Moving `data/repository.rs`, `antenna-model/tests/*`, or `antenna-model/benches/*` — they stay where they are and keep working through re-exports.
- Any physics, API, serialization, or behavior change. Both artifact version axes (`ANTC_ARTIFACT_VERSION` = 4, `CALIBRATION_SCHEMA_VERSION` = "5.0") are untouched — postcard layout does not move.
- Renaming import paths inside antenna-model or calibrate's *tests* (`use antenna_model::model::…` in calibrate tests keeps working via the dev-dependency + re-exports).
- A standing CI guard asserting the dependency property (optional follow-up; D4's exit criteria only require the state).
- If a bug falls out during the move: STOP, file it as a new roadmap item, do not fix in-branch (per the D-unit convention).

---

### Task 1: Scaffold `antenna-core`; move `error.rs` + `warnings.rs`

**Goal:** A compiling three-member workspace where the shared error/warning vocabulary lives in `antenna-core`, utoipa derives are feature-gated, and every existing path still resolves.

**Files:**
- Create: `antenna-core/Cargo.toml`, `antenna-core/src/lib.rs`
- Move (git mv): `antenna-model/src/error.rs` → `antenna-core/src/error.rs`; `antenna-model/src/warnings.rs` → `antenna-core/src/warnings.rs`
- Modify: `Cargo.toml` (workspace members), `antenna-model/Cargo.toml` (add antenna-core dep), `antenna-model/src/lib.rs` (mod → re-export), `antenna-core/src/warnings.rs` (2 cfg_attr sites, lines ~67 and ~221)

**Acceptance Criteria:**
- [ ] Workspace has three members; `cargo build -p antenna-core` succeeds with and without `--features openapi`
- [ ] `antenna_model::warnings::WarningCode`, `antenna_model::error::*`, and `crate::warnings`/`crate::error` inside antenna-model all still resolve (compile proves it)
- [ ] Baseline evidence captured in the commit message or task notes: calibrate's normal dep tree currently shows 9 poem/h3o/utoipa/dashmap/lru lines (the "before" state)
- [ ] Default-tier tests and clippy green

**Verify:** `cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace` → all green, ~980 tests

**Steps:**

- [ ] **Step 1: Branch and capture the baseline**

```bash
git checkout -b refactor/d4-antenna-core-split
cargo tree -p calibrate -e normal | grep -E 'poem|h3o|utoipa|dashmap|lru' | sort -u
# expect: multiple lines (baseline 9 matches total) — save this output for the Task 3 before/after comparison
```

- [ ] **Step 2: Write `antenna-core/Cargo.toml`**

```toml
[package]
name = "antenna-core"
version.workspace = true
edition.workspace = true
authors.workspace = true

[features]
# `utoipa::ToSchema` derives on the API-visible types that live here
# (Position3D, CoordinateSystem, ApiWarning, WarningCode). Only the service
# needs OpenAPI schemas; calibrate must not compile utoipa (roadmap D4:
# the CLI stops compiling the web stack).
openapi = ["dep:utoipa"]

[dependencies]
# config + serde_yaml exist solely for error.rs's `impl From<serde_yaml::Error>`
# / `impl From<config::ConfigError>` on ConfigError — the orphan rule pins those
# impls to the crate that defines ConfigError. Neither is web stack. (Discovered
# during execution: the original plan's dep scan missed these qualified paths.)
config = "0.15.25"
crc32fast = "1"
num-complex = "0.4.6"
# default-features off drops the `heapless-cas` default, which pulls heapless 0.7
# → the unmaintained atomic-polyfill (RUSTSEC-2023-0089). We only need std ser/de.
postcard = { version = "1.1.3", default-features = false, features = [
    "use-std",
] }
rayon = "1.12.0"
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.150"
serde_yaml = "0.9.34"
thiserror = "2.0.18"
tracing = "0.1.44"
# Pinned to match antenna-model's exact pin (tests/openapi_spec.rs asserts
# byte-for-byte spec equality; a utoipa upgrade must arrive as a deliberate PR).
utoipa = { version = "=5.5.0", features = ["macros"], optional = true }

[dev-dependencies]
tempfile = "3.27.0"
```

(The full dep list is declared up front even though model/data move in Task 2 — cargo does not warn on not-yet-used deps, and it keeps Task 2's diff purely a code move.)

- [ ] **Step 3: Add the workspace member**

In the root `Cargo.toml`:

```toml
members = [
    "antenna-core",
    "antenna-model",
    "calibrate",
]
```

- [ ] **Step 4: Move the two files and write core's `lib.rs`**

```bash
mkdir -p antenna-core/src
git mv antenna-model/src/error.rs antenna-core/src/error.rs
git mv antenna-model/src/warnings.rs antenna-core/src/warnings.rs
```

`antenna-core/src/lib.rs`:

```rust
//! antenna-core — the physics engine and calibration data model shared by the
//! `antenna-model` REST service and the `calibrate` CLI (roadmap unit D4).
//!
//! This crate deliberately contains no web stack: no poem, no h3o, no tokio.
//! The `openapi` feature gates the `utoipa::ToSchema` derives the service
//! needs for spec generation, so `calibrate` never compiles utoipa.

// Compiler and linter configuration (kept identical to antenna-model's —
// moved code must stay under the same unwrap/expect/panic policy).
#![deny(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
#![allow(missing_docs, missing_debug_implementations)]

pub mod error;
pub mod warnings;

// Re-export the response-warning vocabulary (roadmap C8 stage 3)
pub use warnings::{ApiWarning, WarningCode};

// Re-export error types from error module
pub use error::{
    AntennaModelError, ApiError, ApiResult, ComputationError, ComputationResult, ConfigError,
    ConfigResult, DataError, DataResult, ErrorContext, Result, ValidationError, ValidationResult,
};
```

- [ ] **Step 5: Feature-gate the utoipa derives in `antenna-core/src/warnings.rs`**

Two derive lists carry `utoipa::ToSchema` (currently lines ~60–67 and ~221). For each, delete `utoipa::ToSchema` from the `#[derive(...)]` list and add a `cfg_attr` line directly above it. Pattern (apply to both sites, keeping each site's other derives exactly as they are):

```rust
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
```

Do not touch anything else in the file — serde attributes, `WarningCode::ALL`, message constants all move verbatim.

- [ ] **Step 6: Rewire `antenna-model`**

`antenna-model/Cargo.toml`, top of `[dependencies]`:

```toml
antenna-core = { path = "../antenna-core", features = ["openapi"] }
```

`antenna-model/src/lib.rs`: delete the lines `pub mod error;` and `pub mod warnings;` and add in their place:

```rust
// The shared error/warning vocabulary lives in `antenna-core` (roadmap D4);
// re-export the modules so every existing `antenna_model::{error,warnings}::…`
// path — and `crate::…` within this crate — keeps resolving unchanged.
pub use antenna_core::{error, warnings};
```

Leave the existing convenience re-exports (`pub use warnings::{ApiWarning, WarningCode};`, `pub use error::{…};`) exactly as they are — they resolve through the re-exported modules.

- [ ] **Step 7: Verify and commit**

```bash
cargo build -p antenna-core
cargo build -p antenna-core --features openapi
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
```

Expected: all green (~980 tests, ~86 s). Then:

```bash
git add -A
git commit -m "refactor(D4): extract antenna-core; move error.rs and warnings.rs

Baseline: cargo tree -p calibrate -e normal shows 9 poem/h3o/utoipa/dashmap/lru
lines before the split.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Move `model/`, `data/{types,loader}.rs`, and `Position3D` into core

**Goal:** The entire physics engine and the artifact data layer live in `antenna-core`; antenna-model re-exports them at their old paths; the generated OpenAPI spec is byte-identical.

**Files:**
- Move (git mv): `antenna-model/src/model/` (whole directory, 14 files) → `antenna-core/src/model/`; `antenna-model/src/data/types.rs` → `antenna-core/src/data/types.rs`; `antenna-model/src/data/loader.rs` → `antenna-core/src/data/loader.rs`
- Create: `antenna-core/src/data/mod.rs`
- Modify: `antenna-core/src/lib.rs` (declare data/model), `antenna-model/src/lib.rs` (model → re-export), `antenna-model/src/data/mod.rs` (hybrid: re-export core + keep repository), `antenna-model/src/api/schemas.rs` (Position3D/CoordinateSystem move out, re-export in), `antenna-core/src/model/coordinates_3d.rs` (receives Position3D + CoordinateSystem), `antenna-core/src/model/pattern.rs` (2 doc links to `crate::service` become plain text), doctest paths in moved files (`antenna_model::` → `antenna_core::`)

**Acceptance Criteria:**
- [ ] `model/` and `data/{types,loader}` compile inside antenna-core; `crate::model`, `crate::data`, `crate::error`, `crate::warnings` all resolve *within core* without edits (this is why model+data move together)
- [ ] `antenna_model::model::…`, `antenna_model::data::…`, `antenna_model::api::schemas::Position3D` all still resolve (compile + tests prove it)
- [ ] `grep -rn 'crate::service\|crate::api\|crate::config' antenna-core/src` → no hits
- [ ] `tests/openapi_spec.rs` (byte-for-byte spec equality) passes — proves the Position3D/CoordinateSystem/WarningCode schema output is unchanged
- [ ] Doctests pass workspace-wide (`cargo test --doc --workspace`)
- [ ] Default-tier tests + clippy green; no test value changed

**Verify:** `cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace && cargo test --doc --workspace` → all green

**Steps:**

- [ ] **Step 1: Move the files**

```bash
git mv antenna-model/src/model antenna-core/src/model
mkdir -p antenna-core/src/data
git mv antenna-model/src/data/types.rs antenna-core/src/data/types.rs
git mv antenna-model/src/data/loader.rs antenna-core/src/data/loader.rs
```

- [ ] **Step 2: Write `antenna-core/src/data/mod.rs`**

```rust
//! Calibration artifact data structures and the ANTC loader.
//!
//! The service-side repository (antennas.yaml loading, caching) stays in the
//! `antenna-model` crate (`data::repository`) — it depends on the service
//! configuration system. This module owns the artifact types and the loader
//! shared with `calibrate`.

pub mod loader;
pub mod types;

// Re-export commonly used types for convenience
pub use types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, BSplineModel4DBuilder,
    CalibrationMetadata, CalibrationMetadataBuilder, ValidationError, ValidityRanges,
    ValidityRangesBuilder,
};
```

- [ ] **Step 3: Extend `antenna-core/src/lib.rs`**

Add below the existing `pub mod` lines (keep alphabetical order):

```rust
pub mod data;
pub mod model;
```

and with the other convenience re-exports:

```rust
// Re-export commonly used types for convenience
pub use data::{AntennaCalibration, BSplineModel4D, CalibrationMetadata, ValidityRanges};
```

- [ ] **Step 4: Move `CoordinateSystem` + `Position3D` into `antenna-core/src/model/coordinates_3d.rs`**

In `antenna-model/src/api/schemas.rs`, locate and **cut**:
- the `CoordinateSystem` enum with its doc comments and serde attributes (currently lines ~32–41),
- the `Position3D` struct with its full doc comment block (the doc comment IS the generated OpenAPI description — move it byte-identical) and its single `impl Position3D` block (starts line ~108; find its end with `grep -n 'impl Position3D' antenna-model/src/api/schemas.rs` and brace-matching).

In `antenna-core/src/model/coordinates_3d.rs`, replace the line `use crate::api::schemas::Position3D;` (line 43) with the pasted definitions placed after the file's imports, adding:

```rust
use serde::{Deserialize, Serialize};
```

(only if not already imported). On both moved types, feature-gate the utoipa derive exactly as in Task 1 Step 5:

```rust
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordinateSystem {
```

```rust
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Position3D {
```

Back in `antenna-model/src/api/schemas.rs`, at the spot where the types were cut, add:

```rust
// Position3D and CoordinateSystem live with the coordinate math in
// `antenna-core` (roadmap D4); re-exported here so the API schema path —
// and the generated OpenAPI components — are unchanged.
pub use crate::model::coordinates_3d::{CoordinateSystem, Position3D};
```

- [ ] **Step 5: Rewire `antenna-model`'s lib.rs and data module**

`antenna-model/src/lib.rs`: delete `pub mod model;` and fold it into the Task 1 re-export line:

```rust
pub use antenna_core::{error, model, warnings};
```

Rewrite `antenna-model/src/data/mod.rs` in full:

```rust
//! Data management module for antenna calibration.
//!
//! The artifact types and ANTC loader live in `antenna-core` (roadmap D4) and
//! are re-exported here so existing `antenna_model::data::…` paths keep
//! resolving. The service-side repository stays local — it depends on the
//! service configuration system.

pub use antenna_core::data::{loader, types};

pub mod repository;

// Re-export commonly used types for convenience
pub use types::{
    AntennaCalibration, AntennaCalibrationBuilder, BSplineModel4D, BSplineModel4DBuilder,
    CalibrationMetadata, CalibrationMetadataBuilder, ValidationError, ValidityRanges,
    ValidityRangesBuilder,
};

// Re-export repository for easy access
pub use repository::CalibrationRepository;
```

- [ ] **Step 6: Fix doctest paths and cross-crate doc links in the moved files**

```bash
grep -rl 'antenna_model::' antenna-core/src | xargs sed -i '' 's/antenna_model::/antenna_core::/g'
```

(Known files: `model/pattern.rs`, `model/illumination.rs`, `model/integration.rs`, `data/loader.rs`.)

In `antenna-core/src/model/pattern.rs`, two intra-doc links target the service layer that now lives in a downstream crate and would be unresolved in core's rustdoc (lines ~37 and ~74). Rewrite them as plain code text, e.g.:

- `[`crate::service::CachedGain`]` → `` `CachedGain` (in `antenna-model`'s service layer) ``
- `[`crate::service::evaluator::ray_trace_stub_warning`]` → `` `ray_trace_stub_warning` (in `antenna-model`'s `service::evaluator`) ``

Then confirm no stale cross-layer references remain:

```bash
grep -rn 'crate::service\|crate::api\|crate::config' antenna-core/src
# expect: no output
```

- [ ] **Step 7: Verify and commit**

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo nextest run -p antenna-model -E 'binary(openapi_spec)'
cargo test --doc --workspace
```

Expected: all green; the openapi_spec byte-equality test passing proves the schema output did not move. Then:

```bash
git add -A
git commit -m "refactor(D4): move the physics model and artifact data layer into antenna-core

Pure move: model/ (14 files), data/{types,loader}.rs, and Position3D/
CoordinateSystem (into coordinates_3d.rs, feature-gated utoipa derives).
antenna-model re-exports everything at its old paths; openapi.yaml is
byte-identical.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Repoint `calibrate` at `antenna-core`; prune dead deps

**Goal:** `calibrate`'s production dependency graph contains no poem/h3o/utoipa/dashmap/lru; the service-serving e2e tests keep running via a dev-dependency; dead deps leave antenna-model.

**Files:**
- Modify: `calibrate/Cargo.toml` (dep swap + dev-dep), `calibrate/src/*.rs` — mechanical `antenna_model::` → `antenna_core::` in exactly these 10 files: `antenna_config.rs`, `artifact_export.rs`, `bin/cr159703_grid.rs`, `boresight_calibration.rs`, `correction_surface.rs`, `design_specs_loader.rs`, `frequency_correction.rs`, `main.rs`, `parameter_tuner.rs`, `sidecar.rs`
- Modify: `antenna-model/Cargo.toml` (remove `ndarray`; remove `num-complex`/`rayon`/`crc32fast` if grep confirms unused after the move)
- Delete: `calibrate/src/mod.rs` (orphaned file beside lib.rs — never part of the module tree; deletion sanctioned by the roadmap hygiene note "delete there or on the next touch of the crate")
- NOT modified: `calibrate/tests/**` — they import `antenna_model::{api, service, data, model, warnings}` through the dev-dependency and keep working unchanged

**Acceptance Criteria:**
- [ ] `cargo tree -p calibrate -e normal | grep -E 'poem|h3o|utoipa|dashmap'` → empty, and the only `lru` remaining traces to `aws-sdk-s3` (`cargo tree -p calibrate -i lru@0.16.4`) — D4's exit criterion. **Corrected during execution:** the baseline 9 lines included `lru v0.16.4`, which reaches calibrate through its own `aws-sdk-s3` dependency, not through antenna-model. It gets the same carve-out `tokio` does. The web-stack `lru v0.18.1` (via antenna-model) must be gone. Net: 9 → 1, all 8 web-stack lines eliminated plus the duplicate `lru`.
- [ ] `cargo tree -p antenna-core -e normal | grep -E 'poem|h3o|tokio|utoipa'` → empty
- [ ] `ndarray` removed from antenna-model (verified dead: zero grep hits); each of `num-complex`, `rayon`, `crc32fast` removed iff `grep -rn <name> antenna-model/src antenna-model/tests antenna-model/benches` is empty (`postcard` stays — `data/repository.rs` uses it)
- [ ] `calibrate/src/mod.rs` deleted; `grep -rn 'antenna_model' calibrate/src` → no code references (Cargo.toml comment updated)
- [ ] Full-tier suite green (both tiers — the calibrate e2e binaries are slow-tier and are exactly the tests this task could break)

**Verify:** `cargo tree -p calibrate -e normal | grep -cE 'poem|h3o|utoipa|dashmap|lru'` → `0`, then `RUST_MIN_STACK=16777216 cargo nextest run --workspace --profile full` → all green

**Steps:**

- [ ] **Step 1: Swap the dependency**

In `calibrate/Cargo.toml`, replace:

```toml
# Antenna model dependency for physics engine
antenna-model = { path = "../antenna-model" }
```

with:

```toml
# Physics engine + artifact types. Deliberately antenna-core, not
# antenna-model: the CLI build must not compile the web stack (roadmap D4).
antenna-core = { path = "../antenna-core" }
```

and add to `[dev-dependencies]`:

```toml
# Test-only: the e2e tests deliberately serve generated artifacts through the
# real service loader/evaluator (`service::compute_gain_from_request`) — the
# served path is what caught C13. Dev-only so the CLI build stays web-free.
antenna-model = { path = "../antenna-model" }
```

- [ ] **Step 2: Mechanical import rename in calibrate/src only**

```bash
grep -rl 'antenna_model::' calibrate/src | xargs sed -i '' 's/antenna_model::/antenna_core::/g'
grep -rn 'antenna_model' calibrate/src
# expect: no remaining code references (comments mentioning the service by
# crate name may stay if they are about the service; update any that describe
# the dependency)
```

Do NOT touch `calibrate/tests/` — those files intentionally exercise antenna-model's service layer via the dev-dependency.

- [ ] **Step 3: Delete the orphaned `calibrate/src/mod.rs`**

```bash
git rm calibrate/src/mod.rs
```

(A `mod.rs` beside `lib.rs` at a crate src root is never part of any module tree; rustc has never compiled it. Verified: no `#[path]` or `mod` declaration references it.)

- [ ] **Step 4: Prune dead deps from `antenna-model/Cargo.toml`**

```bash
for c in ndarray num_complex rayon crc32fast; do
  echo "== $c =="
  grep -rn "$c" antenna-model/src antenna-model/tests antenna-model/benches || echo "UNUSED — remove from Cargo.toml"
done
```

Remove each dependency reported UNUSED (`ndarray` is already confirmed at zero hits; expect `num-complex`, `rayon`, `crc32fast` to also be movable/removable — they were only used by the moved model/data files; keep any with hits). Keep `postcard` (used by `data/repository.rs`).

- [ ] **Step 5: Verify the exit criterion and the full suite; commit**

```bash
cargo tree -p calibrate -e normal | grep -E 'poem|h3o|utoipa|dashmap|lru'
# expect: NO output for poem|h3o|utoipa|dashmap. One `lru` line remains and is
# correct — `cargo tree -p calibrate -e normal -i lru@0.16.4` shows it arriving via
# calibrate's own aws-sdk-s3, the same carve-out tokio gets. Baseline 9 -> 1.
cargo tree -p antenna-core -e normal | grep -E 'poem|h3o|tokio|utoipa'
# expect: NO output
cargo clippy --workspace --all-targets -- -D warnings
RUST_MIN_STACK=16777216 cargo nextest run --workspace --profile full
cargo test --doc --workspace
```

Expected: dep-tree greps empty; full suite green (both tiers; slow tier includes the calibrate e2e binaries this task touches most directly). Then:

```bash
git add -A
git commit -m "refactor(D4): calibrate depends on antenna-core; web stack out of the CLI build

cargo tree -p calibrate -e normal: 9 poem/h3o/utoipa/dashmap/lru lines -> 1
(the survivor is lru via calibrate's own aws-sdk-s3; all web-stack lines gone).
antenna-model becomes a dev-dependency (the e2e tests serve artifacts through
the real service path, which is what caught C13). Orphaned calibrate/src/mod.rs
deleted per the roadmap hygiene note; dead deps pruned from antenna-model.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Truth the docs; final gate

**Goal:** CLAUDE.md and architecture.md describe the three-crate workspace accurately (D4 exit criterion); the roadmap records the unit as done; the full CI-equivalent gate passes.

**Files:**
- Modify: `CLAUDE.md` — workspace-structure diagram (add `antenna-core/` with its module list; correct `antenna-model/` to api/service/config/data-repository); the "Key Physics Modules" heading path (`antenna-model/src/model/` → `antenna-core/src/model/`); the data-layer path reference (`src/data/types.rs` → `antenna-core/src/data/types.rs`); the Commands section's workspace-member example (add `cargo nextest run -p antenna-core`)
- Modify: `docs/architecture.md` — module lists / workspace layout to match `ls` reality for all three crates
- Modify: `docs/roadmap-2026-07.md` — decision-register D4 row → Decided + executed with date; the Phase 4 goal row's "crate split done" gets its ✅ with a one-line summary; strike/annotate the "Structural debt" paragraph's calibrate-compiles-the-web-stack item
- Modify: `docs/roadmap-2026-07-work-units.md` — D4 unit gets a "Delivered" note: what moved, the utoipa `openapi` feature, Position3D's new home, repository.rs staying, antenna-model as calibrate dev-dep, the 9→0 dep-tree measurement, ndarray finding (already unified; dead dep removed)

**Acceptance Criteria:**
- [ ] CLAUDE.md workspace diagram and module paths match `ls` reality (spot-check: every path named in the edited sections exists)
- [ ] architecture.md module lists match `ls antenna-core/src/model antenna-core/src/data antenna-model/src` reality
- [ ] Roadmap D4 rows in both files record done-state with date and the dep-tree measurement
- [ ] `./scripts/check.sh` passes end to end (fmt, clippy -D warnings, full-tier nextest, doctests, audit)

**Verify:** `./scripts/check.sh` → "All gate checks passed."

**Steps:**

- [ ] **Step 1: Update CLAUDE.md's workspace diagram**

Replace the current workspace-structure block with (keep the existing per-file annotations for calibrate; trim antenna-model's to what remains):

```
antenna-model/           # Cargo workspace root
├── antenna-core/       # Shared physics engine + calibration data model (roadmap D4)
│   └── src/
│       ├── model/      # Physics engine (coordinates, geometry, phase, pattern, integration, bessel, fft)
│       ├── data/       # Calibration artifact types (types.rs) + ANTC loader (loader.rs)
│       ├── error.rs    # Shared error vocabulary
│       └── warnings.rs # Typed response-warning vocabulary (WarningCode)
├── antenna-model/      # REST API service binary
│   └── src/
│       ├── api/        # REST layer (poem framework); re-exports Position3D from core
│       ├── service/    # Business logic (evaluator, batch, validator)
│       ├── data/       # Service-side repository (antennas.yaml) — artifact types re-exported from core
│       └── config/     # Configuration system
├── calibrate/          # CLI calibration tool binary (depends on antenna-core only;
│   │                   # antenna-model is a dev-dependency for the served-path e2e tests)
│   └── src/ …          # (unchanged listing)
└── calibration_data/   # (unchanged)
```

Also update: the "Key Physics Modules (`antenna-model/src/model/`)" heading to `antenna-core/src/model/`, the Data Layer bullet's `src/data/types.rs` mention, and add `cargo nextest run -p antenna-core` beside the existing `-p` examples.

- [ ] **Step 2: Update docs/architecture.md**

Find the workspace/module layout sections (`grep -n 'src/model\|workspace\|module' docs/architecture.md`) and update them to the three-crate reality. Keep edits surgical — this is a truth pass, not a rewrite.

- [ ] **Step 3: Update both roadmap files' D4 entries**

`docs/roadmap-2026-07.md`: decision-register row D4 status → `**Decided + executed**`, date 2026-08-13, note "three-crate workspace; calibrate's normal dep tree: 9 poem/h3o/utoipa/dashmap/lru lines → 1, the survivor being `lru` via calibrate's own aws-sdk-s3 (all web-stack lines gone); antenna-model stays a **dev**-dependency because the e2e tests serve artifacts through the real service path (C13); ndarray already unified (0.16.1) and removed as a dead dep; utoipa derives behind core's `openapi` feature, whose OFF configuration the gate now compiles because a workspace build unifies it ON". Phase 4 row: mark the "crate split done" fragment ✅ 2026-08-05. Structural-debt paragraph (~line 157): annotate the calibrate-compiles-the-web-stack sentence as resolved by D4.

`docs/roadmap-2026-07-work-units.md` D4 unit (~line 3305): append a `**Delivered 2026-08-05**` paragraph covering: what moved and what stayed (repository.rs with the service; tests/benches unmoved, working via re-exports); Position3D/CoordinateSystem now defined in `antenna-core::model::coordinates_3d`, re-exported by `api::schemas`; the `openapi` feature; antenna-model as calibrate dev-dependency and why (served-path e2e, C13); the 9→0 measurement; the orphan `calibrate/src/mod.rs` deletion; "no test value changed" confirmation.

- [ ] **Step 4: Final gate and commit**

```bash
./scripts/check.sh
```

Expected: "All gate checks passed." Then:

```bash
git add -A
git commit -m "docs(D4): record the antenna-core split in CLAUDE.md, architecture, roadmap

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

Branch is ready for PR (`finishing-a-development-branch` decides merge/PR).
