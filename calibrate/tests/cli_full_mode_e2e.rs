//! End-to-end test of the `calibrate` binary in full mode, on perturbed-truth data.

mod support;

use calibrate::{AntennaClassRegistry, CorrectionSurfaceParams};
use std::path::{Path, PathBuf};
use std::process::Command;
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

    // The binding quantity is the fitted COEFFICIENT count, not the fitter's cheap
    // `(spline_order + 1)^3 = 125` pre-check — roadmap D20. Full mode requests 4/6/8
    // internal knots at order 4, and each axis contributes `placed_knots + order` basis
    // functions, so the surface declares at most 8 * 10 * 12 = 960 coefficients.
    //
    // The tightest split any test here runs at is `--cv-folds 3` (see
    // `cli_cv_folds_controls_the_reported_fold_count`), whose training fold is 2/3 of the
    // grid. That fold, not the whole grid, is what has to cover the coefficients.
    const MAX_COEFFICIENTS: usize = 8 * 10 * 12;
    const TIGHTEST_TRAINING_FRACTION: f64 = 2.0 / 3.0;

    let tightest_fold = (rows.len() as f64 * TIGHTEST_TRAINING_FRACTION) as usize;
    assert!(
        tightest_fold >= MAX_COEFFICIENTS,
        "a 3-fold CV training split ({tightest_fold} of {} rows) must cover the \
         {MAX_COEFFICIENTS} coefficients the shipped knot counts declare, or the fitter \
         rejects it as underdetermined",
        rows.len()
    );

    let freq_span =
        FIXTURE_FREQUENCIES_MHZ[FIXTURE_FREQUENCIES_MHZ.len() - 1] - FIXTURE_FREQUENCIES_MHZ[0];
    let cone_span = FIXTURE_CONE_DEG[FIXTURE_CONE_DEG.len() - 1] - FIXTURE_CONE_DEG[0];
    let clock_span = FIXTURE_CLOCK_DEG[FIXTURE_CLOCK_DEG.len() - 1] - FIXTURE_CLOCK_DEG[0];

    // Knot *counts* full mode fits with. These are not importable — they're a private
    // local in `calibrate/src/main.rs::surface_fitting_params` (4/6/8) — so they're
    // mirrored here as named constants. If that function's knot counts change, these
    // must change with it.
    const NUM_KNOTS_FREQUENCY: f64 = 4.0;
    const NUM_KNOTS_ECONE: f64 = 6.0;
    const NUM_KNOTS_ECLOCK: f64 = 8.0;

    // The minimum knot *spacing* floors, by contrast, ARE importable: they're public
    // fields of `CorrectionSurfaceParams`, and `default()` carries the same values
    // `surface_fitting_params` hardcodes. Deriving the required spans from here means a
    // change to the floors (e.g. widening `min_knot_spacing_frequency`) is caught
    // automatically instead of silently passing against a stale bare literal.
    let knot_floors = CorrectionSurfaceParams::default();
    let required_freq_span = NUM_KNOTS_FREQUENCY * knot_floors.min_knot_spacing_frequency;
    let required_cone_span = NUM_KNOTS_ECONE * knot_floors.min_knot_spacing_econe;
    let required_clock_span = NUM_KNOTS_ECLOCK * knot_floors.min_knot_spacing_eclock;

    assert!(
        freq_span >= required_freq_span,
        "frequency span {freq_span} MHz too narrow for {NUM_KNOTS_FREQUENCY} knots at \
         {} MHz minimum spacing (need >= {required_freq_span})",
        knot_floors.min_knot_spacing_frequency
    );
    assert!(
        cone_span >= required_cone_span,
        "cone span {cone_span} deg too narrow for {NUM_KNOTS_ECONE} knots at {} deg \
         minimum spacing (need >= {required_cone_span})",
        knot_floors.min_knot_spacing_econe
    );
    assert!(
        clock_span >= required_clock_span,
        "clock span {clock_span} deg too narrow for {NUM_KNOTS_ECLOCK} knots at {} deg \
         minimum spacing (need >= {required_clock_span})",
        knot_floors.min_knot_spacing_eclock
    );
}

/// Standing pin on roadmap unit D11: the fixture must contain rows the pre-D11 parser
/// discarded (it rejected anything below -20 dB/K as "atypical G/T", which is a boresight
/// figure, silently dropping legitimate sidelobe measurements).
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
fn injected_bias_is_bounded() {
    // The bias must stay well inside the accuracy targets it will be measured against.
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

/// The bounds check above says nothing about smoothness: swap the clock cosine for a
/// sawtooth and min/max stay identical. Smoothness is what "deliberately smooth at the
/// scale of the knot spacing" (see the doc comment on `BIAS_*_DB` in support/mod.rs)
/// actually buys us, and it's load-bearing for a later task's recovery assertion — a
/// 4/6/8-knot spline cannot represent a bias with high-frequency content.
///
/// Extrapolate each axis's worst-case adjacent-grid-point change out to the fitter's
/// minimum knot span for that axis; the result must stay under the axis's own
/// coefficient, i.e. even over the tightest span the fitter is willing to place a knot,
/// the term must not swing past its own designed amplitude.
#[test]
fn injected_bias_is_smooth() {
    let knot_floors = CorrectionSurfaceParams::default();

    let freq_rate = FIXTURE_FREQUENCIES_MHZ
        .windows(2)
        .map(|w| {
            (injected_bias_db(w[1], 0.0, 0.0) - injected_bias_db(w[0], 0.0, 0.0)).abs()
                / (w[1] - w[0])
        })
        .fold(0.0_f64, f64::max);
    let freq_change_per_knot_span = freq_rate * knot_floors.min_knot_spacing_frequency;
    assert!(
        freq_change_per_knot_span < BIAS_FREQ_DB,
        "frequency-axis bias changes {freq_change_per_knot_span:.4} dB over one minimum \
         knot span ({} MHz) — exceeds its own amplitude ({BIAS_FREQ_DB} dB), not smooth \
         enough for the fitter's knots",
        knot_floors.min_knot_spacing_frequency
    );

    let cone_rate = FIXTURE_CONE_DEG
        .windows(2)
        .map(|w| {
            (injected_bias_db(400.0, w[1], 0.0) - injected_bias_db(400.0, w[0], 0.0)).abs()
                / (w[1] - w[0])
        })
        .fold(0.0_f64, f64::max);
    let cone_change_per_knot_span = cone_rate * knot_floors.min_knot_spacing_econe;
    assert!(
        cone_change_per_knot_span < BIAS_CONE_DB,
        "cone-axis bias changes {cone_change_per_knot_span:.4} dB over one minimum knot \
         span ({} deg) — exceeds its own amplitude ({BIAS_CONE_DB} dB), not smooth \
         enough for the fitter's knots",
        knot_floors.min_knot_spacing_econe
    );

    let clock_rate = FIXTURE_CLOCK_DEG
        .windows(2)
        .map(|w| {
            (injected_bias_db(400.0, 0.0, w[1]) - injected_bias_db(400.0, 0.0, w[0])).abs()
                / (w[1] - w[0])
        })
        .fold(0.0_f64, f64::max);
    let clock_change_per_knot_span = clock_rate * knot_floors.min_knot_spacing_eclock;
    assert!(
        clock_change_per_knot_span < BIAS_CLOCK_DB,
        "clock-axis bias changes {clock_change_per_knot_span:.4} dB over one minimum \
         knot span ({} deg) — exceeds its own amplitude ({BIAS_CLOCK_DB} dB), not smooth \
         enough for the fitter's knots",
        knot_floors.min_knot_spacing_eclock
    );
}

/// Drift guard: `support::fixture_config` hardcodes the `UHF_Array_Element` parameters
/// (rather than loading `antenna_classes.yaml` itself) so that `generate_rows()` stays
/// free of file I/O and the physics config stays visibly self-contained. That means
/// nothing fails automatically if the YAML entry is edited — the drift would otherwise
/// surface much later as a confusing recovery-tolerance failure in a binary-execution
/// test, with no obvious link back to "the fixture is stale". This test closes that gap
/// by asserting the hardcoded values still match the registry, field by field.
#[test]
fn fixture_config_matches_antenna_classes_yaml() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("antenna_classes.yaml");
    let registry = AntennaClassRegistry::load_from_file(&path).unwrap_or_else(|e| {
        panic!(
            "failed to load antenna classes from {}: {e}",
            path.display()
        )
    });
    let class = registry
        .get_class(FIXTURE_CLASS)
        .unwrap_or_else(|| panic!("{FIXTURE_CLASS} missing from {}", path.display()));

    assert_eq!(
        class.geometry.diameter_m, 8.0,
        "fixture is stale: antenna_classes.yaml diameter_m for {FIXTURE_CLASS} no longer \
         matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.geometry.f_over_d, 0.45,
        "fixture is stale: antenna_classes.yaml f_over_d for {FIXTURE_CLASS} no longer \
         matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.feed.q_factor, 5.0,
        "fixture is stale: antenna_classes.yaml feed.q_factor for {FIXTURE_CLASS} no \
         longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.feed.phase_center_offset_wavelengths, 0.0,
        "fixture is stale: antenna_classes.yaml feed.phase_center_offset_wavelengths for \
         {FIXTURE_CLASS} no longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.feed.asymmetry_factor, 1.1,
        "fixture is stale: antenna_classes.yaml feed.asymmetry_factor for {FIXTURE_CLASS} \
         no longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.mesh.spacing_mm, 10.0,
        "fixture is stale: antenna_classes.yaml mesh.spacing_mm for {FIXTURE_CLASS} no \
         longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.mesh.wire_diameter_mm, 1.0,
        "fixture is stale: antenna_classes.yaml mesh.wire_diameter_mm for {FIXTURE_CLASS} \
         no longer matches support::fixture_config — update fixture_config"
    );
    assert_eq!(
        class.surface.rms_mm, NOMINAL_SURFACE_RMS_MM,
        "fixture is stale: antenna_classes.yaml surface.rms_mm for {FIXTURE_CLASS} no \
         longer matches support::NOMINAL_SURFACE_RMS_MM — update fixture_config"
    );
    assert_eq!(
        class.system_noise_temperature_k, FIXTURE_TEMPERATURE_K,
        "fixture is stale: antenna_classes.yaml system_noise_temperature_k for \
         {FIXTURE_CLASS} no longer matches support::FIXTURE_TEMPERATURE_K — update \
         fixture_config"
    );
}

// ============================================================================
// CLI end-to-end run (roadmap D12)
// ============================================================================

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

/// The fixture CSV text, computed once per test binary run and shared across every
/// `run_calibrate` call.
///
/// `generate_rows()` runs the real physics model over the fixture grid and costs ~1.4s in
/// a debug build — a large fraction of each `run_calibrate` call (~3.2s, subprocess
/// included). Every call writes an identical file (the generator is deterministic — see
/// `generator_is_deterministic`), so recomputing it per call buys nothing but wall-clock
/// time. This cache lives here, not in `support/mod.rs`: `generate_rows` and
/// `write_fixture_csv` themselves stay unmemoized and behaviorally unchanged, since
/// `generator_is_deterministic` depends on calling `generate_rows()` twice for real to
/// prove reproducibility.
fn fixture_csv() -> &'static str {
    static CSV: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CSV.get_or_init(|| rows_to_csv(&generate_rows()))
}

/// The same grid without the injected systematic bias, cached the same way.
///
/// Used only by the parameter-tuning tests: the bias is what the *correction surface* has
/// to recover, and it is ~120–360× larger than the surface-RMS signal the *tuner* has to
/// recover, which makes the two confounded on one fixture. See
/// `support::generate_rows_without_bias` for the measurement.
fn bias_free_fixture_csv() -> &'static str {
    static CSV: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CSV.get_or_init(|| rows_to_csv(&generate_rows_without_bias()))
}

/// Run the real `calibrate` binary in full mode over a freshly generated fixture.
///
/// `extra_args` appends flags such as `--validate` / `--cv-folds N`.
///
/// Asserts success internally (see below) — a future test that needs to exercise an
/// *expected* CLI failure will need a variant that returns the raw `Output` instead of
/// panicking here.
fn run_calibrate(extra_args: &[&str]) -> CalibrateRun {
    run_calibrate_on(fixture_csv(), extra_args)
}

/// As [`run_calibrate`], over a caller-supplied measurement CSV.
fn run_calibrate_on(csv: &str, extra_args: &[&str]) -> CalibrateRun {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("measurements.csv");
    let artifact = dir.path().join("antenna.bin");
    let report = dir.path().join("report.json");
    let metadata = dir.path().join("metadata.json");

    // Each call gets its own tempdir and its own copy of the file on disk — only the
    // physics evaluation behind the cached CSV is shared.
    std::fs::write(&input, csv).expect("write fixture CSV");

    // `--classes-file` defaults to `calibrate/antenna_classes.yaml`, resolved against the
    // process CWD. An integration test's CWD is the crate root, so build the path from
    // CARGO_MANIFEST_DIR to be independent of how the test binary is invoked.
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
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("the service loader must accept a freshly written full-mode artifact");

    assert_eq!(calibration.antenna_id, "d12_uhf_test");
    assert!(
        calibration.correction_surface.is_some(),
        "full mode must ship a correction surface"
    );

    // The premise `main.rs::compute_model_predictions` fits its residuals under: because a
    // full-mode artifact always carries a correction surface, the service evaluates it with
    // the uncorrected-physics terms (spillover, F7 floor) OFF, which is what
    // `with_uncorrected_physics_gates(false)` asserts there. Boresight mode has to decide
    // this per artifact; full mode gets to hard-code it, but only while this holds — so it
    // is pinned rather than assumed (roadmap D17).
    assert!(
        !calibration.physics_is_uncorrected(),
        "a full-mode artifact must present as corrected physics to the service; if full mode \
         ever ships without a correction surface, calibrate/src/main.rs must stop hard-coding \
         `with_uncorrected_physics_gates(false)` and choose per artifact the way \
         calibrate_boresight does"
    );

    // **Roadmap D23, through the real binary.** This fixture's class is
    // `UHF_Array_Element`, whose `asymmetry_factor` is 1.1 — the largest of any shipped
    // class, and the geometry the 1.20 dB worst case was measured on. Until D23 the
    // artifact had no field for it, so `calibrate` fitted residuals against an asymmetric
    // illumination and the service rebuilt the feed at the builder default of 1.0.
    //
    // The unit tests pin each producer in-process; this pins the whole path a user actually
    // exercises — CLI → postcard → the service's loader — where a positional format makes
    // "the field is there but shifted" a real failure mode that an in-process round trip
    // through the same structs cannot see.
    assert_eq!(
        calibration.physical_config.feed.asymmetry_factor, FIXTURE_ASYMMETRY_FACTOR,
        "the artifact must carry the fitting model's asymmetry_factor across the real \
         CLI → postcard → loader path"
    );
    assert_ne!(
        FIXTURE_ASYMMETRY_FACTOR, 1.0,
        "negative control: this fixture's class must be asymmetric, or the assertion above \
         passes against the very default it exists to exclude"
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

    // The correction removes essentially all of the injected bias AT THE MEASUREMENT
    // POINTS: model-only 1.3206 dB -> corrected 0.0014 dB. History: before the D15
    // endpoint fix (`bspline_basis` evaluating to zero at the exact maximum of an axis,
    // starving the last coefficient on every axis to ~0 by the ridge term) this was
    // 0.9756 dB; the fix took it to 0.0058 dB on the old 288-row grid.
    //
    // D20 (2026-08-02) grew the grid to 1728 rows and it fell further, to 0.0014 dB. That
    // is not the fit getting freer — it got *less* free, from 288 points against 600
    // coefficients to 1728 against 960. It is the fit getting more expressive where it
    // matters: on the old grid the frequency axis could place only 2 of its 4 requested
    // internal knots and the clock axis 6 of 8, because a knot must be strictly interior
    // (D19) and those axes had too few distinct values. The larger grid places all of them.
    //
    // This is RMSE at the fitted data points, so it is not by itself evidence the surface
    // is accurate between them — that is `BIAS_RECOVERY_TOLERANCE_DB` below, which D20
    // tightened 0.65 -> 0.20 dB. Both numbers moving together, rather than on-grid RMSE
    // improving alone, is what distinguishes a better-determined fit from a better-
    // interpolating one.
    //
    // The bound is an ABSOLUTE epsilon, not a proportional one: this pipeline is
    // deterministic (no `--tune-parameters`, nothing in the fit path is thread-parallel),
    // and `corrected` was measured reproducible to 4 decimal places (0.0058) across both a
    // debug and a release run on 2026-07-30, so there is no run-to-run variance on this
    // machine to size a percentage against. The +0.002 dB epsilon is therefore sized for
    // cross-platform libm ULP differences in `cos`/`sin`/`atan2` on OTHER hardware, not
    // local noise — deliberately tight because, this close to zero, a loose absolute bound
    // would hide a large proportional regression (the old +0.02 dB was 3.4x the value it
    // bounded).
    let today_corrected_rmse = 0.0014;
    let ceiling = today_corrected_rmse + 0.002;
    assert!(
        corrected < ceiling,
        "corrected RMSE regressed past the measured ceiling: \
         corrected {corrected:.4} dB vs ceiling {ceiling:.4} dB ({today_corrected_rmse:.4} dB \
         measured on 2026-08-02 (D20's 1728-row grid) + 0.002 dB for cross-platform libm \
         ULP noise)"
    );
}

// ============================================================================
// Known-answer recovery and cross-validation (roadmap D12-Task-4)
// ============================================================================

/// Tolerance on bias recovery, in dB.
///
/// UNCHANGED at 0.65 dB by the endpoint-defect fix (`bspline_basis` zero at an axis
/// maximum, starving the topmost coefficient on every axis — see
/// docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md). The four probe
/// errors reproduced bit-for-bit before and after: (450, 3, 30) 0.5928 dB — the worst
/// case, nearest the main lobe; (550, 7, 120) 0.0934 dB; (570, 14, 200) 0.0365 dB; (500,
/// 10, 260) 0.0934 dB. They didn't move because these probes are deliberately off-grid
/// AND interior — none sits in the topmost knot span of any axis that the fix touched
/// (see the probe-placement comment in the test below), so their coefficients were never
/// starved.
///
/// UNCHANGED again at 0.65 dB by roadmap D19 (2026-08-02), which stopped adaptive knot
/// placement from landing internal knots on the axis bounds. All four probe errors below
/// reproduced bit-for-bit across that change too, for a reason worth stating: the knots D19
/// removed were duplicates of a bound, and the basis functions they created had zero-width
/// support — identically zero everywhere, so they contributed nothing to any evaluation.
/// D19 shrank the declared coefficient count from 960 to 600 without moving the surface.
///
/// What actually limits recovery here is overfitting from an underdetermined fit: the
/// shipped configuration has 600 coefficients (6·10·10 after D19) fitting only 288
/// measurement points, so the surface can interpolate every data point almost exactly
/// (`corrected_rmse` 0.0058 dB, see the ceiling assertion above) while oscillating between
/// them — which is exactly what these off-grid probes are catching. That is tracked
/// separately from the endpoint defect, as roadmap unit **D20**: the fitter's
/// data-sufficiency check tests `(spline_order+1)^3 = 125` points as the minimum, when the
/// real requirement is the coefficient count. See
/// docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md, which records this.
/// This tolerance can only tighten once that is addressed; it is deliberately left
/// unchanged by D15 and D19 alike.
///
/// **Tightened 0.65 → 0.20 dB by roadmap D20 (2026-08-02)**, which is what this unit
/// existed to do. Growing the fixture from 288 to 1728 rows made the fit determined (1728
/// points against 960 coefficients; the tightest CV fold sees 1152), and the worst probe
/// error fell **0.5928 → 0.1226 dB, a 4.8× improvement**, confirming the diagnosis: the
/// residual was interpolation error from an underdetermined system, not a limit of the
/// physics or of the correction surface's expressiveness.
///
/// All four probe values reproduce bit-for-bit across debug and release builds and across
/// repeat runs. 0.20 dB is set with headroom above the measured 0.1226 dB worst case —
/// enough to absorb run-to-run libm noise without flaking — while staying far below the
/// injected bias's own 0.7529–1.4529 dB range at these probes, so a surface that fitted
/// nothing would still fail this test.
///
/// The remaining 0.1226 dB is concentrated at one probe (450 MHz, 3° cone — the others sit
/// at 0.0373 / 0.0232 / 0.0433 dB) and is *not* explained by underdetermination any more.
/// Fitting the injected bias alone, with no physics residual, recovers it to 0.004 dB, so
/// the gap is the part of the residual that is not the bias: the 2.0 → 2.6 mm surface-RMS
/// perturbation, whose contribution varies fastest near the main lobe. That is a fixture
/// property, not a defect — but it is the thing to look at first if this number moves.
const BIAS_RECOVERY_TOLERANCE_DB: f64 = 0.20;

#[test]
fn cli_full_mode_recovers_the_injected_bias() {
    let run = run_calibrate(&[]);
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("load artifact");
    let surface = calibration
        .correction_surface
        .as_ref()
        .expect("full mode must ship a correction surface");

    // Interior probe points only — off-grid but each one sits below the topmost knot
    // span of every axis, deliberately. Originally this was because the correction
    // collapsed to ~0 across that span (a `bspline_basis` defect at the exact maximum of
    // an axis); that was fixed 2026-07-30 in a866cfb, and the basis is now a partition of
    // unity at every boundary, so it's history, not a live hazard here.
    //
    // Staying interior is still worthwhile post-fix, for a different reason: off-grid
    // accuracy is limited by the underdetermined fit (600 coefficients, 288 points — see
    // `BIAS_RECOVERY_TOLERANCE_DB` above), and a probe placed exactly at a boundary would
    // conflate boundary behavior with that interpolation error. Keeping the probes
    // interior keeps this test measuring one thing. Concretely, the fitted frequency knot
    // vector is [400, 400, 400, 400, 500, 600, 700, 700, 700, 700], so its topmost span
    // is [600, 700] MHz — every probe's frequency here is <= 570 MHz, clear of it. The
    // cone axis's topmost span is [20, 24] deg (probes <= 14 deg) and the clock axis's is
    // [270, 315] deg (probes <= 260 deg).
    //
    // Those bounds now repeat exactly `order` = 4 times. Until roadmap D19 (2026-08-02)
    // they repeated 5 times on the frequency and clock axes, because adaptive quantile
    // placement put an internal knot ON each bound; the extra copy gave the first and last
    // basis function of those axes a zero-width support, so they were identically zero.
    // Removing them left every value below unmoved, bit-for-bit — a basis function that is
    // zero everywhere contributes zero to every evaluation — while dropping the declared
    // coefficient count from 960 to 600.
    let probes = [
        (450.0_f64, 3.0_f64, 30.0_f64),
        (550.0, 7.0, 120.0),
        (570.0, 14.0, 200.0),
        (500.0, 10.0, 260.0),
    ];

    let mut worst = 0.0_f64;
    for (frequency_mhz, e_cone_deg, e_clock_deg) in probes {
        // Parameters are named azimuth/elevation but the 3D->4D bridge maps
        // clock -> azimuth, cone -> elevation (artifact_export.rs:482). Clock first.
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
    assert!(
        report["corrected_rmse"].as_f64().is_some(),
        "corrected_rmse missing from report despite --validate being off; report was:\n{report:#}"
    );
    assert!(
        report["main_lobe_max_error"].as_f64().is_some(),
        "main_lobe_max_error missing from report despite --validate being off; report was:\n{report:#}"
    );
    assert!(
        report["first_sidelobe_max_error"].as_f64().is_some(),
        "first_sidelobe_max_error missing from report despite --validate being off; \
         report was:\n{report:#}"
    );
}

// ============================================================================
// Tuned end-to-end run and its CI status (roadmap D12 Task 5)
// ============================================================================

/// End-to-end run WITH parameter tuning: the tuner must recover the perturbed surface RMS.
///
/// Runs against the **bias-free** fixture, deliberately. The biased fixture that the rest
/// of this file uses cannot support this assertion, and the reason is quantitative rather
/// than incidental: the injected bias averages +1.22 dB, while the whole 2.0 → 2.6 mm
/// surface-RMS perturbation is worth 0.003 dB at 400 MHz and 0.010 dB at 700 MHz (Ruze
/// loss `exp(-(4πσ/λ)²)` with λ = 43–75 cm). The bias is 120–360× larger and has the same
/// near-constant shape, so it is confounded with surface RMS, and minimising RMSE against
/// it drives surface RMS to its *lower* bound. That is not a hypothesis: with the
/// degenerate-simplex crash fixed and the biased fixture still in place, this test reported
/// **0.1 mm against a 2.6 mm truth**. See `support::generate_rows_without_bias`.
///
/// The bias is what the *correction surface* has to recover, and the correction-surface
/// assertions above keep using it. Two known answers, two fixtures.
///
/// **History.** This test was `#[ignore]`d on 2026-07-30 by roadmap unit D12 because
/// `--tune-parameters` crashed deterministically — `tune_parameters` seeded `NelderMead`
/// with a single vertex where N+1 are required, so argmin underflowed `usize` computing
/// `params[num_param_vecs - 2]`. Fixed 2026-07-31 (`build_initial_simplex`), along with two
/// others found while un-ignoring it: the class-agnostic `ParameterBounds` that put every
/// `UHF_Array_Element` tunable exactly on its cap, and the tuner evaluating its objective
/// under `IntegrationParams::fast()` while the pipeline computed residuals under
/// `default()` — a mismatch reaching 0.088 dB at 24° cone, 26× the signal being fitted.
///
/// Iterations are held low: each Nelder-Mead evaluation runs the physics model over all
/// 288 fixture points, and the objective now uses the denser `default()` integrator, so
/// this run costs ~20 s in a debug build. Four iterations suffice — measured 2026-07-31,
/// the tuner lands on 2.6000 mm from its 2.0 mm start.
#[test]
fn cli_tuned_run_recovers_the_surface_rms_perturbation() {
    let start = std::time::Instant::now();
    let run = run_calibrate_on(
        bias_free_fixture_csv(),
        &["--tune-parameters", "--max-tuning-iterations", "4"],
    );
    let elapsed = start.elapsed();

    println!("tuned end-to-end run took {:.1} s", elapsed.as_secs_f64());

    let metadata: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&run.metadata).expect("read --metadata sidecar"),
    )
    .expect("parse --metadata sidecar");
    assert_eq!(
        metadata["parameters_tuned"], true,
        "the metadata sidecar should record that tuning ran"
    );

    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("load artifact");

    // The data-layer ReflectorGeometry stores millimetres already — no conversion.
    let tuned_rms = calibration.physical_config.reflector.surface_rms_mm;
    println!(
        "surface RMS: nominal {NOMINAL_SURFACE_RMS_MM} mm, truth \
         {PERTURBED_SURFACE_RMS_MM} mm, tuned {tuned_rms:.4} mm"
    );
    // Tolerance is 25% of the 0.6 mm perturbation. The measured recovery is exact to four
    // decimals on macOS/aarch64, so this is slack for platform floating-point differences,
    // not for a sloppy fit — if this ever needs widening, the reason is a finding, not a
    // tolerance to adjust.
    const TOLERANCE_MM: f64 = 0.15;
    assert!(
        (tuned_rms - PERTURBED_SURFACE_RMS_MM).abs() < TOLERANCE_MM,
        "the tuner should recover the perturbed truth {PERTURBED_SURFACE_RMS_MM} mm from \
         its {NOMINAL_SURFACE_RMS_MM} mm nominal start, within {TOLERANCE_MM} mm; \
         got {tuned_rms:.4} mm"
    );
}

/// `--tune-parameters` must survive every `--tuning-mode`, i.e. N = 1, 2 and 3 parameters.
///
/// This is the CLI-level half of the acceptance criteria in
/// `docs/findings-2026-07-30-full-mode-parameter-tuning-broken.md`: the degenerate-simplex
/// crash was independent of tuning mode, so the fix has to be proven across all three. The
/// cheap library-level equivalent lives in `parameter_tuner.rs`
/// (`tune_parameters_completes_for_every_tuning_mode`, 5 measurement points); this one adds
/// the arg-parsing and artifact-writing layers on the real 288-point fixture.
///
/// Deliberately asserts *completion*, not recovery. Recovery is what
/// `cli_tuned_run_recovers_the_surface_rms_perturbation` above is for, and it remains
/// `#[ignore]`d for reasons this test does not touch.
///
/// One iteration each — enough for argmin to build and step the simplex, which is where the
/// crash was.
#[test]
fn cli_tuned_run_completes_for_every_tuning_mode() {
    for mode in ["surface-only", "surface-and-mesh", "all"] {
        let start = std::time::Instant::now();
        let run = run_calibrate(&[
            "--tune-parameters",
            "--tuning-mode",
            mode,
            "--max-tuning-iterations",
            "1",
        ]);
        println!(
            "--tuning-mode {mode} completed in {:.1} s",
            start.elapsed().as_secs_f64()
        );

        // `run_calibrate` already asserts exit status; this pins that tuning actually ran
        // rather than being silently skipped.
        let metadata: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&run.metadata).expect("read --metadata sidecar"),
        )
        .expect("parse --metadata sidecar");
        assert_eq!(
            metadata["parameters_tuned"], true,
            "--tuning-mode {mode} should record that tuning ran"
        );

        antenna_model::data::loader::load_calibration_artifact(&run.artifact).unwrap_or_else(|e| {
            panic!("--tuning-mode {mode} produced an unloadable artifact: {e}")
        });
    }
}
