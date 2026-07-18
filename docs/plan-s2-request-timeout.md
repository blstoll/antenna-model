# Plan — S2: Enforce the configured request timeout

Roadmap unit: `docs/roadmap-2026-07-work-units.md` → **S2** (Phase 2). Effort **S/M** (raised
from S — see the critical finding). Pairs with S3 (compute-side wall-clock budget).
Goal (theme T2): `server.request_timeout_secs` stops being decorative — a request that exceeds
it returns a timeout status with the project's standard JSON body.

## Problem (verified 2026-07-18 @ `b0f5b81`)

- `server.request_timeout_secs: u64` exists (`config/settings.rs:42-43`, default 30 at `:146`,
  validated non-zero at `:331`) but is **only logged** (`api/mod.rs`, alongside the old
  `max_body_size` log line). No timeout of any kind is in the middleware stack (`api/routes.rs`).
- **poem 3.1.12 has no `Timeout` middleware** (confirmed: no such file in `poem/src/middleware/`,
  no `Timeout` export anywhere in the crate). We must write a custom one.

## ⚠️ Critical finding — a timeout middleware alone does NOT work here

The heavy handlers call the service compute **inline** on the async task:
`generate_heatmap_endpoint` → `generate_heatmap(&request, &state.repository)` (`api/handlers.rs:433`),
and the service layer runs **rayon `par_iter()` synchronously** (`service/heatmap.rs:77`,
`service/batch.rs:105`, `service/h3_link_budget.rs:451`). There is **no `spawn_blocking`**.

`tokio::time::timeout(dur, fut)` polls `fut` first and only checks the deadline when `fut`
returns `Pending`. Inline rayon is CPU-bound and never yields, so the wrapped future blocks the
tokio worker thread straight through the deadline and returns `Ok(result)` when compute finishes —
**the timeout never fires** for exactly the compute-bound requests it is meant to bound, and the
exit-criterion test (tiny timeout + heavy heatmap → timeout status) would get a late 200, not 504.

**Therefore S2 has two required parts:**
1. A custom timeout middleware (`tokio::time::timeout`), config-driven.
2. Move the three heavy handler compute call-sites onto `tokio::task::spawn_blocking` so the async
   task yields at a real `.await` (the join handle) — letting the timeout fire and returning the
   response without holding the connection on a blocked worker thread. The rayon work is **not
   cancelled**; it runs detached until it finishes (or until S3's wall-clock budget stops it). This
   is exactly the gotcha the roadmap calls out: "dropping the future doesn't stop the pool."

## Design decision (made — do not re-litigate)

- **Status: `504 GATEWAY_TIMEOUT`** (maintainer decision, 2026-07-18; revised from an initial 408).
  The deadline is a *server-side* wall-clock budget — the client sent a valid request promptly and
  the server then exceeded its own configured processing limit — so the fault belongs on the server
  side (5xx). RFC 7231 §6.5.7 scopes `408 Request Timeout` to a client slow to *send*; a 4xx would
  misattribute the overrun as client-fault and hide it from server-side error-rate SLOs. 504 is
  chosen over `503 + Retry-After` because our timeout is **deterministic in the request payload**
  (the same heavy grid re-costs the same), so there is no honest `Retry-After` value — the remedy is
  a smaller request, not waiting; 503's transient-recovery semantics do not hold here. The literal
  "gateway" framing is a mild stretch (no upstream), accepted as the least-bad standard code and
  reversible pre-C8. **The latent `ApiError::Timeout` mapping is updated 408 → 504 to match**
  (`error.rs`), keeping the taxonomy consistent even though the middleware emits the status directly.
  **S4 follow-up:** admission-control / overload rejection *is* transient (retry when a slot frees),
  so that path should use `503 + Retry-After` — reconsider under S4.
- **Emit the standard JSON body** the same way S1 does (do NOT change error content-type — C4):
  `poem::Error::from_string(serde_json::to_string(&ErrorResponse::new("request_timeout", msg)).unwrap_or_default(), StatusCode::GATEWAY_TIMEOUT)`.
  The machine `error` code stays **`request_timeout`** — it names the condition, not the wire status —
  so no client contract on the code string changes. Keep emitting via the `from_string` +
  `ErrorResponse` idiom for body consistency with every sibling error.
- **Middleware placement:** apply the timeout so it wraps handler execution. Inner of
  `RequestSizeTracker` is fine (size rejection should still be instant). It must wrap the endpoint
  whose future we want to bound.

**Out of scope (do not do):** cancelling/aborting in-flight rayon compute (that's physically what
S3's cooperative wall-clock budget addresses); changing any compute logic or integration sample
density (perf pitfall #2 in CLAUDE.md); the admission-control/worker-pool work (S4). Physics/model
code untouched (standing rule 2).

## Files to change

1. `antenna-model/src/api/middleware.rs` — new `RequestTimeout` middleware.
2. `antenna-model/src/api/routes.rs` — wire `state.config.server.request_timeout_secs` into the
   stack in `create_routes` (and `create_routes_with_size_limits` + its test caller).
3. `antenna-model/src/api/handlers.rs` — move the batch/heatmap/h3 compute call-sites onto
   `spawn_blocking` (call sites only — not the service internals).
4. `antenna-model/tests/integration/error_tests.rs` (or a new `timeout_tests.rs`) — failing-first
   integration test.
5. `openapi.yaml` — add a `504` response to the POST compute endpoints (standing rule 4).
6. `docs/api-documentation.md` — document the timeout + its honest limitation.

## Steps (TDD — write the test first, watch it fail, then implement)

### Step 1 — Failing integration test
- `TestServer::start_with_config(Some(cfg))` with a **small** `cfg.server.request_timeout_secs`
  (the field is `u64` seconds; if sub-second is needed for a fast test, see the note below).
- POST a **heavy** heatmap request (large grid / fine step, or a big H3 `n_rings`) that reliably
  exceeds the timeout. Assert status **504** and a JSON body parsing to `ErrorResponse` with
  `error == "request_timeout"`.
- Run it, confirm it **fails** (today: no timeout, so a late 200).
- **Sub-second note:** `request_timeout_secs` is whole seconds. If a 1 s timeout makes the test
  slow/flaky, either (a) make the heavy request heavy enough to blow past 1 s deterministically, or
  (b) if you must go sub-second, thread a `Duration`-typed internal knob and keep the public config
  in seconds — do NOT change the config field's units. Prefer (a).

### Step 2 — `RequestTimeout` middleware (`api/middleware.rs`)
- Struct `RequestTimeout { timeout: std::time::Duration }` + `Middleware`/`Endpoint` impls
  (mirror `RequestSizeTracker`'s shape).
- In `call`: `match tokio::time::timeout(self.timeout, self.ep.call(req)).await { Ok(r) => r, Err(_elapsed) => Err(<504 JSON error>) }`.
- Add a code comment stating plainly: the timeout returns a response to the client but does **not**
  cancel rayon compute already running on the blocking pool; cooperative compute bounding is S3.
- Import `ErrorResponse` from `crate::api::schemas`.

### Step 3 — Wire config → middleware (`api/routes.rs`)
- In `create_routes`: `let timeout = Duration::from_secs(state.config.server.request_timeout_secs);`
  then `.with(RequestTimeout::new(timeout))` at the chosen position.
- Update `create_routes_with_size_limits` similarly (test helper) and its one caller.

### Step 4 — Move heavy compute onto `spawn_blocking` (`api/handlers.rs`)
For `compute_gain_batch`, `generate_heatmap_endpoint`, and `h3_link_budget`:
- Replace the inline synchronous service call with:
  ```rust
  let state = state.clone();               // Arc<AppState> into the closure
  let result = tokio::task::spawn_blocking(move || {
      generate_heatmap(&request, &state.repository)   // request moved (owned), Send
  })
  .await
  .map_err(|e| /* JoinError → 500 internal_error, standard JSON */)?;
  ```
- Keep the existing `match result { Ok(..) => .., Err(..) => .. }` mapping unchanged.
- Confirm the service fns' inputs/outputs are `Send + 'static` for the closure (owned request,
  `Arc` repository, owned result — expected fine). Do **not** move `&state.repository` borrows
  across the boundary; clone the `Arc`.
- Single-gain (`compute_gain`) is light; leave it inline unless trivial to include for consistency
  (optional — call it out either way).

### Step 5 — Docs + spec (standing rule 4)
- `openapi.yaml`: add a `504` response referencing `ErrorResponse` to `/api/v1/gain/batch`,
  `/api/v1/heatmap` (and `/api/v1/gain` if included). Note the `/api/v1/h3-heatmap` **path** is
  absent from openapi (roadmap C1) — do not add the path here; only touch documented endpoints.
- `docs/api-documentation.md`: document `request_timeout_secs`, the 504, and the **honest
  limitation** — the server stops waiting and responds, but background compute continues until it
  completes or S3's budget halts it.

### Step 6 — Full gate
`cargo test --workspace`, `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`
(`scripts/check.sh`). Existing tests pass unchanged; the new timeout test passes.

## Exit criteria (definition of done)

1. A request exceeding `request_timeout_secs` returns a timeout status (504) with the standard JSON
   body — proven by a failing-first integration test using a tiny timeout + heavy heatmap.
2. The timeout is wired to config and configurable.
3. The "rayon not cancelled" limitation is stated in a code comment **and** in api-documentation.md.
4. `spawn_blocking` refactor leaves all existing compute results/tests unchanged.
5. openapi.yaml + api-documentation.md updated; full gate green.

## Watch-outs

- **The middleware is useless without Step 4** — do not ship middleware-only; the exit-criterion
  test cannot pass with inline-blocking handlers.
- **Don't claim more than delivered:** the response is bounded, the compute is not. Say so.
- **Don't touch physics/integration internals** or reduce sample density.
- **Interactions:** S3 adds the real cooperative compute budget; S4 bounds concurrency and the
  worker/blocking pools. `spawn_blocking` here is compatible with both and is a prerequisite the
  roadmap's S2 gotcha implicitly assumes.
- **Branch:** stack on `feat/s1-body-size-limit` (both edit `middleware.rs`/`routes.rs`) to avoid a
  conflict with the S1 changes.
