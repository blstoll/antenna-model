# Cross-validation assigns folds as contiguous slices of a grid-ordered file

**Filed 2026-08-02 by roadmap unit D14**, which ran `--validate` on a real-anchored full-mode
artifact and got a mean CV RMSE **164× worse** than the fit's own. Roadmap work unit: **D22**.

## The measurement

D14's artifact, 3240 points, 5-fold cross-validation
(`scripts/generate-cr159703-artifact.sh`):

| | RMSE (dB) |
|---|---|
| corrected RMSE (in sample) | **0.0272** |
| fold 1 | **10.0688** |
| fold 2 | 0.5600 |
| fold 3 | 0.1223 |
| fold 4 | 0.6436 |
| fold 5 | **10.8570** |
| reported mean ± σ | **4.4503 ± 4.9187** |

The shape is the whole finding: the two *edge* folds are two orders of magnitude worse than
the middle one.

## Mechanism

`validator.rs::perform_cross_validation` splits by position in the input vector:

```rust
let fold_size = n / num_folds;
let test_start = fold * fold_size;
let test_end = if fold == num_folds - 1 { n } else { (fold + 1) * fold_size };
```

Measurement files are written grid-ordered — frequency-major for both D12's fixture and
D14's generator, and any real instrument sweeping a pattern will produce something similar.
So a contiguous fold is not a random sample of the domain, it is a **slab of it**. With
3240 rows at 6 frequencies (540 rows each) and 5 folds (648 rows each), fold 1 holds out all
of 11 700 MHz plus part of 11 800, leaving the training set with *no data at the bottom of
the frequency axis at all*. Scoring it then measures the spline **extrapolating past its own
knots**, which it is not built to do and was never asked to do.

The interior folds show the same effect in weaker form (0.56 / 0.64 versus 0.12 for the
middle one): they remove a contiguous slab too, but the fit can interpolate across a hole.

## Why it matters

`num_folds` defaults to 5 and `--validate` is the documented way to check a calibration, so
the number this produces is the pipeline's headline quality claim on any artifact fitted from
an ordered file. As it stands that number is neither the surface's generalization error nor a
deliberate extrapolation test — it is a mixture whose proportions depend on how the *file* was
sorted. Re-sorting the same measurements changes it. It also reads as alarming when the
artifact is fine, which trains readers to ignore it.

This is the third defect found in this cross-validation path and the first that concerns *what
the folds are*: roadmap **D10** fixed the fold refit using `CorrectionSurfaceParams::default()`
instead of the artifact's own params, and the unbounded nested recursion in the same function.

## Options

1. **Strided assignment** — `i % num_folds == fold`. One-line change, deterministic, no RNG,
   and on a grid-ordered file every fold becomes a representative sample. Caveat worth stating:
   on a *dense* grid a strided fold's neighbours are all in the training set, so the estimate
   leans optimistic — it measures interpolation between adjacent samples, which is the right
   question for a correction surface and the wrong one for "would this generalize to a new
   antenna state".
2. **Seeded shuffle** — same statistical properties as option 1 with less structure-coupling,
   at the cost of carrying a seed (and the reproducibility contract that comes with it).
3. **Blocked/grouped CV, declared** — keep contiguous folds but say so, and report the metric
   as what it is: an axis-extrapolation stress test. Would want the block axis chosen
   deliberately rather than inherited from file order.
4. **Report per-fold, not just the mean.** Independent of 1–3: the report already carries
   `fold_rmse_values`, and the mean alone hid a 100× spread. Whatever assignment is chosen,
   a σ that large should be surfaced.

Recommended default: **option 1 plus option 4**. Option 3 is defensible but needs the axis to
be an explicit choice; inheriting it from row order is not a design.

## Not fixed under D14

Deliberately. It changes the reported CV figure on every artifact anyone has produced, and
which fold assignment is *correct* depends on what cross-validation is meant to measure here —
a maintainer decision, not a bug fix. D14 records the measurement, and
`scripts/generate-cr159703-artifact.sh` carries a comment so the exemplar's own alarming CV
numbers are not mistaken for the artifact's accuracy.

## ✅ Resolved 2026-08-03 (D22)

Maintainer took **option 1 + option 4**, as recommended, plus a third call the filing did not
have an option list for.

**Fold assignment is strided** — point `i` is held out by fold `i % K` — and the reason is in
the code, because the property that was missing was never "randomness" but *invariance to
which axis varies fastest*. Striding is deterministic, needs no seed, and gives every fold's
training set the full span of every axis. Its bias is stated rather than discovered: on a
dense grid every held-out point has training neighbours, so the score leans optimistic and
measures interpolation, which is the question a correction surface exists to answer. A
deliberate extrapolation test is still option 3 and would have to name its axis on purpose.

**Per-fold values are surfaced** in `format_summary`, not just the mean and σ. The mean alone
is what hid the 100× spread that made this finding hard to see.

**A fold that cannot be refitted is now recorded, not fatal.** This is the second defect
filed with the finding, and the answer was that aborting is worse than reporting: since D20 an
underdetermined fit is a hard error and a fold trains on `(1 − 1/folds)` of the data, so
`--validate` could *remove* an artifact that the same command without it produces — the
artifact's own fit having succeeded on the full set. `CrossValidationResults` now carries
`failed_folds`, the summary declares the run INCOMPLETE and names each failure with both point
counts, and the mean is taken over the folds that were actually scored. That last detail
matters on its own: averaging over `num_folds` would have made cross-validation report a
*better* number the less of it ran.

Re-measured on D14's artifact — see the table at the top for the before column, and
`calibrate/tests/cli_full_mode_real_data_e2e.rs::the_scripts_validated_run_produces_an_artifact`,
whose known-defect pin was inverted here as its own comment instructed.

### There were two cross-validations, and the first fix only reached one

Caught in review, same day. `validator::perform_cross_validation` is not the only k-fold
implementation: `correction_surface::cross_validate` is a second one, called from inside
`fit_correction_surface` whenever `cross_validation_folds > 1` — which
`main::surface_fitting_params` sets **straight from `--validate`**. So on the CLI path the
fit's own cross-validation runs *first*, and it still had both defects:

- It sliced folds contiguously, so a single `--validate` run reported two cross-validation
  numbers computed from two different partitions of the same data. Only the validator's was
  ever looked at.
- Its fold refit propagated with `?`. That made the non-fatal decision above **unreachable
  from the CLI**: an underdetermined fold killed the run inside the fit, before
  `validate_calibration` or the artifact writer was reached — the exact destructive
  `--validate` behaviour the decision was meant to remove.

Both copies now route through one shared definition, `correction_surface::is_held_out`,
which carries the rationale; and `cross_validate` returns `Option<f64>`, reporting an absent
figure rather than failing the fit. The lesson generalises past this function: *fixing the
implementation you found is not the same as fixing the behaviour you were asked to change* —
the question to ask is which copy the user's command actually reaches.

Three smaller defects from the same review, all in the D22 work:

- `fold_rmse_values` is dense (failed folds contribute no entry), so `format_summary`'s
  positional per-fold output relabelled the survivors — with fold 1 failing, fold 2's RMSE
  printed as fold 1's. Labels now come from `scored_fold_numbers()`.
- The aggregate statistics were `f64::NAN` when no fold scored. `serde_json` writes a
  non-finite float as `null` and a plain `f64` cannot read that back, so the `--report` JSON
  this pipeline writes would not have parsed. They are `Option<f64>` now.
- The strided-fold test re-implemented `i % num_folds != fold` inline, making it a test of the
  fixture rather than of the code — it would have passed against a reverted implementation. It
  calls the shared function now, and a behavioural sibling calls
  `perform_cross_validation` itself.

## Pointers

- `calibrate/src/validator.rs::perform_cross_validation` (fold assignment).
- `scripts/generate-cr159703-artifact.sh` (reproduces the measurement above).
- D10's entry in `docs/roadmap-2026-07-work-units.md` (the previous two defects in this
  function).
