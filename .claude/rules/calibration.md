---
description: Calibration pipeline, artifact schema/container version axes, and wire-format rules for calibrate and the artifact data layer.
paths:
  - "calibrate/**"
  - "antenna-core/src/data/**"
  - "calibration_data/**"
  - "scripts/generate-cr159703-artifact.sh"
---

# Calibration and artifact rules

## The invariant that has cost the most

**Every parameter the fitting model uses must be in the artifact, or the service serves a
different antenna than the residuals describe.** C13 and D23 were the same defect two lines
apart in `export_physical_params`.

- **C13:** `physical_config.feed.position` is the feed's design offset *from the focal point*,
  not its vertex-origin position — an on-axis feed is `(0, 0, 0)`. The service adds it to a
  steering position that is already vertex-origin, so the other reading places the feed at
  `z ≈ 2f`. Full-mode `calibrate` did exactly that until 2026-08-02, costing **27.3 dB** of
  boresight gain on the first artifact ever served.
- **D23:** `feed.asymmetry_factor`. `calibrate` fits against the antenna class's value, the
  artifact had no field for it, and `FeedParametersBuilder` defaulted the service to 1.0 — so a
  residual surface fitted against an asymmetric illumination was applied on top of a symmetric
  one, and the evaluation silently moved off the azimuthal-mode integrator branch onto the
  symmetric one. Worst measured **1.20 dB**, but **0.0003 dB at boresight** — a φ-dependent
  error, which is exactly why C13's boresight-focused pass over the same function missed it.
  Asymmetry is a **declared** design property, deliberately not a tuned one: it is horn
  geometry, and boresight data carries no information about it.

Each producer has its own round-trip guard, and the served-path guard in `service::evaluator`
carries a negative control against the symmetric default.

### Unguarded, same shape: `--antenna-class` (filed as roadmap D32)

`calibrate --antenna-class` **defaults to `DSN_34m`** and nothing validates the choice. The
class supplies diameter, f/D, surface RMS, mesh, feed q-factor, phase-center offset,
`asymmetry_factor` and system noise temperature, so calibrating a 3.7 m dish without passing
the flag fits and exports a 34 m antenna's physics under the measured antenna's ID. The only
failure mode is a *typo* — an unknown name errors, a valid-but-wrong name succeeds silently.
There is no cross-check against `--antenna-id`, the measurement data, or `--design-specs`.

**It cannot be caught downstream, and it leaves no trace.** The correction surface is fitted
to residuals, so the wrong class's systematic error is absorbed into the surface and in-sample
RMSE stays excellent (the D20 lesson). `CalibrationMetadata.antenna_class` exists for exactly
this and `export_full_calibration` never sets it, so full-mode artifacts carry `None`;
`PhysicalAntennaConfig` has no id field either. **Always pass `--antenna-class` explicitly,
and check it against the antenna being measured before trusting an artifact.** Filed as
roadmap **D32**; until it closes, this is the only thing standing between a wrong class and a
wrong artifact.

## Artifact wire format

An artifact is an `AntennaCalibration` encoded with **postcard** (documented, versioned wire
format), wrapped in the ANTC header (magic + version + CRC32 + length).
`artifact_export::write_calibration_artifact` is the tool's **only** artifact writer, shared by
full and boresight mode (D2).

**Two version axes; the loader enforces both:**

- **Container** — the ANTC header `u32` (`ANTC_ARTIFACT_VERSION` = **4**), readable before the
  decode.
- **Schema** — `metadata.format_version` (`CALIBRATION_SCHEMA_VERSION` = **"5.0"**), readable
  only after it. A foreign MAJOR is a hard error.

Three recent bumps cover the three cases, and they moved the axes differently:

- **D21 (5.0 / container 4)** added `metadata.angular_resolution` and **fixes no wrong number
  at all** — every 4.0 artifact means what it said and no consumer reads the new field. But
  postcard is positional, so a 4.0 payload is short by the `Option` discriminant and everything
  after it decodes from the wrong offset. *Layout*, not correctness.
- **C13 (3.0)** was *meaning-only*: not a byte of layout moved, but what `feed.position`'s three
  `f64`s meant changed, which no consumer could detect. Schema axis rejected pre-3.0; container
  stayed at 2.
- **D23 (4.0)** was a *layout* change, so it bumped **both**: a 3.0 payload is one `f64` short
  and postcard reads positionally, so the decode itself is untrustworthy and only the container
  stamp is readable early enough to say so.

**Rules:**

- **Producers must stamp the constant, never a literal.** Three carried `"2.0"` by hand and
  would have drifted straight past the bump; D23 found a *fourth* hand-rolled ANTC writer in a
  test carrying a literal `2u32`.
- **There is exactly one definition of the framing** (D27):
  `antenna_core::data::loader::encode_calibration_artifact`, beside the loader that reads it.
  `write_calibration_artifact` wraps it and adds file I/O; test helpers use it too. **Do not lay
  the header out by hand** — that is how the repo accumulated a fourth copy and then a fifth
  that wrote no header at all.
- **ANTC framing is required on load.** The legacy headerless fallback is gone.
- **Do NOT add `#[serde(skip_serializing_if)]` / `skip` / `flatten` to any serialized
  calibration type** — postcard is positional and non-self-describing, so those attributes
  silently corrupt the format. See the note atop `data/types.rs`.

Read `data/loader.rs`'s module docs and `docs/calibration-workflow-guide.md` §10.5.1 before
touching either.

## No `.bin` artifacts ship in-repo (D9, decided 2026-08-16)

Decided, not incidental: an artifact is a build output derived from measurements *plus* this
codebase's physics model, so a committed one goes stale when `PHYSICS_MODEL_VERSION` moves
(4 → 9 in one month) and nothing in the build can notice. The `antennas.yaml` entries that
reference a `.bin` file are `enabled: false` **templates**; the uncalibrated design-spec
antennas are `enabled: true` and load with no artifact, so a clean checkout starts healthy.
**Count the entries rather than quoting a doc — that count has gone stale twice.**

The worked generation path is `scripts/generate-cr159703-artifact.sh` (D14): committed inputs
→ generated grid → `calibrate` → `.bin`, written outside the repo tree and never committed.

## The pipeline

### 1. Parse CSV (`parser.rs`)

E-clock/E-cone are spherical coordinates about boresight, and the parser puts every row in the
**polar convention** on the way in (`MeasurementPoint::to_polar_convention`, D26). A negative
E-cone is legal recorded input — a one-sided pattern cut on a fixed clock plane — but `(φ, −θ)`
names the same direction as `(φ + 180°, θ)`, and only the second form is consumable. The served
elevation is a polar angle from boresight and is **never negative**, so a correction surface
fitted on a signed cone axis is unreachable on the served path.

Before D26 a silent clamp in `export_full_calibration` (`el_lo = min.max(0.0)`) collapsed a
`-14°…0°` cut to `(0.0, 0.0)`: the artifact then reported `is_boresight_only()` over thousands
of measurements and the service applied **no correction at all**, with every other health signal
normal — the D13 signature, "every observable healthy except the one nobody asserted". The
reflection is physics-preserving (measured 3.2e-6 dB), and the export now **refuses** an
out-of-convention extent rather than clamping it: a clamp cannot tell "already correct" from
"silently truncated", which is how this survived.

### 2. Tune parameters (`parameter_tuner.rs`)

Nelder-Mead simplex over physical parameters (surface RMS, mesh spacing, wire diameter). Search
bounds come from `ParameterBounds::from_class` — a multiplicative bracket around each antenna
class's own nominal, **not** a fixed global range (D16).

The objective must be evaluated under the same `IntegrationParams` as
`main.rs::compute_model_predictions` (`default()`), or the tuner optimizes against integrator
discretisation error rather than the physics — see
`docs/findings-2026-07-30-full-mode-parameter-tuning-broken.md` defect 4.

Both per-measurement-point physics sweeps — this objective and `compute_model_predictions` — are
parallel over points via rayon (D18 task 3). Each parallel site **collects into an index-ordered
`Vec` and reduces serially**, so every reported number is bit-for-bit what the serial code
produced. **Do not replace those reductions with a parallel `sum()`/`reduce()`** — f64 addition
is not associative, and this crate's known-answer tests and D13's real-data tolerances pin
measured constants to four decimals.

### 3. Fit the correction surface (`correction_surface.rs`)

B-spline/RBF fitted to residuals (measured − physics). **The correction surface is *residual*,
not absolute gain.**

- **The data requirement is the coefficient count `∏(placed_knots_axis + order)`, not the
  `(spline_order+1)³ = 125` pre-check** (D20). An underdetermined fit is a hard
  `UnderdeterminedFit` error, checked *after* knot generation because the knot counts in
  `CorrectionSurfaceParams` are a *request* that interior-only placement and minimum-spacing can
  reduce. Full mode's shipped 4/6/8 counts declare up to 960 coefficients, so a full-mode dataset
  needs ≥960 points — and ≥1440 if a 3-fold cross-validation must pass, since the *training
  split* is what must cover them. Switching this check on failed 24 tests that had been fitting
  underdetermined surfaces and reporting excellent RMSE: such a fit interpolates its own data
  points almost exactly while oscillating between them. **Size a test fixture to the coefficient
  count of the params it fits, and to the tightest CV fold it runs — not to 125.**
- **Adaptive knots are strictly interior** (D19): a knot equal to an axis bound became
  multiplicity `order+1` after clamping, giving that basis function zero-width support, so 37.5%
  of the shipped configuration's coefficients were attached to identically-zero functions.
  `validate_knot_vector` enforces end multiplicity `== order` and interior `<= order-1`.
- **The angular knots are absolute while the pattern scale is `λ/D`** — 0.06°–5.4° across the
  antennas in this tree — so on anything but a broad-beam antenna the surface carries the
  residual's *envelope trend*, not its lobe structure, and **in-sample RMSE structurally cannot
  see the difference**. Measured 8.42 dB of unrepresentable lobe-scale structure on a 1.22 m dish
  at 12.1 GHz. Since D21 every full-mode fit says so: `assess_angular_resolution` compares the
  *delivered* knot spacing to `λ/D`, `calibrate` warns when it falls short, and the figures ride
  in `CalibrationMetadata.angular_resolution` and the `--metadata` sidecar.

Three things to know before touching the assessment: (1) it reads the **delivered** knot
vectors, never `CorrectionSurfaceParams` — the requested floors are wrong in both directions,
and the knot *count* binds at least as often as the spacing floor. (2) **The clock axis is the
worse of the two, by 5×**, and its requirement *tightens* off-axis: traversing φ at polar angle
θ crosses an arc of `sin θ`, so `Δφ = (λ/D)/sin θ` — the opposite of what an absolute floor
assumes. (3) `MIN_KNOTS_PER_LOBE_PERIOD = 2.0` is **derived** (Nyquist), not fitted, which is why
it carries no margin test.

D26 hardened the assessment against input it cannot measure: `widest_knot_gap` returns `Result`
and refuses an empty, degenerate or non-finite axis instead of reporting a number for it — a NaN
gap used to be *discarded* by `fold(0.0, f64::max)`, reporting a **better**-resolved verdict out
of corrupt input. `f64::INFINITY` now has exactly one meaning in `AngularResolution` (no clock
structure to resolve, on the `sin θ → 0` path). `AngularResolution::validate` — called from
`AntennaCalibration::validate` — refuses such an artifact at load.

**The shipped knot configuration has one owner, `CorrectionSurfaceParams::shipped()`**; it
existed as three hand-copies, so a test could describe a shape nothing ships. And the assessment
is derived **inside** `export_full_calibration` from the same `diameter_m` it stamps — it used to
be a parameter while the caller read the diameter independently, so an artifact could describe
one dish in `diameter_m` and another in `angular_resolution` (the C13/D23 invariant again).

Whether the limitation can be lifted is open, filed as **D24**: the obvious fix — derive the
knots from `λ/D` — is untestable here and would make `calibrate` *refuse* every narrow-beam
antenna under D20's sufficiency check. See
`docs/findings-2026-08-02-correction-surface-angular-resolution.md`.

### 4. Validate (`validator.rs`)

Cross-validation; ensure <1 dB error in main lobe / first sidelobe.

**Folds are strided — point `i` is held out by fold `i % K`** (D22), through the single shared
definition `correction_surface::is_held_out`. **There are two k-fold implementations** and both
must use it: `validator::perform_cross_validation`, and `correction_surface::cross_validate`
inside `fit_correction_surface` — the latter is the one `--validate` reaches *first*, because
`main::surface_fitting_params` sets `cross_validation_folds` straight from the flag. D22's first
cut fixed only the validator, which left the decided behaviour unreachable from the CLI.

Folds used to be contiguous slices of the input file, so on a grid-ordered measurement set the
edge folds held out a whole axis slab and scored an *extrapolation*: 10.07 / 0.56 / 0.12 / 0.64 /
10.86 dB against an in-sample 0.027 dB — a headline `--validate` number that changed if you
re-sorted the same measurements. Striding is deterministic and invariant to which axis varies
fastest; its known bias is the opposite one (optimistic on a dense grid).

**Read `fold_rmse_values`, not just the mean** — the mean alone hid a 100× spread, and
`format_summary` prints every fold for that reason. A fold whose training split cannot be fitted
is **recorded and reported, not fatal**: since D20 an underdetermined fit is a hard error and a
fold trains on `(1 − 1/folds)` of the data, so aborting made `--validate` *remove* an artifact
the same command without it produces. See
`docs/findings-2026-08-02-cross-validation-fold-assignment.md`.

### 5. Serialize

See "Artifact wire format" above.

## Validity ranges

Queries outside calibrated ranges generate warnings but still return values (extrapolated).
The artifact's validity/coverage elevation ranges are polar-angle ranges (see D26 above).

## No system BLAS — the build is pure Rust

`cargo build` / `cargo test` need no environment variables, no Homebrew packages, and no system
libraries on any platform. **Do not add `LDFLAGS`/`CPPFLAGS`, and do not reintroduce
`ndarray-linalg`/OpenBLAS.** The fit exploits the B-spline's local support to accumulate the
normal equations `(BᵀB + λI)` directly from the `order³` non-zero basis values per data point,
then solves the SPD system with an in-house Cholesky factorization — dependency-free and
substantially cheaper than the dense `BᵀB` product it replaced.
