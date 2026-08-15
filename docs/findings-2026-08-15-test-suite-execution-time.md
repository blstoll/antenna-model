# Findings — test-suite execution time (2026-08-15)

Measurements and analysis collected while verifying roadmap **D27**, written for whoever
picks up **D18** (test-suite latency budget) and **D28** (test isolation). Everything here is
measured unless explicitly labelled a hypothesis.

**Machine and conditions:** macOS (darwin 25.6.0), 8-core, warm build, `debug` profile
(`cargo nextest run`, i.e. unoptimized — this matters enormously, see §3). Read §5 before
trusting any single number: some runs were contended, and the contention is called out
per-row.

---

## 1. Where the time actually goes

All rows `--profile full`, run per package because the whole workspace in one invocation
exceeds most tooling timeouts.

| Scope | Tests | Wall | Slow-marked |
|---|---:|---:|---:|
| `antenna-core` | 331 | **40 s** | 0 |
| `antenna-model` — all binaries | 504 | **848 s** | 39 |
| ↳ `integration` binary **alone** | 126 | **522 s** | 17 |
| ↳ 5 other binaries (`reference_validation`, `openapi_spec`, `openapi_routes_match`, `server_test`, `warning_code_vocabulary`) | 26 | 69 s | 2 |
| `calibrate` | 235 | **535 s** | 8 |
| `server_test` alone | 2 | 14 s | 0 |

Total for a full-profile pass: **~24 minutes**, and that is with each package run separately.

Two things fall straight out:

- **`antenna-core` is not the problem.** 331 tests — the physics engine, the Bessel ladder,
  the FFT, the artifact layer — in 40 seconds. Whatever is wrong is not the physics.
- **`antenna-model::integration` is the single biggest lever**: 25 % of that crate's tests
  for ~62 % of its wall clock.

## 2. The cluster that matters

Slowest tests in the `antenna-model` full run:

| Wall | Test |
|---:|---|
| 130.9 s | `integration::timeout_tests::test_heavy_heatmap_times_out_with_504` |
| 104.8 s | `integration::status_code_matrix_tests::legacy_feed_position_key_is_rejected_with_400` |
| 103.7 s | `integration::status_code_matrix_tests::malformed_body_is_400_everywhere` |
| 100.4 s | `integration::status_code_matrix_tests::h3_grid_type_on_heatmap_is_rejected_with_400` |
| 100.0 s | `integration::status_code_matrix_tests::non_finite_request_value_is_a_parse_failure` |
| 97.1 s | `integration::status_code_matrix_tests::geo_altitude_geodetic_emitter_is_accepted_when_tagged` |
| 96.4 s | `integration::status_code_matrix_tests::batch_never_returns_a_null_gain_for_a_validation_failure` |
| 96.0 s | `integration::status_code_matrix_tests::batch_rejection_names_the_failing_item_index` |
| 94.8 s | `integration::status_code_matrix_tests::degenerate_boresight_from_the_service_layer_is_422` |
| 91.1 s | `integration::resilience_tests::test_service_stability_under_mixed_workload` |
| 90.7 s | `integration::status_code_matrix_tests::semantically_invalid_body_is_422_everywhere` |
| 89.7 s | `integration::status_code_matrix_tests::batch_level_constraints_are_422` |
| 88.5 s | `integration::status_code_matrix_tests::a_position_without_coordinate_system_is_rejected_with_400` |
| 85.9 s | `integration::status_code_matrix_tests::unknown_antenna_is_404_everywhere` |
| 84.1 s | `integration::resilience_tests::test_recovery_after_multiple_failed_requests` |
| 83.5 s | `integration::resilience_tests::test_rate_limiting_behavior_under_error_load` |

`legacy_feed_position_key_is_rejected_with_400` posts a body with a wrong JSON key and asserts
a 400. It cannot consume 100 seconds of anything.

**The tell is the uniformity, not the magnitude.** Fifteen of the top sixteen sit in one
binary, and thirteen of those land in a tight 83–105 s band. Tests doing genuinely different
amounts of work do not converge like that. Tests that are all *blocked on the same thing*, and
complete when it releases, do.

Supporting contrast: the same `integration` binary **run alone** did all 126 tests in 522 s,
while inside the full-crate run these individual trivial assertions cost 85–105 s each. The
per-test figures are therefore measuring queueing, not cost — which is the same conclusion
D18 task 4 reached from a different direction (it saw a 404 test marked slow at >40 s).

**Leading hypothesis — NOT yet tested.** `.config/nextest.toml` gives `test_sustained_load`
`threads-required = "num-test-threads"` in both profiles. Nextest must drain every other slot
before starting it and runs nothing alongside it, so the run serializes around one test while
everything sharing the `integration` binary queues behind it. That override is deliberate and
well argued where it is defined — the test asserts a *rate*, which no speedup makes
schedule-independent — so do not simply delete it.

**The experiment that settles it**, and it is cheap: run the `integration` binary with and
without that `threads-required` override and compare wall clock. If the 83–105 s band
collapses, the fix is a scheduling change (isolate the rate test into its own binary or its
own nextest group) and **no individual test needs to get faster**. If it does not collapse,
the hypothesis is dead and the shared-fixture theory in D18 task 2 is next. Record the answer
either way — D18 task 4 exists partly so the next person does not re-run this investigation.

## 3. `test_heavy_heatmap_times_out_with_504` — 131 s, and it does not need to be

This one deserves separate treatment because it is the single most expensive test in the
suite **and** the cheap version is already written, 200 lines above it, in the same file.

**Its own doc comment is wrong about its cost.** `heavy_heatmap_request()` is documented as
costing "hundreds of ms — far above the 50 ms deadline the test sets, yet bounded so the
un-cancellable background rayon finishes in well under a second." Measured: **130.9 s**.

The cost model was written against *release* figures while tests run in *debug*. The sibling
test's own comment supplies the conversion factor: the same 13 m Ka-band offset-feed geometry
is "~2.7 s in debug, ~140 ms in release" for a **single** gain — roughly 19×. The heatmap does
a 12×12 grid (0–45° at 4° steps) = **144 points** of exactly that geometry. 144 × 2.7 s ≈ 390 s
of CPU, which across 8 cores is the ~131 s observed. The number is fully explained; the comment
simply never accounted for the profile tests run in.

**The technique to avoid it is already in this codebase.**
`test_heavy_single_gain_times_out_with_504` (roadmap S2b, same file) runs on
`#[tokio::test(start_paused = true)]`, drives the app in-process through `Endpoint::call`, and
**asserts no wall-clock threshold at all**. Its documentation is worth reading in full before
touching any of this — it records that the socket path *cannot* use a mocked clock (a 35 s
mocked sleep completed in 320 µs of real time, so the deadline elapsed before the request even
arrived and the request returned a late 200), which is exactly why it runs in process.

Crucially, that test still uses an expensive request — but as a **race margin, not a
threshold**: the clock is advanced microseconds after the handler offloads, so any compute
above that suffices, and it then bounds the real cost it pays with
`config.performance.integration_budget_ms = 250`. That bound is the trick the heatmap test
does not use.

**Nothing in the heatmap test's assertions requires real time or a real socket.** It asserts
exactly three things:

1. status is `504`;
2. `x-request-id` is echoed on the timeout error path;
3. the body is the standard `ErrorResponse` with `error == "request_timeout"`.

All three survive an in-process conversion. `build_in_process_app` calls
`create_routes_with_timeout`, and both that and the production `create_routes` delegate to the
**same `build_app`** — identical middleware stack, RequestId included. The only missing piece
is a helper: `call_json` currently returns `(status, body)` and would need a sibling that also
returns headers, for assertion 2.

**Proposed fix** (hypothesis — expected result stated so it can be falsified): convert the
test to `start_paused = true` + in-process, mirroring S2b, shrink the grid to whatever keeps
the race margin (one Ka-band point is already ~5 orders of magnitude of margin), and bound the
un-cancellable rayon with `integration_budget_ms`. **Expected: 131 s → under 1 s, with all
three assertions unchanged.** Verify by measuring before and after; do not assume.

**What would be lost, and the honest counter-argument.** The in-process form does not exercise
real TCP, real hyper/reqwest serialization of the error response, or the socket-level 504. If
that coverage is judged load-bearing, keep *one* socket-level timeout test — but it does not
have to be the expensive one, and the reason the current test is expensive is that a heavy
request was the only available way to win the race against a *real* 50 ms deadline. That
constraint disappears the moment the clock is mocked.

**Correction to the record:** an earlier assessment in this work called this test's cost
"legitimate — it's a timeout test, it's supposed to burn wall clock." That is wrong. The
*assertion* is legitimate; the implementation is the most expensive possible way to make it,
and the cheap way already exists in the same file.

## 4. Practical guidance for running this suite

Hard-won during D27 verification; expect to lose an hour to these otherwise.

- **Run per package, not `--workspace`.** A full-workspace run exceeds common 10-minute
  tooling timeouts. `antenna-core` (40 s) and the non-`integration` `antenna-model` binaries
  (69 s) are quick; budget ~9 min each for `antenna-model::integration` and `calibrate`.
- **Split `antenna-model` further when you need results fast**: `-E 'binary(integration)'`
  versus everything else. The first compile after a change costs an extra ~2–3 min, so a run
  that fits the budget on the second attempt may not on the first.
- **Output is buffered when stdout is not a TTY.** `nextest` writes nothing until it exits, so
  a log that is 73 bytes long does *not* mean the run is stuck. Redirect to a file and check
  the byte count, but do not conclude anything from an empty file.
- **Do not read a summary line while the run is still writing.** A partial log will show
  passes and no failures and look green; the failures land at the end. Wait for the `Summary`
  line before drawing any conclusion.
- **Killing a run leaks bound sockets.** `server_test` binds literal ports 3001/3002 (see
  **D28**), and aborted server tasks from a killed run can keep holding them, so the *next*
  run fails with `AddrInUse` — surfaced misleadingly as `ConnectionRefused` against `/status`.
  If you see that, check for orphaned processes before believing you broke something.
- **Never run two suites concurrently** for the same reason. Also note `pkill -f "cargo-nextest
  run …"` does **not** match: the real argv is `cargo-nextest nextest run …`.

## 5. Measurement hygiene — read before quoting these numbers

- The 848 s `antenna-model` row was **contended**: it recorded two `server_test` failures from
  ports held by a previous killed run, so some queueing in it is self-inflicted. The 522 s
  `integration`-alone row and the 40 s / 535 s rows were clean.
- These are **ad-hoc runs taken during verification, not a controlled study.** They are a
  starting point for D18 task 4, not a substitute for its exit criterion, which asks for a
  reproducible procedure with stated machine state, command, and repetitions.
- D18 task 4 separately records four runs of the *identical* dev-profile suite on one idle
  machine at 339 s / 821 s / 931 s — a 2.7× spread. Nothing here explains that spread; it is
  still open. One further data point: a dev-profile run cancelled early by a test failure had
  completed 453 of 1039 tests in 88 s, i.e. before reaching the `integration` cluster above.
  That is consistent with the cluster owning most of the variance, but it is an observation,
  not a measurement of it.
