# Phase 0 — Guardrails Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a regression net (CI + local gate), make CLAUDE.md truthful, and lock the example requests with a deserialization test — the three guardrail units (G1, G2, G3) that every later roadmap unit depends on — then wire the new GitHub remote so CI activates.

**Architecture:** Three self-contained units executed in order (G1 → G2 → G3), then a remote-activation step. G1 adds `.github/workflows/ci.yml` + `scripts/check.sh` running the same four checks (fmt, clippy, test, audit) and fixes the mechanical clippy lints currently on HEAD. G2 is docs-only (CLAUDE.md). G3 fixes four broken example JSON files and adds an integration test that fails on any future drift. A final step adds the `github.com/blstoll/antenna-model` remote and pushes, which activates the ready-to-run CI.

**Tech Stack:** Rust (cargo workspace: `antenna-model`, `calibrate`), GitHub Actions, `cargo fmt`/`clippy`/`test`/`audit`, OpenBLAS (system lib for `calibrate`'s `ndarray-linalg` `openblas-system` feature), serde/serde_json, poem.

**User decisions (already made):**
- "I have created an empty git repository at github.com/blstoll/antenna-model to house this application." → **G1-hosting = GitHub** (decision-register row G1-hosting resolved to GitHub). CI is committed ready-to-activate and activates on remote push.
- Roadmap standing rules apply verbatim (see `docs/roadmap-2026-07-work-units.md` §"Standing rules"). Most relevant here: **G2 is docs-only, zero code changes**; **never touch physics formulas/signs in a non-physics unit**; **openapi.yaml is hand-maintained** (not touched in P0); **exit criteria are the definition of done**.

---

## Ground-truth verification (done 2026-07-08 at HEAD `d65f780`)

These facts were re-verified against the code before writing this plan. Re-verify any file:line before editing.

- **No `.github/` directory exists**; `scripts/` contains only `build-docker.sh`; **no git remote configured** (`git remote -v` empty).
- **`cargo fmt --all -- --check` is CLEAN on HEAD.**
- **`cargo clippy --workspace --all-targets -- -D warnings` FAILS on HEAD** — 3 `clippy::field_reassign_with_default` errors, all in integration test code (safe to fix, no `model/` semantics):
  - `antenna-model/tests/integration/error_tests.rs:76-78`
  - `antenna-model/tests/integration/error_tests.rs:128-131`
  - `antenna-model/tests/integration/resilience_tests.rs:52-55`
- **`calibrate` needs a system OpenBLAS backend** (`ndarray-linalg = { version = "0.16.0", features = ["openblas-system"] }` in `calibrate/Cargo.toml`). Linux CI → `libopenblas-dev` + `gfortran`. macOS local → `LDFLAGS`/`CPPFLAGS` to Homebrew openblas.
- **`Cargo.lock` is gitignored** (`.gitignore` line `Cargo.lock`). This is unusual for a workspace with binaries and hurts CI reproducibility, but committing it is **out of P0 scope** (hygiene, touches roadmap D6). CI regenerates a lock at build time, so `cargo audit` still works. Noted, not changed here.
- **Deleted modules confirmed gone:** `direct_path.rs`, `surface.rs`, `numerical_stability.rs` are absent from `antenna-model/src/model/`.
- **Actual `antenna-model/src/model/` contents:** `coordinates.rs`, `coordinates_3d.rs`, `correction_interpolator.rs`, `edge_cases.rs`, `geometry.rs`, `illumination.rs`, `integration.rs`, `mesh.rs`, `mod.rs`, `pattern.rs`, `phase.rs`, `ray_trace.rs`. Top-level `antenna-model/src/`: `api/`, `config/`, `data/`, `error.rs`, `lib.rs`, `main.rs`, `model/`, `service/`.
- **`implementation-plan.md` marks Sprints 1–7 all "✅ Complete"** (Sprint 5/6/7 at lines 25–27). CLAUDE.md still says "Sprint 5 (of 8)".
- **The 4D B-spline correction is live:** `antenna-model/src/model/correction_interpolator.rs`, applied at `service/evaluator.rs:265-287`.
- **Request schema types** (`antenna-model/src/api/schemas.rs`): `GainRequest` (222), `BatchGainRequest` (372, `evaluations: Vec<GainRequest>`), `HeatmapRequest` (411), `H3LinkBudgetRequest` (569). `vehicle_attitude: Option<[f64; 4]>` on `GainRequest` (276) and `H3LinkBudgetRequest` (623). **`HeatmapRequest` has NO `vehicle_attitude` field.** The doc comment at `schemas.rs:263` states the quaternion order explicitly: **`[w, x, y, z]`** (w-first). No `#[serde(deny_unknown_fields)]` anywhere in schemas.rs.
- **Example request files** in `examples/requests/`: `gain_request.json`, `gain_request_geodetic.json`, `batch_request.json`, `heatmap_request.json` (all four broken), plus five `geo_*.json` (all `GainRequest`-shaped, attitude omitted → already fine). Broken shapes: object-form `{"w":…,"x":…}` in three files; Euler `{"roll_deg":…}` in `gain_request_geodetic.json`.
- **`serde_json` and `serde` are normal `[dependencies]` of `antenna-model`** (available to integration tests). Existing integration tests import via `use antenna_model::api::schemas::{...}`.

---

## File structure

| File | Unit | Create/Modify | Responsibility |
|---|---|---|---|
| `.github/workflows/ci.yml` | G1 | Create | GitHub Actions: fmt, clippy, test, audit jobs |
| `scripts/check.sh` | G1 | Create | Local gate mirroring CI; the pre-push command |
| `antenna-model/tests/integration/error_tests.rs` | G1 | Modify | Fix 2 mechanical clippy lints |
| `antenna-model/tests/integration/resilience_tests.rs` | G1 | Modify | Fix 1 mechanical clippy lint |
| `docs/roadmap-2026-07.md` | G1 | Modify | Mark G1-hosting register row Decided (GitHub) |
| `CLAUDE.md` | G2 | Modify | Make onboarding doc true (docs-only) |
| `examples/requests/gain_request.json` | G3 | Modify | Object → array quaternion |
| `examples/requests/gain_request_geodetic.json` | G3 | Modify | Euler → identity quaternion |
| `examples/requests/batch_request.json` | G3 | Modify | 3× object → array quaternion |
| `examples/requests/heatmap_request.json` | G3 | Modify | Remove non-schema `vehicle_attitude` |
| `examples/{api_requests.json,curl-examples.sh,postman_collection.json,python_examples.py,QUICKSTART.md,README.md}` | G3 | Modify | Fix the same broken shapes for consistency |
| `antenna-model/tests/example_requests_deserialize.rs` | G3 | Create | Directory-iterating drift test |

---

## Task 1 (G1): Stand up CI + local gate, fix HEAD's clippy lints

**Goal:** A committed CI workflow and a local gate script that both run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo audit`, all green on HEAD; the G1-hosting register row marked Decided (GitHub).

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `scripts/check.sh`
- Modify: `antenna-model/tests/integration/error_tests.rs:76-78`, `:128-131`
- Modify: `antenna-model/tests/integration/resilience_tests.rs:52-55`
- Modify: `docs/roadmap-2026-07.md` (G1-hosting row, ~line 164)

**Acceptance Criteria:**
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0 (was failing with 3 errors).
- [ ] `bash scripts/check.sh` runs all four checks and exits 0 on current HEAD.
- [ ] `.github/workflows/ci.yml` defines jobs for fmt, clippy, test, and a non-blocking audit; installs `libopenblas-dev`+`gfortran` before building `calibrate`.
- [ ] `docs/roadmap-2026-07.md` G1-hosting row Status = **Decided**, "Maintainer, 2026-07-08", note "GitHub — github.com/blstoll/antenna-model".
- [ ] No change to any physics formula, sign, coefficient, or **production** (non-`#[cfg(test)]`) code under `antenna-model/src/model/`. Mechanical clippy fixes **inside `#[cfg(test)]` test modules** of model files ARE permitted (maintainer-approved 2026-07-08 — see note below), consistent with the roadmap's "defer only src/model *semantics*" carve-out.

> **Toolchain-drift note (recorded 2026-07-08):** The installed stable toolchain is clippy 1.95.0, which flags **27** mechanical clippy errors on HEAD, not 3. 17 are outside `src/model/`; 10 are inside `#[cfg(test)]` test modules of `coordinates_3d.rs` and `correction_interpolator.rs` (`manual_range_contains`, `identity_op` — plus `needless_update` induced when a 3-field struct gets `..Default::default()`). None touch physics. **Maintainer approved fixing all 27** to get a green gate in one pass. Fix only mechanical lints; never `#[allow]` and never alter a physics formula/sign/coefficient.

**Verify:** `bash scripts/check.sh` → prints each `==>` header and finishes with `All gate checks passed.` (exit 0). Individually: `cargo clippy --workspace --all-targets -- -D warnings` → no output, exit 0.

**Steps:**

- [ ] **Step 1: Fix the three mechanical clippy lints.** These are `field_reassign_with_default` in test code — convert `let mut x = Default::default(); x.a = …;` into struct-literal `..Default::default()` form. Do NOT touch anything under `src/model/`.

  In `antenna-model/tests/integration/error_tests.rs`, replace lines 76-78:
  ```rust
  let mut config = CalibrationConfig::default();
  config.antenna_config_file = corrupted_config.clone();
  config.fail_fast = true;
  ```
  with:
  ```rust
  let config = CalibrationConfig {
      antenna_config_file: corrupted_config.clone(),
      fail_fast: true,
      ..Default::default()
  };
  ```

  In the same file, replace lines 128-131:
  ```rust
  let mut config = CalibrationConfig::default();
  config.antenna_config_file = antenna_config.clone();
  config.data_directory = temp_dir.clone();
  config.fail_fast = false; // Should continue loading other antennas
  ```
  with:
  ```rust
  let config = CalibrationConfig {
      antenna_config_file: antenna_config.clone(),
      data_directory: temp_dir.clone(),
      fail_fast: false, // Should continue loading other antennas
      ..Default::default()
  };
  ```

  In `antenna-model/tests/integration/resilience_tests.rs`, replace lines 52-55:
  ```rust
  let mut config = CalibrationConfig::default();
  config.antenna_config_file = antenna_config.clone();
  config.data_directory = temp_dir.clone();
  config.fail_fast = false; // Should continue loading despite errors
  ```
  with:
  ```rust
  let config = CalibrationConfig {
      antenna_config_file: antenna_config.clone(),
      data_directory: temp_dir.clone(),
      fail_fast: false, // Should continue loading despite errors
      ..Default::default()
  };
  ```

- [ ] **Step 2: Re-run clippy and fix any remaining mechanical lints.** Clippy aborts a target at its first errors, so more may surface after Step 1.
  Run (macOS): `LDFLAGS="-L/opt/homebrew/opt/openblas/lib" CPPFLAGS="-I/opt/homebrew/opt/openblas/include" cargo clippy --workspace --all-targets -- -D warnings`
  Expected: exit 0, no output. If new lints appear: fix only **mechanical** ones (style/redundancy). If any lint would require editing `antenna-model/src/model/` semantics, STOP, `#[allow(...)]` it locally is NOT acceptable — instead list it in the PR description as deferred and leave the clippy job as-is (it will fail until addressed by a physics-aware unit). Given the audit found only the 3 above, expect none.

- [ ] **Step 3: Create `scripts/check.sh`** (the documented local gate):
  ```bash
  #!/usr/bin/env bash
  # Local CI gate — mirrors .github/workflows/ci.yml. Run before pushing.
  # Exits nonzero on the first failing check.
  set -euo pipefail

  # macOS OpenBLAS linking for the calibrate crate (ndarray-linalg openblas-system).
  # No-op on Linux, where libopenblas-dev provides the system paths.
  if [[ "$(uname)" == "Darwin" ]]; then
    export LDFLAGS="${LDFLAGS:-} -L/opt/homebrew/opt/openblas/lib"
    export CPPFLAGS="${CPPFLAGS:-} -I/opt/homebrew/opt/openblas/include"
  fi

  echo "==> cargo fmt --all -- --check"
  cargo fmt --all -- --check

  echo "==> cargo clippy --workspace --all-targets -- -D warnings"
  cargo clippy --workspace --all-targets -- -D warnings

  echo "==> cargo test --workspace"
  cargo test --workspace

  echo "==> cargo audit (non-blocking)"
  if command -v cargo-audit >/dev/null 2>&1; then
    cargo audit || echo "WARNING: cargo audit reported issues (non-blocking)"
  else
    echo "SKIP: cargo-audit not installed (run: cargo install cargo-audit)"
  fi

  echo "All gate checks passed."
  ```
  Then: `chmod +x scripts/check.sh`

- [ ] **Step 4: Create `.github/workflows/ci.yml`:**
  ```yaml
  name: CI

  on:
    push:
    pull_request:

  env:
    CARGO_TERM_COLOR: always

  jobs:
    fmt:
      name: rustfmt
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
          with:
            components: rustfmt
        - run: cargo fmt --all -- --check

    clippy-test:
      name: clippy + test
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - name: Install OpenBLAS (calibrate uses ndarray-linalg openblas-system)
          run: sudo apt-get update && sudo apt-get install -y libopenblas-dev gfortran pkg-config
        - uses: dtolnay/rust-toolchain@stable
          with:
            components: clippy
        - uses: Swatinem/rust-cache@v2
        - name: Clippy
          run: cargo clippy --workspace --all-targets -- -D warnings
        - name: Test
          run: cargo test --workspace

    audit:
      name: cargo audit (non-blocking)
      runs-on: ubuntu-latest
      continue-on-error: true  # advisories are tracked, not gating, until the allowlist policy is set
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - name: Install cargo-audit
          run: cargo install cargo-audit --locked
        - name: Audit
          run: cargo audit
  ```
  Note on the audit allowlist: the roadmap asks for a "tracked allowlist". `continue-on-error: true` makes the job non-blocking now. When a specific advisory is reviewed and accepted, add it via `cargo audit --ignore RUSTSEC-XXXX-XXXX` in the Audit step (keep each ignore commented with the reason). Do not silence audit wholesale.

- [ ] **Step 5: Mark the G1-hosting decision-register row Decided** in `docs/roadmap-2026-07.md`. Find the row (grep `G1-hosting`, ~line 164) and change its `Status` cell from `Open` and `Decided by / date` cell from `—` to:
  - Status: `**Decided**`
  - Decided by / date: `Maintainer, 2026-07-08`
  - Append to the Recommended-default cell (or Question cell, keep the table valid): note `Repo created at github.com/blstoll/antenna-model; CI committed ready-to-activate.`

- [ ] **Step 6: Run the gate and confirm green.**
  Run: `bash scripts/check.sh`
  Expected: all four `==>` sections pass; final line `All gate checks passed.`; exit 0. (Install cargo-audit first if missing: `cargo install cargo-audit --locked`.)

- [ ] **Step 7: Commit.**
  ```bash
  git add .github/workflows/ci.yml scripts/check.sh \
    antenna-model/tests/integration/error_tests.rs \
    antenna-model/tests/integration/resilience_tests.rs \
    docs/roadmap-2026-07.md
  git commit -m "ci: add GitHub Actions workflow + local gate; fix HEAD clippy lints (G1)"
  ```

---

## Task 2 (G2): Make CLAUDE.md true

**Goal:** CLAUDE.md no longer misleads a future coding agent — correct sprint status, the live B-spline correction, deleted-module references, `antennas.toml`→`.yaml`, the property-test claim, the precomputed-artifact claim, and the module map. **Docs-only; zero code changes.**

**Files:**
- Modify: `CLAUDE.md`

**Acceptance Criteria:**
- [ ] No claim that B-spline correction is unimplemented; sprint status reflects Sprints 5–7 complete (per `implementation-plan.md`).
- [ ] No references to `direct_path.rs`, `surface.rs`, or `numerical_stability.rs`.
- [ ] `antennas.toml` → `antennas.yaml` everywhere it appears.
- [ ] The property-based-tests line annotated "planned — see roadmap unit D7".
- [ ] The precomputed-`.bin`-artifacts claim corrected (no `.bin` files ship; all `antennas.yaml` entries disabled; see D9).
- [ ] The physics-module map matches `ls antenna-model/src/model/` (adds `coordinates_3d.rs`, `correction_interpolator.rs`; removes deleted files).
- [ ] Only `CLAUDE.md` changed (do not touch `architecture.md`/design doc — that's unit D5).

**Verify:** `git diff --name-only` lists only `CLAUDE.md`; then `grep -nE 'antennas\.toml|direct_path\.rs|surface\.rs|numerical_stability\.rs|not yet implemented|Sprint 5\b' CLAUDE.md` returns no live/erroneous hits (only historically-accurate mentions if any remain intentionally).

**Steps:**

- [ ] **Step 1: Correction surface is live (line ~125).** Replace:
  `   - Interpolate **correction surface** (B-spline, not yet implemented in Sprint 5)`
  with:
  `   - Interpolate **correction surface** (4D B-spline — implemented and live in `model/correction_interpolator.rs`, applied in `service/evaluator.rs`)`

- [ ] **Step 2: Sprint status (line 13 and the "Current Sprint Status" section, lines ~225-238).**
  Replace line 13:
  `The system is in **Sprint 5** (of 8) - Core API endpoints are being implemented.`
  with:
  `Sprints 1–7 of 8 are complete (see `docs/implementation-plan.md`): physics engine, calibration tool, core + advanced REST endpoints, partial-calibration support, and boresight calibration are all built and tested.`

  Replace the whole block from `## Current Sprint Status (Sprint 5)` through the "Key Integration Point" paragraph (lines ~225-238) with a truthful summary:
  ```markdown
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
  ```

- [ ] **Step 3: Deleted module references.**
  - Line ~142 (Key Physics Modules): change
    `- **`edge_cases.rs`, `direct_path.rs`, `ray_trace.rs`** - Special case handling`
    to
    `- **`edge_cases.rs`, `ray_trace.rs`** - Special case / large-feed-offset handling`
  - Line ~143: change
    `- **`surface.rs`, `mesh.rs`** - Surface RMS (Ruze equation), mesh transparency`
    to
    `- **`mesh.rs`** - Mesh transparency (wire-mesh reflection efficiency). Surface RMS / Ruze efficiency lives in `pattern.rs`.`
  - Line ~246 (Common Pitfalls #3, Phase Wrapping): remove the `model/numerical_stability.rs` pointer. First `grep -rn 'wrap\|rem_euclid\|2.0 \* PI\|TAU' antenna-model/src/model/phase.rs` to find where phase wrapping now lives; if found there, repoint to `model/phase.rs`; if no single home, change the sentence to: `Phase functions must handle 2π wrapping correctly (see the phase accumulation in `model/phase.rs`).`

- [ ] **Step 4: Add the two missing model modules to the map.** After the `illumination.rs` / `integration.rs` / `pattern.rs` bullets in "Key Physics Modules", ensure these appear:
  - `- **`coordinates_3d.rs`** - 3D position → antenna-frame direction transforms (ECEF/geodetic vehicle geometry)`
  - `- **`correction_interpolator.rs`** - 4D B-spline evaluation of the residual correction surface`

- [ ] **Step 5: `antennas.toml` → `antennas.yaml`** (three sites): the Workspace Structure comment (line ~97), and the Configuration System bullets (lines ~175-176). Change `calibration_data/antennas.toml` → `calibration_data/antennas.yaml`.

- [ ] **Step 6: Precomputed-artifact claim.**
  - Line ~97 workspace comment: change `# Pre-computed calibration artifacts (*.bin, antennas.toml)` to `# Calibration config (antennas.yaml) + generated *.bin artifacts (none checked in; see roadmap D9)`.
  - Line ~132 (Data Layer): change `Loaded from binary `.bin` files at startup` to `Loaded at startup from `.bin` artifacts referenced by `antennas.yaml`. **No `.bin` artifacts ship in-repo; all `antennas.yaml` entries are `enabled: false` today — see roadmap unit D9.**`
  - Line ~176 (Configuration System): change `Binary `.bin` files referenced by `antennas.toml`` to `Binary `.bin` artifacts referenced by `antennas.yaml` (generated locally; none committed — see D9)`.

- [ ] **Step 7: Property-based-tests claim (line ~214).** Change
  `- Property-based tests for coordinate transforms (round-trip accuracy)`
  to
  `- Property-based tests for coordinate transforms (round-trip accuracy) — *planned; not yet implemented, see roadmap unit D7*`

- [ ] **Step 8: Verify docs-only and grep clean.**
  Run: `git diff --name-only` → expect only `CLAUDE.md`.
  Run: `grep -nE 'antennas\.toml|direct_path\.rs|surface\.rs|numerical_stability\.rs' CLAUDE.md` → expect no hits.

- [ ] **Step 9: Commit.**
  ```bash
  git add CLAUDE.md
  git commit -m "docs: make CLAUDE.md true — sprint status, live B-spline, module map, antennas.yaml (G2)"
  ```

---

## Task 3 (G3): Fix broken example requests + lock them with a test

**Goal:** Every file in `examples/requests/` deserializes into its schema type, and an integration test in the CI gate fails if any example drifts again. Fix the same broken shapes in the other `examples/` docs for consistency.

**Files:**
- Modify: `examples/requests/gain_request.json`, `gain_request_geodetic.json`, `batch_request.json`, `heatmap_request.json`
- Modify: `examples/api_requests.json`, `examples/curl-examples.sh`, `examples/postman_collection.json`, `examples/python_examples.py`, `examples/QUICKSTART.md`, `examples/README.md` (same broken shapes; grep them)
- Create: `antenna-model/tests/example_requests_deserialize.rs`

**Acceptance Criteria:**
- [ ] The four broken `examples/requests/*.json` deserialize into `GainRequest` / `BatchGainRequest` / `HeatmapRequest` respectively.
- [ ] `vehicle_attitude` uses the array form `[w, x, y, z]` (w-first, per `schemas.rs:263`); the Euler example becomes the identity quaternion `[1.0, 0.0, 0.0, 0.0]`.
- [ ] `heatmap_request.json` no longer carries a `vehicle_attitude` key (`HeatmapRequest` has no such field).
- [ ] `antenna-model/tests/example_requests_deserialize.rs` iterates `examples/requests/`, deserializes each against an explicit filename→type map, and panics on any unmapped file.
- [ ] The test passes and runs under `cargo test --workspace` (hence in CI + `scripts/check.sh`).
- [ ] `grep -rn '"w":\|roll_deg' examples/` shows no remaining quaternion-object or Euler-attitude shapes.
- [ ] **No change to `antenna-model/src/api/schemas.rs`** (fix examples to the schema, never the reverse).

**Verify:** `cargo test -p antenna-model --test example_requests_deserialize` → `test every_example_request_deserializes ... ok`. Then `grep -rn '"w":\|roll_deg' examples/` → no attitude hits.

**Steps:**

- [ ] **Step 1: Fix `examples/requests/gain_request.json`.** Replace the `vehicle_attitude` object:
  ```json
    "vehicle_attitude": {
      "w": 1.0,
      "x": 0.0,
      "y": 0.0,
      "z": 0.0
    },
  ```
  with the array form:
  ```json
    "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
  ```

- [ ] **Step 2: Fix `examples/requests/gain_request_geodetic.json`.** The Euler form `{"roll_deg":0,"pitch_deg":0,"yaw_deg":0}` is identity → replace the whole block:
  ```json
    "vehicle_attitude": {
      "roll_deg": 0.0,
      "pitch_deg": 0.0,
      "yaw_deg": 0.0
    },
  ```
  with:
  ```json
    "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
  ```
  (Zero roll/pitch/yaw = identity rotation = quaternion `[1,0,0,0]`. If any future example uses non-zero Euler angles, convert with w=cos(½·combined) etc.; here it is identity, so no trig needed.)

- [ ] **Step 3: Fix `examples/requests/batch_request.json`.** It has three `vehicle_attitude` objects (all `{w:1,x:0,y:0,z:0}`). Replace each with `[1.0, 0.0, 0.0, 0.0]`, same as Step 1.

- [ ] **Step 4: Fix `examples/requests/heatmap_request.json`.** `HeatmapRequest` has **no** `vehicle_attitude` field (confirmed: `schemas.rs:411-443`). The current object is silently ignored (no `deny_unknown_fields`) but is misleading. **Delete** the entire `vehicle_attitude` block:
  ```json
    "vehicle_attitude": {
      "w": 1.0,
      "x": 0.0,
      "y": 0.0,
      "z": 0.0
    },
  ```
  (Remove the key and its trailing comma cleanly so the JSON stays valid.)

- [ ] **Step 5: Fix the same shapes elsewhere in `examples/` for consistency.** These are not covered by the test (which only iterates `examples/requests/`) but the roadmap requires consistent fixes.
  Run: `grep -rn '"w":\|roll_deg' examples/` and update each hit in `api_requests.json`, `curl-examples.sh`, `postman_collection.json`, `python_examples.py`, `QUICKSTART.md`, `README.md` to the array quaternion `[1.0, 0.0, 0.0, 0.0]` (or the correct array for any non-identity value present — read the value and convert faithfully). Leave any occurrence of `"w":`/`roll_deg` that is NOT a `vehicle_attitude` value untouched (e.g. unrelated prose).

- [ ] **Step 6: Write the failing test** `antenna-model/tests/example_requests_deserialize.rs`:
  ```rust
  //! Guards that every example request in `examples/requests/` deserializes into
  //! its documented schema type — prevents doc/example drift (roadmap unit G3).

  use antenna_model::api::schemas::{BatchGainRequest, GainRequest, HeatmapRequest};
  use std::path::Path;

  fn assert_parses<T: serde::de::DeserializeOwned>(path: &Path) {
      let text = std::fs::read_to_string(path)
          .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
      if let Err(e) = serde_json::from_str::<T>(&text) {
          panic!(
              "{} did not deserialize into {}: {e}",
              path.display(),
              std::any::type_name::<T>()
          );
      }
  }

  #[test]
  fn every_example_request_deserializes() {
      let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/requests");
      let mut checked = 0usize;

      for entry in std::fs::read_dir(&dir).expect("examples/requests must exist") {
          let path = entry.expect("readable dir entry").path();
          if path.extension().and_then(|e| e.to_str()) != Some("json") {
              continue;
          }
          let name = path.file_name().unwrap().to_str().unwrap().to_string();
          match name.as_str() {
              "batch_request.json" => assert_parses::<BatchGainRequest>(&path),
              "heatmap_request.json" => assert_parses::<HeatmapRequest>(&path),
              // All single-gain examples, including every geo_*.json fixture.
              n if n.starts_with("gain_request") || n.starts_with("geo_") => {
                  assert_parses::<GainRequest>(&path)
              }
              other => panic!(
                  "no schema mapping for examples/requests/{other} — \
                   add it to every_example_request_deserializes"
              ),
          }
          checked += 1;
      }

      assert!(checked >= 9, "expected to check all example requests, only saw {checked}");
  }
  ```
  (If the test crate cannot resolve `serde_json`/`serde`, add `serde_json = "1"` and `serde = "1"` to `antenna-model`'s `[dev-dependencies]` — but they are already normal `[dependencies]`, which integration tests inherit, so this should not be needed.)

- [ ] **Step 7: Run the test — expect it to catch anything still broken, then pass.**
  Run: `cargo test -p antenna-model --test example_requests_deserialize`
  Expected: `test every_example_request_deserializes ... ok`. If it fails naming a file, that example still has a bad shape — fix it and re-run.

- [ ] **Step 8: Confirm no attitude shapes remain and the full gate is green.**
  Run: `grep -rn '"w":\|roll_deg' examples/` → no `vehicle_attitude` hits.
  Run: `cargo test --workspace` (macOS: prefix with the LDFLAGS/CPPFLAGS) → all pass.

- [ ] **Step 9: Commit.**
  ```bash
  git add examples/ antenna-model/tests/example_requests_deserialize.rs
  git commit -m "test: fix broken example requests + lock with deserialization drift test (G3)"
  ```

---

## Task 4: Wire the GitHub remote and activate CI

**Goal:** Connect the local repo to `github.com/blstoll/antenna-model` and push so the committed CI workflow runs for the first time against the full Phase-0 result.

> **This step is outward-facing (publishes the codebase to GitHub). Confirm with the maintainer before pushing** — which branch to push (`main` vs the current `fix/review-findings-2026-07`), and whether the repo should be public/private is already set on GitHub. Do not force-push.

**Files:** none (git remote + push only).

**Acceptance Criteria:**
- [ ] `git remote -v` shows `origin` → `github.com/blstoll/antenna-model`.
- [ ] The chosen branch is pushed; the Actions tab shows a CI run triggered by the push.
- [ ] The first CI run's `fmt` and `clippy + test` jobs are green (audit may be non-green but non-blocking).

**Verify:** `gh run list --limit 1` (or the Actions tab) shows a run for the pushed commit; `gh run view <id>` shows `clippy + test` succeeded.

**Steps:**

- [ ] **Step 1: Add the remote.**
  ```bash
  git remote add origin https://github.com/blstoll/antenna-model.git
  git remote -v
  ```

- [ ] **Step 2: Confirm branch strategy with the maintainer, then push.** Tasks 1–3 committed on `fix/review-findings-2026-07`. Typical flow: land Phase 0 on `main` (or open a PR). Example (push current branch and set upstream):
  ```bash
  git push -u origin fix/review-findings-2026-07
  ```
  Or, if pushing `main` first: `git push -u origin main` then the feature branch.

- [ ] **Step 3: Watch the first CI run.**
  ```bash
  gh run watch    # or: gh run list --limit 1 && gh run view <id> --log
  ```
  Expected: `fmt` and `clippy + test` succeed. If a job fails on something the local gate did not catch (e.g. a Linux/OpenBLAS-specific issue), reproduce with `scripts/check.sh`, fix, and push again.

---

## Self-review

- **Spec coverage (G1):** CI workflow with fmt/clippy/test/audit ✓ (Step 4); local gate green on HEAD ✓ (Steps 3,6); G1-hosting row filed ✓ (Step 5); BLAS backend for Linux CI ✓ (`libopenblas-dev`+`gfortran`); the "HEAD passes clippy" assumption was FALSE, so the mechanical-lint fixes are included ✓ (Steps 1-2). No auto-fix steps in CI ✓.
- **Spec coverage (G2):** all six exit criteria mapped to Steps 1–7; docs-only enforced by Step 8 ✓; explicitly avoids architecture.md/design doc (D5) ✓.
- **Spec coverage (G3):** every file deserializes + explicit map + drift-fail ✓ (Step 6); runs in the G1 gate ✓ (Step 8); consistent fixes across `examples/` ✓ (Step 5); schema untouched ✓; quaternion order verified w-first from the doc comment, not assumed ✓.
- **Dependency order:** G3's test must run in G1's gate → Task 3 depends on Task 1. Task 4 (push) depends on 1–3 so the first CI run covers the whole result. G2 is independent but ordered before the push. Matches roadmap graph (G1 → G2 → G3) and standing rule 1 (G2 truthful before later agent units).
- **Type consistency:** test imports `BatchGainRequest`, `GainRequest`, `HeatmapRequest` — the exact public names at `schemas.rs:222,372,411`, reachable via `antenna_model::api::schemas::` (confirmed public: `lib.rs:16 pub mod api`, `api/mod.rs:16 pub mod schemas`).
- **Placeholder scan:** no TBD/"add error handling"/"similar to" — all code and commands are literal.
