# F7 — Statistical Sidelobe Floor (Ruze Scatter Floor) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement roadmap unit **F7** — make off-axis (sidelobe) predictions *envelope-conservative* instead of systematically optimistic (~8–13 dB below the ITU-R S.580 mask today, per `docs/domain-contract.md` "Off-axis pattern / sidelobe fidelity"). Add an angle-dependent **Ruze scattered-power floor** derived from each antenna's `surface_rms`, applied `max(pattern, floor)` at the existing spillover seam in `pattern.rs::compute_gain`, gated to the **uncalibrated path only** (P1's double-counting gate), and **validated as a one-sided conservative bound** against the F8 reference sidelobe datasets. Bumps P1b's `physics_model_version` (2 → 3).

**Architecture:** The floor is a wide-angle directivity pedestal representing the power the scalar Ruze efficiency removes from boresight. Surface errors are lossless phase errors, so the fraction `(1 − η_ruze)` of captured power does not vanish — it re-radiates as an error pattern over wide angles. The model currently applies `η_ruze` as a scalar gain-loss and lets the pattern keep diving into its nulls (no error floor). F7 adds `floor(θ) = f(1 − η_ruze, Ω_scatter)` and takes `max(pattern, floor)` **without touching the aperture integral** (which also sidesteps the numerical infeasibility of integrating far sidelobes for electrically huge dishes). One free shape constant — the effective scatter solid angle `Ω_scatter` (equivalently a surface correlation length) — is **calibrated once against the reference data** and pinned by the harness; the per-antenna *magnitude* comes from that antenna's own `surface_rms` via `η_ruze`, so the floor generalizes across dishes/bands without a new per-antenna config field.

**Tech Stack:** Rust workspace (antenna-model + calibrate), existing reference-validation harness (`antenna-model/tests/reference_validation.rs`), F8 datasets under `antenna-model/tests/fixtures/reference_datasets/sidelobe_data/`.

**Branch:** `feat/f7-sidelobe-floor` off `main` (created in Task 1, Step 1).

---

## User decisions (register row F7 — Decided 2026-07-12 by maintainer)

Resolved via the three gating questions asked while planning:

1. **Floor mechanism → Ruze scatter floor (physics-derived).** Floor magnitude anchored to `surface_rms` through `(1 − η_ruze)`; validate the *level* against the F8 NTIA/NASA datasets. Chosen over an empirical curve-fit (no physical parameter, poor extrapolation outside C-band D/λ 35–267) and over the "+ ITU-mask envelope output mode" (deferred; see "Out of scope / follow-ups").
2. **Exposure → in-place for uncalibrated (like spillover).** The returned gain becomes `max(pattern, floor)` in the sidelobe region on antennas with **no correction surface**; calibrated antennas are untouched (their correction surface absorbs real sidelobe behavior within coverage). Matches the roadmap seam design and the P1 spillover precedent. Rejected: separate envelope response field (leaves the optimistic number as the primary gain; adds C8-contract schema surface).
3. **Validation bar → conservative envelope (never optimistic).** The harness asserts the floored prediction is **at or above** measured sidelobe peaks (within the data's stated uncertainty) across the reference set — a true one-sided upper bound, not a two-sided percentile fit. `Ω_scatter` is calibrated to the *smallest* floor that still bounds the data conservatively (tight upper bound, not gratuitously inflated).

**Planner defaults applied (flagged, not maintainer-confirmed — override if desired):**
- **No new per-antenna config field.** A single module-level `Ω_scatter` constant (data-calibrated) is used, not a per-antenna correlation length. Alternative — add `surface_correlation_length_m` to the config for per-antenna fidelity — is documented as a follow-up, not built here.
- **Flat wide-angle pedestal shape.** The floor is modeled as a constant-dBi pedestal (NTIA shows the wide-angle floor is roughly flat and roughly D/λ-independent). The near-in region, where the physics pattern already exceeds the pedestal, is unchanged — the `max()` only lifts the deep-null far tail. A rolled-off floor shape is a follow-up if the data demands it.
- **P8 warning retained, message revised** (Task 5) rather than removed: the served number is now a conservative envelope, but uncalibrated off-axis levels are still not calibrated-grade.

---

## Verified current state (2026-07-12, `main` @ `1666e8c`)

- **Seam is where the roadmap says.** `pattern.rs::compute_gain` folds spillover in at lines ~292–299, then `apply_gain_floor` at ~302, and returns. `theta` and `config` (hence `config.reflector.surface_rms`) are in scope throughout. The F7 floor slots in **between** the spillover fold-in and `apply_gain_floor`.
- **Gate mechanism exists and is reusable.** `IntegrationParams` has `apply_spillover: bool` (default `false` in all three presets, `integration.rs:48/65/80`). The evaluator sets `integration_params.apply_spillover = calibration.correction_surface.is_none()` at `evaluator.rs:218`. F7 adds a sibling `apply_sidelobe_floor: bool` threaded identically.
- **Cross-endpoint gate sites.** The spillover gate is mirrored on the heatmap and H3 paths (P1 commit `09d0700`). F7 must set `apply_sidelobe_floor` at the **same** sites — grep `apply_spillover` in `src/service/` finds them: `evaluator.rs:218,328`, plus the heatmap/h3 builders. (Re-grep before editing; verify each.)
- **Reference-gain path.** `evaluator.rs:304–333` recomputes an ideal boresight (θ=0) reference and matches its `apply_spillover` to the actual result to cancel base spillover in `loss_db`. The floor is **inert at θ=0** (pattern ≫ floor at boresight, so `max` = pattern), so it does not perturb `reference_gain_db`/`loss_db` — but set `reference_params.apply_sidelobe_floor` consistently anyway for clarity and future-proofing, and add an assertion that the boresight reference is unchanged.
- **Feature is NOT inert on served antennas.** All four enabled uncalibrated design-spec antennas carry nonzero `surface_rms` (`antennas.yaml`: gs_3.7m 1.5 mm, dsn_13m 0.4 mm, dsn_34m 0.25 mm, dsn_70m 0.5 mm; gbt_100m below). Worked example: gs_3.7m at X-band (λ≈35.7 mm) → `4π·1.5/35.7 = 0.528`, `η_ruze = exp(−0.528²) ≈ 0.756`, so **~24% of power scatters** — a materially nonzero floor. Contrast with P1's initial "negligible" finding: F7 changes real served sidelobe numbers. **Bump `physics_model_version`.**
- **`ruze_efficiency(surface_rms, wavelength)` exists** (`pattern.rs:116`) — reuse it; do not re-derive `η_ruze`.
- **`PHYSICS_MODEL_VERSION = 2`** (`model/mod.rs:45`), stamped by P1b, bumped to 2 by P7. F7 bumps it to **3** (changes `gain_physics` for identical inputs in the sidelobe region).
- **P8 warning is live and independent.** `off_axis_unvalidated_warning` (`evaluator.rs:506`, threshold 3× first-null = `OFF_AXIS_FIRST_NULL_MULTIPLE · FIRST_NULL_COEFFICIENT · λ/D`) warns on uncalibrated off-axis queries. F7 does not remove it; Task 5 revises its wording.
- **S.580 shape test is unaffected by default.** `itu_r_s580_sidelobe_envelope_small_dish` and `itu_probe_fine_envelope` build their own config and call the model with default `IntegrationParams` (flag `false`), so they keep validating the *un-floored* pattern shape. The **new** F7 test explicitly enables the flag. Confirm this in Task 3 (do not let the floor silently break the shape guard).
- **F8 datasets present** at `antenna-model/tests/fixtures/reference_datasets/sidelobe_data/`:
  - `ntia_84_164_sidelobe_statistics.psv` — **absolute dBi** percentile bins (`max/p90/median/p10/min`), C-band, D/λ 35–267, 1°–180°. Primary calibration set (absolute → compares to the floor directly; near D/λ-independent → validates the physics prediction that the absolute floor is set by surface quality, not aperture size).
  - `nasa_cr159703_pattern_peaks.psv` — sidelobe peaks **`level_db_rel_peak`** (rel to main-beam apex), 1.22/1.83 m at 12 GHz, with `surface_condition`/`defocus` provenance. Cross-check of the `surface_rms` scaling; convert rel→absolute using each cut's peak gain (antenna table / text gains in the file header) before comparing.
  - Located in a **separate subdirectory** from the peak-gain `.psv` files precisely so `load_all_reference_points` does not auto-ingest them — the F7 test loads them with a dedicated parser (they have different columns).

## Standing rules that bind every task

1. `cargo test --workspace` after any change under `antenna-model/src/model/` (work-units standing rule 3).
2. **This is a sanctioned physics change to the sidelobe floor only.** Never touch the aperture integral, the illumination/taper math, the beam-steering/beam-deviation sign conventions (`coordinates.rs`), or any lateral math (standing rule 2). The floor is a post-integration `max()`.
3. `docs/domain-contract.md` changes land in the same commit as the code they describe (contract rule). Task 5 is the full pass; earlier tasks carry any small inline edits they necessitate.
4. If any **existing** test changes value for a reason other than "an uncalibrated antenna's deep-sidelobe gain was lifted to the floor," stop and investigate — do not adjust the assertion. In particular, boresight/main-beam gains, all calibrated-antenna outputs, and the S.580 shape test must be byte-for-byte unchanged.
5. `openapi.yaml` is hand-maintained (standing rule 4): mirror it only if a response schema field is added. This plan's default adds **no** request/response field, so no OpenAPI change is expected — confirm and state so in the final PR.

---

### Task 1: Ruze scatter-floor mechanism in the model layer

**Goal:** A `sidelobe_floor_gain(config, theta, wavelength)` function returning the linear-gain pedestal, plus a new `apply_sidelobe_floor` flag, applied `max(pattern, floor)` at the seam. No behavior change unless the flag is on.

**Files:**
- Modify: `antenna-model/src/model/integration.rs` — add `pub apply_sidelobe_floor: bool` to `IntegrationParams` (default `false` in all three presets, next to `apply_spillover`).
- Modify: `antenna-model/src/model/pattern.rs` — new floor function (near `ruze_efficiency`/`overall_efficiency`, ~:116–145); apply it at the seam (~:292–302, after spillover, before `apply_gain_floor`).

**Design (the floor formula):**
- Scattered power fraction: `p_scatter = 1 − ruze_efficiency(config.reflector.surface_rms, wavelength)` (reuse the existing fn). Zero surface RMS → `p_scatter = 0` → floor `= 0` (linear) → `max()` is a no-op. Good degenerate behavior.
- Pedestal directivity (linear, relative to isotropic): `floor_lin = p_scatter · (4π / Ω_SCATTER)`, where `Ω_SCATTER` is a module-level `const` (steradians) — the single data-calibrated shape constant (Task 3 fixes its value; start with a documented placeholder and a `// CALIBRATED IN TASK 3` marker). Rationale: the scattered fraction of total radiated power spread over `Ω_SCATTER` steradians has mean directivity `4π·p_scatter/Ω_SCATTER`. `Ω_SCATTER` is independent of D, so the absolute floor is set by surface quality — matching NTIA's near-D/λ-independent wide-angle floor.
- Shape: flat pedestal (θ-independent magnitude) for the first cut. Keep the `theta` parameter in the signature so a rolled-off shape can be added later without a signature change; document that it is currently unused except possibly to suppress the floor inside the main beam (see next bullet).
- **Main-beam guard:** the floor must never lift the main beam or near-in region — but since `max(pattern, floor)` only raises where `pattern < floor`, and the main beam is ≫ any plausible pedestal, no explicit angular gate is strictly required. Still, add a cheap guard that the floor is only *considered* outside the first null (reuse P8's `FIRST_NULL_COEFFICIENT · λ/D` if convenient) to make the intent explicit and avoid any pathological interaction with a very-low-gain antenna near boresight. Document the choice.
- Apply at the seam: `let gain = if params.apply_sidelobe_floor { gain.max(sidelobe_floor_gain(config, theta, wavelength)) } else { gain };` **before** `apply_gain_floor`.

**Acceptance Criteria:**
- [ ] `IntegrationParams.apply_sidelobe_floor` exists, defaults `false` in `default()`/`fast()`/`accurate()` (or whatever the three presets are); no existing test changes value.
- [ ] `sidelobe_floor_gain` returns `0.0` (linear) when `surface_rms == 0.0`, and a positive pedestal scaling monotonically with `surface_rms` (unit tests at two RMS values).
- [ ] With the flag **on**, a deep-null angle for a nonzero-`surface_rms` antenna returns `max(pattern, floor)` (test: gain at a deep null is lifted to ≈ the pedestal; gain at boresight and in the main beam is **unchanged** to full precision).
- [ ] With the flag **off** (default), gain at every angle is byte-for-byte the pre-F7 value (test).
- [ ] `Ω_SCATTER` is a single documented `const` with a `// CALIBRATED IN TASK 3` marker and a doc comment stating the physical meaning and bump implications.
- [ ] `cargo test --workspace` green.

---

### Task 2: Service-layer gate + cross-endpoint plumbing

**Goal:** Turn the floor on exactly where spillover is on — uncalibrated antennas (no correction surface) — on all four compute paths, with the reference-gain path handled consistently.

**Files:**
- Modify: `antenna-model/src/service/evaluator.rs` — set `integration_params.apply_sidelobe_floor = calibration.correction_surface.is_none();` alongside the spillover gate (~:218); set `reference_params.apply_sidelobe_floor` consistently (~:327–328).
- Modify: `antenna-model/src/service/heatmap.rs` and `antenna-model/src/service/h3_link_budget.rs` — mirror the gate at the same sites the P1 spillover gate lives (grep `apply_spillover` in `src/service/`; set the new flag identically at each).

**Design constraints:**
- **Double-counting gate:** floor applies only when `correction_surface.is_none()` — whole-antenna gate, never per-query. Do not floor out-of-coverage points on calibrated antennas (that would create a discontinuity at the coverage boundary — same rule as P1 spillover).
- **Boresight reference unchanged:** the reference is computed at θ=0 where the floor is inert; assert `reference_gain_db` is identical with the flag on vs off for a nonzero-`surface_rms` fixture.
- **No new response field by default** (maintainer chose in-place, not a separate envelope field). If the executor finds it valuable to signal "floor applied" to consumers, that is a *warning* (reuse existing plumbing / P8 message revision in Task 5) or a C8-stage-3 typed-warning code — **not** a new schema field in this unit.

**Acceptance Criteria:**
- [ ] For an uncalibrated antenna, a deep-sidelobe query on gain/batch/heatmap/h3 returns the floored value (one test per endpoint, mirroring P1's cross-endpoint tests).
- [ ] For a calibrated antenna (correction surface present), **every** output is unchanged vs pre-F7 (explicit before/after test on a calibrated fixture; existing calibrated tests untouched).
- [ ] `reference_gain_db`/`loss_db` unchanged by the floor (boresight-reference invariance test).
- [ ] `cargo test --workspace` green.

---

### Task 3: Calibrate `Ω_SCATTER` and add the conservative-envelope harness test

**Goal:** Fix the single free constant against the F8 data and pin a regression test asserting the floored envelope is a one-sided conservative bound on measured sidelobe peaks.

**Files:**
- Modify: `antenna-model/tests/reference_validation.rs` — add a dedicated parser for the two `sidelobe_data/*.psv` schemas (distinct columns from the peak-gain files) and a new `#[test] sidelobe_floor_conservative_envelope`.
- Modify: `antenna-model/src/model/pattern.rs` — set `Ω_SCATTER` to the calibrated value (replace the Task 1 placeholder).

**Method:**
1. **Primary set — NTIA (absolute dBi).** For each antenna class / band / angular bin, compute the model's floored **absolute** sidelobe gain (build a representative config with the class's D and a plausible `surface_rms`; the model floor is absolute dBi, directly comparable). Calibrate `Ω_SCATTER` to the **smallest** floor such that the floored prediction is ≥ the measured **peak** envelope (the `max_dbi`, or `p90_dbi` per the S.580 10%-exceedance convention — pick and document which; `p90` is the natural "≥90% of peaks bounded" target) minus the bin's uncertainty, across the reference set. Smallest-that-still-bounds ⇒ tight upper bound, not gratuitous inflation.
2. **Cross-check — NASA (rel-to-peak, surface provenance).** Convert `level_db_rel_peak` to absolute using each cut's peak gain (header text gains), then verify the *same* `Ω_SCATTER` conservatively bounds these independently-measured peaks — and that the floor's `surface_rms` scaling tracks the `surface_condition`/`defocus` progression in the data (worse surface → higher measured floor → higher predicted floor). This is the physics-validation payoff: the floor rises with surface error as the data does.
3. **Assert** (the regression test): for ≥90% of reference points, `floored_prediction_dbi ≥ measured_peak_dbi − uncertainty_db`; report the exceedance count and worst margin in the failure message (mirror the existing harness's `--nocapture` reporting style).

**Acceptance Criteria:**
- [ ] `Ω_SCATTER` set to the calibrated value with a comment citing the datasets and the fit criterion.
- [ ] `sidelobe_floor_conservative_envelope` passes: floored prediction bounds ≥90% of NTIA peaks and all NASA cross-check peaks within stated uncertainty; the boresight/main-beam region is excluded from the bound (it is validated by the peak-gain rows, not here).
- [ ] The test **fails** if `Ω_SCATTER` is perturbed to make the floor optimistic (sanity: temporarily double it locally and confirm red) — note this in the PR, don't commit the perturbation.
- [ ] `itu_r_s580_sidelobe_envelope_small_dish` still passes unchanged (the floor flag is off in that test — confirm and state so).
- [ ] `cargo test -p antenna-model --test reference_validation -- --nocapture` output shows the margins.

---

### Task 4: Bump `physics_model_version` (2 → 3)

**Goal:** Record that F7 changes `gain_physics` for identical inputs, per P1b's bump policy.

**Files:**
- Modify: `antenna-model/src/model/mod.rs:45` — `PHYSICS_MODEL_VERSION` 2 → 3; extend the version-history doc comment with the F7 line.

**Acceptance Criteria:**
- [ ] Constant is `3`; version history documents "3 — F7 Ruze sidelobe floor on the uncalibrated path (2026-07)".
- [ ] The P1b loader mismatch-warning test still passes (it compares against the constant; if it hard-codes a version, update the fixture expectation and say so).
- [ ] `cargo test --workspace` green.

---

### Task 5: Docs — contract, api-documentation, roadmap, P8 message

**Goal:** docs = code = behavior; register row and work-unit marked done; P8's honesty warning reconciled with the now-applied floor.

**Files:**
- Modify: `docs/domain-contract.md` — the "Off-axis pattern / sidelobe fidelity" section (~:145–189): record that a **Ruze scattered-power floor is now modeled on the uncalibrated path** (F7, 2026-07-12); keep the "shape validated, near-in first sidelobes still model-limited" caveats; state the floor is a **conservative envelope** (bounds measured peaks, not a best-fit), calibrated against NTIA 84-164 / NASA CR-159703; note it is inert on calibrated antennas and at boresight; cross-reference `Ω_SCATTER` and the new test.
- Modify: `docs/api-documentation.md` — accuracy-caveat section: uncalibrated off-axis gain is now envelope-conservative (upper bound), not optimistic; still not a substitute for calibrated/measured sidelobe data for regulatory interference filings.
- Modify: `antenna-model/src/service/evaluator.rs` — revise the P8 `off_axis_unvalidated_warning` message: the served number is now a **conservative envelope floor**, not a silently optimistic value; direct consumers to calibration data / the ITU mask for precise off-axis work. Keep it a string (C8 stage 3 owns the typed `off_axis_unvalidated` code); update the P8 integration-test expectations if they string-match the old wording.
- Modify: `docs/roadmap-2026-07.md` — register row **F7 → Decided** (2026-07-12; Ruze scatter floor, in-place uncalibrated, conservative-envelope validation) and note implemented; risks/§ text as needed.
- Modify: `docs/roadmap-2026-07-work-units.md` — mark F7 done with the branch/commit, mirroring the P7/P8 "✅ DONE" annotation style; note the two planner defaults (no per-antenna correlation-length field; flat pedestal) so a future reader knows what was and wasn't built.

**Acceptance Criteria:**
- [ ] Contract, api-documentation, roadmap, and work-units updated and internally consistent (no stale "systematically optimistic / no error floor" claims left for the uncalibrated path — reframed as "floor now modeled; near-in first sidelobes and shape still model-limited").
- [ ] P8 message revised and its tests green.
- [ ] `openapi.yaml` unchanged (no schema field added) — or, if the executor added a warning/field, mirrored per standing rule 4 and called out in the PR.
- [ ] Full `cargo test --workspace` green; `scripts/check.sh` (fmt + clippy -D warnings + test) green.

---

## Out of scope / follow-ups (do not build here)

- **ITU-mask envelope output mode** — an optional conservative S.580 envelope as a *separate* output for regulatory screening. Deferred (maintainer chose the physics-derived floor without the extra mode); file as an F7 follow-up if regulatory screening is later needed.
- **Per-antenna surface correlation length** (`surface_correlation_length_m` config field) for per-antenna floor shape/width fidelity instead of the single global `Ω_SCATTER` — **now roadmap unit F9** (register row F9, status **Deferred** 2026-07-12). Additive: F7's floor already carries `theta`, so no rework penalty. **Promotion trigger:** if Task 3's NASA surface-provenance cross-check cannot bound the data across the surface-condition range with a single global `Ω_SCATTER`, flag it and F9 promotes from deferred. Do not build it in this unit.
- **Rolled-off (non-flat) floor shape** — if the data later shows the wide-angle floor is not flat, the `theta` parameter is already in the signature to carry it.
- **Physical sidelobe mechanisms** (edge diffraction, strut/feed-blockage scatter) — out of scope per roadmap §6 regardless of F7; F3 covers blockage geometry when its config parameters exist.
- **Calibrated-path sidelobe modeling** — calibrated antennas keep using their correction surface within coverage; F7 does not touch them.

## Suggested commit sequence (one green workspace per commit)

1. `feat(model): Ruze sidelobe scatter floor + apply_sidelobe_floor flag (F7)` — Task 1.
2. `feat(service): gate sidelobe floor on uncalibrated antennas, all endpoints (F7)` — Task 2.
3. `test(reference-validation): calibrate Ω_scatter; conservative-envelope sidelobe test (F7)` — Task 3.
4. `feat(model): bump physics_model_version 2→3 for the sidelobe floor (F7/P1b)` — Task 4.
5. `docs(F7): contract/api/roadmap + revise P8 off-axis warning wording` — Task 5.
