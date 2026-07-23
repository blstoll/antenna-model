# Plan — S3: Wall-clock budget inside aperture integration

Roadmap unit: `docs/roadmap-2026-07-work-units.md` → **S3** (Phase 2). Effort **M**. Depends on
S2 (landed, #10) — S2's `RequestTimeout` (504) bounds the *response* but explicitly does **not**
cancel the rayon compute it dropped ("the rayon work is not cancelled on timeout"). S3 is the
missing cooperative compute-side bound: a single pathological integration must be able to stop
itself. Goal (theme T2): a runaway aperture integral returns a **typed error**, never a silently
degraded result or an unbounded CPU burn.

## Problem (verified 2026-07-18 @ `da7a2f4`)

- A single `integrate_aperture` call is not free: the served risk row records **dsn_34m Ka at
  θ=90° ≈ 3.3 s** (`roadmap-2026-07.md:269-271`). The radial sample count is adaptive and clamped
  only by `RADIAL_POINTS_SAFETY_MAX = 65_537` (`integration.rs:106`); the coma/mode path costs
  `n_rho × n_phi × modes`. There is **no wall-clock bound anywhere** in the compute path.
- S2's `spawn_blocking` + `RequestTimeout` returns 504 to the client but leaves the rayon work
  running to completion in the background (`api/middleware.rs:440`, `handlers.rs:312-316`). Without
  S3, a slow request still consumes a full core until the integral finishes on its own.
- A `/heatmap` fans out to ~10⁵ points and `/gain/batch` to ~10³ items, each delegating to
  `compute_gain_from_request` (`heatmap.rs:300`, `batch.rs:108`) which runs **two** integrations
  per point (off-axis + the boresight normalization anchor, `pattern.rs:485,526`).

## ⚠️ Critical finding — the work-unit's "iteration boundaries" premise is stale (pre-P10)

S3 was written 2026-07-08; **P10 landed 2026-07-15** and changed the integrator's shape. The unit
says "check elapsed time at iteration boundaries" and points at `IntegrationParams` `max_iterations`
(3/5/8). **That refinement loop no longer runs on the served path.** Post-P10:

- The served path is a *closed form*, not an iterative refinement: symmetric feeds take the 1D
  Hankel radial sum (`hankel_radial_field`, one `for i in 0..n` loop, `integration.rs:~340`);
  asymmetric/coma feeds take the Jₘ mode expansion (`azimuthal_mode_field_inner`, an outer
  `for i in 0..n` radial loop with inner per-mode work, `integration.rs:~410-440`).
- `max_iterations` survives only for the `#[cfg(test)]` 2D reference oracle
  (`integrate_2d_adaptive`) — the docstring at `integration.rs:226-235` says the field is inert on
  the served path.

**Therefore the "iteration boundary" S3 must instrument is the radial-sample loop** inside those
two helpers — that is where the ~3.3 s is spent and the only place a check can interrupt a single
slow call. Checking only *between* `integrate_aperture` calls (per-point) cannot stop the one
integration that is itself the problem.

## Design — budget rides in `IntegrationParams`, checked in the radial loop

`IntegrationParams` is already threaded `service → pattern → integrate_aperture`. Carry the budget
there rather than churning `integrate_aperture`'s signature (the P10 invariant: evaluator / cache /
heatmap / h3 untouched by integrator changes).

1. **New field** `IntegrationParams::time_budget: Option<Duration>` (`integration.rs`). Every
   preset (`adaptive()`, `fast()`, `high_accuracy()`, `default()`) sets a **generous** default via
   a named constant; the two full struct literals in tests get one line added.
2. **`integrate_aperture`** computes `let deadline = params.time_budget.map(|b| Instant::now() + b)`
   at entry (per-call, per-integration granularity — the accepted v1 assumption) and passes
   `Option<Instant>` into the two hot helpers.
3. **`hankel_radial_field` and `azimuthal_mode_field_inner`** change return type to
   `ComputationResult<…>` and check the clock **at chunk boundaries only** — every `N`
   (e.g. 1024) radial samples, not every iteration (`Instant::now()` is too costly per-sample and
   perf pitfall #2 forbids touching sample density). On expiry return the new typed error;
   `integrate_aperture` propagates it with `?`.
4. **Result determinism preserved:** when the deadline is not hit, the check is a pure side-effect —
   fields are byte-identical to today. **Convergence math is untouched** (gotcha). The existing
   non-convergence *warning* behavior is unchanged and orthogonal — the budget is a separate, harder
   *error* stop, exactly as the unit says.

### How config reaches the service layer (decided — thread it explicitly; do not re-litigate)

Neither served entry point sees config today: `compute_gain_from_request(&request, &repository)`
(`evaluator.rs:97`) and `compute_h3_link_budget(…)` (`h3_link_budget.rs:293`) build
`IntegrationParams::adaptive()` internally (`evaluator.rs:213`, `h3_link_budget.rs:327`); batch and
heatmap delegate to `compute_gain_from_request`. The budget default lives in the model layer, but
the **configurable** knob has to be injected at those two sites.

**Decision (maintainer, 2026-07-19): thread the configured budget explicitly — an unwired default
is rejected.** T2 requires "every knob either works or is removed"; a decorative
`integration_budget_ms` is the exact anti-pattern S1/S2 just fixed. Concretely:

- Add a `time_budget: Duration` parameter to the two served entry points; handlers supply
  `state.config.performance.integration_budget_ms`.
- Bound test churn by keeping the current 2-arg signatures as thin wrappers that pass the generous
  default (`DEFAULT_INTEGRATION_BUDGET`), and adding `*_with_budget` variants that
  batch/heatmap/handlers call. Every existing test call-site compiles unchanged; the live path is
  genuinely config-driven.

### Status code — 504, mirroring S2 (do not reuse 503)

A budget overrun is **deterministic in the request payload** (the same heavy grid re-costs the
same) — retrying is futile, the remedy is a smaller request. That is precisely S2's 504 rationale,
not S4's transient-admission 503+`Retry-After`. So:

- New variant `ComputationError::TimeBudgetExceeded { operation, elapsed_ms, budget_ms }`.
- Update `From<ComputationError> for ApiError` (`error.rs:432`, today a blanket → `InternalError`
  500) to map this variant → `ApiError::Timeout` (504), keeping the taxonomy consistent (same
  edit shape S2 made when it moved `Timeout` 408 → 504).
- Machine error code **`computation_budget_exceeded`** — distinct from S2's `request_timeout` so
  ops can tell "middleware gave up waiting" from "a single integral was aborted." The four handlers
  map errors by matching `AntennaModelError` variants (`handlers.rs:~235`); add an arm for
  `Computation(TimeBudgetExceeded{..}) → (504, "computation_budget_exceeded")` (or route through
  `ApiError::from(e).status_code()`).

## Files to change

1. `antenna-model/src/model/integration.rs` — `time_budget` field + default constant; deadline in
   `integrate_aperture`; chunked clock check in `hankel_radial_field` /
   `azimuthal_mode_field_inner` (return type → `ComputationResult`).
2. `antenna-model/src/error.rs` — `ComputationError::TimeBudgetExceeded`; map it to `ApiError::Timeout`.
3. `antenna-model/src/config/settings.rs` — `PerformanceConfig.integration_budget_ms: u64`
   (`default_integration_budget()`, generous); `set_default("performance.integration_budget_ms", …)`;
   reject `== 0` in `validate()`; add to the `Default`/`with_defaults` paths.
4. `antenna-model/src/service/evaluator.rs` + `h3_link_budget.rs` — set
   `integration_params.time_budget` from the configured budget via the new `*_with_budget` entry points.
5. `antenna-model/src/service/heatmap.rs` + `batch.rs` + `api/handlers.rs` — thread the budget from
   `state.config` through to the `*_with_budget` calls.
6. `config/service.yaml` — `performance.integration_budget_ms` with a comment.
7. `antenna-model/tests/…` — model-layer failing-first test + one HTTP integration test.
8. `openapi.yaml` (add `504`/`computation_budget_exceeded` note to the compute endpoints — but the
   `/h3-heatmap` **path** is still absent, roadmap C1: don't add the path) + `docs/api-documentation.md`
   (standing rule 4).

## Steps (TDD — write the test first, watch it fail, then implement)

### Step 1 — Failing-first tests
- **Model-layer unit test** (cheapest, no HTTP): build `IntegrationParams::adaptive()`, set
  `time_budget = Some(Duration::from_nanos(1))`, call `integrate_aperture` at a wide angle on an
  offset-feed config; assert `Err(ComputationError::TimeBudgetExceeded{..})`. A 1 ns budget is
  already expired at the first chunk boundary, so this is deterministic and fast.
- **HTTP integration test:** `TestServer` with a tiny `performance.integration_budget_ms`; POST a
  request; assert **504** and a JSON body parsing to `ErrorResponse` with
  `error == "computation_budget_exceeded"`. Confirm both fail today.

### Step 2 — `time_budget` field + chunked check (`integration.rs`)
- Add the field + a `DEFAULT_INTEGRATION_BUDGET` constant used by all presets.
- In the two helpers: `const BUDGET_CHECK_STRIDE: usize = 1024;` and, when
  `i % BUDGET_CHECK_STRIDE == 0 && deadline.is_some_and(|d| Instant::now() > d)`, return
  `Err(TimeBudgetExceeded{ operation, elapsed_ms, budget_ms })`.
- `integrate_aperture`: compute the deadline once, propagate the helper `Result` with `?`.
- Comment: this is a *cooperative* per-integration stop; it does **not** halt the enclosing rayon
  `par_iter` (that's request-level S2 / concurrency S4). Per-integration granularity is the v1 scope.

### Step 3 — Error taxonomy (`error.rs`) + handler mapping
- Add the variant; map → `ApiError::Timeout` in `From<ComputationError>`; add the handler arm →
  `(504, "computation_budget_exceeded")`.

### Step 4 — Config knob (`settings.rs`, `service.yaml`)
- Field, default fn (**generous** — the slowest known single integration is dsn_34m Ka θ=90° ≈ 3.3 s,
  so default ≥ 10 s; recommend **30_000 ms** with slow-CI headroom), `set_default`, non-zero
  validation, `service.yaml` entry + comment.

### Step 5 — Wire config → service
- `*_with_budget` entry points on `compute_gain_from_request` / `compute_h3_link_budget`; thread
  through `generate_heatmap` / `evaluate_batch`; handlers pass
  `Duration::from_millis(state.config.performance.integration_budget_ms)`. Keep the 2-arg
  signatures as generous-default wrappers so existing test call-sites compile unchanged.

### Step 6 — Docs + full gate
- `openapi.yaml` + `api-documentation.md`: document `integration_budget_ms`, the 504, and the honest
  limitation (per-integration, not per-request; rayon fan-out still bounded only by S2/S4).
- `cargo test --workspace` (incl. `reference_validation` — **verify the generous default leaves it
  and every wide-angle test passing unchanged**), `cargo fmt`, `cargo clippy --workspace
  --all-targets -- -D warnings` (`scripts/check.sh`).

## Exit criteria (definition of done)

1. A single over-budget integration returns `ComputationError::TimeBudgetExceeded` → **504** with the
   standard JSON body (`computation_budget_exceeded`) — proven by a failing-first model test **and**
   a tiny-budget HTTP test.
2. The check is at radial chunk boundaries only; **convergence math and served results are
   byte-identical** when under budget; the non-convergence *warning* path is unchanged.
3. `integration_budget_ms` is wired to config and configurable — the knob is live.
4. The default is generous enough that **all existing tests pass unchanged**, `reference_validation`
   included.
5. openapi.yaml + api-documentation.md updated; full gate green.

## Watch-outs

- **Don't instrument `max_iterations` / the 2D adaptive loop** — it's `#[cfg(test)]`-only post-P10.
  The live boundary is the radial-sample loop (Critical finding).
- **Don't reduce sample density or touch convergence** (CLAUDE.md pitfall #2 / standing rule 2). The
  budget is an *additional abort*, never a degraded answer — "never a silently degraded result."
- **Per-integration ≠ per-request.** State plainly (code + docs) that S3 caps each integral; S2 caps
  the request wall-clock; S4 caps concurrency. A huge heatmap can still spend `budget × points` of
  background CPU after S2's 504 — bounding that fan-out is S4, not S3.
- **`Instant::now()` cost:** check every ~1024 samples, never per-sample.
- **Two integrations per gain** (off-axis + boresight anchor) each get a fresh deadline — intended.
- **Branch:** `feat/s3-integration-budget` off `main` (S1/S2/S6 already landed). Pairs with the S2
  gotcha it completes.
