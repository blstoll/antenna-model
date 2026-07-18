# Plan — S1: Enforce the configured body-size limit

Roadmap unit: `docs/roadmap-2026-07-work-units.md` → **S1** (Phase 2, top of phase). Effort S/M.
Goal (theme T2): the configured `max_body_size_bytes` stops being decorative — oversized
requests are rejected with **413** and the project's standard JSON error body, limit stays
configurable.

## Problem (verified 2026-07-18 @ `a2c97a0`)

- `server.max_body_size_bytes` exists (`config/settings.rs:46-47`, default 10 MB at `:150`,
  validated non-zero at `:338`) but is **only logged** (`api/mod.rs:193`).
- `RequestSizeTracker` (`api/middleware.rs:255-356`) reads `content-length` and only `warn!`s;
  it never rejects. It is constructed with **hardcoded** thresholds at `api/routes.rs:71`
  (`RequestSizeTracker::new()` → 1 MB/10 MB warn) — the config value is never threaded in.
- The existing test `test_request_body_size_limit` (`tests/integration/error_tests.rs:338`)
  passes for the wrong reason: an 11 MB `"x".repeat(...)` body fails JSON parse → 400, and the
  test asserts `400 || 413`. It does not prove size enforcement.

## Design decision (made — do not re-litigate)

**Extend `RequestSizeTracker` into an enforcing limiter keyed on `content-length`, emitting the
project's JSON error body.** Rationale:

- poem's own `poem::middleware::SizeLimit` (`size_limit.rs`) enforces on `content-length` only
  (missing → 411, over → 413). So **content-length enforcement is the framework-blessed level** —
  we are not under-building by matching it. The only thing `SizeLimit` does *worse* for us is its
  error body (poem default text), which would not match sibling errors.
- Every sibling error in this codebase is
  `poem::Error::from_string(serde_json::to_string(&ErrorResponse::new(code, msg)).unwrap_or_default(), status)`.
  Producing the 413 the same way keeps the body identical to siblings and touches **zero** shared
  error plumbing (no `ErrorHandler` changes — C4 owns the text/plain→application/json content-type
  fix later; S1 must not pre-empt it).
- The existing tracker already reads `content-length`; turning warn-only into warn+enforce is the
  minimal, idiomatic change.

**Explicitly out of scope for S1** (do not do): streaming/buffered body caps for
missing/spoofed `content-length` (poem's own `SizeLimit` doesn't do this either); rejecting
missing `content-length` (411) — that is a behavior change with blast radius, defer; converting
error bodies to `application/json` content-type (C4); the request *timeout* (S2). Enforce only
when `content-length` is present and exceeds the limit; otherwise fall through unchanged.

## Files to change

1. `antenna-model/src/api/middleware.rs` — add a hard reject limit to `RequestSizeTracker`.
2. `antenna-model/src/api/routes.rs` — thread `state.config.server.max_body_size_bytes` into the
   middleware in `create_routes`; update `create_routes_with_size_limits` + its one test caller.
3. `antenna-model/tests/integration/error_tests.rs` — rewrite `test_request_body_size_limit`
   (well-formed oversized → exactly 413 + JSON body) and add an under-limit control.
4. `antenna-model/src/api/schemas.rs` — (only if needed) confirm the `"payload_too_large"` code;
   no struct change expected.
5. `openapi.yaml` — add a 413 response to the four POST compute endpoints (standing rule 4).
6. `docs/api-documentation.md` — document the body-size limit and the 413.

## Steps (TDD — write the test first, watch it fail, then implement)

### Step 1 — Failing integration test
In `tests/integration/error_tests.rs`, replace `test_request_body_size_limit`:

- Start a server with a **small** configured limit via
  `TestServer::start_with_config(Some(cfg))` where `cfg.server.max_body_size_bytes` is small
  (e.g. `256`). (Note `helpers.rs:48` hardcodes 10 MB in the *default* test config — you must pass
  a custom `ServiceConfig`; clone the default and shrink the one field.)
- Send a **well-formed** JSON gain request whose serialized size exceeds the small limit (a valid
  gain request body is a few hundred bytes; pick a limit below it, or pad a valid request). Assert
  status is **exactly `413`** (not `400 || 413`) and the body parses to
  `ErrorResponse` with `error == "payload_too_large"`.
- Add a control: with the default (10 MB) limit, a normal valid gain request is **not** 413
  (existing happy-path tests already cover 200; a targeted control assertion is enough).
- Run `cargo test -p antenna-model test_request_body_size_limit` and confirm it **fails** (current
  code returns 400 for the oversized-but-would-be-413 case, or lets it through).

### Step 2 — Enforce in the middleware
In `api/middleware.rs`, add a hard limit to `RequestSizeTracker`:

- Add field `max_request_size: usize` (the reject threshold). Keep `warn_request_size` /
  `warn_response_size`.
- Update constructors: `new()` keeps sane defaults (pick `max_request_size` = 10 MB to match the
  config default; warn thresholds unchanged). Add/adjust a constructor that takes the hard limit
  (e.g. `with_limits(max_request_size, warn_request_size, warn_response_size)`), and thread it
  through `Middleware::transform` into `RequestSizeTrackerImpl`.
- In `RequestSizeTrackerImpl::call`, in the existing `content-length` block: **before** calling the
  inner endpoint, if `size > self.max_request_size`, return
  ```rust
  let body = ErrorResponse::new(
      "payload_too_large",
      format!("Request body of {size} bytes exceeds the maximum of {} bytes", self.max_request_size),
  );
  return Err(poem::Error::from_string(
      serde_json::to_string(&body).unwrap_or_default(),
      poem::http::StatusCode::PAYLOAD_TOO_LARGE,
  ));
  ```
  Keep the existing `warn!` for the soft threshold (both can fire). Import `ErrorResponse` from
  `crate::api::schemas`.
- Keep this check **outermost** in the stack (rejection before body handling) — it already is,
  since `RequestSizeTracker` is the last `.with(...)` in `create_routes`.

### Step 3 — Wire config → middleware
In `api/routes.rs`:

- In `create_routes`, read `let max_body = state.config.server.max_body_size_bytes;` before
  building the route, and replace `RequestSizeTracker::new()` (`:71`) with the constructor that
  passes `max_body` as the hard limit (keep existing warn thresholds).
- In `create_routes_with_size_limits`, add the hard limit (either a new param or derive from the
  warn size); update the single caller `test_routes_with_custom_size_limits`
  (`api/routes.rs` tests) accordingly. Prefer an explicit param so the test can exercise a small
  hard limit directly if useful.
- Confirm `AppState` exposes `config` (it does — `state.config.server.*` is read at
  `api/mod.rs:193`).

### Step 4 — Verify the batch/heatmap gotcha
Confirm the default 10 MB comfortably exceeds a maximum-size 1000-item batch before enforcing
(`max_batch_size = 1000`, `settings.rs:183`). Estimate or assert: build a `BatchGainRequest` with
1000 representative items, `serde_json::to_string` it, assert length `< default_max_body_size()`.
Expected: well under 1 MB. If (unexpectedly) over, **raise the default in the same change and say
so in the PR** — do not silently ship a limit that rejects valid batches.

### Step 5 — Docs + spec (standing rule 4)
- `openapi.yaml`: add a `413` response referencing the `ErrorResponse` schema to `/api/v1/gain`,
  `/api/v1/gain/batch`, `/api/v1/heatmap`, `/api/v1/h3-heatmap`. Mirror the wording of existing
  error responses.
- `docs/api-documentation.md`: document `max_body_size_bytes`, the 413 status, and the JSON body.

### Step 6 — Full gate
Run `cargo test --workspace`, `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`
(the G1 gate, `scripts/check.sh`). All existing tests must pass unchanged except the rewritten
`test_request_body_size_limit`.

## Exit criteria (definition of done — from the work-units doc)

1. Requests exceeding the configured limit get **413** with the project's standard JSON error body.
2. `test_request_body_size_limit` rewritten to send a **well-formed** oversized body and assert
   **413** (plus an under-limit control), and asserts the JSON `error` code.
3. The limit remains **configurable** (driven by `server.max_body_size_bytes`, verified by the
   small-limit test).
4. Default limit confirmed to exceed a maximum 1000-item batch (Step 4).
5. `openapi.yaml` + `docs/api-documentation.md` updated; full workspace gate green.

## Watch-outs

- **Do not touch physics/model code** (standing rule 2) — this is a pure API-layer unit.
- **Do not fix the text/plain content-type** of error bodies — that's C4. Match the existing
  `from_string` idiom exactly; the body is a JSON *string* served as text/plain today, same as all
  siblings.
- The 413 emitted from the outermost middleware may not carry `x-request-id` (RequestId is inner in
  the current stack ordering) — that mirrors the existing stack behavior and is not S1's concern.
- Use `poem::http::StatusCode::PAYLOAD_TOO_LARGE`; `ApiError::PayloadTooLarge` exists but the wire
  path here is the `from_string`+`ErrorResponse` idiom, not `ApiError`.
