# The correction surface's angular resolution is fixed while the pattern scale is not

**Filed 2026-08-02 by roadmap unit D14**, which is the first unit to fit a correction surface
to a *real* antenna's measured sidelobe structure. Roadmap work unit: **D21**.

> **Resolved 2026-08-04 by D21, taking option 2. Two things in the original filing below were
> wrong, and both are corrected in §"What D21 measured" at the end — read that before acting
> on the options table.** In short: the binding constraint on the cone axis is the knot
> *count* as much as the spacing floor, so "derive the floors from `λ/D`" (option 1) would on
> its own have changed nothing; and the clock axis, which this filing treats as the milder
> case, is the **worse-resolved of the two** by a factor of five. Option 1 is no longer
> described here as "the real fix" — see the last section for what would have to be true
> first.

## The finding in one line

`calibrate` ships one angular knot configuration for every antenna — 6 cone knots, minimum
spacing 2°, 8 clock knots, minimum spacing 5° — but the angular scale a pattern varies on is
`λ/D`, which across the antennas this repository already models spans **0.06° to 5.4°**. On a
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

*(The "option 1 as the real fix" half of that recommendation was **withdrawn** on 2026-08-04
— see below. Option 2 was taken.)*

---

## What D21 measured (2026-08-04)

Option 2 shipped: every full-mode fit now computes what its knots resolve against `λ/D`,
warns when it falls short, and records the figures in the artifact
(`CalibrationMetadata.angular_resolution`) and the `--metadata` sidecar. Two things in the
filing above turned out to be wrong.

### 1. The knot *count* binds, not only the spacing floor

The filing says "the binding constraint is `min_knot_spacing_econe = 2.0`, a constant". On
this fixture the six requested cone knots land on 2/4/6/8/10/12°, which *is* exactly the 2°
floor — but only because six knots over a 0–14° span happen to want that spacing anyway.
Lowering the floor alone would have moved nothing; `num_knots_econe = 6` binds at the same
point. **Option 1 as written — "derive the angular knot floors from `λ/D`" — would have had
no effect on this artifact.** A real option 1 has to raise the counts too, which is a
materially bigger change than the filing costs it at.

This is also why the shipped assessment reads the **delivered** knot vectors rather than
`CorrectionSurfaceParams`. Reading the requested floors would have reported the wrong number
on both axes, in opposite directions.

### 2. The clock axis is the worse of the two, by 5×

The filing treats the 5° clock floor as the milder case. Measured on this artifact:

| axis | delivered knot spacing | lobe period | knots per lobe period |
|---|---|---|---|
| cone | 2.00° (six knots, on the 2° floor) | 1.154° (`λ/D` at 12.2 GHz) | **0.577** |
| clock | **40.0°** (eight knots over 350°) | 4.770° | **0.119** |

The clock spacing is *eight times* its own 5° floor, because eight knots over a 350° axis is
what binds — the floor never engages at all. And the requirement is tighter than the cone
axis's, not looser: traversing φ at polar angle θ crosses an arc of `sin θ` in the pattern's
own angular scale, so `Δφ = (λ/D) / sin θ`, evaluated at the coverage edge. The clock axis
needs its finest resolution furthest off-axis and none at all on boresight — the opposite of
what a single absolute floor assumes.

"The coverage edge" is the largest `|θ|` the surface spans, not its largest signed cone
angle: measurements are valid over [-90, 90] and a one-sided cut may run entirely negative,
where the last knot is 0 and the signed maximum would report `sin θ = 0` — an infinite clock
lobe period and a "fully resolved" verdict on the axis this section exists to flag as the
worse one. `assess_angular_resolution` takes `max(|first knot|, |last knot|)`, clamped at 90°
where `|sin|` peaks, and two tests pin it against the mirrored and asymmetric spans.

That asymmetry is general, not a property of this fixture: the number of lobe periods around
the coverage edge is `2π sin θ_max / (λ/D)`, and resolving them needs twice that many clock
knots. Here that is **75.5 periods and ~151 knots, against the 8 shipped** — a factor of 19,
where the cone axis is short by a factor of 3.5.

### Why option 1 is no longer called "the real fix"

It is **unproven and currently untestable**, which is a different thing from deferred:

- **No data in this repository can validate it.** D14's grid is `model + a weighted
  least-squares quadratic residual trend per half-plane` — by construction it contains no
  lobe-scale residual structure. A finer surface fitted to it would recover nothing, because
  there is nothing finer in it to recover. The only lobe-scale evidence available is the 19
  digitized anchors, and those are the *test* set. Option 1 would ship more coefficients, a
  larger dataset requirement, and no measurement showing it helps.
- **It may be unreachable for the antennas that need it most.** Since D20 an underdetermined
  fit is a hard error. Deriving the floors from `λ/D` would make `calibrate` demand a 3D grid
  sampled at ~0.03° in cone for `dsn_34m` X-band — and D14's own register row records the
  maintainer-approved finding that full 3D G/T grids are essentially never published, judged
  a permanent constraint. The result would be `calibrate` refusing to produce an artifact at
  all for the narrow-beam ground stations D9 exists to ship: worse than the present state.
- **The structure may not be the surface's to carry.** Residual lobe structure at this scale
  is as plausibly a lobe/null *position* mismatch — feed position, phase centre, surface
  phase — as a level error. An additive smooth dB surface is the wrong instrument for a
  positional error at any resolution; that fix would belong in the physics/tuning layer.
  Nobody has measured which it is.

The two preconditions for revisiting it are therefore: **real measurements sampled finer than
`λ/D`**, and **evidence that the residual's lobe structure is a level error rather than a
position error**. Recorded as its own roadmap unit rather than left inside D21's closeout,
because neither is a coding task.

## Pointers

- Measurement and budget: `calibrate/tests/cli_full_mode_real_data_e2e.rs`
  (`ANCHOR_STRUCTURE_ALLOWANCE_DB`, `MIN_ANCHORS_IMPROVED`).
- The fill that deliberately injects only the representable trend:
  `calibrate/src/bin/cr159703_grid.rs`.
- The knot floors: `calibrate/src/main.rs::surface_fitting_params`,
  `calibrate/src/correction_surface.rs::CorrectionSurfaceParams::default`.
