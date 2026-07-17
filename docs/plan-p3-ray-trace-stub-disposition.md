# Implementation Plan — P3: Ray-trace stub (feed offsets > 0.5·f) disposition

**Unit:** P3 (roadmap `roadmap-2026-07.md` §5 register + `roadmap-2026-07-work-units.md:548`)
**Effort:** S (≤ half a session)
**Decision status:** **Decided 2026-07-16** — *document + flag* (recommended default, adopted
as-is). Real ray tracing stays gated as feature F2; rejection was ruled out (warn-don't-refuse
philosophy + heatmap grid totality). **No decision step remains; this is execution only.**

All file:line references below were re-verified against the working tree on 2026-07-16. Re-verify
before editing (standing rule: if a cited line no longer matches its description, stop and
re-locate).

---

## 1. What P3 actually requires

From the work-unit exit criteria:

> Register row Decided; one test per endpoint proving the warning appears for a > 0.5·f request
> (`examples/requests/geo_large_feed_offset.json` is a ready fixture); docs updated.
> **Do not modify `ray_trace.rs` math.**

So the deliverable is: **(a)** per-endpoint regression tests that pin the degraded-accuracy
warning, **(b)** documentation of the limitation in `docs/domain-contract.md` and `openapi.yaml`,
and **(c)** resolution of one robustness gap found during scoping (h3-heatmap cache-hit warning
drop — see §3).

This is **not** a physics change: `PHYSICS_MODEL_VERSION` stays `4`, no aperture/phase/steering
math is touched, and no served *gain value* changes.

---

## 2. Current state (verified 2026-07-16)

### 2.1 The `>0.5f` regime and its warnings

- Mode selection: `select_computation_mode` (`edge_cases.rs:154`) returns
  `ComputationMode::RayTracing` when `offset_ratio > SEVERE_OFFSET_THRESHOLD` (`= 0.5`,
  `edge_cases.rs:19`).
- `analyze_edge_cases` pushes warning #1 (`edge_cases.rs:109-114`):
  `"Feed offset (X.XXf = X.XXX m) exceeds severe threshold (0.5f). Ray tracing recommended."`
- The gain dispatch pushes warning #2 (`pattern.rs:327-331`):
  `"WARNING: Ray tracing for large feed offsets (>0.5f) is not fully implemented; gain accuracy
  may be degraded."`
- Both land in `GainComputationResult.warnings` (`pattern.rs:59`) and flow to the service layer.
  Warning #2 (the honest "stub / not fully implemented" flag) is the load-bearing one for P3 and
  the stable substring the tests should assert on: `"not fully implemented"`.

### 2.2 The request path can reach the mode

A REST request cannot change an antenna's *design* feed offset, but the request's `feed_position`
(a pointing target, per the domain contract) is converted to a physical feed displacement in
`compute_feed_position_from_pointing` (`coordinates_3d.rs:613`) and combined with the design offset
(`evaluator.rs:141-157`). A feed aimed far from the reflector boresight therefore produces
`offset_ratio > 0.5`. Confirmed live by two existing tests that already route this geometry through
ray tracing:
- `tests/feed_steering_test.rs:140` (`test_feed_steering_large_offset`, from
  `geo_large_feed_offset.json`).
- `tests/integration/api_tests.rs:127` (`/api/v1/gain`, notes "large offset ratio routes through
  ray tracing").

Neither asserts on the ray-tracing **warning** — that assertion is the P3 gap.

### 2.3 Warning propagation per endpoint (four compute endpoints)

Routes: `/api/v1/gain`, `/api/v1/gain/batch`, `/api/v1/heatmap`, `/api/v1/h3-heatmap`
(`api/routes.rs:49-54`).

| Endpoint | Path | Ray-trace warning today (fresh cache) |
|---|---|---|
| gain | `evaluator.rs:270` extends response with `result.warnings` | ✅ surfaces |
| batch | per-item `compute_gain_from_request` → per-item `warnings` (`batch.rs`) | ✅ surfaces per item |
| heatmap | `heatmap.rs:300-305` per grid point → `heatmap.rs:115-120` flat-map + dedup | ✅ surfaces (deduped) |
| h3-heatmap | `h3_link_budget.rs:68-76` captures `result.warnings` **inside the cache-miss closure** → aggregated at `:496` | ⚠️ surfaces on a **cold** cache only — see §3 |

Because `feed_position` is constant across a heatmap/h3 grid, the mode is identical for every
point, so the warning is emitted at every point and collapses to one line via the existing
`HashSet` dedup (`heatmap.rs:115`, `h3_link_budget.rs` warnings-set). Good — no per-point spam.

### 2.4 Docs today

- `openapi.yaml:626-634` — `GainResponse.warnings` **already** lists "degraded-accuracy large feed
  offsets (ray-trace stub)". ✅ (single-gain + batch reuse this schema.)
- `openapi.yaml:874-882` (h3) and `:1017-1023` (heatmap) warnings descriptions list off-axis and
  rear-hemisphere warnings but **omit** the ray-trace-stub large-offset warning. ❌ add it.
- `docs/domain-contract.md` mentions the `>0.5·f` ray-tracing band only in the efficiency-gate
  context (`:117-123`); there is no single statement that (i) `>0.5f` routes to an acknowledged
  stub, (ii) results carry a degraded-accuracy warning, (iii) that warning is emitted on all four
  compute endpoints. ❌ add it.

---

## 3. The one judgment call — h3-heatmap cache-hit warning drop

**Finding.** The P8 off-axis warning (`h3_link_budget.rs:109`) and the P10-tail rear-hemisphere
warning (`:118`) are appended to `captured_warnings` **outside** the `cache.get_or_compute` closure,
specifically so they re-surface on cache hits (comment at `:143`, `:252`). The ray-tracing warning
is **only** captured inside the miss closure (`:68-76`). The `GainCache` is shared and persistent
(passed into `compute_h3_link_budget`, `:286`), so on a **warm** cache an h3-heatmap request drops
the ray-tracing stub warning even though the served number is still the degraded stub value.

This does not break the P3 exit tests (a fresh `TestServer` has a cold cache, so every distinct
cell is a first-time miss and the warning surfaces). But it means P3's core promise — *"verify the
unreliable warning reaches all four compute endpoints"* — is not **robustly** true for h3-heatmap.

**Recommended resolution (default): fix it, mirroring the P8/P10-tail precedent.** Hoist the
ray-tracing degraded-accuracy warning to the service layer so it is cache-independent:
- Add a small service-layer predicate that recomputes whether the feed offset exceeds the
  ray-trace threshold from `feed_offset` magnitude and `focal_length` (reuse the already-`pub`
  `edge_cases::SEVERE_OFFSET_THRESHOLD`); emit the stub warning from `compute_cell_gain` **outside**
  the cache closure, exactly as `off_axis_unvalidated_warning` and `rear_hemisphere_warning` are
  emitted (`h3_link_budget.rs:109,118`).
- The h3 warnings-set already dedups, so this is harmless on cache misses (the model still pushes
  the same string inside the closure; the set collapses the duplicate). No `ray_trace.rs` math is
  touched; this is warning plumbing only, and it follows two established precedents in the same file.

**Fallback (if the maintainer prefers minimal scope):** leave the cache path as-is, write the
per-endpoint tests against a cold cache, and file the cache-hit drop as a one-line follow-up
finding. Acceptable because it is pre-production and the value itself is unaffected — but it leaves
h3-heatmap's honesty warning cache-dependent, which is the exact class of "test/production gap" the
roadmap keeps warning about.

> **Decision needed before Task 3:** fix now (recommended) vs. defer as a follow-up. Everything
> else in this plan is unconditional. If deferred, drop Task 3 and add a cold-cache note to the
> h3 test.

---

## 4. Tasks (ordered)

### T1 — Test fixture / request builder
- Add a helper producing a `> 0.5·f` request built from the `geo_large_feed_offset.json` geometry
  (mirror the existing literal in `feed_steering_test.rs:145-160`). Reuse it for gain, batch,
  heatmap, and h3 request shapes (all three request types carry `feed_position`:
  `schemas.rs:247,440,598`).
- Add a `has_ray_trace_stub_warning(warnings: &[String]) -> bool` helper asserting the substring
  `"not fully implemented"` (stable across both warning strings; specific to the stub).

### T2 — Per-endpoint regression tests (the core exit criterion)
Create `tests/integration/ray_trace_stub_warning_tests.rs` (mirror the structure of
`off_axis_warning_tests.rs`, which already exercises all four endpoints at `:52/:98/:124/:162`):
1. `/api/v1/gain` — large-offset request ⇒ warning present.
2. `/api/v1/gain/batch` — one large-offset item ⇒ that item's `warnings` contain it.
3. `/api/v1/heatmap` — large-offset `feed_position` ⇒ aggregated `warnings` contain it (exactly
   once, dedup).
4. `/api/v1/h3-heatmap` — large-offset `feed_position` ⇒ aggregated `warnings` contain it.
5. A negative control: a small-offset (boresight-aimed) request on the same antenna ⇒ warning
   **absent** (guards against always-on emission).
- Register the new file in `tests/integration/mod.rs`.

### T3 — (recommended, gated on §3 decision) h3-heatmap cache-independent emission
- Implement the service-layer hoist described in §3. Add a test asserting the warning survives a
  **second** (warm-cache) h3-heatmap request for the same geometry.

### T4 — `docs/domain-contract.md`
- Add a short, explicit note (natural home: end of "Off-axis pattern / sidelobe fidelity" or a new
  "Large feed offsets (> 0.5·f)" bullet near the efficiency gate at `:117-123`): offsets `> 0.5·f`
  route to the acknowledged ray-tracing stub (`ray_trace.rs`, not fully implemented); results carry
  a degraded-accuracy warning on **all four** compute endpoints; real ray tracing is gated as
  feature F2. Cross-reference P3.
- **Sequencing:** P1 also edits this file but is already landed, so no live conflict — still, make
  this a single focused edit.

### T5 — `openapi.yaml`
- Add the ray-trace-stub degraded-accuracy warning to the **heatmap** (`:1017-1023`) and
  **h3-heatmap** (`:874-882`) `warnings` descriptions so all four endpoints document it
  (GainResponse already has it at `:632`). Standing rule 4 (hand-maintained openapi).

### T6 — `docs/api-documentation.md`
- Add the large-offset stub caveat to the accuracy-caveats section (mirror how P8's off-axis
  warning was documented). Keeps docs = code = behavior.

### T7 — Verify
- `cargo test --workspace` (new tests green; no existing test perturbed — this unit changes no
  served value).
- `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`
  (or `scripts/check.sh`).
- Sanity-drive one endpoint against `examples/requests/geo_large_feed_offset.json` to eyeball the
  warning in a real response body.

---

## 5. Exit-criteria mapping

| Work-unit exit criterion | Satisfied by |
|---|---|
| Register row Decided | Already Decided 2026-07-16 (no action) |
| One test per endpoint proving the warning appears for a > 0.5·f request | T1 + T2 |
| `geo_large_feed_offset.json` used as the fixture | T1 |
| Docs updated (domain-contract + openapi descriptions) | T4 + T5 (+ T6) |
| `ray_trace.rs` math untouched | Enforced — no edit to `ray_trace.rs`; T3 is warning plumbing only |

---

## 6. Out of scope / do-not-touch

- **`ray_trace.rs` math** — explicitly forbidden by the unit. Real ray tracing is feature F2.
- **Physics version** — no `gain_physics` change, so `PHYSICS_MODEL_VERSION` stays `4`
  (`model/mod.rs:66`).
- **Feed-steering / beam-deviation signs** — standing rule 2, never touched.
- **The `0.3f–0.5f` moderate-offset band** — post-P2 that routes through `StandardPhysicalOptics`
  with its own separate "spillover not modeled" warning (`edge_cases.rs:114`); P3 is the `>0.5f`
  band only. Do not conflate the two warnings.
- **Rejecting large-offset requests** — ruled out by the decision (warn-don't-refuse; heatmap grids
  must remain total).

---

## 7. Risk / gotchas

- **Two warning strings for `>0.5f`** (edge-case + dispatch). Assert on the specific
  `"not fully implemented"` substring, not a generic "ray" match, to avoid coupling to the
  edge-case wording.
- **Dedup is load-bearing** for heatmap/h3 — assert the warning appears (≥1), not a specific count,
  except where you deliberately assert "exactly once" to pin the dedup behavior.
- **h3 cache lifetime** (§3) is the only subtle correctness point; get the fix-vs-defer call before
  T3.
- **openapi drift** — until C7's guard lands, the three warnings descriptions
  (`:632`, `:877`, `:1020`) are maintained by hand; keep them consistent.
