# Issue #84 — whether the default/full nextest split still earns its keep

**Date:** 2026-09-08  
**Scope:** the four tests still excluded from the default nextest profile after issue #74.
No test, threshold, fixture, or CI command was changed. The follow-up linked from #74 is
issue **#84**; #75 in this repository is a merged pull request for issue #60.

## Recommendation

**Retain the split.** The selection difference is only four tests (1118 versus 1122), but it
is not operationally negligible: on the reference M1 those four approximately **double the
warm test-loop wall time** (8.17 s → 17.62 s mean) and add about **49 s of CPU**. On the
latest constrained GitHub-hosted runner trace they occupy the critical tail for roughly
**25 s**. The split therefore still does the job D18 created it for: it removes a small, concentrated,
contention-sensitive tail from the command developers run repeatedly while leaving that tail
mandatory in `check.sh` and CI.

Do not read this as a CI optimization. CI already invokes `--profile full`; retaining or
collapsing the profiles changes **zero CI test selection and zero CI test time**. The runner
trace is evidence about constrained-machine behaviour, not a saving available to the current
workflow.

## 1. Current selection and local cost

At commit `64c0892` (Apple M1, 8 logical CPUs, rustc 1.96.1, nextest 0.9.133, warmed build),
three alternating runs measured:

| profile | selected | nextest summaries | mean | process real | user + sys |
|---|---:|---|---:|---|---:|
| default | 1118 | 8.616 / 7.905 / 8.002 s | **8.174 s** | 9.11 / 8.50 / 8.54 s | ~40.0 s |
| full | 1122 | 17.681 / 17.629 / 17.550 s | **17.620 s** | 18.20 / 18.52 / 18.04 s | ~89.3 s |

Thus the four-test tail adds **9.45 s by nextest's summary, ~9.54 s process wall, and ~49 s
CPU**. Keeping it out cuts the repeated local loop by about 54%; this is a meaningful benefit
even though both profiles are comfortably below D18's 90 s ceiling.

The only selection difference is the four exact-name clauses in
`.config/nextest.toml:232-241`:

| excluded scenario | concurrent full-run range |
|---|---:|
| untuned synthetic | 3.129–3.189 s |
| tuned synthetic recovery | 6.553–6.685 s |
| three-fold CV | 7.088–7.172 s |
| real-data contract | **9.886–9.926 s** |

Every individual test remains below the default profile's 10 s marker locally, but the
real-data scenario has essentially no margin and the four are concurrent CPU-heavy CLI
physics workloads. D18 already records that a per-test rule is insufficient when nextest's
process-per-test model makes an e2e binary expensive in aggregate
(`docs/roadmap-2026-07-work-units.md:5563-5581`). Issue #74's earlier sequential and
contended figures made the same caution explicit (`.config/nextest.toml:280-297` and
`docs/findings-2026-09-06-issue-74-optimized-numerical-certification.md:37-45,90-99`).
The new alternating runs confirm that the aggregate cost remains large after optimization.

## 2. Constrained-runner evidence

The merged-main run for `64c0892` reported **43.309 s for all 1122 tests**. The four excluded
scenarios took 8.677, 17.600, 24.549, and 24.949 s under the runner's two-core contention.
The event timestamps indicate that a default-only schedule would have exhausted the other
1118 tests at roughly 18.3 s; no default run was made on that runner, so this is a trace-based
estimate rather than a benchmark. The observed full run was held open to 43.3 s by the four,
indicating roughly a **25 s contention-sensitive tail**.

This does not slow CI *because of the split*: `.github/workflows/ci.yml:81-91` and
`scripts/check.sh:49-61` deliberately run `--profile full`. In the same Actions run the
`clippy + test` job was 120 s. The workflow's approximately 3m20 critical path was instead the
separate 196 s non-blocking cargo-audit job, including about 2m54 installing cargo-audit.
Collapsing the test profiles would not improve either figure.

Primary run: [GitHub Actions run 34175775727](https://github.com/blstoll/antenna-model/actions/runs/34175775727),
[`clippy + test` job 101904754567](https://github.com/blstoll/antenna-model/actions/runs/34175775727/job/101904754567).
The log's `Test` step contains the 43.309 s summary and the four per-test durations; run
metadata records the 120 s `clippy + test` and 196 s audit jobs.

## 3. Cost of retaining the mechanism

The ongoing cost is real but small and localized:

* `.config/nextest.toml:232-241,300-302` owns four exact-name exclusions plus `full = all()`;
  a rename fails safe by returning a test to the developer loop rather than losing CI
  coverage (the D18 design at `docs/roadmap-2026-07-work-units.md:5505-5516`).
* Humans must remember that bare nextest omits four scenarios and use `--profile full` for a
  complete run. The distinction is stated at both mandatory entrypoints
  (`scripts/check.sh:57-61`, `.github/workflows/ci.yml:87-91`).
* The numerical-certification audit checks both profile listings
  (`scripts/assert-numerical-certification.sh:1-29,65-130`). That is some structural
  complexity, but it executes no test twice; the three manifest certifications are in both
  tiers, while the four exclusions are separate calibration scenarios.
* Separate profiles preserve useful timeout semantics: default marks a test slow at 10 s,
  while full uses 60 s (`.config/nextest.toml:34-36,300-302`). Collapsing onto the default
  settings would make three of these four routinely emit slow markers on the observed GitHub
  runner, or would require weakening the review signal D18 established.

Removal would simplify commands, comments, the profile-membership audit, and developer
expectations. It would also ensure every bare local run covers the four calibration contracts.
Those are the best arguments for collapsing. They do not outweigh a repeatable 9.5 s wall /
49 s CPU local cost and a much larger constrained-runner tail, especially because CI already
provides mandatory coverage.

## 4. Risks and revisit condition

* **Counterargument — all four satisfy the literal 10 s local rule.** True, narrowly; the
  real-data case is only 0.07–0.11 s below it. D18's standing policy also says to measure under
  contention and records that per-test timing can hide aggregate process cost. Selection
  benefit, not the threshold alone, should decide whether the mechanism has work left to do.
* **Counterargument — 17.6 s is already an excellent full loop.** Also true. The question is
  comparative: paying more than twice the CPU and about twice the wall on every inner-loop run
  buys four expensive end-to-end scenarios that the mandatory full gate will run anyway.
* **Risk — developers may defer the full gate.** Keep the current `check.sh`/CI wiring and
  profile comments. Do not describe bare `cargo nextest run` as complete coverage.
* **Risk — exact-name filters age.** Their failure direction is safe: renamed tests rejoin
  default. Review timing when changing either affected test binary.

Revisit removal when representative alternating runs show that the four no longer form a
material wall/CPU tail on both an ordinary development machine and a constrained runner, or
when a change makes the default/full selections identical for another reason. Avoid a timing
assertion in CI; issue #74 intentionally uses structural selection checks because fitted wall
thresholds have been flaky (`scripts/assert-numerical-certification.sh:24-29`).

## Sources

* [Issue #84](https://github.com/blstoll/antenna-model/issues/84) — the open decision; its
  body contains no requirements beyond the title.
* [Issue #74](https://github.com/blstoll/antenna-model/issues/74) and its
  [resolution comment](https://github.com/blstoll/antenna-model/issues/74#issuecomment-5577559437)
  — optimization acceptance criteria, shipped design, and the explicit #84 follow-up.
* `.config/nextest.toml:34-36,232-241,280-302` — timeout policy, exact selection, and post-#74
  sequential/contended measurements.
* `Cargo.toml:21-66` — the single optimized-core profile and its measured build/test trade-off.
* `docs/roadmap-2026-07-work-units.md:5505-5516,5563-5581` — D18's fail-safe profile design,
  90 s/10 s policy, and process-per-test policy gap.
* `docs/findings-2026-09-06-issue-74-optimized-numerical-certification.md:7-18,37-45,68-99,161-197`
  — before/after results, why the four stayed excluded, real-runner result, and explicit
  instruction to revisit the split.
* `scripts/check.sh:49-61`, `.github/workflows/ci.yml:49-91`, and
  `scripts/assert-numerical-certification.sh:1-29,65-130` — mandatory full coverage and the
  structural profile-selection gate.
