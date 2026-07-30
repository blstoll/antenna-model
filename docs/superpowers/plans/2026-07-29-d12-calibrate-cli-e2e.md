# D12 — calibrate CLI end-to-end test on perturbed-truth synthetic data

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first test that runs the actual `calibrate` binary through parse → predict → fit → validate → artifact, asserting it recovers a deliberately injected known truth — and, along the way, gate full-mode step 6's cross-validation on `--validate`.

**Architecture:** A deterministic generator (test-support code under `calibrate/tests/`) builds a measurement CSV by evaluating the *real* physics model with **perturbed** parameters plus a **closed-form injected bias**, so residuals are non-zero and their shape is known. The CLI then calibrates the *nominal* class against that data. Assertions are known-answer: the correction surface must recover the injected bias, the artifact must load through the **service's** loader (not calibrate's own code), and `--cv-folds` must visibly move the report. No RNG anywhere — the generator is a pure function of its inputs.

**Tech Stack:** Rust, Cargo integration tests (`env!("CARGO_BIN_EXE_calibrate")`), `tempfile`, `serde_json`, the `antenna-model` crate (already a `calibrate` dependency).

**User decisions (already made):**
- "D10 + D11 first, then D12, then D13/D14 (both also behind D2)" — maintainer, 2026-07-29. D10 and D11 landed in PR #26; D12 is next.
- Finding #1 (full-mode step 6 runs the outer CV ungated by `--validate`) is folded into this unit rather than shipped standalone — maintainer, this session.
- Finding #3 (`create_sample_csv`'s unrealistically shallow rolloff) is in D12's scope via the generator; finding #2 (`detect_outliers` on raw G/T) is explicitly **held** for before D13/D14.
- D12's own spec: default CI variant runs **without** `--tune-parameters`; measure the tuned run before deciding whether it needs `#[ignore]`.

---

## Facts established during planning (do not re-derive)

These were measured on this branch on 2026-07-29. They are inputs to the task design.

**1. Physics-model runtime is a non-issue at fixture scale.** `compute_g_over_t` with `IntegrationParams::default()`, measured per point:

| class | freq | release | debug |
|---|---|---|---|
| `TestAntenna_1m` | 8400 MHz | 0.36–1.5 ms | ~2.0 ms avg (4.0 ms worst, θ=90°) |
| `GroundStation_13m` | 4000 MHz | 0.36–1.5 ms | ~1.9 ms avg |
| `UHF_Array_Element` | 450 MHz | 0.28–1.1 ms | ~1.3 ms avg (2.2 ms worst) |

Debug is only ~2.3× release. A 250-point fixture costs ~250 evals in the generator plus ~250 in the CLI ≈ **under 1 second in debug**. No `#[ignore]` is needed for the untuned variant.

**2. `UHF_Array_Element` is the right fixture class.** Its beam is broad enough for the fitter's knot-spacing floors to resolve. Measured G/T at 450 MHz: 8.91 dB/K at 0°, 8.67 at 1°, 7.93 at 2°, 2.55 at 5°, −19.75 at 10°, −41.53 at 20°, −55.75 at 45°. Compare `GroundStation_13m` at 4000 MHz, which falls 33.08 → −10.60 dB/K between 0° and 1° — a sub-degree main lobe that a 2° minimum E-cone knot spacing **cannot** represent. Do not use a narrow-beam class.

The UHF numbers also cross −20 dB/K by ~10°, so a cone grid reaching 20°+ naturally contains rows the pre-D11 parser would have discarded. That is what makes this fixture D11's standing pin.

**3. `q_factor` is NOT tunable in full mode.** `TunableParameters` (`calibrate/src/antenna_config.rs:92-101`) carries only `surface_rms_mm`, `mesh_spacing_mm`, `mesh_wire_diameter_mm`. The D12 spec's suggestion to perturb "surface RMS and feed q-factor" is only half-recoverable: a q-factor perturbation can never be recovered by the full-mode tuner (unlike boresight mode, whose result type does carry `q_factor`). **Perturb `surface_rms_mm` only** for the tuner-recovery assertion; express everything else as the injected bias, which the correction surface absorbs.

**4. Fitter constraints the fixture must satisfy** (`calibrate/src/correction_surface.rs`, `main.rs::surface_fitting_params`):
- ≥ `(spline_order+1)³` = **125 points** after parsing, and the per-fold training set must also clear 125 when `--validate` runs a 5-fold CV (so ≥ ~157 points).
- Knot spacing floors: **50 MHz** frequency, **2°** E-cone, **5°** E-clock. Artifact knot counts are 4/6/8, so the fixture needs roughly ≥ 4×50 MHz frequency span, ≥ 6×2° cone span, ≥ 8×5° clock span.
- **Never loosen these to make a fixture fit — size the fixture to the fitter.**

**5. Relevant APIs.**
- Service loader: `antenna_model::data::loader::load_calibration_artifact(path) -> Result<AntennaCalibration, DataError>` (`antenna-model/src/data/loader.rs:44`).
- 4D correction evaluation: `antenna_model::model::evaluate_correction(&BSplineModel4D, azimuth_deg, elevation_deg, frequency_mhz, temperature_k) -> Result<CorrectionResult>` (`antenna-model/src/model/correction_interpolator.rs:85`). **Two traps here.** (a) The parameters are named *azimuth/elevation*, but the 3D→4D bridge maps **clock → azimuth, cone → elevation** — see the comment at `calibrate/src/artifact_export.rs:482`. Pass clock first, cone second. (b) It returns a `CorrectionResult` struct, not an `f64`; the value is `.correction_db`.
- `AntennaCalibration.correction_surface` is `Option<BSplineModel4D>` (`antenna-model/src/data/types.rs:62`).
- **Surface RMS on the artifact is in millimetres**: `AntennaCalibration.physical_config.reflector.surface_rms_mm` (`antenna-model/src/data/types.rs:362`). Note this is *not* the same unit as the model-layer `ReflectorGeometry` that `compute_model_predictions` builds, which takes metres — the data-layer type stores mm. No conversion is needed when reading it back from an artifact.
- Sidecars: `--metadata` writes `ArtifactMetadata`, `--report` writes `ValidationReport`, both as pretty JSON (`calibrate/src/sidecar.rs:78-95`).
- `calibrate` currently has exactly one dev-dependency, `tempfile`. `serde_json` is a regular dependency and is usable from tests.

**6. Full-mode CLI surface** (`calibrate/src/main.rs`): `--calibration-mode full --input --output --antenna-id --antenna-class --classes-file --validate --cv-folds --tune-parameters --max-tuning-iterations --report --metadata`. `--classes-file` defaults to `calibrate/antenna_classes.yaml`, which is **relative to the process CWD** — an integration test's CWD is the crate root (`calibrate/`), so the test must pass `--classes-file antenna_classes.yaml` or an absolute path. Getting this wrong is the most likely first failure.

---

## File structure

| File | Responsibility |
|---|---|
| `calibrate/tests/support/mod.rs` (create) | Deterministic fixture generator: perturbed-truth CSV writer + the injected-bias closed form. Shared by D12 now and D13/D14 later. |
| `calibrate/tests/cli_full_mode_e2e.rs` (create) | The CLI end-to-end test: runs the binary, asserts recovery, artifact loadability, and CV behavior. |
| `calibrate/src/main.rs` (modify) | Finding #1: gate step 6's cross-validation on `--validate`. |
| `docs/roadmap-2026-07-work-units.md` (modify) | Mark D12 done; record finding #1's resolution and the measured numbers. |
| `docs/roadmap-2026-07.md` (modify) | Retire the "no CLI-level integration test" half of the calibration risk; correct the stale Phase 3 / C7 text. |

Task order matters: **Task 1 (finding #1) lands before the e2e test** so the test is written against the corrected behavior, not the buggy one — the same rule that put D10/D11 ahead of this unit.

---

### Task 1: Gate full-mode cross-validation on `--validate`

**Goal:** `--validate` controls whether cross-validation runs in step 6, matching its documented behavior and step 5's existing gating.

**Background:** `--validate` is documented as "Run cross-validation after fitting" (`main.rs:108-110`). Step 5 honors it — `surface_fitting_params(args.validate, args.cv_folds)` sets `cross_validation_folds: if validate { cv_folds } else { 0 }`. Step 6 does not: `validation_config(args.cv_folds, &surface_params)` sets `num_folds: cv_folds` unconditionally, and `validate_calibration` runs CV whenever `num_folds > 1`. Since `--cv-folds` defaults to 5, **every** full-mode run cross-validates, printing "Performing 5-fold cross-validation" on a run that asked for neither. Step 6's other work (RMSE, main-lobe/sidelobe stats, outliers, band analysis) must still run unconditionally — only the CV part is gated.

**Files:**
- Modify: `calibrate/src/main.rs` — `validation_config` (currently ~line 192) and its call site in `run_calibration` (~line 567)
- Modify: `calibrate/src/main.rs` — `#[cfg(test)] mod tests` at end of file

**Acceptance Criteria:**
- [ ] `validation_config` takes `validate: bool` and sets `num_folds` to `0` when it is false
- [ ] With `--validate`, `num_folds` is `args.cv_folds` (unchanged behavior)
- [ ] Non-CV validation output (corrected RMSE, main-lobe/sidelobe stats, outliers) is unaffected in both cases
- [ ] Existing `cv_folds_reaches_the_fit_and_the_validator` still passes, updated for the new signature

**Verify:** `cargo test -p calibrate --bin calibrate` → all tests pass, including the two new assertions

**Steps:**

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the end of `calibrate/src/main.rs`:

```rust
    /// `--validate` is documented as "Run cross-validation after fitting". Step 5 honors
    /// it; step 6 did not, so every full-mode run cross-validated whether asked or not.
    #[test]
    fn cross_validation_is_gated_on_the_validate_flag() {
        let params = surface_fitting_params(false, 5);
        assert_eq!(
            validation_config(false, 5, &params).num_folds,
            0,
            "without --validate, step 6 must not cross-validate"
        );

        let params = surface_fitting_params(true, 5);
        assert_eq!(
            validation_config(true, 5, &params).num_folds,
            5,
            "with --validate, --cv-folds still sets the fold count"
        );
    }

    /// Gating CV must not disable the rest of step 6.
    #[test]
    fn gating_cross_validation_leaves_the_other_validation_settings_intact() {
        let params = surface_fitting_params(false, 5);
        let ungated = validation_config(false, 5, &params);
        let gated = validation_config(true, 5, &params);

        assert_eq!(ungated.main_lobe_target_db, gated.main_lobe_target_db);
        assert_eq!(
            ungated.first_sidelobe_target_db,
            gated.first_sidelobe_target_db
        );
        assert_eq!(ungated.outlier_threshold_db, gated.outlier_threshold_db);
        assert_eq!(ungated.main_lobe_beamwidths, gated.main_lobe_beamwidths);
        assert_eq!(
            ungated.first_sidelobe_max_deg,
            gated.first_sidelobe_max_deg
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p calibrate --bin calibrate cross_validation_is_gated 2>&1 | tail -20`

Expected: compile error — `validation_config` takes 2 arguments, not 3.

- [ ] **Step 3: Change the signature and gate the fold count**

In `calibrate/src/main.rs`, replace the `validation_config` function:

```rust
/// Validation settings for full-mode step 6.
///
/// `correction_params` **must** be the params the surface being validated was fitted with.
/// Passing `CorrectionSurfaceParams::default()` here (the pre-D10 behavior) made every
/// cross-validation fold refit a markedly more flexible surface — roughly double the knots
/// at 1000× weaker regularization — so the reported CV RMSE described a model family more
/// prone to overfit than the artifact being blessed.
///
/// `num_folds = 0` disables cross-validation only; every other check in step 6 (RMSE,
/// main-lobe and first-sidelobe statistics, outliers, band analysis) runs regardless.
/// Gating it on `--validate` matches the flag's documented meaning ("Run cross-validation
/// after fitting") and step 5, which already honors it — before this, `--cv-folds`' clap
/// default of 5 meant every full-mode run cross-validated whether asked to or not.
fn validation_config(
    validate: bool,
    cv_folds: usize,
    surface_params: &CorrectionSurfaceParams,
) -> ValidationConfig {
    ValidationConfig {
        num_folds: if validate { cv_folds } else { 0 },
        main_lobe_beamwidths: 1.0,
        first_sidelobe_max_deg: 5.0,
        frequency_bands: vec![], // Use default bands
        main_lobe_target_db: 1.0,
        first_sidelobe_target_db: 1.0,
        outlier_threshold_db: 3.0,
        correction_params: surface_params.clone(),
    }
}
```

- [ ] **Step 4: Update the call site**

In `run_calibration`, change:

```rust
    let validation_config = validation_config(args.cv_folds, &surface_params);
```

to:

```rust
    let validation_config = validation_config(args.validate, args.cv_folds, &surface_params);
```

- [ ] **Step 5: Update the existing test for the new signature**

In the same test module, `cv_folds_reaches_the_fit_and_the_validator` currently calls `validation_config(7, &with_validate)`. Replace that test body with:

```rust
    /// `--cv-folds N` reaches both the surface fit and the validation fold count, and
    /// cross-validation stays off entirely without `--validate`.
    #[test]
    fn cv_folds_reaches_the_fit_and_the_validator() {
        let with_validate = surface_fitting_params(true, 7);
        assert_eq!(with_validate.cross_validation_folds, 7);
        assert_eq!(validation_config(true, 7, &with_validate).num_folds, 7);

        let without_validate = surface_fitting_params(false, 7);
        assert_eq!(without_validate.cross_validation_folds, 0);
        assert_eq!(validation_config(false, 7, &without_validate).num_folds, 0);
    }
```

`validation_config_scores_the_surface_that_ships` also calls `validation_config(5, &surface_params)` — update it to `validation_config(true, 5, &surface_params)`. Nothing else in its body changes.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p calibrate --bin calibrate`
Expected: PASS, 4 tests.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add calibrate/src/main.rs
git commit -m "fix: gate full-mode cross-validation on --validate

--validate is documented as \"Run cross-validation after fitting\". Step 5
honored it; step 6 did not, so with --cv-folds defaulting to 5 every full-mode
run cross-validated whether asked or not, printing \"Performing 5-fold
cross-validation\" on a run that requested neither.

validation_config now takes the flag and sets num_folds to 0 when it is false.
Only cross-validation is gated: corrected RMSE, main-lobe and first-sidelobe
statistics, outliers and band analysis run unconditionally as before.

Filed as finding 1 of PR #26 (D10), out of charter there; folded into D12 so
the CLI end-to-end test is written against the corrected behavior.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: Deterministic perturbed-truth fixture generator

**Goal:** A test-support module that writes a measurement CSV containing known-truth data — generated from the real physics model with a perturbed surface RMS plus a closed-form injected bias — and exposes the injected truth so tests can assert recovery.

**Background:** If measurements are generated from the *same* configuration being calibrated, residuals are identically zero, the correction surface fits nothing, and the test asserts nothing. The generator therefore evaluates the model at a **perturbed** `surface_rms_mm` and adds a smooth bias, so calibrating the *nominal* class produces residuals whose shape is known in closed form.

The bias must be smooth at the scale of the knot spacing (50 MHz / 2° / 5°) or the spline genuinely cannot represent it and the recovery assertion fails for legitimate reasons.

**Files:**
- Create: `calibrate/tests/support/mod.rs`

**Acceptance Criteria:**
- [ ] Generating twice produces byte-identical CSV (no RNG, no time, no HashMap iteration order)
- [ ] The grid yields ≥ 200 rows, so a 5-fold CV training split still clears the fitter's 125-point minimum
- [ ] Frequency / cone / clock spans exceed the knot-spacing floors: ≥ 200 MHz, ≥ 12°, ≥ 40°
- [ ] At least 20% of rows have G/T below −20 dB/K (D11's standing pin — these rows must reach the fitter)
- [ ] The injected truth (perturbed RMS, bias coefficients) is exported as named constants
- [ ] A self-test asserts determinism and the row/span/sub-−20 properties

**Verify:** `cargo test -p calibrate --test cli_full_mode_e2e generator -- --nocapture` → self-tests pass, printing the row count and G/T range

**Steps:**

- [ ] **Step 1: Write the generator module**

Create `calibrate/tests/support/mod.rs`:

```rust
//! Deterministic perturbed-truth fixture generation for the `calibrate` CLI tests.
//!
//! The measurements written here are produced by evaluating the *real* physics model
//! with a **perturbed** surface RMS and adding a **closed-form bias**. Calibrating the
//! *nominal* `UHF_Array_Element` class against this data therefore has a known answer:
//! the residual the correction surface must absorb is the injected bias plus the
//! (smooth, small) difference the RMS perturbation makes.
//!
//! Nothing here uses randomness, wall-clock time, or hash iteration order — two runs
//! produce byte-identical output, which the CLI test relies on.

#![allow(dead_code)] // Not every consumer of this module uses every helper.

use antenna_model::model::{
    compute_g_over_t, AntennaConfiguration, AntennaConfigurationBuilder, FeedParametersBuilder,
    IntegrationParams, MeshParametersBuilder, ReflectorGeometryBuilder,
};
use std::path::Path;

// ============================================================================
// The injected truth
// ============================================================================

/// Antenna class the fixture is generated for and calibrated against.
///
/// Chosen for its broad beam: at 450 MHz this class measures 8.91 dB/K at boresight,
/// 7.93 at 2°, 2.55 at 5°, −19.75 at 10° and −41.53 at 20°. A narrow-beam class such as
/// `GroundStation_13m` (33.08 → −10.60 dB/K between 0° and 1°) has a sub-degree main lobe
/// that the fitter's 2° minimum E-cone knot spacing cannot represent.
pub const FIXTURE_CLASS: &str = "UHF_Array_Element";

/// Nominal surface RMS of `UHF_Array_Element`, from `calibrate/antenna_classes.yaml`.
pub const NOMINAL_SURFACE_RMS_MM: f64 = 2.0;

/// Surface RMS the "measurements" are actually generated at.
///
/// This is the perturbation `--tune-parameters` must recover. It is the ONLY physical
/// parameter perturbed: `TunableParameters` carries `surface_rms_mm`, `mesh_spacing_mm`
/// and `mesh_wire_diameter_mm` only, so a q-factor perturbation could never be recovered
/// by the full-mode tuner and would just add an unattributable residual.
pub const PERTURBED_SURFACE_RMS_MM: f64 = 2.6;

/// System noise temperature of `UHF_Array_Element`, from `antenna_classes.yaml`.
/// The CSV's `temperature_k` column must match this or the G/T values are inconsistent
/// with what the calibrator computes.
pub const FIXTURE_TEMPERATURE_K: f64 = 100.0;

/// Coefficients of the injected systematic bias, in dB.
///
/// `bias(f, cone, clock) = A + B*(f - f0)/f_span + C*cos(clock) + D*(cone/cone_span)`
///
/// Deliberately smooth at the scale of the knot spacing (50 MHz / 2° / 5°): one cosine
/// cycle over the full clock range and linear ramps elsewhere. A higher-frequency bias
/// would be unrepresentable by a 4/6/8-knot spline and the recovery assertion would fail
/// for legitimate reasons.
pub const BIAS_CONST_DB: f64 = 0.80;
pub const BIAS_FREQ_DB: f64 = 0.50;
pub const BIAS_CLOCK_DB: f64 = 0.60;
pub const BIAS_CONE_DB: f64 = 0.40;

// ============================================================================
// Grid definition
// ============================================================================

/// Frequencies in MHz. Span 300 MHz across 4 values — comfortably over the 50 MHz
/// minimum knot spacing with the artifact's 4 frequency knots.
pub const FIXTURE_FREQUENCIES_MHZ: [f64; 4] = [400.0, 500.0, 600.0, 700.0];

/// E-cone (polar) angles in degrees. Spans 0–24°: main lobe (0–5°), the shoulder, and
/// deep sidelobes past 10° where G/T falls below −20 dB/K.
pub const FIXTURE_CONE_DEG: [f64; 9] = [0.0, 2.0, 4.0, 6.0, 9.0, 12.0, 16.0, 20.0, 24.0];

/// E-clock (azimuthal) angles in degrees. Spans 0–315° in 45° steps — over the 5°
/// minimum knot spacing with the artifact's 8 clock knots.
pub const FIXTURE_CLOCK_DEG: [f64; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

/// Total rows the generator emits: 4 × 9 × 8 = 288.
pub const FIXTURE_ROW_COUNT: usize = FIXTURE_FREQUENCIES_MHZ.len()
    * FIXTURE_CONE_DEG.len()
    * FIXTURE_CLOCK_DEG.len();

/// The injected bias in dB at a grid location. Pure function — no state, no RNG.
pub fn injected_bias_db(frequency_mhz: f64, e_cone_deg: f64, e_clock_deg: f64) -> f64 {
    let f_lo = FIXTURE_FREQUENCIES_MHZ[0];
    let f_hi = FIXTURE_FREQUENCIES_MHZ[FIXTURE_FREQUENCIES_MHZ.len() - 1];
    let cone_hi = FIXTURE_CONE_DEG[FIXTURE_CONE_DEG.len() - 1];

    BIAS_CONST_DB
        + BIAS_FREQ_DB * (frequency_mhz - f_lo) / (f_hi - f_lo)
        + BIAS_CLOCK_DB * e_clock_deg.to_radians().cos()
        + BIAS_CONE_DB * (e_cone_deg / cone_hi)
}

/// Build the physics configuration for `UHF_Array_Element` at a given surface RMS.
///
/// Mirrors `calibrate/src/main.rs::compute_model_predictions` exactly — same builders,
/// same mm→m conversions, same at-focus feed placement. If that function changes, this
/// must change with it or the fixture stops being perturbed truth.
pub fn fixture_config(surface_rms_mm: f64) -> AntennaConfiguration {
    let diameter_m = 8.0;
    let f_over_d = 0.45;
    let focal_length = diameter_m * f_over_d;

    let reflector = ReflectorGeometryBuilder::default()
        .diameter(diameter_m)
        .focal_length(focal_length)
        .surface_rms(surface_rms_mm / 1000.0)
        .build()
        .expect("fixture reflector geometry");

    let feed = FeedParametersBuilder::default()
        .at_focus(focal_length)
        .q_factor(5.0)
        .phase_center_offset(0.0)
        .asymmetry_factor(1.1)
        .build()
        .expect("fixture feed parameters");

    let mesh = MeshParametersBuilder::default()
        .spacing(10.0 / 1000.0)
        .wire_diameter(1.0 / 1000.0)
        .build()
        .expect("fixture mesh parameters");

    AntennaConfigurationBuilder::default()
        .id("UHF_Array_Element")
        .name("UHF phased array element (low frequency)")
        .reflector(reflector)
        .feed(feed)
        .mesh(mesh)
        .build()
        .expect("fixture antenna configuration")
}

/// One generated measurement row.
pub struct FixtureRow {
    pub e_clock_deg: f64,
    pub e_cone_deg: f64,
    pub frequency_mhz: f64,
    pub g_over_t_db: f64,
    pub temperature_k: f64,
}

/// Generate the full perturbed-truth grid.
pub fn generate_rows() -> Vec<FixtureRow> {
    let config = fixture_config(PERTURBED_SURFACE_RMS_MM);
    let params = IntegrationParams::default();
    let mut rows = Vec::with_capacity(FIXTURE_ROW_COUNT);

    for &frequency_mhz in &FIXTURE_FREQUENCIES_MHZ {
        for &e_cone_deg in &FIXTURE_CONE_DEG {
            for &e_clock_deg in &FIXTURE_CLOCK_DEG {
                let truth = compute_g_over_t(
                    e_cone_deg.to_radians(),
                    e_clock_deg.to_radians(),
                    &config,
                    frequency_mhz * 1e6,
                    FIXTURE_TEMPERATURE_K,
                    &params,
                )
                .expect("fixture G/T evaluation");

                rows.push(FixtureRow {
                    e_clock_deg,
                    e_cone_deg,
                    frequency_mhz,
                    g_over_t_db: truth + injected_bias_db(frequency_mhz, e_cone_deg, e_clock_deg),
                    temperature_k: FIXTURE_TEMPERATURE_K,
                });
            }
        }
    }

    rows
}

/// Render rows as CSV text in the full-mode column order.
///
/// Fixed 6-decimal formatting keeps the output byte-identical across runs and platforms.
pub fn rows_to_csv(rows: &[FixtureRow]) -> String {
    let mut csv =
        String::from("e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n");
    for r in rows {
        csv.push_str(&format!(
            "{:.6},{:.6},{:.6},{:.6},{:.6}\n",
            r.e_clock_deg, r.e_cone_deg, r.frequency_mhz, r.g_over_t_db, r.temperature_k
        ));
    }
    csv
}

/// Generate the fixture and write it to `path`.
pub fn write_fixture_csv(path: &Path) -> Vec<FixtureRow> {
    let rows = generate_rows();
    std::fs::write(path, rows_to_csv(&rows)).expect("write fixture CSV");
    rows
}
```

- [ ] **Step 2: Write the generator self-tests**

Create `calibrate/tests/cli_full_mode_e2e.rs` with just the generator tests for now:

```rust
//! End-to-end test of the `calibrate` binary in full mode, on perturbed-truth data.

mod support;

use support::*;

#[test]
fn generator_is_deterministic() {
    let a = rows_to_csv(&generate_rows());
    let b = rows_to_csv(&generate_rows());
    assert_eq!(a, b, "the fixture generator must be byte-reproducible");
}

#[test]
fn generator_grid_satisfies_the_fitter_constraints() {
    let rows = generate_rows();

    assert_eq!(rows.len(), FIXTURE_ROW_COUNT);
    assert!(
        rows.len() >= 200,
        "a 5-fold CV training split must still clear the fitter's 125-point minimum, \
         got {} rows",
        rows.len()
    );

    let freq_span = FIXTURE_FREQUENCIES_MHZ[FIXTURE_FREQUENCIES_MHZ.len() - 1]
        - FIXTURE_FREQUENCIES_MHZ[0];
    let cone_span = FIXTURE_CONE_DEG[FIXTURE_CONE_DEG.len() - 1] - FIXTURE_CONE_DEG[0];
    let clock_span = FIXTURE_CLOCK_DEG[FIXTURE_CLOCK_DEG.len() - 1] - FIXTURE_CLOCK_DEG[0];

    assert!(freq_span >= 200.0, "frequency span {freq_span} MHz too narrow");
    assert!(cone_span >= 12.0, "cone span {cone_span} deg too narrow");
    assert!(clock_span >= 40.0, "clock span {clock_span} deg too narrow");
}

/// D11's standing pin: the fixture must contain rows the pre-D11 parser discarded.
#[test]
fn generator_produces_realistic_sub_minus_twenty_sidelobes() {
    let rows = generate_rows();
    let deep = rows.iter().filter(|r| r.g_over_t_db < -20.0).count();

    assert!(
        deep * 5 >= rows.len(),
        "at least 20% of rows should sit below -20 dB/K (D11 pin), got {deep} of {}",
        rows.len()
    );

    let min = rows
        .iter()
        .map(|r| r.g_over_t_db)
        .fold(f64::INFINITY, f64::min);
    println!("fixture: {} rows, minimum G/T {:.2} dB/K", rows.len(), min);
}

#[test]
fn injected_bias_is_bounded_and_smooth() {
    // The bias must stay well inside the accuracy targets it will be measured against,
    // and vary smoothly enough for a 4/6/8-knot spline to represent it.
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for &f in &FIXTURE_FREQUENCIES_MHZ {
        for &cone in &FIXTURE_CONE_DEG {
            for &clock in &FIXTURE_CLOCK_DEG {
                let b = injected_bias_db(f, cone, clock);
                min = min.min(b);
                max = max.max(b);
            }
        }
    }
    assert!(min > -1.0 && max < 3.0, "bias range [{min}, {max}] dB");
    println!("injected bias range: [{min:.3}, {max:.3}] dB");
}
```

- [ ] **Step 3: Run the generator tests**

Run: `cargo test -p calibrate --test cli_full_mode_e2e -- --nocapture`

Expected: 4 tests pass. Note the printed row count, minimum G/T, and bias range.

If `generator_produces_realistic_sub_minus_twenty_sidelobes` fails, the cone grid does not reach far enough into the sidelobes — extend `FIXTURE_CONE_DEG` upward (e.g. add 30.0, 36.0). **Do not** lower the −20 threshold; it is D11's boundary, not a tunable.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add calibrate/tests/support/mod.rs calibrate/tests/cli_full_mode_e2e.rs
git commit -m "test(D12): deterministic perturbed-truth fixture generator

Generates measurements from the real physics model at a perturbed surface RMS
(2.0 -> 2.6 mm) plus a closed-form smooth bias, so calibrating the nominal
UHF_Array_Element class has a known answer rather than identically-zero
residuals.

UHF_Array_Element is chosen for its broad beam: 8.91 dB/K at boresight falling
to -41.53 at 20 deg, so a 2-degree minimum E-cone knot spacing can resolve the
main lobe. GroundStation_13m at 4 GHz falls 33.08 -> -10.60 dB/K between 0 and
1 degree and cannot be fitted at these knot floors.

Only surface_rms_mm is perturbed: full-mode TunableParameters carries no
q_factor, so a q perturbation could never be recovered by the tuner.

288 rows over 400-700 MHz / 0-24 deg cone / 0-315 deg clock, clearing the
fitter's 125-point minimum with room for a 5-fold CV split, and with >20% of
rows below -20 dB/K as a standing pin on D11.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: CLI end-to-end run and artifact assertions

**Goal:** Run the actual `calibrate` binary over the fixture and assert it exits 0, writes a service-loadable artifact, and improves on the uncorrected model.

**Files:**
- Modify: `calibrate/tests/cli_full_mode_e2e.rs`
- Modify: `calibrate/Cargo.toml` (add `serde_json` to `[dev-dependencies]` only if the test cannot see the regular dependency — check first; `calibrate` already lists `serde_json` under `[dependencies]`, and integration tests link the crate's public API, not its dependencies, so a dev-dependency entry IS required)

**Acceptance Criteria:**
- [ ] The test invokes `env!("CARGO_BIN_EXE_calibrate")`, not a library function
- [ ] Exit status is success; on failure the test prints captured stdout+stderr
- [ ] The artifact file exists and begins with the ASCII magic `ANTC`
- [ ] `antenna_model::data::loader::load_calibration_artifact` loads it successfully
- [ ] The loaded `AntennaCalibration` has `antenna_id` matching `--antenna-id` and a `Some(..)` `correction_surface`
- [ ] The `--report` sidecar parses as JSON and shows `corrected_rmse < model_only_rmse`

**Verify:** `cargo test -p calibrate --test cli_full_mode_e2e cli_ -- --nocapture` → passes in well under 60 s

**Steps:**

- [ ] **Step 1: Add the dev-dependency**

In `calibrate/Cargo.toml`, under `[dev-dependencies]`:

```toml
[dev-dependencies]
tempfile = "3.27.0"
serde_json = "1.0.150"
```

- [ ] **Step 2: Write the failing test**

Append to `calibrate/tests/cli_full_mode_e2e.rs`:

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

/// Outputs of one full-mode CLI run.
struct CalibrateRun {
    stdout: String,
    stderr: String,
    artifact: PathBuf,
    report: PathBuf,
    metadata: PathBuf,
    _dir: tempfile::TempDir,
}

impl CalibrateRun {
    /// Everything the binary printed, for assertions and failure messages.
    fn output(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    fn report_json(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(&self.report).expect("read --report sidecar");
        serde_json::from_str(&text).expect("parse --report sidecar as JSON")
    }
}

/// Run the real `calibrate` binary in full mode over a freshly generated fixture.
///
/// `extra_args` appends flags such as `--validate` / `--cv-folds N`.
fn run_calibrate(extra_args: &[&str]) -> CalibrateRun {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("measurements.csv");
    let artifact = dir.path().join("antenna.bin");
    let report = dir.path().join("report.json");
    let metadata = dir.path().join("metadata.json");

    write_fixture_csv(&input);

    // `--classes-file` defaults to `calibrate/antenna_classes.yaml`, resolved against the
    // process CWD. An integration test's CWD is the crate root, so the correct path here
    // is `antenna_classes.yaml` — build it from CARGO_MANIFEST_DIR to be independent of
    // however the test binary happens to be invoked.
    let classes_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("antenna_classes.yaml");
    assert!(
        classes_file.exists(),
        "antenna class definitions not found at {}",
        classes_file.display()
    );

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_calibrate"));
    cmd.args(["--calibration-mode", "full"])
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&artifact)
        .args(["--antenna-id", "d12_uhf_test"])
        .args(["--antenna-class", FIXTURE_CLASS])
        .arg("--classes-file")
        .arg(&classes_file)
        .arg("--report")
        .arg(&report)
        .arg("--metadata")
        .arg(&metadata)
        .args(extra_args);

    let out = cmd.output().expect("run the calibrate binary");
    let run = CalibrateRun {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        artifact,
        report,
        metadata,
        _dir: dir,
    };

    assert!(
        out.status.success(),
        "calibrate exited with {:?}\n--- output ---\n{}",
        out.status.code(),
        run.output()
    );

    run
}

#[test]
fn cli_full_mode_writes_a_service_loadable_artifact() {
    let run = run_calibrate(&[]);

    let bytes = std::fs::read(&run.artifact).expect("read artifact");
    assert!(
        bytes.starts_with(b"ANTC"),
        "artifact is missing the ANTC magic; first bytes: {:?}",
        &bytes[..bytes.len().min(8)]
    );

    // The point of this assertion: the artifact must load through the SERVICE's loader,
    // not just calibrate's own round-trip code.
    let calibration =
        antenna_model::data::loader::load_calibration_artifact(&run.artifact)
            .expect("the service loader must accept a freshly written full-mode artifact");

    assert_eq!(calibration.antenna_id, "d12_uhf_test");
    assert!(
        calibration.correction_surface.is_some(),
        "full mode must ship a correction surface"
    );
}

#[test]
fn cli_full_mode_correction_beats_the_uncorrected_model() {
    let run = run_calibrate(&[]);
    let report = run.report_json();

    let model_only = report["model_only_rmse"].as_f64().expect("model_only_rmse");
    let corrected = report["corrected_rmse"].as_f64().expect("corrected_rmse");

    println!("model-only RMSE {model_only:.4} dB, corrected {corrected:.4} dB");
    assert!(
        corrected < model_only,
        "the correction surface must improve on the physics model: \
         corrected {corrected:.4} dB vs model-only {model_only:.4} dB"
    );
    assert!(
        corrected < 0.5 * model_only,
        "the injected bias is a large, smooth, fully representable signal — the fit \
         should remove most of it: corrected {corrected:.4} dB vs model-only {model_only:.4} dB"
    );
}
```

- [ ] **Step 3: Run to verify it fails, then passes**

Run: `cargo test -p calibrate --test cli_full_mode_e2e cli_ -- --nocapture`

Expected first run: FAIL on the missing `serde_json` dev-dependency if Step 1 was skipped; otherwise this should pass. If the binary exits non-zero, the assertion prints its full output — read it. The two most likely causes, in order:

1. `Antenna class 'UHF_Array_Element' not found` → `--classes-file` path is wrong.
2. `Insufficient data for fitting: need at least 125` → the parser dropped rows, or the grid shrank. Print the row count and check the "Parsed N measurements" line in the captured output.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add calibrate/Cargo.toml calibrate/tests/cli_full_mode_e2e.rs
git commit -m "test(D12): run the calibrate binary end to end

First test that executes the actual CLI through parse -> predict -> fit ->
validate -> artifact. Asserts exit 0, the ANTC magic, and — the assertion that
matters — that the artifact loads through the SERVICE's loader
(antenna_model::data::loader), not just calibrate's own round-trip code.

Also pins that the correction surface removes most of the injected bias.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Known-answer recovery and cross-validation assertions

**Goal:** Assert the correction surface recovers the *injected bias* at probe points, and that `--validate` / `--cv-folds` behave as D10 and Task 1 established.

**Background:** This is what makes the test known-answer rather than merely "it ran". The fitted surface should approximate `injected_bias_db(...)` plus the small smooth offset from the RMS perturbation. The recovery tolerance must accommodate that offset — do not assert exact equality with the bias.

**Files:**
- Modify: `calibrate/tests/cli_full_mode_e2e.rs`

**Acceptance Criteria:**
- [ ] The correction evaluated at interior probe points tracks `injected_bias_db` within a stated tolerance
- [ ] Probe points are interior to the grid (extrapolation at the edges is out of scope)
- [ ] `--validate --cv-folds N` produces a report whose `cross_validation.num_folds` is N, for two different N (pins D10)
- [ ] Without `--validate` the report has no cross-validation section (pins Task 1)
- [ ] The tolerance is documented with the reason for its value

**Verify:** `cargo test -p calibrate --test cli_full_mode_e2e -- --nocapture` → all tests pass

**Steps:**

- [ ] **Step 1: Write the recovery test**

Append to `calibrate/tests/cli_full_mode_e2e.rs`:

```rust
/// Tolerance on bias recovery, in dB.
///
/// The fitted surface absorbs the injected bias PLUS the residual left by calibrating a
/// nominal 2.0 mm surface RMS against data generated at 2.6 mm. That second component is
/// smooth and small but not zero, so the recovery is not exact by construction. This
/// tolerance is loose enough to accommodate it and tight enough that a surface fitting
/// nothing (all-zero coefficients, ~0.8–2.3 dB from the truth) still fails.
const BIAS_RECOVERY_TOLERANCE_DB: f64 = 0.35;

#[test]
fn cli_full_mode_recovers_the_injected_bias() {
    let run = run_calibrate(&[]);
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("load artifact");
    let surface = calibration
        .correction_surface
        .as_ref()
        .expect("full mode must ship a correction surface");

    // Interior probe points only — off-grid but inside the fitted domain in every axis.
    // Edge behavior is extrapolation and is not what this assertion is about.
    let probes = [
        (450.0_f64, 3.0_f64, 30.0_f64),
        (550.0, 7.0, 120.0),
        (650.0, 14.0, 200.0),
        (500.0, 10.0, 280.0),
    ];

    let mut worst = 0.0_f64;
    for (frequency_mhz, e_cone_deg, e_clock_deg) in probes {
        // The parameters are named azimuth/elevation, but the 3D->4D bridge maps
        // clock -> azimuth and cone -> elevation (artifact_export.rs:482). Clock first.
        // Returns a CorrectionResult, not an f64.
        let got = antenna_model::model::evaluate_correction(
            surface,
            e_clock_deg,
            e_cone_deg,
            frequency_mhz,
            FIXTURE_TEMPERATURE_K,
        )
        .expect("evaluate the 4D correction surface")
        .correction_db;

        let expected = injected_bias_db(frequency_mhz, e_cone_deg, e_clock_deg);
        let err = (got - expected).abs();
        worst = worst.max(err);

        println!(
            "probe f={frequency_mhz:6.1} cone={e_cone_deg:5.1} clock={e_clock_deg:6.1} \
             -> correction {got:+.4} dB, injected {expected:+.4} dB, err {err:.4} dB"
        );
    }

    assert!(
        worst <= BIAS_RECOVERY_TOLERANCE_DB,
        "the correction surface should recover the injected bias within \
         {BIAS_RECOVERY_TOLERANCE_DB} dB, worst error was {worst:.4} dB"
    );
}
```

- [ ] **Step 2: Write the cross-validation tests**

Append:

```rust
/// D10's standing pin at CLI level: `--cv-folds N` must reach the validator.
#[test]
fn cli_cv_folds_controls_the_reported_fold_count() {
    for folds in [3usize, 6] {
        let n = folds.to_string();
        let run = run_calibrate(&["--validate", "--cv-folds", &n]);
        let report = run.report_json();

        let reported = report["cross_validation"]["num_folds"]
            .as_u64()
            .unwrap_or_else(|| {
                panic!(
                    "--validate --cv-folds {folds} should produce a cross-validation \
                     section; report was:\n{report:#}"
                )
            });
        assert_eq!(reported as usize, folds);

        let values = report["cross_validation"]["fold_rmse_values"]
            .as_array()
            .expect("fold_rmse_values");
        assert_eq!(values.len(), folds, "one RMSE per fold");
    }
}

/// Task 1's pin: without `--validate`, step 6 must not cross-validate — but the rest of
/// the validation report must still be there.
#[test]
fn cli_without_validate_does_not_cross_validate() {
    let run = run_calibrate(&[]);
    let report = run.report_json();

    assert!(
        report["cross_validation"].is_null(),
        "cross-validation ran without --validate; report was:\n{report:#}"
    );
    assert!(
        !run.output().contains("cross-validation"),
        "the binary announced cross-validation on a run that did not request it:\n{}",
        run.output()
    );

    // The rest of step 6 still runs.
    assert!(report["corrected_rmse"].as_f64().is_some());
    assert!(report["main_lobe_max_error"].as_f64().is_some());
    assert!(report["first_sidelobe_max_error"].as_f64().is_some());
}
```

- [ ] **Step 3: Run and calibrate the tolerance**

Run: `cargo test -p calibrate --test cli_full_mode_e2e -- --nocapture`

Read the printed per-probe errors. If `cli_full_mode_recovers_the_injected_bias` fails:

- **Worst error slightly above 0.35 dB** → the RMS-perturbation residual is larger than estimated. Raise `BIAS_RECOVERY_TOLERANCE_DB` to just above the observed worst error and update its doc comment with the measured value and the reason. This is legitimate.
- **Worst error above ~1 dB** → something is wrong, not merely loose. Check that the probe points are interior, that the argument order in `evaluate_correction` is (clock, cone, frequency, temperature), and that `FIXTURE_TEMPERATURE_K` matches the class's `system_noise_temperature_k`. Do **not** just widen the tolerance.
- **Correction is ~0 everywhere** → the surface fitted nothing. Check the report's `improvement_percent`.

- [ ] **Step 4: Verify the whole suite and commit**

```bash
./scripts/check.sh
```

Expected: "All gate checks passed."

```bash
git add calibrate/tests/cli_full_mode_e2e.rs
git commit -m "test(D12): known-answer recovery and CV assertions

The correction surface must recover the injected bias at interior probe points
within a documented tolerance -- this is what makes the test known-answer
rather than just 'the pipeline ran'.

Adds CLI-level pins for D10 (--cv-folds N reaches the validator and produces N
fold RMSEs) and for the --validate gating (no cross-validation section, and no
CV announcement in the output, on a run that did not ask for it).

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Measure the tuned run and decide its CI status

**Goal:** Determine empirically whether an end-to-end run with `--tune-parameters` is affordable in CI, and either include it or mark it `#[ignore]` with the measured justification.

**Background:** The D12 spec defers this to measurement. The risk is real: differential evolution runs `max_iterations` generations over a population, and **each** candidate evaluation computes the model at every measurement point. At ~1.3 ms/point debug and 288 points, one candidate evaluation is ~0.4 s; 100 iterations × a population of ~15 would be ~10 minutes. `--max-tuning-iterations` is the lever.

This task is deliberately last: everything before it is valuable whatever the answer here turns out to be.

**Files:**
- Modify: `calibrate/tests/cli_full_mode_e2e.rs`

**Acceptance Criteria:**
- [ ] A tuned end-to-end run exists and passes
- [ ] Its wall-clock time is measured and recorded in a comment
- [ ] If it exceeds ~60 s it is `#[ignore]`d with the measured time and the run command in the doc comment; if under, it runs by default
- [ ] When it runs, it asserts the tuner moves `surface_rms_mm` from the nominal 2.0 mm toward the perturbed 2.6 mm

**Verify:** `cargo test -p calibrate --test cli_full_mode_e2e tuned -- --nocapture --include-ignored` → passes, printing elapsed time and the recovered RMS

**Steps:**

- [ ] **Step 1: Write the tuned test with timing**

Append to `calibrate/tests/cli_full_mode_e2e.rs`:

```rust
/// End-to-end run WITH parameter tuning.
///
/// Iteration count is held low deliberately: each differential-evolution candidate
/// evaluates the physics model at all 288 fixture points (~0.4 s in a debug build), so
/// cost scales as iterations × population × points. The assertion is directional — the
/// tuner must move surface RMS off the nominal 2.0 mm toward the 2.6 mm the data was
/// generated at — not that it converges exactly, which a short run will not do.
#[test]
fn cli_tuned_run_recovers_the_surface_rms_perturbation() {
    let start = std::time::Instant::now();
    let run = run_calibrate(&["--tune-parameters", "--max-tuning-iterations", "8"]);
    let elapsed = start.elapsed();

    println!("tuned end-to-end run took {:.1} s", elapsed.as_secs_f64());

    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("load artifact");
    println!("{}", run.output());

    let metadata: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&run.metadata).expect("read --metadata sidecar"),
    )
    .expect("parse --metadata sidecar");
    assert_eq!(
        metadata["parameters_tuned"], true,
        "the metadata sidecar should record that tuning ran"
    );

    // Directional recovery: the tuned RMS must move off nominal toward the truth.
    // The data-layer ReflectorGeometry stores millimetres already — no conversion.
    let tuned_rms = calibration.physical_config.reflector.surface_rms_mm;
    println!(
        "surface RMS: nominal {NOMINAL_SURFACE_RMS_MM} mm, truth \
         {PERTURBED_SURFACE_RMS_MM} mm, tuned {tuned_rms:.4} mm"
    );
    assert!(
        tuned_rms > NOMINAL_SURFACE_RMS_MM,
        "the tuner should move surface RMS toward the perturbed truth \
         ({PERTURBED_SURFACE_RMS_MM} mm), but it stayed at or below nominal: {tuned_rms:.4} mm"
    );
}
```

**Note on the RMS field path:** `calibration.physical_config.reflector.surface_rms_mm` was verified against `antenna-model/src/data/types.rs:362` during planning — the data-layer `ReflectorGeometry` stores **millimetres**, unlike the model-layer builder in `compute_model_predictions` which takes metres. If the assertion reads a value near 0.002 rather than near 2.0, the wrong unit is being compared.

- [ ] **Step 2: Measure**

Run: `cargo test -p calibrate --test cli_full_mode_e2e tuned -- --nocapture`

Record the printed elapsed time.

- [ ] **Step 3: Decide and record**

If elapsed is **under ~60 s**, leave the test enabled and change its doc comment's first line to state the measured time, e.g. `/// Measured 2026-07-29: 24 s in a debug build at 8 iterations.`

If elapsed is **over ~60 s**, add `#[ignore = "..."]` above the `#[test]` with the measured time and how to run it:

```rust
#[ignore = "slow: measured NN s in a debug build; run with \
            `cargo test -p calibrate --test cli_full_mode_e2e tuned -- --include-ignored`"]
```

Then lower `--max-tuning-iterations` and re-measure once; if a value of 4 brings it under 60 s while keeping the directional assertion true, prefer the enabled fast version over an ignored slow one. If the directional assertion fails at low iteration counts, keep the higher count and `#[ignore]` it — **do not** weaken the assertion to fit the budget.

- [ ] **Step 4: Run the full gate and commit**

```bash
./scripts/check.sh
git add calibrate/tests/cli_full_mode_e2e.rs
git commit -m "test(D12): tuned end-to-end run, CI status decided by measurement

Adds a --tune-parameters end-to-end run asserting the tuner moves surface RMS
off nominal toward the perturbed truth the fixture was generated at. Cost
scales as iterations x population x points, with every candidate evaluating
the model at all 288 points, so the iteration count is held low and the
measured wall-clock time is recorded in the doc comment.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: Update the roadmap documents

**Goal:** Record D12 as done with its measured numbers, note finding #1's resolution, and correct the stale Phase 3 / C7 text found during planning.

**Background:** Two doc-truth items, both verified on 2026-07-29:
1. The Phase 3 row of `docs/roadmap-2026-07.md` still says *"C7 is now the only remaining Phase 3 unit"* and *"The contract is not yet frozen — freeze happens when C7 lands"*. C7 landed in PR #24 (`5cf26e5`, "contract frozen"), and §7 of the same document already records it as resolved — the two halves disagree.
2. The narrative's calibration risk entry says the accuracy figures are "mechanically sound yet unexercised until D12 lands". D12 landing changes that.

**Files:**
- Modify: `docs/roadmap-2026-07-work-units.md` — the D12 section (~line 2730), the dependency graph (~line 67)
- Modify: `docs/roadmap-2026-07.md` — Phase 3 row (~line 249), Phase 4 row (~line 250), the calibration risk in §7 (~line 339)

**Acceptance Criteria:**
- [ ] D12's unit section opens with a ✅ DONE line naming the date and branch
- [ ] Recorded: fixture class + why, row count, measured G/T range, bias recovery worst-case error, tuned-run wall-clock and its CI status
- [ ] Finding #1's resolution is recorded against D12 (it was filed by D10)
- [ ] The Phase 3 row no longer claims C7 is pending or the contract unfrozen
- [ ] The §7 calibration risk reflects that the pipeline now has an end-to-end test; the remaining `.bin`/D9 gap stays stated
- [ ] The dependency-graph block marks D12 done

**Verify:** `grep -n "C7 is now the only remaining\|contract is not yet frozen" docs/roadmap-2026-07.md` → no matches

**Steps:**

- [ ] **Step 1: Write the D12 completion note**

At the top of the `### D12 — ...` section in `docs/roadmap-2026-07-work-units.md`, insert a completion block in the style D1/D10/D11 use: a `**✅ DONE 2026-07-29** — branch ...` line, the measured numbers from Tasks 2–5, the tests added, and any findings filed and not fixed. Fill every number in from the actual test output — no placeholders.

Include explicitly:
- Fixture class `UHF_Array_Element` and the beam-width reason it was chosen over `GroundStation_13m` (33.08 → −10.60 dB/K between 0° and 1° cannot be resolved at a 2° knot floor).
- That `q_factor` is not a full-mode tunable, so only `surface_rms_mm` was perturbed.
- The measured bias-recovery worst-case error and the tolerance chosen.
- The tuned run's measured time and whether it is `#[ignore]`d.

- [ ] **Step 2: Record finding #1's resolution**

In the D10 section's "One finding, not fixed here" paragraph, append: `**Resolved 2026-07-29 in D12** — `validation_config` now takes `--validate` and sets `num_folds = 0` when it is false; only cross-validation is gated, the rest of step 6 is unchanged.`

- [ ] **Step 3: Correct the Phase 3 row**

In `docs/roadmap-2026-07.md`, in the Phase 3 row, replace `**C7 is now the only remaining Phase 3 unit**, and only once it lands does its openapi drift guard freeze the contract.` with a statement that C7 landed 2026-07-29 (PR #24) and the contract is frozen behind its generated-spec drift guard. Replace the trailing `**The contract is not yet frozen — freeze happens when C7 lands.**` with `**✅ Phase 3 complete — the contract is frozen behind C7's generated-spec drift guard (2026-07-29).**`

- [ ] **Step 4: Update the Phase 4 row and the §7 risk**

Phase 4 row: add D12 to the completed list with a one-clause summary.

§7 risk "The calibration pipeline's accuracy claims are unverified end-to-end": revise so it states the end-to-end test now exists and what it proves (known-answer bias recovery, service-loader acceptance), while keeping the still-open part — no `.bin` artifact ships (D9), and real-data coverage is D13/D14.

- [ ] **Step 5: Update the dependency graph**

In the graph block near the top of `docs/roadmap-2026-07-work-units.md`, mark D12 done alongside the existing `D10, D11 DONE 2026-07-29` line.

- [ ] **Step 6: Verify and commit**

```bash
grep -n "C7 is now the only remaining\|contract is not yet frozen" docs/roadmap-2026-07.md
```
Expected: no output.

```bash
./scripts/check.sh
git add docs/
git commit -m "docs: record D12 done; correct stale Phase 3 / C7 status

Marks D12 complete with its measured numbers (fixture scale, G/T range, bias
recovery error, tuned-run timing) and records that finding 1 from PR #26 --
full-mode step 6 cross-validating regardless of --validate -- was resolved
here.

Also corrects a doc-truth inconsistency found while planning: the Phase 3 row
still claimed C7 was the only remaining unit and the contract not yet frozen,
while section 7 of the same document already recorded C7 as resolved. C7
landed in PR #24.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Risks and open questions

**The bias-recovery tolerance is the one number this plan cannot predict.** It depends on how much residual the surface-RMS perturbation leaves after the spline absorbs the injected bias. Task 4 Step 3 gives explicit guidance on distinguishing "legitimately loose" from "something is wrong". If the worst error lands above ~1 dB, stop and investigate rather than widening.

**The tuned run may be too slow for CI.** Task 5 measures rather than assumes, and is sequenced last so the rest of the unit ships regardless.

**D12 may surface further defects.** The full pipeline has never completed end to end in this repo — D10 and D11 removed the two *known* blockers, but this is the first real exercise of it. If a new defect appears, file it (standing rule 5) rather than working around it; that is the unit doing its job, exactly as D1 did when it filed D10 and D11.

**Out of scope, deliberately:** finding #2 (`detect_outliers` running MAD on raw G/T rather than residuals) is held for before D13/D14 by decision this session. `create_sample_csv`'s unrealistic rolloff is superseded in practice by this generator; replacing or deleting it is not in D12's charter.
