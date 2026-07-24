# Plan — S4: Admission control + resolve dead `worker_threads` config

Roadmap: `docs/roadmap-2026-07-work-units.md` → Phase 2 → **S4** (Effort: M).
Companion prior units: `plan-s1-body-size-limit.md`, `plan-s2-request-timeout.md`,
`plan-s3-integration-budget.md`. Depends on **S1, S2** (both landed — same middleware
stack). Blocks nothing hard; **S5** builds on the same startup region afterward.

All line references verified against HEAD `b2517ee` on 2026-07-22. Re-verify before editing.

---

## Problem (two independent config lies)

1. **`performance.worker_threads` is dead config.** Declared and defaulted
   (`config/settings.rs:124-126`, default `0` = auto), logged at startup
   (`api/mod.rs:206-211`), and **never applied**. Every parallel path
   (`service/batch.rs:122`, `service/heatmap.rs`, `service/h3_link_budget.rs:479`)
   uses rayon's *global* pool with its own default thread count. Setting the knob does
   nothing.

2. **No admission control anywhere.** There is no concurrency-limit middleware
   (`api/routes.rs:83-96` stack is RequestTimeout → RequestSizeTracker → ErrorHandler →
   RequestLogger → RequestId → Tracing). Concurrent heavy requests (batch up to 1000
   items, heatmap up to ~10⁵ points, h3-heatmap) contend on the shared rayon pool
   unboundedly. Each heavy handler already offloads its rayon work to
   `tokio::task::spawn_blocking` (`handlers.rs:324,476,1019`, added by S2), so N
   concurrent heavy requests spawn N blocking tasks all fighting for the same cores —
   latency degrades for everyone with no backpressure.

---

## Design decisions (made — do not re-litigate)

### D1 — Wire `worker_threads`, don't remove it
The roadmap's recommended default. Apply it once at server startup via
`rayon::ThreadPoolBuilder::new().num_threads(n).build_global()`. `worker_threads == 0`
keeps rayon's auto-detection (do **not** call `build_global` in that case — passing 0 is
invalid and auto is the sane default). `build_global()` can only succeed once per process
and returns `Err` if a pool already exists → **log a warning and continue**, never panic
(repo rule: no `unwrap`/`expect`/`panic` on the production path).

### D2 — Admission control = non-blocking rejection, heavy endpoints only
- **Reject, don't queue.** On saturation, fail fast with a status — do not make the
  client wait in a queue (queueing turns a load problem into a latency problem and
  fights the S2 timeout). Use `Semaphore::try_acquire`.
- **Heavy endpoints only:** `/api/v1/gain/batch`, `/api/v1/heatmap`,
  `/api/v1/h3-heatmap`. Single `/api/v1/gain`, health/ready/status, and the antenna
  listing endpoints are cheap and must stay unthrottled (health/ready especially — a
  readiness probe must never be admission-rejected).
- **One shared budget** across all three heavy endpoints: a single
  `Arc<tokio::sync::Semaphore>` instance, cloned into each endpoint's middleware, so the
  limit caps *total* concurrent heavy work, not per-endpoint.

### D3 — `503 + Retry-After`, per the S2 rationale
Saturation is genuinely transient — a slot frees when an in-flight heavy request
finishes — so this is the case the `RequestTimeout` doc-comment (`middleware.rs:428-434`)
explicitly reserved `503 + Retry-After` for. This is deliberately **distinct from S2's
504** (deterministic in the payload, no `Retry-After`). Do **not** reuse 504; do **not**
omit `Retry-After` on the 503.
- **Wire status:** `503 Service Unavailable`.
- **Machine `error` code:** `service_overloaded` (snake_case, matching the S1/S2
  `payload_too_large`/`request_timeout` vocabulary; C3 owns the final vocabulary — reuse
  the established style, don't invent a PascalCase one).
- **`Retry-After`:** a small fixed backoff, config-driven (see D4). A precise
  service-time estimate isn't worth the complexity for v1; a defensible constant is.

### D4 — Two new config fields, both with generous defaults
Add to `PerformanceConfig` (`config/settings.rs`):
- `max_concurrent_heavy_requests: usize` — semaphore permits. **Default `0` = unlimited**
  (admission control effectively off unless configured). Rationale: turning a hard
  request-rejecting limit *on by default* is a behavior change that could surprise the
  pre-production deployment and risks flaking the concurrency tests
  (`concurrent_tests.rs` fires up to ~20 concurrent requests). Off-by-default keeps every
  existing test green by construction; operators opt in. **If the maintainer prefers
  on-by-default**, pick a value ≥ the max concurrency any test drives (≥ 32) and expect
  to re-verify `concurrent_tests.rs` — flagged as an open call below.
- `admission_retry_after_secs: u64` — `Retry-After` header value. Default `5`.

Mirror both in `ServiceConfig::from_file` `set_default` calls and `PerformanceConfig::default()`.

---

## Open call for the maintainer (before implementing)

**Admission-control default:** off (`max_concurrent_heavy_requests = 0`, recommended) vs.
on with a generous cap. Off-by-default is safest for a pre-production service with no live
SLA and keeps the existing concurrency tests untouched; on-by-default exercises the
feature in the default config but needs a cap chosen above test concurrency. Recommend
**off-by-default**, with the limit documented as the production knob to set. *(This is a
config-default choice, not a physics/contract decision — proceeding on the recommendation
if no objection.)*

---

## Files to change

| File | Change |
|---|---|
| `antenna-model/src/config/settings.rs` | Add `max_concurrent_heavy_requests` + `admission_retry_after_secs` fields, defaults, `set_default`s, validation (none required beyond type), and a defaults test. |
| `antenna-model/src/api/middleware.rs` | New `ConcurrencyLimit` middleware (holds `Arc<Semaphore>` + retry-after secs + configured limit); on `try_acquire` failure returns a `503` `Response` with `Retry-After` header and JSON `ErrorResponse` body. |
| `antenna-model/src/api/routes.rs` | Build one shared `Arc<Semaphore>` (only when limit > 0), wrap the three heavy endpoints with `.with(ConcurrencyLimit::clone())`; thread the limit + retry-after through `build_app`. Add a `create_routes_with_concurrency_limit` test builder. |
| `antenna-model/src/api/mod.rs` | In `start_server_with_config`, apply `worker_threads` via `build_global` (guarded); update the "Performance configuration" log to report the applied thread count + admission limit. |
| `antenna-model/src/api/routes.rs` | Deterministic wiring test: call `build_app` with an already-exhausted semaphore so each heavy endpoint 503s *before its handler runs* and cheap endpoints don't. **(Supersedes the originally-planned reqwest/`sleep` integration test — see the note below.)** |
| `antenna-model/src/config/settings.rs` unit tests + `config/service.yaml` | Document both new knobs in the shipped example config. |
| `docs/api-documentation.md`, `openapi.yaml` | Document the 503 response + `Retry-After` on the three heavy endpoints (standing rule 4 — openapi is hand-maintained). |

No physics/model files touched. No change to S1/S2/S3 behavior.

---

## Steps (TDD — write the test first, watch it fail, then implement)

### Step 1 — Failing config test
In `settings.rs` tests: assert `PerformanceConfig::default().max_concurrent_heavy_requests`
and `admission_retry_after_secs` equal their defaults, and that a YAML override round-trips.
Watch it fail to compile (fields don't exist) → add the fields → green.

### Step 2 — Wiring test (deterministic)

> **Design correction (made during implementation).** The original plan called for a
> reqwest/`sleep` integration test that held one real heavy request in flight and raced a
> second against it. That was built, and it **starved unrelated batch tests in the
> full-suite run** (all cores pinned by the heavy heatmap) and was inherently
> environment-dependent (sleep + core count + machine load). It was deleted. The
> mechanism is already covered deterministically by the middleware unit tests (Step 3);
> the only thing an e2e test adds is proof of *wiring* — which endpoints carry the
> limiter — and that can be proven deterministically: `try_acquire` on an **exhausted**
> semaphore fails *immediately, before the handler runs*.

A `routes.rs` unit test (`poem::test::TestClient`, in-process) builds the app via
`build_app` with a semaphore whose only permit is pre-held, then asserts:
1. each heavy endpoint (`gain/batch`, `heatmap`, `h3-heatmap`) returns **503** with
   `error == "service_overloaded"`, a `Retry-After` header, and `content-type:
   application/json` — with an empty body, since the limiter rejects before body parsing;
2. cheap endpoints are untouched: `/api/v1/gain` (empty body) is a 4xx but **not** 503,
   and `/health` / `/status` are 200.

No `sleep`, no heatmap/batch compute, no core/load dependence. This required making
`build_app` accept an injectable `Option<Arc<Semaphore>>` (production builds it from the
config limit via a small `heavy_semaphore(limit)` helper; the test injects an exhausted one).

### Step 3 — `ConcurrencyLimit` middleware (`api/middleware.rs`)
- Struct holds `Arc<Semaphore>`, `retry_after_secs: u64`, `limit: usize` (for the error
  message). `Middleware`/`Endpoint` impl mirrors `RequestTimeoutImpl`.
- `call`: `match self.sem.try_acquire()` — `Ok(_permit)` → hold the permit for the duration
  of `self.ep.call(req).await` (bind it, don't drop early), map to response; `Err(_)` →
  build the 503. Log a `warn!` with request_id + path + limit, mirroring the 413/504 logs.
- **503 construction differs from 413/504:** those return `Err(poem::Error::from_string(..))`
  which cannot carry a custom header. Build a `poem::Response` directly instead — set status
  `SERVICE_UNAVAILABLE`, `Retry-After` header, `content-type: application/json`, body =
  `serde_json::to_string(&ErrorResponse::new("service_overloaded", msg))` — and return
  `Ok(response)`. Returning `Ok` (not `Err`) means it flows back out through RequestLogger
  (logged as a real 503) and bypasses ErrorHandler's error transform, which is what lets the
  header survive. Pin the header + content-type in the Step 2 test so this can't silently
  regress.

### Step 4 — Wire routes (`api/routes.rs`)
- Extend `build_app` params with `heavy_concurrency_limit: usize` and `retry_after_secs: u64`.
- When `heavy_concurrency_limit > 0`, construct `let sem = Arc::new(Semaphore::new(limit));`
  and apply `.with(ConcurrencyLimit::new(sem.clone(), retry_after_secs, limit))` to **each**
  of the three heavy `post(...)` endpoints (per-endpoint `.with`, sharing `sem`). When `== 0`,
  register them unwrapped (unlimited). Keep the existing outer middleware chain unchanged.
- `create_routes` reads both values from `state.config.performance`. Add
  `create_routes_with_concurrency_limit(state, limit, retry_after)` for tests.

### Step 5 — Apply `worker_threads` (`api/mod.rs`)
In `start_server_with_config`, before `routes::create_routes`:
```rust
let wt = config.performance.worker_threads;
if wt > 0 {
    if let Err(e) = rayon::ThreadPoolBuilder::new().num_threads(wt).build_global() {
        tracing::warn!("worker_threads={wt} requested but global rayon pool already \
                        initialized ({e}); using existing pool");
    }
}
```
Update the "Performance configuration" `info!` to report the effective thread count
(`rayon::current_num_threads()`) and the admission limit. Add a small unit/integration check
that `build_global` is invoked (or at least that a configured `worker_threads` doesn't error
the startup path) — note `build_global` is process-global, so avoid asserting exact counts
across the test binary; a targeted test that calls the wiring helper once and tolerates the
"already initialized" `Err` is the honest bound.

### Step 6 — Docs + spec (standing rule 4)
- `openapi.yaml`: add a `503` response with a `Retry-After` header to the three heavy
  endpoints, referencing the shared `ErrorResponse` schema; note `service_overloaded`.
- `docs/api-documentation.md`: document admission control, the 503, `Retry-After`, and both
  new config knobs.
- `config/service.yaml`: add commented `max_concurrent_heavy_requests` and
  `admission_retry_after_secs` with the defaults and a one-line explanation.

### Step 7 — Full gate
`scripts/check.sh` (fmt + `clippy --workspace --all-targets -D warnings` + `cargo test
--workspace`). Confirm `concurrent_tests.rs` stays green (off-by-default guarantees this;
if on-by-default is chosen, confirm the cap exceeds every test's concurrency).

---

## Exit criteria (definition of done — from the work unit)

1. `performance.worker_threads` is applied via `build_global` at startup (guarded against
   the already-initialized `Err`); the startup log reports the effective thread count.
2. A semaphore caps concurrent heavy requests (batch/heatmap/h3-heatmap); saturation →
   **`503` + `Retry-After`** with the standard JSON `ErrorResponse` body and
   `content-type: application/json`; the limit is configurable.
3. Cheap endpoints (health/ready/status/single-gain/listings) are never admission-rejected.
4. Tests: config defaults; 503-under-saturation with header assertions; cheap-endpoint
   negative control; permit-release recovery.
5. `openapi.yaml` + `docs/api-documentation.md` + `config/service.yaml` updated.
6. Full workspace gate green; existing S1/S2/S3 and concurrency tests unchanged.

---

## Watch-outs

- **Do not reuse 504 or drop `Retry-After`** — the whole point of S4 vs S2 is the
  transient-vs-deterministic distinction. The `Retry-After` header only survives if the 503
  is returned as an `Ok(Response)`, not a `poem::Error::from_string` (which has no header
  channel). Assert the header in the test.
- **Permit lifetime:** bind the `try_acquire` guard to a variable that lives across the
  `.await`; a dropped-too-early permit defeats the limit. `try_acquire` returns a
  `SemaphorePermit` borrowing the semaphore — since the semaphore is behind `Arc`, prefer
  `try_acquire_owned` (needs `Arc<Semaphore>`) to avoid lifetime friction in the middleware
  struct.
- **`build_global` is process-global and once-only.** Never `unwrap` it; the test binary may
  already hold a pool. Log-and-continue on `Err`.
- **Heavy-endpoint set is exactly three.** Do not throttle `/api/v1/gain` (single) — it is on
  the cheap path and p95-latency-sensitive.
- **openapi is hand-maintained** until C7 — mirror the 503 manually now or it drifts.
