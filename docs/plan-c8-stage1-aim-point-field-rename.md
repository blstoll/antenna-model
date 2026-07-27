# Implementation Plan — C8 Stage 1: rename the aim-point and feed-offset fields

> **For agentic workers:** REQUIRED SUB-SKILL: use `subagent-driven-development` (recommended)
> or `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Unit:** C8 stage 1 of 4 (`docs/roadmap-2026-07-work-units.md:1691`, register row C8 in
`docs/roadmap-2026-07.md:236`)
**Effort:** M (one session)
**Decision status:** **Decided 2026-07-08** — pre-production confirmed, no consumers exist;
break once now, freeze behind C7. No decision step remains; this is execution only.

**Goal:** Rename the API's feed *aim-point* request field from `feed_position` to
`feed_pointing_location`, and rename the two *physical*-offset response fields so neither can
be confused with it — a clean break with no serde aliases and no computed value changed.

**Architecture:** A pure rename pass across three layers — Rust schema + call sites, the
published contract (`openapi.yaml`, `docs/api-documentation.md`, `examples/`), and the domain
contract. The renames are mechanically derivable by grep; the judgment lives in (a) which
`feed_position` occurrences are the API field versus a genuinely physical feed position that
must **not** be renamed, and (b) the new names for the two response fields. A drift guard for
`examples/responses/` is added *before* the response rename, on the same reasoning that put C11
before C8: a guard added first catches the pass's misses, one added after only ratifies them.

**Tech Stack:** Rust (poem, serde), YAML (`openapi.yaml`), Markdown docs, JSON examples.

**User decisions (already made):**
- C8 = one consolidated breaking pass, executed as four sequential stages, one PR each
  (roadmap §5 row C8, maintainer 2026-07-08).
- Stage 1 renames `feed_position` → `feed_pointing_location`. **No serde aliases, no
  deprecation shims** (work-unit stage 1).
- C8 must not alter any computed value — existing numeric assertions may change *field names*,
  never *values* (work-unit "Out of scope").
- Contract and code change together: `docs/domain-contract.md` is updated in the same commit as
  the field it describes (standing contract rule).

---

## 0. Why C8 is the next unit (confirmation)

The Phase 3 dependency chain is `C3 → C4 → C2 → C9 → C8 → C7`
(`roadmap-2026-07-work-units.md:44`). Verified against `git log`:

| Unit | Status | Evidence |
|---|---|---|
| C3, C4, C2, C1, C10, C11 | done | `aef5206` (#17) |
| C9 | done | `0c8bcb2` (#18) |
| **C8** | **next** | all four predecessors landed; C7 explicitly depends on C8 |

Nothing else in Phase 3 is unblocked, and C7 (the drift guard that freezes the contract) is
gated on C8 by definition. C5 and C6 are marked *superseded by C8* and must not be implemented
standalone. Phases 0–2 are closed; Phase 4 (D-units) sits behind "Phases 1–3 done".

**One open item is deliberately not a blocker:** P10-perf (wide-angle Ka latency) is still open
but is a Phase 1 fast-follow, not a phase-exit criterion, and it touches no API shape.

---

## 1. What stage 1 requires

From the work unit, verbatim:

> **Stage 1 — Rename the aim-point fields.**
> - `feed_position` → `feed_pointing_location` on all three request types (fields at
>   `schemas.rs:247,432,590`). Review the two *physical*-offset response fields
>   (`GeometryInfo.feed_offset_meters`, `FeedInfo.position_offset`) and align them to one
>   naming scheme that cannot be confused with the aim point (e.g. `physical_feed_offset_m`);
>   keep units in the name or the docs, consistently.
> - **No serde aliases, no deprecation shims** — clean break.
> - Update `docs/domain-contract.md`'s parameter-glossary entry **in the same commit**.
> - Exit: grep for `feed_position` finds zero hits outside historical docs
>   (`review-findings-*.md`, superpowers plans) and the contract's changelog note.

**Line-number drift (re-verified 2026-07-26):** the request fields are now at
`schemas.rs:247` (`GainRequest`), **`:440`** (`HeatmapRequest`), **`:605`**
(`H3LinkBudgetRequest`) — the unit's `432`/`590` are stale. `GeometryInfo.feed_offset_meters`
is `schemas.rs:329`; `FeedInfo.position_offset` is `schemas.rs:798`. Re-verify before editing
(standing rule: if a cited line no longer matches its description, stop and re-locate).

---

## 2. Current state (verified 2026-07-26 at `0c8bcb2`)

### 2.1 The API field (rename target — "class A")

`feed_position: Position3D` appears on three request structs, each with the same doc comment:

```rust
    /// Feed pointing target (ECEF or Geodetic).
    ///
    /// **This is the Earth location the feed's beam is aimed at — NOT the
    /// feed's physical location on the antenna.** The service converts the
    /// angular offset between this aim point and `reflector_boresight` into a
    /// physical feed displacement in the antenna frame (including the beam
    /// deviation factor). To model an unsteered (focused) feed, set this equal
    /// to `reflector_boresight`.
    pub feed_position: Position3D,
```

The field is required (no `Option`, no `#[serde(default)]`), so after the rename a body using
the old name fails deserialization with `missing field feed_pointing_location` → **400** under
the C2 policy ("400 = the body could not be parsed. Nothing else.",
`tests/integration/status_code_matrix_tests.rs:6`). That is the intended clean-break behavior
and Task 1 pins it.

Consumers of `request.feed_position` in `src/`: `service/validator.rs:89,192,223,548`,
`service/evaluator.rs:159`, `service/h3_link_budget.rs:95,361`, `service/heatmap.rs:331`, plus
doc comments in `api/handlers.rs:168,190,314,422,451,922`.

### 2.2 Occurrences that must **NOT** be renamed ("class B")

These name a genuinely *physical* feed position and are unrelated to the API aim point.
Renaming them would be a semantic error, not a cleanup:

| Symbol | File | Meaning |
|---|---|---|
| `EClockConeCoordinates::to_feed_position` / `to_feed_position_with_bdf` / `from_feed_position` | `model/coordinates.rs:250,264,279` | E-cone/E-clock ⇄ physical Cartesian feed position in the antenna frame |
| `get_feed_position` | `model/ray_trace.rs:249` | physical feed position from the antenna config |
| `param: "feed_position"` | `model/geometry.rs:318` | validation error naming the **physical** `FeedPosition`, not an API field |
| `feed_position_m` | `calibrate/src/artifact_export.rs:236,288,546`, `calibrate/src/main.rs:696` | physical feed position stored in a calibration artifact |
| local `let feed_position = FeedPosition::new(...)` | `service/evaluator.rs:174`, `service/h3_link_budget.rs:107` | the *computed* physical position; rename the local to `physical_feed_position` for clarity, but it is not an API field |
| test names `test_feed_position_at_focus`, `test_feed_position_displacement`, `test_feed_position_on_axis`, `test_feed_position_offset`, `test_e_clock_cone_feed_position` | `geometry.rs:753,762`, `ray_trace.rs:360,370`, `coordinates.rs:399` | physical-position tests |

`compute_feed_position_from_pointing` (`model/coordinates_3d.rs:613`) is the **bridge**: it
takes the aim point and returns a physical position. Its name is accurate — keep it. Its
doc comment at `:596` references the API field by name and must be updated to the new name.

### 2.3 The two response fields

Both are physical offsets from the focal point, in meters, but they are **different
quantities**:

- `GeometryInfo.feed_offset_meters` (`schemas.rs:329`) — filled at `evaluator.rs:413` from
  `Vector3D::new(feed_x, feed_y, feed_z - focal_length_m)` where `feed_* = steering
  displacement + design offset` (`evaluator.rs:158-174`). It is the **total, per-request**
  offset actually used.
- `FeedInfo.position_offset` (`schemas.rs:798`) — filled at `handlers.rs:677,803,865` from
  `cal.physical_config.feed.position`, the antenna's **static design** offset from
  `calibration_data/design_specs/*.yaml` (e.g. `position: [0.05, 0.0, 0.0]`). Independent of
  any request.

`FeedInfo.position_offset` is also absent from `openapi.yaml` entirely (grep: zero hits) —
noted, not fixed here; spec completeness for the antenna endpoints is C8 stage 4 / C7.

### 2.4 Drift guards that already exist (these are the safety net)

| Guard | Covers | Effect on this pass |
|---|---|---|
| `tests/example_requests_deserialize.rs` (G3) | every `examples/requests/*.json` → its request type; unmapped file ⇒ panic | fails until all 10 request examples are renamed |
| `tests/doc_examples_deserialize.rs` (C11) | blocks in `docs/api-documentation.md` marked `<!-- api-example: TypeName -->`; unmarked API-looking block ⇒ panic | fails until the marked prose examples are renamed |
| `tests/error_code_vocabulary.rs` (C3) | error-code enum in spec + docs | unaffected |
| `scripts/check.sh` | `fmt --check`, `clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, `cargo audit` | `--all-targets` compiles `benches/`, so bench call sites must be renamed too |

**Not guarded:** `examples/responses/*.json`, `examples/api_requests.json`,
`examples/postman_collection.json`, `examples/python_examples.py`, `examples/QUICKSTART.md`,
`examples/TESTING.md`, `openapi.yaml`, `docs/architecture.md`. Task 2 closes the
`examples/responses/` half of that gap; the rest is manual and listed explicitly in Task 4.

---

## 3. Naming decision for the response fields (recorded)

The unit delegates the choice ("align them to one naming scheme … e.g. `physical_feed_offset_m`").
**Chosen scheme — one convention, two names, because they are two quantities:**

| Old | New | Why |
|---|---|---|
| `GeometryInfo.feed_offset_meters` | `physical_feed_offset_m` | The per-request total. `physical_` is the word that distinguishes it from the aim point; `_m` matches the `_m` unit suffix already used across the config/data layer (`phase_center_offset_m`, `axial_defocus_m`, `surface_rms_mm`). |
| `FeedInfo.position_offset` | `design_feed_offset_m` | The static, per-antenna design offset. Naming both `physical_feed_offset_m` would assert they are the same number; they are not (the request one adds beam-steering displacement). |

Both doc comments state the relationship explicitly, so a reader of either field learns the
other exists. Rejected: a single shared name (conflates two quantities); keeping
`_meters` (inconsistent with the `_m` suffix used everywhere else).

---

## 4. Task list

Order matters: Task 2 lands the response-example guard **before** Task 3 renames response
fields.

### Task 1: Rename the request field to `feed_pointing_location`

**Goal:** `feed_position` no longer exists as an API request field anywhere in the Rust
crates, the request examples, or the guarded prose examples; a body using the old name is a
400 naming the new field.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:247,440,605` (the three field definitions) and the
  test fixtures at `:1354,1381,1611,1626,1953`
- Modify: `antenna-model/src/service/validator.rs` (11 hits), `service/evaluator.rs` (20),
  `service/h3_link_budget.rs` (18), `service/heatmap.rs` (6), `service/batch.rs` (1),
  `service/cache.rs` (1), `service/test_support.rs` (1)
- Modify: `antenna-model/src/api/handlers.rs` (6 doc-comment hits)
- Modify: `antenna-model/src/model/coordinates_3d.rs:596` (doc comment naming the API field)
- Modify: `antenna-model/benches/heatmap_benchmarks.rs`, `benches/computation_modes.rs`
- Modify: `antenna-model/tests/feed_steering_test.rs` (5), `tests/beam_steering_direction.rs` (5),
  `tests/integration/{partial_calibration_tests.rs (6), helpers.rs (3),
  h3_link_budget_tests.rs (3), ray_trace_stub_warning_tests.rs (3),
  status_code_matrix_tests.rs (1), off_axis_warning_tests.rs (1), timeout_tests.rs (1)}`
- Rename: `antenna-model/tests/feed_position_is_pointing_target.rs` →
  `antenna-model/tests/feed_pointing_location_is_an_aim_point.rs` (9 hits inside)
- Modify: all 10 files in `examples/requests/`
- Modify: `docs/api-documentation.md:225,255,298,322,419` (guarded by C11's test)
- Test: `antenna-model/tests/integration/status_code_matrix_tests.rs` (new case)

**Mechanical scope is in-scope.** The call sites above were derived by
`grep -rn 'feed_position'` across the workspace. Fixing them to compile is part of this task —
do not stop to ask for scope approval. STOP and report only on genuine semantic ambiguity:
specifically, if you find a `feed_position` occurrence that is **not** in §2.2's class-B table
and **not** the API field.

**Acceptance Criteria:**
- [ ] `GainRequest`, `HeatmapRequest`, `H3LinkBudgetRequest` expose `feed_pointing_location`;
      the field carries no `#[serde(alias)]` and no `#[serde(rename)]`.
- [ ] A `/api/v1/gain` body using the old key `feed_position` returns **400** with a message
      naming `feed_pointing_location`.
- [ ] Every symbol in §2.2's class-B table is unchanged (spot-check by grep).
- [ ] No computed value changes: every existing numeric assertion in the workspace passes
      untouched except for field renames.

**Verify:** `RUST_MIN_STACK=16777216 cargo test --workspace` → all green, including
`every_example_request_deserializes` and `every_documented_example_deserializes`.

**Steps:**

- [ ] **Step 1: Write the failing clean-break test.**

Append to `antenna-model/tests/integration/status_code_matrix_tests.rs` (match the file's
existing helper style — read the top of the file first and reuse its request-posting helper
rather than inventing one):

```rust
/// C8 stage 1: the aim-point field was renamed `feed_position` →
/// `feed_pointing_location` as a **clean break** — no serde alias. A body using the
/// old key is therefore missing a required field, i.e. unparseable, i.e. 400 under
/// C2's policy. Pinned so a well-meaning future change cannot quietly reintroduce
/// backwards compatibility that the C8 decision deliberately rejected.
#[tokio::test]
async fn legacy_feed_position_key_is_rejected_with_400() {
    let body = serde_json::json!({
        "antenna_id": "gs_3.7m_uncalibrated",
        "feed_id": "s_band_feed",
        "vehicle_position": {"x": -118.1234, "y": 34.5678, "z": 100.0},
        "reflector_boresight": {"x": -118.1234, "y": 34.5679, "z": 110.0},
        "feed_position": {"x": -118.124, "y": 34.568, "z": 105.0},
        "emitter_position": {"x": -117.0, "y": 35.0, "z": 400000.0},
        "frequency_mhz": 2200.0
    });

    let (status, payload) = post_json("/api/v1/gain", &body).await;

    assert_eq!(status, 400, "old key must not be silently accepted");
    let text = payload.to_string();
    assert!(
        text.contains("feed_pointing_location"),
        "the 400 must name the field the client should send, got: {text}"
    );
}
```

If the file's helper is not named `post_json`, adapt the call — do not add a second helper.

- [ ] **Step 2: Run it and watch it fail for the right reason.**

```bash
RUST_MIN_STACK=16777216 cargo test -p antenna-model --test integration \
  legacy_feed_position_key_is_rejected_with_400 -- --nocapture
```

Expected: **FAIL** — the request succeeds (200) because `feed_position` is still the live
field name. A failure with status 400 for some *other* reason (e.g. unknown antenna) means
the fixture is wrong; fix the fixture before proceeding.

- [ ] **Step 3: Rename the three field definitions and their doc comments.**

In `antenna-model/src/api/schemas.rs`, for each of the three request structs, the doc comment
body stays as-is; only the declaration line changes:

```rust
    /// Feed pointing target (ECEF or Geodetic).
    ///
    /// **This is the Earth location the feed's beam is aimed at — NOT the
    /// feed's physical location on the antenna.** The service converts the
    /// angular offset between this aim point and `reflector_boresight` into a
    /// physical feed displacement in the antenna frame (including the beam
    /// deviation factor). To model an unsteered (focused) feed, set this equal
    /// to `reflector_boresight`.
    pub feed_pointing_location: Position3D,
```

Do **not** add a cross-reference to the response field here — it is still called
`feed_offset_meters` until Task 3, which adds that sentence once the new name exists.

- [ ] **Step 4: Fix the Rust call sites.**

Compiler-driven. Start from:

```bash
cargo check --workspace --all-targets 2>&1 | grep -E '^(error|  -->)' | head -50
```

Two things the compiler will **not** catch, so grep for them explicitly:

```bash
grep -rn 'feed_position' antenna-model/src antenna-model/tests antenna-model/benches
```

1. String literals — `validator.rs:89,192,223,548` pass `"feed_position"` as the field-name
   argument to `validate_position` / `warn_if_ambiguous`; that string is what a client sees in
   the error body. It must become `"feed_pointing_location"`.
   `validator.rs:1097` asserts on that string.
2. Doc comments in `api/handlers.rs` and `model/coordinates_3d.rs:596`.

Rename the local bindings `let feed_position = FeedPosition::new(...)`
(`evaluator.rs:174`, `h3_link_budget.rs:107`) to `physical_feed_position` — they hold the
computed physical position, and leaving the old name there is exactly the confusion this
stage removes.

Leave everything in §2.2's class-B table alone.

- [ ] **Step 5: Rename the pointing-target regression test file.**

```bash
git mv antenna-model/tests/feed_position_is_pointing_target.rs \
       antenna-model/tests/feed_pointing_location_is_an_aim_point.rs
```

Inside, update the module doc (it opens "the API `feed_position` is the feed's *pointing*
location") and rename the test fn
`feed_position_resolves_relative_to_vehicle_not_absolute` →
`feed_pointing_location_resolves_relative_to_vehicle_not_absolute`. The body calls
`compute_feed_position_from_pointing`, which is class B — **do not rename it**.

- [ ] **Step 6: Update the 10 request examples.**

```bash
sed -i '' 's/"feed_position"/"feed_pointing_location"/g' examples/requests/*.json
git diff --stat examples/requests/
```

Expected: 10 files changed. G3's test is the check.

- [ ] **Step 7: Update the guarded prose examples.**

`docs/api-documentation.md` lines 225, 255, 298, 322, 419 — lines 298/322/419 are inside
`<!-- api-example: … -->` blocks (guarded); 225 and 255 are prose sentences that name the
field and must read naturally, not be sed'd:

- `:225` — "`feed_position` — the Earth location the beam is *aimed at*, not the feed's
  physical…" → replace the identifier only.
- `:255` — "…`feed_position`, and if the beam peak falls outside the rings you requested…" →
  replace the identifier only.

- [ ] **Step 8: Update the domain-contract glossary row — same commit, per the unit.**

The unit requires `docs/domain-contract.md`'s parameter-glossary entry to change **in the same
commit** as the field. Edit row `:72` now:

- Retitle it `GainRequest.feed_pointing_location` / `HeatmapRequest.feed_pointing_location` /
  `H3LinkBudgetRequest.feed_pointing_location`.
- Replace the stale sentence *"The API field doc comment is still bare ("Feed position (ECEF or
  Geodetic)", `schemas.rs:239`); field occurs at `schemas.rs:240,417,568`"* — the doc comment
  has not been bare since the 2026-07-02 pass — with the current locations
  `schemas.rs:247,440,605`.
- Replace the closing *"Consider renaming to `feed_pointing_location` in a future major version
  — flagged, not fixed (breaking API change)"* with:

```markdown
      **Renamed 2026-07-26 (C8 stage 1, breaking):** this field was `feed_position` until the
      v1 contract-finalization pass. The old name was THE anchor bug — it reads as the feed's
      physical location. Clean break, no serde alias: a body using `feed_position` is a 400
      naming `feed_pointing_location` (pinned by
      `tests/integration/status_code_matrix_tests.rs::legacy_feed_position_key_is_rejected_with_400`).
```

Also update invariant row `:94` to point at the renamed test file
`feed_pointing_location_is_an_aim_point.rs`, and any *API-field* reference in rows `:61,62` and
the prose at `:437-438` — but leave the function names `compute_feed_position_from_pointing` and
`EClockConeCoordinates::to_feed_position` alone (class B).

Task 3 appends the response-field cross-reference to this same row once those names exist.

- [ ] **Step 9: Run the full gate.**

```bash
./scripts/check.sh
```

Expected: green. If `every_documented_example_deserializes` or
`every_example_request_deserializes` fails, it is naming a file you missed — fix that file,
do not touch the test.

- [ ] **Step 10: Commit.**

```bash
git add -A
git commit -m "feat(C8 stage 1): rename request field feed_position -> feed_pointing_location

Clean break, no serde alias: the field names an Earth aim point, not the feed's
physical location (docs/domain-contract.md, THE anchor bug). A body using the old
key is now a 400 naming the new field, pinned by
legacy_feed_position_key_is_rejected_with_400.

Physical feed positions (EClockConeCoordinates::to_feed_position,
ray_trace::get_feed_position, artifact_export::feed_position_m,
compute_feed_position_from_pointing) are deliberately unchanged - they name a
different quantity.

No computed value changed.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Add a drift guard for `examples/responses/`

**Goal:** Every file in `examples/responses/` is deserialized into its response type by a test,
so Task 3's response-field rename cannot silently strand a stale example — the same reasoning
that put C11 before C8.

**Files:**
- Create: `antenna-model/tests/example_responses_deserialize.rs`
- Reference (do not modify): `antenna-model/tests/example_requests_deserialize.rs` — copy its
  shape exactly, including the panicking unmapped-file arm

**Acceptance Criteria:**
- [ ] All 8 files in `examples/responses/` are mapped to a response type and deserialize.
- [ ] A new unmapped `.json` file in that directory fails the test (the G3 pattern).
- [ ] The test is in the `scripts/check.sh` gate (automatic — it is a workspace test).

**Verify:** `cargo test -p antenna-model --test example_responses_deserialize` → PASS,
`checked >= 8`.

**Risk, and what to do about it:** these examples are not currently guarded and may have
pre-existing drift unrelated to C8 (`gain_response.json` in particular carries an absurd
`gain_db: -3370985.117…` from a pre-P1 era — a *value* problem, not a shape problem, so it
will still deserialize). If a file fails to parse for a reason unrelated to this rename:
**do not edit the response schema to accommodate it** (standing rule 5 — never fix code to
match a doc). Fix the example if the fix is obvious and mechanical; otherwise leave the file
out of the map with a one-line comment naming the follow-up (`D5`, docs truthfulness) and say
so in the PR description.

**Steps:**

- [ ] **Step 1: Write the test.**

Create `antenna-model/tests/example_responses_deserialize.rs`:

```rust
//! Guards that every example response in `examples/responses/` deserializes into
//! its documented schema type — the response-side sibling of G3's
//! `example_requests_deserialize.rs`. Added ahead of C8 stage 1's response-field
//! renames so the rename pass has a net, rather than being ratified by a guard
//! written afterwards (roadmap principle 2: guardrails first).

use antenna_model::api::schemas::{
    AntennaDetailsResponse, AntennaListResponse, BatchGainResponse, ErrorResponse, GainResponse,
    HealthResponse, HeatmapResponse, StatusResponse,
};
use std::path::Path;

fn assert_parses<T: serde::de::DeserializeOwned>(path: &Path) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    if let Err(e) = serde_json::from_str::<T>(&text) {
        panic!(
            "{} did not deserialize into {}: {e}",
            path.display(),
            std::any::type_name::<T>()
        );
    }
}

#[test]
fn every_example_response_deserializes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/responses");
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&dir).expect("examples/responses must exist") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        match name.as_str() {
            "gain_response.json" => assert_parses::<GainResponse>(&path),
            "batch_response.json" => assert_parses::<BatchGainResponse>(&path),
            "heatmap_response.json" => assert_parses::<HeatmapResponse>(&path),
            "antenna_list_response.json" => assert_parses::<AntennaListResponse>(&path),
            "antenna_details_response.json" => assert_parses::<AntennaDetailsResponse>(&path),
            "health_response.json" => assert_parses::<HealthResponse>(&path),
            "status_response.json" => assert_parses::<StatusResponse>(&path),
            "error_response.json" => assert_parses::<ErrorResponse>(&path),
            other => panic!(
                "no schema mapping for examples/responses/{other} — \
                 add it to every_example_response_deserializes"
            ),
        }
        checked += 1;
    }

    assert!(
        checked >= 8,
        "expected to check all example responses, only saw {checked}"
    );
}
```

- [ ] **Step 2: Run it.**

```bash
RUST_MIN_STACK=16777216 cargo test -p antenna-model --test example_responses_deserialize -- --nocapture
```

Expected: PASS. If a file fails, read the error — it names the offending field. Apply the
risk policy above.

- [ ] **Step 3: Prove the guard is load-bearing.**

```bash
echo '{"nonsense": true}' > examples/responses/scratch_probe.json
cargo test -p antenna-model --test example_responses_deserialize 2>&1 | tail -5
rm examples/responses/scratch_probe.json
```

Expected: FAIL with `no schema mapping for examples/responses/scratch_probe.json`. A guard
that has not been observed failing is not a guard.

- [ ] **Step 4: Commit.**

```bash
git add antenna-model/tests/example_responses_deserialize.rs
git commit -m "test(C8 stage 1): guard examples/responses against schema drift

Response-side sibling of G3's request guard, landed BEFORE the stage-1 response
field renames so it catches their misses instead of ratifying them. Unmapped-file
arm panics; verified failing on an injected file.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Rename the two physical-offset response fields

**Goal:** `GeometryInfo.feed_offset_meters` → `physical_feed_offset_m` and
`FeedInfo.position_offset` → `design_feed_offset_m`, with doc comments that state how the two
relate and that neither is the aim point.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:329` (`GeometryInfo`), `:798` (`FeedInfo`), and the
  test fixtures at `:1877,1918`
- Modify: `antenna-model/src/service/evaluator.rs:413` (construction) and the assertions at
  `:1910,1933,1943,1948,1986,1990,1995,2000,2262,2264`
- Modify: `antenna-model/src/service/batch.rs:218`
- Modify: `antenna-model/src/api/handlers.rs:677,771,803,834,844,865` (three `FeedInfo`
  constructions + three doc comments)
- Modify: `antenna-model/src/api/routes.rs:917,931,969` (JSON-path assertions on
  `position_offset`)
- Modify: `examples/responses/gain_response.json`, `batch_response.json`,
  `antenna_details_response.json`
- Modify: `docs/api-documentation.md:457`

**Acceptance Criteria:**
- [ ] `feed_offset_meters` and `position_offset` return zero hits under
      `grep -rn` over `antenna-model/`, `calibrate/`, `examples/` (docs handled in Task 4/5).
- [ ] Doc comments on both fields name the other field, so neither can be read as the aim
      point.
- [ ] `GeometryInfo.physical_feed_offset_m`'s doc keeps the **`positive = away from the
      reflector vertex`** z-sign convention (it matches `phase.rs`'s `delta_z`; see Task 4 for
      the contradicting text in `openapi.yaml`, which is the stale one).
- [ ] No value changes — `evaluator.rs`'s numeric assertions on the offset components pass with
      their existing expected values.

**Verify:** `./scripts/check.sh` → green, including `every_example_response_deserializes`.

**Steps:**

- [ ] **Step 1: Rename `GeometryInfo`'s field with its doc.**

`antenna-model/src/api/schemas.rs:329`:

```rust
    /// Physical feed offset from the focal point in the antenna frame (meters).
    ///
    /// This is the **total** offset actually used for this request: the antenna's
    /// static design offset (reported as `FeedInfo.design_feed_offset_m`) plus the
    /// displacement induced by steering the beam to `feed_pointing_location`.
    /// It is a physical displacement in the antenna frame — *not* an Earth
    /// location, and not to be confused with `feed_pointing_location`.
    ///
    /// `x` and `y` are the lateral displacement of the feed from the optical axis;
    /// `z` is the axial displacement from the focal point (**positive = away from
    /// the reflector vertex**, matching the phase model's `delta_z` convention).
    /// For an on-axis (boresight-aimed) feed all three components are ~zero.
    pub physical_feed_offset_m: Vector3D,
```

- [ ] **Step 2: Rename `FeedInfo`'s field with its doc.**

`antenna-model/src/api/schemas.rs:798`:

```rust
    /// The feed's **design** offset from the focal point in the antenna frame
    /// (meters) — a static property of this antenna's configuration, identical
    /// for every request. The per-request total (design offset + beam-steering
    /// displacement) is reported as `GeometryInfo.physical_feed_offset_m`.
    pub design_feed_offset_m: Vector3D,
```

- [ ] **Step 3: Fix the call sites (compiler-driven) and the JSON-path assertions.**

```bash
cargo check --workspace --all-targets 2>&1 | grep -E '^(error|  -->)' | head -40
grep -rn 'feed_offset_meters\|position_offset' antenna-model/ calibrate/ examples/
```

`api/routes.rs:917,931,969` reach into the response JSON by key
(`json_value.get("position_offset")`) — the compiler will not flag these; they must be updated
by hand to `"design_feed_offset_m"`.

- [ ] **Step 4: Update the three response examples.**

```bash
sed -i '' 's/"feed_offset_meters"/"physical_feed_offset_m"/g' \
  examples/responses/gain_response.json examples/responses/batch_response.json
sed -i '' 's/"position_offset"/"design_feed_offset_m"/g' \
  examples/responses/antenna_details_response.json
```

Task 2's guard is the check.

- [ ] **Step 5: Update `docs/api-documentation.md:457`** (inside a C11-marked response block —
      the guard will catch it if missed).

- [ ] **Step 6: Append the response-field cross-reference to the contract glossary row.**

Task 1 rewrote `docs/domain-contract.md`'s row `:72`; now that the response names exist, append
to that row's rename note:

```markdown
      The *physical* position remains a derived property, reported as
      `GeometryInfo.physical_feed_offset_m` (the per-request total: design offset + steering
      displacement) and configured as `FeedInfo.design_feed_offset_m` (the static design
      offset). Both renamed in the same pass, so no response field can be mistaken for the
      aim point.
```

- [ ] **Step 7: Run the full gate.**

```bash
./scripts/check.sh
```

Expected: green.

- [ ] **Step 8: Commit.**

```bash
git add -A
git commit -m "feat(C8 stage 1): rename the physical feed-offset response fields

GeometryInfo.feed_offset_meters -> physical_feed_offset_m (per-request total:
design offset + steering displacement).
FeedInfo.position_offset -> design_feed_offset_m (static, per-antenna).

Two names because they are two quantities; both docs now cross-reference each
other and disclaim the aim point. Unit suffix _m matches the config/data layer
(phase_center_offset_m, axial_defocus_m). No value changed.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Mirror the renames into the unguarded contract artifacts

**Goal:** `openapi.yaml` and every remaining example/doc that carries the old names is updated
by hand, since C7's drift guard does not exist yet (standing rule 4).

**Files:**
- Modify: `openapi.yaml` — `feed_position` at `146, 170, 570, 726` (prose), `776`, `1077`
  (required list), `1091`, `1319` (required), `1333`, `1558` (required), `1570`;
  `feed_offset_meters` at `197, 230, 1193`
- Modify: `examples/api_requests.json` (6), `examples/postman_collection.json` (4, inside
  escaped `raw` strings), `examples/python_examples.py` (8, including two Python **parameter
  names** and their docstrings), `examples/QUICKSTART.md` (2), `examples/TESTING.md` (1 —
  `feed_offset_meters`)
- Modify: `docs/architecture.md` — `feed_position` at `649, 685, 781, 792, 862, 906`;
  `feed_offset_meters` at `710, 810, 824`

**Acceptance Criteria:**
- [ ] `openapi.yaml` uses the new names in all three request schemas' `properties` **and**
      their `required` lists, in every `example:` block, and in the `/h3-heatmap` prose at
      `:726`.
- [ ] `openapi.yaml`'s `GeometryInfo` z-sign description is corrected from "positive toward the
      reflector vertex" to "**positive = away from the reflector vertex**" — it contradicts
      `schemas.rs` and `phase.rs` today (a stale survivor of the 2026-07-02 fix, which updated
      the Rust doc only). This is a doc-truth fix inside the field being renamed, not scope
      creep.
- [ ] `python_examples.py` renames the *parameter* `feed_position` on both request-builder
      functions (`:84`, `:152`) plus their docstrings and call sites (`:244`, `:268`), so the
      module still runs.
- [ ] `docs/architecture.md`'s JSON blocks use the new names. Other inaccuracies in that file
      are **D5's job — do not fix them here.**

**Verify:**
```bash
python3 -c "import json,sys; json.load(open('examples/api_requests.json')); json.load(open('examples/postman_collection.json')); print('json ok')"
python3 -m py_compile examples/python_examples.py && echo "python ok"
uv run --quiet --with pyyaml python3 -c "import yaml,sys; yaml.safe_load(open('openapi.yaml')); print('yaml ok')"
```
All three print `ok`.

**Steps:**

- [ ] **Step 1: `openapi.yaml` — request field.** Rename the property key, the `required`
      entries, and the `example:` keys. The property block at `:1091` (and its twins) keeps its
      description; only the key changes:

```yaml
        feed_pointing_location:
          allOf:
            - $ref: '#/components/schemas/Position3D'
          description: >-
            Earth location the feed's beam is aimed at (NOT the feed's physical
            location on the antenna). ...
```

- [ ] **Step 2: `openapi.yaml` — response fields + the z-sign correction.** At `:1193`:

```yaml
        physical_feed_offset_m:
          type: object
          description: >
            Physical feed offset from the focal point in the antenna frame (meters).
            This is the total offset used for this request: the antenna's static design
            offset (FeedInfo.design_feed_offset_m) plus the displacement induced by
            steering the beam to feed_pointing_location.
            x and y are the lateral displacement of the feed from the optical axis;
            z is the axial displacement from the focal point (positive = AWAY from the
            reflector vertex, matching the phase model's delta_z convention).
            All three components are ~zero for an on-axis feed.
          properties:
```

Also rename the `feed_offset_meters` keys in the response `example:` blocks at `:197,230`.

- [ ] **Step 3: The remaining examples.**

```bash
sed -i '' 's/"feed_position"/"feed_pointing_location"/g' examples/api_requests.json
sed -i '' 's/\\"feed_position\\"/\\"feed_pointing_location\\"/g' examples/postman_collection.json
sed -i '' 's/"feed_position"/"feed_pointing_location"/g' examples/QUICKSTART.md
sed -i '' 's/"feed_offset_meters"/"physical_feed_offset_m"/g' examples/TESTING.md examples/api_requests.json
sed -i '' 's/"position_offset"/"design_feed_offset_m"/g' examples/api_requests.json
```

Then hand-edit `examples/python_examples.py` — it is Python identifiers, not JSON keys:

```python
    def build_gain_request(
        ...
        feed_pointing_location: Dict[str, float],
        ...
    ):
        """
        ...
            feed_pointing_location: Earth location the beam is aimed at, dict with x, y, z
        """
        return {
            ...
            "feed_pointing_location": feed_pointing_location,
        }
```

and the two call sites at `:244`/`:268`.

- [ ] **Step 4: `docs/architecture.md` JSON blocks** — rename the keys at the nine cited lines.
      Nothing else in that file.

- [ ] **Step 5: Run the verify commands above, then the full gate.**

```bash
./scripts/check.sh
```

- [ ] **Step 6: Commit.**

```bash
git add -A
git commit -m "docs(C8 stage 1): mirror the field renames into openapi + unguarded examples

Hand-mirrored per standing rule 4 (C7's drift guard does not exist yet): openapi
request schemas incl. required lists, response schemas, examples, postman,
python, QUICKSTART, TESTING, architecture.md JSON blocks.

Also corrects openapi's GeometryInfo z-sign description, which said 'positive
toward the reflector vertex' while schemas.rs and phase.rs say away from it - a
stale survivor of the 2026-07-02 fix that updated only the Rust doc.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Re-true CLAUDE.md and the roadmap; run the exit grep; open the PR

**Goal:** The remaining onboarding docs match the new contract, the roadmap records stage 1 as
done without claiming all of C8 landed, and the unit's exit criterion is demonstrated.

**Note:** `docs/domain-contract.md` is **not** this task's job — the unit requires the glossary
entry to change in the same commit as the field, so Task 1 rewrote row `:72` (plus rows
`:61,62,92,94` and the prose at `:437-438`) and Task 3 appended the response cross-reference.
Confirm that work is present rather than redoing it.

**Files:**
- Modify: `CLAUDE.md:262` (the "Common Pitfalls" bullet naming `feed_position`)
- Modify: `docs/roadmap-2026-07-work-units.md` (stage-1 status note under C8) and
  `docs/roadmap-2026-07.md:211` (Phase 3 row)
- Verify only (do not re-edit): `docs/domain-contract.md`

**Acceptance Criteria:**
- [ ] `CLAUDE.md`'s pitfalls bullet names `feed_pointing_location`; the frame-confusion warning
      it carries is kept, not deleted.
- [ ] `docs/domain-contract.md` row `:72` already carries the resolved rename note from Task 1
      **and** the response cross-reference from Task 3 (read it; if either is missing, that is
      a Task 1/3 miss — add it here and say so in the PR).
- [ ] `roadmap-2026-07.md:211` no longer reads as if all of C8 landed; the C8 unit in the
      work-units doc marks stage 1 done in C9's style and states stages 2–4 remain.
- [ ] The final exit grep returns no output (Step 3 below).

**Verify:**
```bash
grep -rn 'feed_position' --include='*.rs' --include='*.yaml' --include='*.json' \
  --include='*.py' --include='*.sh' . | grep -v '^./target/' | grep -v feed_position_m \
  | grep -v to_feed_position | grep -v from_feed_position | grep -v get_feed_position \
  | grep -v compute_feed_position_from_pointing | grep -v test_feed_position
```
→ **no output.**

**Steps:**

- [ ] **Step 1: Read `docs/domain-contract.md` rows `:61,62,72,92,94` and the prose at
      `:437-438`** and confirm Tasks 1 and 3 left them correct: new field name on the API rows,
      class-B function names untouched, resolved-rename note present, response cross-reference
      present, invariant row pointing at `feed_pointing_location_is_an_aim_point.rs`. Fix any
      gap here and note it in the PR description.

- [ ] **Step 2: Update `CLAUDE.md:262`** — the pitfalls bullet lists "`feed_position` = pointing
      target not physical offset". Change the identifier to `feed_pointing_location` and keep
      the gotcha; the trap is weaker now but the frame confusion it warns about is not.

- [ ] **Step 3: Run the exit grep** (the Verify block above). Expected: no output. Class-B
      symbols are excluded by the filters — if a *new* hit appears that the filters do not
      cover, it is a miss from Tasks 1–4, not a filter problem.

- [ ] **Step 4: Annotate the roadmap.** Under the C8 unit, mark stage 1 done with the date and
      branch, in the style used by C9 (`**[✅ DONE 2026-07-26 — branch …]**`), and note that
      stages 2–4 remain. Update the Phase 3 row in `roadmap-2026-07.md:211` from "C8 contract
      finalization landed" (currently written in the past tense as if complete) to reflect
      stage-by-stage progress.

- [ ] **Step 5: Commit and open the PR.**

```bash
git add -A
git commit -m "docs(C8 stage 1): re-true CLAUDE.md and the roadmap for the rename

CLAUDE.md's coordinate-confusion pitfall now names feed_pointing_location; the
roadmap records stage 1 of four as done rather than implying all of C8 landed.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
git push -u origin feat/c8-stage1-aim-point-field-rename
gh pr create --title "feat(C8 stage 1): rename feed_position -> feed_pointing_location" --body "$(cat <<'EOF'
Stage 1 of 4 of unit C8 (v1 contract finalization — the one sanctioned breaking pass,
decided 2026-07-08 on pre-production grounds). Stages 2-4 (required `coordinate_system`,
typed warnings, endpoint coherence) follow as separate PRs; C7's drift guard then freezes
the contract.

**Breaking changes**
- Request: `feed_position` -> `feed_pointing_location` on `GainRequest`, `HeatmapRequest`,
  `H3LinkBudgetRequest`. Clean break, no serde alias — the old key is a 400 naming the new
  field, pinned by `legacy_feed_position_key_is_rejected_with_400`.
- Response: `GeometryInfo.feed_offset_meters` -> `physical_feed_offset_m` (per-request total:
  design offset + steering displacement); `FeedInfo.position_offset` -> `design_feed_offset_m`
  (static per-antenna). Two names because they are two quantities.

**No computed value changed.** `git diff main -- antenna-model/src/model/` is empty except
doc comments; every existing numeric assertion passes with its original expected value.
`PHYSICS_MODEL_VERSION` stays 5.

**Deliberately unchanged:** every symbol naming a genuinely physical feed position —
`EClockConeCoordinates::to_feed_position`, `ray_trace::get_feed_position`,
`artifact_export::feed_position_m`, `compute_feed_position_from_pointing`.

**Also landed:** a drift guard for `examples/responses/` (the response-side sibling of G3),
added before the response rename so it catches this pass's misses; and a correction to
`openapi.yaml`'s `GeometryInfo` z-sign description, which contradicted `schemas.rs`/`phase.rs`.

Plan: `docs/plan-c8-stage1-aim-point-field-rename.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## 5. Exit criteria for the PR

1. `./scripts/check.sh` green (fmt, clippy `-D warnings` over `--all-targets`, full workspace
   tests, audit).
2. `grep -rn 'feed_position'` over code + spec + examples returns nothing outside the class-B
   physical-position symbols listed in §2.2 and historical docs
   (`docs/superpowers/**`, `docs/review-findings-*.md`, `docs/findings-*.md`,
   `docs/plan-*.md`, and the contract's own changelog note).
3. `feed_offset_meters` and `position_offset` return nothing outside those same historical docs.
4. **No computed value changed** — the diff contains no change to a numeric literal in any
   assertion, and no change under `antenna-model/src/model/`. This is the reviewable property
   that makes a large breaking diff safe; state it in the PR description and make it easy to
   confirm (`git diff main -- antenna-model/src/model/` should be empty except doc comments).
5. `openapi.yaml` parses and describes the new names in properties, `required` lists, and
   examples.
6. New guards observed failing before being accepted: the 400 clean-break test (Task 1 step 2)
   and the response-example guard (Task 2 step 3).

## 6. Out of scope (explicitly)

- **Stages 2–4 of C8** — required `coordinate_system` (stage 2), typed warnings (stage 3),
  `/heatmap` H3 stub removal + spec completeness (stage 4). One PR each, after this one.
- **C7's drift guard** — depends on C8 finishing.
- Any physics or semantics change; any change under `antenna-model/src/model/` beyond doc
  comments. `PHYSICS_MODEL_VERSION` stays **5**.
- `FeedInfo`'s absence from `openapi.yaml` (spec completeness — C8 stage 4 / C7).
- `examples/responses/gain_response.json`'s nonsensical `gain_db` value and the other
  content-truth problems in `docs/architecture.md` — **D5**.
- Renaming any class-B physical-position symbol (§2.2).

## 7. Gotchas

1. **Two meanings, one word.** The single highest-risk mistake in this pass is a blanket
   `sed s/feed_position/feed_pointing_location/`. It would rename
   `EClockConeCoordinates::to_feed_position`, `get_feed_position`,
   `compute_feed_position_from_pointing`, and `artifact_export::feed_position_m` — all of which
   name a genuinely physical position. Rename by call site, guided by §2.2.
2. **The compiler does not cover string literals or JSON paths.** `validator.rs`'s
   `"feed_position"` field-name arguments (which reach the client in error bodies) and
   `routes.rs`'s `json_value.get("position_offset")` assertions both compile fine after a
   partial rename. Grep, don't trust `cargo check`.
3. **`benches/` compiles under the gate.** `clippy --workspace --all-targets` includes
   `antenna-model/benches/{heatmap_benchmarks,computation_modes}.rs`; both use `feed_position`.
4. **`postman_collection.json` stores request bodies as escaped strings** inside `raw` fields —
   the key appears as `\"feed_position\"`, so a plain `"feed_position"` sed misses it.
5. **`RUST_MIN_STACK`.** Run tests via `./scripts/check.sh` or export
   `RUST_MIN_STACK=16777216`; the calibrate 3D→4D round-trip overflows the default stack.
6. **Do not add a serde alias "just for the transition."** The decision explicitly rejected
   shims; Task 1's 400 test exists to make reintroducing one a test failure.
