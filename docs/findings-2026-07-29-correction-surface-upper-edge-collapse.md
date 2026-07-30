# Finding 2026-07-29 — the correction surface collapses to zero across the top knot span of every axis

**RESOLVED 2026-07-30 by commit `a866cfb`** ("fix: evaluate the B-spline basis correctly at a
domain maximum"). The defect was entirely inside `calibrate`'s fitter (`bspline_basis` /
`accumulate_normal_equations` in `calibrate/src/correction_surface.rs`) — **not** in the
service-side 4D interpolator. See "The real mechanism" and "Correction — the service-side 4D
interpolator was never defective" below for what actually happened; the rest of this document
is kept as-filed (including two now-superseded lines in the reproduction table) with corrections
layered on top, so the triage trail stays legible. Follow-on commits: `e87efe6` (re-pinned
`fit_matches_openblas_golden`, which had baked in the buggy basis), `41b7e94` (corrected an
affected-point count and noted a pre-existing, currently-unreachable degenerate-axis gap), and
`c79d2cf` (retargeted D12's improvement-ceiling and bias-recovery assertions to the post-fix
numbers). **This fix does not make the fit well-determined** — see "Still open" below.

**Found by:** roadmap unit D12 (calibrate CLI end-to-end test), Task 3, while investigating why
the fitted correction surface removed only 25% of a deliberately injected, smooth, trivially
representable bias.

**Severity (as filed):** correctness, **reaches the served path**. Latent at filing time — no
`.bin` artifact ships and all four enabled antennas are uncalibrated design-spec entries that
load no correction surface — but it would have become a live wrong-answer bug the moment D9/D14
shipped a real artifact. In fact only the fitter was defective: the served evaluation path
(`evaluate_correction`) was correct throughout, and would have faithfully served whatever
corrupted coefficients a broken fitter produced. See "Severity consequence" below.

**Status:** fixed. Was outside D12's charter (D12 is a test unit; this was a numerics defect in
the fitter) and filed separately for that reason — then fixed on `fix/correction-surface-endpoint`.

---

## The defect

`CorrectionSurface::evaluate` returns **~0 instead of the fitted value** for any query lying in
the **topmost knot span** of *any* of its three axes — frequency, E-cone, or E-clock. It is not
merely an endpoint artifact: the value decays continuously to zero as the query approaches the
upper bound.

At filing time this looked like it also reached the service-side 4D interpolator,
`antenna_model::model::evaluate_correction` (`antenna-model/src/model/correction_interpolator.rs`)
— see the reproduction table below. **That reading was wrong; see "Correction — the service-side
4D interpolator was never defective" below.** The defect was confined to `calibrate`'s own
`bspline_basis` and its use in fitting.

## The real mechanism

`bspline_basis`'s `k == 1` base case in `calibrate/src/correction_surface.rs` used a half-open
span (`t_i ≤ t < t_{i+1}`), so at `t == t_max` — the exact upper domain boundary — no basis
function was non-zero anywhere, and the basis was not a partition of unity there. A pre-existing
"special case for right endpoint" keyed on `i == knots.len() - 2` was supposed to cover exactly
this, but for a clamped knot vector that index is a **padding** index outside the valid basis
range `0..knots.len() - order` — it never fired for a basis function that is actually evaluated.

Measured with all coefficients set to 1.0 (a correct basis must return exactly 1.0 everywhere,
by partition of unity): **1.000000000 at t=0.99, 1.000000000 at t=0.9999, 0.000000000 at
t=1.0.** Only the exact endpoint was wrong — this is a razor-thin, not a broad, failure, which is
consistent with `699.999 → 0.000090` in the reproduction table below.

**The damage was two-stage, and the fitting stage was the harmful one.**
`accumulate_normal_equations` uses the same basis function to build the normal equations, so
every measurement lying **exactly** on an axis maximum contributed an **all-zero row**. On a
regular measurement grid the maximum always has data on it (72 of 288 rows in D12's fixture sit
at 700 MHz). The last coefficient on that axis therefore received no data support at all and was
driven to ~0 by the ridge regularization term — corrupting the fit across the **entire** top knot
span, not just the single boundary point. That is why a query at 699.999 returned 0.000090 while
the basis function value there was already ≈1.0: the basis was fine one part in ten-thousand
short of the edge, but the *coefficient it was multiplying* had already been destroyed during
fitting.

**Severity consequence.** Because the evaluation-side error was confined to the exact boundary
and the service-side interpolator uses a different, correctly-clamping algorithm (see below), the
served path itself was never wrong. What was wrong were the **coefficients** written into a
`.bin` artifact by a defective fitter — and the service would have faithfully served whatever
that fitter produced. This is still blocking for D9/D13/D14 (a shipped artifact would have
carried the corruption into production), but it is an artifact-*production* defect, not a
service defect, and the fix was confined to one file in `calibrate`.

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
| service-side 4D, frequency at max *(with corrupted coefficients — see correction below)* | **+0.000000** | +1.5 |
| service-side 4D, E-cone at max *(with corrupted coefficients — see correction below)* | **+0.000000** | +1.5 |
| service-side 4D, E-clock at max *(with corrupted coefficients — see correction below)* | **+0.000000** | +1.5 |

The lower bound is fine; only the upper bound collapses. `699.999 → 0.000090` is the
load-bearing row: this is a whole-span failure, not an endpoint special case — see "The real
mechanism" above for why (the coefficient, not the basis function, was destroyed across the span).

**Points affected on a regular grid = the union of the three upper faces.** For the grid above,
332 of 1232 points (26.9%) — matching `1 − (6/7)(10/11)(15/16) = 26.95%` exactly. Any regular
measurement grid loses roughly this fraction of its correction **during fitting**, not during
service-side evaluation (see below).

## Correction — the service-side 4D interpolator was never defective

The three "service-side 4D" rows in the reproduction table above are real measurements, but they
were measured against an artifact whose **coefficients were already corrupted** by the fitter
defect described above — they do not show a defect in `evaluate_correction` itself.

`antenna-model/src/model/correction_interpolator.rs` uses the standard NURBS-book Cox-de Boor
recurrence (`basis[0] = 1.0`, then the triangular recurrence) — a **different algorithm** from
calibrate's naive recursive `bspline_basis` — paired with a `find_knot_span` that clamps to the
last valid span rather than doing a half-open bounds check. Verified by partition of unity with
coefficients built **by hand** (i.e. not produced by the fitter, so the corrupted-fitter failure
mode cannot leak in):

| query | `evaluate_correction` |
|---|---|
| interior | 1.000000000 |
| azimuth at max | 1.000000000 |
| elevation at max | 1.000000000 |
| frequency at max | 1.000000000 |
| temperature at max | 1.000000000 |
| all four at max simultaneously | 1.000000000 |

`evaluate_correction`'s apparent failure in the original diagnostic above was entirely
**inherited corrupted coefficients** from the broken fitter, propagated at artifact-export time —
not an independent defect in the interpolation code.

## Why it went unnoticed

1. **The two implementations share coefficients, and the coefficients were corrupted.** This is
   *not* "both algorithms were wrong in the same way" — see the correction above:
   `evaluate_correction` uses a different, correctly-clamping algorithm and evaluates a hand-built
   partition of unity exactly at every boundary. What actually happened:
   `calibrate/src/artifact_export.rs`'s round-trip test (`test_round_trip_matches_3d_evaluation`)
   compares the 3D `CorrectionSurface::evaluate` against the 4D `evaluate_correction` **using the
   same fitted coefficients** and asserts agreement to 1e-9 — which passes, because both read the
   same already-corrupted coefficient array and agree on what they see, not because both
   algorithms are defective. (`find_knot_span` in `correction_interpolator.rs:230-234` does carry
   a comment about matching `calibrate/src/correction_surface.rs::find_knot_interval` for a
   previous off-by-one, but that is a span-index convention match, not a shared basis-evaluation
   bug — the two span/basis implementations differ as shown above.)
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

## The fix

`a866cfb` corrects `bspline_basis`'s `k == 1` base case so the basis is a partition of unity at
the exact domain maximum, and removes reliance on the padding-index right-endpoint special case.
Pinned by three new tests: `basis_is_a_partition_of_unity_on_every_face_and_corner`,
`basis_is_continuous_up_to_the_maximum`, and
`a_fitted_constant_is_recovered_at_the_domain_maximum` (the last is the one that would have
caught the real, fitting-side damage — the first two only pin the basis function itself).

**A pre-existing golden test had to be re-pinned** (`e87efe6`). `fit_matches_openblas_golden`'s
values were captured back when only the *solver* changed (LAPACK `dgesv` → the in-house
Cholesky), so they encoded the buggy basis rather than testing against it; it is a solver-drift
guard, not a basis oracle. Its fixture reaches 8700 MHz, exactly the frequency knot maximum —
**36 of its 288 points** (only `i == 7` of the `8×6×6` loop, corrected from an initial 48 by
`41b7e94` after a review caught the count error). Re-pinned: `sum` 81.54 → 87.17, `c[last]`
**0.1490 → 0.5661** (3.8×), while `c[0]`, `c[1]`, and `c[mid]` barely moved — those three have
`i_freq != 4` so they never touch the frequency-max span, whereas `c[last]` (index 124) is the
single coefficient at `i_freq = 4`, the one that had been starved. The re-pin is justified
against three independent checks, not just accepted as whatever the new code emits:
`fit_satisfies_normal_equations`, `normal_equations_match_dense_reference`, and the genuine basis
oracle `a_fitted_constant_is_recovered_at_the_domain_maximum` (a constant is analytically exactly
representable, independent of any pinned number).

`41b7e94` also documents a pre-existing gap found in review while fixing the count above: a
**fully degenerate axis** (every knot equal) makes every basis function return 0 rather than 1,
since a zero-width span satisfies neither the half-open branch nor the new domain-maximum branch.
Not introduced or fixed by `a866cfb` — noted in `bspline_basis`'s doc comment. Currently
unreachable in the real pipeline because `generate_knot_vector` rejects degenerate ranges
upstream (see "Still open" below).

## Results after the fix

D12's end-to-end fixture (`calibrate/tests/cli_full_mode_e2e.rs`), re-measured and retargeted by
`c79d2cf`:

- **Corrected RMSE 0.9756 → 0.0058 dB — a 168× improvement.** Model-only (uncorrected) RMSE is
  1.3071 dB, unchanged. The correction now removes essentially all of the injected bias **at the
  measurement points**.
- **The four known-answer-recovery probe errors are unchanged: 0.5928 / 0.0934 / 0.0365 /
  0.0934 dB.** `BIAS_RECOVERY_TOLERANCE_DB` therefore stays at **0.65** — deliberately not
  tightened.

**Why the two diverged.** The probes are off-grid (e.g. 450 MHz lies between grid frequencies
400 and 500 MHz) and sit far from any upper edge, so their coefficients were never starved by the
defect and the fix had nothing to correct there. `corrected_rmse`, in contrast, covers all 288
fitted grid points, ~27% of which lie on an upper face and were directly affected. So **the
residual probe error was never the edge collapse** — it is overfitting from an underdetermined
fit: 960 coefficients (`(4+4)(6+4)(8+4)` for 4/6/8 knots at order 4) against 288 data points,
which lets the surface interpolate the fitted points almost exactly while oscillating between
them. This is the same underdetermination refuted as the *cause of the zero* in "What it is not"
above — it was never that, but it is real, remains unfixed, and is exactly what the near-zero
`corrected_rmse` (measured at fitted points) versus the much larger probe errors (measured
off-grid) now demonstrates directly.

## Still open

Two problems adjacent to the edge collapse were surfaced during triage and remain unfixed,
excluded from this fix's scope for stated reasons — plus the pre-existing degenerate-axis gap
noted above:

1. **`validate_knot_vector` does not check multiplicity.** `generate_adaptive_knots`
   (`correction_surface.rs:567`) can place knots at an axis's min or max, and
   `generate_knot_vector` clamps by prepending/appending `order` copies, yielding multiplicity
   **5** at a boundary for order 4 (see "What it is not" above for the concrete frequency-axis
   example). `validate_knot_vector` checks only length and non-decreasing order, never
   multiplicity. **Excluded here** because adding the check would *fail on the current adaptive
   knots* used by the shipped 4/6/8 configuration — fixing it also requires fixing
   `generate_adaptive_knots`' quantile placement, a separate piece of work.
2. **The data-sufficiency check tests the wrong quantity.** `validate_fitting_inputs`
   (`correction_surface.rs:1004`) requires `(spline_order + 1)³ = 125` points; the real
   requirement is the **coefficient count**, `∏(num_knots_axis + order)`, which for the shipped
   4/6/8 configuration is **960**. A run with 126–959 points is silently accepted and
   underdetermined, held together only by the ridge term. **Excluded here** because fixing it
   would make D12's 288-point fixture fail its own minimum outright — a design decision about
   fixture sizing, not a bug fix bundled into this one. **This is what still limits the recovery
   accuracy measured above** — the 0.5928 dB worst-case probe error is overfitting from
   underdetermination, not the edge collapse, and will not improve until this is addressed (or
   the fixture is grown well past 960 points).
3. **A fully degenerate axis** (all knots equal) makes every basis function return 0 rather than
   1, as noted in "The fix" above. Pre-existing, not introduced or fixed by `a866cfb`, and
   currently unreachable because `generate_knot_vector` rejects degenerate `max_val - min_val`
   ranges upstream before a degenerate vector can be built. Documented in `bspline_basis`'s doc
   comment; no `debug_assert!` added.

   **Follow-up (2026-07-30, D15 review):** the "unreachable" qualifier is specific to
   calibrate's 3D fitter path. The **boresight mode is a real producer of degenerate 4D axes**
   on a different code path: `fit_frequency_correction`
   (`calibrate/src/frequency_correction.rs`) builds its azimuth/elevation/temperature axes as
   `order` equal knots (`create_degenerate_knot_vector`), which fails
   `BSplineModel4D::validate`'s `len ≥ shape + order` check — and the service loader validates
   every artifact — so any boresight artifact whose residuals tripped the 0.5 dB
   frequency-correction threshold is **rejected at service load time**. A distinct defect from
   the one fixed here (a loud structural rejection, not a silent zero), pinned by
   `frequency_correction::tests::frequency_correction_is_rejected_by_the_service_side_validator`
   and routed to unit **D13** (with D2 owning the artifact framing).

**This fix does not make the correction-surface fit well-determined.** It corrects a basis
evaluation bug that was corrupting fitted coefficients at every axis maximum; it does not add
more constraints, more data, or a smaller coefficient count. Item 2 above is the concrete
mechanism by which the fit remains underdetermined today.
