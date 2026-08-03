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

## Pointers

- `calibrate/src/validator.rs::perform_cross_validation` (fold assignment).
- `scripts/generate-cr159703-artifact.sh` (reproduces the measurement above).
- D10's entry in `docs/roadmap-2026-07-work-units.md` (the previous two defects in this
  function).
