# The correction surface's angular resolution is fixed while the pattern scale is not

**Filed 2026-08-02 by roadmap unit D14**, which is the first unit to fit a correction surface
to a *real* antenna's measured sidelobe structure. Roadmap work unit: **D21**.

## The finding in one line

`calibrate` ships one angular knot configuration for every antenna — 6 cone knots, minimum
spacing 2°, 8 clock knots, minimum spacing 5° — but the angular scale a pattern varies on is
`λ/D`, which across the antennas this repository already models spans **0.06° to 4.3°**. On a
small dish at a high frequency the surface cannot represent the residual it is being asked to
fit, and nothing in the pipeline says so: the in-sample RMSE stays excellent, because the
grid is sampled no finer than the knots either.

## Measurement

D14's fixture: the NASA CR-159703 1.22 m prime-focus dish at 12.1 GHz, `D/λ = 49.2`, lobe
period `λ/D = 1.16°`, HPBW ≈ 1.44°. Against the shipped 2° minimum cone knot spacing.

19 digitized envelope peaks (measured, from the report's pattern charts) versus the smoothest
curve that knot spacing can carry — a weighted least-squares quadratic in cone angle per
half-plane:

| half-plane | anchors | worst deviation from the representable trend |
|---|---|---|
| H+ (clock 0°) | 6 | 4.29 dB |
| E+ (clock 90°) | 3 | 0.50 dB |
| H− (clock 180°) | 5 | **8.42 dB** |
| E− (clock 270°) | 5 | 6.26 dB |

The deviations are not digitization noise — the rows carry ±0.5–1.5 dB uncertainties. They are
the real antenna's lobe-to-lobe variation: along the H− cut the measured envelope runs
−41.0 dB at 3.6°, −30.0 dB at 5.1°, −35.5 dB at 6.3° — an 11 dB swing across 1.5°, which is
one lobe period. A basis whose knots are 2° apart has no degree of freedom there.

Served consequence, measured end to end (`calibrate/tests/cli_full_mode_real_data_e2e.rs`):
the calibrated pattern reproduces the digitized peaks at **3.19 dB RMS**, against 11.58 dB for
the uncorrected model — a 3.6× improvement that is real and useful — but the two worst peaks
are still 8.03 and 6.55 dB out, and at those two the correction makes the answer *worse* than
raw physics.

## What is NOT the problem

- **Not the fit.** The correction surface reproduces what it was given to 0.027 dB RMSE over
  3240 points (960 coefficients — determined, post-D20).
- **Not the physics model.** Its envelope decays smoothly and is 7–17 dB below the measured
  envelope; that gap is what the correction is for, and the correction removes ~all of the
  trend part of it.
- **Not adaptive knot placement.** Placement is already at data quantiles and strictly
  interior (D19). The binding constraint is `min_knot_spacing_econe = 2.0`, a constant.

## Why the pipeline cannot currently notice

Three properties conspire:

1. `min_knot_spacing_econe` / `_eclock` are **absolute angles**, hardcoded in
   `main.rs::surface_fitting_params` and `CorrectionSurfaceParams::default`. Nothing derives
   them from `λ/D`, and neither the fitter nor the exporter sees the antenna's diameter.
2. **In-sample RMSE cannot see it.** A measurement grid sampled at 1–2° in cone carries no
   structure finer than that either, so the fit reproduces its own data and reports an
   excellent number. Only a comparison against something *off* the grid — D14's digitized
   peaks — exposes the gap.
3. The **existing guard points the other way**: D12's fixture comment records that
   `GroundStation_13m`'s sub-degree main lobe "the fitter's 2° minimum E-cone knot spacing
   cannot represent", and the response was to choose a *broader-beam antenna for the fixture*.
   That was right for D12's purpose and it left the constant unexamined.

## Scope: which antennas this reaches

`λ/D` for the classes and antennas already in the tree, against the 2° cone floor:

| antenna | band | λ/D | knots per lobe period |
|---|---|---|---|
| `UHF_Array_Element` (8 m) | 400–700 MHz | 5.4°–3.1° | 2.7–1.5 |
| CR-159703 (1.22 m) | 12.1 GHz | 1.16° | **0.58** |
| `gs_3.7m` | X-band | 0.58° | **0.29** |
| `dsn_34m` | X-band | 0.06° | **0.03** |

Only the broad-beam UHF class is comfortably resolved. Every real ground-station geometry is
under-resolved by at least 3×, and the calibration data for those antennas would have to be
sampled at least that finely to notice.

## Options

1. **Derive the angular knot floors from `λ/D`** — e.g. at least 2 knots per lobe period,
   `min_knot_spacing_econe ≈ 0.5·λ/D` with the current values as an upper bound. Requires the
   fitter to know the antenna geometry, which the exporter already has and the fitter does not.
   Raises the coefficient count fast (each halving of the cone floor roughly doubles it), so it
   is coupled to D20's data-sufficiency requirement: a finer surface demands a denser dataset,
   and the fitter now refuses rather than pretending.
2. **Keep the floors and report the mismatch.** Compute the achievable resolution at fit time
   and warn — or refuse — when the requested knots cannot resolve `λ/D`. Cheap, honest, and it
   converts a silent limitation into a stated one; it does not make any artifact better.
3. **Do nothing, document the envelope claim.** State that the correction surface carries the
   *envelope trend* of the residual, not its lobe structure, and that per-lobe accuracy at
   `λ/D < 2·knot spacing` is not offered. This is what D14's test budgets against today.

Recommended default: **option 2 now, option 1 as the real fix**, in that order. Option 2 is
small and makes every future artifact self-describing; option 1 changes what the pipeline can
promise but needs a maintainer decision on the coefficient-count/dataset-size trade it forces.

## Pointers

- Measurement and budget: `calibrate/tests/cli_full_mode_real_data_e2e.rs`
  (`ANCHOR_STRUCTURE_ALLOWANCE_DB`, `MIN_ANCHORS_IMPROVED`).
- The fill that deliberately injects only the representable trend:
  `calibrate/src/bin/cr159703_grid.rs`.
- The knot floors: `calibrate/src/main.rs::surface_fitting_params`,
  `calibrate/src/correction_surface.rs::CorrectionSurfaceParams::default`.
