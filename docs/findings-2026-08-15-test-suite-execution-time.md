# Findings — test-suite execution time (2026-08-15)

Roadmap **D18** (test-suite latency budget). The suite's dominant cost was found and fixed;
this records the measurements, the fix, and — because the first diagnosis was wrong in an
instructive way — how the wrong answer survived a plausible-looking argument.

**Headline:** `antenna-model` full-profile went from **848 s to 33.1 s** (25.6×), all 504
tests passing, via a one-line change to a dev-dependency's feature set. The most expensive
test in the suite, `test_heavy_heatmap_times_out_with_504`, went from **130.9 s to 1.2 s**
separately, by converting it to the paused-clock pattern its sibling already used.

**Machine and conditions:** macOS (darwin 25.6.0), 8-core, warm build, `debug` profile
(`cargo nextest run`, i.e. unoptimized — this matters, see §4). Read §6 before quoting any
single number.

---

## 1. The dominant cost: `reqwest::Client::builder().build()` — 11.8 s per call, on macOS

Every integration test starts a `TestServer`, and `TestServer::start_inner` builds a
`reqwest::Client`. Instrumented breakdown of `test_health_endpoint` — a test that starts a
server, issues one `GET /health`, and asserts one field:

| Phase | Time |
|---|---:|
| `CalibrationRepository::load_from_config` | 2.7 ms |
| `AppState::new` | 0.09 ms |
| **`reqwest::Client::builder().build()`** | **11 793 ms** |
| health-endpoint wait (1 attempt) | 1.8 ms |
| **total test** | **12.0 s** |

reqwest's default feature set includes `system-proxy`. On macOS, constructing *any* client
therefore queries the system proxy configuration through `SCDynamicStore`
(`system-configuration` → `configd`). That call is the entire cost of the test — 4 000× the
cost of loading the whole calibration repository.

**Fix**, in `antenna-model/Cargo.toml` (reqwest is a **dev**-dependency only, so this cannot
touch production):

```toml
reqwest = { version = "0.13.4", default-features = false, features = ["json"] }
```

Result: client construction **11.8 s → 8 ms**; `test_health_endpoint` **12.0 s → 0.18 s**.

This is a one-line fix rather than 17 call-site edits, which matters: there are 17
`reqwest::Client` construction sites across the test tree, several of them `Client::new()`
*inside loops and spawned tasks* (`resilience_tests`, `error_tests`), which is why those
tests were the slowest ones after the heatmap. A feature-level fix cannot be forgotten at a
new call site. `.no_proxy()` on the builder was measured and works equally well
(11.8 s → 1.1 ms) but only where someone remembers to write it.

Also dropped, all unreachable for a client that talks to an ephemeral loopback port over
plain HTTP: `default-tls`/`rustls`, `http2`, `charset`. Verified absent from the graph
workspace-wide (`cargo tree --workspace | grep -c system-configuration` → 0). **Do not
restore `default-features`** to obtain one of them; add the feature by name.

## 2. Measured effect

| Scope | Before | After |
|---|---:|---:|
| `antenna-model`, `--profile full`, 504 tests | **848 s** | **33.1 s** |
| ↳ `integration` binary alone, 126 tests | 522 s | — (folded into the above) |
| `integration::timeout_tests::test_heavy_heatmap_times_out_with_504` | 130.9 s | **1.2 s** |
| `integration::status_code_matrix_tests::legacy_feed_position_key_is_rejected_with_400` | 104.8 s | 0.19 s |
| `integration::status_code_matrix_tests::malformed_body_is_400_everywhere` | 103.7 s | 0.19 s |
| `integration::resilience_tests::test_rate_limiting_behavior_under_error_load` | 83.5 s | 1.5 s |
| `integration::api_tests::test_health_endpoint` | 12.0 s | 0.18 s |
| `antenna-core`, 331 tests | 40 s | 40 s (untouched — never the problem) |

After the fix the two slowest tests in `antenna-model` are the genuine physics pins,
`p12_phi_cap_removed_steered_feed_matches_converged_reference` (16.0 s) and
`p12_mode_path_radial_convergence_anchors` (15.5 s) — i.e. the slow tier is now actually the
slow tier, which is what `.config/nextest.toml` always claimed it was.

## 3. How the first diagnosis went wrong

The first pass at this filed a different root cause, and it is worth recording why, because
the reasoning looked sound.

The observation was right: **the tell was the uniformity, not the magnitude** — thirteen
trivial assertions landing in a tight 83–105 s band, when tests doing genuinely different
amounts of work do not converge like that. The inference drawn from it was also right in
form: *tests that are all blocked on the same thing, and complete when it releases, look like
this.*

The named suspect was wrong. The hypothesis was `test_sustained_load`'s
`threads-required = "num-test-threads"` override in `.config/nextest.toml` serializing the
run. The shared blocking resource was real, but it was **`configd`**, not nextest's
scheduler: 126 processes each making a serialized system-configuration query.

Two things would have killed the wrong hypothesis faster, and both are cheaper than the
experiment that was proposed to settle it:

1. **Check the mechanism against the measurement.** nextest is process-per-test and reports
   each test's *own process lifetime*. Time spent queued for a slot is therefore not in the
   number. A scheduling reservation cannot inflate a per-test figure at all, so the
   hypothesis was already inconsistent with the evidence that motivated it.
2. **Run one slow test alone before theorising.** `malformed_body_is_400_everywhere` alone
   was 14.1 s against 103.7 s in the full run. That immediately splits the problem into a
   ~12 s intrinsic constant and a ~7× contention multiplier, and the intrinsic constant is
   both the larger share and the one that can be attributed by instrumenting four lines.

The general lesson, and the reason this section exists: *"which shared resource?"* was
answered by picking the most visible candidate in the repo's own config rather than by
measurement. The proposed experiment (toggle the override, compare wall clock) would have
returned "no change" after ~20 minutes and left the actual cause unfound.

## 4. `test_heavy_heatmap_times_out_with_504` — a second, independent defect

Fixed in the same change, but a genuinely separate problem: this test was expensive for
reasons unrelated to reqwest, and stayed the most expensive test even after §1.

**Its doc comment was wrong about its own cost.** `heavy_heatmap_request()` was documented as
costing "hundreds of ms — far above the 50 ms deadline the test sets, yet bounded so the
un-cancellable background rayon finishes in well under a second." Measured: **130.9 s**.

The cost model was written against *release* figures while tests run in *debug*. The sibling
test's comment supplies the conversion factor: the same 13 m Ka-band offset-feed geometry is
"~2.7 s in debug, ~140 ms in release" for a **single** gain — roughly 19×. The heatmap ran a
12×12 grid = **144 points** of exactly that geometry. 144 × 2.7 s ≈ 390 s of CPU, ≈ 131 s
across 8 cores. Fully explained.

**The cheap technique already existed 200 lines above, in the same file.**
`test_heavy_single_gain_times_out_with_504` (roadmap S2b) runs on
`#[tokio::test(start_paused = true)]`, drives the app in process through `Endpoint::call`, and
asserts no wall-clock threshold at all. Its docs are worth reading before touching any of
this: they record that the socket path *cannot* use a mocked clock (a 35 s mocked sleep
completed in 320 µs real, so the deadline elapsed before the request arrived), which is
exactly why it runs in process. It still uses an expensive request, but as a **race margin,
not a threshold**, and bounds the real cost it pays with `integration_budget_ms = 250`.

**The conversion.** The heatmap test now uses the same pattern: paused clock, in process,
`integration_budget_ms = 250`, and a 2×2 grid instead of 12×12 (once `advance` crosses the
deadline, the race no longer has to be won with compute). All three assertions are unchanged
— 504, `x-request-id` echoed, `error == "request_timeout"` — because `build_in_process_app`
calls `create_routes_with_timeout` and both that and production `create_routes` delegate to
the **same `build_app`**, so the middleware stack including `RequestId` is identical. A new
helper `call_json_with_headers` returns response headers, which `call_json` did not.

**What socket coverage moved, honestly stated.** The in-process form does not exercise real
TCP or hyper's serialization of a 504. A socket-level 504 with the standard JSON body is
still asserted by `budget_tests::test_over_budget_single_gain_returns_504` (on S3's
`computation_budget_exceeded`, and without a request-id assertion), and `x-request-id` echo on
an error path is still asserted over a socket by `error_tests` (on a 413). No socket-level
assertion is lost; only this particular *combination* is now in-process only.

**Correction to the record:** an earlier assessment called this test's cost "legitimate — it's
a timeout test, it's supposed to burn wall clock," and `.config/nextest.toml` said "slow by
construction and no speedup will change that." Both were wrong, in the same way: a property
was asserted of the *mechanism* (timeouts need wall clock) when it was really a property of
one *implementation* of the assertion. The test has rejoined the dev-loop profile.

## 5. Practical guidance for running this suite

Mostly hard-won before the fix; the first two matter much less now that a full `antenna-model`
run is 33 s, but the rest still apply.

- **Output is buffered when stdout is not a TTY.** `nextest` writes nothing until it exits, so
  a zero-byte log does *not* mean the run is stuck.
- **Do not read a summary line while the run is still writing.** A partial log shows passes and
  no failures and looks green; failures land at the end. Wait for the `Summary` line.
- ~~**Killing a run leaks bound sockets.** `server_test` binds literal ports 3001/3002 (see
  **D28**), and aborted server tasks can keep holding them, so the *next* run fails with
  `AddrInUse` — surfaced misleadingly as `ConnectionRefused` against `/status`. Check for
  orphaned processes before believing you broke something, and do not run two suites at
  once.~~ **Fixed by D28, 2026-08-15.** `server_test` binds port 0 like everything else, so
  an orphaned server holds a port nothing else asks for, and two suites can run at once
  (verified with two concurrent `antenna-model` runs). The misleading-signature half is
  fixed too: the bind now happens before the server task is spawned, so a collision is
  returned to the caller as `AddrInUse` instead of being panicked on an orphaned task and
  read as `ConnectionRefused`.
- `pkill -f "cargo-nextest run …"` does **not** match; the real argv is
  `cargo-nextest nextest run …`.
- `calibrate` (~535 s full profile) is now the workspace's dominant cost and is *not* affected
  by any of this — it does not use reqwest. Its cost is real physics: parameter tuning and
  surface fitting in debug. That is the next place to look, and nothing here says whether it is
  reducible.

## 6. Measurement hygiene

- The 848 s "before" figure was **contended** — it recorded two `server_test` failures from
  ports held by a previously killed run, so some of it was self-inflicted. The 522 s
  `integration`-alone and 40 s `antenna-core` rows were clean. The 33.1 s "after" figure was
  clean. Treat 848 → 33.1 as approximate at the top end; the per-test rows (12.0 → 0.18 s,
  130.9 → 1.2 s) are the trustworthy ones, and they are individually decisive.
- D18 task 4 separately records four runs of the *identical* dev-profile suite on one idle
  machine at 339 s / 821 s / 931 s — a 2.7× spread. **That spread is now explained**: a
  contended global `configd` query, taken 126+ times, is exactly the kind of shared serialized
  resource whose cost depends on unrelated system state. It should be re-measured post-fix to
  confirm the variance is gone rather than assumed.
- These were ad-hoc runs during D27/D18 work, not a controlled study. D18 task 4's exit
  criterion asks for a reproducible procedure with stated machine state, command, and
  repetitions; that is still worth doing, and is now cheap.
