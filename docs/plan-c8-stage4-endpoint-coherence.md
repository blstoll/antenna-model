# Implementation Plan — C8 Stage 4: endpoint coherence + spec completeness

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended)
> or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax
> for tracking.

**Unit:** C8 stage 4 of 4 (`docs/roadmap-2026-07-work-units.md`, register row **C8** in
`docs/roadmap-2026-07.md`), absorbing units **C12** (null-vs-omitted `rmse_db`/`r_squared`),
**C14** (openapi feed-listing drift) and the superseded row **C5** (`/heatmap` H3 stub).
**Effort:** L — larger than the unit text implies; see §2.
**Branch:** `feat/c8-stage4-endpoint-coherence`
**Decision status:** **Decided 2026-07-08** (register row C8) — pre-production, no consumers;
break once now, freeze behind C7. Six sub-decisions taken in this plan, §3.
**Status:** planned 2026-07-28.

**Goal:** Make the published contract describe what the service actually serves — remove the
`/heatmap` H3 grid-type stub and its dead error code, stop emitting `null` for two declared
number fields, and reconcile every openapi component behind the four antenna routes with its
Rust type — so that C7's drift guard freezes a *correct* contract rather than an incorrect one.

**Architecture:** Three independent strands landing as one PR. (a) *Removal*: the H3 grid-type
variant disappears from `GridConfig`/`GridData`, taking the validator arm, the two service
arms, and the now-producerless `not_implemented` error code with it. (b) *Shape*: the
`f64::NAN` sentinel the design-spec loader writes for `rmse_db`/`r_squared` is mapped to `None`
at the API boundary onto the codebase's existing `nan_as_null` convention, so the `null` the
endpoint already emits becomes a declared contract instead of an accident of `f64::NAN`
serialization. (c) *Spec truth*: all seven openapi component schemas reachable from
`/api/v1/antennas*` are rewritten
against their Rust types. Nothing in the physics or service layers changes; C8's standing
constraint — **no computed value moves** — holds throughout.

**Tech Stack:** Rust 2021, poem 3 (HTTP), serde/serde_json, `serde_yaml` (not yet used — C7),
`cargo test --workspace`, `./scripts/check.sh` (fmt + clippy `-D warnings` + tests + audit).

**User decisions (already made):**
- *"Yes plan out C8 and include C12 and C14."* — stage 4 is the deliverable; C12 and C14 ride
  along.
- *"I want to reverse my prior decision on C12, let's emit a null, it gives the API one
  convention ('no value → null') consistent with gain_db's established null-for-NaN
  behavior."* — **C12 option 2 adopted**, reversing an earlier lean toward option 1. The
  `null` becomes *deliberate and declared* rather than accidental; see §3.1.

---

## 1. Why stage 4 is next

Stages 1–3 landed on consecutive days: stage 1 (aim-point renames) `95a2c2e` (#19) on
2026-07-26; stage 2 (required `coordinate_system`) `5c99ef2` (#20) and stage 3 (typed warnings)
`d0a675b` (#21) + `b5a6b51` (#22) on 2026-07-27. C8's stages are explicitly sequential, one PR
each. Everything else in Phase 3 is landed (C1, C3, C4, C2, C9, C10, C11), and **C7 — the drift
guard that freezes the contract — depends on C8**. Stage 4 is the last content change before
the freeze.

## 2. What the unit required, and what is actually there

The work-unit text is three bullets:

> **Stage 4 — Endpoint coherence + spec completeness.**
> - Remove the `/heatmap` H3 grid-type stub (`heatmap.rs:168-171,215-218`); unknown grid types
>   become normal validation failures (absorbs old C5).
> - `/h3-heatmap` fully documented (absorbs C1 if it hasn't landed; if C1 landed, update it for
>   stages 1–3's changes).
> - Decide-and-document endpoint naming: keep two endpoints (`/heatmap` rectangular,
>   `/h3-heatmap` link budget) — a full merge remains feature F5.
> - Exit: openapi.yaml describes every registered route with post-C8 schemas; ready for C7.

**The exit criterion is far more expensive than the bullets suggest.** A component-by-component
audit of `openapi.yaml` against the Rust types on 2026-07-28 found that **every one of the
seven schemas behind the four `/api/v1/antennas*` routes is wrong in at least one field**. C14
described `FeedInfo` and the list-feeds wrapper; those are two of eight defects:

| openapi component | defects vs the Rust type in `api/schemas.rs` |
|---|---|
| `AntennaInfo` (`:1698`) | `antenna_id` → `id`; `feeds` → `feed_ids` |
| `AntennaDetailsResponse` (`:1714`) | `antenna_id` → `id`; **missing** `enabled`; `calibration_info` → `calibration`; spurious `calibration_status: string`; `calibration_status_info` → `calibration_status` |
| `FeedInfo` (`:1737`) | `feed_id` → `id`; `frequency_range` → `frequency_range_mhz`; spurious `name`; spurious `phase_center_offset_m` — **C14(a)** |
| list-feeds 200 wrapper (`:974-981`) | spurious `antenna_id` — **C14(b)** |
| `PhysicalParametersInfo` (`:1779`) | `f_over_d` → `f_over_d_ratio`; **missing** `focal_length_m`; spurious nested `feed: {q_factor, phase_center_offset_m}` |
| `ValidityRangesInfo` (`:1804`) | `azimuth`/`elevation`/`frequency`/`temperature` → `azimuth_deg`/`elevation_deg`/`frequency_mhz`/`temperature_k` (all four) |
| `CalibrationInfo` (`:1828`) | `calibration_date` → `date`; `format_version` → `version`; `data_source` → `source`; **missing** `r_squared`; spurious `parameters_tuned` |
| `GridData` (`:1669`) | declared as a flat object with `grid_type: string`; the Rust type is a tagged enum |

A client coding to this spec would fail to find a single field on `GET /api/v1/antennas`. This
is squarely the exit criterion's job and it is why Task 5 is the largest task in the plan.

**Why it survived.** `openapi.yaml` is hand-maintained and no guard covers it (roadmap risk 1;
unit **C15**'s inventory). The antenna routes also have thin test coverage on the *uncalibrated*
path, which is the only path actually served (all four `.bin` antennas are `enabled: false` —
unit D9).

## 3. Decisions taken in this plan

Each is a call a careful implementer would otherwise have to stop and make mid-task. All six
are consistent with decisions already in the register; none contradicts one.

### 3.1 C12 — declare the `null`, via the existing `nan_as_null` convention

**Adopted: option 2** (maintainer, 2026-07-28), reversing an earlier lean toward option 1. The
reason given: one API-wide convention, *"no value → null"*, consistent with `gain_db`'s
established null-for-NaN behavior.

**One factual caveat, recorded once and then set aside.** "One convention" does not describe the
codebase as it stands: `api/schemas.rs` carries **31** `skip_serializing_if` attributes (omit)
against **one** field using `nan_as_null` (`GainResponse.gain_db`). By count, omission is the
incumbent convention 30:1, and this decision aligns C12 with the exception rather than the rule.
The roadmap has also twice moved *away* from `null`-under-200 for numeric fields — **C2** flagged
it on `/gain/batch`, and **C9** invented the finite `NO_PEAK_GAIN_DB = -999_999.0` sentinel
specifically so `f64::NEG_INFINITY` would not become `null`.

**Why the decision nevertheless holds, and is arguably the better rule.** Those 31 omissions and
this `null` are not actually the same question, and conflating them is what made the field
ambiguous in the first place:

- **Omitted = structurally absent.** `PhysicalParametersInfo.mesh` is absent because a solid
  reflector *has no mesh*. `CoverageInfo` is absent because an uncalibrated antenna has no
  coverage region. There is no slot to fill.
- **`null` = the slot exists, the value does not.** Every antenna has a calibration block; the
  only question is whether a fit was performed. That is exactly `gain_db`'s situation — the item
  exists in the results array, the computation failed — and it is why `nan_as_null` was written.

Under that reading `rmse_db: null` is correct *and* `mesh` omitted is correct, and the two
conventions are complementary rather than competing. The C2/C9 hazard is also mitigated here by
the same mechanism stage 3 used for `gain_db`: an adjacent **typed** signal a client can branch
on without inspecting the number — `calibration_status.status == "uncalibrated"`, plus
`num_measurements: 0`. A `null` next to a typed explanation is not the silent-null hazard C2
described.

**What must change (the defect is still real, just fixed in the other direction).** Today the
code *claims* omission and *does* `null`: `CalibrationInfo.rmse_db` is
`Option<f64>` + `#[serde(skip_serializing_if = "Option::is_none")]` with a doc comment reading
*"None for uncalibrated antennas"*, while `handlers.rs:703` wraps `f64::NAN` in `Some(...)`
unconditionally so the attribute never fires. Option 2 makes the code claim `null` and do
`null`:

- `CalibrationInfo.rmse_db` / `r_squared` become plain `f64` with `#[serde(with = "nan_as_null")]`
  — the same declaration `gain_db` carries. This is the *actual* convention alignment; merely
  deleting the `skip_serializing_if` attributes and keeping `Option<f64>` would leave an `Option`
  that is always `Some`, i.e. a second misleading type.
- The doc comments change from *"None for uncalibrated"* to *"null for uncalibrated"*.
- `openapi.yaml` declares both `nullable: true`, and `docs/api-documentation.md` documents the
  `null`-under-200 shape **deliberately** — unit C12 requires this explicitly under option 2
  ("not by omission").
- `examples/responses/antenna_details_response.json` **keeps** its two `null`s unchanged. (Note
  the irony recorded in C12: the `examples/responses/` drift guard once tried to delete these as
  stale keys and was reverted in review. They were right all along.)

**`data/types.rs` is still not touched** — for a different and simpler reason than under option 1.
`CalibrationMetadata.rmse_db`/`r_squared` stay plain `f64` with a `f64::NAN` sentinel, which now
matches the API type exactly, so there is no boundary conversion at all. (Had option 1 been
chosen, leaving them `f64` would have been a compromise justified by postcard's positional wire
format; under option 2 it is simply correct. Either way, do not make them `Option<f64>` — that
is an ANTC format break belonging to **D2**.)

**Sentinel behavior to be aware of, unchanged by this decision.** `data/loader.rs:268,275` warns
when `rmse_db > 1.0` or `r_squared < 0.95`. Both comparisons are `false` for `NaN`, so
design-spec antennas load without a spurious quality warning. That is correct behavior resting on
NaN comparison semantics — do not "fix" it into a warning.

**Blast radius:** two sites. `CalibrationInfo` is constructed exactly once
(`handlers.rs:699-706`) and its two fields are read nowhere — grep-confirmed across the whole
workspace. No test asserts equality on `CalibrationInfo` or `AntennaDetailsResponse`, so the
`NaN != NaN` consequence of their derived `PartialEq` is theoretical.

### 3.2 A removed grid type is a **400**, not a 422

The unit says *"unknown grid types become normal validation failures"*. Under C2's core policy —
**400 = a body that cannot be parsed; 422 = parses but is semantically invalid** — an unknown
tag on a serde-tagged enum is genuinely unparseable: serde rejects it before any validator runs.
So `{"grid_type": "h3", …}` becomes a **400 `invalid_request_body`**.

This matches both prior stages' precedent — stage 1's
`legacy_feed_position_key_is_rejected_with_400` and stage 2's
`a_position_without_coordinate_system_is_rejected_with_400`. Record it explicitly, because the
unit text hints at 422 and a reader could reasonably implement a validator arm to force one.

### 3.3 `GridConfig` stays a tagged enum, single-variant

Do **not** collapse `GridConfig` into a plain struct once `H3` is gone. Keeping the
`#[serde(tag = "grid_type")]` enum preserves `grid_type: "rectangular"` in the wire contract —
dropping it would be a second, gratuitous breaking change, and it would close the door on
feature **F5** (merging `/h3-heatmap` into `/heatmap`), which would add a variant back.

### 3.4 Delete `not_implemented` from the error vocabulary; do not reserve it

The `/heatmap` H3 stub is the **only** producer of `AntennaModelError::NotImplemented`
(grep-confirmed: `heatmap.rs:206,287` are the sole construction sites). Removing the stub leaves
a code with no producer — which is precisely the defect **C3** removed when it deleted the seven
PascalCase `ErrorResponse` constructors that "only ever appeared in their own definitions and
unit tests". Delete the variant, the mapping, and the constant. F5 can re-add a code if it ever
needs one; adding to the vocabulary is non-breaking.

This is also a live test of C3's guard: `tests/error_code_vocabulary.rs` pins `openapi.yaml`'s
two error enums and the `docs/api-documentation.md` table against `error_codes::ALL`, so the
build will refuse to go green until all three published surfaces are updated. Expect it to fail
and let it drive the work.

### 3.5 C14(b) — fix the spec, not the handler

`openapi.yaml:974-981` declares the list-feeds body as `{antenna_id, feeds}`; the handler
returns `{feeds}` only (`handlers.rs:819`). **Drop `antenna_id` from the spec.** The client
already has the antenna id — it is in the request path — and adding a field to a response is
*non-breaking*, so this one does not need to consume the single sanctioned break. If a consumer
ever wants a self-describing body, it can be added after the freeze.

### 3.6 Endpoint naming — two endpoints stay

`/api/v1/heatmap` (rectangular az/el grid, loss surface) and `/api/v1/h3-heatmap` (per-cell link
budget over an Earth-surface H3 grid) remain separate. They differ in more than grid shape — the
H3 endpoint computes FSPL, G/T and per-cell link budget, and takes an Earth-referenced centre.
A full merge stays feature **F5** (register row C5 already says so). Record the rationale in
`docs/api-documentation.md` so the next reader does not re-litigate it.

## 4. File structure

Files this plan creates or modifies, and why each is in scope:

**Rust — source**
- `antenna-model/src/api/schemas.rs` — remove `GridConfig::H3` + `GridData::H3` variants and
  their two unit tests; add a rejection test; remove `error_codes::NOT_IMPLEMENTED` from the
  module and from `ALL`.
- `antenna-model/src/service/validator.rs` — remove the `GridConfig::H3` match arm
  (`:549-598`) and its four unit tests (`:882-932`).
- `antenna-model/src/service/heatmap.rs` — remove the two `GridConfig::H3` arms (`:204-208`,
  `:287-290`) and `test_h3_grid_not_implemented` (`:700-703`).
- `antenna-model/src/error.rs` — remove the `NotImplemented` variant (`:62`) and its `Display`
  arm (`:569`).
- `antenna-model/src/api/error_response.rs` — remove the `NotImplemented` status mapping
  (`:134`) and its unit test (`:295`).
- `antenna-model/src/api/handlers.rs` — C12: drop the `Some(...)` wrapping at the sole
  `CalibrationInfo` construction (`:699-706`), which is no longer needed once the API type and
  the metadata type have the same shape.
- `antenna-model/src/data/repository.rs` — C12: document the `f64::NAN` sentinel at its source
  (`:259-260`), pointing at the boundary conversion.
- `antenna-model/src/api/routes.rs` — C12: extend
  `test_antenna_details_with_uncalibrated_status` (`:1100+`) to assert the *shape* of the
  `calibration` block, which nothing pins today.

**Contract + docs**
- `openapi.yaml` — the H3 removal (GridConfig, GridData, `/heatmap` 4xx blocks), the two error
  enums, and all seven antenna-family component schemas + the list-feeds wrapper.
- `docs/api-documentation.md` — error-code table row; the H3-on-`/heatmap` mentions; the
  two-endpoint rationale; feed/antenna response field names.
- `docs/architecture.md` — the `grid_type: "h3"` heatmap request example (`:931`).
- `examples/api_requests.json` — the two `grid_type: "h3"` examples (`:286` request,
  `:331`-region `heatmap_response_h3`).
- `examples/responses/antenna_details_response.json` — drop the two now-absent `null` keys.
- `docs/domain-contract.md` — record the H3-stub removal + the C12 shape call.
- `docs/roadmap-2026-07.md`, `docs/roadmap-2026-07-work-units.md` — mark stage 4, C12, C14 done;
  note the D2/D9 follow-up from §3.1.
- `CLAUDE.md` — the `/heatmap` H3 stub is described as live; re-true it.

**Optional (Task 8, C15 option 1)**
- `antenna-model/tests/example_api_requests_deserialize.rs` — new guard over
  `examples/api_requests.json`.

## 5. Standing constraints (from C8's charter — do not violate)

1. **No computed value moves.** Every existing numeric assertion in the workspace must still
   hold, unchanged. That property is what makes a breaking pass reviewable. If a number moves,
   stop and report — it means something outside the contract layer was touched.
2. **Mirror openapi + examples + api-documentation by hand, in the same commit** (standing rule
   4 in the work-units doc — C7's guard does not exist yet).
3. **No physics or service-semantics change.** Removal of dead arms only.
4. Leave the workspace green after every task: `cargo test --workspace`.

---

## Task 1: Remove the `/heatmap` H3 grid-type stub (Rust)

**Goal:** Delete the `H3` variants from `GridConfig` and `GridData` and every arm that
matched them, so an `h3` grid type on `/api/v1/heatmap` becomes an unparseable body (400)
instead of a stub that parses, validates, and then fails with `not_implemented`.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:469-492` (`GridConfig`), `:553-577` (`GridData`),
  `:1432-1445` (`test_grid_config_h3_serialization`)
- Modify: `antenna-model/src/service/validator.rs:549-598` (H3 match arm), `:882-932` (four H3
  unit tests)
- Modify: `antenna-model/src/service/heatmap.rs:204-208`, `:287-290`, `:700-703`
  (`test_h3_grid_not_implemented`)
- Test: `antenna-model/src/api/schemas.rs` (inline `#[cfg(test)]`),
  `antenna-model/tests/integration/status_code_matrix_tests.rs`

**Scope note (mechanical fixes are in-scope).** `grep -rn 'GridConfig::H3\|GridData::H3' .`
(excluding `target/`) surfaces exactly the call sites listed above plus two test helpers —
`antenna-model/tests/integration/helpers.rs:481` and
`antenna-model/tests/integration/h3_link_budget_tests.rs:193`, both of which match `GridData::H3`
in a `match` over the heatmap response. **Update those match arms and keep going; do not stop to
ask for scope approval.** They are non-exhaustive-match compile errors, not semantic changes.
Reserve STOP-and-report for a caller whose *behavior* would have to change.

**Acceptance Criteria:**
- [ ] `GridConfig` and `GridData` each have exactly one variant, `Rectangular`, and both keep
      `#[serde(tag = "grid_type", rename_all = "lowercase")]` (decision §3.3).
- [ ] `{"grid_type":"h3", …}` fails `GridConfig` deserialization with an "unknown variant" error.
- [ ] `POST /api/v1/heatmap` with an `h3` grid type returns **400** with error code
      `invalid_request_body` (decision §3.2).
- [ ] `grep -rn 'GridConfig::H3\|GridData::H3' antenna-model/ --include='*.rs'` returns nothing.
- [ ] No numeric assertion anywhere in the workspace changed.

**Verify:** `cargo test --workspace` → all pass (expect ~5 fewer tests: 1 in `schemas.rs`, 4 in
`validator.rs`, 1 in `heatmap.rs`, plus 1 new).

**Steps:**

- [ ] **Step 1: Write the failing tests**

In `antenna-model/src/api/schemas.rs`, *replace* `test_grid_config_h3_serialization`
(`:1432-1445`) with:

```rust
    #[test]
    fn h3_grid_type_is_rejected_as_an_unknown_variant() {
        // The `h3` grid type on /api/v1/heatmap was a NotImplemented stub until C8
        // stage 4 removed it; the real H3 grid is the separate POST /api/v1/h3-heatmap
        // endpoint. An `h3` tag is now an unknown variant — i.e. a body that cannot be
        // parsed, which under roadmap C2's policy is a 400, not a 422.
        let json = r#"{"grid_type":"h3","h3_resolution":7,"center_azimuth_deg":180.0,"center_elevation_deg":45.0,"field_of_view_deg":30.0}"#;

        let err = serde_json::from_str::<GridConfig>(json)
            .expect_err("an `h3` grid_type must not deserialize");

        assert!(
            err.to_string().contains("unknown variant"),
            "expected an unknown-variant parse error, got: {err}"
        );
    }
```

In `antenna-model/tests/integration/status_code_matrix_tests.rs`, append (match the file's
existing helper style — reuse whatever request-builder and `assert_json_error` helpers the
neighbouring tests use rather than hand-rolling a client):

```rust
/// C8 stage 4 removed the `/heatmap` H3 grid-type stub. An `h3` grid_type is now an
/// unknown serde variant, so the body cannot be parsed at all: 400, not the 422 the
/// stub used to return via `not_implemented`.
#[tokio::test]
async fn h3_grid_type_on_heatmap_is_rejected_with_400() {
    let server = test_server().await;

    let mut body = valid_heatmap_request_json();
    body["grid_config"] = serde_json::json!({
        "grid_type": "h3",
        "h3_resolution": 7,
        "center_azimuth_deg": 180.0,
        "center_elevation_deg": 45.0,
        "field_of_view_deg": 30.0
    });

    let response = server.post_json("/api/v1/heatmap", &body).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let parsed = assert_json_error(&response).await;
    assert_eq!(parsed.error, error_codes::INVALID_REQUEST_BODY);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p antenna-model h3_grid_type_is_rejected_as_an_unknown_variant
```
Expected: FAIL — the variant still exists, so the body parses and `expect_err` panics.

- [ ] **Step 3: Remove the `H3` variant from `GridConfig`**

In `antenna-model/src/api/schemas.rs`, replace the enum (currently `:469-492`) with:

```rust
/// Grid configuration for heatmap generation.
///
/// A **single-variant tagged enum by design**: `grid_type` stays in the wire contract so a
/// second grid family can be added later without a breaking change (feature F5 would merge
/// `/api/v1/h3-heatmap` back in here). Do not collapse this into a plain struct.
///
/// The `H3` variant that lived here until C8 stage 4 (2026-07-28) was a `NotImplemented`
/// stub — it parsed and validated, then failed. The real H3 grid is the separate
/// `POST /api/v1/h3-heatmap` endpoint. An `h3` tag is now an unknown variant → 400.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "grid_type", rename_all = "lowercase")]
pub enum GridConfig {
    /// Rectangular azimuth/elevation grid
    Rectangular {
        /// Azimuth range configuration
        azimuth_range_deg: RangeConfig,
        /// Elevation range configuration
        elevation_range_deg: RangeConfig,
    },
}
```

- [ ] **Step 4: Remove the `H3` variant from `GridData`**

In the same file (currently `:553-577`), replace with:

```rust
/// Grid data for heatmap.
///
/// Single-variant tagged enum for the same reason as [`GridConfig`] — `grid_type` stays on
/// the wire. The `H3` variant was removed by C8 stage 4 (2026-07-28); it had no producer,
/// because the only `GridConfig` that could have selected it was the `NotImplemented` stub.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "grid_type", rename_all = "lowercase")]
pub enum GridData {
    /// Rectangular grid data
    Rectangular {
        /// Azimuth values in degrees
        azimuth_values: Vec<f64>,
        /// Elevation values in degrees
        elevation_values: Vec<f64>,
        /// Loss values in dB (2D array: rows are elevation, columns are azimuth)
        loss_db: Vec<Vec<f64>>,
    },
}
```

- [ ] **Step 5: Remove the validator's H3 arm**

In `antenna-model/src/service/validator.rs`, delete the whole `GridConfig::H3 { … } => { … }`
arm (`:549-598`, from `GridConfig::H3 {` through the `Ok(())` and its closing brace). The
`match` then has one arm and stays exhaustive.

Then delete the four now-uncompilable unit tests in the same file (`:882-932`):
`test_validate_h3_grid_valid`, `test_validate_h3_grid_invalid_resolution`,
`test_validate_h3_grid_invalid_elevation`, `test_validate_h3_grid_invalid_fov`.

**Clippy note:** a `match` over a single-variant enum can trip
`clippy::infallible_destructuring_match` if it is only used to bind fields. If
`cargo clippy --workspace --all-targets -- -D warnings` flags it, convert the match to a
destructuring `let`:

```rust
    let GridConfig::Rectangular {
        azimuth_range_deg,
        elevation_range_deg,
    } = grid;
```

and keep the body. Do not add a `#[allow]`.

- [ ] **Step 6: Remove the two service arms in `heatmap.rs`**

In `antenna-model/src/service/heatmap.rs`, delete:

```rust
        GridConfig::H3 { .. } => {
            // H3 support: for now, return error indicating it's not implemented
            return Err(AntennaModelError::NotImplemented {
                feature: "H3 hexagonal grid".to_string(),
            });
        }
```

(`:204-208`) and

```rust
        GridConfig::H3 { .. } => Err(AntennaModelError::NotImplemented {
            feature: "H3 hexagonal grid".to_string(),
        }),
```

(`:287-290`), then delete `test_h3_grid_not_implemented` (`:698-703`). Apply the same
single-arm-match clippy note as Step 5 to `generate_grid_points`.

- [ ] **Step 7: Fix the two integration test-helper match arms**

`antenna-model/tests/integration/helpers.rs:481` and
`antenna-model/tests/integration/h3_link_budget_tests.rs:193` each carry a `GridData::H3` arm in
a `match` over the heatmap response grid. Delete both arms — the matches remain exhaustive over
the one-variant enum. If either match then trips the clippy lint from Step 5, apply the same
destructuring-`let` fix.

- [ ] **Step 8: Run the full suite**

```bash
cargo test --workspace
```
Expected: PASS. `cargo clippy --workspace --all-targets -- -D warnings` → clean.

- [ ] **Step 9: Commit**

```bash
git add antenna-model/src/api/schemas.rs antenna-model/src/service/validator.rs \
        antenna-model/src/service/heatmap.rs antenna-model/tests/integration/helpers.rs \
        antenna-model/tests/integration/h3_link_budget_tests.rs \
        antenna-model/tests/integration/status_code_matrix_tests.rs
git commit -m "feat(C8 stage 4): remove the /heatmap H3 grid-type stub

The H3 variant of GridConfig parsed and validated, then failed with
not_implemented. The real H3 grid is POST /api/v1/h3-heatmap. An \`h3\`
grid_type is now an unknown serde variant -> 400 invalid_request_body,
matching C2's policy (400 = unparseable body). Absorbs register row C5.

GridConfig/GridData stay single-variant tagged enums so grid_type remains
on the wire and feature F5 can add a variant back without a break.

No computed value moved."
```

---

## Task 2: Retire the producerless `not_implemented` error code

**Goal:** Remove `AntennaModelError::NotImplemented` and `error_codes::NOT_IMPLEMENTED` now that
Task 1 deleted their only producer, and update the three published surfaces the C3 drift guard
pins, so the error vocabulary contains no code the service cannot emit.

**Files:**
- Modify: `antenna-model/src/error.rs:62` (variant), `:569` (Display arm)
- Modify: `antenna-model/src/api/error_response.rs:134` (status mapping), `:295` (unit test)
- Modify: `antenna-model/src/api/schemas.rs` — `error_codes::NOT_IMPLEMENTED` const and its
  entry in `error_codes::ALL`
- Modify: `openapi.yaml:1993` (`GainError.code` enum), `:2025` (`ErrorResponse.error` enum)
- Modify: `docs/api-documentation.md:575` (error-code table row)
- Test: `antenna-model/tests/error_code_vocabulary.rs` (existing guard — drives this task)

**Acceptance Criteria:**
- [ ] `grep -rn 'NotImplemented\|not_implemented' antenna-model/src/ openapi.yaml docs/ examples/`
      returns no hits outside historical roadmap/plan docs.
- [ ] `error_codes::ALL` has 10 entries; the two openapi enums and the api-documentation table
      list exactly those 10.
- [ ] `cargo test --test error_code_vocabulary` passes (all 3 tests).
- [ ] No numeric assertion anywhere in the workspace changed.

**Verify:** `cargo test --workspace` → all pass.

**Steps:**

- [ ] **Step 1: Watch the existing guard fail first**

Delete the const and its `ALL` entry in `antenna-model/src/api/schemas.rs`:

```rust
    /// A requested option is recognized but unimplemented — currently only the
    /// `/heatmap` H3 grid-type stub, which C8 stage 4 removes (422).
    pub const NOT_IMPLEMENTED: &str = "not_implemented";
```

...and the `NOT_IMPLEMENTED,` line inside `pub const ALL: &[&str] = &[ … ];`.

Then run:

```bash
cargo test --test error_code_vocabulary
```
Expected: FAIL — the guard reports that `openapi.yaml` and/or `docs/api-documentation.md` list
a code that is not in `error_codes::ALL`. This is the guard doing its job; let it drive Steps
2–4.

- [ ] **Step 2: Remove the Rust error variant and its mapping**

`antenna-model/src/error.rs` — delete the variant (`:62`) and its `Display`/message arm
(`:569`). `antenna-model/src/api/error_response.rs` — delete the match arm at `:134`:

```rust
        AntennaModelError::NotImplemented { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            error_codes::NOT_IMPLEMENTED,
        ),
```

and the corresponding case in the status-mapping table unit test at `:295`.

- [ ] **Step 3: Remove the code from both openapi enums**

In `openapi.yaml`, delete the `- not_implemented` line from `GainError.code`'s `enum` (`:1993`)
**and** from `ErrorResponse.error`'s `enum` (`:2025`). Both must be edited — the guard checks
each.

- [ ] **Step 4: Remove the api-documentation table row**

In `docs/api-documentation.md`, delete line `:575`:

```
| `not_implemented` | 422 | A recognized but unimplemented option — currently only `/heatmap`'s H3 grid type. |
```

- [ ] **Step 5: Verify the guard now passes**

```bash
cargo test --test error_code_vocabulary
```
Expected: PASS (3 tests).

- [ ] **Step 6: Prove the guard would still catch drift**

Temporarily re-add `- not_implemented` to `ErrorResponse.error`'s enum in `openapi.yaml`, run
`cargo test --test error_code_vocabulary` and confirm it FAILS, then revert. (Stages 1–3 each
verified their new guards against injected drift before accepting them; do the same here for the
guard's *narrowed* vocabulary.)

- [ ] **Step 7: Commit**

```bash
git add antenna-model/src/error.rs antenna-model/src/api/error_response.rs \
        antenna-model/src/api/schemas.rs openapi.yaml docs/api-documentation.md
git commit -m "feat(C8 stage 4): retire the producerless not_implemented error code

Task 1 deleted the /heatmap H3 stub, the sole producer of
AntennaModelError::NotImplemented. A code no site can emit is the defect C3
removed when it deleted the PascalCase ErrorResponse constructors, so the
variant, the status mapping, the constant and all three published surfaces go
with it. The vocabulary is 10 codes.

C3's drift guard drove this: it failed until openapi's two enums and the
api-documentation table matched error_codes::ALL.

No computed value moved."
```

---

## Task 3: Mirror the H3 removal into openapi, docs and examples

**Goal:** Bring every published description of `/api/v1/heatmap` in line with Task 1 — the
grid-type enum, the request/response schemas, the 4xx blocks, and the four documents that show
an `h3` heatmap example — so no reader or codegen tool believes `/heatmap` accepts an H3 grid.

**Files:**
- Modify: `openapi.yaml:1591-1631` (`GridConfig`), `:1669-1689` (`GridData`), `:620-645`
  (`/heatmap` 422 block)
- Modify: `docs/api-documentation.md` (the `/heatmap` grid-type description; the two-endpoint
  rationale from decision §3.6)
- Modify: `docs/architecture.md:929-937` (heatmap request example with `grid_type: "h3"`)
- Modify: `examples/api_requests.json` (`heatmap_request_h3` around `:275-292`;
  `heatmap_response_h3` around `:322-345`)
- Test: `antenna-model/tests/doc_examples_deserialize.rs` (existing C11 guard),
  `antenna-model/tests/example_requests_deserialize.rs` (existing G3 guard)

**Acceptance Criteria:**
- [ ] `openapi.yaml`'s `GridConfig.grid_type` enum lists `rectangular` only, and the four
      H3-only properties (`h3_resolution`, `center_azimuth_deg`, `center_elevation_deg`,
      `field_of_view_deg`) are gone from it.
- [ ] `openapi.yaml`'s `GridData` is a tagged object whose `grid_type` enum lists `rectangular`
      only.
- [ ] The `/heatmap` 422 block no longer mentions the H3 grid type or carries the
      `not_implemented` example.
- [ ] `grep -rn '"grid_type": *"h3"' openapi.yaml docs/ examples/` returns nothing.
- [ ] `docs/api-documentation.md` states why `/heatmap` and `/h3-heatmap` are separate
      endpoints (decision §3.6) and points at F5 for the merge.
- [ ] No numeric assertion anywhere in the workspace changed.

**Verify:** `cargo test --workspace` → all pass; `grep -rn '"grid_type": *"h3"' . --include='*.json' --include='*.md' --include='*.yaml' | grep -v roadmap | grep -v plan-` → empty.

**Steps:**

- [ ] **Step 1: Rewrite `openapi.yaml`'s `GridConfig` component**

Replace `:1591-1631` with:

```yaml
    GridConfig:
      type: object
      description: >
        Grid specification for POST /api/v1/heatmap. Discriminated by `grid_type`.
        Only `rectangular` exists today: the `h3` grid type was a not-implemented stub
        and was removed in 2026-07 (roadmap C8 stage 4). The real H3 grid is the
        separate POST /api/v1/h3-heatmap endpoint. A `grid_type` of `h3` is now an
        unparseable body and returns 400 invalid_request_body.
      required:
        - grid_type
        - azimuth_range_deg
        - elevation_range_deg
      properties:
        grid_type:
          type: string
          enum:
            - rectangular
        azimuth_range_deg:
          $ref: '#/components/schemas/RangeConfig'
        elevation_range_deg:
          $ref: '#/components/schemas/RangeConfig'
```

Then add the `RangeConfig` component alongside it (the inline `{min,max,step}` objects were
duplicated in the old schema; the Rust type is a named struct, so name it here too):

```yaml
    RangeConfig:
      type: object
      required:
        - min
        - max
        - step
      properties:
        min:
          type: number
          description: Minimum value in degrees
        max:
          type: number
          description: Maximum value in degrees
        step:
          type: number
          description: Step size in degrees (must be > 0)
```

- [ ] **Step 2: Rewrite `openapi.yaml`'s `GridData` component**

Replace `:1669-1689` with:

```yaml
    GridData:
      type: object
      description: >
        Heatmap grid payload, discriminated by `grid_type` to mirror GridConfig.
        The `h3` variant was removed with the stub in 2026-07 (roadmap C8 stage 4).
      required:
        - grid_type
        - azimuth_values
        - elevation_values
        - loss_db
      properties:
        grid_type:
          type: string
          enum:
            - rectangular
        azimuth_values:
          type: array
          items:
            type: number
          description: Azimuth values in degrees, one per grid column
        elevation_values:
          type: array
          items:
            type: number
          description: Elevation values in degrees, one per grid row
        loss_db:
          type: array
          description: >
            2D array of loss values indexed [elevation][azimuth], referenced to
            metadata.peak_gain_db (roadmap C9). Failed points carry the sentinel
            999999.0.
          items:
            type: array
            items:
              type: number
```

- [ ] **Step 3: Clean the `/heatmap` 422 block**

In `openapi.yaml:620-645`, change the 422 `description` from

```
            The body parsed but is semantically invalid — an out-of-range value, a
            degenerate geometry, a grid exceeding 100,000 points, or the unimplemented
            H3 grid type (`not_implemented`).
```

to

```
            The body parsed but is semantically invalid — an out-of-range value, a
            degenerate geometry, or a grid exceeding 100,000 points.
```

and delete the whole `not_implemented:` example block (the `summary`, `value.error` and
`value.message` lines). Keep the `invalid_grid` example.

Then, in the same path's **400** block, add an example so the new behavior is discoverable:

```yaml
                unsupported_grid_type:
                  summary: H3 grid type (removed — use POST /api/v1/h3-heatmap)
                  value:
                    error: "invalid_request_body"
                    message: "unknown variant `h3`, expected `rectangular`"
```

- [ ] **Step 4: Fix `docs/architecture.md`**

At `:929-937`, replace the `grid_config` block of the heatmap request example:

```json
  "grid_config": {
    "grid_type": "rectangular",
    "azimuth_range_deg": { "min": 0.0, "max": 360.0, "step": 5.0 },
    "elevation_range_deg": { "min": 0.0, "max": 90.0, "step": 2.0 }
  }
```

Check the surrounding "Heatmap Response" example immediately below it (`:940+`) for a
`"grid_type": "h3"` payload and replace it with the rectangular shape if present.

- [ ] **Step 5: Fix `examples/api_requests.json`**

Delete the two H3 examples outright rather than converting them — a rectangular equivalent
already exists in the same file, so converting would leave a duplicate:
- the `heatmap_request_h3` entry (its `grid_config` is at `:284-292`), and
- the `heatmap_response_h3` entry (`:322-345`).

Remove each entry's whole `"<name>": { … }` block including its `description`, and fix the
trailing commas so the file stays valid JSON. Verify:

```bash
python3 -c "import json;json.load(open('examples/api_requests.json'));print('valid json')"
```
Expected: `valid json`.

- [ ] **Step 6: Update `docs/api-documentation.md`**

Two edits:

(a) Wherever the `/api/v1/heatmap` request is described, state that `grid_type` accepts
`rectangular` only.

(b) In the section that introduces the heatmap endpoints (near `:236`, "H3 Link Budget Grid"),
add the decision §3.6 rationale:

```markdown
**Why two heatmap endpoints.** `POST /api/v1/heatmap` returns a *loss surface* over a
rectangular azimuth/elevation grid in the antenna's own frame. `POST /api/v1/h3-heatmap`
returns a per-cell *link budget* — gain, free-space path loss, total path loss and G/T —
over an H3 hexagonal grid laid on the Earth's surface, centred on an Earth location. They
differ in output, in reference frame and in what the caller must supply, not merely in grid
shape, so they stay separate endpoints. `/heatmap` carried an `h3` grid type until 2026-07;
it was a not-implemented stub and was removed (roadmap C8 stage 4). Merging the two is
tracked as roadmap feature **F5**.
```

- [ ] **Step 7: Verify the example guards still pass**

```bash
cargo test --workspace
```
Expected: PASS — in particular `doc_examples_deserialize` (C11) and
`example_requests_deserialize` (G3).

- [ ] **Step 8: Commit**

```bash
git add openapi.yaml docs/api-documentation.md docs/architecture.md examples/api_requests.json
git commit -m "docs(C8 stage 4): mirror the H3 grid-type removal into openapi, docs, examples

GridConfig/GridData now declare grid_type: [rectangular] only, with RangeConfig
extracted as a named component to match the Rust struct. The /heatmap 422 block
loses the not_implemented example and gains a 400 unsupported_grid_type example.
The four h3 heatmap examples across architecture.md and api_requests.json are
removed. api-documentation.md records why the two heatmap endpoints stay separate
(decision C8 s4 3.6; merge remains feature F5).

No computed value moved."
```

---

## Task 4: C12 — declare the `null` on `rmse_db`/`r_squared` via `nan_as_null`

**Goal:** Make the `null` that `GET /api/v1/antennas/{id}` already emits for uncalibrated
antennas a *declared* part of the contract instead of an accident, by moving
`CalibrationInfo.rmse_db`/`r_squared` onto the codebase's existing `nan_as_null` convention —
the same declaration `GainResponse.gain_db` carries — and removing the `Option` +
`skip_serializing_if` machinery that promises an omission the code never performs.

**Decision (recorded, do not re-litigate):** option 2, per decision §3.1. `null` = "the slot
exists, the value does not"; omission stays reserved for structurally absent things (`mesh`,
`coverage`). **The wire output does not change** — it is `null` today and `null` after. What
changes is that the Rust type, the doc comments, the spec and the docs all now say so.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:854-871` (`CalibrationInfo` — the two fields'
  types, attributes and doc comments)
- Modify: `antenna-model/src/api/handlers.rs:699-706` (the sole `CalibrationInfo` construction)
- Modify: `antenna-model/src/data/repository.rs:258-260` (document the sentinel at its source)
- Modify: `antenna-model/src/api/routes.rs` — extend
  `test_antenna_details_with_uncalibrated_status` (around `:1165-1192`)
- **Unchanged:** `examples/responses/antenna_details_response.json` — its two `null`s are
  already correct and stay
- **Unchanged:** `antenna-model/src/data/types.rs` — the metadata fields stay `f64` + NaN
  sentinel, which now matches the API type exactly
- **Not here:** `openapi.yaml`'s `CalibrationInfo` (`:1828`) — Task 5 rewrites that component
  wholesale; `docs/api-documentation.md` — Task 6 documents the shape

**Scope note (mechanical fixes are in-scope).** `grep -rn 'rmse_db\|r_squared' .` across the
workspace confirms `CalibrationInfo`'s two fields are **constructed once**
(`handlers.rs:703-704`) and **read nowhere**. Every other hit belongs to
`CalibrationMetadata`, `calibrate`'s own fit-stats types, or test builders — none of which this
task touches. If the type change surfaces an unexpected call site, fix it mechanically and keep
going.

**Acceptance Criteria:**
- [ ] `CalibrationInfo.rmse_db` and `r_squared` are plain `f64` with
      `#[serde(with = "nan_as_null")]` — no `Option`, no `skip_serializing_if`.
- [ ] `GET /api/v1/antennas/{id}` on an uncalibrated antenna returns a `calibration` object in
      which `rmse_db` and `r_squared` are **present and JSON `null`** (not absent).
- [ ] The same endpoint on a calibrated antenna still returns both as numbers (the existing
      assertion at `routes.rs:867`, `calibration.get("rmse_db").f64() == 0.5`, is unchanged).
- [ ] `antenna-model/src/data/types.rs` is **not** modified — `CalibrationMetadata.rmse_db` /
      `r_squared` stay `f64` (making them `Option<f64>` is an ANTC wire break belonging to D2).
- [ ] `examples/responses/antenna_details_response.json` is **not** modified — its `null`s were
      right all along.
- [ ] The two doc comments say "null for uncalibrated antennas", not "None".
- [ ] No numeric assertion anywhere in the workspace changed.

**Verify:**
`cargo test -p antenna-model test_antenna_details_with_uncalibrated_status -- --nocapture` → PASS

**Steps:**

- [ ] **Step 1: Write the failing test**

In `antenna-model/src/api/routes.rs`, extend `test_antenna_details_with_uncalibrated_status`.
The test currently reads the body with poem's `response.json()` helper, which has no
key-presence/absence assertion — so read the raw body and parse it with `serde_json` instead,
the pattern already used at `routes.rs:440`. Append this *after* the existing
`calibration_status` assertions, replacing the test's final `}`:

```rust
        // C12: an uncalibrated antenna has no fit, so rmse_db / r_squared are JSON null —
        // PRESENT with no value, not absent. `null` is the API's convention for "the slot
        // exists, the value does not" (the same rule GainResponse.gain_db follows via
        // nan_as_null); omission is reserved for structurally absent things like `mesh`.
        // A client branches on calibration_status.status or num_measurements, both typed.
        let raw = response.0.into_body().into_string().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let calibration = parsed
            .get("calibration")
            .and_then(|c| c.as_object())
            .expect("an uncalibrated antenna still reports a calibration block");

        assert_eq!(
            calibration.get("rmse_db"),
            Some(&serde_json::Value::Null),
            "rmse_db must be present and null for an uncalibrated antenna, not omitted; \
             got: {calibration:?}"
        );
        assert_eq!(
            calibration.get("r_squared"),
            Some(&serde_json::Value::Null),
            "r_squared must be present and null for an uncalibrated antenna, not omitted; \
             got: {calibration:?}"
        );
        // The typed signal a client should actually branch on.
        assert_eq!(
            calibration.get("num_measurements").and_then(|v| v.as_u64()),
            Some(0)
        );
    }
```

Because the raw body is consumed here, move the *existing* assertions in that test above this
block, or re-parse them from `parsed` — whichever keeps the test compiling.
(`response.0.into_body()` consumes the response, so it must be the last read.)

**Note:** this test passes *by accident* against today's code — `Some(f64::NAN)` already
serializes to `null`. That is the point: the wire shape is not what is broken. Step 2 makes the
*type* honest, and this test is what stops a future editor from "fixing" the `Option` back into
an omission. Confirm it passes before Step 2 as well as after, and say so in the PR.

- [ ] **Step 2: Move the two fields onto the `nan_as_null` convention**

In `antenna-model/src/api/schemas.rs:854-871`, replace the two field declarations inside
`CalibrationInfo`:

```rust
    /// RMSE of the combined model (physics + correction) in dB.
    ///
    /// Serialized as JSON `null` for uncalibrated (design-spec) antennas, which have no
    /// fit — the same "slot exists, value does not" convention `GainResponse.gain_db`
    /// uses (roadmap C12, 2026-07-28). It is **present and null**, never omitted;
    /// omission is reserved for structurally absent members such as
    /// `PhysicalParametersInfo.mesh`. Branch on `calibration_status.status` or
    /// `num_measurements`, which carry the same information typed.
    #[serde(with = "nan_as_null")]
    pub rmse_db: f64,

    /// R² correlation coefficient of the combined model.
    ///
    /// Serialized as JSON `null` for uncalibrated antennas — see `rmse_db`.
    #[serde(with = "nan_as_null")]
    pub r_squared: f64,
```

This deletes the `Option<f64>` types, both `#[serde(skip_serializing_if = "Option::is_none")]`
attributes, and the *"(None for uncalibrated antennas)"* comments that were never true. Do **not**
merely delete the `skip_serializing_if` attributes and keep `Option<f64>`: that leaves an
`Option` which is always `Some`, i.e. a second type that lies about the same field.

- [ ] **Step 3: Simplify the construction site**

In `antenna-model/src/api/handlers.rs:699-706`, change:

```rust
        rmse_db: Some(calibration.metadata.rmse_db),
        r_squared: Some(calibration.metadata.r_squared),
```

to:

```rust
        rmse_db: calibration.metadata.rmse_db,
        r_squared: calibration.metadata.r_squared,
```

The API type and the metadata type are now the same shape, so there is no boundary conversion.

- [ ] **Step 4: Document the sentinel at its source**

In `antenna-model/src/data/repository.rs`, above the two fields at `:259-260`:

```rust
                // Sentinel: no calibration was fitted, so there is no error metric.
                // `CalibrationMetadata` is postcard-serialized into the ANTC artifact
                // (positional, non-self-describing), so these cannot become `Option`
                // without a format bump — see roadmap D2. The API surfaces them with
                // `#[serde(with = "nan_as_null")]`, so this NaN reaches the client as a
                // deliberate JSON `null` (roadmap C12, 2026-07-28), matching gain_db.
                //
                // Note `data/loader.rs:268,275` warns on `rmse_db > 1.0` / `r_squared <
                // 0.95`; both are false for NaN, so design-spec antennas load without a
                // spurious quality warning. That is intended — do not "fix" it.
                rmse_db: f64::NAN,
                r_squared: f64::NAN,
```

- [ ] **Step 5: Run the test and the full suite**

```bash
cargo test -p antenna-model test_antenna_details_with_uncalibrated_status
cargo test --workspace
```
Expected: PASS. Watch specifically that `routes.rs:867`
(`calibration.get("rmse_db").f64() == 0.5`) still passes — that is the calibrated half, and it
proves the change did not turn real values into nulls.

- [ ] **Step 6: Confirm the two files that must NOT change are untouched**

```bash
git status --short examples/responses/antenna_details_response.json antenna-model/src/data/types.rs
```
Expected: empty output. The example's `null`s were correct before this task and remain correct;
the metadata type stays `f64`.

- [ ] **Step 7: Commit**

```bash
git add antenna-model/src/api/schemas.rs antenna-model/src/api/handlers.rs \
        antenna-model/src/data/repository.rs antenna-model/src/api/routes.rs
git commit -m "fix(C12): declare the null on CalibrationInfo.rmse_db / r_squared

The fields were typed Option<f64> with skip_serializing_if and documented as
\"None for uncalibrated antennas\", but handlers.rs wrapped an f64::NAN sentinel in
Some() unconditionally, so the attribute never fired and every uncalibrated antenna
served \`\"rmse_db\": null\`. The type promised an omission the code never performed.

Resolved in favour of the null (maintainer, 2026-07-28): both fields become plain
f64 with #[serde(with = \"nan_as_null\")], the same declaration GainResponse.gain_db
carries. The API now has one rule for each case — null means the slot exists and the
value does not; omission stays reserved for structurally absent members (mesh,
coverage). A client branches on calibration_status.status or num_measurements.

The wire output is unchanged (null before, null after); the Rust type, the doc
comments and — via Tasks 5 and 6 — the spec and docs now agree with it. Adds the
uncalibrated-shape assertion nothing pinned before: all four .bin antennas are
disabled (D9), so the only shape served was the untested one.

CalibrationMetadata stays f64 + NaN sentinel; making it Option is an ANTC wire break
belonging to D2.

No computed value moved."
```

---

## Task 5: C14+ — reconcile every antenna-endpoint schema in `openapi.yaml`

**Goal:** Rewrite the seven component schemas and one inline wrapper behind
`GET /api/v1/antennas`, `/antennas/{id}`, `/antennas/{id}/feeds` and
`/antennas/{id}/feeds/{feed_id}` so each field name, type and required-ness matches the Rust
type the handler actually serializes — the stage-4 exit criterion, and the reason C7's freeze is
worth having.

**Files:**
- Modify: `openapi.yaml` — `AntennaInfo` (`:1698`), `AntennaDetailsResponse` (`:1714`),
  `FeedInfo` (`:1737`), `PhysicalParametersInfo` (`:1779`), `ValidityRangesInfo` (`:1804`),
  `CalibrationInfo` (`:1828`), `GridData` (done in Task 3), and the list-feeds 200 wrapper
  (`:974-981`)
- Read (source of truth, do not modify): `antenna-model/src/api/schemas.rs` — `AntennaInfo`
  (`:768`), `AntennaDetailsResponse` (`:786`), `FeedInfo` (`:817`), `ValidityRangesInfo`
  (`:838`), `CalibrationInfo` (`:854`), `PhysicalParametersInfo` (`:872`), `MeshInfo` (`:894`),
  `CalibrationStatusInfo` (`:922`)
- Read: `antenna-model/src/api/handlers.rs:801-819` (list-feeds), `:863-871` (feed details),
  `:699-706` (calibration info)

**Acceptance Criteria:**
- [ ] For each of the four antenna routes, every property name in the openapi response schema
      appears in the Rust type, and every non-`skip_serializing_if` Rust field appears in the
      schema.
- [ ] `FeedInfo` declares exactly `id`, `design_feed_offset_m`, `frequency_range_mhz`,
      `q_factor` — C14(a).
- [ ] The list-feeds 200 wrapper declares `feeds` only — C14(b), decision §3.5.
- [ ] The `# DRIFT` comment block at `openapi.yaml:1739-1744` is **deleted**, not updated (YAML
      parsers strip comments, so it is invisible to Swagger UI, codegen and C7's guard alike).
- [ ] `CalibrationInfo` declares `rmse_db`/`r_squared` as `nullable: true` and documents them as
      **`null` for uncalibrated antennas** (Task 4's behavior), and includes `r_squared`, which
      the spec omitted entirely.
- [ ] `openapi.yaml` parses: `python3 -c "import sys,json"` is not enough — use the yaml check in
      Step 7.
- [ ] No Rust source file is modified by this task.

**Verify:**
```bash
uv run --quiet --with pyyaml python3 -c "import yaml;yaml.safe_load(open('openapi.yaml'));print('openapi parses')"
cargo test --workspace
```
→ `openapi parses`, then all tests pass.

**Steps:**

- [ ] **Step 1: Rewrite `AntennaInfo`**

Replace `openapi.yaml:1698-1713`:

```yaml
    AntennaInfo:
      type: object
      description: Summary entry in the GET /api/v1/antennas listing.
      required:
        - id
        - name
        - enabled
        - feed_count
        - feed_ids
      properties:
        id:
          type: string
          description: Antenna identifier, used as the `antenna_id` of every request.
        name:
          type: string
          description: Human-readable antenna name.
        enabled:
          type: boolean
          description: >
            Whether this antenna is served. Entries disabled in antennas.yaml are
            listed but rejected by the compute endpoints.
        feed_count:
          type: integer
          description: Number of feeds available on this antenna.
        feed_ids:
          type: array
          items:
            type: string
          description: Identifiers of the available feeds, used as `feed_id` in requests.
```

- [ ] **Step 2: Rewrite `AntennaDetailsResponse`**

Replace `openapi.yaml:1714-1736`:

```yaml
    AntennaDetailsResponse:
      type: object
      description: Body of GET /api/v1/antennas/{id}.
      required:
        - id
        - name
        - enabled
        - feeds
        - validity_ranges
        - calibration
        - physical_parameters
      properties:
        id:
          type: string
        name:
          type: string
        enabled:
          type: boolean
        feeds:
          type: array
          items:
            $ref: '#/components/schemas/FeedInfo'
        validity_ranges:
          $ref: '#/components/schemas/ValidityRangesInfo'
        calibration:
          $ref: '#/components/schemas/CalibrationInfo'
        physical_parameters:
          $ref: '#/components/schemas/PhysicalParametersInfo'
        calibration_status:
          allOf:
            - $ref: '#/components/schemas/CalibrationStatusInfo'
          description: >
            Calibration status and accuracy estimates. Omitted (not null) when the
            repository has no status for this antenna.
```

Note the two corrections beyond renaming: the old spec had **both** a `calibration_status:
string` and a `calibration_status_info` object; the Rust type has one field,
`calibration_status: Option<CalibrationStatusInfo>`. And `enabled` was missing entirely.

- [ ] **Step 3: Rewrite `FeedInfo` (C14(a)) and delete the `# DRIFT` comment**

Replace `openapi.yaml:1737-1778` — including the `# DRIFT (filed, not fixed here …)` comment
block — with:

```yaml
    FeedInfo:
      type: object
      description: >
        A feed on an antenna, as returned by GET /api/v1/antennas/{id} (inside `feeds`),
        GET /api/v1/antennas/{id}/feeds and GET /api/v1/antennas/{id}/feeds/{feed_id}.
      required:
        - id
        - design_feed_offset_m
        - frequency_range_mhz
        - q_factor
      properties:
        id:
          type: string
          description: Feed identifier, used as the `feed_id` of every request.
        design_feed_offset_m:
          type: object
          description: >
            The feed's DESIGN offset from the focal point in the antenna frame
            (meters) - a static property of this antenna's configuration, identical
            for every request. It is a physical displacement, NOT an Earth location:
            it is not the aim point `feed_pointing_location`. The per-request total
            (this design offset plus the beam-steering displacement) is reported as
            GeometryInfo.physical_feed_offset_m.

            z is positive AWAY from the reflector vertex, matching the phase model's
            delta_z convention.
          required:
            - x
            - y
            - z
          properties:
            x:
              type: number
            y:
              type: number
            z:
              type: number
        frequency_range_mhz:
          type: array
          description: Valid frequency range [min, max] in MHz.
          items:
            type: number
          minItems: 2
          maxItems: 2
        q_factor:
          type: number
          description: Feed illumination pattern q-factor (cos^q model).
```

The removed `name` and `phase_center_offset_m` have no emitter — both `FeedInfo` constructions
(`handlers.rs:801-809`, `:863-871`) build exactly the four fields above.

- [ ] **Step 4: Fix the list-feeds 200 wrapper (C14(b))**

In `openapi.yaml:974-981`, replace the inline schema:

```yaml
              schema:
                type: object
                required:
                  - feeds
                properties:
                  feeds:
                    type: array
                    items:
                      $ref: '#/components/schemas/FeedInfo'
```

(The `antenna_id` property is deleted per decision §3.5 — the handler returns
`json!({ "feeds": feeds })` and the caller already has the id from the path. Adding it later
would be non-breaking.)

- [ ] **Step 5: Rewrite `PhysicalParametersInfo` and `ValidityRangesInfo`**

Replace `openapi.yaml:1779-1827`:

```yaml
    PhysicalParametersInfo:
      type: object
      description: Reflector geometry as configured for this antenna.
      required:
        - diameter_m
        - focal_length_m
        - f_over_d_ratio
        - surface_rms_mm
      properties:
        diameter_m:
          type: number
          description: Reflector diameter in meters.
        focal_length_m:
          type: number
          description: Focal length in meters.
        f_over_d_ratio:
          type: number
          description: Focal length divided by diameter.
        surface_rms_mm:
          type: number
          description: >
            Reflector surface RMS error in millimeters, consumed as the Ruze
            efficiency term.
        mesh:
          allOf:
            - $ref: '#/components/schemas/MeshInfo'
          description: Mesh parameters. Omitted (not null) for a solid reflector.

    MeshInfo:
      type: object
      required:
        - mesh_spacing_mm
        - wire_diameter_mm
      properties:
        mesh_spacing_mm:
          type: number
        wire_diameter_mm:
          type: number

    ValidityRangesInfo:
      type: object
      description: >
        The ranges over which this antenna's calibration is valid. Queries outside
        them are still answered, with an `extrapolated` warning.
      required:
        - azimuth_deg
        - elevation_deg
        - frequency_mhz
        - temperature_k
      properties:
        azimuth_deg:
          type: array
          items:
            type: number
          minItems: 2
          maxItems: 2
        elevation_deg:
          type: array
          items:
            type: number
          minItems: 2
          maxItems: 2
        frequency_mhz:
          type: array
          items:
            type: number
          minItems: 2
          maxItems: 2
        temperature_k:
          type: number
          description: >
            The single temperature the correction surface is evaluated at
            (`temperature_const`), not a range.
```

Two corrections beyond renaming: the spurious nested `feed: {q_factor, phase_center_offset_m}`
object is gone (`PhysicalParametersInfo` has no `feed` field — those live on `FeedInfo`), and
`MeshInfo` becomes a named component instead of an inline `nullable` object, matching the Rust
`Option<MeshInfo>` with `skip_serializing_if` (omitted, not null).

- [ ] **Step 6: Rewrite `CalibrationInfo`**

Replace `openapi.yaml:1828-1843`:

```yaml
    CalibrationInfo:
      type: object
      description: Provenance and fit quality of this antenna's calibration.
      required:
        - date
        - version
        - source
        - rmse_db
        - r_squared
        - num_measurements
      properties:
        date:
          type: string
          description: >
            ISO 8601 calibration timestamp, or "N/A" for a design-spec
            (uncalibrated) antenna.
        version:
          type: string
          description: Calibration format version (e.g. "2.0").
        source:
          type: string
          description: >
            Where the measurements came from, e.g. a file name or
            "design_specifications" for an uncalibrated antenna.
        rmse_db:
          type: number
          nullable: true
          description: >
            RMSE of the combined model (physics + correction) in dB. NULL - present,
            with no value - for uncalibrated antennas, which have no fit (roadmap
            C12). This is the API's convention for a slot that exists without a
            value, the same one GainResponse.gain_db follows; omission is reserved
            for structurally absent members such as PhysicalParametersInfo.mesh.
            Branch on `num_measurements` or `calibration_status.status`, which carry
            the same information typed.
        r_squared:
          type: number
          nullable: true
          description: >
            R-squared of the combined model. NULL for uncalibrated antennas - see
            rmse_db (roadmap C12).
        num_measurements:
          type: integer
          description: Measurement points used; 0 for an uncalibrated antenna.
```

The spurious `parameters_tuned` is gone (it lives on the *artifact* metadata, not on the API's
`CalibrationInfo`; the API surfaces the equivalent as
`calibration_status.parameters_source`), and the entirely-missing `r_squared` is added. Note both
number fields are `required` **and** `nullable` — that is the correct OpenAPI 3.0 encoding for
"always present, sometimes null", and it is what Task 4 made the Rust type say.

- [ ] **Step 7: Verify the spec parses and matches the code, field by field**

```bash
uv run --quiet --with pyyaml python3 -c "import yaml;yaml.safe_load(open('openapi.yaml'));print('openapi parses')"
```
Expected: `openapi parses`.

Then walk each of the four antenna routes by hand against `api/schemas.rs`, checking name, type
and required-ness. Record the audit in the PR description — this is the exit criterion, and
there is no automated check for it until C7.

- [ ] **Step 8: Run the full suite**

```bash
cargo test --workspace
```
Expected: PASS (no Rust changed; this confirms nothing else referenced the old schema names).

- [ ] **Step 9: Commit**

```bash
git add openapi.yaml
git commit -m "docs(C8 stage 4): reconcile every antenna-endpoint schema with the code (C14+)

Every one of the seven openapi components behind /api/v1/antennas* was wrong in at
least one field; C14 had filed two of the eight defects. A client coding to the spec
would not have found a single field on GET /api/v1/antennas.

- AntennaInfo: antenna_id -> id, feeds -> feed_ids
- AntennaDetailsResponse: antenna_id -> id, +enabled, calibration_info -> calibration,
  dropped the spurious calibration_status string, calibration_status_info ->
  calibration_status
- FeedInfo (C14a): feed_id -> id, frequency_range -> frequency_range_mhz, dropped the
  emitterless name and phase_center_offset_m; the stripped-at-parse-time # DRIFT
  comment is deleted rather than updated
- list-feeds wrapper (C14b): dropped the never-sent antenna_id (spec follows the
  handler; adding it back later is non-breaking)
- PhysicalParametersInfo: f_over_d -> f_over_d_ratio, +focal_length_m, dropped the
  spurious nested feed object, MeshInfo extracted as a named component
- ValidityRangesInfo: all four fields renamed to their emitted names
- CalibrationInfo: date/version/source renamed, +r_squared, dropped parameters_tuned,
  rmse_db/r_squared documented as omitted per C12

No Rust changed; no computed value moved."
```

---

## Task 6: Refresh `/h3-heatmap` docs for stages 1–3 and close the unit

**Goal:** Bring `docs/api-documentation.md` and `openapi.yaml`'s `/h3-heatmap` entry up to date
with what stages 1–3 changed, then record stage 4 and its absorbed units as done across the
contract and roadmap docs.

**Files:**
- Modify: `docs/api-documentation.md` — the `/h3-heatmap` sections (`:236-261`, `:325-380`) and
  the antenna/feed response field names
- Modify: `openapi.yaml:734-925` (`/api/v1/h3-heatmap` path entry) — verify only; fix any
  stage-1/2/3 drift found
- Modify: `docs/domain-contract.md` — record the H3-stub removal and the C12 shape call
- Modify: `docs/roadmap-2026-07.md` (Phase 3 row, register rows C5/C8/C12/C14)
- Modify: `docs/roadmap-2026-07-work-units.md` (C8 stage 4, C12, C14 marked done; the D2/D9
  follow-up from decision §3.1)
- Modify: `CLAUDE.md` — the `/heatmap` H3 stub is described as live

**Acceptance Criteria:**
- [ ] Every request/response example under `/h3-heatmap` in `docs/api-documentation.md` uses
      `feed_pointing_location` (stage 1), carries `coordinate_system` on every position (stage
      2), and shows `warnings` as `{code, message}` objects (stage 3).
- [ ] The antenna/feed response examples in `docs/api-documentation.md` use the field names
      Task 5 put in the spec (`id`, `frequency_range_mhz`, `calibration`, …).
- [ ] `CLAUDE.md` no longer describes `/heatmap`'s H3 grid type as a `NotImplemented` stub.
- [ ] `docs/roadmap-2026-07.md`'s Phase 3 row states that C8 stage 4 landed and that only C7
      remains before the freeze.
- [ ] `docs/api-documentation.md` documents the `null`-under-200 shape of
      `CalibrationInfo.rmse_db`/`r_squared` **deliberately** — unit C12 requires this explicitly
      under option 2 ("not by omission") — alongside a one-paragraph statement of the API's two
      no-value conventions (`null` = slot exists / omitted = structurally absent).
- [ ] `docs/roadmap-2026-07-work-units.md` records the decision from §3.1 as a note on units D2
      and D9 (the NaN sentinel stays in the postcard `CalibrationMetadata`; making it
      `Option<f64>` is an ANTC format break that belongs to D2).
- [ ] No numeric assertion anywhere in the workspace changed.

**Verify:** `cargo test --test doc_examples_deserialize` → PASS (the C11 guard checks the marked
blocks in `api-documentation.md` against the live schemas).

**Steps:**

- [ ] **Step 1: Audit the `/h3-heatmap` docs against the current schemas**

```bash
grep -n 'feed_position\|"warnings"' docs/api-documentation.md
grep -n 'coordinate_system' docs/api-documentation.md | wc -l
```

Every position object in every example must carry `coordinate_system`; there must be zero hits
for `feed_position`. Note that C1 documented this endpoint on 2026-07-25, *before* stages 1–3,
so drift is expected — fix what the greps surface.

- [ ] **Step 2: Fix the antenna/feed response examples**

Update the `GET /api/v1/antennas*` examples in `docs/api-documentation.md` to the field names
Task 5 established. In particular the feed example near `:506` uses `frequency_range_mhz`
already — verify the surrounding object uses `id` (not `feed_id`) as the feed's own key, and
that any `calibration_info` key becomes `calibration`.

- [ ] **Step 3: Verify openapi's `/h3-heatmap` path entry**

Read `openapi.yaml:734-925` and confirm the request example uses `feed_pointing_location`,
every position carries `coordinate_system`, and the response's `warnings` reference
`ApiWarning`. Fix any drift found. (Spot-check: `:743` already says `feed_pointing_location`,
so stage 1 was mirrored; confirm stages 2 and 3 were too.)

- [ ] **Step 4: Update `docs/domain-contract.md`**

Add to the resolved-by-design / changelog section, in the style of the stage-2 entry:

```markdown
**Resolved by design 2026-07-28 (C8 stage 4).** `/api/v1/heatmap` no longer accepts an
`h3` grid type. It was a stub that parsed and validated, then returned
`not_implemented`; the real H3 grid has always been the separate
`POST /api/v1/h3-heatmap`. An `h3` grid_type is now an unknown serde variant, i.e. a
400. `GridConfig`/`GridData` stay single-variant tagged enums so `grid_type` remains on
the wire for feature F5.

**Resolved by design 2026-07-28 (C12).** `CalibrationInfo.rmse_db` / `r_squared` are
**present and `null`** for uncalibrated antennas — never omitted. This settles the API's
two no-value conventions, which had been used interchangeably:

- **`null` = the slot exists, the value does not.** Every antenna has a calibration
  block; an uncalibrated one simply has no fit. Declared with
  `#[serde(with = "nan_as_null")]`, the same mechanism `GainResponse.gain_db` uses for a
  failed evaluation.
- **Omitted = structurally absent.** `PhysicalParametersInfo.mesh` on a solid reflector,
  `CalibrationStatusInfo.coverage` on an antenna with no measured region. There is no
  slot to fill.

The `f64::NAN` sentinel stays in `CalibrationMetadata`, which is postcard-serialized and
so cannot hold an `Option` without an artifact format bump (see D2); the API type now has
the same shape, so there is no boundary conversion. Do **not** reintroduce
`Option<f64>` + `skip_serializing_if` here: that combination was the original defect — it
promised an omission the code never performed.
```

- [ ] **Step 5: Re-true `CLAUDE.md`**

Find and fix the description of `/heatmap`'s H3 grid type as a `NotImplemented` stub (in the
"API / service layer" or endpoint-summary area). Replace with a statement that `/heatmap` serves
rectangular grids only and `/h3-heatmap` is the H3 endpoint, with F5 tracking a merge.

- [ ] **Step 6: Update the roadmap documents**

In `docs/roadmap-2026-07.md`:
- Phase 3 table row: mark C8 stage 4 landed (date, branch), and state that **C7 is the only
  remaining Phase 3 unit**.
- Register rows C5, C8, C12, C14: mark done/absorbed with the date.

In `docs/roadmap-2026-07-work-units.md`:
- C8 stage 4: add a `✅ DONE 2026-07-28` header and an "as landed" note in the style of stages
  1–3, including the six decisions from §3 and the finding from §2 (all seven antenna schemas
  were wrong, not just the two C14 filed).
- C12 and C14: mark done, cross-referencing stage 4.
- **D2** and **D9**: add a note recording decision §3.1 — `CalibrationMetadata.rmse_db` /
  `r_squared` remain `f64` with a NaN sentinel; converting them to `Option<f64>` is an ANTC wire
  break and belongs with the version-axes work, not the contract pass.
- **C15**: note that stage 4 edited `examples/api_requests.json` (removing two H3 examples),
  which remains unguarded — this is the last content change before C7's freeze.

- [ ] **Step 7: Verify**

```bash
cargo test --test doc_examples_deserialize
cargo test --workspace
```
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add docs/ CLAUDE.md openapi.yaml
git commit -m "docs(C8 stage 4): refresh /h3-heatmap docs for stages 1-3; close the unit

C1 documented /h3-heatmap on 2026-07-25, before the stage 1/2/3 breaks; its examples
are brought up to feed_pointing_location, required coordinate_system and typed
warnings. Antenna/feed examples follow Task 5's corrected schemas.

Records in domain-contract.md: the H3-stub removal and C12's omitted-not-null call.
Roadmap: stage 4 landed, C7 is the only remaining Phase 3 unit; D2/D9 carry the note
that the NaN sentinel stays in the postcard type by decision.

No computed value moved."
```

---

## Task 7: Stage-4 exit gate — route/spec parity and full CI

**Goal:** Prove the stage-4 exit criterion — *"openapi.yaml describes every registered route with
post-C8 schemas; ready for C7"* — and leave the workspace green under the exact checks CI runs.

**Files:**
- No production changes expected. If this task finds a defect, fix it here and note it in the PR.

**Acceptance Criteria:**
- [ ] The set of `(path, method)` pairs in `openapi.yaml` equals the set registered in
      `antenna-model/src/api/routes.rs:106-134` — 11 routes, 11 path entries. (This is the
      assertion C7 will automate; running it by hand now confirms C7 has a correct contract to
      freeze.)
- [ ] `./scripts/check.sh` passes end to end (fmt --check, clippy `--workspace --all-targets
      -D warnings`, full workspace tests, cargo audit).
- [ ] `git diff main --stat` shows **no** change to any file under `antenna-model/src/model/` —
      the physics layer is untouched.
- [ ] Every numeric assertion in the workspace is unchanged:
      `git diff main -- '*.rs' | grep -E '^[-+].*assert' | grep -E '[0-9]+\.[0-9]+'` shows only
      lines whose *field names* changed, never a value.

**Verify:** `./scripts/check.sh` → exits 0.

**Steps:**

- [ ] **Step 1: Enumerate the registered routes**

```bash
grep -n '\.at(' antenna-model/src/api/routes.rs | sed -n '1,20p'
```
Expected: 11 `.at(` calls — `/health`, `/ready`, `/status`, `/api/v1/gain`,
`/api/v1/gain/batch`, `/api/v1/heatmap`, `/api/v1/h3-heatmap`, `/api/v1/antennas`,
`/api/v1/antennas/:id`, `/api/v1/antennas/:id/feeds`, `/api/v1/antennas/:id/feeds/:feed_id`.

- [ ] **Step 2: Enumerate the spec's paths**

```bash
grep -n '^  /' openapi.yaml
```
Expected: the same 11, with poem's `:id` written as OpenAPI's `{id}`.

- [ ] **Step 3: Diff the two sets**

Compare by hand and record the result in the PR description. Any route without a spec entry, or
any spec entry without a route, is a stage-4 defect — fix it before proceeding.

- [ ] **Step 4: Confirm the physics layer is untouched**

```bash
git diff main --stat -- antenna-model/src/model/
```
Expected: empty output.

- [ ] **Step 5: Confirm no computed value moved**

```bash
git diff main -- '*.rs' | grep -E '^[-+].*assert' | grep -E '[0-9]+\.[0-9]+'
```
Review every line: a numeric literal may appear on both a `-` and a `+` line only when the
surrounding *field name* changed. A literal that changed value is a charter violation — stop and
report.

- [ ] **Step 6: Run the checks exactly as CI does**

```bash
./scripts/check.sh
```
Expected: exits 0. (Use this, not the ad-hoc one-liners — it sets `RUST_MIN_STACK` to match CI.)

- [ ] **Step 7: Commit any fixes and open the PR**

```bash
git push -u origin feat/c8-stage4-endpoint-coherence
gh pr create --title "feat(C8 stage 4): endpoint coherence + spec completeness (C12, C14)" --body "$(cat <<'EOF'
Final stage of the C8 consolidated breaking pass (register row C8, decided 2026-07-08).
Absorbs units C12, C14 and the superseded row C5. **C7 may now freeze the contract.**

## What changed
- **Removed the `/heatmap` H3 grid-type stub** — it parsed, validated, then failed with
  `not_implemented`. An `h3` grid_type is now an unknown serde variant → **400
  invalid_request_body** (C2's policy: 400 = unparseable body). `GridConfig`/`GridData`
  stay single-variant tagged enums so `grid_type` remains on the wire for feature F5.
- **Retired `not_implemented`** — the stub was its only producer. Vocabulary is 10 codes.
  C3's drift guard drove the openapi + api-documentation updates.
- **C12: `rmse_db`/`r_squared` are omitted, not null**, for uncalibrated antennas. Mapped
  at the API boundary, *not* in `CalibrationMetadata` — that type is postcard-serialized
  into the ANTC artifact, where `f64` → `Option<f64>` is a wire break (noted on D2/D9).
- **C14+: every antenna-endpoint schema in openapi.yaml was wrong.** C14 filed two of
  eight defects; all seven components behind `/api/v1/antennas*` are rewritten against
  their Rust types. A client coding to the old spec would not have found a single field
  on `GET /api/v1/antennas`.

## Exit criteria
- 11 registered routes ↔ 11 openapi path entries (verified by hand; C7 automates it).
- `./scripts/check.sh` green.
- **No computed value moved** — no file under `src/model/` changed; every numeric
  assertion in the workspace is unchanged. That property is the review net for the whole
  four-stage pass.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Task 8 (OPTIONAL — unit C15, option 1): guard `examples/api_requests.json`

**Goal:** Extend the existing example-guard pattern to `examples/api_requests.json`, the largest
unguarded client-visible surface, so C7's freeze ratifies a checked file rather than an assumed
one.

**This task is outside the C8/C12/C14 scope the user asked for.** It is included because stage 4
*edits* that file (Task 3 removes two examples from it) and no test would notice if the edit were
wrong — C15 records three separate occasions during stage 1 where this file drifted silently.
**Cut it freely** if stage 4 should stay minimal; C15 remains filed either way.

**Files:**
- Create: `antenna-model/tests/example_api_requests_deserialize.rs`
- Read (pattern to copy): `antenna-model/tests/example_responses_deserialize.rs`
- Modify (only if the new guard finds drift): `examples/api_requests.json`

**Acceptance Criteria:**
- [ ] Every entry under `examples.<name>.request` deserializes into its matching request type;
      every `examples.<name>.response` into its matching response type.
- [ ] An entry whose name maps to no schema **panics** with a message naming the entry — an
      unmapped arm must fail loudly, not silently skip (this is what makes the guard total).
- [ ] The guard fails on injected drift: verified by temporarily renaming one field in the file.
- [ ] No production code changes.

**Verify:** `cargo test --test example_api_requests_deserialize` → PASS.

**Steps:**

- [ ] **Step 1: Read the pattern being copied**

```bash
cat antenna-model/tests/example_responses_deserialize.rs
```
Reuse its module doc style, its round-trip key-presence helper, and its **null-source exemption**
(a `null` in the source cannot be distinguished from an absent optional after deserialization —
the exemption is load-bearing, not a convenience; see that file's module doc and C12).

- [ ] **Step 2: Write the guard**

Create `antenna-model/tests/example_api_requests_deserialize.rs`:

```rust
//! Drift guard for `examples/api_requests.json` (roadmap C15, option 1).
//!
//! This file carries 18 request/response examples and, until now, no test read it.
//! C8 stage 1 found drift in it three times — missing required `failed_points` /
//! `failure_count`, and an undeclared `vehicle_attitude` on two HeatmapRequest bodies —
//! each caught only because a whole-branch review went looking.
//!
//! The file's shape is `{"examples": {"<name>": {"description", "request"|"response"}}}`.
//! Every entry must map to a schema; an unmapped name **panics**, so adding an example
//! without teaching this test about it fails the build rather than silently skipping.

use antenna_model::api::schemas::*;
use serde_json::Value;

/// Deserializes `value` into `T` and asserts every non-null key survived the round trip.
fn check<T>(name: &str, value: &Value)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let parsed: T = serde_json::from_value(value.clone())
        .unwrap_or_else(|e| panic!("example `{name}` does not deserialize: {e}"));
    let reserialized =
        serde_json::to_value(&parsed).unwrap_or_else(|e| panic!("example `{name}`: {e}"));
    assert_keys_survived(name, value, &reserialized);
}

/// Every non-null key in `source` must still be present in `round_tripped`.
/// Null sources are exempt: after deserialization into an `Option`, "absent" and
/// "present-as-null" are the same state.
fn assert_keys_survived(path: &str, source: &Value, round_tripped: &Value) {
    let (Value::Object(src), Value::Object(out)) = (source, round_tripped) else {
        return;
    };
    for (key, src_val) in src {
        let child = format!("{path}.{key}");
        match out.get(key) {
            None if src_val.is_null() => {}
            None => panic!(
                "{child} is present in the example with a non-null value but is absent \
                 after a round trip — serde silently dropped it (unknown field?)"
            ),
            Some(out_val) => assert_keys_survived(&child, src_val, out_val),
        }
    }
}

#[test]
fn every_api_request_example_matches_a_live_schema() {
    let raw = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/api_requests.json"),
    )
    .expect("examples/api_requests.json is readable");
    let doc: Value = serde_json::from_str(&raw).expect("examples/api_requests.json is valid JSON");

    let examples = doc
        .get("examples")
        .and_then(Value::as_object)
        .expect("api_requests.json has an `examples` object");

    assert!(!examples.is_empty(), "no examples found — the guard would vacuously pass");

    for (name, entry) in examples {
        let body = entry
            .get("request")
            .or_else(|| entry.get("response"))
            .unwrap_or_else(|| panic!("example `{name}` has neither `request` nor `response`"));

        match name.as_str() {
            n if n.starts_with("gain_request") => check::<GainRequest>(n, body),
            n if n.starts_with("gain_response") => check::<GainResponse>(n, body),
            n if n.starts_with("batch_request") => check::<BatchGainRequest>(n, body),
            n if n.starts_with("batch_response") => check::<BatchGainResponse>(n, body),
            n if n.starts_with("heatmap_request") => check::<HeatmapRequest>(n, body),
            n if n.starts_with("heatmap_response") => check::<HeatmapResponse>(n, body),
            n if n.starts_with("h3_link_budget_request") => check::<H3LinkBudgetRequest>(n, body),
            n if n.starts_with("h3_link_budget_response") => check::<H3LinkBudgetResponse>(n, body),
            n if n.starts_with("error") => check::<ErrorResponse>(n, body),
            other => panic!(
                "example `{other}` maps to no schema. Add an arm above — a silent skip \
                 is how this file drifted three times during C8 stage 1."
            ),
        }
    }
}
```

**Adjust the match arms to the names actually in the file** — run
`python3 -c "import json;print(sorted(json.load(open('examples/api_requests.json'))['examples']))"`
first and write arms that cover exactly those names. Import paths may need adjusting to whatever
`antenna_model::api::schemas` actually re-exports.

- [ ] **Step 3: Run it**

```bash
cargo test --test example_api_requests_deserialize
```
Expected: PASS. If it FAILS, the file has real drift — fix `examples/api_requests.json`, and
record what was found in the PR description (that finding is the guard's justification).

- [ ] **Step 4: Prove it catches drift**

Temporarily rename one field in one example (e.g. `frequency_mhz` → `frequency_mhZ`), re-run, and
confirm it FAILS with a message naming the example. Revert.

- [ ] **Step 5: Commit**

```bash
git add antenna-model/tests/example_api_requests_deserialize.rs examples/api_requests.json
git commit -m "test(C15): drift guard for examples/api_requests.json

18 client-visible examples with no test reading them; C8 stage 1 found drift in this
file three times, each caught only by a human reviewer. Follows the
example_responses_deserialize.rs pattern, including its null-source exemption. An
unmapped example name panics rather than skipping, so the guard is total.

Verified against injected drift before acceptance."
```

---

## Self-review

**Spec coverage.** The unit's four bullets map to: bullet 1 → Tasks 1, 2, 3; bullet 2 → Task 6;
bullet 3 → decision §3.6 + Task 3 Step 6; exit criterion → Tasks 5 and 7. C12 → Task 4. C14(a)
→ Task 5 Step 3; C14(b) → Task 5 Step 4. No bullet is unassigned.

**Known scope expansion, deliberate.** Task 5 covers five component schemas C14 never mentioned.
The stage-4 exit criterion demands it, and skipping it would have C7 freeze a spec on which no
antenna-endpoint field name is correct.

**Deferred, not done here.**
- **C13** (`design_feed_offset_m` vertex- vs focus-relative origin) — moves a computed value,
  which C8's charter forbids; it is latent behind D9 and must land with it.
- **C15** options 2 and 3 (ratcheting `CONTRACT_DOCS`; validating examples against openapi
  component schemas) — option 3 is C7's stretch goal; option 1 is Task 8 here, optional.
- **D2/D9**: making `CalibrationMetadata.rmse_db`/`r_squared` genuinely optional in the artifact.
  Under the adopted option 2 this is no longer even a compromise — the `f64` + NaN sentinel now
  matches the API type exactly — but if D2 ever revisits the ANTC format, decision §3.1 records
  the constraint.

**Type consistency.** `nan_as_null` is the existing module at `schemas.rs:44-65`, referenced by
Task 4 (the two `CalibrationInfo` fields) and by Task 5's `nullable: true` declarations — the
Rust attribute and the spec keyword must both be present or the pair drifts.
`GridConfig::Rectangular` / `GridData::Rectangular` are the only variant names used after Task 1.
The openapi component names introduced in Tasks 3 and 5 — `RangeConfig`, `MeshInfo` — are both
`$ref`'d and defined.
