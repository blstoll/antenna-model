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
| **T1 — Trustworthy predictions & explicit scope** | Know exactly what the model does and does not claim: fence the unverified aberration heuristic, model spillover on the uncalibrated path (P1 decision), document the remaining unmodeled terms, fail loudly on out-of-range geometry. |
| **T2 — Operational hardening** | Every knob in config either works or is removed: body-size limit, timeout, worker threads, admission control, readiness lifecycle. |
| **T3 — Contract fidelity** | docs = code = behavior: document the hidden endpoint, one error vocabulary, JSON error bodies, consistent status codes, a drift guard so openapi.yaml cannot silently rot again. |
| **T4 — Maintainability & drift prevention** | CI, truthful CLAUDE.md/architecture docs, crate split so the CLI stops compiling the web stack, property tests that make the claimed testing philosophy real. |
| **T5 — Decision-gated capability growth** | Hot-reload, real ray tracing, physical efficiency terms, noise-temperature modeling — each blocked on an explicit maintainer decision recorded in the register. |

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
| P1 | Model spillover / blockage / cross-pol physically? | Implement / document-as-scope / staged | **Staged implement**: spillover promoted into the gain path for antennas *without* a correction surface (double-counting gated, see unit P1); blockage = F3 (data-gated); cross-pol out of scope. Rationale: many antenna systems are expected to lack calibration data, and the unmodeled spillover bias (~0.4–1 dB) alone can consume the <1 dB accuracy budget on the uncalibrated path. **FINDING 2026-07-09 (during P1 execution):** for the four *currently enabled* design-spec antennas (q=8–11, f/D=0.4–0.5 — all highly over-tapered) the code's existing `estimate_spillover` yields only ~0.001–0.05 dB, NOT 0.4–1 dB. The 0.4–1 dB premise is a broad-feed (q≈2–4) figure; it does not hold for these directive designs. Maintainer confirmed 2026-07-09: proceed anyway — the mechanism is correct, cheap, and future-proofs broad-feed antennas; impact on current configs is negligible and documented honestly. | **Decided** | Maintainer, 2026-07-08; finding 2026-07-09 |
| P2 | Unverified Seidel higher-order coefficients on the live path | Verify vs literature / fence with warning / remove | Fence: annotate + warn when contribution > 0.1 dB; seek citation | Open | — |
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
