# Roadmap Work Units — July 2026

Companion to [`roadmap-2026-07.md`](roadmap-2026-07.md) (narrative, themes, decision
register). This document is the execution artifact: bite-sized, criticality-ordered units
of work, each specified so a focused coding agent can execute it in one session with high
likelihood of success.

**Legend**
- **Effort:** S (≤ half a session), M (one session), L (multiple sessions — split before executing).
- **`[DECISION]`:** unit starts by getting a decision-register row (see roadmap §5) decided
  by the maintainer; the recommended default is stated. Do not silently apply the default
  to code without the row being marked Decided.
- File:line references verified 2026-07-08 at `d65f780`. **Re-verify each reference before
  editing** — if a cited line no longer matches its description, stop and re-locate it;
  do not guess.

## Standing rules for all units

1. **Do not trust CLAUDE.md until G2 merges.** Trust code and `docs/domain-contract.md`.
2. **Never change a physics formula, sign, or coefficient in a non-physics unit.** In
   particular, never touch the feed-steering / beam-deviation sign convention
   (`coordinates.rs` negation + BDF) anywhere, in any unit.
3. After any change under `antenna-model/src/model/`, run `cargo test --workspace`, not
   just the touched module's tests.
4. `openapi.yaml` is hand-maintained: any request/response schema change must be mirrored
   there manually until unit C7's drift guard exists.
5. If a doc and the code disagree and no work unit covers it, **stop and file it as a new
   decision item** — never "fix" code to match a doc.
6. All paths are relative to the repo root.
7. Exit criteria are the definition of done. If an exit criterion cannot be met, the unit
   is not done — report why instead of narrowing the criterion.

## Dependency graph

```
G1 ─┬─ G2 ── G3
    ├─ P4, P5, P2 (parallel)      P1 ─┬─ P1b (coordinate w/ D2)
    │                                 └─ P3 ─┐
    │                             P5 ────────┼─ P6 ─ D8, D5
    │  P1b ─ P7;  P8 (independent) ──────────┤
    ├─ S1 ─ S2 ─ S3(after Phase 1) ─ S4 ─ S5 │
    ├─ S6                                    │
    ├─ C3 ─ C4 ─ C2 ─ C8 ─ C7                │
    │  C1(after S6; may fold into C8 stage 4)│
    ├─ D1 ─ D2 ─ D3;  D6                     │
    └─ (Phases 1–3 done) ─ D4 ─ D7
Superseded by C8 (do not implement): S7, C5, C6
Phase 5: F1..F9 (F8 done) gated on register rows (P3, P5/F4, F5, D9, F9); P1 + C8 DECIDED 2026-07-08;
P7 DECIDED 2026-07-10 (auto-refocus), IMPLEMENTED 2026-07-10 (branch
feat/p7-phase-center-auto-refocus; P1b dependency implemented in the same branch);
P8 IMPLEMENTED 2026-07-12 (branch feat/p8-off-axis-honesty-warning);
F7 IMPLEMENTED 2026-07-12 then PARKED 2026-07-13 (inverted premise — see the F7 unit);
UNBLOCKED by P10 2026-07-15, REDESIGN PENDING (D-2) — sequence WITH P10-perf (they interact);
P10 DONE 2026-07-15; post-P10 assessment follow-ups filed 2026-07-15: P10-perf, P10-tail, P11
```

---

## Phase 0 — Guardrails (execute in order: G1 → G2 → G3)

> **STATUS — ✅ Phase 0 COMPLETE, executed & merged to `main` 2026-07-09.**
> G1 `f48b23c` (+ hardening `c13e196`, `4b439c0`), G2 `8c65946`, G3 `c2dceee`. Repo is live
> at github.com/blstoll/antenna-model; CI runs on every push and is green (`rustfmt` +
> `clippy + test` gate; `cargo audit` non-blocking). Extra work beyond the original units,
> driven by the first CI run: committed `Cargo.lock`; `RUST_MIN_STACK` fix for a Linux-only
> libtest stack overflow in the calibrate 3D→4D round-trip test (see D3 follow-up);
> targeted dependency bump clearing 5 advisories (`bf18d60`); two follow-ups filed as GitHub
> issues #1 (D3 stack) and #2 (D6 audit).

### G1 — Stand up CI (ready-to-activate) — Effort: M
**✅ DONE 2026-07-09** — `f48b23c`; hardening in `c13e196` (PR de-dup + concurrency, `Cargo.lock` tracked) and `4b439c0` (`RUST_MIN_STACK`, `checkout@v5`). CI live & green on `main`; local gate is `scripts/check.sh`. Note: HEAD had 27 clippy 1.95.0 lints (not 3) — all mechanical, incl. 10 in `#[cfg(test)]` modules of `src/model/` (maintainer-approved to fix).

- **Entrance criteria / read first:** There is no `.github/workflows/` and **no git remote**
  (verified 2026-07-08). Read: root `Cargo.toml` (workspace members), CLAUDE.md's
  "Code Quality" section, `docs/code-review-checklist.md`, `calibrate/Cargo.toml`
  (ndarray-linalg/OpenBLAS features).
- **Key knowledge:** GitHub Actions for Rust workspaces; BLAS system dependencies.
- **Exit criteria:**
  1. `.github/workflows/ci.yml` committed with jobs: `cargo fmt --check`,
     `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
     `cargo audit` (non-blocking initially, with a tracked allowlist). Activates the moment
     a GitHub remote is added.
  2. A documented local gate (`scripts/check.sh` or a make target) running the same
     commands, **verified green on current HEAD** before merging.
  3. Decision-register row **G1-hosting** filed (default: GitHub).
- **Assumptions:** current HEAD passes `clippy -D warnings`. If it doesn't, fix only
  mechanical lints; defer anything touching `antenna-model/src/model/` semantics and list
  the deferred items in the PR description.
- **Gotchas:** Linux CI needs a BLAS backend for `calibrate` (e.g. `libopenblas-dev`) —
  check `calibrate/Cargo.toml` features before writing the workflow. The macOS
  LDFLAGS/CPPFLAGS note in CLAUDE.md applies to local macOS builds, not Linux CI. Do not
  add auto-fix steps to CI.
- **Depends on:** nothing. **Blocks:** everything else (softly).

### G2 — Make CLAUDE.md true — Effort: S/M
**✅ DONE 2026-07-09** — `8c65946`. All six exit criteria met (live B-spline, Sprints 1–7, deleted-module refs, `antennas.yaml`, property-tests→D7 annotation, precomputed-artifact claim, module map). Also caught & corrected the false "all `antennas.yaml` disabled" claim (4 of 8 are enabled) — see the notes in D9/S5/P1b below.

- **Entrance / read first:** CLAUDE.md in full. Truth sources: `docs/implementation-plan.md`
  (sprints 5–7 marked complete), `antenna-model/src/model/correction_interpolator.rs` +
  `antenna-model/src/service/evaluator.rs:265-287` (B-spline correction is live),
  `ls antenna-model/src/model/`, `calibration_data/antennas.yaml`.
- **Exit criteria:**
  1. No claim that B-spline correction is unimplemented; sprint status matches
     `implementation-plan.md`.
  2. No references to deleted modules `direct_path.rs`, `surface.rs`,
     `numerical_stability.rs` (currently at CLAUDE.md:142-143 and :246).
  3. `antennas.toml` corrected to `calibration_data/antennas.yaml`.
  4. The property-based-tests claim (CLAUDE.md:214) annotated "planned — see roadmap unit D7".
  5. The precomputed-artifacts claim corrected (no `.bin` files ship; see D9).
  6. The module map matches `ls antenna-model/src/**` reality.
- **Gotchas:** Docs-only — zero code changes. Other docs (architecture.md, design doc) are
  unit D5's job; do not drift into them.
- **Depends on:** nothing. **Blocks:** all later agent-executed units (standing rule 1).

### G3 — Fix broken example requests + lock them with a test — Effort: S
**✅ DONE 2026-07-09** — `c2dceee`. Four broken examples fixed to `[w,x,y,z]` arrays (heatmap's non-schema attitude removed); consistency swept across all of `examples/`; drift test `antenna-model/tests/example_requests_deserialize.rs` maps filename→type and panics on any unmapped file (empirically verified). Runs in the G1 gate.

- **Entrance / read first:** `antenna-model/src/api/schemas.rs:276` and `:623`
  (`vehicle_attitude: Option<[f64; 4]>` — **confirm the component order documented in the
  field's doc comment before converting; do not assume w-first**). Broken files:
  `examples/requests/gain_request.json`, `batch_request.json`, `heatmap_request.json`
  (object form `{"w":…,"x":…}`), `gain_request_geodetic.json` (Euler form
  `{"roll_deg":…}`). The newer `geo_*.json` files omit attitude and are fine.
- **Exit criteria:**
  1. Every file in `examples/requests/` deserializes into its corresponding request type.
  2. A test iterates the directory and `serde_json::from_str`s each file against the
     correct schema type (map filename → type explicitly), failing on any new drift.
  3. Test runs in the CI / local gate from G1.
- **Assumptions:** the schema (array quaternion) is correct and the examples are wrong —
  confirmed by audit. Convert object values faithfully; convert the Euler example to an
  equivalent quaternion, or replace with a documented identity attitude if conversion is
  nontrivial.
- **Gotchas:** grep all of `examples/` for `"w":` and `roll_deg` — `curl-examples.sh`,
  `postman_collection.json`, and any Python examples may embed the same broken shapes; fix
  consistently. **Do not change the schema.**
- **Depends on:** G1 (test must run in the gate).

---

## Phase 1 — Prediction correctness & physics scope

### P1 — Spillover efficiency on the uncalibrated path — Effort: M
**[DECIDED 2026-07-08 — staged implement]**

- **Decision (recorded):** The maintainer anticipates missing calibration data for many
  antenna systems, so the unmodeled spillover bias (~0.4–1 dB optimistic) is unacceptable
  on the uncalibrated path. Staged approach: **spillover now** (this unit); **blockage =
  F3** (data-gated on geometry parameters that don't exist in the config yet);
  **cross-pol out of scope** (<0.1 dB on-axis for symmetric prime-focus dishes).
- **Entrance / read first:** `antenna-model/src/model/pattern.rs:130-141`
  (`overall_efficiency` = Ruze × mesh only); `edge_cases.rs:170` (`estimate_spillover` —
  currently computed for warnings only, never multiplied into gain);
  `service/evaluator.rs:265-287` (correction-surface application + calibration-status
  logic); the `q_factor` glossary entry in `docs/domain-contract.md` (this codebase's edge
  taper is the *combined* pattern × space-loss definition — relevant to sanity-checking
  spillover magnitudes).
- **Design constraints (must-follow):**
  1. **Double-counting gate:** apply spillover ONLY when the antenna has **no correction
     surface at all** (whole-antenna gate). Do NOT apply it per-query for out-of-coverage
     points on calibrated antennas — that would create a gain discontinuity at the coverage
     boundary. If boundary behavior should change later, that is a new decision item.
  2. The gate lives in the **service layer** (the evaluator knows calibration state; the
     model layer must not inspect calibration) — thread a flag into the gain-computation
     options rather than importing calibration types into `pattern.rs`.
  3. Keep the existing spillover *warning* path.
- **Exit criteria:**
  1. For an antenna without a correction surface, gain is reduced by the spillover
     efficiency (`10·log10(η_spill)`); a test asserts the applied loss equals what
     `estimate_spillover` predicts for the fixture, plus a *true* sanity bound (loss
     negative and bounded, `(-3, 0)` dB). **CORRECTED 2026-07-09 (during execution):**
     the original "~0.3–1.5 dB for q=8, f/D=0.5" band was WRONG. With the code's existing
     `estimate_spillover`, all four enabled design-spec antennas (q=8–11, f/D=0.4–0.5) are
     heavily over-tapered, so modeled spillover is only ~**0.001–0.05 dB**, not 0.3–1.5 dB.
     The ~0.4–1 dB textbook figure applies to broad feeds (q≈2–4), not these highly
     directive designs. The mechanism is still correct and worth shipping (future-proofs
     broad-feed antennas); its impact on *current* antennas is negligible. See the register
     note on P1.
     **SCOPE REFINEMENT 2026-07-09 (during execution):** spillover is applied only in
     `ComputationMode::StandardPhysicalOptics` (small feed offsets). At large offsets
     (>0.3·f) `estimate_spillover`'s linear offset extrapolation saturates to ~100%,
     which clamped gain to a degenerate −60 dB and crushed 6 realistic off-boresight
     integration scenarios. Those large-offset cases already carry degraded-accuracy
     warnings and now keep their exact pre-P1 gain (maintainer-approved, zero regression).
     A proper large-offset spillover model is F2/ray-tracing territory, not P1.
     **REVISED 2026-07-10 (post-execution):** the 07-09 "negligible" magnitude was itself
     an artifact of the over-tapered q-factors it was measured against. After the
     reference-validation feed-taper fix (q≈1.1–3.1), spillover on the served antennas is
     **material: ~0.8 dB** — the original 0.4–1 dB premise was right. A fractional-q
     truncation in `estimate_spillover` (`powi` → `powf`) was fixed in the same session
     (regression: `edge_cases.rs::test_spillover_honors_fractional_q`). See the register
     row and `docs/domain-contract.md` "Magnitude reality".
  2. Outputs for antennas WITH a correction surface are **unchanged** — all existing tests
     pass untouched, plus an explicit test asserting identical gain before/after for a
     calibrated fixture.
  3. Response warnings/metadata indicate when physical spillover was applied, so consumers
     can tell which model variant produced the number (schema addition → mirror in
     openapi.yaml, standing rule 4).
  4. `docs/domain-contract.md` gains a "Modeled vs unmodeled efficiency terms" section:
     spillover modeled on the uncalibrated path (this unit); blockage/cross-pol unmodeled
     (blockage = F3); `docs/api-documentation.md` accuracy caveats updated.
- **Gotchas:** verify whether `estimate_spillover` returns the *captured*-power fraction or
  the *lost* fraction before converting to dB — get the sign right (an efficiency η ≤ 1
  multiplies gain). Do not alter the aperture integral or any phase math. Honest caveat for
  docs: parameter uncertainty (guessed q-factor, assumed surface RMS) still limits
  uncalibrated accuracy; this removes a known systematic bias, it does not make
  uncalibrated predictions calibrated-grade.
- **Depends on:** G1, G2. Do before P3/P6 (shares domain-contract edits). Companion: P1b.

### P1b — Physics-model version stamp in calibration artifacts — Effort: S
**✅ DONE 2026-07-10** — `1746bc0`. `PHYSICS_MODEL_VERSION` constant added
(`antenna-model/src/model/mod.rs`), stamped into calibration artifacts as
`CalibrationMetadata.physics_model_version` by the calibrate writers; the loader compares
against the service's constant and **warns** (never errors) on mismatch, naming both
values. Bumped to `2` when P7 landed (auto-refocus changes `gain_physics` output for
identical inputs, per this unit's own bump policy).

- **Rationale:** correction surfaces are fitted to `measured − physics` residuals; any
  change to the physics model (P1 here, F2/F3 later) invalidates surfaces fitted against
  the older model. Artifacts must record which physics-model version they were fitted
  against, or future recalibrations will silently mix eras.
- **Entrance / read first:** the metadata struct in `antenna-model/src/data/types.rs`; the
  version checks in `data/loader.rs` (around `:165`); the writer in
  `calibrate/src/artifact_export.rs`; unit D2 (the two existing version axes) — coordinate
  so this doesn't become a third, uncoordinated version mechanism.
- **Exit criteria:** an integer `physics_model_version` field in artifact metadata; the
  calibrate writer stamps the current constant; the loader compares against the service's
  constant and **warns** (not errors) on mismatch, naming both values; the bump policy
  documented (bump whenever a change alters `gain_physics` output for identical inputs);
  a test with a mismatched fixture.
- **Gotchas:** adding a field to the bincode-encoded struct is a schema change — confirm
  how decode handles missing fields for older artifacts. Mitigating fact: **no `.bin`
  artifacts exist in the wild** (none checked in; the four entries that reference a `.bin`
  are `enabled: false`, the four uncalibrated design-spec antennas are `enabled: true`), so
  breaking old-artifact decode is currently cheap — but say so explicitly in the PR and
  handle it via the ANTC header version path documented in D2 if needed.
- **Depends on:** P1 (motivates it); coordinate with D2.

### P10 — Off-axis aperture-integral aliasing (P0 CORRECTNESS) — Effort: L
**FILED 2026-07-13. ✅ DONE / LANDED 2026-07-15 — F7 now UNBLOCKED.**

> **✅ P10 COMPLETE (2026-07-15) — Tasks 0-6 all shipped:**
> - **Task 0-1** — the Hankel / azimuthal-mode integrator (Jₘ coma expansion for the
>   laterally-offset served feeds, D-1) replaced the aliasing 2D quadrature; off-axis gain is
>   numerically converged at all angles (no more 20–35 dB-too-high aliasing).
> - **Task 2-3** — adaptive radial density (`N_rho` from D/λ, θ at ~2× Nyquist, D-4/D-6) and
>   adaptive mode count, each with a runtime convergence self-check (N-vs-2N / M-vs-(M+1)) that
>   warns/refuses rather than silently returning an unconverged value; higher-order path fixed too
>   (D-5).
> - **Task 4** — the P10 validation protocol (reference_validation suite: anchors + plausibility
>   over every enabled antenna × band).
> - **Task 5** — a **single service path** serving **raw physical optics with the F7 sidelobe
>   floor OFF** (D-2 realized: serve raw PO, floor off; the floor is a service-layer param, not
>   part of the fitting physics).
> - **Task 6** — the **honest post-P10 warning**: numerically-correct-but-idealised-levels (not
>   calibrated-grade), keeping "beyond the validated main-beam region" + "ITU-R S.580"; plus this
>   docs-truth pass. `PHYSICS_MODEL_VERSION` = 3 covers the integrator change.
>
> **F7 is UNBLOCKED (redesign pending, D-2):** its floor/substitution beyond θ_valid is the
> remaining redesign, now properly informed by the correct integrator. The filed-status detail
> below is preserved as history.

- **The bug:** the service computes every gain with `IntegrationParams::fast()`
  (`service/evaluator.rs`, `service/h3_link_budget.rs`). Beyond a few degrees off-boresight the
  far-field aperture integral is under-sampled (its phase term varies as `2π·(D/λ)·sinθ` across
  the aperture) and **aliases**, returning gain **20–35 dB too HIGH**. Measured on real served
  antennas: `dsn_34m_uncalibrated` reports **+34 dBi at 90° off-boresight**, and *more* gain at
  5° than at 1°. Affects `/gain`, `/gain/batch`, `/heatmap`, `/h3-heatmap` alike. **Pre-existing.**
- **Why it hid:** a test/production integrator gap. The reference harness validates off-axis
  shape with `high_accuracy()` on the **small** 3.7 m dish (the one config where the integral
  still holds), while production serves `fast()` on dishes up to 100 m.
- **`high_accuracy()` is not the fix:** for D/λ ≈ 953 it still yields +12.8 dBi at θ = 90°.
  Physical-optics far-sidelobe evaluation is infeasible for electrically huge reflectors at any
  grid density affordable inside the <100 ms budget. (The domain contract already says this under
  "Numerical caveat" — nobody had connected it to the served path.)
- **Evidence + reproduction:** `docs/findings-2026-07-13-off-axis-integration-aliasing.md`.
- **✅ SPIKE DONE 2026-07-13 — the fix is a CONTAINED REFACTOR, not a rewrite.** The azimuthal
  integral has a closed form (Jacobi–Anger): `term2` of `phase_path` is exactly the Fourier kernel,
  and every other phase term is a pure aperture-plane function (`phase_feed_displacement` takes no
  θ/φ). So for a symmetric aperture the 2D integral collapses to a **1D Hankel transform**
  `I(θ) = 2π ∫ A(ρ)·exp(j·k·ρ²/(4f)·(1−cosθ))·J₀(k·ρ·sinθ)·ρ dρ`. Measured (dsn_34m, X-band):
  reproduces the 2D **exactly (Δ = 0.00 dB)** at θ = 0/1/5/20°, and at θ = 90° — where the 2D is
  aliased even at 8.4 M points — converges to **−33.30 dBi in ~1 ms**, independently reproducing the
  −33.28 dBi brute-force ground truth that costs **3184 ms**. That is **~3200× faster than the
  correct answer and ~5× faster than the *wrong* answer we ship today**, and it changes the
  complexity class from **O((D/λ)²) → O(D/λ)** (GBT Q-band worst case: ~13 min/point → ~2 ms).
  **The <100 ms budget stops being a constraint.** Evidence + reproduction: findings doc §4a;
  `reference_validation::p10_spike_hankel_vs_2d` (`--ignored`).
- **First thing to settle in P10:** the spike covers the **azimuthally symmetric** case only (feed
  at focus, no coma, no mesh). A laterally displaced feed breaks the symmetry; the generalisation is
  the standard azimuthal-mode expansion (`e^{jmφ′}` ⇒ `2π(−j)^m J_m(a) e^{jmφ}`) — textbook, but not
  yet demonstrated. Establish how many modes realistic coma needs.
- **Method warning:** the spike's first cut used wrong Bessel `J₀` small-argument coefficients and
  produced a *confidently wrong* 22 dB error at θ = 0 while looking perfect at θ = 90° (asymptotic
  branch). Cross-check any implementation at angles with independently known answers — a wrong
  oscillatory integrator is not obviously wrong.
- **⚠️ RESHAPED BY THE SPIKE — the old plan is stale.** The pre-spike plan was "derive the
  *numerical* validity limit `θ_valid(D/λ, grid)` and substitute a model beyond it, because the
  integral cannot be evaluated out there." **That premise is now false.** The Hankel form converges
  at *every* angle at **O(D/λ)** cost (~1 ms at θ = 90°, ~2 ms for the GBT worst case). There is no
  longer a numerical wall forcing substitution. Consequently **`θ_valid` becomes a PHYSICAL
  boundary, not a numerical one** — the angle beyond which the *idealised* PO model (unblocked,
  strut-free, perfect-surface) stops matching reality, which is a completely different question
  from where the quadrature breaks. Do not conflate them.

- **Two independent defects — keep them separate.** They were conflated before, which is how F7
  went wrong:
  1. **Numerical** (this unit): the integral is aliased ⇒ served numbers are garbage.
     *Engineerable, contained, measured.*
  2. **Physical** (F7's redesign): even perfectly converged, idealised PO ≠ reality far off-axis
     (no blockage / strut scatter / edge diffraction — the original ~8–13 dB-below-ITU finding).
     Fixing (1) does **not** fix (2); it is what finally lets you *locate* (2) honestly.

## P10 — outstanding decisions

These are genuine calls, not implementation detail. Per roadmap principle 3 ("no silent physics
changes"), get them decided before/while executing; recommended defaults given.

> **✅ ALL SIX DECIDED 2026-07-14 (maintainer).** D-1..D-4 confirmed at their recommended
> defaults via decision review; D-5/D-6 adopted as engineering defaults. Summary:
> - **D-1 → (a) azimuthal-mode expansion.** Confirmed *required*, not optional: the enabled
>   `gs_3.7m` / `dsn_13m` / `dsn_34m` antennas run **laterally-offset feeds** in
>   `antennas.yaml` (`[0.05,0,0]`, `[0.08,0,0]`/`[0,0.08,0]`, `[0.15,0,0]`/`[0,0.15,0]`), so
>   the symmetric J₀ form does not cover the served configs — a 2D fallback would leave those
>   exact feeds aliased. Offsets are small (offset/f ≈ 0.004–0.011 ⇒ coma is m≈1-dominated),
>   so few modes suffice; establish an explicit mode-count error budget (target <0.1 dB).
> - **D-2 → P10 = correct integrator + honesty warning; the statistical substitution/blend is
>   a SEPARATE F7-redesign unit** the maintainer decides later. Keeps P10 contained to the
>   numerical-correctness defect; does not re-couple the two defects that sank F7. **✅ REALIZED
>   2026-07-15: the served path serves RAW converged PO with the F7 sidelobe floor OFF**
>   (`apply_sidelobe_floor = false` on the single service path); F7's statistical model is the
>   separate redesign, now unblocked.
> - **D-3 → (a) ship the interim honesty fix on `main` now** (strengthen P8 wording to
>   "numerically invalid" and/or clamp reported off-axis gain), ahead of the multi-session P10.
>   **SUPERSEDED 2026-07-15:** P10 landed, so the "numerically invalid" wording is no longer true;
>   the warning now states the post-P10 physical caveat (idealised-PO levels, not calibrated-grade).
> - **D-4 → (a) single adaptive correct path**, `N_rho` from `(D/λ, θ)` at ~2× Nyquist;
>   presets demoted to a safety-factor knob. Closes the test/production integrator gap at the
>   root (P10 exit criterion 4).
> - **D-5 → (a) fix `compute_gain_higher_order` too** (shares the integrand); flag—don't
>   fix—`compute_gain_ray_tracing` (already a P3 stub).
> - **D-6 → ~2× Nyquist** (`N_rho ≈ 4·(D/λ)·sinθ`) + a runtime N-vs-2N convergence self-check
>   that warns/refuses, never silently returns an unconverged value.

| # | Decision | Options | Recommended default |
|---|---|---|---|
| **D-1** | **Coma / asymmetric apertures.** The Hankel collapse assumes azimuthal symmetry. A laterally displaced feed breaks it. **Settle this FIRST — it decides whether P10 is a day or a week.** | (a) azimuthal-mode expansion (`e^{jmφ′}` ⇒ `2π(−j)^m J_m(a) e^{jmφ}`); (b) keep 2D quadrature for asymmetric cases; (c) restrict/refuse | **(a)** — textbook and general. (b) is a trap: those cases would stay *aliased*, i.e. silently broken, which is the bug we are fixing. Establish an explicit mode-count error budget. |
| **D-2** | **What to serve far off-axis once the maths is right?** Converged PO is mathematically correct but physically incomplete out there. | (a) serve converged PO (right maths, optimistic physics); (b) substitute the NTIA-calibrated statistical model (salvaged F7); (c) blend PO → statistical across a transition | **(c) or (b)** — but this is the **F7 redesign decision** and is the maintainer's. It is now *properly informed* for the first time. |
| **D-3** | **Interim honesty on `main` while P10 is built.** `main` serves aliased off-axis gain today behind a soft "not validated" warning. | (a) ship a small immediate fix now (strengthen P8 to *numerically invalid*, and/or refuse off-axis beyond a threshold); (b) wait for P10 | **(a)** — cheap, and the current state actively misleads. User-visible behaviour change ⇒ needs an explicit call. |
| **D-4** | **Fate of the `fast()` / `high_accuracy()` presets.** If Hankel is *correct* AND ~5× faster than `fast()`, the speed/accuracy trade-off the presets encode largely dissolves. | (a) single correct path, `N_rho` derived adaptively from (D/λ, θ); (b) keep presets | **(a)** — retire the presets, or demote them to a radial safety-factor knob. Removes the test/production integrator gap **at the root** (that gap is *why* this hid for so long). |
| **D-5** | **Scope: the non-standard computation modes.** `compute_gain_higher_order` and `compute_gain_ray_tracing` use the same 2D quadrature and are therefore **also aliased**. | (a) fix higher-order too; (b) defer both | **(a) for higher-order** (same integrand + Seidel terms, all aperture-plane ⇒ the mode expansion applies). Ray-tracing is already an acknowledged stub (P3) — flag it, don't fix it here. |
| **D-6** | **Radial sampling policy / safety factor.** Nyquist is `N_rho ≈ 2·(D/λ)·sinθ`. Spike: 2049 pts (≈1.07× Nyquist) → −32.61 dBi (0.7 dB off); 4097 (≈2.15×) → −33.28 (0.02 dB). | pick factor + accuracy target | **≈2× Nyquist** for ~0.02 dB, with a runtime convergence self-check (compare N and 2N; disagreement ⇒ warn or refuse, never silently return). |

## P10 — validation protocol (REQUIRED; do not shortcut)

**A wrong oscillatory integrator is not obviously wrong — it returns a plausible number.**
Learned the hard way: the spike's first cut used incorrect Bessel `J₀` **small-argument**
coefficients and was **confidently wrong by 22 dB at θ = 0** while looking *flawless* at θ = 90°.
It looked fine at 90° precisely because that argument takes the **asymptotic** branch, which was
correct. Special-function implementations fail **branch-locally**: validating at one angle proves
nothing about any other.

Therefore every implementation step must be cross-checked at angles whose answers are already
known independently, spanning the whole range **and both branches**:

| θ | Independent reference |
|---|---|
| **0°** | peak gain — pinned by the existing `reference_residuals_within_tolerance` rows (dsn_34m X-band = 68.96 dBi) |
| **1–5°** | near-in; 2D quadrature is still trustworthy here, and the S.580 shape test validates the envelope |
| **20°** | mid-range; 2D at high accuracy still usable |
| **90°** | far; **ground truth −33.28/−33.30 dBi** (dsn_34m X-band), from two independent methods |

...and repeated **across D/λ** (3.7 m → 100 m) and **across bands** (S → Ka/Q), because the
aliasing onset scales with `(D/λ)·sinθ`. A single-antenna, single-angle green test is exactly the
gap that let this ship.

- **Exit criteria (revised post-spike):**
  1. The served path returns **physically plausible** off-axis gain for every enabled antenna — no
     backlobe above (main-beam − 30 dB), no gain that *rises* with θ.
  2. Hankel agrees with the converged 2D reference at the **full angle grid above**, for at least
     the smallest (3.7 m) and largest (100 m) enabled antennas, in **both** Bessel branches.
  3. A runtime convergence self-check (D-6) — the model never silently returns an unconverged value.
  4. The test/production integrator gap is **closed at the root** (D-4): the harness and the service
     evaluate gain through the *same* code path.
  5. Latency: off-axis gain within the <100 ms p95 budget (the spike says this is now easy).
- **Blocks:** F7. **Depends on:** nothing.

> **✅ P10 DONE 2026-07-15 (branch `feat/p10-off-axis-integrator`, commits `3c2a794`…`e2f401b`).**
> Exit criteria 1-4 fully met and validated (Task 4 protocol: both Bessel branches, all enabled
> antennas × bands, dsn_34m X-band 68.96/14.53/−33.29 vs brute-force ground truth). Served path
> uses the Hankel/Jₘ integrator with the F7 floor OFF (D-2 realized — serve raw PO + honest
> "idealised levels, not calibrated-grade" warning). Exit criterion 5 (latency) is met near-boresight
> and for symmetric large dishes, but see **P10-perf** below.

### P10-perf — Azimuthal-mode integrator wide-angle latency (fast-follow) — Effort: M

- **Filed 2026-07-15 by the P10 final review; maintainer chose "ship correctness now, track latency."**
  The P0 correctness fix (P10) is complete and validated. The **asymmetric** (coma) served path
  breaches the `<100 ms` p95 target for wide-angle **Ka** on an enabled antenna: `dsn_34m` Ka-band
  (32 GHz, feed offset 0.15 m) measures 136 ms @2°, 311 ms @5°, ~3.3 s @90° — results are **correct**
  (`converged=true`), just slow, and wide-angle Ka `/heatmap` is impractical. Root cause: mode count
  scales with `k·δ = 2π·δ/λ` (~100 rad ⇒ ~194 modes at Ka, not the `δ/f`-based "M≈3–5" estimated at
  decision time), and `g_m` is built with an O(n_ρ·n_φ·M) direct DFT + O(n_ρ·M²) Bessel loop
  (`model/integration.rs` ~1114-1138).
- **Fix (well-understood):** FFT the `g_m` φ'-DFT (O(n_φ·log n_φ)) and compute all `J_m(a)` orders in a
  single upward/downward recurrence sweep (O(M) not O(M²)). Expected ~1-2 orders of magnitude on the
  Ka wide-angle case. Guard with the **existing Task 4 validation protocol** (`reference_validation.rs`)
  so the optimization cannot regress the validated numbers — same result to <0.1 dB.
- **Also fold in the P10 review minors:** (a) relax the near-null spurious non-convergence warning
  (absolute-floor on the N-vs-2N check); (b) add a high-order Bessel test near the turning point
  (`m≈a`, m up to ~200); (c) fix `num_evaluations` to count the ×M mode work.
- **SEQUENCING (2026-07-15 post-P10 assessment): plan this unit TOGETHER with the F7 redesign,
  not independently.** The F7 redesign will substitute (or blend in) a statistical model beyond a
  physical `θ_valid` — and the expensive integrations are precisely the ones beyond `θ_valid`
  (wide-angle Ka on offset-feed dishes). Deciding F7's `θ_valid` and combination rule FIRST may
  shrink P10-perf substantially or eliminate its worst cases entirely; conversely, optimizing the
  wide-angle mode path first risks building speed for angles F7 then stops serving from PO at
  all. Concretely: settle the F7-redesign decision (register row F7), then re-scope this unit
  against whatever PO angular range actually remains served.
- **Depends on:** P10 (done). **Blocks:** nothing hard (correctness already shipped); soft-coupled
  to the F7 redesign per the sequencing note. Pre-production, so no live SLA is breached today.

### P10-tail — Rear-hemisphere radial budget + physicality coverage beyond 90° — Effort: S

**Filed 2026-07-15 (post-P10 assessment).**

- **Finding:** `radial_points_for` (`antenna-model/src/model/integration.rs:776`) sizes the
  radial density from `kernel + coma + defocus` cycles but **omits the dish-depth chirp**
  `k·ρ²/(4f)·(1−cosθ)`. In the forward hemisphere the chirp is subdominant (which is why every
  P10 test passes), but behind the dish it inverts: as θ→180°, `sinθ→0` collapses the kernel
  budget toward `min_rho_points` while the chirp peaks at ~`R²/(2fλ)` cycles (dsn_34m X-band:
  ~340 cycles against a ~16-point floor). The N-vs-2N self-check should flag the resulting
  under-sampling as `converged=false` (honest, not silent), but nothing *demonstrates*
  rear-hemisphere behavior: the P10 validation protocol stops at θ=90°, even though the
  original findings tables (`docs/findings-2026-07-13-off-axis-integration-aliasing.md` §2.1)
  included θ=163°.
- **Work:**
  1. Add `chirp_cycles = (R²/(4fλ))·(1−cosθ)` to the cycle sum in `radial_points_for` (one
     line; the safety cap, odd-forcing, and self-check machinery need no change).
  2. Extend `p10_served_offaxis_is_physical_all_enabled_antennas` (or add a sibling test)
     past 90° — at least θ ∈ {120°, 163°, 180°} — asserting no high backlobe and
     converged-or-warned for every enabled antenna × band.
  3. Decide and document the **rear-hemisphere policy**: PO from an unshadowed aperture is
     physically meaningless behind a reflector regardless of numerical convergence (no rim
     diffraction, no dish shadowing of the aperture field). Either fold θ>90° into the F7
     redesign's `θ_valid` or emit a dedicated warning; record the choice in
     `docs/domain-contract.md` ("Off-axis pattern / sidelobe fidelity").
- **Exit criteria:** chirp counted in the radial budget; rear-hemisphere physicality tests
  green; policy documented; every existing forward-hemisphere anchor in
  `reference_validation.rs` still green **unchanged** (standing rule 2 — the chirp addition
  only *raises* sample counts, it must not move any converged value).
- **Depends on:** P10 (done). **Blocks:** nothing; feeds the θ_valid discussion in the F7
  redesign.

### P11 — One predicate for "physics is uncorrected" gates and warnings — Effort: S

**Filed 2026-07-15 (post-P10 assessment) — promoted from
`docs/findings-2026-07-13-off-axis-integration-aliasing.md` §7, where it was recorded on
2026-07-12 but never tracked as a unit.**

- **Finding:** the spillover gate keys on `calibration.correction_surface.is_none()`
  (`service/evaluator.rs:222`) while the P8 off-axis honesty warning keys on
  `CalibrationStatus::Uncalibrated` (`service/evaluator.rs:536-541`). These are **different
  sets**: `calibrate/src/boresight_calibration.rs` (~:637,642,687) produces
  `PartiallyCalibrated` with **no** correction surface whenever there is no frequency
  correction. Such an antenna has its physics modified (spillover applied — and any future F7
  floor would follow the same gate) while serving only a "±1–1.5 dB" partial-calibration
  accuracy claim and **no** off-axis honesty warning.
- **Work:** introduce one named predicate on the calibration (e.g.
  `AntennaCalibration::physics_is_uncorrected()`, true iff there is no correction surface) and
  use it for BOTH the spillover/floor gate and the off-axis warning. Revisit the P8 "don't
  stack warnings on partially-calibrated antennas" design constraint explicitly while doing so
  — that constraint predates the discovery of the no-surface partial-cal case and should be
  re-decided, not silently inherited. Pin with a test: `PartiallyCalibrated` + no surface ⇒
  spillover applied AND the off-axis warning fires.
- **Exit criteria:** a single predicate used by every uncalibrated-physics gate; the mismatch
  case pinned by test; behavior recorded in `docs/domain-contract.md`; any warning-text change
  mirrored in `openapi.yaml`/`docs/api-documentation.md` (standing rule 4).
- **Depends on:** nothing. **Blocks:** the F7 redesign *should* build on the unified predicate
  (its gate reuses this seam) — land P11 first.

### P2 `[DECISION]` — Seidel higher-order aberration coefficients: verify or fence — Effort: M

- **Question:** `higher_order_aberrations` (`antenna-model/src/model/edge_cases.rs:250`)
  adds astigmatism/field-curvature/distortion terms with heuristic coefficient 1 (and one
  bare `/2`), consumed on the live path via `integration.rs:559-570`. No citation.
- **Recommended default:** **Fence, don't fix:** (a) annotate the function
  "HEURISTIC — unverified, see roadmap P2"; (b) add a response-level warning when the
  higher-order term contributes more than 0.1 dB to the result, reusing the existing
  warnings plumbing (trace how `edge_cases.rs` warnings reach `GainResponse` via
  `service/evaluator.rs`); (c) file a follow-up for a domain-expert citation (Seidel
  aberration theory for offset-fed reflectors). **Do not change any coefficient without a
  citation.**
- **Exit criteria:** register row Decided; annotation + threshold warning implemented with
  a test that triggers it; **all existing gain tests pass with unchanged values**.
- **Gotchas for the executing agent:** You are adding a *warning*, not altering math. If
  any existing gain test changes value, you broke something — revert and retry.
- **Depends on:** G1.

### P3 `[DECISION]` — Ray-trace stub (feed offsets > 0.5·f) disposition — Effort: S

- **Question:** Offsets > 0.5·f route to an acknowledged stub (`pattern.rs:260-270` pushes
  a degraded-accuracy warning; `ray_trace.rs:336` TODO: all aperture points "hit" by
  definition). Options: implement real ray tracing (L — feature F2), reject such requests
  (breaking), or document + strengthen flagging.
- **Recommended default:** **Document + flag.** Verify the unreliable warning reaches all
  four compute endpoints (gain, batch, heatmap, h3-heatmap), not just single-gain; add the
  limitation to `docs/domain-contract.md` and the relevant `openapi.yaml` descriptions.
- **Exit criteria:** register row Decided; one test per endpoint proving the warning
  appears for a > 0.5·f request (`examples/requests/geo_large_feed_offset.json` is a ready
  fixture); docs updated. **Do not modify `ray_trace.rs` math.**
- **Depends on:** P1 (both edit domain-contract.md — sequence to avoid conflicts).

### P4 — f_over_d out-of-range: fail loudly — Effort: S

- **Entrance / read first:** `antenna-model/src/model/geometry.rs:100-105` — the
  `if !(0.2..=1.0).contains(&f_over_d)` block has an **empty body** (silent no-op). Trace
  where f/D originates: `data/loader.rs`, `calibrate/src/antenna_config.rs`,
  `calibration_data/design_specs/*.yaml` — it comes from artifacts/config, not requests, so
  the primary fix is load-time validation.
- **Exit criteria:** out-of-range f/D produces a typed error at artifact/config load (and
  the geometry.rs silent branch becomes a real error path — no panics, per repo rule);
  unit test for the out-of-range case; in-range behavior unchanged (existing tests pass).
- **Assumptions:** the encoded range [0.2, 1.0] is correct; don't widen or narrow it.
- **Depends on:** G1.

### P5 — Unify G/T computation; fix stale G/T docs — Effort: S

- **Entrance / read first:** `antenna-model/src/model/pattern.rs:512`
  (`compute_g_over_t` — zero non-test callers) vs the inline duplicate at
  `service/h3_link_budget.rs:585` (`gain_db - 10.0 * t.log10()`); `service/evaluator.rs:61`
  — the module doc diagram advertises a `g_over_t_db` output that `GainResponse`
  (`api/schemas.rs`) does not have.
- **Exit criteria:** h3_link_budget calls `pattern::compute_g_over_t` (one implementation);
  the evaluator doc header corrected; a test pinning h3 G/T output unchanged for a known
  input; `docs/domain-contract.md` notes T is a user-supplied passthrough (noise-temperature
  modeling = F4).
- **Gotchas:** **Verify the two formulas are numerically identical before consolidating.**
  If they differ, STOP and escalate as a new decision item — do not pick one.
- **Depends on:** G1. Feeds S6 (temperature *validation* happens there, not here).

### P7 — Auto-refocus `phase_center_offset`; tighten Ka reference tolerance — Effort: M
**[DECIDED 2026-07-10 — model auto-refocus]**
**✅ DONE 2026-07-10** — branch `feat/p7-phase-center-auto-refocus`, commits `ba87160`
(model: `phase_center_offset` compensated, new explicit `axial_defocus` field carries the
defocus math), `a31c512` + `6c2e1a8` (plumbing: `axial_defocus_m` threaded YAML →
data-layer `FeedParameters` → evaluator/h3 model-feed builders, service-level tests
`test_phase_center_offset_m_is_inert_at_service_level` /
`test_axial_defocus_m_reduces_gain_at_service_level`), `10c8204` (harness: Ka tolerance
5.0 → 1.5 dB, X 1.5 → 1.0 dB in `dsn_34m_bwg.psv`; measured post-fix residuals X +0.17 dB,
Ka +0.01 dB). Exit criteria 1–3 met, including the domain-contract update (done in
this same docs pass). This unit's P1b dependency (`1746bc0`) was implemented earlier in
the same branch, not on a separate one — see P1b above. **Stretch criterion (exit
criterion 4, second multi-band reference antenna) intentionally NOT implemented**: judged
unnecessary because cross-D/λ generalization is already evidenced by `dsn_34m_uncalibrated`
carrying nonzero datasheet phase-center offsets at both X-band (0.015 m) and Ka-band
(0.008 m) under the now-tightened tolerances, plus the pre-existing GBT 100-m L/Q-band rows
(1.4–43 GHz) — see `docs/findings-2026-07-10-ka-phase-center-defocus.md` follow-up step 5.

- **Decision (recorded):** `phase_center_offset_m` is a **raw feed property** that the model
  compensates: the evaluator positions the feed axially so the phase center lands at the
  focal point (matching how real antennas are operated — large dishes refocus per band), so
  the field no longer produces an uncompensated defocus. Deliberate defocus becomes a new
  explicit field. Chosen over "config realism" (redefine the field as residual-after-focus
  and set ≈0 by convention) on correctness/long-term grounds: the convention leaves a
  standing trap where entering a datasheet phase-center value (0.005–0.02 m — exactly what
  the old design specs had) silently costs multi-dB at Ka. Full diagnosis:
  `docs/findings-2026-07-10-ka-phase-center-defocus.md`.
- **Entrance / read first:** the findings doc above (decomposition table + root cause);
  `antenna-model/src/model/integration.rs:526` (`feed_axial_offset =
  position.z − focal_length + phase_center_offset` — the term to change);
  `test_phase_center_offset_produces_defocus_loss` (`integration.rs:994`);
  the `phase_center_offset` glossary entry in `docs/domain-contract.md`; the harness fixture
  `antenna-model/tests/fixtures/reference_datasets/dsn_34m_bwg.psv` (Ka tolerance 5.0 dB,
  deliberately loose pending this unit).
- **Design constraints (must-follow):**
  1. `phase_center_offset` stops contributing to the defocus term — the model assumes the
     feed is positioned so its phase center sits at the focus. A **new explicit config
     field** (e.g. `axial_defocus_m`, default 0) expresses deliberate defocus; the defocus
     *math* stays intact and reachable through it.
  2. `position.z − focal_length` remains a live defocus contribution — it represents actual
     feed placement, not a feed property.
  3. **Scope: the axial term only.** Do not touch lateral/steering math or sign conventions
     (standing rule 2 — this unit is a sanctioned physics change, but only to the axial
     defocus expression).
  4. This changes `gain_physics` output for identical inputs → **bump
     `physics_model_version`** per P1b's policy.
- **Exit criteria:**
  1. Harness DSN 34-m residuals ≈ 0.1 dB at **both** X and Ka (per the findings-doc
     decomposition); **Ka tolerance in `dsn_34m_bwg.psv` tightened 5.0 → 1.5 dB** (and X to
     ~1.0 dB if the residual supports it).
  2. `test_phase_center_offset_produces_defocus_loss` reworked to pin the new explicit
     field; a companion test asserts a nonzero `phase_center_offset` alone produces **no**
     defocus loss.
  3. All workspace tests green; `docs/domain-contract.md` glossary entry + open-items bullet
     updated **in the same change** (contract rule).
  4. Stretch: add a second multi-band reference antenna (e.g. DSN 34-m HEF) to confirm the
     fix generalizes across D/λ.
- **Gotchas:** the dead `illumination::phase_center_offset_phase` (`illumination.rs:357`) is
  a *different*, unused implementation — do not wire it in; remove it or leave it for the
  dead-code sweep, but don't confuse it with the live path.
- **Depends on:** P1b (the version-stamp mechanism this unit bumps).

### P8 — Off-axis honesty warning — Effort: S
**✅ DONE 2026-07-12** — branch `feat/p8-off-axis-honesty-warning`, commit `8d0c4f8`.
`service/evaluator.rs::off_axis_unvalidated_warning` (constants
`FIRST_NULL_COEFFICIENT = 1.6`, `OFF_AXIS_FIRST_NULL_MULTIPLE = 3.0` → threshold =
3× first-null angle ≈ 4.8·λ/D rad), called from the gain pipeline (batch/heatmap
inherit per-item/per-point) and from the H3 per-cell path (`compute_cell_gain`,
outside the gain cache so it surfaces on cache hits). All four exit criteria met:
warning tested per endpoint (`tests/integration/off_axis_warning_tests.rs`,
incl. boresight negative case + heatmap dedup assertion), no existing test
modified, contract/api-documentation/openapi updated. Message deliberately
constant per (antenna, frequency) — no per-query angle — so heatmap/H3 warning
aggregation dedups it; C8 stage 3 owns the typed-code conversion.

- **Rationale (as filed):** the model's off-axis (sidelobe) gain was systematically optimistic
  (~8–13 dB below the ITU-R S.580 mask; see the contract's "Off-axis pattern / sidelobe
  fidelity" section) and must not be silently served for interference / off-axis-EIRP use.
  Until/unless F7 lands, the honest answer is a warning. **F7 has since landed
  (2026-07-12, branch `feat/f7-sidelobe-floor`)**: the served uncalibrated off-axis value is
  now envelope-conservative rather than optimistic, and the warning message was revised
  alongside F7 to say so — see the F7 unit below.
- **Entrance / read first:** contract section above;
  `service/evaluator.rs:411` (`generate_calibration_warnings` — the implementation site;
  `corrected_el` is already at the call site, `:339`); the existing warning kinds it emits
  (uncalibrated / partially-calibrated / outside-calibrated-region), to avoid double-warning.
- **Design constraints:**
  1. Warn when a query on an **uncalibrated** antenna is beyond the validated
     main-beam/near-in region. Calibrated-but-out-of-coverage already gets the
     extrapolation warning — do not stack a second warning there.
  2. Threshold expressed in units of λ/D (beamwidth-relative, not a fixed angle — a 34-m Ka
     beam is ~0.017° wide): e.g. θ beyond ~3× the first-null angle (≈1.6·λ/D rad for tapered
     illumination). Executor picks the exact constant and documents it in the contract.
  3. Message points consumers at the ITU mask / calibration data for off-axis use
     (mirror the contract's language: sidelobe levels are optimistic; shape is validated,
     levels are not).
  4. String warning now; C8 stage 3 converts it to typed code `off_axis_unvalidated`
     (already added to C8's enumerated list).
- **Exit criteria:** warning appears on all four compute endpoints for a large-θ
  uncalibrated query (test per endpoint); no warning inside the main beam; existing tests
  untouched; `docs/api-documentation.md` accuracy-caveat section updated; openapi.yaml
  mirrored (standing rule 4).
- **Depends on:** G1. Independent of P7. Sequence before or with C8 stage 3.

### P6 — Refresh `docs/domain-contract.md` "Open items" — Effort: S (phase closer)

- **Exit criteria:**
  1. Resolved items marked resolved with pointers: `phase_center_offset_phase` → now axial
     defocus at `integration.rs:516-517` (glossary entry at contract :76 also updated);
     duplicate Ruze in `surface.rs` → file deleted (glossary :77 updated).
  2. `transparency_at_wavelength` open item cross-references unit D8; f_over_d item
     cross-references P4.
  3. P1/P2/P3/P5/P7/P8 outcomes recorded in the contract where relevant.
  4. The design-doc-drift process item (contract :110-114) **re-verified** against the
     post-`aee11f9` design doc — it may already be resolved; check, don't assume.
- **Depends on:** P1–P5, P7, P8.

---

## Phase 2 — Safety & operational correctness

### S1 — Enforce the configured body-size limit — Effort: S/M (top of phase)

- **Entrance / read first:** `config/settings.rs:46-48` (`max_body_size_bytes`),
  `api/mod.rs:193` (limit only logged), `api/middleware.rs:320-333` (`RequestSizeTracker`
  warns, never rejects). Find the existing test:
  `grep -rn test_request_body_size_limit antenna-model/` — **its current pass is for the
  wrong reason** (11 MB blob fails JSON parse → 400 after full buffering); treat it as
  untrustworthy. Check the pinned poem version's `SizeLimit` middleware availability.
- **Exit criteria:** requests exceeding the configured limit get **413** with the project's
  standard JSON error body; the test rewritten to send a *well-formed* oversized body and
  assert 413; limit remains configurable.
- **Gotchas:** batch and heatmap requests are legitimately large — confirm the default in
  `config/service.yaml` comfortably exceeds a maximum-size 1000-item batch before
  enforcing; if not, raise the default in the same change and say so.
- **Depends on:** G1.

### S2 — Enforce the configured request timeout — Effort: S

- **Entrance / read first:** `settings.rs:42-44`, `api/routes.rs` middleware stack (no
  timeout of any kind), `api/mod.rs:194` (log-only).
- **Exit criteria:** timeout middleware wired to `request_timeout_secs`; an integration
  test (tiny configured timeout + heavy heatmap request) asserting the timeout status
  (504 or project-standard); documented in api-documentation.md.
- **Gotchas:** a poem-layer timeout does **not** cancel rayon work already submitted
  (dropping the future doesn't stop the pool) — state this in a code comment; compute-side
  bounding is S3's job. Don't claim more than the middleware delivers.
- **Depends on:** G1. Pairs with S3.

### S3 — Wall-clock budget inside aperture integration — Effort: M

- **Entrance / read first:** `model/integration.rs` `IntegrationParams` presets
  (max_iterations 3/5/8 — the only bound today); how many integrations a single heatmap
  (up to 100k points) or batch (up to 1000 items) fans out to (`service/heatmap.rs`,
  `service/batch.rs`).
- **Exit criteria:** integration checks elapsed time at iteration boundaries against a
  configurable budget; over-budget returns a **typed error** (never a silently degraded
  result); default budget generous enough that **all existing tests pass unchanged**;
  config knob in `settings.rs` + `config/service.yaml` with docs; a tiny-budget test
  asserting the error surfaces as a clean 4xx/5xx.
- **Assumptions:** per-integration (not per-request) granularity is acceptable for v1.
- **Gotchas:** check the clock at iteration boundaries only (cheap). **Do not change
  convergence math.** Note the existing behavior where non-convergence yields a warning —
  that stays; the budget is a separate, harder stop.
- **Depends on:** S2; after Phase 1 (touches the model layer).

### S4 — Admission control + resolve dead `worker_threads` config — Effort: M

- **Entrance / read first:** `settings.rs` performance section; `service/batch.rs`,
  `service/heatmap.rs`, `service/h3_link_budget.rs` all use rayon's **global** pool; no
  concurrency-limit middleware anywhere.
- **Exit criteria:**
  1. `performance.worker_threads` wired via `rayon::ThreadPoolBuilder::build_global` at
     startup (recommended) — or removed from config; recommended: wire it.
  2. A semaphore caps concurrent heavy requests (batch/heatmap/h3-heatmap); when saturated,
     return 429 or 503 with the standard JSON error; limit configurable.
- **Gotchas:** `build_global` can only be called once and errors if a pool already exists
  (tests may have initialized it) — handle the `Err` gracefully. Do not create per-request
  rayon pools.
- **Depends on:** S1, S2 (same middleware stack — land sequentially).

### S5 — Real graceful shutdown, readiness lifecycle, honor `fail_fast` — Effort: M

- **Entrance / read first:** `api/mod.rs:72` (readiness defaults `true` at construction),
  `:178-186` (total calibration-load failure → warn + empty repository + healthy server,
  regardless of `calibration.fail_fast`), `:301-316` (`shutdown_cleanup()` is a no-op that
  nothing invokes); health/ready handlers in `api/handlers.rs`; `data/repository.rs`.
- **Exit criteria:**
  1. Readiness starts false; flips true only after calibration load completes.
  2. All-loads-failed + `fail_fast` → process exits nonzero at startup; without
     `fail_fast`, the server starts but readiness/health reflect the degraded state
     (keep the existing response *shapes*).
  3. On shutdown signal: readiness flips false, `shutdown_cleanup()` is actually invoked,
     in-flight requests drain.
  4. Tests for the fail_fast path and the readiness flip.
- **Assumptions:** Kubernetes-style deployment (a `helm/` dir exists), so
  readiness-false-before-drain is the right pattern.
- **Gotchas:** do not change the `/health` and `/ready` response schemas (they're in
  openapi.yaml). Distinguish "zero antennas *enabled*" from "configured but failed to
  load". **Note (2026-07-09):** the current `antennas.yaml` is NOT all-disabled — it has
  four `enabled: true` uncalibrated design-spec antennas (which load without a `.bin`) and
  four `enabled: false` entries that reference absent `.bin` files. So the live default
  state is "four antennas loaded, uncalibrated," not "zero configured." Test both the
  loaded-uncalibrated path and a genuine load-failure. See D9.
- **Depends on:** S4 (same startup code region).

### S6 — Close H3 link-budget validator gaps — Effort: S

- **Entrance / read first:** `service/validator.rs:203-226`
  (`validate_h3_link_budget_request` — validates positions, `frequency_mhz`, `n_rings`,
  quaternion; **skips** the fields below). Copy the gain endpoint's validation style and
  error codes exactly.
- **Exit criteria:**
  1. `temperature_k`: must be > 0 (a non-positive value currently reaches
     `t.log10()` at `h3_link_budget.rs:585` → NaN in the response) with a sane upper bound
     (match any existing temperature bound; if none, require > 0 and ≤ 10000 K).
  2. `pointing_frequency_mhz`: validated with the same `validate_frequency` call the gain
     and heatmap validators already use (`validator.rs:96,182`).
  3. `h3_resolution`: range-checked in the validator (0–15, or narrower if
     `h3_link_budget.rs` assumes so — today invalid values are caught late by
     `h3o::Resolution::try_from` at `h3_link_budget.rs:273`; validation belongs in the
     validator layer for consistency).
  4. Tests for each rejection + one passing boundary case; openapi.yaml constraint
     descriptions mirrored (standing rule 4).
- **Gotchas:** reuse the existing snake_case error codes and message format — don't invent
  new ones (C3 owns vocabulary).
- **Depends on:** G1. Independent of Phase 3.

### S7 — GEO coordinate-ambiguity policy — **SUPERSEDED by C8 (decided 2026-07-08)**

The warn-everywhere + `strict_coordinates` design existed only because breaking the API was
assumed off-limits. With pre-production confirmed, C8 stage 2 makes `coordinate_system`
**required**, eliminating the auto-detection ambiguity instead of warning about it. Do not
implement this unit. The stale threshold comments this unit would have fixed
(`schemas.rs:9` says 1000 km, constant is 6400 km; `validator.rs:266` says 10,000 km,
constant is 400,000 km) move into C8 stage 2.

---

## Phase 3 — API contract quality

Sequencing: **C3 → C4 → C2** share the handler error paths — land in that order, then
**C8** (the consolidated breaking pass), then **C7** (drift guard) freezes the result.
C1 can run in parallel with C3–C2. C5 and C6 are superseded by C8.

### C1 — Document `/api/v1/h3-heatmap` — Effort: S/M

- **Entrance / read first:** `api/routes.rs` (the registered route + method), the h3
  handler in `api/handlers.rs`, the orphaned schemas at `openapi.yaml:750,822`
  (`H3LinkBudgetRequest`/`Response` exist but no path references them), existing openapi
  path entries as a style reference, `docs/api-documentation.md` endpoint sections.
- **Exit criteria:** an `/api/v1/h3-heatmap` path entry in openapi.yaml wired to the
  existing schemas, with error responses matching **current** behavior (including its
  current status-code quirks — C2 owns changing them, and updates the spec again);
  an api-documentation.md section with a working example (reuse a passing request body
  from `tests/integration/h3_link_budget_tests.rs`); that example added under
  `examples/requests/` (automatically covered by G3's deserialization test).
- **Note:** may be absorbed into C8 stage 4 instead of running standalone — if C8 is
  imminent, fold it in; if C8 is far off, land this first (documenting current behavior is
  cheap and gives C8 a baseline) and let C8 update it.
- **Depends on:** G3, S6 (new validation constraints must appear in the spec).

### C3 — Single error-code vocabulary; delete dead PascalCase constructors — Effort: S

- **Entrance / read first:** `api/schemas.rs:~1085-1110` — `ErrorResponse` convenience
  constructors emitting PascalCase codes (`"AntennaNotFound"`, `"FeedNotFound"`,
  `"InvalidParameter"`); handlers emit snake_case codes (`"validation_error"`,
  `"antenna_not_found"`, …).
- **Exit criteria:** PascalCase constructors deleted (**grep-confirm zero callers for each
  first**); the set of live snake_case codes enumerated in api-documentation.md and in the
  openapi.yaml error-schema description; a small unit test or const list preventing typo
  drift if cheap.
- **Gotchas:** if any constructor *does* have a caller, converting that call site changes
  wire output — flag it explicitly in the PR description.
- **Depends on:** G1.

### C4 — Error bodies as `application/json` — Effort: S

- **Entrance / read first:** the `poem::Error::from_string(serde_json::to_string(…))`
  pattern (e.g. `handlers.rs:203-206`) — poem serves these as `text/plain`. **Find all
  sites by grep**, not just the cited one.
- **Exit criteria:** one shared error-response helper replaces the ad-hoc pattern at all
  sites; all error responses carry `Content-Type: application/json`; **body bytes
  unchanged** — an integration test asserts both the header and the body on a 422 and a 400.
- **Depends on:** C3.

### C2 `[DECISION]` — Unify validation status codes — Effort: M

- **Question:** validation failures return 422 from the pre-check path
  (`handlers.rs:205,428,905`) but 400 when the same class of error surfaces from the
  service layer (`handlers.rs:341,463,976`); batch=400 vs single=422 for equivalent inputs.
  Changing codes is a behavioral API change.
- **Recommended default:** **400 = malformed/undeserializable body; 422 = well-formed but
  semantically invalid** — both layers, all endpoints. Treat as bug-fix-grade in v1 (no
  client can have relied on the inconsistency), with a changelog note.
- **Exit criteria:** register row Decided; an integration test matrix
  (endpoint × {malformed, invalid}) **written first**, then codes fixed until it passes;
  openapi.yaml responses updated for every endpoint; api-documentation.md error section
  updated.
- **Gotchas:** grep handlers.rs for every `StatusCode`/`from_status` site (~6); the matrix
  test is the net that catches a missed one. Don't touch error *bodies* here (C3/C4 own
  those).
- **Depends on:** C3, C4 (land after, to avoid triple-editing the same lines).

### C5 — `/heatmap` H3 grid-type stub — **SUPERSEDED by C8 (decided 2026-07-08)**

The variant removal happens in C8 stage 4 alongside the rest of the endpoint-coherence
work. Do not implement standalone. (Full H3-into-`/heatmap` merge remains feature F5,
still gated.)

### C6 — `feed_position` naming trap — **SUPERSEDED by C8 (decided 2026-07-08)**

The docs-only design existed only under the no-breaking-changes assumption. With
pre-production confirmed, C8 stage 1 performs the actual rename to
`feed_pointing_location`. Do not implement the docs-only variant.

### C8 — v1 contract finalization (the one sanctioned breaking pass) — Effort: L
**[DECIDED 2026-07-08 — pre-production confirmed: no consumers exist; break once now, then freeze]**

- **Rationale (recorded):** The maintainer confirmed nothing consumes this API yet
  (no remote, no shipped `.bin` artifacts, only uncalibrated design-spec antennas enabled). Breaking cost is
  ~zero today and permanent after the first integration. All desirable breaking changes
  land in this single pass; C7's drift guard freezes the contract immediately after. A
  full redesign was considered and rejected: there is no efficiency case (aperture
  integration dominates latency, not JSON shape), so only naming/consistency/safety
  changes are in scope.
- **Effort note:** L — execute as **four sequential stages, one PR each**, in this order.
  Each stage leaves the workspace green (`cargo test --workspace`) and openapi.yaml +
  `examples/requests/` + `docs/api-documentation.md` updated (G3's example test is the net
  that catches missed examples).

**Stage 1 — Rename the aim-point fields.**
- `feed_position` → `feed_pointing_location` on all three request types (fields at
  `schemas.rs:247,432,590`). Review the two *physical*-offset response fields
  (`GeometryInfo.feed_offset_meters`, `FeedInfo.position_offset`) and align them to one
  naming scheme that cannot be confused with the aim point (e.g.
  `physical_feed_offset_m`); keep units in the name or the docs, consistently.
- **No serde aliases, no deprecation shims** — clean break.
- Update `docs/domain-contract.md`'s parameter-glossary entry **in the same commit**
  (contract rule: contract and code change together).
- Exit: grep for `feed_position` finds zero hits outside historical docs
  (`review-findings-*.md`, superpowers plans) and the contract's changelog note.

**Stage 2 — Make `coordinate_system` required (remove auto-detection).**
- `Position3D.coordinate_system` becomes a required field; missing → deserialization/
  validation error naming the exact field path. Delete the magnitude-based auto-detection
  (`Position3D::coordinate_system()` heuristic, `ECEF_THRESHOLD_M` at `schemas.rs:126`) and
  the now-dead `coordinate_ambiguity_warnings` plumbing (`validator.rs:451-463`,
  `evaluator.rs:105`); **keep** per-system range validation (ECEF magnitude, geodetic
  lon/lat/alt bounds).
- Fix the stale threshold comments while in the area (`schemas.rs:9`, `validator.rs:266`) —
  or delete them with the machinery they describe.
- Update the domain contract's frame table + GEO-trap gotcha (the trap no longer exists —
  record it as resolved-by-design, don't silently delete the history).
- Exit: a geodetic GEO-altitude position without a tag is now a 4xx with a clear message
  (test); all examples carry explicit `coordinate_system`; contract updated.
- Gotcha: `test_explicit_coordinate_system_overrides_detection` (`schemas.rs:1180`) and the
  detection unit tests must be rewritten to assert the new required-field behavior, not
  deleted wholesale.

**Stage 3 — Typed warnings.**
- `warnings: Vec<String>` → `Vec<ApiWarning> { code, message }` on all response types
  (currently at `schemas.rs:307,511,691`). Enumerate the code set from existing producers
  (grep `warnings.push` / warning constructors): expect at least `extrapolated`,
  `out_of_coverage`, `ray_trace_degraded`, `non_convergence`, plus the codes added by
  roadmap units P1 (`spillover_applied`), P2 (`higher_order_heuristic`), and P8
  (`off_axis_unvalidated`) — coordinate with those units if they land first (strings then;
  codes now).
- Exit: every producer emits a code + human message; the code enum documented in
  api-documentation.md + openapi; integration tests assert codes, not string matches.

**Stage 4 — Endpoint coherence + spec completeness.**
- Remove the `/heatmap` H3 grid-type stub variant (`heatmap.rs:168-171,215-218`); unknown
  grid types become normal validation failures (absorbs old C5).
- `/h3-heatmap` fully documented (absorbs C1 if it hasn't landed; if C1 landed, update it
  for stages 1–3's changes).
- Decide-and-document endpoint naming: keep two endpoints (`/heatmap` rectangular,
  `/h3-heatmap` link budget) — a full merge remains feature F5.
- Exit: openapi.yaml describes every registered route with post-C8 schemas; ready for C7.

- **Depends on:** C3 → C4 → C2 landed first (error contract settled before the breaking
  pass); G3 (example test); S6 (validation constraints exist to document). **Blocks:** C7.
- **Out of scope (explicitly):** batch shared-context request shape (additive later via
  optional top-level defaults); poem-openapi codegen migration; any physics/semantics
  change — this pass renames and reshapes, it must not alter any computed value (existing
  numeric assertions in tests are the net: they may change *field names*, never *values*).

### C7 — OpenAPI drift guard — Effort: M

- **Entrance / read first:** `api/routes.rs` route registration; openapi.yaml paths.
- **Exit criteria:** a CI test that parses openapi.yaml (serde_yaml) and asserts the
  path+method set equals the registered route set — failing when a route exists without a
  spec entry or vice versa. Stretch (optional): validate G3's example files against the
  openapi component schemas. A note in docs about the guard.
- **Assumptions:** migrating to poem-openapi codegen is **out of scope** — register it as a
  possible future item in the roadmap doc, not part of this unit.
- **Depends on:** C8 (the contract must be finalized first — this guard is what freezes it).

---

## Phase 4 — Structure, debt, docs

### D1 — Retire the deprecated legacy serializer in calibrate — Effort: S

- **Entrance / read first:** `calibrate/src/serializer.rs` — 612 lines, header honestly
  marked DEPRECATED (`:3-7`): it serializes the legacy `CalibrationArtifact` (3D surface)
  which the service **cannot** load (wrong struct + serde-bincode mode), and says it is
  "retained only for the optional `--metadata`/`--report` JSON sidecars and existing
  tests". Its only workspace-visible consumer is the re-export at `calibrate/src/lib.rs:57`.
- **Exit criteria:** verify whether the `--metadata`/`--report` sidecar paths in
  `calibrate/src/main.rs` actually use this module. If yes: extract only the JSON-sidecar
  helpers and delete the binary-artifact (`save_artifact`/`load_artifact`) surface. If no:
  delete the module and the `lib.rs:57` re-export entirely. Workspace builds + tests green;
  `docs/calibration-workflow-guide.md` checked for references.
- **Gotchas:** never delete a live code path — migrate callers to `artifact_export.rs`
  first. The dangerous part is specifically the binary writer that produces unloadable
  artifacts; that must not survive.
- **Depends on:** G1.

### D2 — Reconcile the two artifact version axes — Effort: S

- **Entrance / read first:** `data/loader.rs` — ANTC header `u32` version (=1) vs
  `metadata.format_version` string ("2.0" expected, warned at `loader.rs:165`); the writer
  side in `calibrate/src/artifact_export.rs`.
- **Exit criteria:** the relationship defined in a doc comment + a section in
  `calibration-workflow-guide.md` (recommend: header u32 = container/binary layout version;
  `format_version` = semantic schema version); the loader validates both with clear errors
  on mismatch; one test with a wrong-version fixture. **Do not bump either version.**
- **Depends on:** D1.

### D3 — Round-trip test for the 3D→4D correction-surface bridge — Effort: M

- **Entrance / read first:** `calibrate/src/artifact_export.rs` (`to_bspline_4d` —
  dimension remap + coefficient reindex + synthetic flat temperature axis),
  `calibrate/src/correction_surface.rs` (3D), `model/correction_interpolator.rs` (4D
  consumer).
- **Exit criteria:** a test that fits a small synthetic surface in calibrate → exports →
  loads via the antenna-model loader → evaluates at sample points → asserts agreement with
  the pre-export surface within tolerance; edge tests (single-frequency, boundary knots).
- **Gotchas:** **test-only unit.** If a bug falls out, STOP and file it as a new
  correctness item — no drive-by fixes.
- **Follow-up flagged 2026-07-09 (Phase 0 / G1):** this test already exists as
  `calibrate::artifact_export::tests::test_round_trip_matches_3d_evaluation` (so D3's "add
  the test" exit criterion is partly satisfied — verify/extend its edge coverage rather
  than duplicating). More important: on the first CI run it **stack-overflowed on the Linux
  debug build** (SIGABRT) while passing on macOS — the 3D→4D round-trip B-spline evaluation
  is stack-hungry and exceeded libtest's ~2 MiB worker-thread stack. Phase 0 worked around
  it with `RUST_MIN_STACK=16 MiB` in CI + `scripts/check.sh` (commit `4b439c0`). **D3 should
  investigate the recursion depth in the evaluation path (`to_bspline_4d` / the B-spline
  evaluator) and make it iterative / bounded so the workaround can be removed.** This is a
  robustness item, not a correctness bug (the numeric result is right: max round-trip error
  ~4e-15).
- **Depends on:** D1, D2.

### D4 `[DECISION]` — Crate split: extract `antenna-core` — Effort: L

- **Question:** `calibrate` depends on the whole `antenna-model` crate, compiling
  poem/h3o/the web stack for a CLI; `ndarray` 0.15.6 and 0.16.1 are both in the tree
  (calibrate pinned via ndarray-linalg 0.16).
- **Recommended default:** **Do it.** Extract `antenna-core` (contents of
  `antenna-model/src/model/` + `data/types.rs`) as a third workspace member; service and
  calibrate both depend on it. Attempt ndarray unification during the split; if
  ndarray-linalg blocks it, document and accept dual versions.
- **Exit criteria:** three-crate workspace; `cargo tree -p calibrate` shows no
  poem/h3o/tokio-web deps; all tests pass; CI green; CLAUDE.md + architecture.md module
  maps updated.
- **Gotchas for the executing agent:** this is a mechanical **move**, not a rewrite —
  `git mv` files, fix `use` paths, change nothing else; commit in reviewable steps. **If
  any test value changes, the move went wrong.**
- **Depends on:** Phases 1–3 complete (merge-conflict avoidance).

### D5 — Design-docs truth sweep — Effort: M

- **Entrance / read first:** `docs/architecture.md:~1350-1372` (lists nonexistent
  `interpolation.rs`/`bspline.rs`/`extrapolation.rs`; calibrate `fitter.rs`);
  `docs/antenna-model-design-doc.md` — Zernike per-point sections (:269,317 —
  unimplemented; the correction surface absorbs surface error), direct-path interference
  (:170 — the mode was removed in `c850165`), feed-steering sign section (:130-132);
  `docs/review-findings-2026-06-10.md`.
- **Exit criteria:**
  1. architecture.md module lists match `ls` reality for both crates.
  2. Design-doc sections either corrected or marked "historical — not implemented".
  3. The feed-steering sign section **verified against `model/coordinates.rs` code**
     (post-`aee11f9` it may already match): add a "verified 2026-07 vs code" note if it
     agrees, or file a NEW decision item if it genuinely disagrees — do not edit that
     section's math without verification.
  4. review-findings-2026-06-10.md gets a status column mapping each finding to
     resolved-commit or roadmap unit ID.
- **Gotchas:** docs-only. Standing rule 2 applies doubly here.
- **Depends on:** P6, G2 (after physics docs settle); after D4 if D4 happens (module map).

### D6 — Repo hygiene: tarpaulin artifact, S3 dependency gating — Effort: S

- **Exit criteria:** the committed `tarpaulin-report.html` (3.1 MB, repo root) deleted and
  the pattern gitignored; `aws-sdk-s3` + `aws-config` in `calibrate` (used in exactly one
  file, `parser.rs`, for optional S3 CSV input) moved behind an off-by-default cargo
  feature (e.g. `s3-input`) with a clear CLI error when invoked without it; CI/clippy stays
  green for both feature states.
- **Follow-up flagged 2026-07-09 (Phase 0 / G1):** the first CI run's `cargo audit` job
  (non-blocking, `continue-on-error`) reported **17 vulnerabilities + 9 warnings**. The
  large majority come from the AWS SDK subtree pulled in by `aws-sdk-s3` + `aws-config`:
  `aws-lc-sys` (RUSTSEC-2026-0044/45/46/47/48 — sig-bypass/timing/name-constraint),
  `rustls-webpki`, `rustls-pemfile`, `tar`, `time`, `bytes`. **Gating that subtree behind
  the off-by-default `s3-input` feature (this unit) removes ~11 of the 17 advisories from
  the default build.** The remainder are non-AWS and stay for triage/allowlist:
  `bincode 2.0.1` (unmaintained), `anyhow` (unsound `downcast_mut`), `rand`/`lru`/
  `crossbeam-epoch` (unsound), `instant`/`paste` (unmaintained). **Elevate this unit's
  priority** — it is now the primary lever on the advisory count. After it lands, add
  explicit `cargo audit --ignore RUSTSEC-…` entries (with rationale) for any accepted
  residual advisories, turning the tracked-allowlist mechanism on.
- **Depends on:** G1.

### D7 — Property-based tests (make CLAUDE.md's claim true) — Effort: M

- **Entrance / read first:** `model/coordinates.rs`, `model/coordinates_3d.rs` (transform
  pairs), `model/pattern.rs` (bounds candidates), existing test style. Knowledge: proptest.
- **Exit criteria:** proptest as dev-dependency; properties implemented: coordinate
  round-trips within tolerance over valid domains; gain finite and ≤ the ideal-aperture
  bound for random valid inputs; Ruze efficiency ∈ (0,1] and monotone-decreasing in surface
  RMS; runs in CI within reasonable time (cap case counts); the CLAUDE.md:214 annotation
  from G2 updated to "implemented".
- **Gotchas:** constrain generators to the *validated physical domain* (positive diameters,
  frequencies within [100, 50000] MHz, etc.) or you'll "discover" inputs that validation
  already rejects upstream. **Property failures are findings to file, not things to fix
  inline.**
- **Depends on:** Phase 1 complete (physics stable); D4 optional.

### D8 — Remove dead `MeshParameters::transparency_at_wavelength` — Effort: S

- **Entrance:** `model/geometry.rs:437`; only callers are its own unit tests
  (`geometry.rs:752,756`).
- **Exit criteria:** function + its tests removed (the P1 decision — staged spillover —
  does not wire this simplified mesh-transparency path; the live path keeps
  `mesh::mesh_reflection_efficiency`). `docs/domain-contract.md` open item updated (P6
  cross-reference).
- **Depends on:** P6.

### D9 `[DECISION]` — Calibration-artifact shipping story — Effort: S

- **Question:** `calibration_data/antennas.yaml` has four `enabled: false` entries (each
  references an absent `.bin` calibration file) and four `enabled: true` uncalibrated
  design-spec entries (which load without a `.bin`); no `.bin` artifacts exist anywhere in
  the repo; CLAUDE.md claimed precomputed artifacts ship. Commit binaries, generate in CI,
  or docs-only? **(Corrected 2026-07-09: this row previously said "all entries are
  `enabled: false`" — wrong; 4 of 8 are enabled. The README quickstart and `/health`/
  `/status` copy must describe the four-uncalibrated-antennas default, not an empty repo.)**
- **Recommended default:** **Docs-only, no binaries in the repo.** Document the generation
  command (extending `calibration-workflow-guide.md`) and add a `scripts/` helper or make
  target that produces artifacts locally from `calibration_data/`; verify the path once
  locally.
- **Exit criteria:** register row Decided; a documented, once-verified generation path; a
  README quickstart section explaining the empty-by-default state and how `/health` and
  `/status` reflect it.
- **Depends on:** G2, S5 (readiness semantics for the zero-artifact state).

---

## Phase 5 — Decision-gated features

Do not start any of these until the corresponding decision-register row is Decided.

### F1 — Calibration hot-reload — Effort: M/L

Recommend an authenticated admin endpoint (`POST /admin/reload-calibrations`) over
file-watching (k8s configmap semantics make watching fragile). Uses the existing
`RwLock<HashMap>` in `data/repository.rs`. Exit: reload swaps the repository atomically;
in-flight requests unaffected; a failed reload keeps the old data and returns an error;
test with two artifact versions. **Gotcha:** never hold the write lock across artifact
parsing — parse first, swap second.

### F2 — Real ray tracing for feed offsets > 0.5·f — Effort: L (gated on P3 flipping)

Requires domain-expert input (`ray_trace.rs:336` TODO — occlusion/blockage geometry).
Explicitly requires physics review of results against published offset-fed reflector data
before the degraded-accuracy warning may be removed.

### F3 — Physical blockage efficiency term — Effort: M/L (spillover already done in P1)

Feed/strut aperture blockage (~0.1–0.5 dB typical). Data-gated: requires new antenna-config
geometry parameters (feed package diameter, strut widths) that don't exist today — the term
applies only when the parameters are present, and is skipped with a scope note when absent.
Reuses P1's double-counting gate (uncalibrated path only) and bumps P1b's
`physics_model_version`.

### F4 — Antenna noise-temperature model for G/T — Effort: L (gated on P5/F4 row)

Sky/ground pickup + spillover-noise contributions. Same double-counting caveat as F3.

### F5 — Merge H3 into `/heatmap` — Effort: M/L (gated: C8 kept two endpoints)

C8 stage 4 settled on two documented endpoints; this feature would merge them behind a
grid-type-discriminated contract. Contract design first (discriminated response union),
then implementation delegating to `service/h3_link_budget.rs`. Requires a new register
decision — and note it would be a post-C8 breaking change, so it needs v2-grade
justification per roadmap principle 4.

### F6 — Cross-platform `/status` memory metric — Effort: S

`/status` `memory_bytes` reads `/proc/self/statm` (Linux-only). Use the `sysinfo` crate or
report an explicit `supported: false` off-Linux. Low risk; schedulable any time after
Phase 2.

### F7 — Statistical sidelobe envelope/floor model — Effort: M/L (gated on register row F7 **and** reference sidelobe data)

**✅ UNBLOCKED 2026-07-15 (redesign pending, D-2) — P10 landed and removed the blocker.**
P10's Hankel / azimuthal-mode integrator fixed the aliasing, so off-axis gain is now numerically
correct and (per D-2) the served path carries **raw PO with the floor OFF**. F7's remaining scope
is the redesign — a **replacement** model for the idealised-PO tail beyond a physical θ_valid (not
a `max()` floor over an aliased pattern) — now properly informed. *History (parked 2026-07-13,
resolved-by-P10):* **⛔ PARKED 2026-07-13 — DID NOT MERGE `feat/f7-sidelobe-floor`. WAS BLOCKED ON P10.**
F7 was built on an inverted premise. Its founding claim (modelled sidelobes ~8–13 dB *too low*)
was measured with `high_accuracy()` on the small 3.7 m dish; the **served** path uses `fast()`,
where the pattern aliases **20–35 dB too HIGH** (unit **P10**, P0). A floor that only ever
*raises* gain therefore cannot fire — it engaged in **0 of 6** real service geometries. When F7
returns it must be a **replacement** model beyond θ_valid, not a `max()` floor over an aliased
pattern.

*Salvage on the branch:* the corrected derivation — **Ω = 4π (isotropic)** is the only
power-conserving choice (the floor is applied over the whole sphere), collapsing to
`floor = 1 − η_ruze`; **bounded by 0 dBi** (cannot swamp a main beam); tracks the NTIA 84-164
wide-angle **median** to ±6 dB/bin (~2.5 dB band-mean), pinned by
`reference_validation::sidelobe_floor_tracks_measured_median`, which also asserts power
conservation and the 0 dBi ceiling. The shipped **Ω = 0.25 sr was wrong** — a cone-derived level
applied across 4π, implying 136–326% of the antenna's total radiated power. Also reusable: the
`apply_sidelobe_floor` flag, the uncalibrated gate, the `PHYSICS_MODEL_VERSION` stamp, and the
digitised NTIA/NASA datasets. Register decision had been revised to **best-estimate (median)**,
not conservative envelope (maintainer, 2026-07-12) — that call still stands for the redesign.

**Redesign guidance (2026-07-15 post-P10 assessment) — read before scoping:**

1. **Prefer an incoherent power sum over both `max()` and hard substitution at θ_valid:**
   `G = 10·log₁₀(10^(PO/10) + 10^(floor/10))`. Scattered energy adds to the coherent pattern
   in *power*, so this is the physically motivated combination; it is continuous (no seam
   artifacts in heatmaps), converges to the floor exactly where idealised PO underestimates,
   and softens the need to pick a sharp θ_valid at all. Keep the salvaged level
   (`Ω = 4π`, `floor = 1 − η_ruze`, 0 dBi bound, NTIA-median pinning) and the existing honest
   framing that `(1 − η_ruze)` is a surface-quality-scaled empirical proxy, not a literal
   power budget (the measured floor's frequency-flatness already shows it is not literally
   Ruze scatter).
2. **Precondition — bound the boresight-tuner coupling first** (findings §7 item 2, currently
   untracked anywhere else): `calibrate/src/boresight_calibration.rs` tunes `surface_rms` as a
   catch-all for boresight gain deficits, and any floor keyed on `(1 − η_ruze)` converts that
   inflated σ directly into off-axis power. Bounded by the 0 dBi ceiling, but it must be
   measured on the real calibrations and documented (or the tuner constrained) before the
   floor ships.
3. **Build on P11's unified predicate** (land P11 first) so the floor's gate and the honesty
   warning can never diverge again.
4. **Sequence with P10-perf** (see that unit's note): decide θ_valid / the combination rule
   here first — it determines how much of P10-perf's wide-angle optimization work is even
   needed.
5. **Take the rear-hemisphere policy from P10-tail** as an input: θ > 90° is outside PO's
   physical validity regardless of convergence and is a natural part of this unit's θ_valid
   definition.

**✅ DONE 2026-07-12** — branch `feat/f7-sidelobe-floor`, commits `06b8cfe` (Ruze sidelobe
scatter floor + `apply_sidelobe_floor` flag), `7e043b4` (gate on uncalibrated antennas, all
endpoints), `08abfaa` (explicit batch endpoint floor coverage; heatmap inheritance noted),
`a9f0ac0` (calibrate `OMEGA_SCATTER`; conservative-envelope test), `044f1f5` (bump
`physics_model_version` 2 → 3, P1b). Floor applied as `max(pattern, floor)` at the spillover
seam in `model/pattern.rs::compute_gain`, gated on `correction_surface.is_none()` (reuses P1's
double-counting gate) and threaded identically through gain/batch/heatmap/H3. Validated as a
conservative envelope against NTIA 84-164 (`reference_validation::sidelobe_floor_conservative_envelope`)
and cross-checked vs NASA CR-159703 surface-error scaling
(`sidelobe_floor_surface_scaling_matches_nasa`). P8's warning message revised alongside this
unit to describe the modeled floor (still contains the stable marker substring
`"beyond the validated main-beam region"`).

**Two planner defaults were adopted as-is (no deviation):**
1. **No per-antenna surface correlation-length field.** Kept the single global `Ω_SCATTER =
   0.25 sr` called out as a "candidate floor mechanism" below; per-antenna width is deferred
   to unit **F9** rather than built here.
2. **Flat pedestal shape.** The floor is a constant-dBi wide-angle pedestal (no angle-dependent
   rolloff beyond the `max(pattern, floor)` seam itself), matching the "envelope, not detailed
   shape" goal — it does not attempt to reproduce near-in first-sidelobe structure.

Out of scope, unchanged from the plan: physical edge-diffraction/strut-scatter modeling, and
an ITU-mask envelope output mode (considered, not built).

---

Makes off-axis predictions *envelope-conservative* instead of systematically optimistic
(today: ~8–13 dB below the ITU-R S.580 mask — contract "Off-axis pattern / sidelobe
fidelity"). Approach: an angle-dependent floor applied at the existing spillover seam in
`pattern.rs::compute_gain` (`pattern.rs:284-302`, where `theta` is already in scope) — e.g.
`max(pattern, floor(θ))` — **without touching the aperture integral**, which also sidesteps
the numerical infeasibility of integrating far sidelobes for electrically huge dishes.
Candidate floor mechanisms: Ruze scattered-power floor derived from `surface_rms`
(the power the scalar Ruze efficiency removes from boresight has to go *somewhere*);
blockage-raised sidelobes when F3's geometry parameters exist; an optional ITU-mask
envelope output mode for regulatory screening. Reuses P1's uncalibrated-only gate
(calibrated antennas' correction surfaces absorb real sidelobe behavior within coverage);
bumps P1b's `physics_model_version`. **Data gate:** the S.580 harness test validates
pattern *shape* only — floor *levels* need real reference sidelobe data before this can
claim accuracy; without such data the unit must not start. **DATA GATE MET 2026-07-12:**
digitized reference datasets now committed at
`tests/fixtures/reference_datasets/ntia_84_164_sidelobe_statistics.psv` (120 rows:
statistical sidelobe-peak distributions for 22 C-band earth stations, 2.8–13 m,
D/λ 35–267, 1°–180°) and `nasa_cr159703_pattern_peaks.psv` (97 sidelobe peaks from
1.22/1.83 m prime-focus paraboloids at 12 GHz with surface-error provenance). **Register
row decided and unit implemented 2026-07-12** — see the "✅ DONE" block above.
**Explicitly out of scope**
(roadmap §6): physical edge-diffraction and strut-scatter modeling — domain-expert
territory, same class as F2. Until this lands, unit P8's warning is the honest answer.

**Candidate reference data identified 2026-07-12** (web search; URLs fetched & verified
that day; no machine-readable pattern file exists anywhere — all require digitization):
1. **NTIA Report 84-164** (Harman & Jennings 1984, public domain,
   https://its.ntia.gov/publications/download/84-164_ocr.pdf): measured sidelobe-peak
   statistics for 22 commercial earth-station antennas, 2.8–13 m, C-band, D/λ = 35–267
   (analyzed in D/λ<100 vs >100 subsets — brackets the harness's 3.7 m D/λ≈99 dish);
   per-angular-bin max/90%/median/10%/min over 1°–180° (Figs 16–21). Cheapest to extract
   (~10 bins × 5 percentiles × 6 figures) and directly matches the statistical-envelope
   design. Caveat: population statistics, not a single antenna's pattern.
2. **NASA CR-159703** (Collin & Gabel 1979,
   https://ntrs.nasa.gov/api/citations/19800004009/downloads/19800004009.pdf — use the
   `api/citations` link, the archive link is an HTML landing page): measured E/H-plane
   cuts of 1.22 m and 1.83 m prime-focus paraboloids (f/D 0.38, same topology as the
   model), 11.7–12.2 GHz, D/λ ≈ 49/73, ±12°, with a measured surface-error →
   sidelobe-degradation storyline (ties the floor to `surface_rms`). Scanned strip
   charts; moderate digitization effort.
3. (Optional) **ITU-R Report BO.2029** (2002, itu.int, free): ±180° cuts of 30+ small
   DTH dishes, 10.7–12.75 GHz — but nearly all offset-fed and D/λ ≤ 64; secondary.
Ruled out (do not re-search): ITU-R pattern library (analytical only), Eutelsat/Intelsat
approval lists (no pattern data published), ETSI EN 301 428 (masks/methods only),
ITU-R S.732 (method, no data; the S.465/S.580 campaign data lives in offline CCIR
reports — NTIA 84-164 is the accessible equivalent), DSN 810-005 (no wide-angle cuts
found), CommScope/RFS NSMA files (envelopes, not measurements), Ruze 1966 (paywalled;
mm-wave research dish, low incremental value). FCC IBFS filings have per-model measured
plots (e.g. GD/Prodelin 3.7/3.8 m) but typically only ±10°, raster, and pre-2015
attachments are blocked to scripted fetches.

### F8 — Reference sidelobe data collection (F7 data gate) — Effort: M
**✅ DONE 2026-07-12** — commit `1666e8c` (landed alongside the P8 warning). Digitized reference
sidelobe datasets committed under
`antenna-model/tests/fixtures/reference_datasets/sidelobe_data/`:
- `ntia_84_164_sidelobe_statistics.psv` — absolute-dBi percentile envelopes
  (max/p90/median/p10/min) per angular bin for 22 C-band earth stations (2.8–13 m, D/λ 35–267),
  1°–180°.
- `ntia_84_164_antennas.psv` — the backing antenna/gain table.
- `nasa_cr159703_pattern_peaks.psv` — 97 sidelobe peaks from 1.22/1.83 m prime-focus paraboloids
  at 12 GHz, with surface-error / defocus provenance.

No machine-readable pattern file existed upstream — all three required manual digitization
(source, method, and axis calibration recorded in each file header). Files live in a separate
`sidelobe_data/` subdirectory so the peak-gain harness (`load_all_reference_points`) does not
auto-ingest them. This **met F7's data gate**; the candidate-source survey (kept and ruled-out)
is preserved in the F7 unit above. **Blocks:** F7 (data gate — now met).

### F9 `[DECISION]` — Per-antenna sidelobe-floor width (surface correlation length) — Effort: M/L (deferred; gated on register row F9)

Enhancement to F7. F7 ships a **single global** effective scatter solid angle (`Ω_SCATTER`,
data-calibrated) setting the angular spread/shape of the Ruze scatter floor; the floor's
per-antenna *magnitude* already scales through each antenna's own `surface_rms` via `(1 − η_ruze)`.
F9 replaces the global constant with a **per-antenna** surface correlation length so the floor's
angular width is antenna-specific — enabling a *best-fit* floor shape rather than the conservative
flat pedestal F7 validates.

- **Decision (recommended default): defer, then implement only if the data demands it.** For F7's
  chosen goal — a one-sided *conservative envelope*, never optimistic — the global constant is the
  right altitude: NTIA 84-164 shows the wide-angle floor is roughly antenna-independent in absolute
  dBi, so a single spread constant bounds the reference set. The per-antenna field mainly buys
  *best-fit* fidelity, a goal F7 explicitly did **not** adopt.
- **Trigger to promote from deferred:** F7's Task 3 NASA surface-provenance cross-check cannot bound
  the data across the surface-condition range with one global `Ω_SCATTER`, **or** a later consumer
  needs best-fit (not envelope) off-axis levels.
- **Scope / cost (measured against P7's `axial_defocus_m` plumbing — 11 files, +119 lines):** a new
  optional reflector field `surface_correlation_length_m` threaded the full config→data→model chain:
  `calibrate/src/design_specs_loader.rs` (spec field + validation + a tuner-range-or-fixed decision),
  `config/settings.rs` (field + validation), `data/types.rs` `ReflectorGeometry` (struct + builder +
  `build()` + validation), `data/repository.rs` (config→data seam), `model/geometry.rs`
  `ReflectorGeometry` (positional `new()` signature + builder), the mm→m build seams at
  `evaluator.rs`/`h3_link_budget.rs`, and the calibrate artifact writers (`artifact_export.rs`,
  `boresight_calibration.rs` — bincode layout change; cheap only because no `.bin` artifacts exist,
  per P1b). Plus raw-struct-literal fixture churn across `service/*`, `data/types.rs`, `api/routes.rs`.
  Roughly **doubles F7's model + plumbing footprint**; Effort M/L. Make the field `Option<f64>`
  defaulting to F7's global `Ω_SCATTER` so absence is inert and no existing YAML/artifact changes.
- **Additive, no rework penalty:** F7's floor function already carries `theta` in its signature, so
  the per-antenna width slots in as a shape term without touching the seam or the uncalibrated gate.
- **Exit criteria (when undeferred):** register row F9 Decided; `surface_correlation_length_m` plumbed
  end-to-end (optional, global-default fallback); F7's conservative-envelope test still passes **and**
  a new best-fit test shows per-antenna width tracks the NASA surface-condition/defocus progression
  tighter than the global constant; `physics_model_version` bumped (P1b); `docs/domain-contract.md` +
  `docs/api-documentation.md` updated.
- **Out of scope (inherited from F7 / roadmap §6):** physical edge-diffraction / strut-scatter
  mechanisms; the ITU-mask envelope output mode (a separate F7 follow-up).
- **Depends on:** F7 landed.
