# P1 — Spillover Efficiency on the Uncalibrated Path — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** On antennas with no correction surface, fold physical feed-spillover efficiency into the computed gain (a ~0.3–1.5 dB reduction) consistently across all four compute endpoints, and report the applied loss in the response — while leaving every calibrated-antenna output bit-for-bit unchanged.

**Architecture:** Spillover is applied inside the model layer (`compute_gain`/`compute_gain_db`) behind a plain `apply_spillover: bool` on `IntegrationParams`. The *decision* to apply lives in the service layer — each caller sets the flag from `calibration.correction_surface.is_none()`. The model never sees calibration types. The applied loss (dB) is returned in `GainComputationResult` and surfaced on `ComputationMetadata.spillover_loss_db`. Applying inside the shared model function is what makes gain/batch/heatmap/h3 consistent for free — heatmap and h3 bypass the evaluator entirely and call `compute_gain_db` directly.

**Tech Stack:** Rust, poem (REST), serde, hand-maintained `openapi.yaml`.

**User decisions (already made):**
- Register row **P1 Decided 2026-07-08** — staged: spillover now (this unit); blockage = F3 (data-gated); cross-pol out of scope.
- "Apply it in `compute_gain_db` rather than handle it at the api layer." (user, this conversation) — chosen over a service-layer-only patch because heatmap/h3 bypass the evaluator, so service-only would silently leave two endpoints on the biased number.
- "A boolean would suffice unless there's additional context we should return… I'm reluctant to introduce a warning string for anything that another application is expected to handle." → then "Why the boolean if we're returning `Option<f64>`?" — **Final:** signal with a single field `ComputationMetadata.spillover_loss_db: Option<f64>` (`None` ⇒ not applied). No boolean, no warning string. The existing human-facing "estimated spillover %" advisory warning stays untouched.

---

## Physics / sign note (read once before Task 1)

`estimate_spillover` (`edge_cases.rs:170`, private) returns the **spilled (lost)** power fraction, not the captured fraction. The multiplicative efficiency is therefore `η = 1 − spillover`, and the dB reduction is `10·log10(η)` (a **negative** number). `analyze_edge_cases` already computes this fraction on every call and exposes it publicly as `EdgeCaseAnalysis.spillover_fraction` — we do **not** re-derive it. Sanity band for `q=8, f/D=0.5`: applied loss ≈ **−0.3 to −1.5 dB**.

**Do not** touch the aperture integral, phase math, Ruze/mesh efficiency, or any coefficient. This unit only multiplies the finished gain by an already-computed efficiency.

## Reference-gain interaction (already handled by design)

In `compute_gain_from_request`, the ideal-reference gain (when `include_reference`) is computed with the **same** `integration_params` object as the actual gain (`evaluator.rs:315`). So setting `apply_spillover` once applies it to *both* actual and reference. Because the ideal reference shares the antenna's `q_factor`/`f_over_d`, its base spillover cancels in `loss_db = reference − actual`, preserving the documented "`loss_db` ≈ 0 dB at boresight with a focused feed" invariant (`evaluator.rs:295`). No special-casing needed — but Task 2 pins this with a test.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `antenna-model/src/model/integration.rs` | `IntegrationParams` | Add `apply_spillover: bool` field (default `false`). |
| `antenna-model/src/model/pattern.rs` | `compute_gain` / `compute_gain_db` / `GainComputationResult` | Apply `η = 1−spillover` when flag set; carry `spillover_loss_db: Option<f64>`. Model-layer unit tests. |
| `antenna-model/src/service/evaluator.rs` | gain + reference path | Set flag from `correction_surface.is_none()`; put `spillover_loss_db` on metadata. |
| `antenna-model/src/service/batch.rs` | batch metadata literal | Add the new metadata field (batch reuses per-item responses). |
| `antenna-model/src/service/heatmap.rs` | rectangular heatmap | Set the flag so uncalibrated cells match `/gain`. |
| `antenna-model/src/service/h3_link_budget.rs` | h3 link budget | Set the flag on both `compute_gain_db` call sites. |
| `antenna-model/src/api/schemas.rs` | `ComputationMetadata` | Add `spillover_loss_db: Option<f64>` (+ doc). Update the test literal at `:1800`. |
| `openapi.yaml` | `ComputationMetadata` schema | Mirror the new field (standing rule 4). |
| `docs/domain-contract.md` | scope contract | New "Modeled vs unmodeled efficiency terms" section. |
| `docs/api-documentation.md` | accuracy caveats | Note uncalibrated-path spillover is now modeled; honest limits. |

---

### Task 1: Model layer — apply spillover behind a flag, expose the loss

**Goal:** `compute_gain`/`compute_gain_db` reduce gain by `10·log10(1−spillover)` when `params.apply_spillover` is set, and return the applied loss; default (flag off) behavior is byte-identical to today.

**Files:**
- Modify: `antenna-model/src/model/integration.rs:77-148` (`IntegrationParams` struct + `Default`/`fast`/`high_accuracy`)
- Modify: `antenna-model/src/model/pattern.rs:52-59` (`GainComputationResult`), `:216-284` (`compute_gain`), `:431-455` (`compute_gain_db`)
- Test: `antenna-model/src/model/pattern.rs` (`#[cfg(test)]` module, new test)

**Acceptance Criteria:**
- [ ] `IntegrationParams` has a public `apply_spillover: bool`, `false` in every constructor.
- [ ] With the flag off, gain and `spillover_loss_db` (== `None`) are unchanged vs. baseline — all existing `pattern.rs`/workspace tests pass untouched.
- [ ] With the flag on, `compute_gain_db` returns `gain` reduced by exactly `10·log10(1 − analyze_edge_cases(config,θ,φ).spillover_fraction)`, and `spillover_loss_db == Some(that value)`.
- [ ] For a `q=8, f/D=0.5` boresight fixture the applied loss lands in `[-1.5, -0.3]` dB.

**Verify:** `cargo test -p antenna-model --lib model::pattern` → all pass; `cargo test --workspace` → green.

**Steps:**

- [ ] **Step 1: Add the flag to `IntegrationParams`.**

In `integration.rs`, add the field to the struct (after `use_higher_order_aberrations`, `:103`):

```rust
    /// Fold physical feed-spillover efficiency into the returned gain.
    ///
    /// Decided by the SERVICE layer (set only for antennas with no correction
    /// surface — the surface otherwise absorbs spillover empirically). The model
    /// itself never inspects calibration; it only reads this bool.
    pub apply_spillover: bool,
```

Add `apply_spillover: false,` to the explicit literals in `Default` (`:107-117`), `fast()` (`:124-133`), and `high_accuracy()` (`:138-147`). `with_higher_order_aberrations` (`mut self`) and `with_adaptive_refinement` (`..self.clone()`) carry it automatically — leave them.

- [ ] **Step 2: Add `spillover_loss_db` to `GainComputationResult`.**

In `pattern.rs:52-59`:

```rust
pub struct GainComputationResult {
    /// Computed gain in linear units (or dB if from compute_gain_db)
    pub gain: f64,

    /// Warnings from edge case analysis and computation
    pub warnings: Vec<String>,

    /// Physical spillover loss (dB, negative) folded into `gain` when
    /// `IntegrationParams::apply_spillover` was set; `None` otherwise.
    pub spillover_loss_db: Option<f64>,
}
```

- [ ] **Step 3: Apply spillover in `compute_gain` (linear space).**

Replace the tail of `compute_gain` (`pattern.rs:280-283`) — currently:

```rust
    // Apply gain floor for numerical stability
    let gain = apply_gain_floor(gain);

    Ok(GainComputationResult { gain, warnings })
```

with:

```rust
    // Physical spillover efficiency (uncalibrated path only; gated by the caller).
    // `analysis.spillover_fraction` is the LOST fraction, so η = 1 − fraction.
    let (gain, spillover_loss_db) = if params.apply_spillover {
        let eta = (1.0 - analysis.spillover_fraction).clamp(1e-6, 1.0);
        (gain * eta, Some(10.0 * eta.log10()))
    } else {
        (gain, None)
    };

    // Apply gain floor for numerical stability
    let gain = apply_gain_floor(gain);

    Ok(GainComputationResult {
        gain,
        warnings,
        spillover_loss_db,
    })
```

(`analysis` is already in scope from `:226`.)

- [ ] **Step 4: Propagate the loss through `compute_gain_db`.**

In `pattern.rs:451-454`, carry the field (it is already in dB — pass it through unchanged):

```rust
    Ok(GainComputationResult {
        gain: apply_gain_floor_db(gain_db),
        warnings: result.warnings,
        spillover_loss_db: result.spillover_loss_db,
    })
```

- [ ] **Step 5: Fix any remaining `IntegrationParams { … }` / `GainComputationResult { … }` literals that break compilation.**

Run: `cargo build -p antenna-model 2>&1 | rg "missing field|IntegrationParams|GainComputationResult"`
Known literal sites: `pattern.rs:360` (`..effective_params` — OK, carries automatically), test literals at `pattern.rs:926` and `integration.rs:938`. For any literal without a `..spread`, add `apply_spillover: false,`. There are no other `GainComputationResult { … }` literals besides the two edited above.

Run: `cargo build --workspace` → Expected: clean build.

- [ ] **Step 6: Write the model-layer test.**

Add to the `#[cfg(test)] mod tests` in `pattern.rs`. Model it on the existing `compute_gain_db` tests (e.g. `:754`) for fixture construction:

```rust
#[test]
fn test_spillover_applied_only_when_flagged() {
    // q=8, f/D=0.5 boresight fixture (mirror the existing compute_gain_db tests).
    let reflector = ReflectorGeometry::builder()
        .diameter(1.0)
        .focal_length(0.5) // f/D = 0.5
        .surface_rms(0.001)
        .build()
        .unwrap();
    let feed = FeedParameters::builder()
        .at_focus(0.5)
        .q_factor(8.0)
        .build()
        .unwrap();
    let config = AntennaConfiguration::builder()
        .id("spill")
        .name("Spill")
        .reflector(reflector)
        .feed(feed)
        .build()
        .unwrap();

    // Expected loss comes straight from the public edge-case analysis (== estimate_spillover).
    let analysis = analyze_edge_cases(&config, 0.0, 0.0);
    let expected_loss_db = 10.0 * (1.0 - analysis.spillover_fraction).log10();

    let mut params = IntegrationParams::fast();

    // Flag OFF: unchanged, no loss reported.
    let base = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
    assert!(base.spillover_loss_db.is_none());

    // Flag ON: gain drops by exactly the expected loss; loss reported.
    params.apply_spillover = true;
    let with = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
    let reported = with.spillover_loss_db.expect("loss reported when applied");

    assert!((reported - expected_loss_db).abs() < 1e-9, "reported {reported} vs {expected_loss_db}");
    assert!((with.gain - base.gain - expected_loss_db).abs() < 1e-6, "gain delta must equal reported loss");

    // Sanity band for q=8, f/D=0.5 (loss is negative).
    assert!((-1.5..=-0.3).contains(&reported), "loss {reported} dB outside expected band");
}
```

Ensure `analyze_edge_cases` and the geometry/config types are imported in the test module (they are already used by neighboring tests; add `use super::super::edge_cases::analyze_edge_cases;` if not in scope).

- [ ] **Step 7: Run tests.**

Run: `cargo test -p antenna-model --lib model::` → Expected: PASS including the new test.
Run: `cargo test --workspace` → Expected: green (existing values unchanged — flag defaults off).

- [ ] **Step 8: Commit.**

```bash
git add antenna-model/src/model/integration.rs antenna-model/src/model/pattern.rs
git commit -m "feat(model): apply feed-spillover efficiency behind an opt-in flag (P1)"
```

---

### Task 2: Service wiring (gain + batch) + response field + openapi

**Goal:** `compute_gain_from_request` enables spillover for antennas with no correction surface and reports the applied loss on `ComputationMetadata.spillover_loss_db`; calibrated antennas are unchanged; the field is mirrored in openapi.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:344-362` (`ComputationMetadata` struct), `:1800` (test literal)
- Modify: `antenna-model/src/service/evaluator.rs:210` (params), `:353-359` (metadata)
- Modify: `antenna-model/src/service/batch.rs:200-205` (metadata literal)
- Modify: `openapi.yaml:663-678` (`ComputationMetadata` schema)
- Test: `antenna-model/src/service/evaluator.rs` (`#[cfg(test)]` module)

**Acceptance Criteria:**
- [ ] `ComputationMetadata` has `spillover_loss_db: Option<f64>` with `#[serde(skip_serializing_if = "Option::is_none")]`.
- [ ] For an antenna with `correction_surface == None`, the response gain is reduced and `metadata.spillover_loss_db == Some(<loss>)`.
- [ ] For an antenna with a correction surface, gain is **identical** to pre-change and `metadata.spillover_loss_db == None`.
- [ ] For an uncalibrated, boresight, focused-feed request with `include_reference: true`, `loss_db ≈ 0` (within existing tolerance) — the reference invariant holds.
- [ ] `openapi.yaml` `ComputationMetadata` lists `spillover_loss_db`.

**Verify:** `cargo test -p antenna-model --lib service::evaluator` → PASS; `cargo test --workspace` → green.

**Steps:**

- [ ] **Step 1: Add the response field.** In `schemas.rs`, inside `ComputationMetadata` (after `extrapolated`, `:361`):

```rust
    /// Physical spillover loss folded into `gain_db`, in dB (a small **negative**
    /// value). `null` when physical spillover was NOT applied — i.e. the antenna
    /// has a correction surface (which absorbs spillover empirically). Present only
    /// on the uncalibrated path, so consumers can tell which model variant produced
    /// the number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spillover_loss_db: Option<f64>,
```

- [ ] **Step 2: Fix the metadata literals so it compiles.**
  - `evaluator.rs:353-359`: add `spillover_loss_db,` (populated in Step 3).
  - `batch.rs:200-205`: add `spillover_loss_db: None,` (batch aggregate metadata; per-item responses already carry their own metadata via the evaluator).
  - `schemas.rs:1800` (test literal): add `spillover_loss_db: None,`.

- [ ] **Step 3: Set the flag + read the loss in `compute_gain_from_request`.**

At `evaluator.rs:210`, change:

```rust
    // Use fast integration parameters for <100ms target
    let integration_params = IntegrationParams::fast();
```

to:

```rust
    // Use fast integration parameters for <100ms target
    let mut integration_params = IntegrationParams::fast();
    // Double-counting gate: physical spillover only when NO correction surface
    // exists (the surface otherwise absorbs it). Whole-antenna gate — never per query.
    // Shared with the ideal-reference computation below, so base spillover cancels in loss_db.
    integration_params.apply_spillover = calibration.correction_surface.is_none();
```

The actual-gain result is `result` (`:250`). After `final_gain_db` is computed, the loss to report is `result.spillover_loss_db`. In the `ComputationMetadata` literal (`:353`) set:

```rust
            spillover_loss_db: result.spillover_loss_db,
```

- [ ] **Step 4: Mirror in openapi.yaml.** Under `ComputationMetadata.properties` (after `extrapolated`, `openapi.yaml:677-678`):

```yaml
        spillover_loss_db:
          type: number
          nullable: true
          description: >-
            Physical feed-spillover loss (dB, negative) folded into gain_db on the
            uncalibrated path. Null when not applied (antenna has a correction
            surface, which absorbs spillover empirically).
```

- [ ] **Step 5: Write service tests.**

In the `evaluator.rs` test module, reuse `create_test_calibration(:463)` (uncalibrated by default — confirm it leaves `correction_surface == None`) and the `constant_surface_db(...)` helper used by the h3 tests (`h3_link_budget.rs:761`) to build a calibrated fixture. If `constant_surface_db` is not visible from `evaluator.rs`, construct the `Some(correction_surface)` the same way the existing correction-path evaluator tests do (grep `correction_surface = Some` in `service/`).

```rust
#[test]
fn test_spillover_applied_on_uncalibrated_only() {
    let request = create_test_request(); // boresight-ish; see existing helper

    // Uncalibrated: correction_surface == None -> spillover applied.
    let cal_uncal = create_test_calibration(CalibrationStatus::Uncalibrated);
    assert!(cal_uncal.correction_surface.is_none());
    let resp_uncal = compute_gain_from_request(&request, &cal_uncal).unwrap();
    let loss = resp_uncal.metadata.spillover_loss_db.expect("loss present on uncalibrated");
    assert!(loss < 0.0 && loss >= -1.5, "loss {loss} dB in band");

    // Calibrated: same fixture + a correction surface -> spillover NOT applied,
    // gain identical to a no-spillover baseline.
    let mut cal_cal = create_test_calibration(CalibrationStatus::FullyCalibrated);
    cal_cal.correction_surface = Some(constant_surface_db(0.0)); // 0 dB surface: isolates the gate
    let resp_cal = compute_gain_from_request(&request, &cal_cal).unwrap();
    assert!(resp_cal.metadata.spillover_loss_db.is_none(), "no loss field when calibrated");
}

#[test]
fn test_reference_loss_db_invariant_preserved_with_spillover() {
    // Uncalibrated, boresight, focused feed, include_reference: loss_db stays ~0
    // because spillover applies to BOTH actual and ideal reference (shared params).
    let mut request = create_test_request();
    request.include_reference = true;
    // ensure boresight + focused feed per the existing at-boresight test setup
    let cal = create_test_calibration(CalibrationStatus::Uncalibrated);
    let resp = compute_gain_from_request(&request, &cal).unwrap();
    let loss_db = resp.loss_db.expect("reference requested");
    assert!(loss_db.abs() < 0.1, "loss_db {loss_db} should be ~0 at boresight/focused feed");
}
```

Adjust fixture/field names to the actual helpers (`CalibrationStatus` variants, `constant_surface_db`, `include_reference` field) — confirm each by grep before writing. If `create_test_request` is not boresight/focused, set the feed to focus and angles to 0 as the existing boresight tests do.

- [ ] **Step 6: Run tests.**

Run: `cargo test -p antenna-model --lib service::` → Expected: PASS.
Run: `cargo test --workspace` → Expected: green (calibrated numeric assertions unchanged).

- [ ] **Step 7: Commit.**

```bash
git add antenna-model/src/api/schemas.rs antenna-model/src/service/evaluator.rs antenna-model/src/service/batch.rs openapi.yaml
git commit -m "feat(service): gate spillover on uncalibrated antennas; report spillover_loss_db (P1)"
```

---

### Task 3: Cross-endpoint consistency — heatmap + h3-heatmap

**Goal:** The rectangular-heatmap and h3-link-budget paths apply the identical spillover gate, so an uncalibrated antenna returns the same reduced gain from every endpoint (they call `compute_gain_db` directly, not through the evaluator).

**Files:**
- Modify: `antenna-model/src/service/heatmap.rs` (its `IntegrationParams` construction; near the `correction_applied` logic `:186`)
- Modify: `antenna-model/src/service/h3_link_budget.rs:213` and `:372` (both `compute_gain_db` call sites; correction handled at `:229`, `:382`)
- Test: `antenna-model/src/service/heatmap.rs` and `.../h3_link_budget.rs` test modules

**Acceptance Criteria:**
- [ ] Both paths set `apply_spillover = calibration.correction_surface.is_none()` on the `IntegrationParams` they pass to `compute_gain_db`.
- [ ] A heatmap cell / h3 point for an uncalibrated antenna at boresight equals the `/gain` result for the same geometry (both reduced); a calibrated antenna is unchanged.

**Verify:** `cargo test -p antenna-model --lib service::heatmap service::h3_link_budget` → PASS; `cargo test --workspace` → green.

**Steps:**

- [ ] **Step 1: Locate each path's `IntegrationParams`.** Run: `rg -n "IntegrationParams::(fast|default|high_accuracy)|let .*params" antenna-model/src/service/heatmap.rs antenna-model/src/service/h3_link_budget.rs`. Each compute path builds params once before the per-cell/per-point loop.

- [ ] **Step 2: Set the flag (heatmap).** Where heatmap constructs its integration params, make it `mut` and add, using the same `calibration.correction_surface.is_none()` gate already available (the code reads `cal.correction_surface.is_some()` at `:186`):

```rust
    let mut integration_params = IntegrationParams::fast(); // or the existing constructor
    integration_params.apply_spillover = calibration.correction_surface.is_none();
```

Thread this `integration_params` into the existing `compute_gain_db` call(s). Do not otherwise change the loop.

- [ ] **Step 3: Set the flag (h3).** Same one-line gate on the params used at both `compute_gain_db` sites (`h3_link_budget.rs:213`, `:372`). Use the antenna's `calibration.correction_surface.is_none()`. Both call sites must use the flagged params.

- [ ] **Step 4: Consistency test.** In each test module, assert an uncalibrated boresight cell/point matches the single-gain result. Reuse the module's existing request/fixture builders:

```rust
#[test]
fn test_heatmap_uncalibrated_matches_single_gain_spillover() {
    // Build an uncalibrated calibration and a 1-cell heatmap at boresight;
    // compute the same geometry via compute_gain_from_request; assert equal.
    // (Reuse this module's existing heatmap fixture + evaluator import.)
    // The two gains must match to < 1e-6 dB.
}
```

Flesh out with the module's real fixtures (grep the existing `#[test]` fns for the heatmap/h3 request builders and calibration constructors). The assertion is `(cell_gain - single_gain).abs() < 1e-6`.

- [ ] **Step 5: Run tests.**

Run: `cargo test -p antenna-model --lib service::` → Expected: PASS.
Run: `cargo test --workspace` → Expected: green.

- [ ] **Step 6: Commit.**

```bash
git add antenna-model/src/service/heatmap.rs antenna-model/src/service/h3_link_budget.rs
git commit -m "feat(service): apply spillover gate on heatmap + h3 paths for cross-endpoint consistency (P1)"
```

---

### Task 4: Documentation — scope contract + accuracy caveats

**Goal:** `domain-contract.md` gains a "Modeled vs unmodeled efficiency terms" section; `api-documentation.md` accuracy notes state that uncalibrated-path spillover is now modeled and remain honest about residual uncertainty.

**Files:**
- Modify: `docs/domain-contract.md` (new subsection under Parameter glossary / before Open items `:101`)
- Modify: `docs/api-documentation.md:86-91` (accuracy status list)

**Acceptance Criteria:**
- [ ] `domain-contract.md` has a "Modeled vs unmodeled efficiency terms" subsection: spillover modeled on the uncalibrated path (this unit, `pattern.rs`), blockage unmodeled → F3, cross-pol out of scope; cross-references the double-counting gate.
- [ ] `api-documentation.md` uncalibrated accuracy note mentions spillover is now folded in, with the honest caveat that parameter uncertainty still limits uncalibrated accuracy.
- [ ] No code changes in this task; `docs/` only.

**Verify:** `rg -n "Modeled vs unmodeled" docs/domain-contract.md` → 1 hit; manual read of both edits.

**Steps:**

- [ ] **Step 1: Add the domain-contract section.** Insert before `## Open items surfaced while mining` (`domain-contract.md:101`):

```markdown
## Modeled vs unmodeled efficiency terms

The live gain path multiplies these efficiency factors into directivity:

| Term | Where | Applies to |
|---|---|---|
| Ruze (surface roughness) | `pattern.rs::overall_efficiency` (`ruze_efficiency`) | all antennas |
| Mesh reflection | `pattern.rs::overall_efficiency` (`mesh::mesh_reflection_efficiency`) | mesh reflectors |
| **Feed spillover** | `pattern.rs::compute_gain` behind `IntegrationParams::apply_spillover` (roadmap **P1**) | **uncalibrated antennas, StandardPhysicalOptics mode only** — `correction_surface.is_none()` AND small feed offset |

**Double-counting gate:** spillover is applied *only* when the antenna has no correction
surface at all (whole-antenna gate, decided in the service layer). For calibrated antennas
the fitted correction surface already absorbs spillover empirically, so applying it again
would double-count. The applied loss is reported as `ComputationMetadata.spillover_loss_db`
(dB, negative; `null` when not applied).

**Mode gate (large-offset carve-out):** spillover is additionally applied only in
`ComputationMode::StandardPhysicalOptics` (small feed offsets). For large offsets (>0.3·f,
routing to higher-order/ray-tracing modes) `estimate_spillover`'s linear offset
extrapolation saturates to ~100% and is unvalidated; those cases already carry
degraded-accuracy warnings and retain their exact pre-P1 gain. So `spillover_loss_db` is
`null` for large-offset queries even on uncalibrated antennas.

**Unmodeled (by decision):**
- **Blockage** (feed/strut aperture blockage, ~0.1–0.5 dB) — deferred to feature **F3**;
  data-gated on antenna-config geometry parameters that do not exist yet.
- **Cross-polarization** — out of scope (<0.1 dB on-axis for symmetric prime-focus dishes).

**Honest caveat:** modeling spillover removes a known systematic bias (~0.4–1 dB optimism)
on the uncalibrated path; it does **not** make uncalibrated predictions calibrated-grade —
guessed q-factor and assumed surface RMS still limit accuracy there.
```

- [ ] **Step 2: Update api-documentation accuracy note.** At `api-documentation.md:86-91`, under the accuracy/status list, append to the uncalibrated description a sentence:

```markdown
- **Uncalibrated (design specs only)**: physical feed-spillover efficiency is now folded
  into the returned gain (reported per-response as `metadata.spillover_loss_db`), removing a
  ~0.4–1 dB optimistic bias. Accuracy remains limited by design-spec parameter uncertainty
  (q-factor, surface RMS) and by unmodeled blockage/cross-pol — this is not calibrated-grade.
```

(Adapt to the exact wording/format of the existing list item; do not restructure the section.)

- [ ] **Step 3: Commit.**

```bash
git add docs/domain-contract.md docs/api-documentation.md
git commit -m "docs(P1): record spillover as modeled on the uncalibrated path; accuracy caveats"
```

---

## Final verification (after all tasks)

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace` → green; **no existing numeric assertion changed** (spillover is opt-in; calibrated outputs identical).
- [ ] `scripts/check.sh` (the G1 local gate) passes.
- [ ] Manual: POST an uncalibrated-antenna gain request → response shows a negative `metadata.spillover_loss_db`; a calibrated antenna omits the field.

## Coordination notes (do not implement here)

- **P1b** (physics-model version stamp) is the companion unit: this change alters `gain_physics` for uncalibrated antennas, which is exactly the trigger to bump `physics_model_version`. Leave the bump to P1b, but mention this PR in P1b's motivation.
- **C8 stage 3** will convert `warnings: Vec<String>` to typed `ApiWarning{code,message}` and expects a `spillover_applied` code. We deliberately did **not** add a spillover *warning* (the signal is the structured `spillover_loss_db` field). If C8 still wants a code, it reads the field — no string to type. Note this in the C8 stage-3 handoff.
- **P3/P6** also edit `domain-contract.md`; sequence after this unit to avoid conflicts (per roadmap).
```

