# Plan — S6: Close the H3 link-budget validator gaps

Roadmap unit: `docs/roadmap-2026-07-work-units.md` → **S6** (Phase 2). Effort **S**. Independent
of Phase 3 and of S1–S5. Goal (theme T2): the H3 link-budget endpoint validates the same fields
its siblings do, so bad input is rejected up front instead of producing NaN or a late framework
error.

## Problem (verified 2026-07-18 @ `b0f5b81`)

`validate_h3_link_budget_request` (`service/validator.rs:203-227`) validates positions,
`frequency_mhz`, `n_rings (≤10)`, and the attitude quaternion — but **skips three fields** that
the request carries (`api/schemas.rs`, `H3LinkBudgetRequest`):

- **`temperature_k: Option<f64>`** — a non-positive or NaN value flows straight into
  `pattern::g_over_t_from_gain_db(gain_db, t)` at `service/h3_link_budget.rs:626`, producing a
  NaN `g_over_t_db` in the response (no error).
- **`pointing_frequency_mhz: Option<f64>`** — the gain and heatmap validators already validate this
  (`validator.rs:97,183`); the H3 validator does not.
- **`h3_resolution: Option<u8>`** — an out-of-range value is caught late by
  `h3o::Resolution::try_from` at `h3_link_budget.rs:304` instead of in the validator layer.

## Design (mirror the existing validator idiom exactly)

- **Reuse existing snake_case error codes and `ValidationError` variants** — C3 owns the error
  vocabulary; do **not** invent new codes. Use `ValidationError::OutOfRange { param, value, min, max }`
  (already defined, `error.rs:206`) for range failures and `ValidationError::InvalidValue` for
  non-finite, matching `validate_frequency`'s style (`validator.rs:343`).
- Validate optional fields only when `Some` (mirrors the gain validator's `pointing_frequency_mhz`
  handling at `validator.rs:97`).

## Files to change

1. `antenna-model/src/service/validator.rs` — extend `validate_h3_link_budget_request`; add a
   `validate_temperature` helper; add unit tests.
2. `openapi.yaml` — mirror the new constraints onto the `H3LinkBudgetRequest` schema component.
3. `docs/api-documentation.md` — note the H3 request field constraints.

## Steps (TDD — write the tests first, watch them fail, then implement)

### Step 1 — Failing unit tests (in `validator.rs`'s `#[cfg(test)] mod tests`)
Copy the `validate_frequency` test style (`validator.rs:751+`). Add tests asserting
`validate_h3_link_budget_request` **rejects**:
- `temperature_k = Some(0.0)`, `Some(-5.0)`, `Some(f64::NAN)`, `Some(20000.0)` (> upper bound);
- `pointing_frequency_mhz = Some(60_000.0)` (and `Some(50.0)`) — out of `[100, 50000]`;
- `h3_resolution = Some(16)` (> 15).

And one **passing boundary** case: a request with `temperature_k = Some(10000.0)`,
`pointing_frequency_mhz = Some(50_000.0)`, `h3_resolution = Some(15)` validates `Ok`.
(Build a valid base `H3LinkBudgetRequest` fixture once; mutate one field per test.)
Run: `cargo test -p antenna-model validate_h3` — confirm the new tests **fail**.

### Step 2 — Implement in `validate_h3_link_budget_request`
Add, after the existing checks:
- `if let Some(pf) = req.pointing_frequency_mhz { validate_frequency(pf, "pointing_frequency_mhz")?; }`
- `if let Some(t) = req.temperature_k { validate_temperature(t, "temperature_k")?; }`
- `if let Some(r) = req.h3_resolution { if r > 15 { return Err(ValidationError::OutOfRange { param: "h3_resolution".into(), value: r as f64, min: 0.0, max: 15.0 }); } }`

Add the helper (near `validate_frequency`):
```rust
fn validate_temperature(t_k: f64, param: &str) -> ValidationResult<()> {
    if !t_k.is_finite() {
        return Err(ValidationError::InvalidValue { param: param.into(), reason: format!("value is not finite: {t_k}") });
    }
    // No pre-existing temperature bound in the codebase — spec-sanctioned range (S6).
    if !(0.0 < t_k && t_k <= 10_000.0) {
        return Err(ValidationError::OutOfRange { param: param.into(), value: t_k, min: 0.0, max: 10_000.0 });
    }
    Ok(())
}
```
(Confirm `> 0` strictly — `g_over_t_from_gain_db` takes `log10(t)`, so `t = 0` and `t < 0` must
both be rejected.)

### Step 3 — Docs + spec (standing rule 4)
- `openapi.yaml`: add `minimum`/`maximum`/description constraints to `temperature_k`,
  `pointing_frequency_mhz`, `h3_resolution` on the **`H3LinkBudgetRequest` schema component**.
  Note: the `/api/v1/h3-heatmap` *path* is absent from openapi (roadmap C1); the request/response
  **schema components** still exist there (orphaned) — that is where these constraints belong. Do
  not add the missing path (out of scope; left for C1 per the maintainer).
- `docs/api-documentation.md`: document the H3 field constraints in the H3 section (mirror how the
  gain endpoint's constraints are described).

### Step 4 — Full gate
`cargo test --workspace`, `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`
(`scripts/check.sh`). Existing tests pass unchanged; new validator tests pass.

## Exit criteria (definition of done)

1. `temperature_k` rejected when non-positive / non-finite, with a sane upper bound (≤ 10000 K).
2. `pointing_frequency_mhz` validated via the same `validate_frequency` call the gain/heatmap
   validators use.
3. `h3_resolution` range-checked (0–15) in the validator layer, not late in `h3_link_budget.rs`.
4. A test per rejection + one passing boundary case; existing tests unchanged.
5. openapi.yaml (`H3LinkBudgetRequest` schema) + api-documentation.md constraints mirrored; gate green.

## Watch-outs

- **Do not invent error codes / new `ValidationError` variants** — reuse `OutOfRange`/`InvalidValue`
  (C3 owns vocabulary).
- **`> 0` strictly** for temperature (log10 domain), not `>= 0`.
- The temperature upper bound (10000 K) is new because the codebase has none today — this is
  explicitly sanctioned by the S6 spec; keep it as documented.
- No physics/model code (standing rule 2). Validator layer only.
- **Branch:** base on `main` (files are disjoint from S1/S2 — validator.rs, not middleware) so it
  can run fully in parallel with S2.
