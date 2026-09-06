//! End-to-end test of the `calibrate` binary in full mode, on perturbed-truth data.

mod support;

use antenna_core::AntennaCalibration;
use calibrate::{AntennaClassRegistry, CorrectionSurfaceParams};
use std::path::{Path, PathBuf};
use std::process::Command;
use support::*;

#[test]
fn generator_is_deterministic() {
    let frequencies_mhz = [410.0, 635.0];
    let e_cone_deg = [1.0, 9.0];
    let e_clock_deg = [15.0, 125.0, 280.0];

    let grid = FixtureGrid {
        frequencies_mhz: &frequencies_mhz,
        e_cone_deg: &e_cone_deg,
        e_clock_deg: &e_clock_deg,
    };
    let first = generate_rows_on_grid(grid);
    let second = generate_rows_on_grid(grid);

    let ordered_coordinates: Vec<_> = first
        .iter()
        .map(|row| (row.frequency_mhz, row.e_cone_deg, row.e_clock_deg))
        .collect();
    assert_eq!(
        ordered_coordinates,
        [
            (410.0, 1.0, 15.0),
            (410.0, 1.0, 125.0),
            (410.0, 1.0, 280.0),
            (410.0, 9.0, 15.0),
            (410.0, 9.0, 125.0),
            (410.0, 9.0, 280.0),
            (635.0, 1.0, 15.0),
            (635.0, 1.0, 125.0),
            (635.0, 1.0, 280.0),
            (635.0, 9.0, 15.0),
            (635.0, 9.0, 125.0),
            (635.0, 9.0, 280.0),
        ],
        "the parallel generator must retain frequency/cone/clock nesting order"
    );
    assert_eq!(first, second, "ordered generated rows must be reproducible");
    assert_eq!(
        rows_to_csv(&first),
        rows_to_csv(&second),
        "rendered fixture CSV must be byte-reproducible"
    );
}

#[test]
fn generator_grid_satisfies_the_fitter_constraints() {
    const CV_FOLDS: usize = 3;

    struct Axis<'a> {
        label: &'static str,
        values: &'a [f64],
        requested_knots: usize,
        minimum_spacing: f64,
    }

    let params = CorrectionSurfaceParams::shipped();
    let axes = [
        Axis {
            label: "frequency",
            values: &FIXTURE_FREQUENCIES_MHZ,
            requested_knots: params.num_knots_frequency,
            minimum_spacing: params.min_knot_spacing_frequency,
        },
        Axis {
            label: "E-cone",
            values: &FIXTURE_CONE_DEG,
            requested_knots: params.num_knots_econe,
            minimum_spacing: params.min_knot_spacing_econe,
        },
        Axis {
            label: "E-clock",
            values: &FIXTURE_CLOCK_DEG,
            requested_knots: params.num_knots_eclock,
            minimum_spacing: params.min_knot_spacing_eclock,
        },
    ];

    for axis in &axes {
        let cardinality = axis.values.len();
        assert!(
            cardinality >= axis.requested_knots + 2,
            "{} needs at least {} interior values plus two bounds; got {cardinality} values",
            axis.label,
            axis.requested_knots
        );

        let span = axis.values[cardinality - 1] - axis.values[0];
        let required_span = axis.requested_knots as f64 * axis.minimum_spacing;
        assert!(
            span >= required_span,
            "{} span {span} is too narrow for {} shipped knots at {} minimum spacing; \
             need at least {required_span}",
            axis.label,
            axis.requested_knots,
            axis.minimum_spacing
        );
    }

    // The binding quantity is the fitted coefficient count, not the cheap
    // `(spline_order + 1)^3` pre-check. The largest held-out fold determines the smallest
    // training split, so use the ceiling rather than assuming the row count divides evenly.
    let maximum_coefficients = axes
        .iter()
        .map(|axis| axis.requested_knots + params.spline_order)
        .product::<usize>();
    let row_count = axes.iter().map(|axis| axis.values.len()).product::<usize>();
    let minimum_training_rows = row_count - row_count.div_ceil(CV_FOLDS);
    assert!(
        minimum_training_rows >= maximum_coefficients,
        "a {CV_FOLDS}-fold CV training split has only {minimum_training_rows} of \
         {row_count} rows for up to {maximum_coefficients} shipped coefficients"
    );
}

fn assert_fixture_grid_dimensions_and_order(rows: &[FixtureRow]) {
    assert_eq!(rows.len(), FIXTURE_ROW_COUNT, "full fixture row count");

    let rows_per_frequency = FIXTURE_CONE_DEG.len() * FIXTURE_CLOCK_DEG.len();
    for (index, row) in rows.iter().enumerate() {
        let frequency_index = index / rows_per_frequency;
        let within_frequency = index % rows_per_frequency;
        let cone_index = within_frequency / FIXTURE_CLOCK_DEG.len();
        let clock_index = within_frequency % FIXTURE_CLOCK_DEG.len();
        assert_eq!(
            (row.frequency_mhz, row.e_cone_deg, row.e_clock_deg),
            (
                FIXTURE_FREQUENCIES_MHZ[frequency_index],
                FIXTURE_CONE_DEG[cone_index],
                FIXTURE_CLOCK_DEG[clock_index],
            ),
            "full fixture coordinate order at row {index}"
        );
    }
}

/// Standing pin on roadmap unit D11: the fixture must contain rows the pre-D11 parser
/// discarded (it rejected anything below -20 dB/K as "atypical G/T", which is a boresight
/// figure, silently dropping legitimate sidelobe measurements).
fn assert_fixture_has_realistic_sub_minus_twenty_sidelobes(rows: &[FixtureRow]) {
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
    let knot_floors = CorrectionSurfaceParams::shipped();

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

/// Drift guard between the actual nominal fixture and `antenna_classes.yaml`.
///
/// The fixture remains free of file I/O, so compare the configuration it constructs—not a
/// second list of literals—with the registry. Core geometry uses SI units while the class
/// registry stores surface and mesh dimensions in millimetres; those conversions are kept
/// explicit here to pin that boundary.
#[test]
fn fixture_config_matches_antenna_classes_yaml() {
    const MILLIMETRES_PER_METRE: f64 = 1_000.0;

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
    let fixture = fixture_config(NOMINAL_SURFACE_RMS_MM);
    let mesh = fixture
        .mesh
        .as_ref()
        .expect("the fixture class uses a mesh reflector");

    assert_eq!(fixture.id, class.class_id, "fixture class id");
    assert_eq!(fixture.name, class.description, "fixture description");
    assert_eq!(
        fixture.reflector.diameter, class.geometry.diameter_m,
        "fixture reflector diameter"
    );
    assert_eq!(
        fixture.reflector.f_over_d(),
        class.geometry.f_over_d,
        "fixture reflector f/D"
    );
    assert_eq!(
        fixture.reflector.surface_rms * MILLIMETRES_PER_METRE,
        class.surface.rms_mm,
        "fixture surface RMS (m converted to mm)"
    );
    assert_eq!(fixture.feed.q_factor, class.feed.q_factor, "fixture feed q");
    // The shipped class records zero, the only offset invariant under wavelengths-to-metres
    // conversion across this multi-frequency fixture.
    assert_eq!(
        fixture.feed.phase_center_offset, class.feed.phase_center_offset_wavelengths,
        "fixture phase-centre offset"
    );
    assert_eq!(
        fixture.feed.asymmetry_factor, class.feed.asymmetry_factor,
        "fixture feed asymmetry"
    );
    assert_eq!(
        mesh.spacing * MILLIMETRES_PER_METRE,
        class.mesh.spacing_mm,
        "fixture mesh spacing (m converted to mm)"
    );
    assert_eq!(
        mesh.wire_diameter * MILLIMETRES_PER_METRE,
        class.mesh.wire_diameter_mm,
        "fixture wire diameter (m converted to mm)"
    );
    assert_eq!(
        class.system_noise_temperature_k, FIXTURE_TEMPERATURE_K,
        "fixture system noise temperature"
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

/// Run the real `calibrate` binary in full mode over caller-owned fixture rows.
///
/// Keeping generation outside this imperative shell exposes the exact rows to every named
/// scenario assertion. Each nextest process owns its scenario; there is no persistent or
/// cross-process fixture cache that can outlive a physics-model change (GitHub issue #68).
/// `extra_args` appends flags such as `--validate` / `--cv-folds N`.
fn run_calibrate(rows: &[FixtureRow], extra_args: &[&str]) -> CalibrateRun {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("measurements.csv");
    let artifact = dir.path().join("antenna.bin");
    let report = dir.path().join("report.json");
    let metadata = dir.path().join("metadata.json");

    std::fs::write(&input, rows_to_csv(rows)).expect("write fixture CSV");

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

/// Immutable outputs shared by every assertion in the untuned full-mode scenario.
struct UntunedScenario {
    rows: Vec<FixtureRow>,
    run: CalibrateRun,
    artifact_bytes: Vec<u8>,
    calibration: AntennaCalibration,
    report: serde_json::Value,
}

fn run_untuned_scenario() -> UntunedScenario {
    let rows = generate_rows();
    let run = run_calibrate(&rows, &[]);
    let artifact_bytes = std::fs::read(&run.artifact).expect("read untuned artifact");

    // The scenario must cross the SERVICE loader boundary, not just calibrate's own
    // round-trip code.
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("the service loader must accept a freshly written full-mode artifact");
    let report = run.report_json();

    UntunedScenario {
        rows,
        run,
        artifact_bytes,
        calibration,
        report,
    }
}

#[test]
fn cli_untuned_scenario_preserves_artifact_correction_and_no_validation() {
    let scenario = run_untuned_scenario();

    assert_fixture_grid_dimensions_and_order(&scenario.rows);
    assert_fixture_has_realistic_sub_minus_twenty_sidelobes(&scenario.rows);
    assert_service_loadable_corrected_artifact(&scenario);
    assert_correction_beats_the_uncorrected_model(&scenario);
    assert_injected_bias_recovery(&scenario);
    assert_cross_validation_was_not_requested(&scenario);
}

fn assert_service_loadable_corrected_artifact(scenario: &UntunedScenario) {
    let bytes = &scenario.artifact_bytes;
    assert!(
        bytes.starts_with(b"ANTC"),
        "artifact export: missing the ANTC magic; first bytes: {:?}",
        &bytes[..bytes.len().min(8)]
    );

    let calibration = &scenario.calibration;
    assert_eq!(
        calibration.antenna_id, "d12_uhf_test",
        "service load: antenna ID"
    );
    assert_eq!(
        calibration.metadata.num_measurements,
        scenario.rows.len(),
        "artifact metadata must preserve the parsed fixture measurement count"
    );
    let coverage = calibration
        .calibration_coverage
        .as_ref()
        .expect("full-mode artifact must describe its measurement coverage");
    assert_eq!(
        coverage.num_measurements,
        scenario.rows.len(),
        "service-loaded coverage must preserve the parsed fixture measurement count"
    );
    assert!(
        calibration.correction_surface.is_some(),
        "artifact export: full mode must ship a correction surface"
    );

    // The premise `main.rs::compute_model_predictions` fits its residuals under: because a
    // full-mode artifact always carries a correction surface, the service evaluates it with
    // the uncorrected-physics terms (spillover, F7 floor) OFF, which is what
    // `with_uncorrected_physics_gates(false)` asserts there. Boresight mode has to decide
    // this per artifact; full mode gets to hard-code it, but only while this holds — so it
    // is pinned rather than assumed (roadmap D17).
    assert!(
        !calibration.physics_is_uncorrected(),
        "corrected-physics classification: a full-mode artifact must present as corrected \
         physics to the service; if full mode ever ships without a correction surface, \
         calibrate/src/main.rs must stop hard-coding \
         `with_uncorrected_physics_gates(false)` and choose per artifact the way \
         calibrate_boresight does"
    );

    // **Roadmap D23, through the real binary.** This fixture's class is
    // `UHF_Array_Element`, whose `asymmetry_factor` is 1.1 — the largest of any shipped
    // class, and the geometry the 1.20 dB worst case was measured on. Until D23 the
    // artifact had no field for it, so `calibrate` fitted residuals against an asymmetric
    // illumination and the service rebuilt the feed at the builder default of 1.0.
    assert_eq!(
        calibration.physical_config.feed.asymmetry_factor, FIXTURE_ASYMMETRY_FACTOR,
        "non-default asymmetry: the artifact must carry the fitting model's \
         asymmetry_factor across the real CLI → postcard → service-loader path"
    );
    assert_ne!(
        FIXTURE_ASYMMETRY_FACTOR, 1.0,
        "non-default asymmetry negative control: this fixture must be asymmetric"
    );
}

fn assert_correction_beats_the_uncorrected_model(scenario: &UntunedScenario) {
    let report = &scenario.report;
    let model_only = report["model_only_rmse"].as_f64().expect("model_only_rmse");
    let corrected = report["corrected_rmse"].as_f64().expect("corrected_rmse");

    println!("model-only RMSE {model_only:.4} dB, corrected {corrected:.4} dB");
    assert!(
        corrected < model_only,
        "correction quality: the correction surface must improve on the physics model: \
         corrected {corrected:.4} dB vs model-only {model_only:.4} dB"
    );

    // The correction removes essentially all of the injected bias AT THE MEASUREMENT
    // POINTS: model-only 1.3206 dB -> corrected 0.0014 dB. D20 grew the grid to 1728 rows
    // and made the 960-coefficient fit determined. This in-sample bound complements, rather
    // than replaces, the off-grid probes below.
    const CORRECTED_RMSE_CEILING_DB: f64 = 0.0034;
    assert!(
        corrected <= CORRECTED_RMSE_CEILING_DB,
        "corrected RMSE: {corrected:.4} dB exceeds the measured 0.0014 dB plus 0.002 dB \
         cross-platform libm allowance ({CORRECTED_RMSE_CEILING_DB:.4} dB ceiling)"
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

fn assert_injected_bias_recovery(scenario: &UntunedScenario) {
    let surface = scenario
        .calibration
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

        assert!(
            got.is_finite(),
            "off-grid bias recovery: probe f={frequency_mhz} cone={e_cone_deg} \
             clock={e_clock_deg} produced non-finite correction {got}"
        );

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

/// D10's standing CLI pin. K=3 is the non-default validated scenario; multiple requested
/// counts are covered cheaply at the parsed-arguments production boundary in `main.rs`.
#[test]
fn cli_cv_three_folds_reports_finite_scores() {
    const FOLDS: usize = 3;
    let rows = generate_rows();
    let run = run_calibrate(&rows, &["--validate", "--cv-folds", "3"]);
    let report = run.report_json();

    let reported = report["cross_validation"]["num_folds"]
        .as_u64()
        .unwrap_or_else(|| {
            panic!(
                "--validate --cv-folds {FOLDS} should produce a cross-validation \
                 section; report was:\n{report:#}"
            )
        });
    assert_eq!(reported as usize, FOLDS, "requested CV fold count");

    let values = report["cross_validation"]["fold_rmse_values"]
        .as_array()
        .expect("fold_rmse_values");
    assert_eq!(values.len(), FOLDS, "one RMSE per fold");
    for (index, value) in values.iter().enumerate() {
        let score = value
            .as_f64()
            .unwrap_or_else(|| panic!("CV fold {index} score is not numeric: {value}"));
        assert!(
            score.is_finite(),
            "CV fold {index} score is not finite: {score}"
        );
    }
}

/// Without `--validate`, step 6 must not cross-validate — but the numerical quality report
/// must still be present. This helper deliberately shares the untuned scenario's one CLI run.
fn assert_cross_validation_was_not_requested(scenario: &UntunedScenario) {
    let report = &scenario.report;

    assert!(
        report["cross_validation"].is_null(),
        "no-validation behavior: cross-validation ran without --validate; report was:\n{report:#}"
    );
    assert!(
        !scenario.run.output().contains("cross-validation"),
        "no-validation behavior: the binary announced cross-validation on an untuned run:\n{}",
        scenario.run.output()
    );

    for field in [
        "corrected_rmse",
        "main_lobe_max_error",
        "first_sidelobe_max_error",
    ] {
        let value = report[field].as_f64().unwrap_or_else(|| {
            panic!(
                "no-validation quality report: {field} is not numeric despite --validate \
                 being off; report was:\n{report:#}"
            )
        });
        assert!(
            value.is_finite(),
            "no-validation quality report: {field} is non-finite: {value}"
        );
    }
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
/// Iterations are held low: each Nelder-Mead evaluation runs the physics model over every
/// fixture point (1728 since D20), and the objective uses the denser `default()` integrator.
/// Four iterations suffice — measured 2026-07-31, the tuner lands on 2.6000 mm from its
/// 2.0 mm start.
///
/// **Cost, re-measured 2026-08-17 under D18 task 3 (parallel per-point sweep, plus D31's memo
/// of the initial evaluation): 330 s → 186 s in a full-binary run, and 57 s standalone.** Both
/// pairs are like-for-like; do not compare the standalone figure against the contended one, as
/// an earlier draft of this line did. The tuned value is unchanged — 2.6000 mm before and
/// after — because the parallel sweep collects in index order and reduces serially, and D31
/// memoizes rather than deletes; see the bit-identity notes in `parameter_tuner.rs`. The old
/// figure this line carried, "~20 s", was measured on the 288-row grid and was already
/// optimistic by an order of magnitude before any of this.
#[test]
fn cli_tuned_run_recovers_the_surface_rms_perturbation() {
    let start = std::time::Instant::now();
    let rows = generate_rows_without_bias();
    let run = run_calibrate(
        &rows,
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
