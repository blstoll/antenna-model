# Design: Apply beam squint in the H3 link budget

**Date:** 2026-06-19
**Status:** Approved (pending spec review)
**Finding:** Follow-up to the 2026-06-10 review-findings work (final-review item I1; the pre-existing H3 beam-squint gap).

## Problem

The `/h3` link-budget endpoint accepts `pointing_frequency_mhz` on `H3LinkBudgetRequest`
but never uses it. In `antenna-model/src/service/h3_link_budget.rs`, both gain-computing
paths — `compute_cell_gain` (per-cell, cached) and the boresight-cell path inside
`compute_h3_link_budget` — compute the emitter direction `(az_deg, el_deg)` via
`compute_emitter_direction_with_attitude`, build a `GainCacheKey`, and call
`compute_gain_db` directly. Neither calls `apply_beam_squint_correction`.

By contrast, the `/gain` evaluator (`service/evaluator.rs`) applies the full squint
pipeline: it computes the feed displacement and clock angle from `(feed_x, feed_y)` and,
when `pointing_frequency_mhz` differs from `frequency_mhz` by more than 0.1 MHz, shifts
the emitter direction in direction-cosine `(u, v)` space via `apply_beam_squint_correction`
(Task 10 / finding F9).

Consequence: for a request where pointing frequency differs from operating frequency,
`/h3` silently returns different (uncorrected) gains than `/gain` for identical geometry.
Today this is harmless because no caller supplies a pointing offset (all current callers
leave `pointing_frequency_mhz = None`), but it is a latent correctness gap. A
`TODO(squint)` comment was added at the H3 call site during the review cleanup to track it.

## Goals

- `/h3` honors `pointing_frequency_mhz`: per-cell gains include beam-squint correction
  using the same physics as `/gain`.
- The squint logic has a single source of truth shared by `/gain` and both `/h3` call sites.
- The applied squint magnitude is surfaced on the `/h3` response, consistent with `/gain`.
- When `pointing == operating` frequency (or `pointing_frequency_mhz` is `None`), behavior
  is unchanged and existing H3 tests still pass.

## Non-goals

- No change to the squint *physics* (`apply_beam_squint_correction` is unchanged).
- No change to the correction-surface application (Task 8) or the cache design.
- No per-cell squint reporting (the magnitude is constant across cells for a request).

## Design

### 1. Shared squint helper

Extract the squint application currently inlined in the evaluator into one function in
`antenna-model/src/model/coordinates_3d.rs`, next to `apply_beam_squint_correction`
(keeps the physics together):

```rust
/// Apply frequency-offset beam squint to an emitter direction.
///
/// Computes the feed radial displacement and clock angle from `(feed_x, feed_y)` and,
/// when the operating and pointing frequencies differ by more than 0.1 MHz, shifts the
/// direction in (u, v) space via `apply_beam_squint_correction`. Otherwise returns the
/// input direction unchanged.
///
/// Returns `(az_deg, el_deg, squint_deg)`; `squint_deg == 0.0` when no correction is applied.
pub fn squint_corrected_direction(
    az_deg: f64,
    el_deg: f64,
    operating_freq_mhz: f64,
    pointing_freq_mhz: f64,
    feed_x: f64,
    feed_y: f64,
    focal_length_m: f64,
) -> (f64, f64, f64) {
    let feed_displacement_m = (feed_x * feed_x + feed_y * feed_y).sqrt();
    let displacement_clock_angle_rad = feed_y.atan2(feed_x);
    if (pointing_freq_mhz - operating_freq_mhz).abs() > 0.1 {
        apply_beam_squint_correction(
            az_deg,
            el_deg,
            pointing_freq_mhz,
            operating_freq_mhz,
            feed_displacement_m,
            focal_length_m,
            displacement_clock_angle_rad,
        )
    } else {
        (az_deg, el_deg, 0.0)
    }
}
```

The `pointing_freq_mhz.unwrap_or(operating)` defaulting stays at each call site (the helper
takes an explicit pointing frequency), so the helper is pure and easy to test.

### 2. Wire into all three call sites

- **Evaluator** (`service/evaluator.rs`): replace the inline displacement/clock-angle/gate
  block with a single call to `squint_corrected_direction`. Behavior-preserving — guarded
  by existing `/gain` squint tests. The reported `beam_squint_deg` continues to use the
  returned `squint_deg`.
- **H3 `compute_cell_gain`** and **H3 boresight-cell path** (`service/h3_link_budget.rs`):
  after computing the raw `(az_deg, el_deg)`, call `squint_corrected_direction` and use the
  corrected `(az, el)` for the cache key and `compute_gain_db`. Remove the `TODO(squint)`
  comment.

### 3. Cache-key interaction

The `GainCacheKey` is built from `(az, el, freq, feed_x, feed_y, feed_z)`. The **corrected**
`(az, el)` is fed into both the cache key and `compute_gain_db`. This is correct: the cached
value is the gain at the angle actually evaluated, and two cells that squint to the same
effective angle legitimately share a cache entry. Squint is applied *before* the cache
lookup; the Task 8 correction surface is applied *after* the lookup. The two are independent
and both remain correct.

### 4. Response field

Add to `H3LinkBudgetResponse` (`api/schemas.rs`):

```rust
/// Beam squint magnitude applied (degrees), when pointing frequency differs from
/// operating frequency. Omitted when no squint is applied.
#[serde(skip_serializing_if = "Option::is_none")]
pub beam_squint_deg: Option<f64>,
```

The squint magnitude depends only on feed displacement (constant per request) and the
frequency offset (constant per request), so it is computed once and is the same value the
per-cell corrections use. Set `Some(x)` when `x > 0.001°`, else `None` — mirroring `/gain`.
Add the field to `openapi.yaml` under the `H3LinkBudgetResponse` schema.

Because the magnitude is constant per request, `compute_h3_link_budget` can compute it once
(from `feed_x, feed_y, focal_length`, and the operating/pointing frequencies) when assembling
the response, rather than threading it out of every cell.

### 5. Testing

- **Discriminating test:** an `/h3` request with `pointing_frequency_mhz != frequency_mhz`
  and a steered/offset feed (displacement > 0) produces cell gains that differ from the same
  request with no pointing offset, and `beam_squint_deg` is `Some`.
- **Parity/regression:** with `pointing == operating` (or `None`), `/h3` output is unchanged
  and `beam_squint_deg == None`; existing H3 tests still pass.
- **Helper unit test:** `squint_corrected_direction` returns the input unchanged with
  `squint_deg == 0.0` when the frequency offset is ≤ 0.1 MHz; returns a shifted direction
  with non-zero `squint_deg` otherwise.
- **Evaluator regression:** existing `/gain` squint tests must stay green (proves the
  refactor is behavior-preserving).

## Files affected

- `antenna-model/src/model/coordinates_3d.rs` — new `squint_corrected_direction` helper (+ unit test).
- `antenna-model/src/model/mod.rs` — re-export the helper.
- `antenna-model/src/service/evaluator.rs` — call the helper instead of the inline block.
- `antenna-model/src/service/h3_link_budget.rs` — apply squint at both call sites; compute
  response-level magnitude; remove `TODO(squint)`.
- `antenna-model/src/api/schemas.rs` — `beam_squint_deg: Option<f64>` on `H3LinkBudgetResponse`.
- `openapi.yaml` — document the new response field.

## Risks

- The evaluator refactor touches the working `/gain` path. Mitigated by the existing
  `/gain` squint test suite, which must remain green.
- Cache correctness: corrected angles must be used consistently for both the cache key and
  the gain call (do not key on raw angles and evaluate at corrected angles, or vice versa).
