# Correction-surface endpoint fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the B-spline basis so a query at the exact maximum of an axis is no longer evaluated as zero, which today silently corrupts the fitted correction surface across the whole top knot span of every axis.

**Architecture:** One targeted change to the `k == 1` base case of `bspline_basis` in `calibrate/src/correction_surface.rs`, proved by a partition-of-unity test at every corner and face of the domain. Then re-measure D12's end-to-end numbers and tighten the two tolerances it had to leave weak. The service-side 4D interpolator needs **no change** — it was verified correct during planning.

**Tech Stack:** Rust, `cargo test`, the existing `calibrate` + `antenna-model` workspace.

**User decisions (already made):**
- "Fix before D13 and D14, and before D9 ships any artifact" — this session. D12 filed the defect; both remaining calibration units and the artifact-shipping unit are downstream of it.
- Fix goes in its **own PR**, not folded into D12's — this session. Rationale recorded: it changes numerics rather than tests, and folding it in would destroy D12's before/after evidence by retuning the very tolerances that document the defect.
- Tighten D12's two weakened tolerances **in this PR**, as the visible proof the fix worked — this session.

---

## Planning findings — read these before touching code

Established empirically on 2026-07-30 against `main` at `dfde586`. **Two of them correct the findings doc**, which was written from symptoms before the mechanism was isolated.

### 1. The root cause is the `k == 1` base case, and it is one line

`calibrate/src/correction_surface.rs:408-419`:

```rust
fn bspline_basis(i: usize, k: usize, t: f64, knots: &[f64]) -> f64 {
    if k == 1 {
        if i < knots.len() - 1 && t >= knots[i] && t < knots[i + 1] {
            return 1.0;
        }
        // Special case for right endpoint
        if i == knots.len() - 2 && t == knots[i + 1] {
            return 1.0;
        }
        return 0.0;
    }
    ...
```

The interval is half-open, so at `t == t_max` the first branch never fires. The "special case for right endpoint" targets `i == knots.len() - 2`, which for a **clamped** knot vector is a *padding* index sitting outside the valid basis range `0..n_basis` (`n_basis == knots.len() - order`) — so it is never among the indices `evaluate_basis_functions` actually evaluates, and never fires usefully.

**Measured (partition of unity: all coefficients 1.0, so a correct basis must return exactly 1.0 everywhere):**

| query | `CorrectionSurface::evaluate` |
|---|---|
| t = 0.0 (min) | 1.000000000 |
| t = 0.5 | 1.000000000 |
| t = 0.99 | 1.000000000 |
| t = 0.9999 | 1.000000000 |
| **t = 1.0 (max)** | **0.000000000** |

Only the exact endpoint is wrong. The basis is correct arbitrarily close to it.

### 2. The damage is two-stage — and the fitting stage is the harmful one

The evaluation failure alone would be minor. The real damage is in the **fit**:

`accumulate_normal_equations` calls the same `evaluate_basis_functions`. Every measurement lying exactly on an axis maximum therefore contributes an **all-zero row** to the normal equations. On a regular measurement grid the maximum *always* has data on it — D12's fixture has 72 of 288 rows at 700 MHz. The last coefficient in that axis consequently gets no data support and is driven to ~0 by the ridge term.

That is why D12 measured `699.999 → 0.000090` while the basis at 699.999 is ≈1.0: the basis was fine, the *coefficient* was ~0. The corruption spans the entire top knot span, not just the endpoint.

**So the acceptance test must check a fitted surface, not only the basis.** A basis-only test would pass while the fit stayed broken.

### 3. CORRECTION to the findings doc — the service-side 4D interpolator is NOT broken

`docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md` claims the defect exists in both `CorrectionSurface::evaluate` and the service-side `evaluate_correction`. **That is wrong**, and Task 3 must fix the doc.

`antenna-model/src/model/correction_interpolator.rs` uses the standard NURBS-book Cox-de Boor recurrence (`basis[0] = 1.0`, then the triangular recurrence) — a *different algorithm* from calibrate's naive recursive `bspline_basis` — with a `find_knot_span` that clamps to the last valid span. Verified by the same partition-of-unity construction, with coefficients built by hand rather than by the fitter:

| query | `evaluate_correction` |
|---|---|
| interior | 1.000000000 |
| azimuth at max | 1.000000000 |
| elevation at max | 1.000000000 |
| frequency at max | 1.000000000 |
| temperature at max | 1.000000000 |
| **all four at max simultaneously** | **1.000000000** |

The 4D interpolator's apparent failure in D12's diagnostic was **inherited corrupted coefficients** from the broken fitter, not its own defect.

**Consequence for severity:** the served path is not itself wrong. Artifacts are *fitted* wrong, and the service then faithfully serves the bad coefficients. Still blocking for D9/D13/D14 — a shipped artifact would carry the corruption — but this is an artifact-production defect, not a service defect, and the fix is confined to one file in `calibrate`.

### 4. Deliberately out of scope (file, do not fix here)

The findings doc lists two adjacent problems. Neither belongs in this plan:

- **`validate_knot_vector` does not check multiplicity.** Real, but adding the check would *fail on the current adaptive knots*, which produce multiplicity 5 for order 4 (`[400,400,400,400,400,500,600,700,700,700,700,700]`). Fixing it therefore requires also fixing `generate_adaptive_knots`' quantile placement — a separate, larger change.
- **The data-sufficiency check tests `(spline_order+1)³ = 125` when the real requirement is the coefficient count** (960 for the shipped 4/6/8 knot config). Fixing it would make D12's 288-point fixture fail outright, forcing either a much larger fixture or reduced knot counts. That is a design decision, not a bug fix.

Both remain relevant to how tight Task 2's tolerances can get — the system stays underdetermined (288 points, 960 coefficients) even after this fix, so **do not expect near-exact recovery**.

---

## File structure

| File | Responsibility |
|---|---|
| `calibrate/src/correction_surface.rs` (modify) | The one-line base-case fix, plus unit tests for the basis and a fitted-surface regression test. |
| `calibrate/tests/cli_full_mode_e2e.rs` (modify) | Tighten D12's two tolerances to the newly measured values. |
| `docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md` (modify) | Correct the false "both implementations" claim; record the real mechanism and the resolution. |
| `docs/roadmap-2026-07.md`, `docs/roadmap-2026-07-work-units.md` (modify) | Record the unit as done; retire the risk entry D12 added. |

---

### Task 1: Fix the endpoint in the B-spline basis

**Goal:** A query at the exact maximum of any axis evaluates correctly, and a fitted surface no longer collapses across the top knot span.

**Files:**
- Modify: `calibrate/src/correction_surface.rs` — `bspline_basis` (~line 408) and the `#[cfg(test)] mod tests` block (~line 1096)

**Acceptance Criteria:**
- [ ] Partition of unity holds to 1e-12 at every corner and face of the domain, including all three axes at max simultaneously
- [ ] Partition of unity still holds at interior points and at the axis minima (no regression)
- [ ] A surface *fitted* to a known constant recovers that constant at the domain maximum, not just at interior points
- [ ] The existing `calibrate` test suite passes unchanged
- [ ] `antenna-model`'s tests pass unchanged (no change was made there, but the 3D→4D round-trip in `artifact_export.rs` compares the two)

**Verify:** `cargo test -p calibrate --lib correction_surface -- --nocapture` → all pass, including the three new tests

**Steps:**

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `calibrate/src/correction_surface.rs`:

```rust
    // ========================================================================
    // Endpoint evaluation — see
    // docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md
    // ========================================================================

    /// Clamped knot vector on `[lo, hi]` with `n_internal` evenly spaced internal knots.
    fn clamped_knots(lo: f64, hi: f64, n_internal: usize, order: usize) -> Vec<f64> {
        let mut k = vec![lo; order];
        for i in 1..=n_internal {
            k.push(lo + (hi - lo) * i as f64 / (n_internal + 1) as f64);
        }
        k.extend(std::iter::repeat(hi).take(order));
        k
    }

    /// A surface whose coefficients are all 1.0. A correct B-spline basis is a partition
    /// of unity, so this must evaluate to exactly 1.0 everywhere in its domain — including
    /// on the boundary.
    fn unit_surface(order: usize) -> CorrectionSurface {
        let knots_frequency = clamped_knots(400.0, 700.0, 2, order);
        let knots_econe = clamped_knots(0.0, 24.0, 4, order);
        let knots_eclock = clamped_knots(0.0, 315.0, 6, order);
        let shape = [
            knots_frequency.len() - order,
            knots_econe.len() - order,
            knots_eclock.len() - order,
        ];
        CorrectionSurface {
            coefficients: vec![1.0; shape[0] * shape[1] * shape[2]],
            shape,
            knots_frequency,
            knots_econe,
            knots_eclock,
            spline_order: order,
            fit_stats: FitStatistics {
                num_points: 0,
                rmse_db: 0.0,
                max_residual_db: 0.0,
                r_squared: 0.0,
                cross_validation_rmse: None,
                improvement_percent: 0.0,
            },
        }
    }

    /// The regression this fixes: the basis was a partition of unity everywhere *except*
    /// at the exact maximum of an axis, where every basis function evaluated to zero.
    /// Measured before the fix: 1.000000000 at t=0.9999, 0.000000000 at t=1.0.
    #[test]
    fn basis_is_a_partition_of_unity_on_every_face_and_corner() {
        let s = unit_surface(4);
        let (f_lo, f_hi) = (400.0, 700.0);
        let (c_lo, c_hi) = (0.0, 24.0);
        let (k_lo, k_hi) = (0.0, 315.0);
        let f_mid = 0.5 * (f_lo + f_hi);
        let c_mid = 0.5 * (c_lo + c_hi);
        let k_mid = 0.5 * (k_lo + k_hi);

        // Interior, all six faces, and all eight corners.
        let mut probes = vec![
            ("interior", f_mid, c_mid, k_mid),
            ("freq min face", f_lo, c_mid, k_mid),
            ("freq MAX face", f_hi, c_mid, k_mid),
            ("cone min face", f_mid, c_lo, k_mid),
            ("cone MAX face", f_mid, c_hi, k_mid),
            ("clock min face", f_mid, c_mid, k_lo),
            ("clock MAX face", f_mid, c_mid, k_hi),
        ];
        for &f in &[f_lo, f_hi] {
            for &c in &[c_lo, c_hi] {
                for &k in &[k_lo, k_hi] {
                    probes.push(("corner", f, c, k));
                }
            }
        }

        for (label, f, c, k) in probes {
            let got = s.evaluate(f, c, k).expect("evaluate");
            assert!(
                (got - 1.0).abs() < 1e-12,
                "{label} ({f}, {c}, {k}): basis summed to {got:.12}, not 1.0 — \
                 the B-spline basis is not a partition of unity there"
            );
        }
    }

    /// Approaching the maximum must not be discontinuous with reaching it.
    #[test]
    fn basis_is_continuous_up_to_the_maximum() {
        let s = unit_surface(4);
        for &f in &[699.0_f64, 699.9, 699.99, 699.999, 699.999_999, 700.0] {
            let got = s.evaluate(f, 12.0, 180.0).expect("evaluate");
            assert!(
                (got - 1.0).abs() < 1e-12,
                "at frequency {f} the basis summed to {got:.12}, not 1.0"
            );
        }
    }

    /// The consequence that actually mattered: because the fitter uses the same basis, a
    /// measurement sitting exactly on an axis maximum contributed an all-zero row to the
    /// normal equations, so the last coefficient got no data support and collapsed to ~0
    /// under regularization — corrupting the whole top knot span, not just the endpoint.
    /// A basis-only test would pass while the fit stayed broken, so assert on a FIT.
    #[test]
    fn a_fitted_constant_is_recovered_at_the_domain_maximum() {
        // Deliberately OVERdetermined: 7^3 = 343 points against
        // (2+4)^3 = 216 coefficients. The shipped 4/6/8 configuration is
        // underdetermined (288 points, 960 coefficients), which degrades the fit for a
        // separate, still-open reason — this test must isolate the endpoint behaviour,
        // so it must not also be starved of data.
        let freqs: Vec<f64> = (0..7).map(|i| 400.0 + 50.0 * i as f64).collect();
        let cones: Vec<f64> = (0..7).map(|i| 4.0 * i as f64).collect();
        let clocks: Vec<f64> = (0..7).map(|i| 52.5 * i as f64).collect();

        let mut measurements = Vec::new();
        let mut predictions = Vec::new();
        for &f in &freqs {
            for &c in &cones {
                for &k in &clocks {
                    // Residual is a constant 1.5 dB, which a B-spline represents exactly.
                    measurements.push(MeasurementPoint::new(k, c, f, 1.5, 100.0));
                    predictions.push(0.0);
                }
            }
        }
        assert_eq!(measurements.len(), 343);

        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 2,
            num_knots_econe: 2,
            num_knots_eclock: 2,
            regularization: 1e-9,
            adaptive_knots: false,
            cross_validation_folds: 0,
            min_knot_spacing_frequency: 50.0,
            min_knot_spacing_econe: 2.0,
            min_knot_spacing_eclock: 5.0,
        };
        let surface = fit_correction_surface(&measurements, &predictions, &params)
            .expect("fitting a constant must succeed");

        for (label, f, c, k) in [
            ("interior", 550.0, 12.0, 180.0),
            ("frequency at MAX", 700.0, 12.0, 180.0),
            ("frequency just under MAX", 699.99, 12.0, 180.0),
            ("cone at MAX", 550.0, 24.0, 180.0),
            ("clock at MAX", 550.0, 12.0, 315.0),
            ("all three at MAX", 700.0, 24.0, 315.0),
        ] {
            let got = surface.evaluate(f, c, k).expect("evaluate");
            assert!(
                (got - 1.5).abs() < 1e-3,
                "{label}: fitted constant recovered as {got:.6}, expected 1.5"
            );
        }
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p calibrate --lib correction_surface 2>&1 | tail -20`

Expected: `basis_is_a_partition_of_unity_on_every_face_and_corner` fails on a MAX face (reporting `0.000000000000`), `basis_is_continuous_up_to_the_maximum` fails at 700.0, and `a_fitted_constant_is_recovered_at_the_domain_maximum` fails on the MAX probes.

If any of the three *passes* before the fix, stop and report — the reproduction has drifted and the rest of this plan's premise needs rechecking.

- [ ] **Step 3: Fix the base case**

Replace the `k == 1` branch of `bspline_basis` in `calibrate/src/correction_surface.rs`:

```rust
fn bspline_basis(i: usize, k: usize, t: f64, knots: &[f64]) -> f64 {
    if k == 1 {
        // Base case: characteristic function of the half-open span [knots[i], knots[i+1]).
        if i < knots.len() - 1 && t >= knots[i] && t < knots[i + 1] {
            return 1.0;
        }
        // The domain maximum needs the last non-degenerate span to be closed on the
        // right, or no basis function is non-zero there at all.
        //
        // This is not cosmetic. `accumulate_normal_equations` evaluates the basis at
        // every measurement, so before this a point sitting exactly on an axis maximum
        // contributed an all-zero row: the last coefficient in that axis got no data
        // support and was driven to ~0 by the ridge term, corrupting the fit across the
        // entire top knot span rather than just at the endpoint. On a regular grid the
        // maximum always has data on it. See
        // docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md.
        //
        // The previous attempt at this keyed on `i == knots.len() - 2`, which for a
        // clamped knot vector is a padding index outside the valid basis range
        // `0..knots.len() - order`, so it never fired for a basis function that is
        // actually evaluated.
        if i + 1 < knots.len()
            && t == knots[knots.len() - 1]
            && knots[i + 1] == t
            && knots[i] < knots[i + 1]
        {
            return 1.0;
        }
        return 0.0;
    }
    ...
```

Leave the recursive case untouched.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p calibrate --lib correction_surface`
Expected: PASS, including the three new tests.

Then the whole crate and the workspace:

```bash
cargo test -p calibrate
cargo test -p antenna-model
```

`antenna-model` was not modified, but `artifact_export.rs`'s 3D→4D round-trip compares the two implementations — it should still pass, and now for the right reason (they agree because both are correct, rather than because both were wrong in the same way).

- [ ] **Step 5: Commit**

```bash
cargo fmt
cargo clippy -p calibrate --all-targets -- -D warnings
git add calibrate/src/correction_surface.rs
git commit -m "fix: evaluate the B-spline basis correctly at a domain maximum

bspline_basis's k==1 base case used a half-open span, so at t == t_max no
basis function was non-zero and the basis was not a partition of unity there.
The existing right-endpoint special case keyed on i == knots.len() - 2, which
for a clamped knot vector is a padding index outside the valid basis range, so
it never fired for a basis function that is actually evaluated.

The evaluation error was the minor half. Because accumulate_normal_equations
uses the same basis, every measurement lying exactly on an axis maximum
contributed an all-zero row to the normal equations -- and on a regular grid
the maximum always has data on it (72 of 288 rows in D12's fixture sit at
700 MHz). The last coefficient in that axis therefore had no data support and
was driven to ~0 by the ridge term, corrupting the fit across the entire top
knot span. That is why a query at 699.999 returned 0.000090 while the basis
there was already ~1.0.

Pinned by three tests: partition of unity at every face and corner, continuity
up to the maximum, and -- the one that would have caught the real damage -- a
constant residual fitted and then recovered at the domain maximum.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: Re-measure D12's end-to-end numbers and tighten its tolerances

**Goal:** D12's two deliberately weakened assertions are tightened to what the fixed fitter actually achieves, making the improvement visible and preventing silent regression.

**Files:**
- Modify: `calibrate/tests/cli_full_mode_e2e.rs` — the improvement ceiling in `cli_full_mode_correction_beats_the_uncorrected_model` and `BIAS_RECOVERY_TOLERANCE_DB`

**Acceptance Criteria:**
- [ ] Both the before and after values are recorded in the commit message
- [ ] The improvement ceiling is tightened to just above the newly measured `corrected_rmse`
- [ ] `BIAS_RECOVERY_TOLERANCE_DB` is tightened to just above the newly measured worst-case probe error
- [ ] Both doc comments are rewritten: they currently blame the edge-collapse defect, which is now fixed — they must instead state what still limits recovery
- [ ] The whole suite passes: 11 passed, 1 ignored

**Verify:** `cargo test -p calibrate --test cli_full_mode_e2e -- --nocapture` → 11 passed, 1 ignored, with the new numbers printed

**Steps:**

- [ ] **Step 1: Measure the new values**

Run: `cargo test -p calibrate --test cli_full_mode_e2e -- --nocapture 2>&1 | grep -E "probe f=|model-only RMSE"`

Record the printed model-only RMSE, corrected RMSE, and all four per-probe errors.

**Pre-fix values, for the before/after record:**
- model-only RMSE **1.3071 dB**, corrected **0.9756 dB** (ratio 0.746, 25.4% improvement)
- probe errors: **0.5928** (450 MHz, 3°, 30°), **0.0934** (550, 7, 120), **0.0365** (570, 14, 200), **0.0934** (500, 10, 260)
- improvement ceiling was `corrected < 0.9756 + 0.02`; `BIAS_RECOVERY_TOLERANCE_DB` was `0.65`

Two of the tests will now **fail**, because both assertions are upper bounds pinned to the old, worse numbers. That is the fix working. Read the new values from the failure messages and the printed output.

- [ ] **Step 2: Tighten the improvement ceiling**

In `cli_full_mode_correction_beats_the_uncorrected_model`, replace the ceiling constant with the newly measured `corrected_rmse` plus the same absolute `0.02` dB epsilon, and rewrite the comment. It currently reads as an apology for the edge-collapse defect; it must now say what the *remaining* limit is — the system is still underdetermined at 288 points against 960 coefficients (`(4+4)(6+4)(8+4)` for the shipped 4/6/8 knot counts at order 4), which is tracked separately as the fitter's data-sufficiency check testing the wrong quantity.

Keep the epsilon absolute rather than proportional, and keep the existing justification: the pipeline is deterministic to 4 decimal places across debug and release, so the epsilon covers cross-platform libm ULP differences, not run-to-run variance.

- [ ] **Step 3: Tighten the recovery tolerance**

Set `BIAS_RECOVERY_TOLERANCE_DB` to just above the newly measured worst-case probe error, preserving a similar proportional margin to the previous 0.65 vs 0.5928 (about 10%).

Rewrite its doc comment with the new per-probe numbers. **Sanity-check that the tolerance still discriminates:** the injected bias at the four probes is 1.4529, 0.8667, 0.7529 and 1.0291 dB, so a surface that fitted nothing would return 0 and miss by those amounts. If the new tolerance exceeds the smallest of them (0.7529), the test can no longer distinguish "fitted well" from "fitted nothing" — say so and stop rather than shipping a vacuous assertion.

- [ ] **Step 4: Verify**

```bash
cargo test -p calibrate --test cli_full_mode_e2e -- --nocapture
./scripts/check.sh
```
Expected: 11 passed, 1 ignored; then "All gate checks passed."

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add calibrate/tests/cli_full_mode_e2e.rs
git commit -m "test: tighten D12's tolerances now the endpoint fix has landed

D12 had to leave two assertions weak, both bounded by the endpoint defect it
filed. With that fixed, both are retightened to what the fitter now achieves.

Improvement ceiling: corrected RMSE <BEFORE> dB -> <AFTER> dB.
Bias recovery: worst-case probe error <BEFORE> dB -> <AFTER> dB, tolerance
<OLD> -> <NEW> dB.

Both doc comments rewritten: they previously attributed the shortfall to the
edge collapse, which no longer applies. What still limits recovery is that the
fit remains underdetermined -- 288 fixture points against 960 coefficients for
the shipped 4/6/8 knot configuration -- which is tracked separately.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

Replace every `<BEFORE>`/`<AFTER>`/`<OLD>`/`<NEW>` with the real measured values before committing.

---

### Task 3: Correct the findings doc and update the roadmap

**Goal:** The findings doc stops asserting something false about the service-side interpolator, and the roadmap records the fix.

**Files:**
- Modify: `docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md`
- Modify: `docs/roadmap-2026-07.md` — the section 7 risk entry added by D12
- Modify: `docs/roadmap-2026-07-work-units.md` — add the unit and mark it done

**Acceptance Criteria:**
- [ ] The findings doc no longer claims the service-side 4D `evaluate_correction` is defective
- [ ] It records the real mechanism (half-open base case; the fitting-stage damage) and the partition-of-unity evidence for the 4D interpolator being correct
- [ ] It carries a RESOLVED header naming the fixing commit
- [ ] The section 7 risk entry for the edge collapse is retired (struck through with a resolution note, matching how the document retires other risks)
- [ ] The two deliberately-out-of-scope items (multiplicity check, data-sufficiency quantity) are still listed as open, and the reason each was excluded is stated
- [ ] Nothing claims the fix made the system well-determined — it did not

**Verify:** `grep -n "service-side 4D" docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md` → every remaining mention says it is correct, not defective. Then `./scripts/check.sh` → "All gate checks passed."

**Steps:**

- [ ] **Step 1: Correct the findings doc**

Add a `**RESOLVED 2026-07-30**` block at the top naming the fixing commit. Then correct the body:

- The "The defect" section currently says the 4D interpolator "has the **same behavior**". Replace with the verified truth: the 4D interpolator uses the standard NURBS Cox-de Boor recurrence with a correctly clamping `find_knot_span`, and is a partition of unity at every boundary including all four axes at max simultaneously (1.000000000 in each case, with coefficients built by hand rather than by the fitter). Its apparent failure in the original diagnostic was inherited corrupted coefficients.
- The reproduction table's 4D rows must be corrected or removed — they attribute to the interpolator a failure that belonged to the fitter.
- The "Why it went unnoticed" section's first point (the two implementations agreeing) needs revising: the round-trip test passed because the *coefficients* were shared, not because both algorithms were wrong. The point about it only sampling interior points still stands.
- Add the real mechanism: the half-open `k == 1` base case, the never-firing `i == knots.len() - 2` special case, and — the important part — that the fitter uses the same basis, so max-edge measurements contributed all-zero rows and starved the last coefficient.
- Keep the two refuted hypotheses section as-is. It is still correct and still valuable.

- [ ] **Step 2: Retire the roadmap risk**

In `docs/roadmap-2026-07.md` section 7, find the risk entry D12 added about the upper-edge collapse. Strike it through and append a resolution note naming the date and commit, matching the style the document already uses for resolved risks (e.g. the "Resolved 2026-07-09 by Phase 0 (G1)" entries).

Do **not** retire the parts that remain true: the fit is still underdetermined, `--tune-parameters` is still broken, and no `.bin` artifact ships yet.

- [ ] **Step 3: Add the unit to the work-units doc**

Add a short unit section in Phase 4 recording this work — the mechanism, the fix, D12's before/after tolerance numbers from Task 2, and the two items deliberately left open. Follow the voice of the D10/D11/D12 sections.

- [ ] **Step 4: Verify and commit**

```bash
./scripts/check.sh
git add docs/
git commit -m "docs: correct the edge-collapse finding and record the fix

The findings doc claimed the service-side 4D evaluate_correction shared the
defect. It does not -- verified by partition of unity at every boundary,
including all four axes at max, with hand-built coefficients: 1.000000000 in
every case. It uses the standard NURBS Cox-de Boor recurrence with a correctly
clamping span lookup. Its apparent failure in the original diagnostic was
inherited corrupted coefficients from the broken fitter.

Records the real mechanism and retires the risk entry, while keeping what is
still open: the fit remains underdetermined, --tune-parameters is still
non-functional, and no artifact ships yet.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Risks

**The fix may not be sufficient on its own.** It restores data support to the last coefficient of each axis, but the system stays underdetermined (288 points vs 960 coefficients). Task 2 measures rather than assumes; if the improvement is small, that is information, not failure — report the numbers and set the tolerances to what was actually achieved rather than to what was hoped for.

**The `t == knots[len-1]` float comparison is exact.** That is deliberate and correct here: the fix targets a query landing *exactly* on the stored maximum, which is what a measurement grid produces (the CSV value round-trips to the same `f64` that built the knot vector). A tolerance-based comparison would widen the closed-right span and change results just inside the boundary. Do not "improve" this to an epsilon comparison.

**Do not touch the recursive case of `bspline_basis`.** Its zero-denominator guards are what make repeated (clamped) knots work at all; the endpoint problem is entirely in the base case.
