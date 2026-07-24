# Plan — S5: Real graceful shutdown, readiness lifecycle, honor `fail_fast`

> **For agentic workers:** REQUIRED SUB-SKILL: use `subagent-driven-development` (recommended)
> or `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the service's startup and shutdown lifecycle honest — readiness starts false and
flips true only after calibration data actually loads; `calibration.fail_fast` aborts startup
instead of being silently swallowed; a shutdown signal flips readiness false, drains in-flight
requests under a bounded timeout, and actually runs `shutdown_cleanup()`.

**Architecture:** No new subsystems. Three surgical seams in `api/mod.rs` — a testable
`initialize_repository()` that classifies the load outcome, a testable `begin_shutdown()` that
performs the readiness flip and pre-drain delay, and a `shutdown_cleanup()` call that finally has
a caller — plus two new `ServerConfig` knobs and honest `/health` reporting.

**Tech Stack:** Rust 2021, poem (`run_with_graceful_shutdown`), tokio signals, `AtomicBool`
readiness in `AppState`, `config` crate for settings.

**User decisions (already made):**
- "Merge S4 now, as its own PR" → done, `27ae7ed` / PR #13. S5 branches off fresh `main`.
- "Write a plan to complete S5 and close out the current phase" → S5 is the last open Phase 2
  unit (S1, S2, S3, S6 merged; S7 superseded by C8), so this plan also carries the phase-closeout
  bookkeeping (Task 6).

Roadmap: `docs/roadmap-2026-07-work-units.md` → Phase 2 → **S5** (Effort: M).
Companion prior units: `plan-s1-body-size-limit.md`, `plan-s2-request-timeout.md`,
`plan-s3-integration-budget.md`, `plan-s4-admission-control.md`.
Depends on **S4** (same startup code region — merged, so no stacking).

All line references verified against HEAD `27ae7ed` on 2026-07-23. Re-verify before editing.

---

## Problem (four independent lifecycle lies)

1. **Readiness is a lie from the first instant.** `AppState::new` sets
   `ready: Arc::new(AtomicBool::new(true))` (`api/mod.rs:72`, comment: *"Default to ready for
   simple deployments"*). `/ready` therefore returns 200 before calibration data has loaded —
   the exact window a readiness probe exists to cover. Nothing ever calls `mark_ready()` on the
   production path, because nothing needs to.

2. **`calibration.fail_fast` is honored one layer down and thrown away one layer up.**
   `CalibrationRepository::load_from_config` *does* respect it (`data/repository.rs:103` returns
   early on the first error when set). But `start_server_with_config` catches that `Err`
   unconditionally (`api/mod.rs:179-186`), logs a `warn!`, and proceeds with
   `CalibrationRepository::new()` — an empty repository — **regardless of `fail_fast`**. The
   default is `fail_fast: true` (`settings.rs:197`, `config/service.yaml:25`), so the shipped
   config asks to abort on load failure and gets a silently-degraded server instead. Every gain
   request then 404s while `/health` and `/ready` both report 200.

3. **`shutdown_cleanup()` has no caller.** It is `pub async fn` at `api/mod.rs:345-360`, fully
   written, and dead. The graceful-shutdown future (`api/mod.rs:234-238`) only logs
   `"Graceful shutdown initiated"`. Readiness is never flipped false on the way down, so a
   Kubernetes load balancer keeps routing new traffic into a pod that is already draining.

4. **The drain is unbounded.** `run_with_graceful_shutdown(app, signal, None)` (`api/mod.rs:239`)
   passes `None` for the drain timeout — poem waits indefinitely for in-flight requests. The
   shipped chart sets `terminationGracePeriodSeconds: 30`
   (`helm/antenna-model/templates/deployment.yaml:103`), so a single slow request (a heatmap with
   an S3 budget of up to 30 s) can outlast the grace period and take a `SIGKILL` mid-flight,
   skipping cleanup entirely.

---

## Design decisions (made — do not re-litigate)

### D1 — Readiness starts `false`; production flips it true after the load
Flip the `AppState::new` initializer to `AtomicBool::new(false)` and mark ready in
`start_server_with_config` only on a healthy load. This is the whole point of exit criterion 1.

**In-scope mechanical fallout** (derived by grep, not memory — update these and keep going, do
**not** stop to ask for scope approval):
- `api/handlers.rs:1129-1131` (`test_app_state_readiness`, in that file's `mod tests`) asserts
  `state.is_ready()` on a fresh state → invert to `assert!(!state.is_ready())`.
- `api/routes.rs:575` (`test_all_endpoints_present`) GETs `/ready` expecting 200 from a
  `with_defaults()` state → add `state.mark_ready();` before `create_routes`.
- `tests/integration/helpers.rs:92-95` (`TestServer::start_inner`) builds `AppState::new(...)`
  after a successful repo load and never marks ready → add `state.mark_ready();`. This
  *mirrors* the production sequencing rather than papering over it, and is what keeps
  `api_tests.rs:35` (`test_ready_endpoint`, asserts `status == "ready"`) green.
- `api/routes.rs:497-499` (`test_ready_route_when_ready`) and `:589` (`test_readiness_transitions`)
  already call `mark_ready()` first — no change.

### D2 — `fail_fast` aborts startup via the existing `Result`, no new error type
`start_server_with_config` already returns `Result<(), std::io::Error>` and `main.rs:64-67`
already does `error!(...); std::process::exit(1)` on `Err`. So "exit nonzero at startup" needs
**no new plumbing** — just return an `io::Error` instead of swallowing. Do not add a new error
enum, do not call `std::process::exit` from inside `api/`.

### D3 — Degraded ≠ dead: `/health` stays 200, `/ready` goes 503
Kubernetes semantics, and this is the one place it is easy to get catastrophically wrong: a
failing **liveness** probe restarts the pod. A calibration-load failure is not fixed by a
restart, so returning non-200 from `/health` would produce an infinite `CrashLoopBackOff`.
Therefore:
- `/health` → **always HTTP 200**. Reflect the degraded state in the existing `status` string
  field only (`"healthy"` → `"degraded"`). Response *shape* unchanged, per the work unit's
  explicit "keep the existing response shapes" gotcha.
- `/ready` → 503 with the existing `{"status": "not_ready"}` body. Unchanged shape.

### D4 — `/health` derives "degraded" from `repository.antenna_count() == 0`, not a flag
Two candidate mechanisms: a new `degraded: Arc<AtomicBool>` on `AppState` set by startup, or
deriving it from the repository being empty. **Chosen: derive.** An empty repository *is* the
degraded condition — the service cannot answer a single gain request — and the derive needs no
new mutable state to keep in sync, stays correct during shutdown drain (where readiness is false
but the data is fine), and also covers the "zero antennas enabled in config" case that a
load-failure flag would miss.

Accepted cost: `api/routes.rs:482-493` (`test_health_route`) builds a `with_defaults()` state
with an empty repository and asserts `status == "healthy"`; it becomes `"degraded"`. That is one
assertion update and it is *honest* — a service with no antennas loaded is degraded. All six
integration-test call sites asserting `"healthy"` (`api_tests.rs:23`, `error_tests.rs:640`,
`resilience_tests.rs:219/266/306/373/401/467`, `concurrent_tests.rs:357`) run against the loaded
fixture repository and stay green.

### D5 — Distinguish "zero enabled" from "all failed" by error *message*, not by shape
`load_from_config` returns the same `ConfigurationError { reason: "No calibrations loaded" }`
whether the config had zero enabled antennas or N enabled antennas that all failed
(`repository.rs:118-122`). The work unit calls out this distinction. Fix it by branching the
**message** only — keep the `Ok`/`Err` shape identical, so the tests that pin current behavior
(`error_tests.rs:79` `fail_fast: true`, `:134` `fail_fast: false`, `repository.rs:708/742`) stay
green by construction. Grep confirms the string `"No calibrations loaded"` appears in exactly one
place (`repository.rs:120`) and is asserted by no test.

### D6 — Two new `ServerConfig` knobs, defaults coherent with the shipped chart
- `shutdown_readiness_delay_secs: u64` — how long to keep serving after the readiness flip,
  before the drain begins, so load balancers observe the flip. **Default `0`.** Rationale: a
  nonzero default makes local `Ctrl+C` feel hung and slows any test that exercises the path;
  and on real pod *deletion* Kubernetes removes the pod from Endpoints immediately on entering
  `Terminating`, independent of the probe. Document `5` as the recommended production value for
  LB-propagation margin. Be honest in the docs about the limit: the shipped chart probes `/ready`
  every 5 s with `failureThreshold: 3` (`helm/antenna-model/values.yaml:121-129`), so a purely
  *probe-driven* removal can take up to 15 s — the delay covers propagation, it does not
  guarantee probe-observed removal.
- `shutdown_timeout_secs: u64` — bounded drain, replacing the current `None`. **Default `25`.**
  Chosen so `shutdown_readiness_delay_secs (5, recommended) + 25 ≤ 30 =
  terminationGracePeriodSeconds`, leaving cleanup room before `SIGKILL`. Do not default this to
  30: that consumes the entire grace period and cleanup never runs.

### D7 — Populate `antenna_ids` at startup (in scope, 3 lines)
`AppState::set_antenna_ids` (`api/mod.rs:115`) is called only by `routes.rs` tests — **production
never populates it**, so `/status` in production always omits `antenna_count` and `antenna_ids`
(they are `skip_serializing_if = "Option::is_none"`, `schemas.rs:1009-1014`). Since exit
criterion 2 requires `/status` to reflect the degraded state, populate it from
`repository.list_antennas()` right after the load. Directly serves the criterion; three lines.

---

## Files to change

| File | Change |
|---|---|
| `antenna-model/src/api/mod.rs` | `AppState::new` readiness `false`; new `LoadOutcome` enum + `initialize_repository()`; new `begin_shutdown()`; wire both into `start_server_with_config`; bounded drain; call `shutdown_cleanup`; populate `antenna_ids`; update tests. |
| `antenna-model/src/data/repository.rs` | Branch the `loaded_count == 0` error message: zero-enabled vs all-failed (D5). Add a unit test per branch. |
| `antenna-model/src/api/schemas.rs` | `HealthResponse::degraded()` constructor + test. Shape unchanged. |
| `antenna-model/src/api/handlers.rs` | `/health` takes `Data<&Arc<AppState>>` and reports `healthy`/`degraded`; doc-comment update; fix `test_app_state_readiness`. |
| `antenna-model/src/api/routes.rs` | Fix `test_health_route` (D4) and `test_all_endpoints_present` (D1). |
| `antenna-model/src/config/settings.rs` | Two new `ServerConfig` fields + `default_*` fns + `Default` impl + `set_default`s + defaults/round-trip tests. |
| `antenna-model/tests/integration/helpers.rs` | `state.mark_ready()` after a successful load (D1). |
| `config/service.yaml` | Document both new knobs. |
| `openapi.yaml` | `/health` `degraded` status; fix the pre-existing `/ready` 503 schema drift (spec says `ErrorResponse`, code returns `{"status":"not_ready"}`). |
| `docs/api-documentation.md` | Lifecycle section: readiness semantics, degraded state, shutdown sequence, both knobs. |
| `docs/roadmap-2026-07-work-units.md` | Mark S5 done; mark Phase 2 closed. |

No physics/model files touched. No change to S1/S2/S3/S4 behavior.

---

## Task 1 — Readiness starts false and flips true only after the load

**Goal:** `/ready` returns 503 until calibration data has actually loaded.

**Files:**
- Modify: `antenna-model/src/api/mod.rs:72` (initializer), `:46-48` (doc comment)
- Modify: `antenna-model/src/api/handlers.rs` — `test_app_state_readiness` (grep
  `assert!(state.is_ready())`, ~line 1129)
- Modify: `antenna-model/src/api/routes.rs` — `test_all_endpoints_present` (~line 575)
- Modify: `antenna-model/tests/integration/helpers.rs:92-95`
- Test: `antenna-model/src/api/mod.rs` tests module (new `test_app_state_starts_not_ready`)

**Acceptance Criteria:**
- [ ] A freshly constructed `AppState` reports `is_ready() == false`.
- [ ] `mark_ready()` / `mark_not_ready()` still round-trip.
- [ ] `TestServer`-backed integration tests still see `/ready` → 200 (helpers marks ready after
      a successful load, mirroring production).
- [ ] `cargo test -p antenna-model` green.

**Verify:** `cargo test -p antenna-model --lib api::` → all pass; then
`cargo test -p antenna-model --test integration_tests ready` → `test_ready_endpoint` passes.

**Steps:**

- [ ] **Step 1: Write the failing test** in the `mod tests` block of `antenna-model/src/api/mod.rs`

```rust
    #[test]
    fn test_app_state_starts_not_ready() {
        // Readiness is a startup lifecycle signal (roadmap S5): it must be false until
        // calibration data has actually loaded. A default-constructed state has loaded
        // nothing, so it must not advertise readiness.
        let state = AppState::with_defaults();
        assert!(
            !state.is_ready(),
            "AppState must start NOT ready; readiness is set only after the calibration load"
        );

        state.mark_ready();
        assert!(state.is_ready());
        state.mark_not_ready();
        assert!(!state.is_ready());
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p antenna-model --lib test_app_state_starts_not_ready`
Expected: FAIL — `AppState must start NOT ready; readiness is set only after the calibration load`

- [ ] **Step 3: Flip the initializer** in `antenna-model/src/api/mod.rs`

Replace line 72:

```rust
            ready: Arc::new(AtomicBool::new(true)), // Default to ready for simple deployments
```

with:

```rust
            // Readiness starts FALSE (roadmap S5). It flips true only after
            // `start_server_with_config` completes a healthy calibration load, and flips
            // back to false at the top of graceful shutdown. Constructing a state is not
            // evidence that the service can serve anything.
            ready: Arc::new(AtomicBool::new(false)),
```

And update the struct field doc at `:46-48`:

```rust
    /// Readiness state — false until the calibration load completes, true while serving,
    /// false again once graceful shutdown begins (roadmap S5).
    pub ready: Arc<AtomicBool>,
```

- [ ] **Step 4: Fix the three mechanical call sites**

In `antenna-model/src/api/handlers.rs`, `test_app_state_readiness`:

```rust
    #[test]
    fn test_app_state_readiness() {
        let state = AppState::with_defaults();

        // Starts NOT ready (roadmap S5) — readiness is earned by a successful load.
        assert!(!state.is_ready());

        // Mark ready
        state.mark_ready();
        assert!(state.is_ready());

        // Mark not ready
        state.mark_not_ready();
        assert!(!state.is_ready());
    }
```

In `antenna-model/src/api/routes.rs`, `test_all_endpoints_present`:

```rust
    #[tokio::test]
    async fn test_all_endpoints_present() {
        let state = Arc::new(AppState::with_defaults());
        state.mark_ready(); // S5: readiness starts false; this test asserts /ready == 200
        let app = create_routes(state);
        let cli = TestClient::new(app);
```

In `antenna-model/tests/integration/helpers.rs`, inside `start_inner`, immediately after the
`AppState::new` line:

```rust
        let state = Arc::new(AppState::new(config.clone(), repository));
        // Mirror the production startup sequence (roadmap S5): the repository above loaded
        // successfully, so the service is ready. Without this, /ready would 503 for the
        // whole test run.
        state.mark_ready();
```

- [ ] **Step 5: Run the suite**

Run: `cargo test -p antenna-model`
Expected: PASS (all previously-green tests plus the new one)

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/api/mod.rs antenna-model/src/api/handlers.rs \
        antenna-model/src/api/routes.rs antenna-model/tests/integration/helpers.rs
git commit -m "feat(S5): readiness starts false, flips true only after calibration load"
```

---

## Task 2 — Distinguish "no antennas enabled" from "all enabled antennas failed"

**Goal:** The zero-calibrations error says *which* zero it is, so the startup log and the
operator can tell a misconfiguration from a broken artifact.

**Files:**
- Modify: `antenna-model/src/data/repository.rs:113-122`
- Test: `antenna-model/src/data/repository.rs` tests module (two new tests)

**Acceptance Criteria:**
- [ ] Zero enabled antennas → `Err` whose message names the empty-configuration case.
- [ ] N enabled antennas that all fail (with `fail_fast: false`) → `Err` whose message reports
      how many failed.
- [ ] The `Ok`/`Err` *shape* is unchanged — every existing repository and `error_tests.rs`
      assertion stays green.

**Verify:** `cargo test -p antenna-model --lib data::repository` → all pass; then
`cargo test -p antenna-model --test integration_tests error_` → unchanged.

**Steps:**

- [ ] **Step 1: Write the failing tests** in the `mod tests` block of `repository.rs`

These use a temp `antennas.yaml`. Follow the existing fixture style in that module
(`repository.rs:700-750` builds `CalibrationConfig` literals).

```rust
    #[test]
    fn test_load_from_config_reports_zero_enabled_distinctly() {
        // A config with no *enabled* antennas is an operator misconfiguration, not a
        // broken artifact. The error must say so (roadmap S5) — start_server_with_config
        // surfaces this string in the startup log.
        let dir = std::env::temp_dir().join("s5_zero_enabled");
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("antennas.yaml");
        std::fs::write(
            &cfg_path,
            r#"
antennas:
  - antenna_id: "disabled_one"
    enabled: false
    calibration_file: "nope.bin"
    feeds:
      - feed_id: "primary"
"#,
        )
        .expect("write temp antenna config");

        let config = CalibrationConfig {
            data_directory: dir.clone(),
            antenna_config_file: cfg_path,
            fail_fast: false,
        };

        let err = CalibrationRepository::load_from_config(&config)
            .expect_err("zero enabled antennas must be an error");
        let msg = err.to_string();
        assert!(
            msg.contains("No antennas enabled"),
            "expected the zero-enabled message, got: {msg}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_from_config_reports_all_failed_distinctly() {
        // One enabled antenna pointing at a missing artifact, fail_fast off: loading
        // continues, ends with zero loaded, and the error must distinguish this from the
        // zero-enabled case above.
        let dir = std::env::temp_dir().join("s5_all_failed");
        let _ = std::fs::create_dir_all(&dir);
        let cfg_path = dir.join("antennas.yaml");
        std::fs::write(
            &cfg_path,
            r#"
antennas:
  - antenna_id: "broken_one"
    enabled: true
    calibration_file: "absent.bin"
    feeds:
      - feed_id: "primary"
"#,
        )
        .expect("write temp antenna config");

        let config = CalibrationConfig {
            data_directory: dir.clone(),
            antenna_config_file: cfg_path,
            fail_fast: false,
        };

        let err = CalibrationRepository::load_from_config(&config)
            .expect_err("all-failed must be an error");
        let msg = err.to_string();
        assert!(
            msg.contains("failed to load"),
            "expected the all-failed message, got: {msg}"
        );
        assert!(
            !msg.contains("No antennas enabled"),
            "all-failed must not be reported as zero-enabled: {msg}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
```

> **Note for the executor:** the exact YAML keys above (`antenna_id`, `enabled`,
> `calibration_file`, `feeds[].feed_id`) are copied from `error_tests.rs:120-127` and
> `calibration_data/antennas.yaml`. If `AntennaConfig` deserialization rejects them, read
> `settings.rs::AntennaConfigEntry` and match its actual `serde` field names — do not guess.

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p antenna-model --lib s5_ 2>&1 | tail -20` — or by name:
`cargo test -p antenna-model --lib test_load_from_config_reports`
Expected: FAIL — both get `"No calibrations loaded"`

- [ ] **Step 3: Branch the message** — replace `repository.rs:118-122`

```rust
        if loaded_count == 0 {
            // Distinguish the two zero-calibration cases (roadmap S5): an operator who
            // enabled nothing, versus enabled entries that all failed to load. Same Err
            // shape, different message — start_server_with_config logs it verbatim.
            let reason = if enabled_count == 0 {
                "No antennas enabled in configuration".to_string()
            } else {
                format!(
                    "All {} enabled antenna(s) failed to load ({} error(s))",
                    enabled_count, error_count
                )
            };
            return Err(DataError::ConfigurationError { reason });
        }
```

This needs the enabled count captured before the loop consumes `enabled_antennas`. Immediately
after `let enabled_antennas = antenna_config.enabled_antennas();` (`repository.rs:86`), add:

```rust
        let enabled_count = enabled_antennas.len();
```

and leave the existing `info!` that already reads `enabled_antennas.len()` — or switch it to
`enabled_count` if the borrow checker complains about the later move.

- [ ] **Step 4: Run to green**

Run: `cargo test -p antenna-model --lib test_load_from_config_reports`
Expected: PASS

Run: `cargo test -p antenna-model --test integration_tests error_`
Expected: PASS — unchanged, since only the message changed

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/data/repository.rs
git commit -m "feat(S5): distinguish zero-enabled from all-failed in the calibration load error"
```

---

## Task 3 — Honor `fail_fast` at startup; degraded state on `/health` and `/status`

**Goal:** `fail_fast: true` + a failed load exits the process nonzero. `fail_fast: false` starts
a server that says so: `/ready` 503, `/health` 200 `"degraded"`, `/status` reflects the empty set.

**Files:**
- Modify: `antenna-model/src/api/mod.rs:165-195` (`start_server_with_config`), plus a new
  `LoadOutcome` enum and `initialize_repository()` above `apply_worker_threads`
- Modify: `antenna-model/src/api/schemas.rs:984-991` (add `degraded()`)
- Modify: `antenna-model/src/api/handlers.rs:44-46` (`health` handler) and its doc comment
  `:30-43`
- Modify: `antenna-model/src/api/routes.rs:482-493` (`test_health_route`)
- Test: `antenna-model/src/api/mod.rs` tests module (three new tests),
  `antenna-model/src/api/schemas.rs` tests module (one new test)

**Acceptance Criteria:**
- [ ] `initialize_repository` with a bad config + `fail_fast: true` → `Err(io::Error)`.
- [ ] Same config with `fail_fast: false` → `Ok((empty_repo, LoadOutcome::Degraded))`.
- [ ] A good config → `Ok((repo, LoadOutcome::Healthy))` with `antenna_count() > 0`.
- [ ] `/health` returns **200** with `status == "degraded"` when the repository is empty, and
      `status == "healthy"` when it is not. Never non-200.
- [ ] `main.rs` exits nonzero on the `fail_fast` path — via the existing `Err` → `exit(1)`
      path, no new code in `main.rs`.

**Verify:** `cargo test -p antenna-model --lib api::` → all pass, including the three new
`initialize_repository` tests.

**Steps:**

- [ ] **Step 1: Write the failing tests** in `antenna-model/src/api/mod.rs` tests module

```rust
    /// Build a CalibrationConfig pointing at a nonexistent antenna config file.
    fn broken_calibration_config(fail_fast: bool) -> crate::config::CalibrationConfig {
        crate::config::CalibrationConfig {
            data_directory: std::env::temp_dir().join("s5_does_not_exist"),
            antenna_config_file: std::env::temp_dir().join("s5_does_not_exist/antennas.yaml"),
            fail_fast,
        }
    }

    #[test]
    fn test_initialize_repository_fail_fast_returns_err() {
        // calibration.fail_fast = true means "refuse to start on a load failure". Before
        // S5 this Err was swallowed and the server booted empty (roadmap S5, problem 2).
        let err = initialize_repository(&broken_calibration_config(true))
            .expect_err("fail_fast must propagate the load failure to the caller");
        let msg = err.to_string();
        assert!(
            msg.contains("fail_fast"),
            "the startup error must name the knob that caused the abort, got: {msg}"
        );
    }

    #[test]
    fn test_initialize_repository_without_fail_fast_starts_degraded() {
        let (repo, outcome) = initialize_repository(&broken_calibration_config(false))
            .expect("fail_fast=false must start the server anyway");
        assert_eq!(outcome, LoadOutcome::Degraded);
        assert_eq!(
            repo.antenna_count(),
            0,
            "a degraded start serves from an empty repository"
        );
    }

    #[test]
    fn test_initialize_repository_healthy_on_real_fixtures() {
        // The checked-in fixtures load four uncalibrated design-spec antennas.
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixtures = std::path::PathBuf::from(manifest_dir).join("tests/fixtures");
        let config = crate::config::CalibrationConfig {
            data_directory: fixtures.clone(),
            antenna_config_file: fixtures.join("test_antennas.yaml"),
            fail_fast: true,
        };

        let (repo, outcome) =
            initialize_repository(&config).expect("fixture config must load cleanly");
        assert_eq!(outcome, LoadOutcome::Healthy);
        assert!(
            repo.antenna_count() > 0,
            "a healthy load must produce a non-empty repository"
        );
    }
```

And in `antenna-model/src/api/schemas.rs` tests module:

```rust
    #[test]
    fn test_health_response_degraded() {
        // Shape is unchanged (one `status` field) — only the value differs. The work unit
        // explicitly requires keeping the /health response shape.
        let response = HealthResponse::degraded();
        assert_eq!(response.status, "degraded");

        let json = serde_json::to_value(&response).expect("serialize");
        assert_eq!(json["status"], "degraded");
        assert_eq!(
            json.as_object().map(|o| o.len()),
            Some(1),
            "HealthResponse must stay a single-field object"
        );
    }
```

- [ ] **Step 2: Run and watch them fail**

Run: `cargo test -p antenna-model --lib test_initialize_repository`
Expected: FAIL to compile — `cannot find function 'initialize_repository'`, `LoadOutcome` not
found, `HealthResponse::degraded` not found

- [ ] **Step 3: Add `LoadOutcome` + `initialize_repository`** to `antenna-model/src/api/mod.rs`,
      just above `apply_worker_threads`

```rust
/// Outcome of the startup calibration load (roadmap S5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOutcome {
    /// At least one calibration loaded — the service can answer gain requests.
    Healthy,
    /// Nothing loaded and `calibration.fail_fast` is off. The server starts so that
    /// `/health` and `/status` can report *why* it is useless, but readiness stays false
    /// so no load balancer routes real traffic to it.
    Degraded,
}

/// Load the calibration repository and classify the outcome (roadmap S5).
///
/// This is the seam that makes `calibration.fail_fast` real. Before S5,
/// `start_server_with_config` caught the load error unconditionally and continued with an
/// empty repository, so the shipped `fail_fast: true` default was silently ignored.
///
/// # Returns
/// * `Ok((repo, Healthy))` — at least one calibration loaded.
/// * `Ok((empty, Degraded))` — load failed but `fail_fast` is off; start anyway, not ready.
/// * `Err(io::Error)` — load failed and `fail_fast` is on. The caller returns this up to
///   `main`, which logs it and exits nonzero. No `process::exit` inside the API layer.
pub fn initialize_repository(
    config: &crate::config::CalibrationConfig,
) -> Result<(CalibrationRepository, LoadOutcome), std::io::Error> {
    match CalibrationRepository::load_from_config(config) {
        Ok(repo) => {
            info!(
                antenna_count = repo.antenna_count(),
                calibration_count = repo.calibration_count(),
                "Calibration data loaded successfully"
            );
            Ok((repo, LoadOutcome::Healthy))
        }
        Err(e) if config.fail_fast => {
            tracing::error!(
                error = %e,
                "Failed to load calibration data and calibration.fail_fast is set; refusing to start"
            );
            Err(std::io::Error::other(format!(
                "calibration load failed and calibration.fail_fast is enabled: {e}"
            )))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Failed to load calibration data; starting DEGRADED with an empty repository \
                 (readiness stays false, /health reports degraded). Set calibration.fail_fast \
                 to abort startup instead."
            );
            Ok((CalibrationRepository::new(), LoadOutcome::Degraded))
        }
    }
}
```

> `std::io::Error::other` is stable since Rust 1.74. If the toolchain rejects it, use
> `std::io::Error::new(std::io::ErrorKind::InvalidData, format!(...))`.

- [ ] **Step 4: Rewrite the load block** in `start_server_with_config` — replace
      `api/mod.rs:174-186` with

```rust
    let (repository, load_outcome) = initialize_repository(&config.calibration)?;
```

- [ ] **Step 5: Add `HealthResponse::degraded`** in `antenna-model/src/api/schemas.rs`, next to
      `healthy()`

```rust
    /// Create a degraded response.
    ///
    /// The service is alive and responding, but has no calibration data loaded, so it
    /// cannot answer gain requests. Deliberately still served with HTTP 200: `/health` is
    /// the Kubernetes **liveness** probe, and a restart does not fix missing calibration
    /// data — returning non-200 here would produce an endless CrashLoopBackOff. Readiness
    /// (`/ready`) is the signal that keeps traffic away. See roadmap S5, decision D3.
    pub fn degraded() -> Self {
        Self {
            status: "degraded".to_string(),
        }
    }
```

- [ ] **Step 6: Make `/health` state-aware** in `antenna-model/src/api/handlers.rs`

```rust
#[handler]
pub async fn health(state: Data<&Arc<AppState>>) -> Json<HealthResponse> {
    // Always HTTP 200 — this is the liveness probe (see HealthResponse::degraded). An
    // empty repository means no antenna can be evaluated, which is degraded but alive.
    if state.repository.antenna_count() == 0 {
        Json(HealthResponse::degraded())
    } else {
        Json(HealthResponse::healthy())
    }
}
```

Update its doc comment (`handlers.rs:30-43`) to document both values:

```rust
/// # Response
/// Returns HTTP 200 with JSON body containing:
/// - status: "healthy" when calibration data is loaded
/// - status: "degraded" when the service is responsive but has no calibration data loaded
///
/// Always returns 200 — a non-200 liveness response would restart the pod, which cannot
/// fix missing calibration data. Use `/ready` to keep traffic away (roadmap S5).
```

- [ ] **Step 7: Fix `test_health_route`** in `antenna-model/src/api/routes.rs:482-493` — a
      `with_defaults()` state has an empty repository, so it is degraded (D4)

```rust
        assert_eq!(json_value.object().get("status").string(), "degraded");
```

Add a sibling test proving the healthy branch, using the existing `create_test_repository()`
helper already in that module (`routes.rs:~604`):

```rust
    #[tokio::test]
    async fn test_health_route_reports_healthy_with_loaded_antennas() {
        let config = crate::config::ServiceConfig::with_defaults();
        let state = Arc::new(AppState::new(config, create_test_repository()));
        let app = create_routes(state);
        let cli = TestClient::new(app);

        let response = cli.get("/health").send().await;
        response.assert_status_is_ok();

        let body = response.json().await;
        assert_eq!(body.value().object().get("status").string(), "healthy");
    }
```

- [ ] **Step 8: Wire readiness + antenna_ids** into `start_server_with_config`, immediately after
      the `let state = Arc::new(AppState::new(config.clone(), repository));` line

Task 1 left a self-cleaning marker in the `ready` field doc (`api/mod.rs:46-47`) and the
initializer comment (`:72-75`): *"wired in Task 3 of this branch"* / *"wired in Tasks 3 and 4"*.
**Strip the Task 3 half of those markers here**, since this step is what wires it — leave the
Task 4 (shutdown) half for Task 4 to remove.

```rust
    // Publish the loaded set so /status reports it (before S5, production never called
    // set_antenna_ids, so antenna_count/antenna_ids were always omitted — roadmap S5 D7).
    state.set_antenna_ids(state.repository.list_antennas());

    match load_outcome {
        LoadOutcome::Healthy => {
            state.mark_ready();
            info!(
                antenna_count = state.repository.antenna_count(),
                "Service marked READY"
            );
        }
        LoadOutcome::Degraded => {
            tracing::warn!(
                "Service starting DEGRADED: no calibration data loaded. Readiness stays \
                 false and /health reports \"degraded\"; gain requests will 404."
            );
        }
    }
```

- [ ] **Step 9: Run to green**

Run: `cargo test -p antenna-model --lib`
Expected: PASS

Run: `cargo test -p antenna-model`
Expected: PASS — integration tests load fixtures, so `/health` stays `"healthy"` there

- [ ] **Step 10: Commit**

```bash
git add antenna-model/src/api/mod.rs antenna-model/src/api/schemas.rs \
        antenna-model/src/api/handlers.rs antenna-model/src/api/routes.rs
git commit -m "feat(S5): honor calibration.fail_fast at startup; report degraded state honestly"
```

---

## Task 4 — Real graceful shutdown: readiness flip, bounded drain, cleanup invoked

**Goal:** A shutdown signal flips readiness false, optionally waits for load balancers to notice,
drains in-flight requests under a bounded timeout, then actually runs `shutdown_cleanup()`.

**Files:**
- Modify: `antenna-model/src/config/settings.rs` — `ServerConfig` fields, `default_*` fns,
  `Default` impl (`:238-247`), `from_file` `set_default`s (`:305-312` region)
- Modify: `antenna-model/src/api/mod.rs:230-241` (shutdown wiring) + new `begin_shutdown`
- Modify: `config/service.yaml`
- Test: `antenna-model/src/config/settings.rs` tests module,
  `antenna-model/src/api/mod.rs` tests module

**Acceptance Criteria:**
- [ ] `ServerConfig::default().shutdown_readiness_delay_secs == 0` and
      `shutdown_timeout_secs == 25`; both round-trip through a YAML override.
- [ ] `begin_shutdown` flips readiness from true to false.
- [ ] `begin_shutdown` with a nonzero delay actually waits (assert elapsed ≥ the delay), using
      tokio's paused clock so the test costs no wall-clock time.
- [ ] `start_server_with_config` passes `Some(drain_timeout)` — not `None` — to
      `run_with_graceful_shutdown`, and calls `shutdown_cleanup(&state)` after it returns,
      including when the server returned an error.
- [ ] `config/service.yaml` documents both knobs.

**Verify:** `cargo test -p antenna-model --lib config::settings` and
`cargo test -p antenna-model --lib begin_shutdown` → all pass.

**Steps:**

- [ ] **Step 1: Write the failing config test** in `antenna-model/src/config/settings.rs` tests

```rust
    #[test]
    fn test_shutdown_defaults() {
        let server = ServerConfig::default();
        // 0 = flip readiness and drain immediately. Local Ctrl+C stays snappy; operators
        // set ~5 in Kubernetes for load-balancer propagation (roadmap S5, D6).
        assert_eq!(server.shutdown_readiness_delay_secs, 0);
        // 25 s keeps delay + drain inside the chart's terminationGracePeriodSeconds: 30,
        // leaving room for cleanup before SIGKILL.
        assert_eq!(server.shutdown_timeout_secs, 25);
    }

    #[test]
    fn test_shutdown_knobs_round_trip_from_yaml() {
        let yaml = r#"
server:
  host: "0.0.0.0"
  port: 3000
  shutdown_readiness_delay_secs: 7
  shutdown_timeout_secs: 12
calibration:
  data_directory: "calibration_data"
  antenna_config_file: "calibration_data/antennas.yaml"
logging:
  level: "info"
"#;
        let dir = std::env::temp_dir().join("s5_shutdown_cfg");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("service.yaml");
        std::fs::write(&path, yaml).expect("write temp config");

        let config = ServiceConfig::from_file(path.to_string_lossy().as_ref())
            .expect("config must parse");
        assert_eq!(config.server.shutdown_readiness_delay_secs, 7);
        assert_eq!(config.server.shutdown_timeout_secs, 12);

        let _ = std::fs::remove_dir_all(&dir);
    }
```

> **Note for the executor:** mirror whatever style the existing `settings.rs` YAML round-trip
> tests use (there is at least one for the S4 knobs). If they use a different temp-file or
> `Config::builder` idiom, follow it rather than the sketch above.

- [ ] **Step 2: Run and watch it fail**

Run: `cargo test -p antenna-model --lib test_shutdown_defaults`
Expected: FAIL to compile — `no field 'shutdown_readiness_delay_secs' on type 'ServerConfig'`

- [ ] **Step 3: Add the fields** to `ServerConfig` in `antenna-model/src/config/settings.rs`

```rust
    /// Seconds to keep serving after readiness flips false, before the drain begins
    /// (roadmap S5).
    ///
    /// On a shutdown signal the service marks itself not-ready immediately, then waits this
    /// long before poem stops accepting connections, so load balancers have a window to
    /// observe the flip and stop routing new traffic.
    ///
    /// Default `0` (flip and drain immediately) — keeps local `Ctrl+C` snappy. Recommended
    /// production value: `5`. Note the shipped chart probes `/ready` every 5 s with
    /// `failureThreshold: 3`, so a purely probe-driven removal can take up to 15 s; this
    /// delay covers LB propagation, it does not guarantee probe-observed removal. On real
    /// pod deletion Kubernetes drops the pod from Endpoints immediately anyway.
    #[serde(default = "default_shutdown_readiness_delay")]
    pub shutdown_readiness_delay_secs: u64,

    /// Maximum seconds to wait for in-flight requests to drain during shutdown (roadmap S5).
    ///
    /// Before S5 this was unbounded (`None`), so one slow heatmap could outlast the pod's
    /// grace period and take a `SIGKILL` mid-flight, skipping cleanup. Default `25`, chosen
    /// so `shutdown_readiness_delay_secs` (5, recommended) + 25 stays inside the chart's
    /// `terminationGracePeriodSeconds: 30`.
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,
```

Add the default fns next to the other `default_*` fns:

```rust
fn default_shutdown_readiness_delay() -> u64 {
    0 // flip readiness and drain immediately; operators opt into an LB grace window — S5
}

fn default_shutdown_timeout() -> u64 {
    25 // seconds; + a 5 s readiness delay stays under the chart's 30 s grace period — S5
}
```

Extend `impl Default for ServerConfig` (`settings.rs:238-247`):

```rust
impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            request_timeout_secs: default_request_timeout(),
            max_body_size_bytes: default_max_body_size(),
            shutdown_readiness_delay_secs: default_shutdown_readiness_delay(),
            shutdown_timeout_secs: default_shutdown_timeout(),
        }
    }
}
```

And the `from_file` builder, next to the other `server.*` `set_default` calls:

```rust
            .set_default(
                "server.shutdown_readiness_delay_secs",
                default_shutdown_readiness_delay() as i64,
            )?
            .set_default(
                "server.shutdown_timeout_secs",
                default_shutdown_timeout() as i64,
            )?
```

- [ ] **Step 4: Write the failing shutdown-seam test** in `antenna-model/src/api/mod.rs` tests

```rust
    #[tokio::test(start_paused = true)]
    async fn test_begin_shutdown_flips_readiness_and_waits() {
        use std::time::Duration;
        use tokio::time::Instant;

        let state = AppState::with_defaults();
        state.mark_ready();
        assert!(state.is_ready());

        let started = Instant::now();
        begin_shutdown(&state, Duration::from_secs(5)).await;

        assert!(
            !state.is_ready(),
            "shutdown must flip readiness false so load balancers stop routing new traffic"
        );
        assert!(
            started.elapsed() >= Duration::from_secs(5),
            "the pre-drain delay must actually elapse"
        );
    }

    #[tokio::test]
    async fn test_begin_shutdown_zero_delay_does_not_wait() {
        use std::time::Duration;

        let state = AppState::with_defaults();
        state.mark_ready();

        // Default config: no delay. Must still flip readiness, and must not sleep.
        begin_shutdown(&state, Duration::ZERO).await;
        assert!(!state.is_ready());
    }
```

> `start_paused = true` needs tokio's `test-util` feature on the dev-dependency. Check
> `antenna-model/Cargo.toml`; if it is absent, add `test-util` to the `tokio` dev-dependency
> features (do **not** add it to the normal dependency features).

- [ ] **Step 5: Run and watch it fail**

Run: `cargo test -p antenna-model --lib begin_shutdown`
Expected: FAIL to compile — `cannot find function 'begin_shutdown'`

- [ ] **Step 6: Add `begin_shutdown`** to `antenna-model/src/api/mod.rs`, next to
      `shutdown_signal`

```rust
/// First half of graceful shutdown: stop advertising readiness, then pause (roadmap S5).
///
/// Runs *inside* the future handed to `run_with_graceful_shutdown`, because poem stops
/// accepting new connections the moment that future resolves. Flipping readiness and
/// sleeping here — rather than after — is what gives load balancers a window to route new
/// traffic elsewhere while this instance is still able to serve it.
async fn begin_shutdown(state: &AppState, readiness_delay: std::time::Duration) {
    state.mark_not_ready();
    info!(
        readiness_delay_secs = readiness_delay.as_secs(),
        "Readiness set to false; pausing before draining in-flight requests"
    );

    if !readiness_delay.is_zero() {
        tokio::time::sleep(readiness_delay).await;
    }
}
```

- [ ] **Step 7: Wire the real shutdown sequence** — replace `api/mod.rs:230-241` with

This step wires the shutdown half of the readiness lifecycle, so **remove the remaining
"wired in Task 4" marker** Task 1 left in the `ready` field doc (`api/mod.rs:46-47`) and the
initializer comment (`:72-75`). After this task the comments describe behavior that actually
exists, with no forward references left.

```rust
    // Graceful shutdown (roadmap S5): flip readiness false and pause so load balancers
    // stop sending new work, then drain in-flight requests under a bounded timeout, then
    // run cleanup. Before S5 this future only logged, the drain was unbounded (`None`),
    // and `shutdown_cleanup` had no caller at all.
    let readiness_delay =
        Duration::from_secs(state.config.server.shutdown_readiness_delay_secs);
    let drain_timeout = Duration::from_secs(state.config.server.shutdown_timeout_secs);
    let shutdown_state = state.clone();

    info!(
        readiness_delay_secs = readiness_delay.as_secs(),
        drain_timeout_secs = drain_timeout.as_secs(),
        "Graceful shutdown configured"
    );

    let result = Server::new(TcpListener::bind(&addr))
        .run_with_graceful_shutdown(
            app,
            async move {
                shutdown_signal().await;
                info!("Graceful shutdown initiated");
                begin_shutdown(&shutdown_state, readiness_delay).await;
            },
            Some(drain_timeout),
        )
        .await;

    // Runs on both the clean and the errored path — cleanup is exactly what must not be
    // skipped when the server came down badly.
    shutdown_cleanup(&state).await;

    result
```

Add `use std::time::Duration;` to the imports at the top of `api/mod.rs` if not already present.

- [ ] **Step 8: Document both knobs** in `config/service.yaml`, in the `server:` block

```yaml
  # Graceful shutdown (roadmap S5).
  # On SIGTERM/SIGINT the service marks itself not-ready, waits
  # `shutdown_readiness_delay_secs` so load balancers observe the flip, then drains
  # in-flight requests for at most `shutdown_timeout_secs` before running cleanup.
  # Keep delay + timeout below the pod's terminationGracePeriodSeconds (30 in the shipped
  # Helm chart) or the drain gets SIGKILLed and cleanup never runs.
  shutdown_readiness_delay_secs: 0   # default 0; recommended 5 in Kubernetes
  shutdown_timeout_secs: 25          # default 25
```

- [ ] **Step 9: Run to green**

Run: `cargo test -p antenna-model --lib config::settings`
Expected: PASS

Run: `cargo test -p antenna-model`
Expected: PASS

- [ ] **Step 10: Commit**

```bash
git add antenna-model/src/config/settings.rs antenna-model/src/api/mod.rs \
        antenna-model/Cargo.toml config/service.yaml
git commit -m "feat(S5): real graceful shutdown — readiness flip, bounded drain, cleanup invoked"
```

---

## Task 5 — Docs and spec (standing rule 4)

**Goal:** `openapi.yaml` and `docs/api-documentation.md` describe the lifecycle the code now has,
and the pre-existing `/ready` 503 schema drift is corrected.

**Files:**
- Modify: `openapi.yaml:49-83` (`/health`, `/ready`)
- Modify: `docs/api-documentation.md` (new Service Lifecycle section)

**Acceptance Criteria:**
- [ ] `/health` in the spec documents both `healthy` and `degraded`, and states it is always 200.
- [ ] `/ready`'s 503 references the correct body (`{"status": "not_ready"}` — a `HealthResponse`,
      not `ErrorResponse` as the spec currently claims).
- [ ] `api-documentation.md` documents readiness semantics, the degraded state, the shutdown
      sequence, `fail_fast` behavior, and both new config knobs.
- [ ] No behavior change in this task — docs only.

**Verify:** `python3 -c "import yaml,sys; yaml.safe_load(open('openapi.yaml'))"` → no output
(valid YAML). Re-read the two `/health` and `/ready` blocks against `handlers.rs`.

**Steps:**

- [ ] **Step 1: Update `/health` and `/ready`** in `openapi.yaml`

```yaml
  /health:
    get:
      tags:
        - health
      summary: Health check (liveness probe)
      description: |
        Always returns 200 while the process is responsive. Used for the Kubernetes liveness
        probe, so it deliberately never returns a failure status: a restart cannot fix
        missing calibration data.

        The `status` field reports `healthy` when calibration data is loaded, or `degraded`
        when the service is running with an empty repository (calibration load failed and
        `calibration.fail_fast` was off). A degraded instance never becomes ready, so it
        receives no traffic — see `/ready`.
      operationId: getHealth
      responses:
        '200':
          description: Service is responsive ("healthy", or "degraded" with no data loaded)
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/HealthResponse'

  /ready:
    get:
      tags:
        - health
      summary: Readiness check (readiness probe)
      description: |
        Returns 200 only once the calibration load has completed successfully. Returns 503
        during startup, when the calibration load failed, and for the whole graceful-shutdown
        drain window.
      operationId: getReady
      responses:
        '200':
          description: Service is ready to serve requests ({"status": "ready"})
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/HealthResponse'
        '503':
          description: |
            Not ready — startup, failed calibration load, or shutting down
            ({"status": "not_ready"}).
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/HealthResponse'
```

- [ ] **Step 2: Add a Service Lifecycle section** to `docs/api-documentation.md`, near the
      health/status endpoint docs

```markdown
## Service Lifecycle (roadmap S5)

### Startup

1. Configuration loads, logging initializes.
2. The calibration repository loads from `calibration.antenna_config_file`.
3. On success: `/status` is populated with the loaded antenna IDs and readiness flips to
   **true**.
4. On failure, behavior depends on `calibration.fail_fast`:
   - `true` (the shipped default): the process logs the error and **exits nonzero**. It does
     not start a useless server.
   - `false`: the server starts **degraded** — readiness stays false, `/health` reports
     `"degraded"`, `/status` lists no antennas, and gain requests return 404.

Readiness is false from process start until step 3 completes, so a readiness probe never
routes traffic to an instance that cannot serve it.

### Shutdown

On `SIGTERM` or `SIGINT`:

1. Readiness flips to **false** immediately — `/ready` returns 503.
2. The service keeps serving for `server.shutdown_readiness_delay_secs`, giving load
   balancers a window to observe the flip and stop sending new requests.
3. New connections stop being accepted; in-flight requests drain for at most
   `server.shutdown_timeout_secs`.
4. Cleanup runs (final status log, resource release).

`/health` stays 200 throughout the drain — the instance is alive, just no longer accepting
new work.

### Configuration

| Knob | Default | Notes |
|---|---|---|
| `calibration.fail_fast` | `true` | Abort startup if the calibration load fails. |
| `server.shutdown_readiness_delay_secs` | `0` | Grace window after the readiness flip. Recommended `5` in Kubernetes. |
| `server.shutdown_timeout_secs` | `25` | Bounded drain. Keep `delay + timeout` under the pod's `terminationGracePeriodSeconds` (30 in the shipped chart) or the drain is SIGKILLed and cleanup is skipped. |
```

- [ ] **Step 3: Validate the spec parses**

Run: `python3 -c "import yaml; yaml.safe_load(open('openapi.yaml')); print('ok')"`
Expected: `ok`

- [ ] **Step 4: Commit**

```bash
git add openapi.yaml docs/api-documentation.md
git commit -m "docs(S5): document the startup/shutdown lifecycle; fix /ready 503 schema drift"
```

---

## Task 6 — Full gate, roadmap closeout, PR

**Goal:** The workspace gate is green, S5 and Phase 2 are marked done in the roadmap, and the PR
is open.

**Files:**
- Modify: `docs/roadmap-2026-07-work-units.md` (S5 entry, Phase 2 header)
- Modify: `docs/roadmap-2026-07.md` if it carries a phase-status line
- Add: `docs/plan-s5-graceful-shutdown.md` (this file, committed alongside — matches the S1–S4
  convention)

**Acceptance Criteria:**
- [ ] `./scripts/check.sh` exits 0 (fmt, `clippy --workspace --all-targets -D warnings`,
      `cargo test --workspace`).
- [ ] `concurrent_tests.rs`, `resilience_tests.rs`, `error_tests.rs` all green — these are the
      suites most exposed to the readiness and `/health` changes.
- [ ] Roadmap marks S5 done with the commit SHA, matching the format used for S1–S4/S6.
- [ ] Roadmap notes Phase 2 complete (S1, S2, S3, S4, S5, S6 done; S7 superseded by C8).
- [ ] PR opened against `main`.

**Verify:** `./scripts/check.sh` → `All gate checks passed.`

**Steps:**

- [ ] **Step 1: Run the full gate**

```bash
./scripts/check.sh
```

Expected: ends with `All gate checks passed.` The only `cargo audit` finding should be the
pre-existing, allowed `paste` unmaintained advisory (RUSTSEC-2024-0436).

- [ ] **Step 2: Mark S5 done in `docs/roadmap-2026-07-work-units.md`**

Match the format the merged units use — read the S4 entry (search for `S4 — Admission control`)
and mirror its "DONE" annotation style exactly, including the SHA and date. Add a note for the
two decisions a future reader will want:
- `/health` stays 200 when degraded (liveness must not restart-loop on a data problem).
- Degraded is derived from `repository.antenna_count() == 0`, not a separate flag.

- [ ] **Step 2b: Record the follow-ups this execution surfaced**

None are S5 scope; all were found while implementing it and are worth a roadmap entry so they
aren't lost. Add them to `docs/roadmap-2026-07-work-units.md` under whatever "discovered debt"
convention that file already uses (read it — do not invent a new section if one exists):

1. **`test_startup_with_corrupted_calibration_binary` tests nothing.** Its fixture
   (`tests/integration/error_tests.rs:120-128`) writes `antenna_id:`, a top-level `feeds:` list,
   and `feed_id:` — none of which match `AntennaConfigEntry`'s serde shape (`id`, `name`,
   `calibration_file`, `enabled`; `feeds` exists only nested under `design_specs`, keyed by
   `id`). The fixture fails to deserialize, and the test ends in `let _ = result;`, so it has
   never asserted anything. Pre-existing.
2. **`tests/fixtures/test_antennas.yaml` paths are crate-root-relative**, not
   `data_directory`-relative, so the intuitive `data_directory = tests/fixtures` doubles the
   prefix and two antennas silently fail to load. Most fixture-based tests paper over this with
   `fail_fast: false`. Now that S5 makes `fail_fast` real, that workaround silently weakens any
   test asserting a healthy load. Either fix the fixture's paths or document the convention.
3. **The `fail_fast` fatal error does not name the failing antenna IDs.** The per-antenna
   `warn!` (`repository.rs:105`) carries them, but the fatal line an operator sees last does
   not. S5 added `current_dir` to that message; adding the IDs would finish the job.
4. **Startup concerns are accumulating in `api/mod.rs`** — `apply_worker_threads` (S4) and now
   `initialize_repository` + `begin_shutdown` (S5). A third justifies extracting `api/startup.rs`.

- [ ] **Step 3: Mark Phase 2 complete**

Update the Phase 2 header in `docs/roadmap-2026-07-work-units.md` (line ~786) and the phase
tracker in `docs/roadmap-2026-07.md` to record that all of S1–S6 are merged and S7 is superseded
by C8, so Phase 2 is closed.

- [ ] **Step 4: Commit and push**

```bash
git add docs/
git commit -m "docs(S5): mark S5 done and close out Phase 2"
git push -u origin feat/s5-graceful-shutdown
```

- [ ] **Step 5: Open the PR**

Write the body to a scratch file first (it is long, and `--body` mangles multi-line markdown in
zsh), then create the PR:

```bash
BODY=$(mktemp -t s5-pr-body)
cat > "$BODY" <<'EOF'
Implements roadmap Phase 2 unit **S5**, per the committed plan in
`docs/plan-s5-graceful-shutdown.md`. This closes Phase 2 (S1–S6 merged; S7 superseded by C8).

## Four lifecycle lies, fixed

1. **Readiness started true.** `AppState::new` hard-coded `AtomicBool::new(true)`, so `/ready`
   returned 200 before calibration data had loaded — the exact window a readiness probe exists
   to cover. Readiness now starts false and flips true only after a successful load.
2. **`calibration.fail_fast` was honored one layer down and thrown away one layer up.** The
   repository respected it; `start_server_with_config` then caught the `Err` unconditionally and
   booted with an empty repository anyway. It now aborts startup (nonzero exit) when set.
3. **`shutdown_cleanup()` had no caller.** It is now invoked after the drain, on both the clean
   and the errored path.
4. **The drain was unbounded** (`None`), so one slow request could outlast the pod's 30 s grace
   period and take a SIGKILL mid-flight. Now bounded by `server.shutdown_timeout_secs`.

## Behavior change for operators — read this

`calibration.fail_fast: true` is the **shipped default**. A deployment with a broken
`antennas.yaml` that previously booted into a silently degraded state will now **refuse to
start and exit nonzero**. That is the requested behavior (exit criterion 2), not an accident.
Set `fail_fast: false` to keep the old degraded-start behavior — now with honest signals:
readiness stays false, `/health` reports `"degraded"`, `/status` lists no antennas.

## Why `/health` still returns 200 when degraded

`/health` is the Kubernetes **liveness** probe. A failing liveness probe restarts the pod, and a
restart cannot fix missing calibration data — returning non-200 would produce an endless
CrashLoopBackOff. The degraded signal goes in the existing `status` string field (response shape
unchanged, per the work unit); `/ready` is what keeps traffic away.

## Shutdown sequence

`SIGTERM`/`SIGINT` → readiness flips false → serve for `shutdown_readiness_delay_secs` (default
`0`, recommended `5` in Kubernetes) so load balancers observe the flip → stop accepting, drain
in-flight for at most `shutdown_timeout_secs` (default `25`) → cleanup. Defaults are chosen so
`delay + timeout` stays inside the chart's `terminationGracePeriodSeconds: 30`, leaving room for
cleanup before SIGKILL.

## Also in this PR

- The zero-calibrations error now distinguishes "no antennas enabled in configuration" from
  "all N enabled antennas failed to load" — same `Err` shape, different message.
- `/status` finally reports the loaded antenna IDs: production never called `set_antenna_ids`,
  so `antenna_count`/`antenna_ids` were always omitted from the response.
- `openapi.yaml` `/ready` 503 drift fixed — the spec claimed `ErrorResponse`, the handler
  returns `{"status": "not_ready"}`.

## Verification

`scripts/check.sh` green: `cargo fmt --check`, `cargo clippy --workspace --all-targets
-D warnings`, `cargo test --workspace`. `cargo audit`'s only finding is the pre-existing allowed
`paste` unmaintained advisory. No physics/model files touched.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF

gh pr create --base main --head feat/s5-graceful-shutdown \
  --title "feat(S5): graceful shutdown, readiness lifecycle, honor calibration.fail_fast" \
  --body-file "$BODY"
```

---

## Exit criteria (definition of done — from the work unit)

1. Readiness starts false; flips true only after calibration load completes. → Tasks 1, 3
2. All-loads-failed + `fail_fast` → process exits nonzero at startup; without `fail_fast`, the
   server starts but readiness/health reflect the degraded state (response *shapes* kept). →
   Task 3
3. On shutdown signal: readiness flips false, `shutdown_cleanup()` is actually invoked, in-flight
   requests drain. → Task 4
4. Tests for the `fail_fast` path and the readiness flip. → Tasks 1, 3, 4
5. Docs + spec updated. → Task 5
6. Full workspace gate green; Phase 2 closed. → Task 6

---

## Watch-outs

- **Never return non-200 from `/health`.** It is the liveness probe. A degraded service must
  stay "alive" or Kubernetes restart-loops it forever over a problem a restart cannot fix. The
  degraded signal goes in the `status` *field*; the traffic signal goes on `/ready`.
- **Do not change the `/health`, `/ready`, or `/status` response shapes.** They are in
  `openapi.yaml` and the work unit calls this out explicitly. Only the `status` string value
  changes.
- **`fail_fast: true` is the shipped default**, so Task 3 is a real behavior change for anyone
  running with a broken `antennas.yaml`: they used to get a degraded server, they now get a
  nonzero exit. That is the requested behavior — call it out prominently in the PR body, do not
  soften it by changing the default.
- **The readiness delay must be inside the shutdown future.** poem stops accepting connections
  the instant that future resolves; sleeping after `run_with_graceful_shutdown` returns would
  make the delay useless.
- **Do not call `std::process::exit` from `api/`.** Return the `io::Error`; `main.rs` already
  owns the exit path.
- **`load_from_config`'s `Ok`/`Err` shape is pinned by tests.** Task 2 changes the error
  *message* only. Do not "improve" it into a new return type — `error_tests.rs` and the
  repository tests depend on the current shape.
- **`AppState::with_defaults()` has an empty repository**, so under D4 it now reports
  `"degraded"`. Expect exactly one existing assertion (`routes.rs::test_health_route`) to need
  updating; if more turn up, they are legitimately the same mechanical fix — update and keep
  going.
- **openapi is hand-maintained** until C7 — mirror the changes manually now or it drifts.
