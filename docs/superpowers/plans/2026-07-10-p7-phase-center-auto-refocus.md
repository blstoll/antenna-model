# P7 — Auto-Refocus `phase_center_offset` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement roadmap unit P7 (decided 2026-07-10: model auto-refocus) — `phase_center_offset` becomes a compensated feed property that no longer produces axial defocus; a new explicit `axial_defocus` field expresses deliberate defocus; the Ka reference tolerance tightens 5.0 → 1.5 dB. Includes prerequisite unit P1b (`physics_model_version` artifact stamp), which turned out to be **unimplemented** despite P7 depending on it.

**Architecture:** The single physics consumption site is `integration.rs:527` (`feed_axial_offset = position.z − focal_length + phase_center_offset`). Auto-refocus = the compensation exactly cancels the offset, so the fix is to stop adding `phase_center_offset` there and add the new `axial_defocus` field instead (the defocus math itself is untouched and stays reachable). Plumbing: model-layer `FeedParameters` (geometry.rs) → data-layer `FeedParameters` (data/types.rs) → YAML `FeedSpecConfig` (config/settings.rs) → repository/evaluator/h3 builders. Because this changes `gain_physics` for identical inputs, P1b's `physics_model_version` is introduced first (stamped in artifacts, warned on mismatch at load) and bumped by the physics change.

**Tech Stack:** Rust workspace (antenna-model + calibrate), bincode 2 artifacts, serde YAML config, existing reference-validation harness (`antenna-model/tests/reference_validation.rs`).

**Branch:** `feat/p7-phase-center-auto-refocus` off `main` (created in Task 1, Step 1).

**User decisions (already made):**
- Register row P7 Decided 2026-07-10 by maintainer: **model auto-refocus** (raw feed property, model compensates; deliberate defocus via a new explicit field). Recorded in `docs/roadmap-2026-07.md` §5 and `docs/findings-2026-07-10-ka-phase-center-defocus.md`.
- P1b policy (from its roadmap unit, motivated by P1 Decided 2026-07-08): integer version, loader **warns** (not errors) on mismatch, bump whenever `gain_physics` changes for identical inputs.
- Planner default applied (flagged, not user-confirmed): P1b is folded into this branch as Task 1 because P7 hard-depends on it and it is Effort S. If the maintainer wants P1b as a separate PR, split Task 1 out — it is self-contained.

---

## Verified current state (2026-07-10, `main` @ `3fda87d`)

- `physics_model_version` / `PHYSICS_MODEL_VERSION` exists **nowhere** in the workspace — P1b is not implemented.
- Baseline harness residuals (ran `cargo test -p antenna-model --test reference_validation reference_residuals -- --nocapture`):
  - `dsn_34m_uncalibrated` X-band **−0.62 dB** (tolerance 1.5), Ka-band **−3.40 dB** (tolerance 5.0)
  - `dsn_70m_uncalibrated` X **+0.08**, `gbt_100m_uncalibrated` L **+0.19**, Q **−0.34**
- `dsn_34m_uncalibrated` in `calibration_data/antennas.yaml` still carries datasheet phase-center values (S 0.02, X 0.015, Ka 0.008 m) — the Ka defocus is live. The dsn_70m/gbt reference entries were zeroed as a workaround (`# focused` comments, section comment at `antennas.yaml:243-244`).
- The only model-layer consumer of `config.feed.phase_center_offset` is `integration.rs:527`. Lateral/steering math never touches it.
- The roadmap gotcha about dead `illumination::phase_center_offset_phase` is **stale** — that function no longer exists (grep confirms zero hits). The domain-contract open item still claims it exists; Task 5 corrects it.
- The calibrate tuner does NOT tune `phase_center_offset` (fixed parameter — see `parameter_tuner.rs` module doc), so making it inert has no optimizer implications.
- Metadata/feed structs derive bincode `Encode, Decode`: adding fields changes binary layout and breaks decode of pre-existing `.bin` artifacts. Acceptable and explicitly sanctioned by the P1b unit: **no `.bin` artifacts exist anywhere** (all four `calibration_file` entries are `enabled: false`). Every commit adding such a field must say this in its message.
- No API/OpenAPI changes anywhere in this plan: `axial_defocus_m` is service-side config (antennas.yaml / design specs), not a request or response field. Standing rule 4 (mirror openapi.yaml) is not triggered.

## Standing rules that bind every task

1. `cargo test --workspace` after any change under `antenna-model/src/model/` (standing rule 3).
2. Never touch lateral/steering math or sign conventions (`coordinates.rs` negation + BDF). This unit is a sanctioned physics change **to the axial defocus expression only**.
3. `docs/domain-contract.md` changes land in the same commit as the code they describe (contract rule) — Tasks 2/3 carry small contract edits; Task 5 is the full pass.
4. If any existing test fails for a reason that is NOT "gain changed because a nonzero `phase_center_offset` stopped defocusing," stop and investigate — do not adjust the assertion.

---

### Task 1: P1b — `physics_model_version` stamp (constant, artifact field, loader warning)

**Goal:** Artifacts record which physics model they were fitted against; the loader warns (never errors) on mismatch; the bump policy is documented.

**Files:**
- Modify: `antenna-model/src/model/mod.rs` (new `PHYSICS_MODEL_VERSION` constant)
- Modify: `antenna-model/src/data/types.rs` (`CalibrationMetadata` field ~:163, `CalibrationMetadataBuilder` ~:987-1090)
- Modify: `antenna-model/src/data/loader.rs` (warn after the `format_version` check at :165-170; helper fn; tests)
- Modify: `antenna-model/src/data/repository.rs` (`CalibrationMetadata` struct literal at :243)
- Modify: `calibrate/src/artifact_export.rs` (metadata builder at :339)
- Modify: `calibrate/src/boresight_calibration.rs` (metadata builder at :653)
- Modify: `docs/calibration-workflow-guide.md` (new "Physics-model versioning" section)
- Compile errors will reveal any other `CalibrationMetadata { … }` struct literals (types.rs tests) — add the field there too.

**Acceptance Criteria:**
- [ ] `pub const PHYSICS_MODEL_VERSION: u32` exists in `antenna-model/src/model/mod.rs` with a doc comment stating the bump policy and a version history list.
- [ ] `CalibrationMetadata.physics_model_version: u32` exists (`#[serde(default)]`, 0 = unknown/pre-stamp).
- [ ] `calibrate`'s two artifact writers stamp `PHYSICS_MODEL_VERSION`; the repository's uncalibrated (design-spec) path stamps it too.
- [ ] Loader emits a `warn!` naming BOTH values on mismatch, via a pure helper `physics_model_version_mismatch(artifact, current) -> Option<String>` with unit tests (mismatch → Some containing both numbers; match → None; 0/unknown → Some).
- [ ] A loader round-trip test: artifact encoded with version 999 loads Ok and preserves the value.
- [ ] `docs/calibration-workflow-guide.md` documents the new axis and its relation to the two existing version axes (ANTC header u32 = container layout; `format_version` string = schema; `physics_model_version` u32 = physics semantics), cross-referencing roadmap unit D2.
- [ ] `cargo test --workspace` green.

**Verify:** `cargo test -p antenna-model data::` and `cargo test --workspace` → all pass.

**Steps:**

- [ ] **Step 1: Create the branch**

```bash
git checkout main && git pull && git checkout -b feat/p7-phase-center-auto-refocus
```

- [ ] **Step 2: Add the constant** in `antenna-model/src/model/mod.rs` (after the module declarations, before the re-exports):

```rust
/// Version of the physics model's gain computation.
///
/// Correction surfaces are fitted to `measured − physics` residuals, so any change
/// that alters `gain_physics` output for identical inputs invalidates surfaces fitted
/// against the older model. Calibration artifacts record the version they were fitted
/// against (`CalibrationMetadata::physics_model_version`) and the loader warns on
/// mismatch (`data/loader.rs`).
///
/// # Bump policy
/// Bump whenever a change alters `gain_physics` output for identical inputs
/// (new efficiency terms, phase-model changes, defocus semantics, ...).
///
/// # History
/// - 1: baseline at introduction (P1b) — post-P1 model (spillover applied on the
///   uncalibrated path, fractional-q spillover fix)
pub const PHYSICS_MODEL_VERSION: u32 = 1;
```

- [ ] **Step 3: Write the failing helper tests** in `antenna-model/src/data/loader.rs` tests module:

```rust
    #[test]
    fn test_physics_model_version_mismatch_message() {
        let msg = physics_model_version_mismatch(999, 1).expect("mismatch must warn");
        assert!(msg.contains("999") && msg.contains('1'), "must name both versions: {msg}");
        assert!(physics_model_version_mismatch(1, 1).is_none());
        // 0 = unknown / pre-stamp artifact: still a mismatch worth warning about
        assert!(physics_model_version_mismatch(0, 1).is_some());
    }

    #[test]
    fn test_load_artifact_with_mismatched_physics_model_version() {
        let mut calibration = create_test_calibration();
        calibration.metadata.physics_model_version = 999;

        let mut temp_file = NamedTempFile::new().unwrap();
        let config = config::standard();
        let encoded = bincode::encode_to_vec(&calibration, config).unwrap();
        temp_file.write_all(&encoded).unwrap();
        temp_file.flush().unwrap();

        // Mismatch must WARN, not error: load succeeds and preserves the stamp.
        let loaded = load_calibration_artifact(temp_file.path()).unwrap();
        assert_eq!(loaded.metadata.physics_model_version, 999);
    }
```

Run: `cargo test -p antenna-model physics_model_version` → FAIL (field/function don't exist — compile error is the red state).

- [ ] **Step 4: Add the metadata field** in `antenna-model/src/data/types.rs`, at the end of `CalibrationMetadata` (after `measurement_density`):

```rust
    // ========== Physics-model versioning (roadmap P1b) ==========
    /// Version of the physics model this calibration was fitted against
    /// (see `crate::model::PHYSICS_MODEL_VERSION`). 0 = unknown (artifact predates
    /// the version stamp). NOTE: adding this field changed the bincode layout;
    /// pre-P1b `.bin` artifacts no longer decode (none exist — sanctioned by P1b).
    #[serde(default)]
    pub physics_model_version: u32,
```

Extend `CalibrationMetadataBuilder`: add field `physics_model_version: Option<u32>`, method

```rust
    pub fn physics_model_version(mut self, version: u32) -> Self {
        self.physics_model_version = Some(version);
        self
    }
```

and in `build()`: `physics_model_version: self.physics_model_version.unwrap_or(0),`. Fix any `CalibrationMetadata { … }` struct literals the compiler flags (types.rs tests): use `physics_model_version: 0` in generic test fixtures.

- [ ] **Step 5: Loader helper + warn** in `antenna-model/src/data/loader.rs`. Add `use crate::model::PHYSICS_MODEL_VERSION;`; after the `format_version` warn block (:165-170):

```rust
    if let Some(msg) = physics_model_version_mismatch(
        calibration.metadata.physics_model_version,
        PHYSICS_MODEL_VERSION,
    ) {
        warn!("{}", msg);
    }
```

and the helper (file scope, next to `validate_calibration`):

```rust
/// Warning to emit when an artifact was fitted against a different physics-model
/// version than this service computes with. Correction surfaces are fitted to
/// `measured − physics` residuals, so a mismatch can silently degrade accuracy;
/// this is a warning, not an error (roadmap P1b policy).
fn physics_model_version_mismatch(artifact: u32, current: u32) -> Option<String> {
    (artifact != current).then(|| {
        format!(
            "Calibration artifact physics_model_version {} does not match the service's \
             physics model version {}; the correction surface was fitted against a \
             different physics model and residual corrections may be stale — recalibrate",
            artifact, current
        )
    })
}
```

- [ ] **Step 6: Stamp at the three construction sites.**
  - `antenna-model/src/data/repository.rs:243` struct literal: add `physics_model_version: PHYSICS_MODEL_VERSION,` (import `use crate::model::PHYSICS_MODEL_VERSION;`). The design-spec path computes live against the current model, so stamping current is correct.
  - `calibrate/src/artifact_export.rs:339` builder chain: add `.physics_model_version(PHYSICS_MODEL_VERSION)` (import `use antenna_model::model::PHYSICS_MODEL_VERSION;`).
  - `calibrate/src/boresight_calibration.rs:653` builder chain: same addition.

- [ ] **Step 7: Run tests**

Run: `cargo test -p antenna-model physics_model_version` → PASS, then `cargo test --workspace` → all green (macOS: prefix with the OpenBLAS `LDFLAGS`/`CPPFLAGS` from CLAUDE.md if calibrate fails to link).

- [ ] **Step 8: Document the policy.** In `docs/calibration-workflow-guide.md`, add a short "Physics-model versioning" section: the three version axes (ANTC header `u32` = container/binary layout; `metadata.format_version` string = semantic schema; `metadata.physics_model_version` u32 = physics semantics — full reconciliation of the first two is roadmap unit D2), the bump policy (any change altering `gain_physics` for identical inputs), and the loader's warn-on-mismatch behavior (0 = pre-stamp artifact).

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(data): stamp calibration artifacts with physics_model_version (P1b)

Artifacts record the physics-model version they were fitted against; the
loader warns (never errors) on mismatch. Bincode layout change breaks decode
of pre-P1b .bin artifacts — none exist anywhere (sanctioned by roadmap P1b)."
```

---

### Task 2: Model-layer auto-refocus — `axial_defocus` field, defocus expression, version bump

**Goal:** `phase_center_offset` stops contributing to the aperture defocus phase; the new `axial_defocus` field carries deliberate defocus through the identical math; `PHYSICS_MODEL_VERSION` bumps to 2.

**Files:**
- Modify: `antenna-model/src/model/geometry.rs` (`FeedParameters` struct :173-193, `new()` :197-211, builder :300-357)
- Modify: `antenna-model/src/model/integration.rs` (:523-527 defocus expression; tests :990-1022)
- Modify: `antenna-model/src/model/mod.rs` (bump constant, history line)
- Modify: `antenna-model/src/model/edge_cases.rs` (:332, :356 — test struct literals gain the new field)
- Modify: `antenna-model/src/model/ray_trace.rs` (:306 — same)
- Modify: `docs/domain-contract.md` (glossary `phase_center_offset` entry :76 — minimal same-commit edit; full pass in Task 5)

**Acceptance Criteria:**
- [ ] `FeedParameters` has `axial_defocus: f64` (`#[serde(default)]`, default 0.0 in `new()` and builder); `new()` keeps its 4-arg signature.
- [ ] `integration.rs` defocus term reads `position.z − focal_length + axial_defocus`; `phase_center_offset` appears nowhere in the phase computation.
- [ ] `test_axial_defocus_produces_defocus_loss`: 5 cm `axial_defocus` costs > 1 dB at 8.4 GHz (reworked from `test_phase_center_offset_produces_defocus_loss`).
- [ ] `test_phase_center_offset_alone_produces_no_defocus_loss`: 5 cm `phase_center_offset` with zero `axial_defocus` produces gain identical to focused (|Δ| < 1e-9 dB).
- [ ] `PHYSICS_MODEL_VERSION` = 2 with a history line naming P7.
- [ ] `cargo test --workspace` green. Expected legitimate changes only: harness residuals improve for `dsn_34m_uncalibrated` (its yaml carries nonzero phase-center values); any other numeric test failure must be investigated per standing rule 4, not adjusted.

**Verify:** `cargo test -p antenna-model model::integration` → PASS; `cargo test -p antenna-model --test reference_validation reference_residuals -- --nocapture` → dsn_34m Ka residual ≈ −0.1 dB (was −3.40); `cargo test --workspace` → green.

**Steps:**

- [ ] **Step 1: Write the failing companion test** in `antenna-model/src/model/integration.rs` tests module (replacing `test_phase_center_offset_produces_defocus_loss` at :990-1022 — keep the `mk` closure and gain-computation pattern from the old test):

```rust
    /// Auto-refocus (roadmap P7): phase_center_offset is a recorded feed property
    /// the model compensates — it must NOT change gain. Deliberate defocus goes
    /// through the explicit axial_defocus field instead.
    #[test]
    fn test_phase_center_offset_alone_produces_no_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let feed_pco = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.05, 1.0).unwrap();

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_pco = crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_pco), 8.4e9, &params)
            .unwrap()
            .gain;

        assert!(
            (g_focused - g_pco).abs() < 1e-9,
            "phase_center_offset is auto-refocused and must not change gain: \
             focused={g_focused:.6}, pco={g_pco:.6}"
        );
    }

    /// The defocus math stays live through the explicit field: a 5 cm deliberate
    /// axial defocus must cost >1 dB at 8.4 GHz (same physics the old
    /// test_phase_center_offset_produces_defocus_loss pinned).
    #[test]
    fn test_axial_defocus_produces_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let mut feed_defocused =
            FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        feed_defocused.axial_defocus = 0.05;

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_defocused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_defocused), 8.4e9, &params)
                .unwrap()
                .gain;

        assert!(
            g_focused - g_defocused > 1.0,
            "5 cm axial_defocus must cost >1 dB defocus at 8.4 GHz: \
             focused={g_focused:.2}, defocused={g_defocused:.2}"
        );
    }
```

Run: `cargo test -p antenna-model model::integration::tests::test_axial_defocus` → FAIL (no `axial_defocus` field — compile error is the red state).

- [ ] **Step 2: Add the field** in `antenna-model/src/model/geometry.rs`. In `FeedParameters` (after `phase_center_offset`):

```rust
    /// Deliberate axial defocus of the feed's phase center from the focal point,
    /// in meters (positive = away from the reflector vertex). Default 0 (focused).
    /// This is the explicit knob for intentional defocus; it enters the aperture
    /// phase as a quadratic (defocus) term in `integration.rs`.
    #[serde(default)]
    pub axial_defocus: f64,
```

Rewrite the `phase_center_offset` doc comment:

```rust
    /// Phase center offset in meters (distance from physical feed to phase center,
    /// along the feed axis; positive = away from the reflector vertex).
    /// Typically ±λ/4, frequency-dependent.
    ///
    /// AUTO-REFOCUS (roadmap P7, decided 2026-07-10): recorded feed property only.
    /// The model assumes the feed is positioned so its phase center sits at the
    /// focal point (real antennas are refocused per band), so this field does NOT
    /// enter the gain computation. Use `axial_defocus` for deliberate defocus.
    pub phase_center_offset: f64,
```

In `new()` (signature unchanged — 4 args), initialize `axial_defocus: 0.0`. In `FeedParametersBuilder`: add field `axial_defocus: Option<f64>`, method

```rust
    /// Set deliberate axial defocus (meters from the focal point; default 0)
    pub fn axial_defocus(mut self, defocus: f64) -> Self {
        self.axial_defocus = Some(defocus);
        self
    }
```

and in `build()` construct via struct literal (or set after `new()`):

```rust
        let mut params =
            FeedParameters::new(position, q_factor, phase_center_offset, asymmetry_factor)?;
        params.axial_defocus = self.axial_defocus.unwrap_or(0.0);
        Ok(params)
```

Fix the struct-literal test fixtures the compiler flags: `edge_cases.rs:332,356` and `ray_trace.rs:306` gain `axial_defocus: 0.0,`.

- [ ] **Step 3: Swap the defocus term** in `antenna-model/src/model/integration.rs:523-527`:

```rust
    // Axial offset of the feed's PHASE CENTER from the focal point: physical
    // z-offset plus any DELIBERATE defocus (positive = away from the vertex,
    // matching phase_feed_displacement's delta_z). The feed's own
    // phase_center_offset is assumed compensated by per-band feed positioning
    // (auto-refocus, roadmap P7 decided 2026-07-10) and does not contribute.
    let feed_axial_offset =
        config.feed.position.z - config.reflector.focal_length + config.feed.axial_defocus;
```

- [ ] **Step 4: Bump the version.** In `antenna-model/src/model/mod.rs`: constant → `2`, history gains:

```rust
/// - 2: P7 auto-refocus — `phase_center_offset` no longer contributes axial defocus
///   (compensated feed property); deliberate defocus via the new `axial_defocus` field
```

- [ ] **Step 5: Run the model tests, then the whole workspace**

Run: `cargo test -p antenna-model model::integration` → PASS.
Run: `cargo test --workspace` → green. If a numeric assertion outside this plan's tests fails, apply standing rule 4: confirm the fixture used a nonzero `phase_center_offset` (legitimate physics change → the harness rows for dsn_34m are the expected case and pass within existing tolerances) — anything else, stop and investigate.
Run: `cargo test -p antenna-model --test reference_validation reference_residuals -- --nocapture` and record the new residual table for Task 4 (expect dsn_34m Ka ≈ −0.1, X ≈ +0.2; dsn_70m/gbt unchanged — their configs already had zero offsets).

- [ ] **Step 6: Minimal same-commit contract edit.** In `docs/domain-contract.md` glossary `phase_center_offset` row (:76): change "**Not yet implemented** — pending roadmap unit P7" to "**Implemented 2026-07-10 (P7)**: `integration.rs` uses `axial_defocus` only; `phase_center_offset` is compensated (no gain effect), pinned by `test_phase_center_offset_alone_produces_no_defocus_loss`." (Full glossary rework happens in Task 5.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(model): auto-refocus phase_center_offset; add explicit axial_defocus (P7)

phase_center_offset is a compensated feed property (real antennas refocus per
band) and no longer produces aperture defocus; deliberate defocus moves to the
new axial_defocus field through the identical math. Bumps PHYSICS_MODEL_VERSION
to 2 per P1b policy. Closes the -3.4 dB Ka residual on the DSN 34-m harness row."
```

---

### Task 3: Data/service plumbing — `axial_defocus_m` from YAML to the model

**Goal:** Deliberate defocus is expressible end-to-end: YAML design spec → data layer → evaluator/h3 → model; `phase_center_offset_m` in configs is provably inert at the service level.

**Files:**
- Modify: `antenna-model/src/data/types.rs` (data-layer `FeedParameters` :394-404, builder :1252-1283)
- Modify: `antenna-model/src/config/settings.rs` (`FeedSpecConfig` :400-418)
- Modify: `antenna-model/src/data/repository.rs` (feed construction :221-229; tests)
- Modify: `antenna-model/src/service/evaluator.rs` (feed builder :182-187; tests)
- Modify: `antenna-model/src/service/h3_link_budget.rs` (feed builder :105)
- Modify: `calibrate/src/artifact_export.rs` (`ExportPhysicalParams` :238, `DataFeedParameters` literal :285-289, test literal :543)
- Modify: `calibrate/src/boresight_calibration.rs` (`DataFeedParameters` literal :591, test :724)
- Modify: `calibrate/src/main.rs` (literal :697)
- Compile errors reveal any remaining data-`FeedParameters` struct literals (batch.rs:251, heatmap.rs:644,747, h3_link_budget.rs:651, evaluator.rs:504,1400, repository/loader tests…) — add `axial_defocus_m: 0.0`.

**Acceptance Criteria:**
- [ ] Data-layer `FeedParameters.axial_defocus_m: f64` (`#[serde(default)]`) + builder method; `FeedSpecConfig.axial_defocus_m` (`#[serde(default)]`) parses from YAML and defaults to 0 when absent.
- [ ] `repository.rs` passes `feed_spec.axial_defocus_m` through; `evaluator.rs` and `h3_link_budget.rs` set `.axial_defocus(…)` on the model feed builder.
- [ ] Service-level test: two design-spec calibrations identical except `phase_center_offset_m` (0.0 vs 0.02) produce byte-identical gain.
- [ ] Service-level test: `axial_defocus_m: 0.05` produces measurably lower gain than 0.0.
- [ ] Settings test: YAML feed spec without `axial_defocus_m` deserializes with 0.0; with the key, the value is preserved.
- [ ] calibrate writers carry `axial_defocus_m: 0.0` explicitly (deliberate defocus is a service-config concept; the calibrate CLI does not expose it — one-line comment at each site).
- [ ] `cargo test --workspace` green.

**Verify:** `cargo test -p antenna-model service:: config:: data::` and `cargo test --workspace` → PASS.

**Steps:**

- [ ] **Step 1: Write the failing settings test** in `antenna-model/src/config/settings.rs` tests (copy the style of the existing design-spec YAML tests around :1019):

```rust
    #[test]
    fn test_feed_spec_axial_defocus_default_and_explicit() {
        // Absent key -> defaults to 0.0 (all existing configs stay valid)
        let yaml_absent = r#"
          id: "f1"
          name: "Feed 1"
          position: [0.0, 0.0, 0.0]
          q_factor: 1.14
          phase_center_offset_m: 0.01
          frequency_range: [8000.0, 8500.0]
        "#;
        let feed: FeedSpecConfig = serde_yaml::from_str(yaml_absent).unwrap();
        assert_eq!(feed.axial_defocus_m, 0.0);

        // Explicit key -> preserved
        let yaml_explicit = r#"
          id: "f2"
          name: "Feed 2"
          position: [0.0, 0.0, 0.0]
          q_factor: 1.14
          phase_center_offset_m: 0.0
          axial_defocus_m: 0.05
          frequency_range: [8000.0, 8500.0]
        "#;
        let feed: FeedSpecConfig = serde_yaml::from_str(yaml_explicit).unwrap();
        assert_eq!(feed.axial_defocus_m, 0.05);
    }
```

(Adapt the deserialization call to whatever the neighboring tests use — if they parse whole `AntennaConfigEntry` documents, embed the feed in one the same way.)

Run: `cargo test -p antenna-model test_feed_spec_axial_defocus` → FAIL (field doesn't exist).

- [ ] **Step 2: Add the config + data fields.** `antenna-model/src/config/settings.rs` `FeedSpecConfig` (after `phase_center_offset_m`):

```rust
    /// Deliberate axial defocus of the feed phase center from the focal point,
    /// in meters (optional; default 0 = focused). phase_center_offset_m is
    /// compensated by the model (auto-refocus, roadmap P7) — this is the explicit
    /// knob for intentional defocus.
    #[serde(default)]
    pub axial_defocus_m: f64,
```

`antenna-model/src/data/types.rs` data-layer `FeedParameters` (after `phase_center_offset_m`):

```rust
    /// Deliberate axial defocus of the feed phase center from the focal point, in
    /// meters. Default 0 (focused). `phase_center_offset_m` is compensated by the
    /// model (auto-refocus, roadmap P7) and does not produce defocus; this field is
    /// the explicit defocus knob. NOTE: bincode layout change — see P1b caveat.
    #[serde(default)]
    pub axial_defocus_m: f64,
```

`FeedParametersBuilder` (types.rs :1252-1283): add field `axial_defocus_m: Option<f64>`, method `pub fn axial_defocus_m(mut self, defocus: f64) -> Self { … }`, and `axial_defocus_m: self.axial_defocus_m.unwrap_or(0.0),` in `build()`.

- [ ] **Step 3: Thread it through.**
  - `repository.rs:221-229`: add `axial_defocus_m: feed_spec.axial_defocus_m,` to the feed literal.
  - `evaluator.rs:182-187`: add `.axial_defocus(calibration.physical_config.feed.axial_defocus_m)` to the `ModelFeedParams::builder()` chain.
  - `h3_link_budget.rs:105`: same addition to its feed builder chain.
  - calibrate literals (`artifact_export.rs:285-289`, `boresight_calibration.rs:591`): `axial_defocus_m: 0.0, // deliberate defocus is service-config only; not exposed by the calibrate CLI`. Do NOT add it to `ExportPhysicalParams` unless the compiler forces it — keep calibrate's surface unchanged.
  - Sweep remaining compile errors (test fixtures listed in **Files**) with `axial_defocus_m: 0.0`.

- [ ] **Step 4: Write the service-level tests** in `antenna-model/src/service/evaluator.rs` tests module. The module's pattern (see `test_compute_gain_uncalibrated_antenna` at :758): `create_test_calibration(status)` fixture + `CalibrationRepository::new()` + `repo.add_calibration(...)` + `compute_gain_from_request(&create_test_request(), &repo)`. The fixture's feed struct literal is at :500-505. Add a shared helper and the two tests:

```rust
    /// Evaluate the standard test request against an uncalibrated fixture whose
    /// feed has been mutated by `mutate` — returns the served gain_db.
    fn gain_with_feed_mutation(mutate: impl FnOnce(&mut FeedParameters)) -> f64 {
        let mut calibration = create_test_calibration(CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        });
        mutate(&mut calibration.physical_config.feed);
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(calibration);
        compute_gain_from_request(&create_test_request(), &repo)
            .unwrap()
            .gain_db
    }

    /// P7 auto-refocus, end-to-end: a config-level phase_center_offset_m must not
    /// change the served gain (it is a compensated feed property).
    #[test]
    fn test_phase_center_offset_m_is_inert_at_service_level() {
        let g_zero = gain_with_feed_mutation(|_| {});
        let g_pco = gain_with_feed_mutation(|feed| feed.phase_center_offset_m = 0.02);
        // Same deterministic code path, same physics inputs -> bit-identical.
        assert_eq!(
            g_zero, g_pco,
            "phase_center_offset_m must be inert (auto-refocus, P7)"
        );
    }

    /// P7: axial_defocus_m is the live deliberate-defocus knob, end-to-end.
    #[test]
    fn test_axial_defocus_m_reduces_gain_at_service_level() {
        let g_focused = gain_with_feed_mutation(|_| {});
        let g_defocused = gain_with_feed_mutation(|feed| feed.axial_defocus_m = 0.05);
        assert!(
            g_focused - g_defocused > 0.5,
            "5 cm axial_defocus_m at 8.4 GHz must cost measurable gain: \
             focused={g_focused:.2}, defocused={g_defocused:.2}"
        );
    }
```

Run: `cargo test -p antenna-model service::evaluator` → PASS (inert test passes because Task 2 landed; defocus test exercises the new plumbing).

- [ ] **Step 5: Full workspace**

Run: `cargo test --workspace` → green.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(service): plumb axial_defocus_m from design specs to the model (P7)

phase_center_offset_m in configs is now provably inert at the service level
(auto-refocus); deliberate defocus is expressible via the new optional
axial_defocus_m YAML field. Bincode layout change: see P1b caveat (no .bin
artifacts exist)."
```

---

### Task 4: Tighten reference tolerances; update config/fixture commentary

**Goal:** The harness actually guards Ka-band: dsn_34m tolerances drop (Ka 5.0 → 1.5 dB, X 1.5 → 1.0 dB), and the fixture/yaml comments describe the post-P7 reality.

**Files:**
- Modify: `antenna-model/tests/fixtures/reference_datasets/dsn_34m_bwg.psv` (tolerance column + comment block)
- Modify: `antenna-model/tests/reference_validation.rs` (module-doc caveat "Known open item: Ka-band phase-center defocus" → resolved)
- Modify: `calibration_data/antennas.yaml` (reference-section comment :243-244; `# focused` comments on dsn_70m/gbt entries)

**Acceptance Criteria:**
- [ ] `dsn_34m_bwg.psv`: X row `tolerance_db` = 1.0, Ka row = 1.5; the `tolerance_db notes` comment block rewritten with the ACTUAL post-P7 residuals from the harness run (not predictions).
- [ ] Harness passes with the tightened tolerances: `reference_residuals_within_tolerance` green, |Ka residual| ≲ 0.2 dB, |X residual| ≲ 0.3 dB. If the X residual exceeds ~0.5 dB, keep X at 1.5 (the roadmap says tighten X "if the residual supports it") and record why in the fixture comment.
- [ ] `antennas.yaml` reference-section comment no longer presents `phase_center_offset_m = 0` as a required workaround (values are inert under auto-refocus; the comment should say so and point at `axial_defocus_m` for deliberate defocus). `dsn_34m_uncalibrated`'s datasheet values (0.02/0.015/0.008) stay — they are now honest, harmless feed metadata and double as live proof of auto-refocus in the harness.
- [ ] `cargo test --workspace` green.
- [ ] Roadmap stretch criterion (second multi-band reference antenna) intentionally NOT implemented: cross-D/λ generalization is already evidenced by dsn_34m carrying nonzero datasheet offsets at both X (0.015 m) and Ka (0.008 m) under tightened tolerances, plus the existing GBT L/Q rows (1.4–43 GHz). Record this disposition in the Task 5 roadmap note.

**Verify:** `cargo test -p antenna-model --test reference_validation -- --nocapture --test-threads=1` → all rows `ok` with the new tolerances; `cargo test --workspace` → green.

**Steps:**

- [ ] **Step 1: Capture the post-P7 residuals**

Run: `cargo test -p antenna-model --test reference_validation reference_residuals -- --nocapture` and note the dsn_34m X and Ka residuals (expected ≈ +0.2 and ≈ −0.1 from the findings-doc decomposition; use the actual numbers).

- [ ] **Step 2: Tighten the fixture.** Edit the two data rows' `tolerance_db` (X: `1.5` → `1.0`, Ka: `5.0` → `1.5`) and replace the `tolerance_db notes` comment block (fill in actual residuals):

```
# tolerance_db notes:
#   X-band 1.0 dB — after the q_factor fix and P7 auto-refocus (2026-07-10) the model lands
#     ~<X_RESID> dB from measured; the residual is the honest prime-focus-vs-shaped-Cassegrain
#     topology gap.
#   Ka-band 1.5 dB — P7 auto-refocus compensates the phase-center defocus (the design's
#     0.008 m = 0.85λ at Ka previously cost ~3.3 dB); the model lands ~<KA_RESID> dB from
#     measured. Was 5.0 dB while the defocus was unmodeled.
#     See docs/findings-2026-07-10-ka-phase-center-defocus.md (resolved by roadmap P7).
```

Also update the stale figures in the header caveats if they reference the deferred fix (the "fix deferred" phrasing).

- [ ] **Step 3: Update `reference_validation.rs` module doc.** Replace "Known open item: Ka-band phase-center defocus (…) " with "Ka-band phase-center defocus resolved by P7 auto-refocus (2026-07-10) — `phase_center_offset_m` is compensated; see the fixture notes."

- [ ] **Step 4: Update `antennas.yaml` commentary.** Reference-section comment (:243-244) becomes:

```yaml
  # Feeds are set to a ~-11 dB edge taper q_factor (via q_factor_from_taper).
  # phase_center_offset_m is a recorded feed property the model COMPENSATES
  # (auto-refocus, roadmap P7, 2026-07-10) — it does not produce defocus; use
  # axial_defocus_m (default 0) for deliberate defocus studies.
  # (see docs/findings-2026-07-10-ka-phase-center-defocus.md)
```

Change the dsn_70m/gbt `# focused` inline comments to `# recorded feed property; compensated (P7)` or simply drop them.

- [ ] **Step 5: Run tests**

Run: `cargo test -p antenna-model --test reference_validation -- --nocapture --test-threads=1` → all pass with tightened tolerances. Then `cargo test --workspace` → green.

- [ ] **Step 6: Commit** (paste the new residual table into the commit body)

```bash
git add -A
git commit -m "test(reference-validation): tighten DSN 34-m Ka tolerance 5.0->1.5 dB post-P7

Auto-refocus closes the Ka defocus residual (-3.40 -> ~-0.1 dB); X tightened
1.5->1.0 dB. antennas.yaml comments updated: phase_center_offset_m is
compensated, axial_defocus_m is the explicit defocus knob."
```

---

### Task 5: Docs truth pass — contract, findings, roadmap

**Goal:** Every document that describes `phase_center_offset` semantics or P7/P1b status matches the shipped code.

**Files:**
- Modify: `docs/domain-contract.md` (glossary :76; new `axial_defocus` glossary row; open-items bullet :187-192)
- Modify: `docs/findings-2026-07-10-ka-phase-center-defocus.md` (status header; follow-up checklist)
- Modify: `docs/roadmap-2026-07.md` (register row P7 note; §7 risk bullet "Loose Ka reference tolerance until P7 lands")
- Modify: `docs/roadmap-2026-07-work-units.md` (P1b + P7 marked ✅ DONE with commit hashes; dependency-graph note)

**Acceptance Criteria:**
- [ ] Contract glossary `phase_center_offset` row rewritten: compensated feed property (auto-refocus, P7, implemented 2026-07-10), no gain effect, pinned by `test_phase_center_offset_alone_produces_no_defocus_loss`; a NEW glossary row documents `axial_defocus` / `axial_defocus_m` (deliberate defocus, meters, positive away from vertex, default 0, consumed at `integration.rs` `feed_axial_offset`).
- [ ] Contract open-items bullet (:187-192) updated: P7 implemented; ALSO corrects the stale claim that `illumination::phase_center_offset_phase` exists as dead code (it was already removed — grep-verified 2026-07-10; zero hits).
- [ ] Findings doc status header → "Implemented (P7) 2026-07-10"; follow-up steps 2–4 marked done (step 5, the extra multi-band antenna, marked "not needed — see roadmap P7 note").
- [ ] Roadmap register P7 row gains an "implemented" note + commit hashes; §7 Ka-tolerance risk bullet marked resolved; work-units P1b and P7 get the repo-conventional "✅ DONE <date> — <commits>" header lines, including the stretch-criterion disposition (dsn_34m nonzero offsets at X+Ka under tight tolerances + GBT L/Q rows already evidence cross-D/λ generalization).
- [ ] Docs-only commit; `git diff --stat` shows no source changes.

**Verify:** `grep -rn "Not yet implemented" docs/domain-contract.md` → no P7-related hits; `grep -n "phase_center_offset_phase" docs/domain-contract.md` → only as "removed" history; `cargo test --workspace` still green (nothing compiled changed).

**Steps:**

- [ ] **Step 1:** Rewrite the contract glossary row :76 and add the `axial_defocus` row (keep the table format; keep the historical Ka-root-cause pointer to the findings doc). Update the open-items bullet: implementation landed (name the tests), and correct the `phase_center_offset_phase` claim to "already removed; grep-verified 2026-07-10".

- [ ] **Step 2:** Findings doc: flip the `**Status:**` line to implemented; check off follow-up items 2 (fix applied — harness Ka ≈ −0.1), 3 (tolerance tightened), 4 (q-truncation was already fixed 2026-07-10); annotate 5 as not-needed with the rationale from Task 4.

- [ ] **Step 3:** Roadmap docs: register row P7 → append "**IMPLEMENTED 2026-07-10** (branch `feat/p7-phase-center-auto-refocus`, commits <hashes>)"; §7 risk bullet → "resolved by P7 (Ka tolerance now 1.5 dB)". Work-units: add the ✅ DONE headers to P1b and P7 mirroring the Phase-0/P1 convention; note in P7's entry that the version-stamp dependency (P1b) was implemented in the same branch.

- [ ] **Step 4:** Commit

```bash
git add docs/
git commit -m "docs(P7): record auto-refocus implementation in contract, findings, roadmap"
```

---

## Out of scope (explicit)

- **API/OpenAPI**: no request/response schema changes; `axial_defocus_m` is service-side config only. If a future unit exposes it per-request, that is a new decision.
- **Second multi-band reference antenna (roadmap stretch)**: intentionally skipped — see Task 4 AC for the evidence-based rationale, recorded in the roadmap in Task 5.
- **calibrate CLI support for deliberate defocus**: writers stamp `axial_defocus_m: 0.0`; exposing it in `antenna_config.rs` is future work if ever needed.
- **`phase_center_offset_wavelengths` unit quirk in calibrate** (`antenna_config.rs:63` value passed into the meters-typed model builder at `parameter_tuner.rs:166` / `main.rs:201`): pre-existing; becomes inert under P7 (the field no longer affects gain). Not fixed here — noted so the executor doesn't "fix" it mid-task (standing rule: file, don't drive-by).
- **Removing `phase_center_offset` from structs**: the field stays as recorded feed metadata (configs carry datasheet values; removing it would churn every constructor for zero behavior change).

## Post-merge follow-ups (not tasks)

- Update memory `dsn-reference-validation.md` (P7 landed, Ka tolerance 1.5) — the coordinating session does this after execution.
- Roadmap P6 (contract open-items phase closer) picks up any residual doc drift.
