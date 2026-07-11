# Roadmap — July 2026

Written after the whole-application review of 2026-07-08 (three-track audit: physics/model
layer, API/service layer, tooling/debt sweep), which followed the debt-paydown work on
`fix/review-findings-2026-07`. Companion document:
[`roadmap-2026-07-work-units.md`](roadmap-2026-07-work-units.md) — the prioritized,
agent-executable work-unit breakdown. This document is the narrative: where the
application stands, what we are optimizing for, and in what order.

All file:line references were verified against the code on 2026-07-08 (branch
`fix/review-findings-2026-07`, HEAD `d65f780`). Line numbers drift; re-verify before acting.

---

## 1. Current-state assessment

### Physics / model layer — healthy

The July review-fix branch closed the correctness findings from the 2026-07-02 review, and a
fresh contract-conformance pass (re-derived, not trusted) confirms the fixes hold:

- Beam-steering sign flip present and pinned (`coordinates.rs:222-223`, negated x/y;
  `tests/beam_steering_direction.rs`).
- Beam deviation factor applied (`coordinates.rs:220`).
- ENU→ECEF transpose used correctly in the heatmap path (`heatmap.rs:357-362`).
- Azimuth normalization on all az-producing paths (`coordinates_3d.rs:539,722,727,757`).
- Squint argument-order trap documented at the call site and pinned by test
  (`coordinates_3d.rs:785` + `test_squint_corrected_direction_frequency_argument_order`).
- All invariant tests listed in `docs/domain-contract.md` exist.
- `phase_center_offset` is now consumed by the live path as an axial defocus term
  (`integration.rs:516-517`) — the domain contract's open item on this is stale.

**The 4D B-spline correction surface is fully implemented and live**
(`model/correction_interpolator.rs`, applied at `service/evaluator.rs:265-287`, evaluated at
calibration `temperature_const`, knot vectors validated at load). CLAUDE.md's claim that
this is "not yet implemented in Sprint 5" is false — the project is roughly two sprints
ahead of its own primary onboarding doc.

What remains in the physics layer is **scope, not bugs**:

- The efficiency model is only Ruze × mesh (`pattern.rs:130-141`). Spillover is estimated
  solely to emit warnings (`edge_cases.rs:170`) and never reduces gain; feed/strut blockage
  and cross-polarization loss are not modeled anywhere. For calibrated antennas the fitted
  correction surface absorbs these losses empirically.
- The T in G/T is a pure user-supplied passthrough (`pattern.rs:512`,
  `h3_link_budget.rs:585`) — no antenna noise-temperature model.
- Feed offsets > 0.5·f route to an acknowledged ray-tracing stub
  (`pattern.rs:260-270`, `ray_trace.rs:336` TODO); results there carry an
  "accuracy may be degraded" warning.
- One unverified heuristic sits on the live path: the higher-order Seidel aberration terms
  (`edge_cases.rs:250`, consumed via `integration.rs:559-570`) use coefficient 1 with no
  citation.

**Addendum 2026-07-10 — reference-data validation.** A reference-validation harness now
exists and runs in CI (`antenna-model/tests/reference_validation.rs`: three active tests +
one `#[ignore]`d diagnostic), scoring the uncalibrated model against real published antennas
(DSN 34-m BWG, DSN 70-m, GBT 100-m). It has already paid for itself three times over:

- **Feed q-factor fix (landed):** the design-spec q-factors (8–11) were grossly over-tapered
  for this codebase's *field*-pattern convention, under-predicting DSN 34-m peak gain by
  ~5 dB; corrected to q≈1.1–3.1 for a ~−11 dB edge taper, pinned by
  `feed_taper_q_sweep_dsn_34m_xband`.
- **Ka phase-center defocus (diagnosed → unit P7):** the remaining −3.5 dB Ka residual is
  entirely `phase_center_offset_m` acting as a 1/λ-scaling axial defocus
  (`docs/findings-2026-07-10-ka-phase-center-defocus.md`). Register row P7 decided:
  model auto-refocus.
- **Off-axis sidelobe scope (documented → P8/F7):** the model is a main-beam instrument;
  its sidelobes fall at the correct ~25·log₁₀(θ) slope but ~8–13 dB below the ITU-R S.580
  mask, because all sidelobe-*raising* mechanisms (blockage, strut scatter, edge diffraction,
  surface-error scatter floor) are unmodeled. Recorded in the contract's "Off-axis pattern /
  sidelobe fidelity" section; honesty warning = unit P8, envelope model = gated feature F7.

### API / service layer — functional but operationally soft

Feature-wise this layer is far more complete than CLAUDE.md implies: batch, rectangular
heatmap, H3 link budget, antenna/feed listing, partial-calibration statuses, and
multi-feed support are all built and tested (712 tests workspace-wide; production paths are
clean of `unwrap`/`expect`/`panic`). The problems are of a different kind:

- **Configured protections are fictional.** `max_body_size_bytes` and
  `request_timeout_secs` exist in config (`config/settings.rs:42-48`) but are only logged at
  startup (`api/mod.rs:193-194`) — no middleware enforces either. The existing body-size
  test passes for the wrong reason (oversized body fails JSON parse, not size rejection).
  `performance.worker_threads` is likewise dead config: all parallel paths use rayon's
  global pool. There is no admission control.
- **A shipped endpoint is invisible.** `/api/v1/h3-heatmap` is implemented and has 9
  integration tests, but has no path entry in `openapi.yaml` (its schemas are orphaned at
  `openapi.yaml:750,822`) and no mention in `docs/api-documentation.md`.
- **Validation is uneven.** The H3 link-budget validator (`validator.rs:203-226`) skips
  `temperature_k` (a negative value reaches `t.log10()` → NaN in the response),
  `pointing_frequency_mhz` (validated on gain/heatmap at `validator.rs:96,182` but not
  here), and `h3_resolution` (caught late by `h3o::Resolution::try_from` at
  `h3_link_budget.rs:273` instead of the validator).
- **The error contract is inconsistent.** Validation failures return 422 from the
  pre-check path but 400 from the service layer; handlers emit snake_case error codes while
  a dead set of PascalCase constructors lives in `schemas.rs:~1085-1110`; error bodies are
  serialized JSON served as `text/plain`.
- **Coordinate-ambiguity warnings are gain-only.** The GEO auto-detection trap (geodetic
  altitudes above 6400 km silently misparse as ECEF) produces warnings only on the gain
  endpoint (`evaluator.rs:105`); heatmap and h3-heatmap accept ambiguous input silently.
- **Lifecycle is shallow.** Readiness defaults to `true` at construction (`api/mod.rs:72`);
  a total calibration-load failure yields a healthy, empty server regardless of `fail_fast`
  (`api/mod.rs:180-186`); `shutdown_cleanup()` is a no-op that nothing invokes
  (`api/mod.rs:301-316`).
- `/heatmap`'s H3 grid type is a `NotImplemented` stub (`heatmap.rs:168-171,215-218`) while
  the real H3 implementation lives at the separate `/h3-heatmap` endpoint.

### Tooling / docs — the largest debt area

- **No CI of any kind** — no workflow files, and the repository currently has **no git
  remote** — despite CLAUDE.md prescribing `clippy -D warnings`, `cargo audit`, and a test
  quality bar. Nothing enforces any of it.
- **Docs actively mislead.** CLAUDE.md misstates the sprint status and the correction-surface
  implementation state, references three deleted modules (`direct_path.rs`, `surface.rs`,
  `numerical_stability.rs`), and names `antennas.toml` where the file is `antennas.yaml`.
  `architecture.md` lists model files that do not exist. The design doc still describes an
  unimplemented per-point Zernike model and a removed direct-path interference mode.
  `review-findings-2026-06-10.md` reads as if all findings were still open.
- **Broken examples.** Four request examples fail deserialization: `gain_request.json`,
  `batch_request.json`, `heatmap_request.json` use a `{"w":…}` object for
  `vehicle_attitude`, and `gain_request_geodetic.json` uses Euler angles; the schema
  requires a `[w,x,y,z]` array (`schemas.rs:276`).
- **Structural debt.** `calibrate` depends on the entire `antenna-model` crate, so building
  the CLI compiles poem/h3o/the whole web stack; `ndarray` 0.15 and 0.16 are both in the
  tree; the deprecated 612-line `calibrate/src/serializer.rs` writes binary artifacts the
  service cannot load (honestly marked deprecated, but still exported via `lib.rs:57`);
  the artifact format has two unrelated version axes (ANTC header `u32` = 1 vs
  `metadata.format_version` string "2.0", `loader.rs:165`); a 3.1 MB `tarpaulin-report.html`
  is committed at the repo root; no property-based tests exist despite CLAUDE.md claiming
  them; no `.bin` calibration artifacts exist anywhere (the four `antennas.yaml` entries
  that reference a `.bin` calibration file are `enabled: false`; the four uncalibrated
  design-spec antennas are `enabled: true` and load without a `.bin`) although CLAUDE.md
  claims precomputed artifacts ship. **Correction (2026-07-09): earlier drafts of this
  document said "all `antennas.yaml` entries are `enabled: false`" — that is false; 4 of 8
  are enabled. Units D9, S5, and P1b were written against the wrong premise — see their
  updated notes.**

---

## 2. Guiding principles

1. **Ordering rule:** prediction correctness → safety/operational correctness → API
   contract quality → structure/debt → new features.
2. **One justified inversion:** CI and doc-truth guardrails (Phase 0) run *first*, even
   though they are categorically "tooling." Every later unit will be executed by coding
   agents that (a) need a regression net and (b) would currently be misled by CLAUDE.md.
   This is risk reduction, not a priority inversion.
3. **No silent physics changes.** Any contract-vs-code disagreement or scope ambiguity
   becomes an explicit decision unit with a plain-language question and a recommended
   default. The maintainer is deliberately not a domain expert in this physics; ambiguity
   resolution is theirs.
4. **Break once, then freeze.** The maintainer confirmed on 2026-07-08 that nothing
   consumes this API yet (pre-production: no remote, no shipped `.bin` artifacts, only
   uncalibrated design-spec antennas enabled). Breaking changes are therefore cheapest *now* and get progressively more
   expensive from the first integration onward. The roadmap concentrates every desirable
   breaking change into one consolidated pass (unit **C8 — v1 contract finalization**),
   lands the openapi drift guard (C7) immediately after, and treats the contract as frozen
   from that point. Anything breaking proposed after C8 needs a real v2 justification.

## 3. Themes

| Theme | What it means here |
|---|---|
| **T1 — Trustworthy predictions & explicit scope** | Know exactly what the model does and does not claim: fence the unverified aberration heuristic, model spillover on the uncalibrated path (P1 decision), document the remaining unmodeled terms, fail loudly on out-of-range geometry. 2026-07-10: also fix the phase-center defocus semantics (P7) and warn honestly on off-axis queries the model cannot answer (P8). |
| **T2 — Operational hardening** | Every knob in config either works or is removed: body-size limit, timeout, worker threads, admission control, readiness lifecycle. |
| **T3 — Contract fidelity** | docs = code = behavior: document the hidden endpoint, one error vocabulary, JSON error bodies, consistent status codes, a drift guard so openapi.yaml cannot silently rot again. |
| **T4 — Maintainability & drift prevention** | CI, truthful CLAUDE.md/architecture docs, crate split so the CLI stops compiling the web stack, property tests that make the claimed testing philosophy real. |
| **T5 — Decision-gated capability growth** | Hot-reload, real ray tracing, physical efficiency terms, noise-temperature modeling, statistical sidelobe envelope (F7) — each blocked on an explicit maintainer decision recorded in the register. |

## 4. Phases

| Phase | Goal | Exit criteria |
|---|---|---|
| **0 — Guardrails** ✅ **DONE 2026-07-09** | Regression net + truthful onboarding docs before anything else. | ✅ CI committed, live & green on `main` (github.com/blstoll/antenna-model); CLAUDE.md true; all examples deserialize under a drift test. Commits G1 `f48b23c`, G2 `8c65946`, G3 `c2dceee` (+ CI hardening `c13e196`/`4b439c0`, deps `bf18d60`). |
| **1 — Prediction correctness & physics scope** | No unexplained numbers on the live path; scope decisions recorded. | P1–P3 decisions in the register; spillover applied on the uncalibrated path with calibrated outputs unchanged (P1) and artifacts stamped with a physics-model version (P1b); f/D fails loudly; single G/T implementation; domain-contract open items current. |
| **2 — Safety & operational correctness** | Config promises kept; bounded work; honest lifecycle. | Oversized → 413; slow → timeout; integration has a wall-clock budget; concurrency capped; readiness/fail_fast/shutdown real; H3 validator complete. (Coordinate-ambiguity handling moved to C8, which removes the ambiguity instead of warning about it.) |
| **3 — API contract quality** | A client can trust the spec and the error contract — finalized once, then frozen. | One error vocabulary, JSON bodies, one status-code policy (C2–C4); **C8 contract finalization landed**: `feed_position` renamed, `coordinate_system` required, typed warnings, coherent heatmap endpoints, `/h3-heatmap` documented; openapi drift guard (C7) in CI freezing the result. |
| **4 — Structure, debt, docs** | The codebase stops accumulating the debt classes found in this review. | Legacy serializer gone; version axes documented+validated; 3D→4D bridge round-trip-tested; crate split done; design docs truthful; property tests in CI. |
| **5 — Decision-gated features** | New capability, only where the register says go. | Per-feature; see work units F1–F6. |

## 5. Decision register

Work in Phase 5 (and the flagged units below) does not start until its row is **Decided**.
Defaults are recommendations; the maintainer decides.

| ID | Question | Options | Recommended default | Status | Decided by / date |
|----|----------|---------|---------------------|--------|-------------------|
| G1-hosting | Where will this repo live? (No remote configured today.) | GitHub / other forge / local-only | GitHub — repo created at github.com/blstoll/antenna-model; CI committed and live (green on `main` 2026-07-09). | **Decided** | Maintainer, 2026-07-08 |
| P1 | Model spillover / blockage / cross-pol physically? | Implement / document-as-scope / staged | **Staged implement**: spillover promoted into the gain path for antennas *without* a correction surface (double-counting gated, see unit P1); blockage = F3 (data-gated); cross-pol out of scope. Rationale: many antenna systems are expected to lack calibration data, and the unmodeled spillover bias (~0.4–1 dB) alone can consume the <1 dB accuracy budget on the uncalibrated path. **FINDING 2026-07-09 (during P1 execution):** for the four *currently enabled* design-spec antennas (q=8–11, f/D=0.4–0.5 — all highly over-tapered) the code's existing `estimate_spillover` yields only ~0.001–0.05 dB, NOT 0.4–1 dB. The 0.4–1 dB premise is a broad-feed (q≈2–4) figure; it does not hold for these directive designs. Maintainer confirmed 2026-07-09: proceed anyway — the mechanism is correct, cheap, and future-proofs broad-feed antennas; impact on current configs is negligible and documented honestly. **REVISED 2026-07-10:** the 07-09 "negligible" finding was itself an artifact of the over-tapered q-factors. After the reference-validation feed-taper fix (q≈1.1–3.1 for a ~−11 dB edge taper), spillover is **material: ~0.8 dB** — the original 0.4–1 dB premise was right after all. A fractional-q truncation in `estimate_spillover` (`powi` → `powf`) was also fixed. See `docs/domain-contract.md` "Magnitude reality". | **Decided** | Maintainer, 2026-07-08; findings 2026-07-09, 2026-07-10 |
| P2 | Unverified Seidel higher-order coefficients on the live path | Verify vs literature / fence with warning / remove | Fence: annotate + warn when contribution > 0.1 dB; seek citation | Open | — |
| P7 | `phase_center_offset_m` semantics (root cause of the Ka-band under-prediction — the field acts as a 1/λ-scaling axial defocus, `docs/findings-2026-07-10-ka-phase-center-defocus.md`) | Config realism (redefine as residual-after-focus, set ≈0 by convention) / model auto-refocus (raw feed property, model compensates) | **Auto-refocus.** Correctness over blast radius: config-realism leaves a standing trap where entering a datasheet phase-center value silently costs multi-dB at Ka; auto-refocus matches how real antennas are operated (refocused per band) and is correct per-band by mechanism, not convention. Deliberate defocus moves to a new explicit field. Cheap to change now — no `.bin` artifacts in the wild; bumps `physics_model_version` (P1b). See unit P7. **IMPLEMENTED 2026-07-10** (branch `feat/p7-phase-center-auto-refocus`, commits `1746bc0`, `ba87160`, `a31c512`, `6c2e1a8`, `10c8204`). | **Decided** | Maintainer, 2026-07-10 |
| F7 | Statistical off-axis sidelobe envelope/floor model? (The physics model's sidelobes are systematically ~8–13 dB optimistic — see contract "Off-axis pattern / sidelobe fidelity".) | Implement envelope/floor model / docs + warning only / full physical modeling | Envelope/floor model, **data-gated**: requires reference sidelobe data (the ITU S.580 harness test validates pattern *shape* only — levels need real data). Near-term honesty warning is NOT gated (unit P8, decided with this row's filing). Physical edge-diffraction/strut modeling is out of scope regardless (§6). | Open (P8 warning decided) | Maintainer filed 2026-07-10 |
| P3 | Ray-trace stub for feed offsets > 0.5·f | Implement (F2) / reject requests / document + flag | Document + flag on all endpoints | Open | — |
| P5/F4 | Model antenna noise temperature in G/T? | Model / keep user-supplied passthrough | Keep passthrough; document scope | Open | — |
| S7 | GEO coordinate-ambiguity policy | Warn everywhere / reject ambiguous / remove ambiguity | **Superseded by C8**: `coordinate_system` becomes required, eliminating auto-detection ambiguity entirely (better than warning about it). | **Decided** | Maintainer, 2026-07-08 |
| C2 | HTTP status policy for validation failures | Unify on 400 / unify on 422-semantic | 400 = malformed body; 422 = semantically invalid; everywhere | Open | — |
| C5 | `/heatmap` H3 grid-type stub | Remove variant / implement (F5) / keep stub | **Superseded by C8** (stage 4 removes the variant; full merge remains F5). | **Decided** | Maintainer, 2026-07-08 |
| C6 | `feed_position` naming trap | Rename now (breaking) / docs-only in v1 | **Superseded by C8** (stage 1 renames to `feed_pointing_location` now — pre-production confirmed, no consumers to break). | **Decided** | Maintainer, 2026-07-08 |
| C8 | Rework the API contract before first integration? | Full redesign / consolidated breaking pass / keep v1 stable | **One consolidated breaking pass** (rename `feed_position`, require `coordinate_system`, typed warnings, coherent heatmap endpoints), then freeze via C7. Full redesign rejected (no efficiency case — physics dominates latency); keep-stable rejected (pre-production, breaking cost ≈ 0 today). | **Decided** | Maintainer, 2026-07-08 |
| D4 | Extract a shared `antenna-core` crate? | Split / keep two-crate layout | Split (mechanical move, after Phases 1–3) | Open | — |
| D9 | Ship calibration `.bin` artifacts in-repo? | Commit binaries / generate in CI / docs-only | No binaries; document + script the generation path | Open | — |

## 6. Non-goals

Unless a decision-register row flips them:

- Full physical-optics ray tracing for large feed offsets (F2 exists as a gated option).
- Antenna noise-temperature / sky-noise modeling behind G/T (F4).
- Any breaking API change after C8 lands (the C8 pass is the one sanctioned break; the
  contract is frozen behind the C7 drift guard afterward).
- Batch shared-context request shape (each batch item currently repeats full
  vehicle/antenna context — redundant but harmless; can be added *non-breaking* later via
  optional top-level defaults that items inherit).
- Committing binary calibration artifacts to the repository.
- Migrating to poem-openapi codegen (noted as a possible future item under C7; the drift
  guard is the v1 answer).
- Physical sidelobe mechanisms — aperture-edge diffraction and quadripod strut scatter.
  These are domain-expert territory (same class as F2 ray tracing). Feature F7, if its row
  flips, covers only a *statistical* envelope/floor; the physical mechanisms stay out of
  scope regardless.

## 7. Risks

- **openapi.yaml is hand-maintained** and will keep drifting until unit C7's guard lands;
  every schema-touching unit before that must mirror changes manually (standing rule 4 in
  the work-units doc).
- **Shared rayon global pool** couples batch, heatmap, and H3 load until S4; concurrent
  heavy requests contend unboundedly today.
- **One unverified physics heuristic** (Seidel terms) remains on the live path until P2 is
  executed; predictions at moderate feed offsets carry uncited aberration contributions.
- **No remote / no CI** means every quality gate is manual until G1 lands and the repo is
  pushed somewhere; until then, regressions are caught only by whoever remembers to run
  `cargo test --workspace`.
- **Decision latency**: five of the six feature units are decision-gated; if the register
  sits undecided, Phase 5 stalls by design. That is intentional but worth stating.
- ~~**Loose Ka reference tolerance until P7 lands**: the DSN 34-m Ka row in the reference
  harness carries a deliberate 5.0 dB tolerance (masking the known phase-center defocus),
  so the harness cannot catch Ka-band regressions smaller than that until P7 tightens it
  to ~1.5 dB.~~ **Resolved 2026-07-10 by P7**: Ka tolerance tightened 5.0 → 1.5 dB (X also
  tightened 1.5 → 1.0 dB) in `dsn_34m_bwg.psv`; measured residuals now +0.01 dB (Ka) /
  +0.17 dB (X), comfortably inside both tolerances.
