---
description: REST contract rules — typed warning/error vocabularies, OpenAPI generation, and the example guards that fail when a schema changes.
paths:
  - "antenna-model/src/api/**"
  - "antenna-model/src/service/**"
  - "antenna-model/tests/**"
  - "examples/**"
  - "openapi.yaml"
---

# API contract rules

## `openapi.yaml` is generated, never hand-edited

Since C7: `cargo run -p antenna-model --bin generate_openapi`.

## Typed warnings and errors

Response warnings are typed: `ApiWarning { code: WarningCode, message: String }` (C8 stage 3).

- `WarningCode` is a **closed** enum in `antenna-core/src/warnings.rs` — a peer of `error.rs`,
  since the model layer produces warnings too.
- `ErrorCode` is the closed enum in `api/schemas.rs` (promoted from `&str` consts by C7).

**Adding a producer means adding a variant, updating `WarningCode::ALL` and
`docs/api-documentation.md`, then regenerating `openapi.yaml`.**
`tests/warning_code_vocabulary.rs` fails otherwise; `tests/error_code_vocabulary.rs` enforces the
same procedure for errors.

**`code` is the contract, `message` is not** — never branch on message text (the substring test
that C8 stage 3 deleted from `service/heatmap.rs` is why). Heatmap/H3 aggregation dedupes on
`(code, message)`, so a warning meant to appear once per response must keep its message constant
across grid points.

## Changing a request/response schema means updating the examples too

Four guards will say so. Three check an example against the Rust type
(`tests/example_requests_deserialize.rs`, `example_responses_deserialize.rs`,
`example_api_requests_deserialize.rs`). The fourth, `tests/openapi_examples_validate.rs`
(C15 option 3, 2026-08-22), checks **76 JSON example bodies from five sources** — the `examples/`
tree, `examples/postman_collection.json`, and `openapi.yaml`'s own inline
`content.application/json.examples` (the bodies Swagger UI and Redoc render) — against the
generated spec, each one resolved through the `(method, path[, status])` endpoint it claims
rather than a hand-picked component name. Postman's seven bodyless GETs are asserted a different
way: their URLs must still resolve to a documented route.

That fourth guard is the only one that can see a **spec-vs-type gap** (a lying
`#[schema(value_type = …)]`, a field the derive cannot see, a custom serializer like
`nan_as_null` whose wire shape utoipa cannot know) and the only one that asserts `enum`
membership, `minimum`, tuple arity, or nullability.

**Two of its properties are deliberate and must not be relaxed:**

1. A JSON Schema keyword that is **unimplemented — or present but malformed — is a hard
   failure**, never a silent skip. A validator that shrugs at what it cannot read keeps passing
   while checking less.
2. **Every negative control is paired with a positive one.**

If you add a component schema, either give it an example or add it to `UNEXERCISED_COMPONENTS`
with the reason — and that reason is **checked, not trusted**: the list fails if an example does
reach a component it names.

## Endpoint coherence

`/heatmap` serves **rectangular grids only** — the `h3` grid type was a `not_implemented` stub,
removed 2026-07-28 (C8 stage 4); the real H3 grid is the separate `/h3-heatmap` endpoint. A merge
of the two is tracked as feature **F5**, not yet decided.

## Served-value honesty

- The **P10 off-axis integrator** (2026-07-15) makes served off-axis gain numerically converged
  at all angles. Served values on uncalibrated antennas are *idealised* physical optics (no
  blockage/strut/edge-diffraction), stated honestly by the off-axis warning (P8).
- The **F7 sidelobe-floor redesign** (`PHYSICS_MODEL_VERSION` 5) added the Huygens obliquity
  factor `(1+cosθ)/2` on the far-field conversion plus the statistical Ruze sidelobe floor on
  uncorrected-physics antennas (power sum forward, floor-only rear). Calibrated antennas are
  unaffected — see `docs/domain-contract.md`.
- **Read `docs/domain-contract.md`** before touching `service/heatmap.rs` or any API field named
  `*position*`/`*boresight*`. `feed_pointing_location` is a pointing *target*, not a physical
  offset.
