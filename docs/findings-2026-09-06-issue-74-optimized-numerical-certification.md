# Issue #74 — running the heavy numerical regressions in an optimized build

**Date:** 2026-09-06
**Scope:** how the retained heavy numerical regressions are compiled and routed, not what
they assert. No test body, threshold, anchor, density or tolerance was touched.

**Result:** `antenna-core` is now compiled at `opt-level = 3` in the debug and test builds.
The complete `--profile full` suite went **110.5 s → 17.7 s wall and 770.7 s → 81.3 s CPU** for the same 1122 tests;
the dev inner loop went **20.4 s → 8.3 s wall, 131.3 s → 33.6 s CPU** while gaining two tests; both heavy physics
pins rejoined the dev loop (see §2). The cost is one crate compiling more slowly: **2.54 s → 7.38 s**
wall to build `antenna-core` from clean. `./scripts/check.sh`, the complete gate, went
**156.2 s → 36.5 s wall and 822.5 s → 100.3 s CPU** (both sides recompiling the crate first).
A separate optimized test lane was built first,
measured, and **rejected** — the numbers are in §3, and they are why the shipped change is
one Cargo profile override rather than a second build.

All figures: 8-core M-series laptop (4 performance + 4 efficiency), warm build unless said
otherwise, `cargo-nextest 0.9.133`, `rustc 1.96.1`.

## 1. What the tests cost, and why

The three regressions issue #74 names spend essentially all of their time inside
`antenna_core::model`'s aperture integration:

| Test | debug (before) | optimized (after) |
|---|---|---|
| `mode_path_reports_a_radial_error_even_when_the_density_cap_binds` | 35.99 s | 2.12 s |
| `p12_phi_cap_removed_steered_feed_matches_stored_anchors` | 7.44 s | 0.43 s |
| `p12_mode_path_radial_convergence_anchors` | 0.088 s | 0.016 s |

Sequential (`--test-threads 1`), same runner both sides. The first is the real-cap
certification roadmap P13 added and issue #72 kept: it must evaluate ~3 × 65,537 radial
samples, because reaching the cap is what it asserts. The second holds three stored
field-level anchors across a steered lobe. The third was 14.52 s until issue #71 replaced
its runtime dense references with fixed-axis anchors — it was already cheap.

The same property holds for the four `calibrate` full-mode scenarios, which are physics
sweeps behind a CLI:

| Test | before | after (sequential) | after (in a concurrent full run) |
|---|---|---|---|
| `cli_tuned_run_recovers_the_surface_rms_perturbation` | 85.9 s | 3.96 s | 6.21 s |
| `cli_cv_three_folds_reports_finite_scores` | ~10 s | 3.78 s | 6.70 s |
| `cli_untuned_scenario_preserves_artifact_correction_and_no_validation` | ~11 s | 1.06 s | 2.99 s |
| `real_data_scenario_preserves_generator_cli_artifact_and_service_contract` | 7.6 s | 6.43 s | 9.61 s |

## 2. What shipped

```toml
[profile.dev.package.antenna-core]
opt-level = 3
```

One block, not two: `profile.test` inherits `profile.dev` including package overrides, so
this covers `antenna-core` both as a library dependency of `antenna-model`/`calibrate` and
as its own unit-test binary. A `[profile.test.package.antenna-core]` twin was written first
on the assumption that it was needed, and deleted after `cargo build -p antenna-core
--profile test -v` reported `-C opt-level=3` without it.

Only `opt-level` is overridden, so **`debug-assertions` and `overflow-checks` stay on** —
verified directly with `cargo build -p antenna-core --profile test -v`, which shows
`-C opt-level=3 -C debug-assertions=on`. The certification runs with the same checks it ran
with before, and the same 1122 tests pass.

Consequences, all of them measured:

| | before | after |
|---|---|---|
| `--profile full`, warm, wall / CPU (1122 tests) | 110.5 s / 770.7 s | 17.7 s / 81.3 s |
| default dev loop, warm, wall / CPU | 20.4 s / 131.3 s (1116 tests) | 8.3 s / 33.6 s (1118) |
| edit a core file, then run `--profile full` | 138.1 s | 43.9 s |
| `./scripts/check.sh`, the complete gate, wall / CPU | 156.2 s / 822.5 s | 36.5 s / 100.3 s |
| **cold cache** (`cargo clean`, then build + `--profile full`) | 159.0 s / 1007.5 s | 74.3 s / 351.8 s |
| build `antenna-core` from clean (`--tests`) | 2.54 s / 6.12 s CPU | 7.38 s / 33.18 s CPU |

The cold-cache row is the one that answers "does the extra compilation eat the win on a
fresh runner": it does not. A `cargo clean` followed by a full build and the complete
`--profile full` suite costs **159.0 s → 74.3 s wall, 1007.5 s → 351.8 s CPU**. Nothing about
the dependency tree changes — it is compiled unoptimized on both sides, so the only unit that
costs more is `antenna-core` itself, and one crate's extra codegen is far smaller than the
test time it removes. CI's `Swatinem/rust-cache` configuration is untouched and needs no
change: there is still exactly one target directory and one profile in play.

The `antenna-core` row is the entire compilation trade-off: **+4.8 s wall, +27 s CPU per
`antenna-core` recompile**, against ~92 s of wall and ~690 s of CPU returned on every full
run. It is paid on a cold build and whenever `antenna-core` itself changes, and CI pays it
once per job either way.

Both heavy physics pins therefore left the slow tier — a change to the dev loop's contents
that issue #74 did not ask for, taken because D18's own rule ("a new test costing >10 s
either gets faster or joins the slow tier") points that way once they cost 2.12 s and 0.43 s,
and because it is what makes the certification run in the tier developers actually use. It is
reversible without losing the CI-blocking half: move the line below the `[full-only]` marker
in the manifest: at 2.12 s and 0.43 s they are far
under D18's 10 s policy line, so the dev inner loop now runs the numerical certification
that CI runs. The four `calibrate` scenarios stay excluded — they are under the line
sequentially but reach 9.61 s under contention, and returning them would make `default` and
`full` select the same 1122 tests, which is a decision about whether the two-tier split
still earns its keep rather than a consequence of this issue.

`scripts/assert-numerical-certification.sh` (run by `scripts/check.sh` and CI) asserts what
no test result can show:

* every test in `.config/numerical-certification-manifest.txt` is selected by `--profile
  full` **and** by the default profile — a rename, a move between binaries, or an edited
  filter otherwise leaves a green suite with less in it;
* `antenna-core` still carries the `opt-level` override — deleting it costs no correctness
  and ~92 s of wall per run, and every test still passes.

Each half runs behind a live control (a name that must NOT be found; a profile key that must
be readable and one that must be absent), because a guard whose power nothing asserts is the
rot roadmap P13 records. There is no wall-clock assertion anywhere in it: timing thresholds
in this repo have a history — the sustained-load test flaked twice before issue #73 deleted
it — and "this test is in this profile, this crate is built at this opt-level" states the
same property without a fitted constant.

## 3. The design that was built first, measured, and rejected

The obvious reading of #74 is a second lane: a `[profile.numeric]` Cargo profile, a
`profile.numeric` nextest profile selecting the two expensive tests by exact name, those two
excluded from `profile.full` so nothing runs twice, a manifest audit around the selection,
and both `scripts/check.sh` and CI calling one script that owns the invocation. That was
built and it worked — the two tests ran in 2.44 s instead of 43.53 s, and four mutations
(bogus manifest entry, renamed test, exclusion removed from `profile.full`, `--cargo-profile`
dropped) each failed the gate as intended.

It was rejected on its total-cycle numbers:

| cycle | before | two-lane design |
|---|---|---|
| full check, warm, nothing changed | 110.5 s wall / 770.7 s CPU | 105.0 s / 709.6 s |
| full check after editing a core file | 138.1 s wall / 798.5 s CPU | **151.0 s / 862.7 s** |
| first run on a cold cache | — | + 65 s (optimized dependency tree) |

The lane needs its own optimized build of `antenna-core`, `antenna-model` and their
dependencies. Rebuilding those two crates optimized costs ~24 s wall / ~129 s CPU, while the
debug suite only gives back ~14 s of wall for the tests removed from it — the 36 s test was
not on the critical path, since `calibrate`'s tuned scenario was. So on the cycle that
matters most, edit-then-check, the lane was **13 s slower in wall and 64 s more CPU**, and a
cold CI cache paid another minute. A 2-core runner does not rescue it: the CPU column is the
one that maps to wall time there, and it moves the wrong way.

The profile override wins because it optimizes the code once, in the build everything
already uses, and every test that touches the physics benefits — including the `calibrate`
scenarios, which the lane could not have helped at all.

Two smaller options were measured and not taken:

* **`--cargo-profile release` for the lane** (`lto = true`, `codegen-units = 1`): at
  opt-level 2/cu 16, 3/cu 16 and 3/cu 256 the cap test ran in 2.12 / 2.03 / 2.02 s and the
  rebuild took 14.5 / 13.4 / 13.6 s. The curve is flat; LTO buys ~0.1 s for a much slower
  build.
* **`[profile.dev.package."*"] opt-level`**, optimizing the dependency tree too: the cap test
  is 2.12 s with only `antenna-core` optimized against 2.03 s in a fully optimized build, so
  ~0.1 s of a ~18 s suite, in exchange for every cold CI build recompiling every dependency.

Also considered and not taken: turning `debug-assertions` off in the certification build. It
buys 0.32 s on the cap test (1.71 s vs 2.03 s) and would mean the numerical certification
runs with weaker checks than the tier it replaced.

## 4. What was not verified

**CI behaviour is reasoned, not observed.** Every figure here is from the reference laptop;
the branch has not been pushed, so no CI run exists for this change. The reasoning is that CI
compiles one profile in one target directory either way, pays `antenna-core`'s extra codegen
once per job, and gets the same test-time reduction — but the runner has 2-4 cores against
this machine's 8, and the CPU columns above are the ones that map to wall time there. Confirm
on the first CI run and record the numbers here, the way the P10-perf note in
`.config/nextest.toml` confirmed its prediction against PR #49.

## 5. What to watch

* **The `10 s` policy line now sits in a much faster suite.** A test that costs 10 s today is
  doing ~6x the work one costing 10 s did last week. The line was not re-tuned here; that is
  a D18 question, along with whether `default` and `full` should stay distinct at all.
* **Do not compare a figure across 2026-09-06 without saying which side it is from.** Every
  pre-#74 measurement in `.config/nextest.toml` was taken against an unoptimized core, and
  the ones for physics-bound tests are 3-20x too high for today's build. The file now says so
  at the top; the D18 caveat about standalone vs contended figures still applies on top of it.
* **Optimized code is harder to step through.** If a debugger session on core internals needs
  it, drop the opt-level locally — the assertion script will fail the gate if it goes missing
  from the committed manifest, which is the intended asymmetry.
