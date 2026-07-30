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
4. `openapi.yaml` is **generated** (unit C7, 2026-07-29) — never hand-edit it. After any
   request/response schema, handler `#[utoipa::path]`, or `api::openapi` change, run
   `cargo run -p antenna-model --bin generate_openapi` and commit the diff **as a contract
   change**; `tests/openapi_spec.rs` fails until the committed file matches the code. The
   error/warning tables in `docs/api-documentation.md` remain hand-maintained (their own
   tests pin them).
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
    ├─ S1 ─ S2 ─ S3(after Phase 1) ─ S4 ─ S5 │  (Phase 2 DONE 2026-07-23,
    │  S1b, S2b (Phase 2 review follow-ups)   │   S1b+S2b open — see below)
    ├─ S6                                    │
    ├─ C3 ─ C4 ─ C2 ─ C9 ─ C8 ─ C7           │
    │  C9 DONE 2026-07-26 (loss_db now peak- │
    │      referenced on both heatmaps)      │
    │  C8 STAGES 1-2 OF 4 DONE (aim-point    │
    │      renames 07-26; required           │
    │      coordinate_system 07-27; stages   │
    │      3–4 remain before C7 can freeze)  │
    │  C12, C13, C14 filed 2026-07-26 out of │
    │      C8 stage 1's findings — none are  │
    │      stage 1's to fix (each moves a    │
    │      contract or a computed value)     │
    │  C1 DONE 2026-07-25 (did not fold into │
    │      C8; C9 then C8 stage 4 update it) │
    │  C10, C11 DONE 2026-07-25 (both filed  │
    │      out of C1's findings; C11 is C7's │
    │      prose sibling, landed before C8   │
    │      so it guards C8's own rewrites)   │
    ├─ D1 ─ D2 ─ D3;  D6                     │
    │  D1 DONE 2026-07-29 (serializer.rs →    │
    │      sidecar.rs); filed D10 + D11, and  │
    │      handed findings to D2 and D6       │
    │  D10, D11 DONE 2026-07-29 (correctness- │
    │      class; both landed BEFORE the      │
    │      calibrate CLI integration-test     │
    │      work, as required)                 │
    │  (D10,D11) ─ D12 ─ D13, D14             │
    │  D12 DONE 2026-07-30 (branch feat/d12-  │
    │      calibrate-cli-e2e): CLI e2e on     │
    │      perturbed-truth synthetic, known-  │
    │      answer recovery; filed 2 findings  │
    │      (edge-collapse; tune-parameters    │
    │      broken) + 1 flake fix (D11 log-    │
    │      capture tests)                     │
    │      D13, D14 open (filed 2026-07-29:   │
    │      real-data boresight + NASA-        │
    │      anchored full-mode artifacts;      │
    │      D13/D14 also need D2; D14 feeds D9)│
    └─ (Phases 1–3 done) ─ D4 ─ D7
Superseded by C8 (do not implement): S7, C5, C6
Phase 5: F1..F9 (F8 done) gated on register rows (P3, P5/F4, F5, D9, F9); P1 + C8 DECIDED 2026-07-08;
P7 DECIDED 2026-07-10 (auto-refocus), IMPLEMENTED 2026-07-10 (branch
feat/p7-phase-center-auto-refocus; P1b dependency implemented in the same branch);
P8 IMPLEMENTED 2026-07-12 (branch feat/p8-off-axis-honesty-warning);
F7 IMPLEMENTED 2026-07-12 then PARKED 2026-07-13 (inverted premise — see the F7 unit);
UNBLOCKED by P10 2026-07-15; REDESIGN DECIDED 2026-07-16 (power-sum + obliquity factor +
floor-only rear hemisphere) — sequence WITH P10-perf (they interact);
P10 DONE 2026-07-15; post-P10 assessment follow-ups filed 2026-07-15: P10-perf, P10-tail, P11
(P10-tail + P11 DONE 2026-07-15/16); P2 DECIDED 2026-07-16 (REMOVE the Seidel mode; Stage-1
gate tripped and removal re-affirmed same day — the terms are wrong-sign/wrong-scale additions
on top of complete exact physics, not duplicates); P3 DECIDED + EXECUTED 2026-07-16 (document +
flag; warning pinned on all four endpoints, H3 cache-hit gap fixed)
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
- **RE-SCOPED 2026-07-16/17 — F7 landed (branch `feat/f7-redesign-power-sum-obliquity`).** F7 landed
  with a **forward power sum** (PO is still computed at every forward angle — forward wide-angle
  cost is **unchanged** by F7) and a **floor-only rear hemisphere** (rear aperture integration is
  now **SKIPPED** on the uncorrected-physics served path — the pathological θ→180° chirp-budget
  case no longer runs there at all). Re-scope accordingly: the remaining hot case is **forward
  wide-angle Ka on offset-feed antennas** (the `dsn_34m` numbers above are unaffected by F7 and
  still apply); the rear-hemisphere half of the original latency concern is moot for antennas
  served with uncorrected physics and unchanged (still slow, still correct) for calibrated
  antennas that still run the rear PO integral.

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

### P2 `[DECISION]` — Seidel higher-order aberration terms: REMOVE (double-counted) — Effort: S/M

**DECIDED 2026-07-16 (maintainer): remove after a redundancy check** — superseding the
original "fence" default, on new evidence found 2026-07-16: the base coma model
(`phase_feed_displacement`, `model/phase.rs:156`) is the **exact geometric path difference**
`k·(path_displaced − path_ideal)` and therefore already contains *every* order of the
feed-displacement aberration (steering, defocus, astigmatism, coma, distortion) exactly. The
Seidel terms in `higher_order_aberrations` (`model/edge_cases.rs:253`) are low-order Taylor
approximations of that same physics, ADDED ON TOP of the exact value in `aperture_plane_g`
(`model/integration.rs`) when `use_higher_order_aberrations` is set — i.e. the
`HigherOrderAberrations` mode double-counts the δ², δ³ terms and makes the phase *less*
accurate than the standard mode it replaces. They are not merely "unverified"; they are
structurally redundant.

- **Work (in order):**
  1. **Redundancy check first (the safety gate):** a test that numerically extracts the
     δ²·ρ²·cos(2(φ'−α)), δ²·ρ², and δ³·ρ³·cos(φ'−α) components from the exact
     `phase_feed_displacement` output (fit at a 0.35f offset across the aperture) and shows
     they match the Seidel forms to leading order — *proving* the double count, not just
     asserting it. **If this check fails, STOP: revert to the original fence plan and
     re-open the register row.**

     > **⚠️ STAGE-1 GATE TRIPPED 2026-07-16 — REMOVAL RE-AFFIRMED BY MAINTAINER SAME DAY.**
     > The check (`p2_stage1_seidel_double_count_redundancy_check`, `edge_cases.rs:537`;
     > extraction cross-checked against an independent closed form to 4 decimals) split:
     > the exact phase **does** carry the full δ²/δ³ aberration content — astigmatism
     > (cos2φ′), field curvature (constant), distortion (cos1φ′), plus a trefoil (cos3φ′)
     > with no Seidel counterpart — but the Seidel terms **do not match** it: astigmatism
     > sign-flipped at every radius; field-curvature/distortion ratios swing ~45×/~89×
     > across ρ (spurious 1/f signature); distortion has the wrong pupil power (Seidel
     > coded ρ³ where both the exact model and classical aberration theory give leading
     > ρ¹). The "exact duplicate" rationale is falsified; the corrected rationale is
     > **stronger**: the mode stacks wrong-sign/wrong-scale/wrong-shape terms on top of
     > already-complete exact physics, so removal makes the 0.3–0.5f band strictly *more*
     > correct. Proceed to step 2 under the corrected rationale. **Keep the Stage-1 test,
     > renamed as a completeness pin** (e.g.
     > `exact_feed_displacement_phase_contains_all_low_order_aberrations`): its
     > load-bearing half (exact model carries the full low-order content) is the permanent
     > justification for the mode's absence; the failed Seidel-correspondence half becomes
     > doc-comment history explaining why the mode was removed rather than fixed. NOTE:
     > because the removed terms were wrong-sign (not duplicates), the 0.3–0.5f
     > before/after gain delta may be *larger* than the double-count framing implied —
     > expected; the step-3 regression test pins the new values and the
     > `PHYSICS_MODEL_VERSION` bump is non-negotiable.
  2. Remove `higher_order_aberrations`, the `HigherOrderAberrations` computation mode
     (`pattern.rs` dispatch + `edge_cases.rs` mode selection), the
     `use_higher_order_aberrations` param plumbing, and the mode-path branches in
     `integration.rs` (`aperture_plane_g`, `mode_count_for` asymmetry handling stays — it
     serves the illumination/coma cases, not Seidel).
  3. Offsets formerly routed to the removed mode (0.3f–0.5f) fall through to
     `StandardPhysicalOptics`, whose exact coma phase covers them; the ray-tracing threshold
     (>0.5f) is untouched.
- **Exit criteria:** the Stage-1 test committed and green **in its renamed completeness-pin
  form** (asserting the exact model's full low-order aberration content; the failed
  Seidel-correspondence assertion documented, not asserted); mode removed; **no served value
  changes for any enabled antenna** (all offsets ≤0.027f never entered the mode — every
  existing anchor/gain test passes unchanged); a 0.3–0.5f-offset regression test pins the
  new (exact-only) behavior; `PHYSICS_MODEL_VERSION` bumped (values in the 0.3–0.5f band
  change by construction — that is the fix); domain-contract + CLAUDE.md coma sections
  updated (including the corrected removal rationale).
- **Depends on:** G1 (done). **Coordinate with:** F7 redesign (both bump
  `PHYSICS_MODEL_VERSION`; land P2 first or batch the bump).

### P3 `[DECISION]` — Ray-trace stub (feed offsets > 0.5·f) disposition — Effort: S

**DECIDED 2026-07-16 (maintainer): document + flag** — the recommended default, adopted
as-is. Real ray tracing stays gated as feature F2; rejection was ruled out (warn-don't-refuse
philosophy, heatmap grid totality). Execute the unit as specified below.

> **✅ DONE 2026-07-16 (pending commit).** All exit criteria met; no physics/`ray_trace.rs`
> math changed; `PHYSICS_MODEL_VERSION` unchanged (4); every existing served value unchanged
> (full workspace green). Landed:
> - **Per-endpoint tests** (`tests/integration/ray_trace_stub_warning_tests.rs`): gain, batch,
>   heatmap, h3-heatmap each pinned to surface the stub warning for a > 0.5·f request; a
>   small-offset negative control; **and a warm-cache H3 test** proving the fix (verified
>   load-bearing — the warm-cache assertion fails when the fix is reverted). Large-offset
>   geometry reuses `builders::uncalibrated_antenna_request` (feed aimed at ground beside the
>   vehicle vs. boresight at a 400 km satellite ⇒ offset ≈ 3·f). *Note:*
>   `geo_large_feed_offset.json` was NOT used — from GEO the feed/boresight angular gap is
>   small (offset < 0.5·f); the "ready fixture" premise in the plan was wrong, corrected during
>   execution.
> - **h3-heatmap cache-hit gap fixed:** the ray-tracing warning was captured only inside the H3
>   gain-cache miss closure, so a warm (shared, persistent) cache dropped it. Now re-emitted at
>   the service layer via `service/evaluator.rs::ray_trace_stub_warning` **outside** the closure,
>   mirroring the P8 off-axis and P10-tail rear-hemisphere precedents in `h3_link_budget.rs`. The
>   warning string was extracted to `model::pattern::RAY_TRACING_STUB_WARNING` (a `pub const`) as
>   the single source of truth shared by the model push and the service re-emission.
> - **Docs:** `docs/domain-contract.md` gained a "Large feed offsets (> 0.5·f): ray-tracing stub"
>   subsection; `openapi.yaml` heatmap + h3-heatmap `warnings` descriptions now mention the stub
>   warning (GainResponse already did); `docs/api-documentation.md` gained a large-feed-offset
>   caveat. Plan: `docs/plan-p3-ray-trace-stub-disposition.md`.

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
**✅ DONE 2026-07-17 (pending commit).** Canonical constants `F_OVER_D_MIN`/`F_OVER_D_MAX`
([0.2, 1.0]) added to `model/geometry.rs`; the silent no-op branch in
`ReflectorGeometry::validate` now returns a typed `ValidationError` naming the ratio and its
inputs. All load/validation seams aligned to the same constants (they previously disagreed):
`data/types.rs` artifact-load validate ((0, 2.0] → [0.2, 1.0]), `config/settings.rs`
design-spec validate (same), `calibrate/design_specs_loader.rs` ([0.2, 2.0] → [0.2, 1.0]),
`calibrate/antenna_config.rs` ((0, 1.0) exclusive → [0.2, 1.0]). Six new tests (TDD, watched
red first) cover below-range/above-range rejection + boundary acceptance at model, data,
settings, and both calibrate seams; full workspace green (in-range behavior unchanged).
Domain-contract `f_over_d` glossary row + open-items entry re-trued in the same change.

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
**✅ DONE 2026-07-17 (pending commit).** The two formulas were verified textually and
numerically identical (`gain_db − 10·log₁₀(T)`) before consolidating — no escalation needed.
One shared implementation now exists: `pattern::g_over_t_from_gain_db` (re-exported from
`model`), called by both `pattern::compute_g_over_t` (which keeps its T>0 check) and the H3
per-cell path (which passes its already-corrected gain — deliberately NOT `compute_g_over_t`
itself, which would recompute gain without the correction surface and change served values).
H3 output pinned by `test_h3_g_over_t_matches_gain_minus_10log10_t` (written before the
refactor, green across it; also pins G/T absent when `temperature_k` is absent). The
evaluator module-doc header no longer advertises a `g_over_t_db` output on `GainResponse`.
Domain-contract gains a `temperature_k` glossary row: T is a user-supplied passthrough
(noise-temperature modeling = F4); the missing H3 temperature bound stays S6's job. No
warning/schema text changed, so no openapi.yaml mirror needed (standing rule 4).

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
aggregation dedups it; C8 stage 3 did the typed-code conversion (2026-07-27) — and note
the dedup key is now `(code, message)`, so the constant-message requirement still holds.

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
  4. String warning at the time; C8 stage 3 converted it to typed code
     `off_axis_unvalidated` (2026-07-27).
- **Exit criteria:** warning appears on all four compute endpoints for a large-θ
  uncalibrated query (test per endpoint); no warning inside the main beam; existing tests
  untouched; `docs/api-documentation.md` accuracy-caveat section updated; openapi.yaml
  mirrored (standing rule 4).
- **Depends on:** G1. Independent of P7. Sequence before or with C8 stage 3.

### P6 — Refresh `docs/domain-contract.md` "Open items" — Effort: S (phase closer)
**✅ DONE 2026-07-18 (pending commit) — Phase 1 closed.** All four exit criteria verified
against code first (every stale claim re-checked, not assumed):
1. Resolved items marked with pointers: `phase_center_offset` → axial-defocus rows were
   already current (P7-era edit); the duplicate-Ruze item is now marked resolved — `surface.rs`
   is deleted (grep-verified) and `pattern.rs::ruze_efficiency` is the single implementation
   (glossary `surface_rms` row + open item both updated).
2. `transparency_at_wavelength` open item + `mesh_spacing` glossary row now cross-reference
   **D8** (still dead code, re-verified at `geometry.rs:473`; P1 deliberately did not wire it);
   the `f_over_d` item cross-references **P4** (resolved 2026-07-17 in the P4 pass).
3. P1/P2/P3/P5/P7/P8 outcomes confirmed present in the contract (efficiency-terms section,
   offset-gate note, ray-trace-stub section, `temperature_k` row, P7 glossary rows, P8
   off-axis section) — no gaps found.
4. The design-doc-drift process item was **already resolved upstream**: design-doc §2.5 now
   documents the beam-steering sign flip and BDF division matching
   `coordinates.rs::to_feed_displacement_with_bdf` exactly — verified line-by-line and marked
   resolved-with-history in the contract.
Also fixed while verifying: the `axial_defocus` glossary row's stale `integration.rs:529`
pointer (now `:911`/`:752`/`:981` post-P10). Docs-only change; no code touched.

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

> **🔴 OPEN gap — found by the Phase 2 review 2026-07-24, tracked as S1b.** The 413 is
> gated entirely on a parseable `content-length` header (`api/middleware.rs:348-385`) with
> **no `else` arm**, so a `Transfer-Encoding: chunked` POST bypasses it and poem's `Json`
> extractor buffers the body unbounded. The docstring at `api/middleware.rs:265-268` claims
> this matches "the framework-blessed level" of `poem::middleware::SizeLimit` — it does not:
> poem's `SizeLimit` returns **411 Length Required** when the header is absent, it does not
> fall through. Fix in S1b: an `else` arm that bounds the body with
> `req.take_body().into_bytes_limit(max)` → `set_body(bytes)` (caps memory at `max + 4096`,
> keeps chunked clients working, returns the same 413 body). A blanket 411 is wrong here —
> the middleware also wraps every GET. **S1's exit criteria are met only for
> content-length-bearing requests until S1b lands.**

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

> **🔴 OPEN gap — found by the Phase 2 review 2026-07-24, tracked as S2b.**
> `request_timeout_secs` is **unenforceable on `POST /api/v1/gain`**. `RequestTimeout` is a
> `tokio::time::timeout` around the endpoint future; `compute_gain` (`api/handlers.rs:222`)
> runs the synchronous physics call directly on the async task, so the future never yields
> and the timeout can never preempt it. It is the only heavy-compute handler that does not
> `spawn_blocking` — `compute_gain_batch` (`:332`), `generate_heatmap_endpoint` (`:484`) and
> `h3_link_budget` (`:1027`) all do. Consequence: **S3's `integration_budget_ms` is the only
> live bound on single-gain latency, and that is nowhere documented.** Fix in S2b: wrap the
> compute in `spawn_blocking`, matching the other three handlers.

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

> **✅ DONE 2026-07-22 (pending commit).** Both halves landed; full workspace green.
> Plan: `docs/plan-s4-admission-control.md`.
> - **`worker_threads` wired:** `api::apply_worker_threads` calls
>   `rayon::ThreadPoolBuilder::build_global` once at startup (`api/mod.rs`), guarded — a
>   positive value sizes the global pool, `0` = auto, and an already-initialized pool
>   logs-and-continues (never panics). The "Performance configuration" startup log now
>   reports `worker_threads_effective = rayon::current_num_threads()` alongside the
>   configured value and the admission limit.
> - **Admission control:** new `ConcurrencyLimit` middleware (`api/middleware.rs`) holding
>   a shared `Arc<tokio::sync::Semaphore>`, applied per-endpoint to the three heavy routes
>   (`gain/batch`, `heatmap`, `h3-heatmap`) in `routes.rs` so one budget caps *total*
>   concurrent heavy work. Non-blocking `try_acquire_owned`; on saturation returns
>   **`503 service_overloaded` + `Retry-After`** built as an `Ok(Response)` (the
>   `poem::Error::from_string` path used by 413/504 has no header channel — this is the
>   load-bearing implementation detail, pinned by test). Cheap endpoints (single gain,
>   health/ready/status, listings) are never wrapped.
> - **Two config fields** (`config/settings.rs`, `config/service.yaml`):
>   `max_concurrent_heavy_requests` (**default 0 = unlimited/disabled** — maintainer chose
>   off-by-default 2026-07-22, the plan's recommended option) and
>   `admission_retry_after_secs` (default 5). Limit `0` makes the middleware a transparent
>   pass-through, so every existing test (incl. `concurrent_tests.rs`) stays green unchanged.
> - **Tests (all deterministic — no `sleep`, no real compute, no core/load dependence):**
>   4 middleware unit tests (reject-saturated w/ header+content-type+body, admit-free,
>   disabled-passthrough, permit-release) cover the *mechanism*; 1 `routes.rs` wiring test
>   (`build_app` given an already-exhausted semaphore ⇒ each heavy endpoint 503s *before its
>   handler runs*, cheap endpoints untouched) covers *which endpoints are limited*; plus 3
>   config-default tests and an `apply_worker_threads` graceful-Err test (warms the rayon pool
>   first, then asserts the already-initialized path doesn't panic). **An earlier draft used a
>   reqwest/`sleep` e2e file racing a real heavy heatmap against the permit; it was deleted** —
>   it starved unrelated batch tests in the full-suite run (CPU contention) and was
>   environment-dependent. `try_acquire` on an exhausted semaphore rejects instantly with zero
>   compute, so the injected-semaphore unit test proves the wiring deterministically and the
>   real-server round-trip added no coverage the middleware stack didn't already exercise.
> - **Docs/spec:** `openapi.yaml` gained a `503` (+ `Retry-After` header) on the two
>   *documented* heavy endpoints (`gain/batch`, `heatmap`); `/h3-heatmap` is absent from
>   openapi entirely (its documentation is unit **C1**) so its 503 will be added when C1 lands.
>   `docs/api-documentation.md` gained an "Admission Control" section.
>
> **Note — the S2 status-code note said "429 or 503"; chose 503 + Retry-After** per the
> transient-vs-deterministic rationale below (429 would misclassify overload as a
> client-rate-limit fault). `Retry-After` is a small fixed config-driven backoff, not a
> per-request service-time estimate.

- **Entrance / read first:** `settings.rs` performance section; `service/batch.rs`,
  `service/heatmap.rs`, `service/h3_link_budget.rs` all use rayon's **global** pool; no
  concurrency-limit middleware anywhere.
- **Exit criteria:**
  1. `performance.worker_threads` wired via `rayon::ThreadPoolBuilder::build_global` at
     startup (recommended) — or removed from config; recommended: wire it.
  2. A semaphore caps concurrent heavy requests (batch/heatmap/h3-heatmap); when saturated,
     return 429 or 503 with the standard JSON error; limit configurable.
- **Status-code note (from S2, 2026-07-18):** admission-control saturation *is* a transient
  condition — a slot frees when an in-flight request finishes — so this is the place for
  **`503 + Retry-After`** with a *defensible* `Retry-After` (≈ a typical heavy request's service
  time, or a small fixed backoff). This is deliberately distinct from S2's request-timeout, which
  returns **504** with **no** `Retry-After` because that failure is deterministic in the request
  payload (retrying the same heavy grid re-fails identically; the remedy is a smaller request, not
  waiting). Do not reuse 504 here, and do not omit `Retry-After` on the 503. See the 504-vs-503
  rationale in `RequestTimeout` (`api/middleware.rs`) and `docs/plan-s2-request-timeout.md`.
- **Gotchas:** `build_global` can only be called once and errors if a pool already exists
  (tests may have initialized it) — handle the `Err` gracefully. Do not create per-request
  rayon pools.
- **Depends on:** S1, S2 (same middleware stack — land sequentially).

### S5 — Real graceful shutdown, readiness lifecycle, honor `fail_fast` — Effort: M

> **✅ DONE 2026-07-23 — last *planned* Phase 2 unit.** (It did **not** close Phase 2
> outright: the 2026-07-24 Phase 2 review, which ran in parallel with S5 and so was not
> reflected in this closeout, found two open gaps in S1 and S2 — now filed as **S1b** and
> **S2b** at the end of this phase. Phase 2 flips to unqualified DONE when those land.)
> All four S5 exit criteria met; full workspace gate
> green (`scripts/check.sh`: fmt + `clippy --workspace --all-targets -D warnings` +
> `cargo test --workspace`; only `cargo audit` finding is the pre-existing allowed `paste`
> RUSTSEC-2024-0436). Plan: `docs/plan-s5-graceful-shutdown.md`. Seven commits
> (`88d0268`…`4013bda`) on `feat/s5-graceful-shutdown`, plus this closeout commit.
> - **Readiness lifecycle:** `AppState::new` now starts readiness **false**; the production
>   path flips it true (`mark_ready()`) only after a healthy calibration load, and
>   `begin_shutdown` flips it false at the top of graceful shutdown. `/ready` therefore 503s
>   during startup, on a failed load, and for the whole drain window.
> - **`fail_fast` honored:** new `initialize_repository()` + `LoadOutcome` seam
>   (`api/mod.rs`). `calibration.fail_fast: true` (the shipped default) + a failed load →
>   returns `Err(io::Error)` (naming the knob + the CWD) which flows through
>   `start_server_with_config`'s existing `?` up to `main.rs`'s existing `exit(1)` — **no new
>   code in `main.rs`, no `process::exit` inside `api/`**. `fail_fast: false` → starts
>   **degraded** (empty repository, readiness stays false).
> - **Degraded state is honest, shapes preserved:** `/health` stays **HTTP 200** always
>   (it's the k8s *liveness* probe — a non-200 there restart-loops the pod over a data
>   problem a restart can't fix), reporting `status: "degraded"` when
>   `repository.antenna_count() == 0`, `"healthy"` otherwise. Derived from the empty
>   repository, **not** a new `AppState` flag. `/status` now *always* emits `antenna_count`
>   /`antenna_ids` (`0`/`[]` when degraded) so monitoring can tell "0 antennas" from "field
>   not implemented" — previously production never called `set_antenna_ids` at all, so both
>   were always omitted.
> - **Real graceful shutdown:** `shutdown_cleanup()` (dead since Sprint 5) now has a caller —
>   invoked unconditionally after the drain, on both the clean and errored server-return
>   path. The drain is **bounded** (`Some(drain_timeout)`, was `None`). The readiness-flip +
>   pre-drain pause happen *inside* the future handed to `run_with_graceful_shutdown` (poem
>   stops accepting the instant that future resolves, so the delay had to be there, not
>   after).
> - **Two config fields** (`config/settings.rs`, `config/service.yaml`):
>   `server.shutdown_readiness_delay_secs` (**default 0** — flip-and-drain immediately, keeps
>   local Ctrl+C snappy; recommended 5 in k8s for LB propagation) and
>   `server.shutdown_timeout_secs` (**default 25**, so the *default* pairing 0 + 25 leaves
>   5 s of the chart's `terminationGracePeriodSeconds: 30` for cleanup before SIGKILL;
>   operators running the recommended delay of 5 must lower the timeout to **20** — 5 + 25
>   lands exactly on 30 and leaves cleanup nothing. Corrected 2026-07-24 after review;
>   the original note claimed 5 + 25 fit).
> - **The zero-calibration load error now distinguishes** "No antennas enabled in
>   configuration" from "All N enabled antenna(s) failed to load (M error(s))" — same `Err`
>   shape, different message (`data/repository.rs`), verified by two independent branch
>   mutations.
> - **Tests:** readiness-starts-false pin; two `initialize_repository` outcome tests
>   (fail_fast→Err naming the knob, degraded→`Ok((empty, Degraded))`) + a healthy-fixtures
>   test; `HealthResponse::degraded` shape pin; `begin_shutdown` paused-clock delay test
>   (mutation-verified — deleting the sleep fails it) + zero-delay test; config default +
>   YAML round-trip tests; `/status`-degraded and `/health`-healthy route tests; and
>   `server_test.rs` extended to assert `/ready`→200 + `antenna_count>0` on the real startup
>   path (this caught that `TestServer` never populated `antenna_ids` — fixed in
>   `helpers.rs`, mirroring production).
> - **Docs/spec:** `openapi.yaml` `/health` documents `healthy`+`degraded`/always-200, and
>   the pre-existing `/ready` **503 schema drift is fixed** (referenced `ErrorResponse`; the
>   handler returns `{"status":"not_ready"}` = `HealthResponse`). `docs/api-documentation.md`
>   gained a Service Lifecycle section (startup/shutdown/fail_fast + both knobs).
>
> **Discovered debt (found while implementing S5, none in scope — filed for follow-up):**
> - **`test_startup_with_corrupted_calibration_binary` tests nothing.** Its fixture
>   (`tests/integration/error_tests.rs:120-128`) uses field names (`antenna_id`, top-level
>   `feeds:`, `feed_id`) that don't match `AntennaConfigEntry`'s serde shape (`id`, `name`,
>   `calibration_file`, `enabled`; `feeds` only nests under `design_specs`, keyed by `id`),
>   so it fails to deserialize and the test ends in `let _ = result;`. Pre-existing.
> - **`tests/fixtures/test_antennas.yaml` `calibration_file` paths are crate-root-relative,
>   not `data_directory`-relative,** so the intuitive `data_directory = tests/fixtures`
>   doubles the prefix and two antennas silently fail to load. Fixture-based tests paper over
>   it with `fail_fast: false`; now that S5 makes `fail_fast` real, that workaround silently
>   weakens any healthy-load assertion. Fix the fixture paths or document the convention.
> - **The `fail_fast` fatal error names the CWD but not the failing antenna IDs** (the
>   per-antenna `warn!` at `repository.rs:105` has them; the fatal line doesn't).
> - **Startup concerns are accumulating in `api/mod.rs`** — `apply_worker_threads` (S4) +
>   `initialize_repository`/`begin_shutdown` (S5). A third justifies extracting
>   `api/startup.rs` / `api/shutdown.rs`.

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
implement this unit. **✅ Discharged by C8 stage 2, 2026-07-27**: the tag is required, the
heuristic and the `coordinate_ambiguity_warnings` plumbing are deleted, and both stale
threshold comments this unit would have fixed (`schemas.rs:9` said 1000 km against a 6400 km
constant — deleted with the machinery; `validator.rs` said 10,000 km against a 400,000 km
constant — corrected) are gone.

### S1b — Close the chunked-encoding bypass of the 413 — Effort: S

**✅ DONE 2026-07-24** (filed the same day by the Phase 2 review). All four exit criteria met;
Phase 2 banner flipped to unqualified DONE.

- **What landed:** `RequestSizeTrackerImpl::call` gained an else arm for the
  undeclared-length case: `req.take_body().into_bytes_limit(max)`, then `set_body` the
  buffered bytes back so downstream extractors still see them. `ReadBodyError::PayloadTooLarge`
  maps to the same `413` + `payload_too_large` JSON as the header path (shared
  `too_large_error` constructor, so the two arms cannot drift); other read errors map to
  `400 invalid_request_body` rather than handing a truncated body downstream. The soft
  `warn_request_size` log fires on this arm too, keyed on bytes actually read.
- **Regression proof (measured, not assumed):** with the else arm disabled, an oversized
  chunked POST returned **200** — the bypass, reproduced. With it, **413**.
- **Tests:** `integration::error_tests::test_chunked_request_body_size_limit` (over limit →
  413 + `payload_too_large`), `…::test_chunked_request_under_limit_succeeds` (under limit →
  200 with a real gain, proving the body is handed back), `…::test_bodyless_get_unaffected_by_size_limit`
  (GET `/health` and `/api/v1/antennas` under a 1-byte limit → 200, pinning that we do *not*
  adopt poem's blanket 411). A `helpers::raw_http` module speaks HTTP/1.1 over `TcpStream`,
  because `reqwest` will not emit chunked bodies without its optional `stream` feature and the
  wire framing is the whole point.
- **Docs:** the false "matching `poem::middleware::SizeLimit`" docstring is replaced with a
  two-arm description that states the 411 divergence and why (this middleware wraps the GETs);
  `docs/api-documentation.md` "Request Body Size Limit" and all three `413` blocks in
  `openapi.yaml` now describe both framings.

*Original unit follows.*

- **Entrance / read first:** `api/middleware.rs:338-409` (`RequestSizeTrackerImpl::call`) —
  the whole 413 lives inside `if let Some(size) = headers().get("content-length")…`; and
  the docstring at `:259-268`, whose "framework-blessed level, matching
  `poem::middleware::SizeLimit`" claim is **false on the header-absent branch** (poem
  returns 411 there; we fall through). Read poem 3.1.12's
  `Body::into_bytes_limit` (`src/body.rs:213`) — it caps the buffer at `limit + 4096` and
  returns `ReadBodyError::PayloadTooLarge`.
- **Exit criteria:**
  1. A `Transfer-Encoding: chunked` POST whose body exceeds `max_body_size_bytes` gets
     **413** with the project's standard JSON body — the same body as the header path.
  2. A chunked POST *under* the limit still succeeds (no regression for chunked clients).
  3. GET requests, which never carry `content-length`, are unaffected.
  4. The `:259-268` docstring corrected to describe what the code actually does.
- **Gotchas:** do **not** mirror poem's blanket 411 — this middleware wraps every route
  including the GETs. The else arm must `req.take_body().into_bytes_limit(max)` and
  `set_body` the result back, or the handler gets an empty body. Peak memory becomes
  `max + 4096` on the chunked path, which is what the `Json` extractor would have consumed
  anyway — this bounds it rather than adding cost.
- **Depends on:** S1.

### S2b — Make the request timeout enforceable on `POST /api/v1/gain` — Effort: S

**✅ DONE 2026-07-24** (filed the same day by the Phase 2 review). All three exit criteria met;
Phase 2 banner flipped to unqualified DONE.

- **What landed:** `compute_gain` now runs `compute_gain_from_request_with_budget` inside
  `tokio::task::spawn_blocking`, with the same `JoinError` → `500 internal_error` handling as
  `compute_gain_batch`. All four compute handlers are structurally identical on this point now.
- **Regression proof (measured, not assumed):** with the handler reverted to the inline call, a
  gain request taking **2.62 s** against a **10 ms** deadline returned **200** — a late success,
  the timeout structurally unable to fire. With `spawn_blocking`, **504**. Re-confirmed against
  the final paused-clock test: **2.68 s** of real compute with mocked time advanced **31 s**
  still produced no `request_timeout`.
- **Tests:** `integration::timeout_tests::test_heavy_single_gain_times_out_with_504`, which
  asserts **no wall-clock threshold**. It runs on a paused clock (`start_paused`), where mocked
  time advances only while the runtime is idle — and "the executor is not idle" *is* the bug, so
  the property under test becomes the thing that gates the clock. Fixed: the handler parks at the
  `spawn_blocking` join, the runtime idles, `advance` past the deadline takes effect → 504
  `request_timeout`. Broken: the task never yields, mocked time cannot move, and the timeout
  polls an already-ready future → it can never fire, whatever the deadline.
  `…::test_single_gain_under_timeout_still_succeeds` is the end-to-end control over real HTTP.
- **Test-design notes (each was measured, not assumed):**
  - **In process, not over a socket.** The test drives the app via `Endpoint::call`
    (`helpers::build_in_process_app` / `call_json`) rather than `TestServer` + reqwest. Over a
    socket the mocked clock cannot order the request against the deadline: a 35 s mocked sleep
    completed in **320 µs of real time**, so the deadline elapsed before the request crossed
    loopback, the timer registered after the jump, and the request returned a late 200.
    `start_paused` with `TestServer` is worse still — the health-check retry loop auto-advances
    through all 50 attempts instantly and the server never boots. No middleware is bypassed:
    `create_routes_with_timeout` builds the same stack the server binds to a port.
  - **The load-bearing assertion is the `error` code, not the status.** S3's
    `computation_budget_exceeded` is also a 504, so a status-only check could pass for the wrong
    reason.
  - **`integration_budget_ms: 250`** in this test bounds the *un-cancellable* background rayon
    (S2's standing limitation), which the runtime otherwise waits for at drop: **2.71 s → 0.52 s**.
    It cannot affect the assertion — the 504 is produced in mocked time microseconds in, long
    before 250 ms of real time elapse. It does change how the pre-S2b handler *fails* this test
    (wrong `error` code rather than a 200); the production symptom, with the 30 s default budget,
    was the late 200.
  - The request's ~2.7 s debug cost is a **race margin, not a threshold** — the clock is advanced
    microseconds after the offload, so the margin is ~5 orders of magnitude and nothing asserts
    on the duration.
- **Docs:** `docs/api-documentation.md` "Request Timeout" previously stated that `/api/v1/gain`
  is *not* offloaded and that S3's per-integration budget is its only bound — now corrected to
  name all four endpoints and to state that both bounds apply to single gain and fire
  independently. Note `openapi.yaml`'s `/api/v1/gain` `504` block already listed
  `request_timeout`; that claim was aspirational before this unit and is now true.
- **Unchanged, deliberately:** the poem-layer timeout still does not *cancel* the rayon work
  (S2's standing caveat) — it bounds the client's wait, not the CPU. `/api/v1/gain` remains
  **not** admission-limited (`routes.rs`), per the unit's gotcha.

*Original unit follows.*

- **Entrance / read first:** `api/handlers.rs:222` — `compute_gain` calls
  `compute_gain_from_request_with_budget` inline on the async task. Compare
  `compute_gain_batch` (`:332`), `generate_heatmap_endpoint` (`:484`), `h3_link_budget`
  (`:1027`), which all wrap their compute in `tokio::task::spawn_blocking`. `RequestTimeout`
  (`api/middleware.rs`) is a `tokio::time::timeout` around the endpoint future, so a future
  that never yields is never preempted.
- **Exit criteria:**
  1. `compute_gain` runs its compute under `spawn_blocking`, matching the other three
     handlers (including their `JoinError` handling).
  2. A test with a sub-second `create_routes_with_timeout` proves a slow single-gain request
     returns **504**, not a late 200.
  3. `docs/api-documentation.md` states which bound applies to single gain — today S3's
     `performance.integration_budget_ms` is the *only* live bound on that route and that is
     documented nowhere.
- **Gotchas:** the poem-layer timeout still does not *cancel* the rayon work (S2's standing
  caveat, and see the oversubscription note on `ConcurrencyLimit`); it only stops the client
  from waiting. Don't overclaim in the docs. `/api/v1/gain` is deliberately **not**
  admission-limited (`routes.rs:110-111`) — leave that as is; this unit is about
  preemptability, not admission.
- **Depends on:** S2.

---

## Phase 3 — API contract quality

Sequencing: **C3 → C4 → C2** share the handler error paths — land in that order, then
**C9** (the one value-changing break), then **C8** (the consolidated breaking pass), then
**C7** (drift guard) freezes the result. C1, C10 and C11 ran in parallel with C3–C2 and are
done. C5 and C6 are superseded by C8.

**Why C9 sits outside C8** even though both are breaking: C8's charter forbids altering any
computed value, and its review net is exactly that — every existing numeric assertion must
still hold, so a reviewer can confirm "no number moved" across a large rename/reshape diff.
C9 moves numbers by design. Folding it in would destroy the one property that makes a
four-stage breaking pass safe to review. It lands immediately *before* C8 instead, which
still satisfies "break once, then freeze" — the freeze point is C7, and nothing consumes
the API yet.

### C1 — Document `/api/v1/h3-heatmap` — Effort: S/M
**[✅ DONE 2026-07-25 — branch `feat/c3-error-code-vocabulary`]**

Delivered: an `/api/v1/h3-heatmap` path entry in `openapi.yaml` wired to the previously
orphaned `H3LinkBudgetRequest`/`H3LinkBudgetResponse` schemas, documenting **current**
behavior including the C2 status codes (400/404/422) and the S1b/S2/S3/S4 transport codes
(413/504/503) it inherits from the shared middleware; an "H3 Link Budget Grid" section in
`docs/api-documentation.md` plus a worked cURL example and full response; and
`examples/requests/h3_link_budget_request.json`, mapped into G3's
`every_example_request_deserializes` (whose unmapped-file arm panics, so the example
cannot drift out of coverage silently).

The example is **verified, not illustrative**: it uses `gs_3.7m_uncalibrated` /
`s_band_feed` from the shipped `calibration_data/antennas.yaml`, and the documented
response was captured from a cold-cache local service running that exact body (only
`computation_time_ms` varies between runs). The error-body examples in the spec were
likewise captured from live 400/404/422 responses rather than written from the source —
the first draft's invented `n_rings` message did not match what the validator emits.

Three things found while writing it, all documented rather than changed (C1 is a
documentation unit; each is someone else's to decide):

1. **`loss_db` is referenced to the grid *centre cell*, not the beam peak**, so cells
   nearer the peak carry a **negative** `loss_db`, and `total_path_loss_db`
   (`= fspl + loss_db`) can fall below the free-space value. This is what the code says
   and does; it is a genuine trap for a client, so it now has an explicit paragraph in
   both the spec and the docs, and the worked example was deliberately chosen to *show* a
   negative value rather than hide it. **Superseded 2026-07-25 by unit C9** (maintainer
   decided the same day to reference `loss_db` to the grid peak, matching `/heatmap`): the
   paragraphs and the worked example written here are C9's to rewrite, and they were
   written knowing that — documenting the behavior accurately is what made the decision
   possible.
2. **`/h3-heatmap` drops model-produced warnings on a gain-cache hit.** A repeat of an
   identical request returns a shorter `warnings` array than the first call (measured: the
   spillover warning present on call 1, absent on call 2). This is deliberate and
   commented in `h3_link_budget.rs:145-146,204-209`, and it is why P3 had to re-emit
   `RAY_TRACING_STUB_WARNING` outside the cache closure — geometry-class warnings (P3, P8,
   P10-tail) are emitted outside and are unaffected; model-class warnings are not.
   `/gain` and `/heatmap` were checked and do **not** behave this way. **Superseded
   2026-07-25 by unit C10, which fixed it the same day** — the "defer to C8 stage 3" call
   recorded here was wrong: the swallowed set includes the P10 non-convergence warning, so
   this was an honesty defect against a stated invariant, not a contract-polish question.
   The paragraph C1 added to `api-documentation.md` describing the behavior was removed
   when C10 landed.
3. **`docs/api-documentation.md` still carried the `{"w":…}` object form of
   `vehicle_attitude`** in its cURL and JavaScript examples — the exact deserialization
   break G3 fixed in `examples/requests/`, surviving in the prose the whole time (the
   schema requires `[w,x,y,z]`). Both corrected; they are not covered by G3's test, which
   only reads `examples/requests/*.json`. **The coverage gap behind it is closed by unit
   C11** (2026-07-25), which puts the prose examples under a schema guard.

Not done here, deliberately: the `cells` array's per-cell shape had no schema at all
(`items: {type: object}`), which would have left the endpoint documented in name only, so
an `H3CellResult` component was added and wired up — the one addition beyond a pure path
entry. C8 will revisit this entry for the `feed_position` rename, required
`coordinate_system`, and typed warnings; C7 then freezes it.

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
**[✅ DONE 2026-07-24 — branch `feat/c3-error-code-vocabulary`]**

Delivered: all seven PascalCase `ErrorResponse` constructors deleted (grep-confirmed zero
callers — they only ever appeared in their own definitions and unit tests, so the
PascalCase codes never reached the wire); `api::schemas::error_codes` added as the single
source of truth for the **11** live codes, with every emission site in `handlers.rs` and
`middleware.rs` referencing the constants instead of a string literal, so a typo is now a
compile error. Docs re-trued: `openapi.yaml`'s `ErrorResponse` schema gained the code
`enum` and the missing `field` property (and `details` was corrected from `object` to
`string` — the Rust type is `Option<String>`, so the old spec could not describe any real
body), `docs/api-documentation.md` gained the error-code table, and
`docs/architecture.md` §6.3 lost a fabricated variant-per-error `enum ApiError` with
bespoke per-error fields that was never built. `examples/api_requests.json` advertised two
codes the service cannot emit (`InvalidAttitude`, `CoordinateTransformError`); corrected to
`validation_error` and `internal_error`. Drift guard: `tests/error_code_vocabulary.rs`
(3 tests) pins the spec enum and the docs table against `error_codes::ALL` and fails on any
PascalCase code reappearing in the published contract — each verified to fail on injected
drift before being accepted.

Deliberately **not** done here (C2 owns it): the spec still files `validation_error` under
its current `400` response block, and the status inconsistencies are documented as a
caveat rather than fixed.

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
**[✅ DONE 2026-07-24 — branch `feat/c3-error-code-vocabulary`]**

Delivered: `api::error_response::json_error` is now the only place in the crate that turns
an `ErrorResponse` into a `poem::Error`, and all **19** production sites (16 in
`handlers.rs`, 3 in `middleware.rs`) go through it. Bodies are byte-identical — pinned by
`assert_json_error`, which re-serializes the parsed body and asserts equality with the raw
bytes.

**The diagnosis in this unit was optimistic.** `poem::Error::from_string` does not serve
`text/plain`; it sets **no `Content-Type` header at all**. Verified by reverting the helper
and watching all four original tests report `got "<missing>"`.

Two things found while executing, both fixed here:

1. **One error site carried no code at all.** The antenna-details handler's
   feed-lookup failure used `from_string` with a bare `format!` string, so that 404 had no
   `error` field — it was not an `ErrorResponse` in any sense. Now emits `feed_not_found`.
2. **Malformed request bodies never reached our error path.** poem's `Json` extractor
   rejects an unparseable body before the handler runs, returning its own `400` +
   `text/plain` + no code. `ErrorHandler` now normalizes that one case to
   `invalid_request_body`, preserving poem's parse-location message. The rewrite is gated
   on `Error::is_from_response()` (false only for framework-generated errors, since every
   error of ours is built from a response) **and** `status == 400`, so a handler's own 400
   is never touched — pinned by `normalization_does_not_rewrite_our_own_400`.

**Known remaining exception, deliberately left for C2/C8:** routing-level rejections poem
raises on its own — `404` for an unrouted path, `405`, `415` — are still framework-shaped
`text/plain`. Normalizing them requires error codes that do not exist in the vocabulary
(there is no `route_not_found` / `method_not_allowed` / `unsupported_media_type`), and
minting them is a contract decision, not a transport fix. Consequence worth knowing: a
`404` can arrive in either shape today. Documented in `api-documentation.md`.

Tests: `tests/integration/error_content_type_tests.rs` (6 tests) covers the four distinct
construction paths — handler pre-check (422), handler-maps-service-error (400), GET handler
(404), middleware (413) — plus the framework normalization and its guard. All four original
tests were verified to fail against the pre-C4 helper.

- **Entrance / read first:** the `poem::Error::from_string(serde_json::to_string(…))`
  pattern (e.g. `handlers.rs:203-206`) — poem serves these as `text/plain`. **Find all
  sites by grep**, not just the cited one.
- **Exit criteria:** one shared error-response helper replaces the ad-hoc pattern at all
  sites; all error responses carry `Content-Type: application/json`; **body bytes
  unchanged** — an integration test asserts both the header and the body on a 422 and a 400.
- **Depends on:** C3.

### C2 `[DECISION]` — Unify validation status codes — Effort: M
**[DECIDED 2026-07-24 — all three calls at the recommended option; see the register row for the full rationale]**

**[✅ DONE 2026-07-24 — branch `feat/c3-error-code-vocabulary`]**

Shipped as decided, all three calls. What is worth knowing beyond the decision record:

- **The fix is structural, not four edits.** `api::error_response::validation_status` and
  `service_status` are now the only two functions that pick a status; the four handlers
  call them. The bug the unit describes — `/gain`'s missing `Validation(_)` arm — was a
  symptom of four hand-maintained `match` blocks over the same error type, so patching the
  arm would have left the cause in place. Adding an error variant is now one edit.
- **Shape choice for call (B): the *first* option, not the second.** The unit suggested the
  validators-stop-checking-existence option as "closer to how `/h3-heatmap` works today"
  and the smaller diff. It is the wrong fit once call (C) is in scope: batch pre-validation
  has to classify per-item existence failures, and moving lookups into handlers would make
  the batch handler re-implement 1000 lookups the validator already does. Existence
  failures instead left the validation *class*, as
  `ValidationError::{AntennaNotFound, FeedNotFound}`. `/h3-heatmap` moved the *other* way —
  it now calls the shared `validate_antenna_feed_exists` instead of hand-building its own
  404, which is what made it disagree with its siblings in the first place.
- **Batch needed a new error type.** `BatchValidationError` carries the item index *and*
  preserves the inner failure class. The pre-C2 validator flattened item errors into
  `InvalidValue { param: "evaluations[i]" }` — index kept, class erased, so every item
  failure would have been a 422 including absent antennas. Index reaches the wire as
  `field: "evaluations[3]"` and is also prefixed onto `message`.
- **Defect (1) was latent, not reachable.** Nothing in the single-gain evaluator raises
  `AntennaModelError::Validation(_)` — the geometry builders return `ValidationError`, but
  the evaluator wraps every one as `Generic` (correctly: bad calibration data is a
  server-side fault). So the 500 fallthrough could not be triggered over HTTP, and no
  integration test can fail-before-fix on it. It is covered instead by
  `service_status_policy_table`, a unit test enumerating every rejection class with its
  required (status, code) — which does fail when the arm is removed.
- **Two behavior changes beyond the three calls**, both intentional:
  1. An **empty** batch was 200-with-zero-results (indistinguishable from a successful
     no-op) and is now 422, like any other batch-level constraint violation.
  2. A **non-finite** number in a request is a **400**, not a 422. JSON has no `NaN` or
     `Infinity` literal; serializers emit `null`, which will not deserialize into a
     declared `f64`, so the body really is unparseable. Consequence worth recording: the
     validator's own non-finite checks are **unreachable over HTTP** and guard only the
     in-process service API. Pinned by `non_finite_request_value_is_a_parse_failure`.
- **The service now emits no 400 from any typed error.** `the_service_never_answers_400_from_a_typed_error`
  asserts it. That removed the only endpoint capable of producing the input for C4's
  `normalization_does_not_rewrite_our_own_400` guard, which moved from an integration test
  to `middleware::tests::error_handler_does_not_rewrite_our_own_400` with a synthetic
  endpoint — better-scoped anyway, since the invariant is about error construction rather
  than any endpoint. Fixing that test's middleware ordering (`ErrorHandler` must be applied
  *before* `RequestId` to sit inside it) also revealed that the pre-existing
  `test_error_handler` had the same inversion and was vacuous; corrected.
- **Verification.** All 7 pre-fix failures in the new matrix were observed before any
  implementation. Afterwards, three regressions were injected and confirmed caught:
  `InvalidCoordinate → 400` (3 tests), `AntennaNotFound → 422` (3 tests), and removing the
  batch pre-check (5 tests). C3's fix also missed two `"feed_not_found"` / `"antenna_not_found"`
  string literals in `get_feed_details`; converted to constants here.
- **Scope limitation.** The exit criteria call for openapi.yaml updated "for every
  endpoint". `/api/v1/h3-heatmap` **is not in openapi.yaml at all** — adding it is unit
  **C1**. The three documented compute endpoints gained `422` and `404` blocks with examples
  and had their `400` descriptions corrected; `ErrorResponse.error`'s status caveat was
  replaced with the settled rule. api-documentation.md and architecture.md §6.3 likewise.
- **Untouched, as scoped:** the per-item batch response *shape* (`gain_db: null` + a
  warning string for compute-class failures) was C8 stage 3's — ✅ landed 2026-07-27 as a
  typed `GainResponse.error`. Routing-level 404/405/415
  remain framework-shaped `text/plain` — that needs new error codes, which is C8's call, so
  the "a 404 can arrive in either shape" caveat C4 documented still stands.

- **Question:** validation failures return 422 from the pre-check path but 400 when the
  same class of error surfaces from the service layer; batch differs again. Changing codes
  is a behavioral API change.

**Measured current state (verified against the code 2026-07-24 — the question above
understated it):**

| Case | `/gain` | `/gain/batch` | `/heatmap` | `/h3-heatmap` |
|---|---|---|---|---|
| Malformed / unreadable body | 400 | 400 | 400 | 400 |
| Well-formed, semantically invalid | 422 | **200**, `gain_db: null` | 422 | 422 |
| Unknown `antenna_id` / `feed_id` | 422 | **200** + null item | 422 | **404** |
| Batch-level (empty, >1000) | — | **400** | — | — |
| `Validation(_)` from the service layer | **500** ⚠️ | 400 | 400 | 400 |
| `InvalidCoordinate` from the service | 400 | per-item | 400 | 400 |

**Decided policy:**

- **(A) 400 = body that cannot be parsed; 422 = parses but is semantically invalid** — at
  *both* the pre-check and service layers, on all four compute endpoints. Service-layer
  `InvalidCoordinate` therefore moves 400 → **422**.
- **(B) Unknown `antenna_id`/`feed_id` referenced in a request body → 404 everywhere**,
  with the existing `antenna_not_found` / `feed_not_found` codes. Reserve 422 for
  parameters that are wrong rather than absent.
- **(C) `/gain/batch` pre-validates every item and rejects the whole batch 422**, naming
  the failing item index. Per-item degradation survives only for *compute*-class failures
  (time-budget exceeded, non-convergence).

**Two defects in scope, both found while scoping this unit:**

1. `/gain`'s service-error match (`handlers.rs:268-278`) has **no `Validation(_)` arm** —
   it falls through `_ => INTERNAL_SERVER_ERROR`. Any validation error the pre-check does
   not catch is served as a **500**.
2. The 404 arms on `/gain` and `/heatmap` are **dead code today**:
   `validate_gain_request` / `validate_heatmap_request` take the repository and reject
   unknown antenna/feed as `InvalidAntennaId` / `InvalidValue` (`validator.rs:432-451`)
   → 422, so `FeedNotFound → 404` is unreachable. `validate_h3_link_budget_request` does
   *not* take the repository, so `/h3-heatmap` reaches the lookup and returns a real 404.
   Same input, two answers. Call (B) makes all four agree on 404.

- **Exit criteria:** an integration test matrix (endpoint ×
  {malformed, semantically-invalid, unknown-antenna, unknown-feed}) **written first**,
  then codes fixed until it passes; the `/gain` 500-fallthrough covered by a test that
  fails before the fix; a batch test asserting 422-with-item-index (and that no response
  ever carries `gain_db: null` for a validation-class failure); openapi.yaml responses
  updated for every endpoint; api-documentation.md error section updated. Treat as
  bug-fix-grade in v1 (no client can have relied on the inconsistency), with a changelog
  note.
- **Implementation note for (B):** the existence failures must leave the validation error
  class so handlers can map them — `ValidationError::InvalidAntennaId` and the
  feed-not-found `InvalidValue` arm at `validator.rs:441-450` become a distinct
  not-found variant (or the validators stop performing existence checks and the handlers
  do the lookup, as `/h3-heatmap` already does). Pick one shape and apply it to all four
  endpoints; the second option is closer to how `/h3-heatmap` works today.
- **Gotchas:** grep handlers.rs for every `StatusCode` site (~20 including the
  non-validation ones); the matrix test is the net that catches a missed one. Don't touch
  error *bodies* here (C3/C4 own those). The per-item batch *shape* — an explicit
  `{code, message}` instead of `NaN` + a warning string (`batch.rs:199-227`) — is a
  response-shape break and belonged to **C8 stage 3** (✅ done 2026-07-27), not here; C2 only stops
  validation-class failures from reaching that path.
- **Depends on:** C3, C4 (land after, to avoid triple-editing the same lines).

### C5 — `/heatmap` H3 grid-type stub — **SUPERSEDED by C8 (decided 2026-07-08)**

The variant removal happens in C8 stage 4 alongside the rest of the endpoint-coherence
work. Do not implement standalone. (Full H3-into-`/heatmap` merge remains feature F5,
still gated.)

### C6 — `feed_position` naming trap — **SUPERSEDED by C8 (decided 2026-07-08)**

The docs-only design existed only under the no-breaking-changes assumption. With
pre-production confirmed, C8 stage 1 performs the actual rename to
`feed_pointing_location`. Do not implement the docs-only variant.

### C10 — `/h3-heatmap` loses warnings on a gain-cache hit — Effort: S
**[✅ DONE 2026-07-25 — branch `feat/c3-error-code-vocabulary`]**

**The defect.** `/api/v1/h3-heatmap` is the only endpoint that reads through
`GainCache::get_or_compute`, and the model's warnings were captured *inside* the
cache-MISS closure. A second identical request therefore came back with a shorter
`warnings` array than the first — measured on `test_simple`: the spillover and
feed-offset-band warnings present on call 1, gone on call 2. `/gain`, `/heatmap` and
`/batch` were checked and are unaffected; they do not cache.

**Why it outranked its size.** The swallowed set included
`INTEGRATION_NONCONVERGENCE_WARNING`, raised by the P10 self-check. CLAUDE.md states that
check is "surfaced as a response warning, never silent" — on a warm `/h3-heatmap` that was
false: a gain from a non-converged aperture integral was served with no warning at all.
That is the same honesty class as P8/P10/F7, not contract polish, which is why it was
fixed here rather than deferred into C8 stage 3 as first proposed.

**The fix, by warning class** — the classification is what makes it a fix rather than a
patch:

- *Computation-derived* (non-convergence): rides **with** the cached value. `GainCache`
  now stores `CachedGain { value, converged }` instead of a bare `f64`, and the warning is
  re-emitted from the flag on hit and miss alike. It cannot be re-derived any other way —
  a hit is precisely the path that skips the integration that would reveal it.
- *Configuration-derived* (spillover, feed-offset band): emitted **once per request** by
  `compute_h3_link_budget`. `analyze_edge_cases` takes `(θ, φ)` and ignores both
  (`_theta`, `_phi`), so these were identical at every cell anyway — deriving them once,
  outside the cache, is both cheaper and cache-independent.
- *Geometry-derived* (off-axis, rear-hemisphere, ray-tracing stub): already re-emitted
  outside the closure by P3/P8/P10-tail. Unchanged — C10 generalizes what P3 did for one
  warning to the rest.

Nothing is smuggled out of the closure any more, so the `Cell`-style capture hack is gone
and the "which warnings survive a hit?" question has a structural answer instead of a
per-warning one.

- **Tests:** `tests/integration/h3_link_budget_tests.rs::test_h3_warnings_stable_across_cache_hits`
  (cold vs warm warning sets must be equal, with a non-empty guard so it cannot pass
  vacuously) — verified to fail before the fix, naming both dropped warnings. Plus
  `service::cache::tests::test_nonconvergence_flag_survives_cache_hit`, which asserts the
  flag specifically: checking the hit's `value` alone would pass even with the flag
  dropped, which is exactly how the original bug survived.
- **Known gap:** no test drives a *genuinely* non-converged integration end-to-end over
  HTTP — the flag's transport is pinned at the cache layer and the emission is a single
  unconditional branch off it, but the two are not covered by one test. Triggering real
  non-convergence needs a pathological geometry; not worth the runtime here.

### C11 — Drift guard for API examples embedded in prose — Effort: S
**[✅ DONE 2026-07-25 — branch `feat/c3-error-code-vocabulary`]**

The prose sibling of **C7**: C7 will guard `openapi.yaml`'s paths against the route table;
C11 guards `docs/api-documentation.md`'s worked examples against the request/response
schemas. Filed and landed after C1 found that the cURL and JavaScript examples still
carried the `{"w": …}` object form of `vehicle_attitude` — the *exact* deserialization
break G3 fixed in `examples/requests/` — because G3's test only ever read
`examples/requests/*.json`.

Landed **before** C8 on purpose, by the same argument as roadmap principle 2 (guardrails
first): C8 rewrites every one of these blocks across four stages, so a guard that exists
beforehand catches C8's misses, while one added afterwards only ratifies whatever survived.

- **Delivered:** `antenna-model/tests/doc_examples_deserialize.rs` (2 tests).
  `every_documented_example_deserializes` reads each block marked
  `<!-- api-example: TypeName -->` (an HTML comment, invisible in rendered Markdown) and
  deserializes it into that schema; ten blocks are marked, requests and responses alike.
  Payload extraction is per fence language: `json` whole-block, `bash` from the cURL
  `-d '…'` argument, `javascript` from the `JSON.stringify(…)` argument.
- **`every_api_example_block_is_marked`** is the half that matters: an *unmarked* block
  that looks like an API payload is a hard failure, so a future example cannot quietly sit
  outside coverage — which is precisely how the original break survived.
- **One doc change to enable it:** the JavaScript example's object literal was rewritten
  with quoted keys. Still valid JavaScript, now also valid JSON, so the example a reader
  copies is the one the test checks.
- **Scope:** `docs/api-documentation.md` only, via the `CONTRACT_DOCS` const. The design
  and workflow docs contain JSON blocks too (28 unmarked blocks across `architecture.md`,
  `partial-calibration-design.md`, `calibration-workflow-guide.md`,
  `partial-calibration-setup-summary.md`, `kubernetes-deployment.md`) but they are
  illustrative and knowingly aspirational in places — auditing them is **D5**'s job. Adding
  a file to `CONTRACT_DOCS` is one line, so D5 should ratchet them in as it makes each true.
- **Verified by injection:** re-introducing the historical `{"w": …}` form fails
  `every_documented_example_deserializes` at the exact line; adding an unmarked
  `antenna_id` block fails `every_api_example_block_is_marked`. Both were confirmed to fail
  before being accepted.

### C9 `[DECISION]` — `/h3-heatmap` `loss_db`: reference it to the grid peak, not the centre cell — Effort: S
**[DECIDED 2026-07-25 — maintainer: reference `loss_db` to the beam peak; peak = max gain over
the cells actually evaluated, matching `/heatmap`]**
**[✅ DONE 2026-07-26 — branch `feat/c9-h3-loss-peak-referenced`]**

**As landed.** Step 6 (the separate boresight-reference evaluation) is gone, so the endpoint
runs one *fewer* gain evaluation per request. `compute_cell_result` now returns a private
`CellGain` — the peak-independent quantities only — and `H3CellResult` is constructed solely
in the caller's second pass, so a cell cannot escape with an unfilled loss (the two-pass
shape is enforced by the type, not by a comment). Measured on the shipped
`gs_3.7m_uncalibrated` example the numbers moved exactly as predicted: centre cell
`loss_db 0.0 → 5.43`, the neighbour `−5.43 → 0.0`, and its `total_path_loss_db`
`126.45 → 131.88` (now equal to its FSPL, since it is the peak).

**Degenerate case (the gotcha, decided here).** With step 6 removed there is no fallback
reference when *no* cell yields a finite gain. Reusing `/heatmap`'s convention rather than
inventing a second one: `FAILED_POINT_LOSS_DB` became `pub(crate)` and a negated
counterpart `NO_PEAK_GAIN_DB = -999_999.0` was added beside it, reported as
`metadata.peak_gain_db` when there is no peak. A finite value is required — `f64::NEG_INFINITY`
serializes to JSON `null` for a field the schema declares as a number, which is the
null-under-200 hazard C2 called out. Pinned by
`h3_all_cells_failed_reports_a_finite_peak_sentinel` (forces total failure with a zero S3
integration budget).

**One change beyond the unit's H3 scope, deliberate:** `/heatmap` had the identical
`peak_gain_db = -inf → null` hole in its own all-points-failed path. It now reports the same
sentinel. Two lines, and leaving it would have meant the shared convention disagreed with
itself on the very endpoint C9 aligns to.

**Tests added:** `h3_loss_is_referenced_to_the_grid_peak_not_the_centre_cell` (unit — a
0.06·f lateral design-feed displacement steers the peak off the centre cell, asserted
non-vacuously, so it fails outright on the pre-C9 rule),
`h3_all_cells_failed_reports_a_finite_peak_sentinel` (unit),
`test_h3_peak_cell_is_the_zero_loss_reference` (integration — replaces
`test_h3_center_cell_minimum_loss`), and
`test_heatmap_and_h3_heatmap_reference_loss_by_the_same_rule` (integration — the drift guard:
on both endpoints the minimum loss over the grid is exactly 0.0 and no value is negative).

**The defect.** `/api/v1/h3-heatmap` returns `loss_db = gain(centre cell) − gain(this cell)`
(`h3_link_budget.rs:648`, reference built at `h3_link_budget.rs:395-471`). The centre cell is
merely the cell nearest `feed_position`; it is not the beam peak, and with any feed offset the
beam is steered away from it. Consequences on the live path:

- **`loss_db` goes negative** for every cell stronger than the centre cell. Measured on the
  shipped `gs_3.7m_uncalibrated` / `s_band_feed` at 2200 MHz, `n_rings: 2`, resolution 7: the
  centre cell reports `loss_db: 0.0` at 29.18 dB gain while a neighbour reports
  **`loss_db: −5.43`** at 34.61 dB — the grid peak is not the grid centre.
- **`total_path_loss_db` (`= fspl + loss_db`) inherits it** and can fall *below* the
  free-space path loss, which reads as a link that beats free space.
- **The two heatmap endpoints disagree.** `/api/v1/heatmap` already uses
  `peak_gain_db − gain` with `peak_gain_db = max` over successful grid points
  (`heatmap.rs:112-130`). Same field name, same concept, two different references.

**The decision.** Peak = **max gain over the cells actually evaluated** — i.e.
`loss_db = metadata.peak_gain_db − gain_db`, the rule `/heatmap` already applies. The
alternative (numerically maximizing gain over direction to find the *true* beam peak,
independent of the grid) was considered and rejected for this unit: it needs a robust 2D
maximizer over a pattern whose peak is steered off-axis by the feed offset, costs extra
integrations per request, and would force a matching change to `/heatmap`. It stays available
as a later feature if a consumer ever needs cross-request comparability.

- **Entrance / read first:** `h3_link_budget.rs:395-471` (step 6, the boresight reference),
  `:648` (the subtraction), `:546-556` (`peak_gain_db`, already max-over-cells — so the new
  reference is a value the response already reports), and `heatmap.rs:112-130` for the rule
  being adopted.
- **Work:**
  1. Delete step 6 and the `boresight_gain_db` parameter threaded through
     `compute_cell_result`. This *removes* one gain evaluation per request — the unit is a net
     performance win, not a cost.
  2. Restructure to `/heatmap`'s two-pass shape: compute every cell's gain, take the peak,
     then fill `loss_db` and `total_path_loss_db` in a second pass. The parallel cell loop
     currently builds a complete `H3CellResult` including `loss_db`, so the loss fields become
     post-fill.
  3. Update the `H3CellResult.loss_db` docstring (`schemas.rs:658-664`) — it is currently an
     accurate description of the behavior being removed.
- **Exit criteria:**
  - `loss_db ≥ 0` for every cell, `== 0.0` at exactly the peak cell, on every endpoint test.
  - `total_path_loss_db ≥ free_space_path_loss_db` for every cell (the property that fails
    today) — assert it directly; it is the client-visible symptom.
  - `loss_db == metadata.peak_gain_db − gain_db` for every cell, pinned by a test, so the
    response is internally consistent and a reader can re-derive it.
  - A test asserting `/heatmap` and `/h3-heatmap` compute loss by the same rule, so they cannot
    drift apart again.
  - openapi `H3CellResult.loss_db` / `total_path_loss_db` descriptions and the
    `docs/api-documentation.md` "H3 Link Budget Grid" section rewritten — **both currently
    document the centre-cell reference at length, including a worked example chosen to
    display a negative `loss_db`** (added by C1, 2026-07-25). The example response's
    `loss_db` / `total_path_loss_db` values must be recaptured from a live run, not edited by
    hand.
- **Gotchas:**
  - **`tests/integration/h3_link_budget_tests.rs:108-158` pins the old semantics** ("Center
    cell has loss_db ≈ 0.0 and is the minimum-loss cell") and must be rewritten to assert the
    *peak* cell is the zero, not deleted. Its own comment already concedes that "coma lobes
    can produce negative values" — the trap was known at the time and encoded as expected
    behavior.
  - **All-cells-failed fallback.** `peak_gain_db` currently falls back to `boresight_gain_db`
    when no cell yields a finite gain (`:552-556`); that fallback disappears with step 6.
    Decide and test the degenerate case explicitly — `/heatmap`'s answer is a
    `FAILED_POINT_LOSS_DB` sentinel per failed point (`heatmap.rs:120-130`); reuse it rather
    than inventing a second convention.
  - **Mixed correction basis.** Step 6 deliberately applied the correction surface to the
    reference so both sides of the subtraction shared a basis. With a grid peak the reference
    *is* one of the cells, so the basis is consistent by construction — but cells outside
    calibration coverage are uncorrected while in-coverage cells are corrected, so a grid can
    still straddle two bases. `/heatmap` already has this property; note it, do not solve it
    here.
  - Standing rule 4: mirror the schema-description changes into `openapi.yaml` by hand (C7's
    guard does not exist yet).
- **Depends on:** C2 (the shared status/error work in the same handlers). **Blocks:** C8 — land
  C9 first so C8 stage 4 documents the settled semantics once instead of rewriting the C1 entry
  twice.
- **Out of scope:** the true-beam-peak search (see the decision above); any change to
  `/api/v1/gain`'s `loss_db`, which is a third and *deliberately* different quantity
  (`reference_gain_db − gain_db`, ideal-aperture referenced, `evaluator.rs:336-411`) and is not
  a grid quantity at all.

### C12 — `CalibrationInfo.rmse_db` / `r_squared`: documented as omitted, emitted as `null` — Effort: S
**[✅ DONE 2026-07-28 — landed alongside C8 stage 4, branch `feat/c8-stage4-endpoint-coherence`, commits `371f2d3` + `2bd734b`]**

**The defect.** `antenna-model/src/api/schemas.rs:846,850` document these two fields as
*"(None for uncalibrated antennas)"*, and both carry
`#[serde(skip_serializing_if = "Option::is_none")]` (`:847,851`) — which reads as "omitted
from the response body for uncalibrated antennas". **The code cannot do that.** Three
places have to line up and none of them do:

- `antenna-model/src/data/types.rs:148,151` types the underlying `CalibrationMetadata`
  fields as plain `f64`, so `None` is not even representable upstream.
- `antenna-model/src/data/repository.rs:259-260` fills them with `f64::NAN` for design-spec
  (uncalibrated) antennas — the sentinel the `Option` was supposed to be.
- `antenna-model/src/api/handlers.rs:701-702` — the only `CalibrationInfo` construction in
  the repo — wraps them in `Some(...)` unconditionally.

The `skip_serializing_if` attribute therefore never fires, and `GET /api/v1/antennas/{id}`
emits `"rmse_db": null, "r_squared": null` on every uncalibrated antenna. This is the same
hazard class C2 called out for `/gain/batch` (a JSON `null` under HTTP 200 for a field the
schema declares as a number), reached by a different route: `f64::NAN` has no JSON
encoding, so `serde_json` writes `null`.

**Why it survived.** Nothing pins the *uncalibrated* response shape.
`antenna-model/src/api/routes.rs:867` asserts `rmse_db` only on the calibrated path, and all
four `.bin` antennas are `enabled: false` (unit **D9**), so the only shape actually served
is the one nothing tests.

**Options** (a contract call either way — this is why it is a unit and not a patch):
1. **Map `NaN` → `None` in `handlers.rs`.** The attribute starts firing, the field is
   genuinely absent for uncalibrated antennas, and the existing doc comments become true.
   Cheapest, and it makes the type's intent real; `num_measurements: 0` already carries the
   "no measurements" signal for a client that needs to branch.
2. **Delete the two doc comments and the two `skip_serializing_if` attributes**, and declare
   `"rmse_db": null` the contract for uncalibrated antennas. Also defensible — but then the
   `null`-under-200 shape must be documented in openapi + `docs/api-documentation.md`
   deliberately, not by omission.

Either way, add a test that asserts the *uncalibrated* body (the missing coverage above),
and update `examples/responses/antenna_details_response.json`, whose current `null`s are
correct today and would become wrong under option 1.

**Why it was not fixed in C8 stage 1.** Both options change the response shape, and stage 1's
charter is a pure rename that moves no value and no shape. Filed per standing rule 5. Natural
home is **C8 stage 4** (spec completeness) or a standalone unit before **C7** freezes the
contract; the decision should be recorded in the register first.

**Found** 2026-07-26 during C8 stage 1 Task 2. The new `examples/responses/` drift guard
initially misread the *correct* `null`s in `antenna_details_response.json` as stale keys and
deleted them; that was caught in review and reverted, and the guard now exempts null-valued
sources for exactly this reason.

**Resolved: option 2 (declare the `null`), 2026-07-28.** `rmse_db`/`r_squared` became plain
`f64` with `#[serde(with = "nan_as_null")]`, dropping the `Option` + `skip_serializing_if`
pair — **the wire output is unchanged** (`null` before, `null` after); only the type and the
doc comments (plus this file, `docs/domain-contract.md`, and `docs/api-documentation.md`) now
say so. A test asserting the uncalibrated body has both keys present and JSON `Value::Null`
closes the missing-coverage gap noted above. `examples/responses/antenna_details_response.json`
was **not** modified — its `null`s were already correct, which is exactly what motivated
option 2 over option 1. See `docs/domain-contract.md`, "Resolved by design 2026-07-28 (C12)".

### C13 — `design_feed_offset_m`'s origin is producer-dependent (vertex vs focus) — Effort: S/M

**The defect.** The API documents this field as the feed's offset **from the focal point** —
`antenna-model/src/api/schemas.rs:803` (the `FeedInfo` docstring) and
`antenna-model/src/api/handlers.rs:834` (the handler docstring). One of the two producers of
the underlying artifact field disagrees:

- **Design-spec path (focus-relative — matches the doc).**
  `calibration_data/antennas.yaml` uses `position: [0.0, 0.0, 0.0]` to mean "feed at the
  focus" (`:118,163,209,265,295`), and `antenna-model/src/data/repository.rs:232-235` maps
  those three numbers into `FeedParameters.position` verbatim.
- **`calibrate` path (vertex-relative — contradicts the doc).**
  `calibrate/src/main.rs:696` writes `feed_position_m: (0.0, 0.0, focal_length_m)` under the
  comment *"On-axis configuration: feed at the focal point."* Same intent, different origin.

Consuming code assumes the design-spec convention. `antenna-model/src/service/evaluator.rs:170-174`
adds `design_pos` to a steering position that is **already vertex-origin** —
`compute_feed_position_from_pointing` → `to_feed_position_with_bdf`, which returns
`(dx, dy, focal_length + dz)` (`antenna-model/src/model/coordinates.rs:250`) — and the sum is
converted to focus-relative once, by the single `− focal_length_m` at `evaluator.rs:181`. A
`.bin`-calibrated antenna would therefore land at z ≈ 2f and report
`GeometryInfo.physical_feed_offset_m.z ≈ f` instead of ≈ 0: a focal-length-sized phantom
axial defocus on every request, plus a `FeedInfo.design_feed_offset_m` that means something
different depending on which tool produced the artifact.

**Latent only.** All four antennas that reference a `.bin` artifact are `enabled: false`, so
no served response takes the `calibrate` path today — cross-reference unit **D9**, which owns
the artifact-shipping story. D9 and this unit must land together or D9 ships the bug.

**Options:**
1. **Make `calibrate` focus-relative** (`(0.0, 0.0, 0.0)` for an on-axis feed), matching
   `antennas.yaml`, the API docstrings, and `evaluator.rs`'s arithmetic. One line plus its
   comment; nothing in the service moves.
2. **Make the artifact field vertex-relative** and subtract `f` at the design-spec loader
   instead. More churn, and it puts the frame conversion in the loader rather than at the one
   place that already does it.

Whichever is chosen, state the origin in the field's own doc (`data/types.rs`) so a third
producer cannot guess wrong, and add a test that an on-axis feed from *each* producer yields
`physical_feed_offset_m ≈ (0,0,0)` at zero steering — the assertion that would have caught this.

**Why it was not fixed in C8 stage 1.** Fixing it moves a computed value
(`physical_feed_offset_m`, and the gain that depends on the resulting aperture phase), which
stage 1's charter explicitly forbids: "no number moved" is the property that makes the rename
pass reviewable.

**Found** 2026-07-26 by the C8 stage 1 Task 4 review, while confirming that the renamed
response fields describe what the code actually computes.

### C14 — `openapi.yaml`'s feed-listing surface disagrees with the service — Effort: S
**[SUPERSEDED 2026-07-28 by C7 — see the C8 stage 4 "as landed" note above and C7's
post-generation acceptance checklist below]**

**The defect.** Two adjacent, pre-existing mismatches on the feed endpoints. Both predate C8;
stage 1 corrected only the one field it renamed.

**(a) The `FeedInfo` component (`openapi.yaml:1751`) declares six properties; two are right.**
Against the Rust type at `antenna-model/src/api/schemas.rs:799-816`:

| openapi declares | service emits | verdict |
|---|---|---|
| `feed_id` | `id` | wrong name |
| `name` | — | no emitter produces it |
| `design_feed_offset_m` | `design_feed_offset_m` | ✅ (renamed by C8 stage 1) |
| `q_factor` | `q_factor` | ✅ |
| `phase_center_offset_m` | — | no emitter produces it |
| `frequency_range` | `frequency_range_mhz` | wrong name only — the array-of-2 shape is correct |

Both constructions of `FeedInfo` in the repo (`antenna-model/src/api/handlers.rs:801-809` for
the list endpoint, `:863-871` for the detail endpoint) emit exactly the four Rust fields, so
a client coding to the spec would look up `feed_id` and `frequency_range` and find neither.

**(b) The list-feeds 200 wrapper (`openapi.yaml:974-981`) declares an `antenna_id` that is
never sent.** The spec says the body is `{antenna_id, feeds}`; the handler returns
`Ok(Json(json!({ "feeds": feeds })))` (`antenna-model/src/api/handlers.rs:819`) — `feeds` only.

**What stage 1 left behind, and why it is not enough.** Stage 1 renamed
`position_offset` → `design_feed_offset_m` in the component and left an in-file `# DRIFT`
comment on the `FeedInfo` schema (`openapi.yaml:1753-1758`) flagging the rest for the next
editor. That comment is a stopgap with two known holes: **every YAML parser strips it**, so it
is invisible to Swagger UI, codegen, and C7's future guard alike; and it documents only (a),
not the (b) wrapper.

**Options:**
1. **Fix as part of C8 stage 4** (spec completeness — "openapi.yaml describes every registered
   route with post-C8 schemas"). This is squarely stage 4's job, and it removes the `# DRIFT`
   comment rather than leaving a stripped-at-parse-time note in the shipped spec.
2. **Let C7's drift guard force it.** C7 as scoped asserts the *path+method* set, which would
   catch neither (a) nor (b); its optional stretch goal (validating example files against the
   component schemas) would catch both. If (1) is skipped, promote that stretch goal to
   required, or these mismatches survive the freeze.

Option 1 is preferred: C7's purpose is to freeze a correct contract, not to discover an
incorrect one.

**Why it was not fixed in C8 stage 1.** Stage 1's charter is the three-field rename; correcting
`feed_id`/`frequency_range`/`name`/`phase_center_offset_m` and the response wrapper is a
different (and larger) contract-truthfulness change. Standing rule 4 required mirroring the one
renamed field by hand, which is what stage 1 did.

**Found** 2026-07-26 while mirroring the C8 stage 1 renames into `openapi.yaml` (Task 4).

**Superseded 2026-07-28 (C8 stage 4 audit).** A full sweep of every schema behind
`/api/v1/antennas*` found five more components wrong beyond this unit's (a)/(b) —
`AntennaInfo`, `AntennaDetailsResponse`, `PhysicalParametersInfo`, `ValidityRangesInfo`, and
`CalibrationInfo` all disagree with their Rust types too, plus `GridData` (used by
`/heatmap`, outside the `/antennas*` surface but found in the same sweep) still describes
the pre-Task-1 flat, untagged shape. Hand-fixing eight defects into a spec that was about to
be regenerated wholesale was rejected as wasted, drift-prone work: **C7 is re-scoped to
auto-generate `openapi.yaml` from the Rust types via `utoipa`**, which fixes all eight by
construction instead of by a hand patch that can drift a third time. This unit's fix is
therefore **not implemented**; its two defects, and the six more found alongside them, are
preserved as C7's post-generation acceptance checklist (see the C7 section below).

### C15 — Client-visible surfaces that no drift guard covers — Effort: S/M

**The gap.** Four guards exist, and between them they cover less of the published contract than
their presence suggests:

| guard | covers |
|---|---|
| `antenna-model/tests/example_requests_deserialize.rs` (G3) | `examples/requests/*.json` |
| `antenna-model/tests/doc_examples_deserialize.rs` (C11) | marked blocks in `docs/api-documentation.md` **only** — `CONTRACT_DOCS` is a one-element array |
| `antenna-model/tests/example_responses_deserialize.rs` (C8 stage 1) | `examples/responses/*.json` |
| the compiler | Rust call sites |

Nothing covers: `openapi.yaml`; `examples/api_requests.json` (18 request/response examples);
`examples/postman_collection.json`; `examples/python_examples.py` (not even syntax-checked in
CI); `examples/QUICKSTART.md`, `examples/TESTING.md`, `examples/README*.md`; or any `docs/*.md`
other than `api-documentation.md`.

**This is not theoretical — C8 stage 1 produced the evidence three times.**
1. `docs/partial-calibration-design.md` carried a renamed field and appeared in **no** task
   inventory; the stage would have failed its own exit criterion had a reviewer not found it.
2. `openapi.yaml`'s `FeedInfo` spelled the field `position`, so the exit grep for
   `position_offset` **passed by luck** rather than by correctness (see C14).
3. `examples/api_requests.json` sat one directory away from the examples C8 stage 1 repaired and
   kept the identical drift — missing required `failed_points` / `failure_count`, and an
   undeclared `vehicle_attitude` on two `HeatmapRequest` bodies. Stage 1 fixed these on its way
   past, but only because a whole-branch review went looking; no test would have said a word.

**Options:**
1. **Extend the existing pattern to `examples/api_requests.json`** — cheapest high-value close.
   The file is `{examples: {<name>: {description, request|response}}}`, so it drops into the
   `example_responses_deserialize.rs` shape with a name→schema match arm and a panicking
   unmapped arm. This alone would have caught (3) automatically.
2. **Ratchet `CONTRACT_DOCS`** (`doc_examples_deserialize.rs`) to cover more of `docs/` as D5
   makes each file true — C11 already anticipated this; adding a file is one line.
3. **Promote C7's optional stretch goal** (validate example files against the openapi component
   schemas) to required, which is the only mechanism that would cover `openapi.yaml` itself.

**Sequencing.** This is C7's charter — the point of a freeze is that what is frozen is checked.
Doing (1) and (2) *before* C7 means the freeze ratifies verified surfaces rather than assumed
ones, which is the same argument that put C11 ahead of C8 and the response guard ahead of the
C8 stage 1 renames.

**Why it was not fixed in C8 stage 1.** Building new guards is not a field rename, and stage 1
deliberately kept its diff to "renames, no value moves" so that property stayed reviewable.

**Found** 2026-07-27 by the whole-branch review of C8 stage 1.

**Update 2026-07-28 (C8 stage 4).** C7's re-scope to `utoipa` generation closes the
`openapi.yaml` row of the table above at the source — a generated spec cannot drift from the
types it is generated from the way a hand-maintained one can. Stage 4 also **edited**
`examples/api_requests.json` (removing the two H3 examples Task 1's removal obsoleted), and
that file remains covered by no guard — option 1 above is now the largest concrete item left
in this unit's gap list, and the last content change to that file before C7's freeze.

### C8 — v1 contract finalization (the one sanctioned breaking pass) — Effort: L
**[DECIDED 2026-07-08 — pre-production confirmed: no consumers exist; break once now, then freeze]**
**[✅ ALL 4 STAGES DONE — stage 1 2026-07-26 (`feat/c8-stage1-aim-point-field-rename`),
stage 2 2026-07-27 (`feat/c8-stage2-required-coordinate-system`), stage 3 2026-07-27
(`feat/c8-stage3-typed-warnings`), stage 4 2026-07-28
(`feat/c8-stage4-endpoint-coherence`). C8 is closed; only **C7** remains before the
contract freezes.]**

**Stage 1, as landed.** The three field renames only — `feed_position` →
`feed_pointing_location` on all three request types, `GeometryInfo.feed_offset_meters` →
`physical_feed_offset_m`, and `FeedInfo.position_offset` → `design_feed_offset_m`, so that no
response field can be read as the aim point. Clean break, no serde aliases: a body using
`feed_position` is a 400 naming the new field, pinned by
`tests/integration/status_code_matrix_tests.rs::legacy_feed_position_key_is_rejected_with_400`.
**No computed value moved** — the numeric assertions across the workspace are unchanged, which
is the property that makes the pass reviewable. `docs/domain-contract.md`'s glossary row records
the old name and the rename date, and cross-references both renamed response fields; the frame
conversion at `evaluator.rs:181` is called out there as a conversion rather than a third term,
because the two-offset story invites exactly that misreading. Mirrored into `openapi.yaml`,
`examples/`, `docs/api-documentation.md` and CLAUDE.md. Three findings surfaced and were filed
rather than fixed, per standing rule 5 and stage 1's no-value-moves charter: **C12** (null vs
omitted `rmse_db`/`r_squared`), **C13** (`design_feed_offset_m` origin, vertex vs focus), and
**C14** (openapi feed-listing drift).

**Stage 2, as landed (2026-07-27).** `Position3D.coordinate_system` is a required field of
type `CoordinateSystem` (no `Option`, no `#[serde(default)]`); the `ECEF_THRESHOLD_M` constant,
the `coordinate_system()` heuristic method, and the whole `coordinate_ambiguity_warnings` /
`warn_if_ambiguous` path (with its `evaluator.rs` call site) are deleted. Per-frame range
validation is untouched — it now dispatches on the declared tag, so the same three numbers can
pass as ECEF and fail as geodetic (pinned). `Position3D::new` was replaced by
`Position3D::ecef(x,y,z)` and `Position3D::geodetic(lon,lat,alt)`: a constructor that silently
picks a frame is the same trap one layer down. An untagged position is a **400** naming
`coordinate_system`, pinned per position field × per endpoint by
`status_code_matrix_tests::a_position_without_coordinate_system_is_rejected_with_400`, with
`::geo_altitude_geodetic_emitter_is_accepted_when_tagged` as the acceptance half so the guard
cannot pass by rejecting everything. **No computed value moved** — all 606 unit tests' numeric
assertions are unchanged; the ~125 call-site conversions each took the frame the old heuristic
would have chosen.

**Finding, fixed in-unit:** tagging the published examples exposed that the heuristic was
*already* misreading them in production. The example literally named `ecef_coordinates` in
`openapi.yaml` — and its four siblings in `examples/api_requests.json`, plus the same family in
`docs/api-documentation.md`, `docs/architecture.md` and `docs/calibration-workflow-guide.md`,
25 positions in all — used Earth-surface ECEF values (~4.5 Mm) that sit *below* the 6400 km
boundary, so every one of them was being served as geodetic. Corrected to `ecef` while
tagging. This is a documentation fix, not a computed-value move: no test asserted on those
examples' numbers.

**Stage 3, as landed (2026-07-27).** `warnings` is `Vec<ApiWarning>` (`{code, message}`) on
all three response types, with a **closed** 14-variant `WarningCode` enum in a new
`warnings.rs` — a peer of `error.rs`, not an `api::` member, because the model layer
produces warnings and does not otherwise depend on the API layer. Every producer emits a
code; `service::heatmap`'s `w.contains("extrapolat") || w.contains("out of range")`
predicate (a substring test against prose owned by two other modules, whose second phrase
matched nothing any producer still emitted) became a code check. The two integration-test
"stable substring markers" became code constants; wording assertions that pin the *honesty*
of the P8/F7 messages were kept, moved to `.message`, and left with the unit tests that own
them. New `tests/warning_code_vocabulary.rs` mirrors `error_code_vocabulary.rs`: openapi's
`ApiWarning.code` enum and the api-documentation table must match `WarningCode::ALL`
exactly. **No computed value moved** (910 workspace tests, numeric assertions unchanged).

Stage 3 also discharged the per-item batch shape **C2 deferred into it**: a failed item
carries `error: {code, message}` instead of `gain_db: null` plus a `"Computation failed: …"`
string among its warnings. The code comes from `service_status` — the same vocabulary the
HTTP error bodies use — so an item that blew the integration budget reports
`computation_budget_exceeded` rather than a flattened generic failure.

Two of the unit's suggested code names had no producer and were **not** minted:
`spillover_applied` (P1 reports spillover as `metadata.spillover_loss_db`, a number) and
`higher_order_heuristic` (P2 removed the emitting mode on 2026-07-16). Full detail:
`docs/plan-c8-stage3-typed-warnings.md`.

**Finding, fixed in-unit:** `examples/api_requests.json` and `docs/architecture.md` both
documented a warning no producer has ever emitted —
`"Beam squint correction applied (pointing_freq != operating_freq)"` — alongside the
`geometry.beam_squint_deg` field that actually reports it. Removed rather than given a
code. Both files are in **C15**'s uncovered-surface inventory, which is why it survived.

**Stage 4, as landed (2026-07-28, branch `feat/c8-stage4-endpoint-coherence`).** Removed the
`H3` variants from `GridConfig`/`GridData` entirely — both stay single-variant **tagged**
enums (`grid_type: "rectangular"` only), keeping the tag on the wire so feature F5 can add a
variant back without a break; collapsing them into plain structs was considered and rejected
for exactly that reason. An `h3` grid_type is now an unknown serde variant, which the C2
status policy resolves to **400 `invalid_request_body`** — not 422, which is what the unit's
original framing loosely suggested ("unknown grid types become normal validation failures").
The request cannot be parsed into a known variant at all, so it is unparseable, not
semantically invalid; C2's 400-vs-422 boundary applies directly, and this was confirmed
rather than re-litigated. Retired the producerless `error_codes::NOT_IMPLEMENTED` /
`AntennaModelError::NotImplemented` — its only producer was the stub Task 1 removed, and a
code with no producer is exactly the defect class C3's drift guard exists to catch; the
vocabulary is **deleted to 10 codes, not reserved** for a future reintroduction.
`docs/domain-contract.md`, `docs/architecture.md`, `docs/api-documentation.md` and
`examples/api_requests.json` were mirrored (two H3 examples removed from the latter — an
edit C15 still has no guard over). Absorbs **C5** (superseded).

Also landed on this branch, ahead of the endpoint-coherence Task 6: **C12** was resolved in
favour of **emitting the `null`** — `CalibrationInfo.rmse_db`/`r_squared` became plain `f64`
with `#[serde(with = "nan_as_null")]` (the same convention `GainResponse.gain_db` already
used for a failed evaluation), replacing the `Option` + `skip_serializing_if` pair that
promised an omission the code never performed. The wire output did not change — `null`
before, `null` after; only the type and the docs now say so. See the C12 section below and
`docs/domain-contract.md`'s "Resolved by design 2026-07-28 (C12)" entry.

**`openapi.yaml` was deliberately NOT touched by stage 4.** An audit of every schema behind
`/api/v1/antennas*` (the planned scope of the openapi-reconciliation task) found **all seven
components wrong** — not just the two C14 filed — which reopened whether hand-fixing a spec
that had already drifted twice (once before C14, once again to produce this audit) was the
right mechanism at all. Decision: **C7 is re-scoped from "hand-maintained spec plus a
path+method drift guard" to "auto-generate `openapi.yaml` from the Rust types with
`utoipa`."** Hand-editing eight-plus defects into a spec C7 was about to regenerate wholesale
would have been wasted, drift-prone work with a third window for the same class of bug before
the freeze. The planned openapi-reconciliation work is therefore **superseded, not executed**
— see **C14** below. The audit table is preserved as **C7's post-generation acceptance
checklist** (see the C7 section).

The contract is **not yet frozen** — freeze happens when C7 lands.

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

**Stage 1 — Rename the aim-point fields. ✅ DONE 2026-07-26** (see the "as landed" note above).
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

**Stage 2 — Make `coordinate_system` required (remove auto-detection). ✅ DONE 2026-07-27** (see the "as landed" note above).
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

**Stage 3 — Typed warnings. ✅ DONE 2026-07-27** (see the "as landed" note above).
- `warnings: Vec<String>` → `Vec<ApiWarning> { code, message }` on all response types
  (currently at `schemas.rs:307,511,691`). Enumerate the code set from existing producers
  (grep `warnings.push` / warning constructors): expect at least `extrapolated`,
  `out_of_coverage`, `ray_trace_degraded`, `non_convergence`, plus the codes added by
  roadmap units P1 (`spillover_applied`), P2 (`higher_order_heuristic`), and P8
  (`off_axis_unvalidated`) — coordinate with those units if they land first (strings then;
  codes now).
- Exit: every producer emits a code + human message; the code enum documented in
  api-documentation.md + openapi; integration tests assert codes, not string matches.

**Stage 4 — Endpoint coherence + spec completeness. ✅ DONE 2026-07-28** (see the "as landed"
note above).
- Remove the `/heatmap` H3 grid-type stub variant (`heatmap.rs:168-171,215-218`); unknown
  grid types become normal validation failures (absorbs old C5). **Done** — `GridConfig`/
  `GridData` are single-variant tagged enums; an `h3` grid_type is a 400.
- `/h3-heatmap` fully documented (absorbs C1 if it hasn't landed; if C1 landed, update it
  for stages 1–3's changes). **Done** — the endpoint's docs in `docs/api-documentation.md`
  were audited against stages 1–3 and found already current (stage 1/3 had mirrored the
  field renames and typed warnings in when they landed); the one remaining drift was a
  `GET /api/v1/antennas` Python example still using pre-C14 field names, fixed in the same
  pass.
- Decide-and-document endpoint naming: keep two endpoints (`/heatmap` rectangular,
  `/h3-heatmap` link budget) — a full merge remains feature F5. **Done** — recorded in
  `docs/api-documentation.md`'s "Why two heatmap endpoints" note and mirrored into
  `docs/domain-contract.md` and `CLAUDE.md`.
- Exit: ~~openapi.yaml describes every registered route with post-C8 schemas; ready for
  C7~~ **amended in-flight (see the "as landed" note):** openapi.yaml is intentionally
  **not** updated by stage 4 — C7 was re-scoped to auto-generate it, so hand-fixing it here
  would have been thrown away before it shipped. The **route** parity half of this exit
  criterion (11 registered routes ↔ 11 openapi path entries) still holds against the
  existing, schema-drifted spec; the **schema** content inside those 11 entries is now C7's
  to produce correctly, not stage 4's to patch.

- **Depends on:** C3 → C4 → C2 landed first (error contract settled before the breaking
  pass); G3 (example test); S6 (validation constraints exist to document). **Blocks:** C7.
- **Out of scope (explicitly):** batch shared-context request shape (additive later via
  optional top-level defaults); poem-openapi codegen migration; any physics/semantics
  change — this pass renames and reshapes, it must not alter any computed value (existing
  numeric assertions in tests are the net: they may change *field names*, never *values*).

### C7 — OpenAPI drift guard — Effort: M

**✅ LANDED 2026-07-29** (branch `feat/c7-utoipa-openapi`, five commits, stages A–E), as
re-scoped: `openapi.yaml` is **generated** from the Rust types and handler annotations via
`utoipa 5.5` (pinned `=5.5.0`; `yaml` + `preserve_order`/`preserve_path_order` for
deterministic emission). As landed:

- **Guards.** `tests/openapi_spec.rs` pins the committed file byte-for-byte to
  `ApiDoc::openapi().to_yaml()` (regenerate with
  `cargo run -p antenna-model --bin generate_openapi`); `tests/openapi_routes_match.rs`
  pins the spec's `(method, path)` set to the `routes.rs` registrations (the original exit
  criterion), with a parser-honesty probe that exercises every scanned route through the
  real endpoint stack. The two vocabulary tests were re-aimed at the `$ref`'d
  `WarningCode`/`ErrorCode` components and survive as utoipa-upgrade canaries; the
  api-documentation.md halves are unchanged.
- **Groundwork.** `error_codes` `&str` consts promoted to a closed `ErrorCode` enum
  (wire format unchanged, pinned); `GET /antennas/{id}/feeds` typed as
  `FeedListResponse{feeds}` (wire bytes unchanged); the three `nan_as_null` fields carry
  `#[schema(value_type = Option<f64>, required)]` (hazard 1 below — resolved with a
  `value_type` override rather than `nullable = true`, same emitted `type: [number, null]`).
- **Prose.** The load-bearing descriptions (hazard 2 below) were ported verbatim into
  handler `#[utoipa::path]` attributes and type doc comments; prose shared across endpoints
  lives once under `src/api/openapi_descriptions/*.md` via `include_str!`. Spec-visible doc
  comments were curated (no rustdoc doctests or `[`intra-doc`]` link syntax leaks).
- **Post-generation acceptance checklist: all eight rows verified** in the generated file
  (see the table below). The spec is now OpenAPI **3.1** (was 3.0.3); docs point 3.1-capable
  viewers. `security: []` is no longer declared (absence means the same thing).
- **The contract is now frozen** — post-C8 shapes, behind the generate-and-diff guard.
- **Stretch goal (validating `examples/*` files against component schemas) remains open**,
  coordinated with C15 (whose `examples/api_requests.json` guard landed in stage 4).

- **Entrance / read first:** `api/routes.rs` route registration; openapi.yaml paths.
- **Re-scoped 2026-07-28 (during C8 stage 4).** Originally "hand-maintained spec plus a
  path+method drift guard." A stage-4 audit of every schema behind `/api/v1/antennas*` found
  **all seven components wrong** (C14 had filed only two of the resulting eight defects — see
  below), which made hand-fixing `openapi.yaml` before this unit lands wasted, drift-prone
  work: the spec would drift a third time before the freeze. **C7 is now scoped to
  auto-generate `openapi.yaml` from the Rust types with `utoipa`**, fixing every schema
  defect by construction instead of by a hand-authored patch. C14 and C5's openapi half are
  superseded by this re-scope; the path+method drift-guard exit criterion below is unaffected
  by it — a generated spec still needs the same route-coverage assertion.
- **Exit criteria:** a CI test that parses openapi.yaml (serde_yaml) and asserts the
  path+method set equals the registered route set — failing when a route exists without a
  spec entry or vice versa. Stretch (optional): validate G3's example files against the
  openapi component schemas. A note in docs about the guard.
- **Assumptions:** migrating to poem-openapi codegen is **out of scope** — register it as a
  possible future item in the roadmap doc, not part of this unit. *(Superseded by the
  2026-07-28 re-scope above — `utoipa` generation **is** now in scope. Line kept as history,
  not silently deleted; see standing rule on roadmap docs as a precise record.)*
- **Coordinate with C15** (filed 2026-07-27): the path+method assertion above covers routes,
  not payload shapes, and C15 inventories the client-visible surfaces that **no** guard covers
  today — `openapi.yaml`'s own schemas, `examples/api_requests.json`, the postman collection,
  the Python examples, and every `docs/*.md` except `api-documentation.md`. C15's option 3 is
  this unit's stretch goal; if C15 does not land first, promoting that stretch goal to required
  is what keeps the freeze from ratifying unchecked surfaces. C14's two mismatches are concrete
  instances the path+method check would miss.
- **Depends on:** C8 (the contract must be finalized first — this guard is what freezes it).
  **C8 landed all four stages 2026-07-28 — C7 is unblocked.**

**Post-generation acceptance checklist (2026-07-28 audit, absorbs C14).** All seven
components behind the four `/api/v1/antennas*` routes disagree with their Rust types; C14 had
filed only two of the eight resulting defects. After generation, the emitted spec must show
the right-hand column:

| component | spec said (wrong) | Rust emits (correct) |
|---|---|---|
| `AntennaInfo` | `antenna_id`, `feeds` | `id`, `feed_ids` |
| `AntennaDetailsResponse` | `antenna_id`, `calibration_info`, `calibration_status` as a *string* plus a separate `calibration_status_info`; no `enabled` | `id`, `calibration`, `calibration_status` (object, optional), `enabled` |
| `FeedInfo` | `feed_id`, `frequency_range`, plus emitterless `name` and `phase_center_offset_m` | `id`, `frequency_range_mhz`, `design_feed_offset_m`, `q_factor` — and nothing else |
| list-feeds 200 wrapper | `{antenna_id, feeds}` | `{feeds}` only |
| `PhysicalParametersInfo` | `f_over_d`, a nested `feed` object; no `focal_length_m` | `f_over_d_ratio`, `focal_length_m`, `diameter_m`, `surface_rms_mm`, optional `mesh` |
| `ValidityRangesInfo` | `azimuth`, `elevation`, `frequency`, `temperature` | `azimuth_deg`, `elevation_deg`, `frequency_mhz`, `temperature_k` |
| `CalibrationInfo` | `calibration_date`, `format_version`, `data_source`, `parameters_tuned`; no `r_squared` | `date`, `version`, `source`, `rmse_db`, `r_squared`, `num_measurements` |
| `GridData` | flat untagged object | single-variant tagged enum (`grid_type: rectangular`) |

**Two utoipa migration hazards found while scoping (2026-07-28).**

1. **`nullable` is invisible to utoipa.** Three fields use `#[serde(with = "nan_as_null")]`
   and serialize `f64::NAN` to JSON `null`: `CalibrationInfo.rmse_db`,
   `CalibrationInfo.r_squared`, and `GainResponse.gain_db`. utoipa derives `type: number` from the Rust `f64`
   and cannot see the custom serializer, so the naively generated schema would **forbid** the
   value the endpoint actually returns. Each site already carries a `// TODO(C7):` marker —
   `grep -rn 'TODO(C7)' antenna-model/src/` finds all three (`schemas.rs:282,874,882`). They
   need `#[schema(nullable = true)]`.
2. **The prose is load-bearing and will not survive generation for free.** `openapi.yaml`'s
   descriptions are contract documentation accumulated across units C1, C2, C3, C9, and C8
   stage 3 — the 400-vs-422 rule, C9's grid-peak-vs-beam-peak `loss_db` limitation, the
   warning-code vocabulary semantics, the failure sentinels. utoipa lifts Rust `///` doc
   comments into `description`, so this prose must be migrated onto the Rust types or it is
   lost outright when the hand-maintained file is replaced. Budget for it — it is the real
   cost of the migration, not the mechanical `#[derive(ToSchema)]` wiring.

---

## Phase 4 — Structure, debt, docs

### D1 — Retire the deprecated legacy serializer in calibrate — Effort: S

**✅ LANDED 2026-07-29** (branch `refactor/d1-retire-legacy-serializer`).

**The unit's premise was half-stale on arrival, and the surviving half was the real work.**
The dangerous part — `save_artifact`/`load_artifact`, the binary writer producing artifacts
the service could not load — had already been deleted by the 2026-07-18 bincode → postcard
migration, which shrank the module from 612 lines to 288 and rewrote its header. The answer
to the gating question ("do the sidecar paths actually use this module?") is **yes**:
`main.rs` called `export_metadata_json`/`export_validation_json` on every `--metadata` /
`--report` run.

What was left was the `CalibrationArtifact` wrapper the binary writer had left behind. It was
**dead weight in the shape of a live type**: neither exporter read its `antenna_config` or
`correction_surface` fields (`export_metadata_json` serializes `.metadata`,
`export_validation_json` serializes `.validation_report`), yet building it forced
`correction_surface.clone()` — a full 3D B-spline cloned per run purely to be dropped — and
kept the name "calibration artifact" attached to a thing that is not the artifact. Its
constructor `CalibrationArtifact::new` was already unreachable from production (main.rs built
the struct literally), so `new`, `summary`, and the two private range-extractor helpers were
dead too, alive only via the module's own test.

As landed:

- `calibrate/src/serializer.rs` → **`calibrate/src/sidecar.rs`** (`git mv`). The "serializer"
  name outlived the serializer; the module is now only the two JSON sidecar exporters plus
  `ArtifactMetadata`. The module header records the two-stage history so the next reader does
  not re-derive it.
- **Deleted:** `CalibrationArtifact`, `CalibrationArtifact::new`, `summary()`,
  `extract_frequency_range`, `extract_angular_range`. Kept: `ArtifactMetadata`,
  `SerializationError`, both exporters (now taking `&ArtifactMetadata` / `&ValidationReport`
  directly instead of a wrapper), sharing one private `write_json` helper.
- `main.rs` builds `ArtifactMetadata` directly; the `correction_surface.clone()` and the
  now-unused `AntennaConfiguration::new` call are gone (no information lost — the metadata
  literal already sourced `parameters_tuned` from `args`, not from the config).
- **Tests:** the deleted `test_artifact_summary` is replaced by two exporter round-trip tests
  (write → read → parse) — the exporters had no direct coverage before, only coverage
  through a type that is now gone.
- Docs re-trued where they named the renamed file: `CLAUDE.md` (workspace map),
  `docs/architecture.md`, `docs/implementation-plan.md`.
  `docs/calibration-workflow-guide.md` was checked and needed nothing — it documents the
  `--metadata`/`--report` *flags*, never the module. Historical records
  (`docs/review-findings-2026-06-10.md`, `docs/superpowers/plans/*`) keep the old name by
  design.
- `./scripts/check.sh` green (fmt, clippy `-D warnings`, full workspace tests, audit).

**Four findings filed, none fixed here** (standing rule 5 — none is D1's charter). Findings 2
and 4 came out of the failed end-to-end attempt described below; both were confirmed against
the code, and both are real defects rather than test-harness artifacts:

1. **Boresight mode writes a headerless artifact.** `main.rs:342-347` writes a bare
   `postcard::to_allocvec` with no ANTC magic/version/CRC, while full mode uses
   `write_antc_artifact`. The loader accepts both (`loader.rs:112`, "legacy headerless"), so
   this is not a break — but it means boresight artifacts carry no version stamp and no
   integrity check. → **inherited by D2** (version axes); recorded in that unit.
2. **Cross-validation validates a different surface than the one shipped.** → **new unit D10.**
3. **`calibrate/src/mod.rs` is orphaned.** A `mod.rs` beside `lib.rs` at a crate src root is
   never compiled; this one is a 16-line stale early draft of `lib.rs` declaring only
   `antenna_config` and `parser`. Nothing to salvage — confirmed 2026-07-29. → **D6** (repo
   hygiene); delete there or on the next touch of the crate.
4. **The parser silently discards every measurement below −20 dB/K.** → **new unit D11.**
   This is the root cause of the residual collapse below, and it reaches real calibration
   data, not just synthetic grids.

**The end-to-end attempt, and what it exposed.** Full-mode calibration could not be driven to
completion on synthetic data. The point count reaching the fitter collapses to ~134 regardless
of input size (240 → 154, 576 → 138, 1920 → 134), converging on a fixed near-boresight
population rather than scaling with the input. Root cause is D11: `MeasurementPoint::validate`
rejects any row with `g_over_t_db < -20.0` and `parse_csv_content` drops rejected rows with
only an `eprintln!`, so a denser or wider grid loses almost everything outside the main lobe.
The two defects then stack — with ~134 survivors the validator's fold refit trains on ~107
points, D10's unrequested nested CV trains on ~86, and the fitter's minimum of
`(spline_order+1)³ = 125` (`correction_surface.rs:1010`) trips.

Consequence for this unit: the `--metadata`/`--report` write is the last step of that pipeline,
so the CLI wiring changed here is covered by the new unit tests and the compiler, **not** by an
end-to-end run. No CLI-level integration test exists for `calibrate`
(`calibrate/tests/integration_test.rs` covers config loading only); a plan for that gap is
being written separately (2026-07-29) and is not tracked as a unit here.

---

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
  Plus the inherited finding below: boresight-mode output carries an ANTC header, and a
  round-trip test covers **both** producers.
- **Inherited 2026-07-29 from D1 (finding 1) — boresight mode writes a headerless artifact.**
  `calibrate/src/main.rs:342-347` writes a bare `postcard::to_allocvec(&calibration)` with no
  ANTC magic, version, CRC32, or length, while full mode goes through
  `main.rs::write_antc_artifact`. The service loads both — `data/loader.rs:56` takes the ANTC
  branch only when the magic matches and otherwise falls through to the legacy headerless
  path (`loader.rs:112`) — so nothing is broken today. What is missing is precisely what this
  unit is about: a boresight artifact has **no version stamp** (so the loader's
  `ANTC_SUPPORTED_VERSION` check at `loader.rs:81` cannot fire on it, and a future format
  change would silently mis-decode it instead of being rejected loudly) and **no CRC** (so
  truncation or corruption surfaces as a postcard decode error at best, wrong numbers at
  worst). Both boresight and full mode should emit the same framing; the headerless *reader*
  stays for backward compatibility, but nothing this repo produces should still rely on it.
  Note the second version axis rides along here too: `CalibrationMetadata.format_version` is
  inside the payload, so a headerless artifact is not un-versioned in the semantic sense —
  only in the container sense. Say which axis is which, as the exit criteria above require.
- **Note (2026-07-28, from C12 / C8 stage 4):** `CalibrationMetadata.rmse_db`/`r_squared`
  stay plain `f64` with a NaN sentinel by decision, not oversight — converting them to
  `Option<f64>` would change postcard's positional wire encoding, which is an ANTC
  artifact-format break belonging with this unit's version-axes reconciliation, not the API
  contract pass. The API-facing `CalibrationInfo` now has the identical `f64` +
  `nan_as_null` shape, so there is no boundary conversion between the two — see C12 and
  `docs/domain-contract.md`, "Resolved by design 2026-07-28 (C12)".
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
  green for both feature states; `calibrate/src/mod.rs` deleted (see below).
- **Inherited 2026-07-29 from D1 (finding 3) — `calibrate/src/mod.rs` is orphaned.** A
  `mod.rs` sitting beside `lib.rs` at a crate's `src/` root is never part of any module tree,
  so the compiler never reads it and no lint will ever flag it. This one is a 16-line stale
  early draft of `lib.rs`: it declares only `antenna_config` and `parser` and re-exports
  their types. Checked 2026-07-29 — **nothing to salvage**, it is a strict subset of
  `lib.rs`. Safe to delete here or on the next touch of the crate, whichever comes first.
- **Priority note (2026-07-29):** this unit's original headline rationale — "the primary
  lever on the advisory count", filed 2026-07-09 against 17 vulnerabilities + 9 warnings from
  the AWS subtree — is **stale**. `calibrate/Cargo.toml:11-18` already disables
  `aws-config`/`aws-sdk-s3` default features to drop the vulnerable rustls 0.21 connector,
  and upstream bumps closed the rest: `cargo audit` on `main` now reports **one** advisory
  (`RUSTSEC-2024-0436`, `paste` unmaintained, already allowlisted). Gating the S3 subtree is
  still worth doing for CLI build weight and dependency surface, but this is hygiene now, not
  a security lever — priority drops back to "cheap, fold in anywhere."
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
- **Note (2026-07-28, from C12 / C8 stage 4):** `CalibrationMetadata.rmse_db`/`r_squared`
  remain `f64` with a NaN sentinel by decision — making them `Option<f64>` is an ANTC
  wire-format break and belongs with D2's version-axes work, not this unit or the contract
  pass. Recorded here because D9 owns the artifact-shipping story this sentinel is part of.
- **Depends on:** G2, S5 (readiness semantics for the zero-artifact state).

### D10 — Cross-validation validates a different surface than the one shipped — Effort: S

**✅ DONE 2026-07-29** — branch `fix/d10-d11-calibrate-correctness`. All four exit criteria met.
Defect (b) turned out to be **worse than filed**: the nested CV is not merely unrequested, it is
**unbounded recursion**. `cross_validate` (`correction_surface.rs:931`) refits each fold by
calling `fit_correction_surface` with the *same* params — `cross_validation_folds` still 5 — so
every level re-enters cross-validation on a training set 20% smaller than the last, until the
geometric shrink crosses the `(spline_order + 1)³` minimum and the whole run fails. Measured on
a 256-point fixture: 205 → 164 → 132 → 106 < 125 → `InsufficientData { min_required: 125,
actual: 106 }`. **Cross-validation could not complete at all** before this fix, on any input;
that is why D1's end-to-end attempt died where it did.

Consequently the guard is applied at **both** layers, not just the one the unit named:
`CorrectionSurfaceParams::without_nested_cross_validation()` is the single definition of "an
inner fit on behalf of an outer CV", and it is called by `validator::perform_cross_validation`
(the fold refit) *and* by `correction_surface::cross_validate` (the fold fit). The second call
site is what actually stops the recursion — a `fit_correction_surface` caller that asks for CV
directly, which is exactly what `main.rs` step 5 does under `--validate`, never reaches the
validator at all. No fitting math was touched, and `CorrectionSurfaceParams::default()` is
unchanged.

**Before/after on a 256-point synthetic grid** (2 frequencies × 16 E-cone × 8 E-clock, 5 folds):

| | mean CV RMSE |
|---|---|
| before — `::default()` params, recursion live | **run fails**: `InsufficientData { 125, 106 }` |
| after — `::default()` params (recursion fixed, for comparison only) | 1.391888 dB |
| after — artifact params `4/6/8 @ 1e-3` (**what now ships**) | **1.433256 dB** |

The reported number moved **+0.041 dB worse**, in the predicted direction: the default's 8/8/12
knots at 1e-6 regularization is a markedly more flexible family, and it was flattering the
artifact. `correction_params.cross_validation_folds` no longer changes the result at all
(1.433256 either way) — the nested-CV knob is inert by construction.

**Tests:** `main.rs::tests::validation_config_scores_the_surface_that_ships` (pins defect (a) —
the wiring is extracted into `surface_fitting_params`/`validation_config` so the binary's
config construction is unit-testable, and the test asserts the config differs from the default
so the fixture cannot silently stop discriminating);
`validator::tests::{fold_refit_uses_caller_knot_counts_and_regularization,
fold_refit_uses_caller_spline_order, no_nested_cross_validation_under_any_caller_configuration,
num_folds_controls_the_reported_fold_count}`;
`correction_surface::tests::{without_nested_cross_validation_preserves_every_other_field,
cross_validation_does_not_recurse_into_itself}` (the last is sized so recursion is what fails:
176 points → 141 in a fold clears the 125 minimum, a second level would train on 113 and trip it).

**One finding, not fixed here** (standing rule 5 — outside this unit's charter): full-mode
step 6 runs `validate_calibration` **unconditionally** with `num_folds: args.cv_folds`, whose
clap default is 5. So the *outer* cross-validation runs on every full-mode invocation even
without `--validate`, whose help text is "Run cross-validation after fitting". Step 5 correctly
gates on `--validate`; step 6 does not. Gating it is a CLI behavior change this unit was not
chartered to make. → file against the `calibrate` CLI integration-test work.

**Resolved 2026-07-30 in D12 Task 1** — `validation_config` now takes `--validate` and sets
`num_folds = 0` when it is false; only cross-validation is gated, the rest of step 6 is
unchanged.

---

**Filed 2026-07-29** out of D1's finding 2. **Correctness-class, not hygiene** — it sits in
Phase 4 by crate, not by severity. Two defects in one plumbing mistake.

**Defect (a): the fold refit uses default fitting parameters.** `calibrate/src/main.rs:546-557`
builds the `CorrectionSurfaceParams` the shipped artifact is actually fitted with — spline
order 4, **4/6/8** knots (frequency/E-cone/E-clock), regularization **1e-3**. Ten lines later,
`main.rs:585` hands the validator `correction_params: CorrectionSurfaceParams::default()`
instead, and the per-fold refit at `calibrate/src/validator.rs:736-740` fits with *that*:
**8/8/12** knots, regularization **1e-6** (`correction_surface.rs:113-128`). So every
cross-validation fold fits roughly **double the knots at 1000× weaker regularization** — a
markedly more flexible surface, and the direction that matters: the CV number reported for the
artifact is measured on a model family more prone to overfit than the one being blessed. The
`num_folds` field is threaded correctly (`main.rs:579`); only `correction_params` is dropped.

**Defect (b): nested cross-validation runs unrequested.** Because the default carries
`cross_validation_folds: 5`, each fold's refit starts its *own* 5-fold CV inside
`fit_correction_surface` (`correction_surface.rs:351`). This happens even when the user did not
ask for cross-validation at all — `main.rs:553` sets `cross_validation_folds: 0` unless
`--validate` — and `--cv-folds` never reaches it. Visible as an unexplained "Running 5-fold
cross-validation…" on a run that requested neither, and it is what shrinks each fold's training
set far enough to trip the fitter's `(spline_order+1)³ = 125` minimum on data that would
otherwise fit (see D11 for the other half of that stack).

- **Fix:** (1) `main.rs` passes the same `surface_params` into `ValidationConfig` rather than
  `::default()`; (2) `validate_calibration` **unconditionally** forces
  `cross_validation_folds = 0` on its internal refits — nested CV inside a CV fold is never
  wanted, so this must not depend on the caller getting (1) right.
- **Exit criteria:** the fold refit provably uses the caller's knot counts and regularization
  (a test that passes distinguishable params and asserts the refit surface's shape/knot
  vectors match them); no nested CV under any caller configuration (a test that sets
  `cross_validation_folds: 5` on the outer params and asserts the inner fit does not recurse);
  `--cv-folds N` observably changes the fold count in the report. Expect the reported CV RMSE
  to **move** — it was measuring the wrong model, so a changed number is the fix working, not
  a regression. Record the before/after on a fixture in the commit message.
- **Gotchas:** do not "fix" this by changing `CorrectionSurfaceParams::default()` — the
  default is legitimate for other callers; the bug is that `main.rs` does not pass its own
  params. Do not touch the fitting math.
- **Sequencing:** **before** any `calibrate` CLI integration-test work — it is one of the two
  defects that made an end-to-end run impossible, and integration tests written against the
  current behavior would pin the bug.
- **Depends on:** nothing. Independent of D1/D2.

### D11 — The parser silently discards every measurement below −20 dB/K — Effort: S/M

**✅ DONE 2026-07-29** — branch `fix/d10-d11-calibrate-correctness`. All four exit criteria met.
`MeasurementPoint::validate` is now physicality-only: an explicit finite check on all five
fields (previously absent — a `NaN` frequency, temperature or G/T passed validation, since
`NaN` fails every comparison in both directions) plus the existing angle / frequency /
temperature bounds. The G/T range test is gone from validation and reappears as
`MeasurementPoint::has_atypical_g_over_t`, counted into two new `DataQualityReport` fields
(`atypical_g_over_t_count`, `g_over_t_range`), rendered as a warning line by
`DataQualityReport::format`, and echoed at WARN by the CLI's step-1 summary. The former
`[-20, 70]` literal is now the named `parser::TYPICAL_G_OVER_T_RANGE_DB`, documented as a
*boresight* figure and explicitly not a validity test. The `eprintln!` is replaced by a
structured `tracing::warn!` carrying `source`, `dropped` and `retained` fields plus a sample
capped at `MAX_REPORTED_PARSE_ERRORS = 10`, with a second line naming the number withheld.

**Before/after on a realistic pattern** (ITU-R S.580 envelope `29 − 25·log10 θ` dBi out to 48°
then a −10 dBi floor, T_sys = 50 K, so peak G/T 41.5 dB/K falling to −27 dB/K at wide angles):

| grid | rows | retained BEFORE | retained AFTER |
|---|---|---|---|
| 10 × 8 × 2 freq | 160 | 32 (20.0%) | **160 (100%)** |
| 16 × 12 × 2 | 384 | 96 (25.0%) | **384 (100%)** |
| 24 × 20 × 2 | 960 | 200 (20.8%) | **960 (100%)** |
| 40 × 24 × 2 | 1920 | 432 (22.5%) | **1920 (100%)** |

The old gate discarded ~78% of every grid, and — matching D1's observation exactly — the
survivors were the near-boresight population rather than a fixed fraction: everything beyond
θ ≈ 19°, where the S.580 envelope crosses −20 dB/K, was dropped regardless of grid density.

**Downstream statistics moved, as the unit predicted** (1920-row grid, 0.5° beamwidth). These
are the numbers to expect, not regressions:

| | BEFORE (survivors only) | AFTER (all rows) |
|---|---|---|
| total points | 432 | 1920 |
| main-lobe points | 48 | 48 (unchanged — the gate never cut the main lobe) |
| sidelobe points | 384 | **1872** |
| G/T range | (−19.7, 41.5) | **(−29.6, 41.5)** |
| `outlier_count` | 48 | **768** |

**Tests:** `parser::tests::{realistic_off_axis_measurements_are_not_dropped,
deep_sidelobe_points_are_individually_valid, non_finite_values_are_rejected,
quality_report_warns_about_atypical_g_over_t_without_dropping_it,
quality_report_stays_quiet_when_all_points_are_typical,
malformed_rows_are_dropped_and_reported_through_tracing, a_clean_parse_reports_no_warning,
the_reported_failure_sample_is_bounded}`. The `tracing` assertions capture real subscriber
output through a `MakeWriter` rather than trusting the call site.

**Two findings, not fixed here** (standing rule 5):

1. **`detect_outliers` is no longer meaningful on full-pattern data.** It applies a modified
   Z-score (MAD on raw `g_over_t_db`) across all points, which asks "how far is this point
   from the median of the pattern" — a sensible question on a main-lobe-only population and a
   meaningless one on a population spanning 70 dB. The flagged count rose 48 → **768 of 1920
   (40%)** purely because the data got wider. Nothing is wrong with the measurements; the
   statistic is being applied to the wrong quantity (it should run on *residuals*, as
   `validator::identify_outliers` already does, not on raw G/T). Deliberately left alone —
   changing it is a design decision, and the D11 gotcha explicitly forbids tuning numbers to
   look unchanged.
2. **`create_sample_csv` cannot produce data that would have exercised this bug.** Its
   synthetic pattern is `41.5 − (θ/5)²`, bottoming out at **+5.5 dB/K** at θ = 30° — a
   quadratic-in-degrees rolloff no real dish has, and comfortably above the old −20 gate. The
   generator sat just inside the threshold that was silently destroying real data, which is
   part of why the gate survived so long. → the `calibrate` CLI integration-test work should
   replace it with a realistic envelope.

---

**Filed 2026-07-29** out of D1's finding 4. **Correctness-class**, and it reaches real
calibration data, not just synthetic grids.

**The defect.** `MeasurementPoint::validate` (`calibrate/src/parser.rs:53`) treats G/T as a
physicality check and rejects any row outside `[-20, 70]` dB/K (`parser.rs:78-84`, comment:
"G/T typically ranges from -10 to 60 dB/K for realistic antennas"). That is a **boresight**
figure. Off the main lobe a real pattern falls tens of dB below peak, so legitimate sidelobe
measurements sit far below −20 dB/K and are rejected as if malformed. `parse_csv_content`
(`parser.rs:351-354`) then *drops* rejected rows, collects the reasons into an `errors` vector,
and — provided at least one row survived — reports them with a single `eprintln!`
(`parser.rs:371-376`) before returning success. Not `tracing`, so it bypasses the configured
log level, the JSON formatter, and every log sink the project otherwise uses; a silent
`eprintln!` is how this hid.

**Evidence (2026-07-29, from D1's end-to-end attempt).** Point count reaching the fitter, by
input grid size: 240 → 154, 576 → 138, 1920 → 134. It converges on the near-boresight
population instead of scaling with input — the exact signature of a fixed-threshold cut on a
pattern that falls off with angle. Widening or densifying the grid adds only rows that get
discarded.

**Why it matters beyond testing.** The project's stated accuracy requirement covers the **first
sidelobe** explicitly (CLAUDE.md, "Accuracy Requirements"; `ValidationConfig.first_sidelobe_*`
targets), and the correction surface exists to absorb residuals across the measured range. A
gate that removes sidelobe rows before fitting means the surface is fitted, validated, and
reported against main-lobe data only, while the report still prints first-sidelobe statistics
computed from whatever handful of rows happened to clear −20. This silently narrows real
calibrations, not just test fixtures.

- **Fix:** keep `validate()` to **physicality only** — finite values, and the angle /
  frequency / temperature bounds it already checks — and drop the G/T range test from it.
  Move the G/T range check into `DataQualityReport` as a *warning* (out-of-typical-range
  count, with min/max), where an operator can see it without losing the data. Route
  dropped-row reporting through `tracing::warn!` with a **count** plus a bounded sample of
  reasons, so a large drop is loud and does not scroll past as a wall of text.
- **Exit criteria:** a fixture with realistic off-axis G/T values (−40 dB/K and below) parses
  with **zero** rows dropped; a fixture with genuinely malformed rows still drops them and the
  drop is reported through `tracing` at WARN with an accurate count; the quality report
  surfaces out-of-typical-range points as a warning rather than silently discarding them; no
  `eprintln!` remains in the parse path.
- **Gotchas:** check `is_main_lobe`/`DataQualityReport` and the validator's main-lobe and
  first-sidelobe partitioning after this lands — they will start seeing the sidelobe
  population for the first time, and their point counts and reported statistics will move.
  That is the defect being fixed, not a regression; capture before/after on a fixture. **Do
  not** adjust any accuracy target to keep numbers looking the same.
- **Sequencing:** before CLI integration-test work, alongside D10 — together they are what
  made an end-to-end run impossible.
- **Depends on:** nothing.

### D12 — calibrate CLI end-to-end test on perturbed-truth synthetic data — Effort: M

**✅ DONE 2026-07-30** — branch `feat/d12-calibrate-cli-e2e`. Six commits: `3f7b657` (Task 1),
`5e7bc6d` + `4348eba` (Task 2), `d0ab870` + `e183a64` (Task 3), `cab335c` + `1b0a029` (Task 4),
plus `d9c6f44` (flake fix), `921b6ca` and `2b70d69` (findings), and `586aa7a` (Task 5). Final
suite: `cargo test -p calibrate --test cli_full_mode_e2e` → **11 passed, 1 ignored**, ~11 s.

**Task 1 — cross-validation gated on `--validate`.** `validation_config` in
`calibrate/src/main.rs` gained a `validate: bool` parameter and sets `num_folds = 0` when
false. Only cross-validation is gated; corrected RMSE, main-lobe/first-sidelobe statistics,
outliers and band analysis run unconditionally. Closes the finding D10 filed above.

**Task 2 — the fixture.** Deterministic perturbed-truth generator in
`calibrate/tests/support/mod.rs`. Class `UHF_Array_Element`, chosen for its broad beam
(8.91 dB/K at boresight, −41.53 at 20°) because the fitter's 2° minimum E-cone knot spacing
cannot resolve a narrow-beam class — `GroundStation_13m` falls 33.08 → −10.60 dB/K between 0°
and 1° at 4 GHz. **288 rows**: 4 frequencies (400/500/600/700 MHz) × 9 E-cone (0–24°) × 8
E-clock (0–315°). **Minimum G/T −68.22 dB/K**; **144 of 288 rows (50%) below −20 dB/K** — a
standing pin on D11. Injected bias range **[0.200, 2.300] dB**, matching the closed-form
extremes of its coefficients. Surface RMS perturbed **2.0 → 2.6 mm**; only surface RMS is
perturbed, since full-mode `TunableParameters` has no `q_factor`, so a q perturbation could
never be recovered. A drift guard (`fixture_config_matches_antenna_classes_yaml`) fails loudly
if `antenna_classes.yaml`'s entry diverges from the hardcoded fixture config.

**Task 3 — the end-to-end run.** First test executing the real binary through
parse → predict → fit → validate → artifact. The artifact loads through the **service's**
loader (`antenna_model::data::loader`), not just calibrate's own round-trip code. **Model-only
RMSE 1.3071 dB → corrected 0.9756 dB** (ratio 0.746, a 25.4% improvement). The improvement
assertion is bounded at `corrected < 0.9756 + 0.02` dB — an absolute epsilon, because the
pipeline is deterministic to 4 decimal places across debug and release runs, so the epsilon
covers cross-platform libm ULP differences rather than run-to-run variance. **This bound is
deliberately weak and should be tightened once the edge-collapse defect is fixed** (finding 1
below). Fixture generation is cached across `run_calibrate` calls: single-threaded suite time
**12.89 s → 11.55 s**.

**Task 4 — known-answer recovery.** Recovery of the injected bias at four interior probes,
plus CLI-level pins for `--cv-folds` (N = 3 and 6 produce N fold RMSEs) and for the Task 1
gating (no `cross_validation` section and no CV announcement without `--validate`). Per-probe
errors: **0.5928 dB** at (450 MHz, 3°, 30°) — the worst case, nearest the main lobe; **0.0934**
at (550, 7, 120); **0.0365** at (570, 14, 200); **0.0934** at (500, 10, 260). Tolerance
`BIAS_RECOVERY_TOLERANCE_DB = 0.65 dB`. A review caught that the original probe 3 sat at
620 MHz, *inside* the fitted frequency knot vector's topmost span [600, 700] MHz — so it was
partly measuring the edge-collapse defect (finding 1) rather than fit quality. Moving it to
570 MHz cut its error from 0.1716 to 0.0365 dB. The 0.5928 dB worst case is itself far worse
than it should be: on an *overdetermined* grid (1232 points) the same bias is recovered to
~0.003 dB. The shipped fixture is 288 points against **960 coefficients** —
`(4+4)(6+4)(8+4)` for the artifact's 4/6/8 knot counts at order 4 — so it is badly
underdetermined. **The known-answer assertion is therefore weaker than it should be** until
the fitter's data-sufficiency check is fixed (that check requires `(spline_order+1)³ = 125`,
which is the wrong quantity — see finding 1).

**Task 5 — the tuned run: blocked by a defect, test committed `#[ignore]`d.**
`calibrate --tune-parameters` **crashes** — `attempt to subtract with overflow` in argmin's
Nelder-Mead, ~0.7 s in, for every `--tuning-mode` and every `--max-tuning-iterations`.
`parameter_tuner.rs:389` builds the simplex with a single vertex where `N+1` are required. The
identical bug was found and fixed in `boresight_calibration.rs` (2025-11-27) but never ported.
Separately, `ParameterBounds::default()` caps `surface_rms_mm` at (0.1, **2.0**) mm — exactly
`UHF_Array_Element`'s nominal — so the 2.6 mm perturbation is unreachable even once the crash
is fixed. Task 5's original charter (measure the wall clock, decide CI status) **could not be
carried out** — the run never completes. The test exists, is `#[ignore]`d with the
reproduction command, and is ready to enable once the defect lands; the timing measurement was
not done.

**Three findings, none fixed here** (standing rule 5 — D12 doing its job):

1. `docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md` (commit `921b6ca`) —
   the correction surface returns ~0 across the topmost knot span of **every** axis, in both
   calibrate's 3D `CorrectionSurface::evaluate` and the service-side 4D `evaluate_correction`.
   Fitting a constant 1.5 dB: interior 1.500000, any axis at max 0.000000, 699.999 →
   0.000090. Loses 332 of 1232 points (26.9%) on a regular grid. **Latent today** (no `.bin`
   ships, all enabled antennas uncalibrated) but a served-path wrong answer as soon as D9/D14
   ship an artifact. **Recommend fixing before D14**, since D14 builds a real-anchored
   artifact for the served calibrated path. The doc also records two refuted hypotheses
   (degenerate adaptive knots; underdetermination) and two adjacent problems (the wrong
   data-sufficiency quantity; `compute_r_squared` returning a hardcoded 1.0 when
   `ss_tot == 0`).
2. `docs/findings-2026-07-30-full-mode-parameter-tuning-broken.md` (commit `2b70d69`) —
   Task 5's two defects above.
3. **Flake fix, commit `d9c6f44`** — the three D11 log-capture tests in
   `calibrate/src/parser.rs` failed ~20% of the time under parallel execution (2 of 10 runs of
   `cargo test -p calibrate --lib parser`), passing 12/12 in isolation and 6/6 single-threaded.
   A scoped `tracing` subscriber is thread-local, but dispatcher registration/drop moves
   tracing's **global** max-level filter, so a sibling's event was discarded before reaching
   its subscriber and the capture came back empty. Serializing the capturing tests made it
   *worse* (7 of 20). Fixed by installing one global subscriber that never unregisters,
   writing to a thread-local buffer: 30/30 green afterwards, 10/10 on the full lib suite.
   **Introduced by D11 in PR #26 — this was a live intermittent CI failure on `main`.**

---

**Filed 2026-07-29.** D1's closeout stated the gap plainly: `calibrate` has library-level
integration tests (`calibrate/tests/`) but **nothing that runs the built binary** through
parse → tune → fit → validate → artifact. The full-mode `main.rs` wiring is covered only by
unit tests and the compiler. D10 and D11 are what made an end-to-end run impossible; once
they land, this unit builds the test they were blocking.

**Design principle — perturbed truth, not same-model fill.** If the synthetic measurements
are generated by the *same* model configuration being calibrated, residuals are identically
zero: the pipeline runs but the correction surface fits nothing and the test asserts
nothing. Instead, generate "measurements" from the physics model with **deliberately
perturbed parameters** (e.g. surface RMS and feed q-factor offset from the design spec)
plus a **known injected smooth systematic bias** (a closed-form function of frequency/cone/
clock — deterministic, no RNG) and optionally small noise. Calibrating the *nominal* design
spec against this data gives known-answer assertions no real dataset can: the tuner must
recover the perturbation, and the correction surface must recover the injected bias.

- **Scope:**
  1. A deterministic generator (test-support code in `calibrate/tests/`; it may use the
     `antenna-model` crate directly — `calibrate` already depends on it) that writes a
     dense measurement CSV: ≥3 frequencies spanning well over the 50 MHz knot minimum, a
     cone/clock grid covering the main lobe through the first sidelobes, and **realistic
     off-axis G/T values below −20 dB/K** — which makes this test also the standing pin on
     D11's fix (those rows must reach the fitter).
  2. A CLI integration test that runs the actual binary (Cargo provides
     `env!("CARGO_BIN_EXE_calibrate")` in integration tests) with
     `--calibration-mode full --validate` into a temp dir.
  3. Assertions: exit 0; the artifact has the ANTC magic/header and loads through the
     *service's* loader (`antenna-model` `data/loader.rs`), not just calibrate's own code;
     the tuned parameters recover the injected perturbation within a stated tolerance; the
     correction surface evaluated at probe points recovers the injected bias within
     tolerance; corrected RMSE ≪ uncorrected RMSE; `--cv-folds N` observably changes the
     report (pins D10).
- **Runtime:** the default CI variant runs **without** `--tune-parameters` (the
  differential-evolution tuner has its own unit tests); add one tuned end-to-end run and
  measure it before deciding whether it needs `#[ignore]`/nightly.
- **Exit criteria:** the CLI e2e test above exists, runs in CI, and each listed assertion
  is present; the generator is deterministic (two runs produce byte-identical CSV); a
  fixture-level comment documents the injected truth so tolerances are auditable.
- **Gotchas:** never loosen the fitter's `(spline_order+1)³` minimum or the knot-spacing
  floors to make a fixture fit — size the fixture to the fitter, not vice versa. Keep the
  injected bias smooth at the scale of the knot spacing, or the recovery assertion will
  fail for legitimate reasons (the spline cannot represent it).
- **Depends on:** D10, D11 (hard — both defects sit on this test's path). Not blocked on
  D2: full mode already writes the ANTC header via `write_antc_artifact`.

### D15 — Fix the correction-surface upper-edge-collapse (D12 finding 1) — Effort: S

**✅ DONE 2026-07-30** — branch `fix/correction-surface-endpoint`. Four commits: `a866cfb` (the
fix), `e87efe6` (golden re-pin), `41b7e94` (comment corrections), `c79d2cf` (retargeted D12's
assertions). Fixes
`docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md`, filed by D12 as finding 1.

**Mechanism.** `bspline_basis`'s `k == 1` base case (`calibrate/src/correction_surface.rs`) used
a half-open span `t_i ≤ t < t_{i+1}`, so at `t == t_max` no basis function was non-zero and the
basis was not a partition of unity at the exact domain maximum. A pre-existing "right endpoint"
special case keyed on `i == knots.len() - 2` — for a clamped knot vector that is a *padding*
index outside the valid basis range `0..knots.len() - order`, so it never fired for a basis
function that is actually evaluated. Measured with all coefficients 1.0 (a correct basis must
sum to exactly 1.0): 1.000000000 at t=0.99, 1.000000000 at t=0.9999, 0.000000000 at t=1.0 —
razor-thin, not broad. The damaging half was the fitting side: `accumulate_normal_equations`
uses the same basis, so every measurement lying exactly on an axis maximum (always populated on
a regular grid — 72 of D12's 288 rows sit at 700 MHz) contributed an all-zero row, starving the
top coefficient on that axis to ~0 via the ridge term and corrupting the fit across the entire
top knot span. That is why 699.999 MHz returned 0.000090 while the basis there was already
≈1.0 — the basis was fine; the coefficient it multiplied had already been destroyed.

**The service-side 4D interpolator was never defective — this is the correction to D12's
finding.** `antenna-model/src/model/correction_interpolator.rs` uses the standard NURBS-book
Cox-de Boor recurrence (`basis[0] = 1.0` then the triangular recurrence) — a different algorithm
from calibrate's naive recursive `bspline_basis` — with a `find_knot_span` that clamps to the
last valid span. Verified by partition of unity with coefficients built by hand (bypassing the
fitter entirely): interior, azimuth-at-max, elevation-at-max, frequency-at-max,
temperature-at-max, and all four axes at max simultaneously all return exactly 1.000000000. Its
apparent failure in D12's original diagnostic was inherited corrupted coefficients from the
broken fitter — the served path was never itself wrong; artifacts were fitted wrong and the
service faithfully served the bad coefficients. Still blocking for D9/D13/D14 (a shipped
artifact would have carried the corruption), but it is an artifact-production defect, not a
service defect, and the fix touched exactly one file, `calibrate/src/correction_surface.rs`.

**Before/after (D12's fixture, `calibrate/tests/cli_full_mode_e2e.rs`, `c79d2cf`):** corrected
RMSE **0.9756 → 0.0058 dB** (168× improvement; model-only 1.3071 dB unchanged). The four
known-answer probe errors are **unchanged** — 0.5928 / 0.0934 / 0.0365 / 0.0934 dB —
because the probes are off-grid and away from any upper edge, so their coefficients were never
starved; `BIAS_RECOVERY_TOLERANCE_DB` stays at 0.65, deliberately not tightened. The probe
residual was therefore never the edge collapse — it is overfitting from underdetermination: 960
coefficients (`(4+4)(6+4)(8+4)` for 4/6/8 knots at order 4) against 288 points, letting the
surface interpolate fitted points almost exactly while oscillating between them.

**Golden re-pin (`e87efe6`, corrected by `41b7e94`).** `fit_matches_openblas_golden` pinned
values captured when only the solver changed (LAPACK `dgesv` → in-house Cholesky), so it encoded
the buggy basis rather than testing against it — a solver-drift guard, not a basis oracle. Its
fixture reaches 8700 MHz, the frequency knot maximum, on exactly **36 of 288 points** (`i == 7`
of an `8×6×6` loop — `e87efe6`'s commit message said 48; `41b7e94` corrected the count and
cannot edit the earlier commit message). Re-pinned: `sum` 81.54 → 87.17, `c[last]` 0.1490 →
0.5661 (3.8×), while `c[0]`, `c[1]`, `c[mid]` barely moved (they never touch the frequency-max
span; `c[last]`, index 124, is the single coefficient at `i_freq = 4`, the starved one).
Justified against three checks: `fit_satisfies_normal_equations`,
`normal_equations_match_dense_reference`, and the new `a_fitted_constant_is_recovered_at_the_
domain_maximum` — the genuine basis oracle, since a constant is analytically exactly
representable.

**New tests:** `basis_is_a_partition_of_unity_on_every_face_and_corner`,
`basis_is_continuous_up_to_the_maximum`, `a_fitted_constant_is_recovered_at_the_domain_maximum`.

**Left open, three items, none fixed here:**

1. `validate_knot_vector` does not check multiplicity — `generate_adaptive_knots` can place
   knots at an axis's min/max, producing multiplicity 5 at a boundary for order 4 on the
   shipped adaptive-knot config. Excluded: adding the check would fail on the *current* adaptive
   knots; fixing it also requires fixing `generate_adaptive_knots`' quantile placement.
2. The data-sufficiency check tests `(spline_order+1)³ = 125` when the real requirement is the
   coefficient count, 960 for the shipped 4/6/8 config. Excluded: fixing it would make D12's
   288-point fixture fail its own minimum outright — a fixture-sizing design decision, not part
   of this fix. **This is what still limits the recovery accuracy above** — the 0.5928 dB
   worst-case probe error is underdetermination overfitting, not the edge collapse, and will not
   improve until this is addressed.
3. A fully degenerate axis (every knot equal) makes every basis function return 0 rather than 1
   — pre-existing, not introduced or fixed here, currently unreachable because
   `generate_knot_vector` rejects degenerate ranges upstream. Noted in `bspline_basis`'s doc
   comment (`41b7e94`).

**This fix does not make the correction-surface fit well-determined** — it corrects a basis
evaluation bug that was corrupting fitted coefficients at every axis maximum; item 2 above is
the concrete mechanism by which the fit remains underdetermined today.

### D13 — Real-data boresight calibration test (NTIA frequency sweeps) — Effort: S/M

**Filed 2026-07-29.** The 2026-07-29 assessment of the digitized reference data (see the
narrative roadmap, Addendum 2026-07-29) found that **boresight mode is the one calibration
path real published data can drive today**: boresight calibration is a frequency-sweep fit
(`BoresightMeasurements::from_csv`, columns `frequency_mhz,g_over_t_db,temperature_k`;
Nelder-Mead over a handful of parameters — no large point minimum), and
`ntia_84_164_antennas.psv` contains genuine multi-frequency boresight gain measurements.
Best candidate: **Andrew 43998, 10 m — 6 frequencies spanning 3700–6425 MHz** (SA 8002A,
5 frequencies, is an alternative/second fixture).

- **Scope:**
  1. A committed fixture CSV derived from the NTIA rows: gain → G/T via a documented
     assumed constant system temperature; `temperature_k` column constant. Fixture header
     documents provenance and every assumption (F8's headers are the template), including:
     the Rx-band (3.7–4.2 GHz) and Tx-band (5.9–6.4 GHz) rows very likely came from
     different feeds on the real hardware, and f/D is not published — the design-spec entry
     uses an assumed typical value, stated in the header.
  2. A design-specs file entry for the dish (boresight mode requires `--design-specs`).
  3. CLI e2e test: run the binary with `--calibration-mode boresight`; assert exit 0, the
     artifact carries the ANTC header (this pins D2's fix for D1-finding-1 — boresight
     currently writes bare headerless postcard, `main.rs:340-345`), it loads through the
     service loader, the antenna serves with `PartiallyCalibrated` status plus the
     partial-calibration warning, and served boresight gain at the measured frequencies
     lands within a stated tolerance of the NTIA values.
- **Exit criteria:** the fixture + test above in CI; provenance header complete; tolerance
  stated with a one-line justification in the test.
- **Gotchas:** the boresight CSV parser is separate from the full-mode parser and fails
  hard rather than dropping rows — D11's gate does not apply here; do not "harmonize" the
  two parsers in this unit. Real data means real residuals: pick the tolerance from the
  measured before/after, not from wishful thinking, and record it.
- **Depends on:** D2 (so the test pins the final headered artifact format, not the legacy
  one), D12 (reuses its CLI-harness pattern).

### D14 — Real-anchored full-mode artifact: NASA CR-159703 hybrid fill — Effort: M/L

**Filed 2026-07-29; approach approved by the maintainer 2026-07-29** (register row D14).
The same data assessment concluded **no digitized dataset can drive full-mode fitting** —
the fitter needs ≥125 points and per-axis spans over the knot minimums, while the best real
single-antenna, single-configuration set (`nasa_cr159703_pattern_peaks.psv`, cut
`122_kumar_C_h_121`) is ~12–16 envelope peaks at one frequency — and that this is likely
permanent, because full 3D G/T grids are essentially never published. The maintainer's
accepted fallback: **use our own model to fill the gaps, anchored to the real digitized
measurements** — not ideal, stated honestly, tangible until a better dataset exists.

**Why this dish:** the CR-159703 antennas are true prime-focus paraboloids (f/D 0.38,
~20 dB-taper Kumar feed) — the **only** digitized dataset whose topology matches the
model's (DSN is shaped-Cassegrain, GBT offset-Gregorian), so measured-minus-model residuals
are meaningful rather than dominated by a topology gap.

- **Method (each fabrication step documented in the fixture/script provenance):**
  1. Design-spec entry for the 1.22 m dish from the report's figures (f/D 0.38, feed
     geometry per the file header; text gain 41.4 dBi and HPBW ~1.5° as anchors).
  2. Compute the model's pattern; take residual = digitized peak (made absolute via the
     text gain anchor) − model at the peak angles.
  3. Interpolate the residual smoothly across cone angle; across clock, use the H/E plane
     pair where both exist, otherwise a documented axisymmetry assumption.
  4. Synthesize the dense grid = model + interpolated residual over the report's
     11.7–12.2 GHz band, residual **assumed frequency-flat** (a documented fabrication),
     with a plausible constant system temperature for the G/T conversion.
  5. Run the full-mode CLI on it; load the artifact through the service.
- **Assertions:** the served **calibrated** pattern reproduces the digitized peaks within
  their stated uncertainties (±1.0–1.5 dB level, ±0.3–0.5° angle, plus the absolute-anchor
  uncertainty — budget them explicitly); calibrated beats uncalibrated at the anchor
  points; the antenna serves with `Calibrated` status.
- **D9 exemplar:** the generation lives as a documented script (`scripts/`), making this
  the worked example of D9's "document + script the generation path" recommended default —
  note it in D9 when this lands.
- **Exit criteria:** script + fixtures + CLI e2e + service-side test in CI; every
  fabricated element (fill, frequency-flatness, clock symmetry, T_sys, absolute anchor)
  listed in one provenance block; the artifact is generated, never committed (D9).
- **Gotchas:** fixture headers must label the fill as model-interpolated — this dataset
  must never be quotable as "measured". Anchor-recovery tolerances must include the
  digitization uncertainty budget, not just fit error. If the fitted surface cannot
  reproduce the anchors within budget, that is a *finding about the pipeline or the fill*,
  to be reported — not a reason to widen the budget.
- **Depends on:** D10, D11, D12 (infrastructure and prerequisite fixes), D2 (artifact
  format settled). Feeds D9.

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

**✅ DONE 2026-07-16/17** — branch `feat/f7-redesign-power-sum-obliquity`, commits (oldest→newest,
`main..HEAD` as of this unit's doc-cleanup task):
- `d5f1bad` test(F7): bound the boresight-tuner surface_rms -> sidelobe-floor coupling (ship precondition 2)
- `de83a0c` style: cargo fmt reference_validation.rs
- `bddcf3c` docs: fix stale "Two handoffs" count after adding item 3
- `291ed3c` feat(F7): Huygens obliquity factor (1+cos(theta))/2 on the far-field conversion; re-derive wide-angle anchors
- `62e5414` docs(F7): re-true field_to_dbi docstring and rear-test header after the obliquity change
- `f57721d` feat(F7): power-sum floor combination forward, floor-only rear hemisphere (model layer, flag still off)
- `2c807e9` test(F7): re-add deep-null guard assertion to the power-sum floor test
- `6d5fc7e` feat(F7): enable the sidelobe floor on the served path via physics_is_uncorrected() (P11 gate)
- `00b51c6` feat(F7): re-word off-axis and rear-hemisphere honesty warnings for the power-sum floor
- (plus this doc-cleanup commit closing out Task 6: `PHYSICS_MODEL_VERSION` 5, openapi/api-docs/
  domain-contract/CLAUDE.md/roadmap re-trued)

All three redesign calls decided 2026-07-16 (below) were executed as decided: incoherent power
sum forward, Huygens obliquity factor on the far-field conversion, floor-only rear hemisphere
with rear PO integration skipped on uncorrected-physics antennas. `PHYSICS_MODEL_VERSION` bumped
to 5. The doc-truth cleanup flagged below (openapi.yaml + related docs) is this same task's scope.

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

**✅ REDESIGN DECIDED 2026-07-16 (maintainer) — all three open calls resolved at the
recommended options; the unit is now implementable:**

1. **Combination rule: incoherent POWER SUM** — `G = 10·log₁₀(10^(PO/10) + 10^(floor/10))`
   (not `max()`, not hard substitution). No θ_valid threshold parameter exists in the
   forward hemisphere; the floor takes over smoothly wherever idealised PO under-predicts.
2. **Add the Huygens obliquity factor `(1+cosθ)/2` to the far-field integrand** — the
   missing textbook element factor P10-tail identified (root cause of the fictitious
   converged rear backlobe; forward levels ~up-to-6 dB hot at 90°). Applies to BOTH the
   Hankel and azimuthal-mode paths (it is a θ-only multiplier outside the aperture
   integral, so it does not disturb the P10 quadrature). Re-derive the wide-angle anchors
   in `reference_validation.rs` (they are internal-consistency values, not measurements) and
   re-run the full P10 validation protocol; the θ=0 peak anchors must NOT move (factor = 1
   at boresight).
3. **Rear hemisphere (θ > 90°): floor-only** — exclude the PO term from the power-sum
   entirely behind the dish (aperture PO is categorically invalid there even with
   obliquity, and P10-tail measured a +7…+13 dBi converged-but-fictitious backlobe that
   would dominate the ≤0 dBi floor). The NTIA 84-164 calibration data spans 1°–180°, so the
   floor is data-backed in the rear hemisphere. P10-tail's rear-hemisphere warning stays
   (reworded from "value is an extrapolation artifact" to "statistical floor only").

Confirmed unchanged from earlier decisions: best-estimate **median** level (2026-07-12);
`Ω = 4π`, `floor = 1 − η_ruze`, 0 dBi bound; gate on P11's
`physics_is_uncorrected()` predicate; the boresight-tuner `surface_rms`→floor coupling must
be measured/bounded before shipping (precondition 2 below); `PHYSICS_MODEL_VERSION` bump
(coordinate with P2's bump); re-scope P10-perf after landing.

**Doc-truth cleanup owned by this unit (filed 2026-07-16 during P3 execution).** The three
`openapi.yaml` `warnings`-field descriptions still describe the F7 floor as *active* —
"off-axis sidelobe levels … now include a Ruze scatter-floor best estimate (tracks measured
median sidelobe statistics, not a precise per-antenna prediction)". That has been **stale since
D-2/P10 (2026-07-15)**: the served path carries **raw idealised PO with the floor OFF**, so the
current served off-axis value is *not* floor-blended. The warning *message strings* were already
re-trued at P10 (they now say "idealised PO … floor intentionally off"); only these schema-level
summaries lag. Locations to re-true (verify line numbers — openapi is hand-maintained, standing
rule 4):
  - `openapi.yaml` `GainResponse.warnings` description (~:636–639) — reused by `/gain` + `/gain/batch`.
  - `openapi.yaml` heatmap-response `warnings` description (~:1023–1024).
  - `openapi.yaml` h3-response `warnings` description (~:880–881).
When F7 lands (floor back on via the power-sum), rewrite these to describe **what F7 actually
serves** — a PO ⊕ statistical-floor power sum, best-estimate/median, gated on
`physics_is_uncorrected()` — rather than reverting to the old "best estimate" wording verbatim.
(`docs/api-documentation.md`'s off-axis caveat, ~:100–121, is already current — it states the
raw-PO/floor-OFF D-2 truth — so it needs only the floor-on update, not a stale-claim fix.) If F7
is deferred long-term, re-true these to the raw-PO/floor-OFF present tense as a standalone
doc-truth fix rather than leaving them wrong.

**✅ DONE 2026-07-16/17 (Task 6 of the F7 redesign).** All three `openapi.yaml` `warnings`
descriptions were rewritten to describe the power sum — idealised PO ⊕ statistical floor,
best-estimate median tracking NTIA 84-164, gated on `physics_is_uncorrected()` — and the
rear-hemisphere clause in each now distinguishes floor-only (uncorrected physics) from
numerical extrapolation (corrected physics). `docs/api-documentation.md`'s off-axis and
rear-hemisphere caveats were re-trued the same way. `docs/domain-contract.md`'s three-tier
rear policy and F7 handoff list, `CLAUDE.md`'s Project Status/module-map bullets, and this
roadmap's F7 register row + Risks section were all re-trued in the same pass. See the commit
listed in the DONE block above.

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
