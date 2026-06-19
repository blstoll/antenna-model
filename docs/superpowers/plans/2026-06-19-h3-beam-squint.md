# H3 Beam Squint Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-extended-cc:subagent-driven-development (recommended) or superpowers-extended-cc:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `/h3` link-budget endpoint honor `pointing_frequency_mhz` by applying beam-squint correction to per-cell gains, using one shared helper, and surface the applied squint on the response.

**Architecture:** Extract the squint application currently inlined in the `/gain` evaluator into a single `squint_corrected_direction` helper in `coordinates_3d.rs`. Call it from the evaluator (behavior-preserving) and from both `/h3` gain paths, feeding the squint-corrected `(az, el)` into both the cache key and `compute_gain_db`. Add a response-level `beam_squint_deg: Option<f64>` to `H3LinkBudgetResponse`, mirroring `/gain`.

**Tech Stack:** Rust, `poem` (API), existing `apply_beam_squint_correction` physics (direction-cosine `(u,v)` squint from Task 10).

**Spec:** `docs/superpowers/specs/2026-06-19-h3-beam-squint-design.md`

---

## File Structure

- `antenna-model/src/model/coordinates_3d.rs` — new `squint_corrected_direction` helper + unit tests (lives next to `apply_beam_squint_correction`).
- `antenna-model/src/model/mod.rs` — re-export the helper.
- `antenna-model/src/service/evaluator.rs` — replace the inline squint block (lines ~166–190) with a helper call.
- `antenna-model/src/service/h3_link_budget.rs` — apply squint at both gain call sites (`compute_cell_gain` ~line 165, boresight cell ~line 301); compute response-level squint magnitude; remove the `TODO(squint)` comment.
- `antenna-model/src/api/schemas.rs` — `beam_squint_deg: Option<f64>` on `H3LinkBudgetResponse` (struct at line 649).
- `openapi.yaml` — document the new response field.

---

### Task 1: Shared `squint_corrected_direction` helper

**Goal:** A single, pure, tested function that applies frequency-offset beam squint to an emitter direction.

**Files:**
- Modify: `antenna-model/src/model/coordinates_3d.rs` (add fn next to `apply_beam_squint_correction`, ~line 631; tests in the existing `Beam Squint` test module)
- Modify: `antenna-model/src/model/mod.rs` (re-export)

**Acceptance Criteria:**
- [ ] `squint_corrected_direction(az, el, operating, pointing, feed_x, feed_y, focal_length)` returns the input direction unchanged with `squint_deg == 0.0` when `|pointing − operating| ≤ 0.1` MHz.
- [ ] When the offset exceeds 0.1 MHz and feed displacement is non-zero, it returns a direction shifted by `apply_beam_squint_correction` with a non-zero `squint_deg`.
- [ ] Re-exported from `model::mod`.

**Verify:** `cargo test -p antenna-model squint_corrected` → PASS

**Steps:**

- [ ] **Step 1: Write failing tests** in the `Beam Squint` tests module of `coordinates_3d.rs`:

```rust
#[test]
fn test_squint_corrected_direction_no_offset_passthrough() {
    // Frequency offset within 0.1 MHz → no correction, squint 0.
    let (az, el, squint) =
        squint_corrected_direction(45.0, 10.0, 8400.0, 8400.05, 0.5, 0.0, 13.6);
    assert!((az - 45.0).abs() < 1e-9);
    assert!((el - 10.0).abs() < 1e-9);
    assert_eq!(squint, 0.0);
}

#[test]
fn test_squint_corrected_direction_applies_when_offset() {
    // Real frequency offset + lateral feed displacement → direction shifts, squint > 0.
    let (az, el, squint) =
        squint_corrected_direction(0.0, 2.0, 8400.0, 8800.0, 1.0, 0.0, 13.6);
    assert!(squint > 0.0, "expected non-zero squint, got {squint}");
    assert!((el - 2.0).abs() > 1e-6 || (az - 0.0).abs() > 1e-6, "direction should change");
}

#[test]
fn test_squint_corrected_direction_zero_displacement_no_squint() {
    // No lateral displacement → no squint even with a frequency offset.
    let (az, el, squint) =
        squint_corrected_direction(30.0, 5.0, 8400.0, 8800.0, 0.0, 0.0, 13.6);
    assert!((az - 30.0).abs() < 1e-9);
    assert!((el - 5.0).abs() < 1e-9);
    assert_eq!(squint, 0.0);
}
```

- [ ] **Step 2: Run, expect FAIL**

Run: `cargo test -p antenna-model squint_corrected`
Expected: FAIL — `cannot find function squint_corrected_direction`.

- [ ] **Step 3: Implement** the helper in `coordinates_3d.rs`, immediately after `apply_beam_squint_correction`:

```rust
/// Apply frequency-offset beam squint to an emitter direction.
///
/// Computes the feed radial displacement and clock angle from `(feed_x, feed_y)` and,
/// when the operating and pointing frequencies differ by more than 0.1 MHz, shifts the
/// direction in direction-cosine `(u, v)` space via [`apply_beam_squint_correction`].
/// Otherwise the input direction is returned unchanged.
///
/// This is the single source of truth shared by the `/gain` evaluator and the `/h3`
/// link budget. The `pointing_frequency_mhz.unwrap_or(operating)` defaulting is the
/// caller's responsibility; this function takes an explicit pointing frequency.
///
/// Returns `(az_deg, el_deg, squint_deg)`; `squint_deg == 0.0` when no correction applies.
/// The returned `squint_deg` magnitude depends only on the feed displacement and the
/// frequency offset, not on the input direction.
pub fn squint_corrected_direction(
    az_deg: f64,
    el_deg: f64,
    operating_freq_mhz: f64,
    pointing_freq_mhz: f64,
    feed_x: f64,
    feed_y: f64,
    focal_length_m: f64,
) -> (f64, f64, f64) {
    if (pointing_freq_mhz - operating_freq_mhz).abs() <= 0.1 {
        return (az_deg, el_deg, 0.0);
    }
    let feed_displacement_m = (feed_x * feed_x + feed_y * feed_y).sqrt();
    let displacement_clock_angle_rad = feed_y.atan2(feed_x);
    apply_beam_squint_correction(
        az_deg,
        el_deg,
        pointing_freq_mhz,
        operating_freq_mhz,
        feed_displacement_m,
        focal_length_m,
        displacement_clock_angle_rad,
    )
}
```

- [ ] **Step 4: Re-export** in `antenna-model/src/model/mod.rs`. Find the `pub use coordinates_3d::{ ... }` block and add `squint_corrected_direction` to the list (alongside `apply_beam_squint_correction`).

- [ ] **Step 5: Run, expect PASS**

Run: `cargo test -p antenna-model squint_corrected` → PASS (3 tests).
Then `cargo build --release` and `cargo clippy -p antenna-model -- -D warnings` → clean.

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/model/coordinates_3d.rs antenna-model/src/model/mod.rs
git commit -m "feat: add squint_corrected_direction helper (single source of truth for beam squint)"
```

---

### Task 2: Refactor the `/gain` evaluator to use the helper

**Goal:** Replace the evaluator's inline squint block with a call to `squint_corrected_direction`, with no behavior change.

**Files:**
- Modify: `antenna-model/src/service/evaluator.rs` (lines ~166–190; the `feed_displacement_m`/`displacement_clock_angle_rad`/`apply_beam_squint_correction` block)

**Acceptance Criteria:**
- [ ] The evaluator computes `(corrected_az, corrected_el, squint_magnitude_deg)` via `squint_corrected_direction`.
- [ ] The local `feed_displacement_m` and `displacement_clock_angle_rad` bindings (only used by the squint call) are removed.
- [ ] `response.geometry.beam_squint_deg` is unchanged in behavior (`Some(x)` when `x > 0.001`).
- [ ] All existing `/gain` squint tests still pass.

**Verify:** `cargo test -p antenna-model beam_squint && cargo test -p antenna-model compute_gain` → PASS

**Steps:**

- [ ] **Step 1: Replace the inline block.** In `evaluator.rs`, delete lines that compute `feed_displacement_m` (line ~169), `displacement_clock_angle_rad` (line ~170), the `pointing_freq` binding (lines ~174–177), and the `if (pointing_freq - request.frequency_mhz).abs() > 0.1 { apply_beam_squint_correction(...) } else { ... }` expression (lines ~178–190). Replace with:

```rust
    // Apply beam squint correction if pointing frequency differs from operating frequency.
    // Must be done AFTER computing feed position since squint depends on actual displacement.
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);

    let (corrected_az, corrected_el, squint_magnitude_deg) = squint_corrected_direction(
        emitter_az,
        emitter_el,
        request.frequency_mhz,
        pointing_freq,
        feed_x,
        feed_y,
        focal_length_m,
    );
```

Keep `feed_offset` (line ~164) and everything downstream (`corrected_az`/`corrected_el` → `theta_rad`/`phi_rad`, and `squint_magnitude_deg` → `beam_squint_deg`) exactly as-is.

- [ ] **Step 2: Update imports.** Ensure `squint_corrected_direction` is imported in `evaluator.rs` (it imports from `crate::model::...` near line 78 where `apply_beam_squint_correction` is imported). Replace `apply_beam_squint_correction` in that `use` with `squint_corrected_direction` if `apply_beam_squint_correction` is no longer referenced elsewhere in the file (grep to confirm; if unused, remove it from the import to avoid a dead-import warning).

- [ ] **Step 3: Run, expect PASS (behavior preserved)**

Run: `cargo test -p antenna-model beam_squint` → PASS
Run: `cargo test -p antenna-model compute_gain` → PASS
Run: `cargo test -p antenna-model` (full) → PASS; `cargo clippy -p antenna-model -- -D warnings` → clean.

- [ ] **Step 4: Commit**

```bash
git add antenna-model/src/service/evaluator.rs
git commit -m "refactor: evaluator uses shared squint_corrected_direction helper"
```

---

### Task 3: Apply beam squint in both `/h3` gain paths

**Goal:** Both H3 gain call sites apply squint-corrected `(az, el)` to the cache key and `compute_gain_db`, so per-cell gains honor `pointing_frequency_mhz`.

**Files:**
- Modify: `antenna-model/src/service/h3_link_budget.rs` (`compute_cell_gain` ~lines 165–190; boresight-cell path in `compute_h3_link_budget` ~lines 301–316; remove the `TODO(squint)` comment ~lines 165–171)
- Test: `antenna-model/src/service/h3_link_budget.rs` (tests module)

**Acceptance Criteria:**
- [ ] In `compute_cell_gain`, after computing raw `(az_deg, el_deg)`, the corrected direction is used for BOTH the `GainCacheKey` and the `theta_rad/phi_rad` passed to `compute_gain_db`.
- [ ] The boresight-cell path in `compute_h3_link_budget` does the same.
- [ ] The `TODO(squint)` comment is removed.
- [ ] A large frequency offset + steered feed changes the per-cell gains vs. no offset.
- [ ] With `pointing_frequency_mhz == None` (or equal to operating), cell gains are unchanged.

**Verify:** `cargo test -p antenna-model h3` → PASS

**Steps:**

- [ ] **Step 1: Write failing tests** in the `h3_link_budget.rs` tests module. These rely on the existing `make_h3_test_calibration` / `make_h3_test_request` helpers. The request's feed must be steered off boresight so feed displacement > 0; set `feed_position` away from `reflector_boresight` if the helper doesn't already.

```rust
#[test]
fn test_h3_squint_changes_cell_gains_with_pointing_offset() {
    let calibration = make_h3_test_calibration();

    // Baseline: no pointing offset.
    let mut req_baseline = make_h3_test_request();
    req_baseline.pointing_frequency_mhz = None;

    // Squint: large pointing/operating offset. Steer the feed off boresight so the
    // feed displacement (and hence squint) is non-zero.
    let mut req_squint = make_h3_test_request();
    req_squint.feed_position = Position3D::new(
        req_squint.reflector_boresight.x + 0.05,
        req_squint.reflector_boresight.y,
        req_squint.reflector_boresight.z,
    );
    req_squint.pointing_frequency_mhz = Some(req_squint.frequency_mhz * 1.4);
    // Mirror the same feed steering into the baseline so ONLY the squint differs.
    req_baseline.feed_position = req_squint.feed_position.clone();

    let resp_baseline = compute_h3_link_budget(&calibration, &req_baseline).unwrap();
    let resp_squint = compute_h3_link_budget(&calibration, &req_squint).unwrap();

    let gains_baseline: Vec<f64> = resp_baseline.cells.iter().map(|c| c.gain_db).collect();
    let gains_squint: Vec<f64> = resp_squint.cells.iter().map(|c| c.gain_db).collect();
    assert_eq!(gains_baseline.len(), gains_squint.len());
    assert_ne!(
        gains_baseline, gains_squint,
        "a large pointing-frequency offset with a steered feed must change cell gains"
    );
}

#[test]
fn test_h3_no_pointing_offset_is_unchanged() {
    let calibration = make_h3_test_calibration();

    let mut req_none = make_h3_test_request();
    req_none.pointing_frequency_mhz = None;

    let mut req_equal = make_h3_test_request();
    req_equal.pointing_frequency_mhz = Some(req_equal.frequency_mhz);

    let resp_none = compute_h3_link_budget(&calibration, &req_none).unwrap();
    let resp_equal = compute_h3_link_budget(&calibration, &req_equal).unwrap();

    let gains_none: Vec<f64> = resp_none.cells.iter().map(|c| c.gain_db).collect();
    let gains_equal: Vec<f64> = resp_equal.cells.iter().map(|c| c.gain_db).collect();
    assert_eq!(gains_none, gains_equal, "pointing == operating must not change gains");
}
```

If `Position3D` field access differs (e.g. it uses `.x/.y/.z` vs tuple), adapt to the actual struct (it is used elsewhere in this test module — match that usage).

- [ ] **Step 2: Run, expect FAIL** (squint not yet applied → both responses identical):

Run: `cargo test -p antenna-model test_h3_squint_changes_cell_gains_with_pointing_offset`
Expected: FAIL on `assert_ne!` (gains equal because squint is ignored).

- [ ] **Step 3: Apply squint in `compute_cell_gain`.** Replace the `TODO(squint)` comment block and the raw direction usage (~lines 165–189). After the `compute_emitter_direction_with_attitude` call that yields `(az_deg, el_deg)`, insert:

```rust
    // Apply beam squint (honors pointing_frequency_mhz). Corrected angles are used for
    // BOTH the cache key and the gain evaluation so cached values match the angle used.
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);
    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let (az_deg, el_deg, _squint_deg) = squint_corrected_direction(
        az_deg,
        el_deg,
        request.frequency_mhz,
        pointing_freq,
        feed_x,
        feed_y,
        focal_length_m,
    );
```

The existing `GainCacheKey::new(az_deg, el_deg, ...)` and `theta_rad = el_deg.to_radians()` / `phi_rad = az_deg.to_radians()` lines then use the shadowed corrected values unchanged. Delete the multi-line `// TODO(squint): ...` comment.

- [ ] **Step 4: Apply squint in the boresight-cell path** of `compute_h3_link_budget` (~line 301). After its `compute_emitter_direction_with_attitude` call yields `(az_deg, el_deg)`, insert the identical block:

```rust
        let pointing_freq = request
            .pointing_frequency_mhz
            .unwrap_or(request.frequency_mhz);
        let focal_length_m = calibration.physical_config.reflector.focal_length_m;
        let (az_deg, el_deg, _squint_deg) = squint_corrected_direction(
            az_deg,
            el_deg,
            request.frequency_mhz,
            pointing_freq,
            feed_x,
            feed_y,
            focal_length_m,
        );
```

(The boresight block already has `feed_x`, `feed_y`, `calibration`, and `request` in scope.)

- [ ] **Step 5: Add the import.** Add `squint_corrected_direction` to the `use crate::model::{...}` imports at the top of `h3_link_budget.rs` (near the existing `compute_emitter_direction_with_attitude` import).

- [ ] **Step 6: Run, expect PASS**

Run: `cargo test -p antenna-model h3` → PASS (including the two new tests).
Run: `cargo test -p antenna-model` (full) → PASS; `cargo clippy -p antenna-model -- -D warnings` → clean.

- [ ] **Step 7: Commit**

```bash
git add antenna-model/src/service/h3_link_budget.rs
git commit -m "fix: apply beam squint in H3 link budget so /h3 honors pointing_frequency_mhz"
```

---

### Task 4: Surface `beam_squint_deg` on the H3 response

**Goal:** Add an optional `beam_squint_deg` to `H3LinkBudgetResponse`, populated once per request, mirroring `/gain`; document it in openapi.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs` (`H3LinkBudgetResponse`, struct at line 649)
- Modify: `antenna-model/src/service/h3_link_budget.rs` (compute magnitude once; set field in the returned response, ~lines 283 and 455–467)
- Modify: `openapi.yaml` (`H3LinkBudgetResponse` schema)

**Acceptance Criteria:**
- [ ] `H3LinkBudgetResponse` has `#[serde(skip_serializing_if = "Option::is_none")] pub beam_squint_deg: Option<f64>`.
- [ ] It is `Some(x)` (x = squint magnitude in degrees) when a pointing offset produces squint > 0.001°, else `None`.
- [ ] The field appears in `openapi.yaml`.
- [ ] Existing H3 response construction sites compile (the field is added everywhere `H3LinkBudgetResponse { ... }` is built).

**Verify:** `cargo test -p antenna-model h3` → PASS

**Steps:**

- [ ] **Step 1: Write a failing test** in `h3_link_budget.rs` tests:

```rust
#[test]
fn test_h3_reports_beam_squint_deg() {
    let calibration = make_h3_test_calibration();

    let mut req = make_h3_test_request();
    req.feed_position = Position3D::new(
        req.reflector_boresight.x + 0.05,
        req.reflector_boresight.y,
        req.reflector_boresight.z,
    );
    req.pointing_frequency_mhz = Some(req.frequency_mhz * 1.4);
    let resp = compute_h3_link_budget(&calibration, &req).unwrap();
    assert!(
        resp.beam_squint_deg.is_some_and(|s| s > 0.0),
        "expected Some(squint>0), got {:?}",
        resp.beam_squint_deg
    );

    let mut req_none = make_h3_test_request();
    req_none.pointing_frequency_mhz = None;
    let resp_none = compute_h3_link_budget(&calibration, &req_none).unwrap();
    assert!(resp_none.beam_squint_deg.is_none(), "no offset → None");
}
```

- [ ] **Step 2: Run, expect FAIL** (no field yet → compile error / missing field):

Run: `cargo test -p antenna-model test_h3_reports_beam_squint_deg`
Expected: FAIL — `no field beam_squint_deg on H3LinkBudgetResponse`.

- [ ] **Step 3: Add the field** to `H3LinkBudgetResponse` in `schemas.rs`, after `calibration_status`:

```rust
    /// Beam squint magnitude applied (degrees), when the pointing frequency differs from
    /// the operating frequency. Omitted when no squint is applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beam_squint_deg: Option<f64>,
```

- [ ] **Step 4: Compute the magnitude once** in `compute_h3_link_budget`, right after `build_antenna_config` returns `(antenna_config, feed_x, feed_y, feed_z)` (~line 283). The squint magnitude is direction-independent, so evaluate at `(0.0, 0.0)`:

```rust
    // Squint magnitude is constant per request (depends only on feed displacement and the
    // frequency offset, not on cell direction). Compute once for the response field.
    let pointing_freq = request
        .pointing_frequency_mhz
        .unwrap_or(request.frequency_mhz);
    let focal_length_m = calibration.physical_config.reflector.focal_length_m;
    let (_, _, squint_magnitude_deg) = squint_corrected_direction(
        0.0,
        0.0,
        request.frequency_mhz,
        pointing_freq,
        feed_x,
        feed_y,
        focal_length_m,
    );
    let beam_squint_deg = if squint_magnitude_deg > 0.001 {
        Some(squint_magnitude_deg)
    } else {
        None
    };
```

If a `pointing_freq`/`focal_length_m` binding already exists in this scope from Task 3's boresight block, reuse it rather than re-binding (avoid `unused`/shadowing warnings — the boresight block is in a nested `{ }` scope, so a function-level binding here is fine; name them once at function scope and reuse in the boresight block).

- [ ] **Step 5: Set the field** in the returned `H3LinkBudgetResponse { ... }` (~line 455). Add `beam_squint_deg,` alongside the other fields.

- [ ] **Step 6: Update any other construction sites.** Run `grep -n "H3LinkBudgetResponse {" antenna-model/` and add `beam_squint_deg: None` (or the computed value) to every struct literal, including tests.

- [ ] **Step 7: Update openapi.yaml.** In the `H3LinkBudgetResponse` schema's `properties`, add:

```yaml
        beam_squint_deg:
          type: number
          format: double
          nullable: true
          description: >-
            Beam squint magnitude applied (degrees), present only when the request's
            pointing_frequency_mhz differs from the operating frequency. Omitted when no
            squint is applied.
```

- [ ] **Step 8: Run, expect PASS**

Run: `cargo test -p antenna-model h3` → PASS.
Run: `cargo test --all` → PASS; `cargo build --release`; `cargo clippy -p antenna-model -- -D warnings` → clean.
Validate openapi: `npx --yes @redocly/cli lint openapi.yaml` (or `python3 -c "import yaml; yaml.safe_load(open('openapi.yaml'))"`).

- [ ] **Step 9: Commit**

```bash
git add antenna-model/src/api/schemas.rs antenna-model/src/service/h3_link_budget.rs openapi.yaml
git commit -m "feat: report beam_squint_deg on the H3 link budget response"
```

---

## Notes

- **Calibration artifacts:** unaffected — this change does not alter the physics model or the correction-surface application, only which angle the H3 path evaluates the existing pattern at.
- **Cache correctness:** the corrected `(az, el)` must be used for BOTH the cache key and the gain evaluation. Keying on raw angles while evaluating at corrected angles (or vice versa) would return stale gains — Task 3 Step 3/4 shadow `az_deg`/`el_deg` before the cache key is built specifically to prevent this.
