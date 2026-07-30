# Finding 2026-07-29 — the correction surface collapses to zero across the top knot span of every axis

**Found by:** roadmap unit D12 (calibrate CLI end-to-end test), Task 3, while investigating why
the fitted correction surface removed only 25% of a deliberately injected, smooth, trivially
representable bias.

**Severity:** correctness, **reaches the served path**. Latent today — no `.bin` artifact ships
and all four enabled antennas are uncalibrated design-spec entries that load no correction
surface — but it becomes a live wrong-answer bug the moment D9/D14 ship a real artifact.

**Status:** filed, not fixed. Outside D12's charter (D12 is a test unit; this is a numerics
defect in the fitter and the interpolator).

---

## The defect

`CorrectionSurface::evaluate` returns **~0 instead of the fitted value** for any query lying in
the **topmost knot span** of *any* of its three axes — frequency, E-cone, or E-clock. It is not
merely an endpoint artifact: the value decays continuously to zero as the query approaches the
upper bound.

The service-side 4D interpolator, `antenna_model::model::evaluate_correction`
(`antenna-model/src/model/correction_interpolator.rs`), has the **same behavior**.

## Reproduction

Fit a correction surface to a residual that is the **constant 1.5 dB** everywhere, on a
7 × 11 × 16 grid (frequency 400–700 MHz, E-cone 0–24°, E-clock 0–337.5°), spline order 4, knot
counts 4/6/8, regularization 1e-9. A constant is exactly representable by a B-spline basis
(partition of unity), so every query should return 1.5.

Measured:

| query | returned | expected |
|---|---|---|
| interior (525, 7, 100) | **+1.500000** | +1.5 |
| frequency at min (400, 7, 100) | +1.500000 | +1.5 |
| frequency at max (700, 7, 100) | **+0.000000** | +1.5 |
| frequency just under max (699.999, 7, 100) | **+0.000090** | +1.5 |
| E-cone at max (525, 24, 100) | **+0.000000** | +1.5 |
| E-clock at max (525, 7, 337.5) | **+0.000000** | +1.5 |
| service-side 4D, frequency at max | **+0.000000** | +1.5 |
| service-side 4D, E-cone at max | **+0.000000** | +1.5 |
| service-side 4D, E-clock at max | **+0.000000** | +1.5 |

The lower bound is fine; only the upper bound collapses. `699.999 → 0.000090` is the
load-bearing row: this is a whole-span failure, not an endpoint special case.

**Points affected on a regular grid = the union of the three upper faces.** For the grid above,
332 of 1232 points (26.9%) — matching `1 − (6/7)(10/11)(15/16) = 26.95%` exactly. Any regular
measurement grid loses roughly this fraction of its correction.

## Why it went unnoticed

1. **The two implementations agree with each other.** `calibrate/src/artifact_export.rs`'s
   round-trip test (`test_round_trip_matches_3d_evaluation`) compares the 3D
   `CorrectionSurface::evaluate` against the 4D `evaluate_correction` and asserts agreement to
   1e-9 — which passes, because *both* are wrong in the same way. Worse, `find_knot_span` in
   `correction_interpolator.rs:230-234` carries a comment saying a previous off-by-one was fixed
   so that it "matches `calibrate/src/correction_surface.rs::find_knot_interval`" — the bug was
   propagated by making the two agree.
2. **The round-trip test only samples interior points** (clocks 10–349, cones 0.5–9.5, freqs
   8050–8350 against a wider domain), so it cannot reach the failing span.
3. **The reported fit statistics disguise it.** `compute_fit_statistics`
   (`correction_surface.rs:889`) evaluates the surface at the data points, so the boundary
   points contribute their full residual — but the summary numbers are hard to read:
   - `r_squared` returns a hardcoded **1.0** whenever `ss_tot == 0`
     (`correction_surface.rs:944-946`), which is exactly the constant-residual case. So a
     constant fit reports `R² = 1.0000` *and* `rmse = 0.77867` simultaneously — a contradiction
     that reads as success.
   - On the real D12 fixture the binary logged `Correction surface fitted successfully.
     RMSE: 0.976 dB, R²: -3.124`. A **negative R² alongside "fitted successfully"** is the
     symptom to watch for.

## What it is *not*

Two hypotheses were tested and **refuted** during triage — recorded so they are not re-chased:

1. **Not the degenerate adaptive knots.** `generate_adaptive_knots`
   (`correction_surface.rs:567`) picks knots by quantile index into the *sorted, duplicated*
   data, which on grid-structured data can land on the axis min or max; `generate_knot_vector`
   then clamps by prepending/appending `order` copies, yielding multiplicity **5** at a boundary
   for order 4. This is real — the frequency axis produces
   `[400, 400, 400, 400, 400, 500, 600, 700, 700, 700, 700, 700]` — and
   `validate_knot_vector` (`correction_surface.rs:610`) does **not** catch it (it checks only
   length and non-decreasing, never multiplicity). **But it is not the cause of this defect:**
   forcing `adaptive_knots: false` produces a clean uniform knot vector and gives a
   *numerically identical* fit (rmse 0.9756, R² −3.1242 either way). Worth fixing separately as
   latent fragility; not this.
2. **Not underdetermination.** For knot counts 4/6/8 at order 4 the surface has
   `(4+4)·(6+4)·(8+4) = 960` coefficients. The D12 fixture supplied 288 points, so the system
   was badly underdetermined — but enlarging the grid to 1232 points (overdetermined) only moved
   R² from −3.12 to −1.93. Underdetermination is a real separate problem (see below); it is not
   what produces the zero.

## Two adjacent problems this surfaced

Both are separate from the edge collapse and should be filed alongside it:

- **The fitter's data-sufficiency check measures the wrong quantity.**
  `validate_fitting_inputs` (`correction_surface.rs:1004`) requires
  `(spline_order + 1)³ = 125` points. The actual requirement is the **coefficient count**,
  `∏(num_knots_axis + order)`, which for the shipped 4/6/8 configuration is **960**. A run with
  126–959 points is accepted, silently underdetermined, and held together only by the ridge
  term. With `regularization: 0.0` the solve correctly fails with "the 960 basis functions are
  not identifiable from 1232 data points" — so the information exists, it is just not checked
  up front.
- **`compute_r_squared`'s `ss_tot == 0` early return of 1.0** (`correction_surface.rs:944`)
  reports a perfect fit for the degenerate case rather than "undefined". This is what made the
  constant-residual diagnostic read as success at first glance.

## Suggested fix direction (not prescriptive)

The span lookup needs to treat the upper bound as belonging to the last *non-degenerate* span,
in both implementations, and the fix must be verified at the domain edges rather than only in
the interior. Because the two implementations are cross-checked against each other, **fix and
test them together** — a change to one that keeps the round-trip test green proves nothing about
correctness.

Acceptance should include: fitting a known constant and asserting recovery at every corner and
face of the domain, not just interior points.
