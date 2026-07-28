# Implementation Plan — C8 Stage 2: make `coordinate_system` required

**Unit:** C8 stage 2 of 4 (`docs/roadmap-2026-07-work-units.md`, register rows **C8** and **S7**
in `docs/roadmap-2026-07.md`)
**Effort:** M (one session)
**Decision status:** **Decided 2026-07-08** — pre-production confirmed, no consumers exist;
break once now, freeze behind C7. Register row **S7** was superseded by this stage on the same
date. No decision step remains; this was execution only.
**Status:** ✅ **DONE 2026-07-27**, branch `feat/c8-stage2-required-coordinate-system`.

**Goal:** Make `Position3D.coordinate_system` a required field and delete the magnitude-based
auto-detection it replaces, so a position's frame is always declared and never guessed.

---

## 0. Why stage 2 was next

Stage 1 (the aim-point field renames) landed 2026-07-26 at `95a2c2e` (#19). The Phase 3 chain
is `C3 → C4 → C2 → C9 → C8 → C7`; C3/C4/C2 landed in `aef5206` (#17), C9 in `0c8bcb2` (#18),
and stage 2's other stated dependencies (G3's example guard, S6's validation constraints) were
already in place. C8's stages are explicitly sequential, one PR each, so stage 2 was the only
unblocked item.

## 1. What the unit required

> **Stage 2 — Make `coordinate_system` required (remove auto-detection).**
> - `Position3D.coordinate_system` becomes a required field; missing → deserialization/
>   validation error naming the exact field path. Delete the magnitude-based auto-detection
>   (`Position3D::coordinate_system()` heuristic, `ECEF_THRESHOLD_M`) and the now-dead
>   `coordinate_ambiguity_warnings` plumbing; **keep** per-system range validation.
> - Fix the stale threshold comments while in the area — or delete them with the machinery
>   they describe.
> - Update the domain contract's frame table + GEO-trap gotcha (record as resolved-by-design,
>   don't silently delete the history).
> - Exit: a geodetic GEO-altitude position without a tag is now a 4xx with a clear message
>   (test); all examples carry explicit `coordinate_system`; contract updated.
> - Gotcha: the detection unit tests must be **rewritten** to assert the new required-field
>   behavior, not deleted wholesale.

Plus C8's standing constraint: **this pass must not alter any computed value.**

## 2. The defect being removed

Auto-detection classified a position as ECEF when any of `|x|`, `|y|`, `|z|` exceeded
`ECEF_THRESHOLD_M` = 6400 km. That boundary is not decidable, and it failed in **both**
directions:

| Direction | Example | Old behavior |
|---|---|---|
| Geodetic read as ECEF | GEO satellite, `alt = 35,786,000 m` | Parsed as a near-Earth-centre ECEF point → confidently wrong gain under HTTP 200 |
| ECEF read as geodetic | Earth-surface ECEF, e.g. `(4510731, 4510731, 3488865)` | Below the boundary → parsed as `lon = 4,510,731°` |

The first direction was known and warned about (`coordinate_ambiguity_warnings`). The second
was not — and it was live in the shipped documentation, which is how this stage found it.

## 3. Naming decision for the constructors (recorded)

`Position3D::new(x, y, z)` could not survive a required field, so it needed a replacement.
Two named constructors were chosen over a 4-argument `new`:

```rust
Position3D::ecef(x, y, z)                       // meters from Earth's centre
Position3D::geodetic(lon_deg, lat_deg, alt_m)   // WGS84
```

Rationale: the parameter *meanings* differ per frame, so the geodetic constructor can name its
arguments; and a `new` that silently picks a frame is the same trap the wire format is
shedding, one layer down. `is_ecef()` / `is_geodetic()` are kept; the `coordinate_system()`
*method* is gone, since the field now answers directly.

## 4. What landed

**Schema (`api/schemas.rs`).** `coordinate_system: CoordinateSystem` — no `Option`, no
`#[serde(default)]`, no `skip_serializing_if`, so the tag is always on the wire in both
directions. `ECEF_THRESHOLD_M` and the `coordinate_system()` heuristic deleted; module and
type docs rewritten to state *why* the tag is required, not just that it is.

**Service.** `warn_if_ambiguous`, `coordinate_ambiguity_warnings`,
`GEODETIC_AMBIGUITY_ALTITUDE_M` and the `evaluator.rs` call site deleted (a comment block
marks the spot and says what it was for). Per-frame range validation is unchanged and now
dispatches on the declared tag. The stale `validate_ecef_position` doc comment ("< 10,000 km
from center", against a 400,000 km constant) was corrected.

**Call sites (~125).** Each untagged site was converted to the frame the old heuristic would
have chosen, which is what keeps every numeric assertion identical. Sites that already set an
explicit tag were converted to *that* tag — one of them (`batch.rs`, an Earth-surface ECEF
emitter) would have been silently flipped by a magnitude-only conversion, which is the same
defect class this unit removes.

**Tests, rewritten rather than dropped** (per the unit's gotcha). The auto-detection suite
became required-field and declared-frame assertions:

| Removed | Replaced by |
|---|---|
| `test_position3d_{ecef,geodetic,boundary}_detection`, `test_detection_threshold_is_6400km`, `test_explicit_coordinate_system_overrides_detection` | `schemas.rs::the_declared_frame_is_the_frame_at_any_magnitude` |
| `test_position3d_backward_compatible_deserialization` | `schemas.rs::a_position_without_a_coordinate_system_is_rejected` (asserts the opposite — this stage is the sanctioned break) |
| `test_position3d_no_coordinate_system_not_serialized` | folded into `test_position3d_serialization` (the tag is now always serialized) |
| `coordinates_3d.rs::test_coordinate_detection_boundary` | `test_frame_tag_is_read_not_inferred_from_magnitude` |
| the three `test_warn_if_ambiguous_*` + `test_coordinate_ambiguity_warnings_full_request` | `validator.rs::{geo_altitude_geodetic_position_is_validated_as_geodetic, the_same_numbers_validate_differently_per_declared_frame, a_position_without_a_coordinate_system_does_not_deserialize}` |

**New HTTP guards** (`tests/integration/status_code_matrix_tests.rs`):

- `a_position_without_coordinate_system_is_rejected_with_400` — strips the tag from **one
  position field at a time, on all four compute endpoints** (13 cells), asserting 400 +
  `invalid_request_body` + a message naming `coordinate_system`. Per-field because each
  request type declares its positions independently: a `#[serde(default)]` on any single one
  restores the hazard, and a one-field guard would wave the rest through.
- `geo_altitude_geodetic_emitter_is_accepted_when_tagged` — the acceptance half. Without it,
  "reject everything untagged" would pass equally well if tagged GEO input were also broken.

Both assert the tag is actually present in the fixture before removing it, so the guard fails
loudly rather than silently testing nothing if serialization changes.

**Contract + examples.** `openapi.yaml` (`Position3D.required`, the schema and intro prose,
11 inline example positions), `examples/` (requests, `api_requests.json`, the Postman
collection via a JSON round-trip, `python_examples.py`, QUICKSTART/README/TESTING),
`docs/api-documentation.md`, `docs/domain-contract.md` (frame table, the GEO gotcha rewritten
as *resolved by design* with the history retained, glossary rows, two invariant rows),
`docs/architecture.md`, `docs/antenna-model-design-doc.md`,
`docs/calibration-workflow-guide.md`, `docs/partial-calibration-setup-summary.md`, CLAUDE.md.

## 5. Finding, fixed in-unit: 25 mis-served example positions

Tagging by magnitude alone reproduced the heuristic's *second* failure direction, so the
tagged output was audited with an independent rule — a position tagged `geodetic` whose
`|x| > 180` or `|y| > 90` cannot be geodetic. That flagged 25 positions across `openapi.yaml`,
`examples/api_requests.json`, `docs/api-documentation.md`, `docs/architecture.md` and
`docs/calibration-workflow-guide.md`, including the example literally named
`ecef_coordinates`. All were Earth-surface ECEF values (~4.5 Mm) sitting below the 6400 km
boundary — every one had been served as geodetic. Corrected to `ecef`.

The mirror audit (positions tagged `ecef` that read as plausible lon/lat with a large
altitude — the GEO trap) found none.

This is a documentation correction, not a computed-value move: no test asserted on those
examples' numbers.

## 6. Exit criteria — met

- [x] Untagged position → 400 naming the field, on every position field of all four compute
      endpoints (test).
- [x] Tagged GEO-altitude geodetic position accepted end-to-end (test).
- [x] `ECEF_THRESHOLD_M`, the heuristic method, and the ambiguity plumbing: zero references in
      code (only the comment block recording the removal).
- [x] `Position3D::new`: zero references.
- [x] All examples carry explicit `coordinate_system` (G3, C11 and the response guards pass).
- [x] Domain contract updated in the same commit, with the trap recorded as resolved-by-design.
- [x] `./scripts/check.sh` green (fmt, clippy `-D warnings`, full workspace tests, audit).
- [x] **No computed value moved** — 606 unit tests' numeric assertions unchanged.

## 7. Out of scope (explicitly)

- Stages 3 (typed warnings) and 4 (endpoint coherence + spec completeness), then C7's freeze.
- The `vehicle_attitude` `{"w": …}`-object drift still present in `docs/architecture.md` and
  `docs/partial-calibration-setup-summary.md`: pre-existing, unrelated to frames, and already
  covered by **C15** (unguarded client-visible surfaces) and **D5** (design-doc truth). Left
  alone rather than half-fixed under a frame unit.
- The historical sprint records (`docs/implementation-plan*.md`) describing auto-detection as
  built in Sprint 5. They are accurate as history; re-truing them is D5's call, not a frame
  unit's.
