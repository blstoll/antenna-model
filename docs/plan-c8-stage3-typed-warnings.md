# Implementation Plan — C8 Stage 3: typed warnings

**Unit:** C8 stage 3 of 4 (`docs/roadmap-2026-07-work-units.md`, register row **C8** in
`docs/roadmap-2026-07.md`; also discharges the per-item batch-shape item deferred from
row **C2**)
**Effort:** L (one session)
**Decision status:** **Decided 2026-07-08** — pre-production confirmed, no consumers
exist; break once now, freeze behind C7. Two sub-decisions taken 2026-07-27 (below).
**Status:** ✅ **DONE 2026-07-27**, branch `feat/c8-stage3-typed-warnings`.

**Goal:** Replace `warnings: Vec<String>` with `Vec<ApiWarning> { code, message }` on all
three response types, so a client can branch on a stable machine-readable code instead of
pattern-matching prose — and so the set of things the service can warn about is
enumerable, documented, and drift-guarded.

---

## 0. Why stage 3 was next

Stage 1 (aim-point renames) landed 2026-07-26 at `95a2c2e` (#19); stage 2 (required
`coordinate_system`) landed 2026-07-27 at `5c99ef2` (#20). C8's stages are explicitly
sequential, one PR each, and stage 3's dependencies (C3 → C4 → C2 for the error contract,
G3/C11 for the example guards, S6 for validation constraints) were all in place.

## 1. What the unit required

> **Stage 3 — Typed warnings.**
> - `warnings: Vec<String>` → `Vec<ApiWarning> { code, message }` on all response types.
>   Enumerate the code set from existing producers (grep `warnings.push` / warning
>   constructors): expect at least `extrapolated`, `out_of_coverage`,
>   `ray_trace_degraded`, `non_convergence`, plus the codes added by roadmap units P1
>   (`spillover_applied`), P2 (`higher_order_heuristic`), and P8 (`off_axis_unvalidated`).
> - Exit: every producer emits a code + human message; the code enum documented in
>   api-documentation.md + openapi; integration tests assert codes, not string matches.

Plus the item C2 explicitly deferred here: the per-item batch failure **shape**
(`gain_db: null` + a `"Computation failed: …"` warning string) becomes an explicit
`{code, message}`.

Plus C8's standing constraint: **this pass must not alter any computed value.**

## 2. The defect being removed

Three problems, all consequences of prose being the only channel:

1. **Clients had to pattern-match prose** — and so did the service. `service::heatmap`
   counted extrapolated grid points with
   `w.contains("extrapolat") || w.contains("out of range")`: a substring test against
   messages owned by two *other* modules. The second phrase matched nothing any producer
   still emitted.
2. **Nothing enumerated the set.** A new `warnings.push(format!(…))` anywhere in `model/`
   or `service/` added a class no test, no doc, and no reviewer could notice.
3. **Rewording was a breaking change**, so messages were frozen by accident rather than by
   decision — awkward for exactly the messages that most needed revision (the P8/P10/F7
   honesty warnings were rewritten three times).

The integration tests had already grown the workaround: `off_axis_warning_tests.rs` and
`ray_trace_stub_warning_tests.rs` each carried a hand-picked "stable substring marker"
const, one of them documented as chosen to avoid colliding with a *different* warning
whose message also said "ray tracing".

## 3. Decisions taken (maintainer, 2026-07-27)

**(a) Batch per-item failure shape → a new typed `error` field.** `GainResponse` gains
`error: Option<GainError>`, present only on a failed `/gain/batch` item. Considered and
rejected: reporting the failure as a warning with code `computation_failed` (smaller diff,
but a failure stays indistinguishable from a quality caveat — the hazard C2 named), and
deferring to stage 4 (leaves a second response-shape break for the stage that should be
finishing, not starting, breaks).

*Refinement made during execution:* the code is **not** a new `computation_failed`
constant. It is drawn from the existing `error_codes` vocabulary via
`api::error_response::service_status` — the same mapping the HTTP error bodies use — so
the failure **class** survives into the item. An item that blows the integration budget
reports `computation_budget_exceeded`, not a flattened generic failure.

**(b) Code granularity → one code per cause (14 codes).** Considered and rejected: ~8
coarser codes grouping the feed-offset family and the extrapolation family. Rejected
because the grouping would have merged precisely the distinction the ray-trace test was
already hand-rolling — `severe_feed_offset` ("the geometry is extreme") versus
`ray_trace_degraded` ("the stub computed your number").

The unit's suggested names were followed where they still applied. Two did not:
`spillover_applied` (P1 reports spillover as `metadata.spillover_loss_db`, a number, not a
warning — the only spillover *warning* is the >10% advisory, so it is
`spillover_significant`) and `higher_order_heuristic` (P2 **removed** the mode that would
have emitted it, 2026-07-16 — no producer, no code).

## 4. What landed

**New module `warnings.rs`,** a peer of `error.rs` rather than a member of `api::`,
because the model layer produces warnings and does not otherwise depend on the API layer.
Re-exported from `api::schemas` so the wire types a client cares about all resolve under
one path. It holds:

- `WarningCode` — a **closed** enum, snake_case on the wire, with `ALL` and `as_str()`
  (hand-written match, so adding a variant fails to compile in two places, not one).
- `ApiWarning { code, message }` — `Hash`/`Eq`/`Ord` derived, so the aggregating endpoints
  can dedupe into a `HashSet` and sort without a bespoke comparator.
- `Display` as `[code] message`, so the ~10 existing `{}` log sites kept working unchanged.

**The 14 codes**, with their producers:

| Code | Producer |
|---|---|
| `extrapolated` | `model::correction_interpolator` |
| `out_of_coverage` | `service::evaluator` |
| `correction_not_applied` | `service::evaluator` |
| `uncalibrated` | `service::evaluator` |
| `partially_calibrated` | `service::evaluator` |
| `off_axis_unvalidated` | `service::evaluator` (P8, P11) |
| `rear_hemisphere_invalid` | `service::evaluator` (P10-tail) |
| `non_convergence` | `model::pattern` + `/h3-heatmap` cache re-emission (C10) |
| `ray_trace_degraded` | `model::pattern` + `service::evaluator` re-emission (P3) |
| `severe_feed_offset` | `model::edge_cases` |
| `feed_offset_spillover_unmodeled` | `model::edge_cases` |
| `spillover_significant` | `model::edge_cases` |
| `points_extrapolated` | `service::heatmap` |
| `point_computation_failed` | `service::heatmap`, `service::h3_link_budget` |

`point_computation_failed` is deliberately one code for both endpoints: the cause is
identical and only the word for a grid element differs. It also gave `/heatmap`'s two
per-point failure strings a code they never had.

**Two constructor functions** in `model::pattern` (`nonconvergence_warning()`,
`ray_trace_stub_warning()`) are now the single build points for the two warnings that are
emitted from more than one place. This is load-bearing beyond tidiness: aggregation dedupes
on `(code, message)`, so a re-emission whose text drifted would produce **two** array
entries for one cause. The message constants stay `pub` and unchanged.

**`service::heatmap`'s substring predicate** became
`w.is(Extrapolated) || w.is(OutOfCoverage)` — the change with the most behavioural weight
in the unit, since the old form silently depended on prose in two other modules.

**Tests.** The two integration-test substring markers became code constants. Unit tests
split cleanly in two: classification assertions now check codes, while the assertions that
pin honest *wording* (the P8/F7 off-axis message must still say "IDEALISED", must not
regress to the stale "intentionally off") were kept and moved to `.message`. Those tests
are the reason messages are still worth pinning somewhere — just not in every endpoint
test. Two "no calibration warnings present" tests improved: they used to *exclude* two
convergence phrases by substring (and so passed for any unrelated new class) and now
*select* the calibration class by code.

New `tests/warning_code_vocabulary.rs`, modelled directly on the existing
`error_code_vocabulary.rs`: every code appears in `openapi.yaml`'s `ApiWarning.code` enum
and in the `docs/api-documentation.md` table, the enum contains nothing the service cannot
emit, and no published file still shows a bare-string warning.

**Docs and spec.** `openapi.yaml` gained `ApiWarning` and `GainError` component schemas;
the three response `warnings` blocks each dropped a long duplicated prose description in
favour of a `$ref`, with the per-code detail now living once in the component.
`docs/api-documentation.md` gained a **Warning codes** section (table + the two long-form
notes for `off_axis_unvalidated` and `rear_hemisphere_invalid` + the aggregation rule).

**No computed value moved.** All 910 workspace tests pass with their numeric assertions
unchanged, which is the property that makes this pass reviewable.

## 5. Finding, fixed in-unit

Converting the published examples surfaced a warning **no producer has ever emitted**:

```json
"warnings": ["Beam squint correction applied (pointing_freq != operating_freq)"]
```

It appeared in `examples/api_requests.json` and `docs/architecture.md`. Beam squint *is*
reported — as the structured `geometry.beam_squint_deg` field, which both examples already
carried alongside the invented warning. Removed rather than given a code: minting a
vocabulary entry for something nothing emits would have put a permanent lie in the frozen
contract.

Both files are in unit **C15**'s inventory of client-visible surfaces that no drift guard
covers, and neither is under the C11 deserialization guard — which is exactly why this
survived. The new `no_bare_string_warnings_remain_in_the_published_contract` test now
covers both files for this specific class of drift, but C15's broader gap stands.

## 6. Out of scope, as scoped

- Routing-level 404/405/415 remain framework-shaped `text/plain` (C4's standing caveat).
- Any physics or semantics change. This pass reshapes and renames only.
- The stage 4 items: the `/heatmap` H3 grid-type stub removal, spec completeness, and the
  C12/C14 findings filed by stage 1.

## 7. What remains before the contract freezes

Stage 4 (endpoint coherence + spec completeness), then C7's drift guard. C12
(`rmse_db`/`r_squared` null-vs-omitted, needs a register decision), C13
(`design_feed_offset_m` origin, must land with D9), C14 (openapi feed-listing drift,
stage 4's job) and C15 (unguarded surfaces) are all still open.
