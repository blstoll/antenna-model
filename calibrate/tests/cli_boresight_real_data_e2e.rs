//! Roadmap D13 — end-to-end `calibrate` boresight run over **real published measurements**.
//!
//! Companion to `cli_boresight_mode_e2e.rs`, which drives the same code path with synthetic
//! sweeps chosen to sit either side of the correction-fit threshold. This file swaps those for
//! NTIA Report 84-164's measured earth-station gains and asserts what a synthetic fixture
//! cannot: that the physics model, tuned by the real binary against real data, lands within a
//! stated tolerance of numbers somebody actually measured.
//!
//! Two fixtures, both real, covering both branches of the boresight path:
//!
//! * **Andrew 43998, 10 m, six frequencies 3700–6425 MHz.** The tuner reconciles all six
//!   published gains to well under the 0.5 dB correction-fit threshold, so no correction
//!   surface is fitted and the served value is *tuned physics alone*.
//! * **Scientific-Atlanta 8002A, 10 m, five frequencies 3700–6175 MHz.** Its published
//!   transmit-band gain is inconsistent with its receive-band gains under any single-reflector
//!   model (almost certainly a different feed), so the residual stays above the threshold and a
//!   frequency correction surface is fitted, attached, and — the assertion that matters —
//!   actually reached on the served path.
//!
//! Provenance and every added assumption live in the fixture headers themselves. Read
//! `tests/fixtures/ntia_84_164_*_boresight.csv` before changing any number here.
//!
//! # What this file measured, and the defect it records
//!
//! `calibrate`'s boresight objective evaluates the physics with `IntegrationParams::default()`,
//! whose `apply_spillover` is **false**. The service turns spillover **on** for exactly those
//! artifacts that carry no correction surface (`evaluator.rs`: `integration_params
//! .apply_spillover = calibration.physics_is_uncorrected()`). A boresight artifact with no
//! frequency correction is therefore *served with a loss term its own calibration never saw* —
//! here a constant −0.326 dB across the whole sweep. Strip that term back off and the served
//! values reproduce the published gains at 0.083 dB RMSE, exactly the tuner's own reported
//! figure. Both halves are pinned below
//! (`andrew_43998_served_gain_lands_within_tolerance_of_the_published_gains` and
//! `andrew_43998_matches_the_published_gains_exactly_once_the_spillover_term_is_removed`), so
//! the gap cannot widen unnoticed and closing it will be visible as a test failure rather than
//! a silent improvement. Filing, not fixing, is deliberate: the fix changes what the tuner
//! optimizes and therefore every boresight artifact's parameters, which is its own unit.

use antenna_model::api::schemas::{GainRequest, GainResponse, Position3D};
use antenna_model::data::repository::CalibrationRepository;
use antenna_model::data::types::{
    AntennaCalibration, CalibrationStatus, CALIBRATION_SCHEMA_VERSION,
};
use antenna_model::service::compute_gain_from_request;
use antenna_model::warnings::WarningCode;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

// ============================================================================
// The published rows
// ============================================================================
//
// Restated here so that an edit to a fixture CSV cannot silently move the values the
// assertions are made against: the fixture and this table have to be changed together or the
// `..._fixture_matches_the_published_table` tests below fail.

/// Assumed system noise temperature baked into both fixtures' `temperature_k` column.
///
/// `g_over_t_db = gain_dbi − 10·log10(T_SYS_K)` exactly, and the model's own
/// `G/T = gain − 10·log10(temperature_k)` inverts it exactly, so the assumption cancels and
/// the fit runs against the published gains themselves. See the fixture headers: this value is
/// an assumption of the fixture, not published data.
const T_SYS_K: f64 = 100.0;

/// Andrew 43998, 10 m: (frequency MHz, published gain dBi). NTIA 84-164 tables A-2 and A-3.
const ANDREW_PUBLISHED: [(f64, f64); 6] = [
    (3700.0, 50.4),
    (3950.0, 51.0),
    (4200.0, 51.4),
    (5925.0, 54.0),
    (6175.0, 54.3),
    (6425.0, 54.5),
];

/// Scientific-Atlanta 8002A, 10 m: (frequency MHz, published gain dBi). Tables A-2 and A-3.
const SA_PUBLISHED: [(f64, f64); 5] = [
    (3700.0, 50.6),
    (3950.0, 50.8),
    (4000.0, 50.8),
    (4200.0, 51.0),
    (6175.0, 50.8),
];

/// Convert a published gain to the `g_over_t_db` column value the fixture must carry.
fn published_gain_to_g_over_t(gain_dbi: f64) -> f64 {
    gain_dbi - 10.0 * T_SYS_K.log10()
}

// ============================================================================
// Tolerances — every one measured, not wished for
// ============================================================================

/// Served-vs-published tolerance for the **uncorrected** (Andrew) artifact, in dB.
///
/// Measured worst case is 0.483 dB at 3950 MHz, of which a constant 0.326 dB is the
/// spillover term the tuner never saw (see the module header) and the rest is the tuner's own
/// 0.083 dB RMSE against real data. 0.75 dB keeps ~35% headroom while staying inside both the
/// project's <1 dB main-lobe requirement and the artifact's own ±1.5 dB accuracy claim, so a
/// pass here is still a meaningful statement about accuracy.
const ANDREW_SERVED_TOLERANCE_DB: f64 = 0.75;

/// Per-point tolerance once the spillover term is removed from the served value, in dB.
///
/// Measured worst case 0.157 dB at 3950 MHz; the six residuals reproduce the tuner's reported
/// 0.0828 dB RMSE to the digit. 0.25 dB is the nearest round number above the measurement.
const ANDREW_SPILLOVER_FREE_TOLERANCE_DB: f64 = 0.25;

/// Served-vs-published tolerance for the **corrected** (SA 8002A) artifact, in dB.
///
/// Measured worst case 0.055 dB at 3950 MHz: the fitted frequency correction absorbs the
/// residual almost exactly at the measured frequencies, which is what a correction surface
/// evaluated at its own knots should do. 0.25 dB is deliberately far tighter than the Andrew
/// figure — this artifact carries a correction, so the service leaves spillover off and there
/// is no calibrate/service model mismatch to accommodate. If this ever needs loosening, the
/// correction is no longer reaching the served path.
const SA_SERVED_TOLERANCE_DB: f64 = 0.25;

// ============================================================================
// Running the real binary over the committed fixtures
// ============================================================================

struct RealDataRun {
    stdout: String,
    stderr: String,
    artifact: PathBuf,
    antenna_id: String,
    feed_id: String,
    // Kept alive so the temp dir outlives the artifact path above.
    _dir: tempfile::TempDir,
}

impl RealDataRun {
    fn output(&self) -> String {
        format!(
            "--- stdout ---\n{}\n--- stderr ---\n{}",
            self.stdout, self.stderr
        )
    }

    /// Load through the **service's** loader, not calibrate's own round-trip code.
    fn load(&self) -> AntennaCalibration {
        antenna_model::data::loader::load_calibration_artifact(&self.artifact).unwrap_or_else(|e| {
            panic!(
                "the service loader must accept the artifact calibrate just wrote: {e}\n{}",
                self.output()
            )
        })
    }
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Run the real `calibrate` binary in boresight mode over a committed real-data fixture.
///
/// The fixture is handed to the binary **as committed**, provenance block and all. A data file
/// whose own documentation stops it from being run is a file nobody can re-derive the numbers
/// from, which is why `BoresightMeasurements::from_csv` skips `#` lines.
fn run_over_fixture(csv: &str, specs: &str, antenna_id: &str, feed_id: &str) -> RealDataRun {
    let dir = tempfile::tempdir().expect("temp dir");
    let artifact = dir.path().join("antenna.bin");

    let input = fixture(csv);
    let specs_path = fixture(specs);
    assert!(input.exists(), "fixture CSV missing: {}", input.display());
    assert!(
        specs_path.exists(),
        "design specs missing: {}",
        specs_path.display()
    );

    let out = Command::new(env!("CARGO_BIN_EXE_calibrate"))
        .args(["--calibration-mode", "boresight"])
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&artifact)
        .args(["--antenna-id", antenna_id])
        .args(["--feed-id", feed_id])
        .arg("--design-specs")
        .arg(&specs_path)
        .output()
        .expect("run the calibrate binary");

    let run = RealDataRun {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        artifact,
        antenna_id: antenna_id.to_string(),
        feed_id: feed_id.to_string(),
        _dir: dir,
    };

    assert!(
        out.status.success(),
        "calibrate exited with {:?}\n{}",
        out.status.code(),
        run.output()
    );

    run
}

/// Each fixture is calibrated **once** per test binary and shared. The tuner runs ~100
/// Nelder-Mead iterations over a 10 m dish at up to 6.4 GHz (D/λ = 214); paying that per test
/// would multiply the debug-profile cost of this file by the number of assertions it makes.
fn andrew() -> &'static RealDataRun {
    static RUN: OnceLock<RealDataRun> = OnceLock::new();
    RUN.get_or_init(|| {
        run_over_fixture(
            "ntia_84_164_andrew_43998_10m_boresight.csv",
            "ntia_andrew_43998_10m_design_specs.yaml",
            "ntia_and_43998_10m",
            "c_band",
        )
    })
}

fn sa_8002a() -> &'static RealDataRun {
    static RUN: OnceLock<RealDataRun> = OnceLock::new();
    RUN.get_or_init(|| {
        run_over_fixture(
            "ntia_84_164_sa_8002a_10m_boresight.csv",
            "ntia_sa_8002a_10m_design_specs.yaml",
            "ntia_sa_8002a_10m",
            "c_band",
        )
    })
}

// ============================================================================
// Serving the artifact through the real service path
// ============================================================================

/// A `GainRequest` aimed **exactly at boresight**: the emitter sits at the reflector's own aim
/// point, so the query's polar angle is 0 and its azimuth is `atan2` on float noise.
///
/// That is the geometry a boresight-only artifact is calibrated for, and the geometry the
/// `azimuth_range = (0, 0)` coverage encoding used to reject outright (roadmap D13's second
/// blocker) — which is what makes `correction_applied` below worth asserting.
fn boresight_request(run: &RealDataRun, frequency_mhz: f64) -> GainRequest {
    let target = Position3D::geodetic(-117.0, 35.0, 400_000.0);
    GainRequest {
        antenna_id: run.antenna_id.clone(),
        feed_id: run.feed_id.clone(),
        vehicle_position: Position3D::geodetic(-118.0, 34.0, 100.0),
        reflector_boresight: target.clone(),
        // The feed is aimed at the SAME point the reflector is, so the derived physical feed
        // displacement is zero and the served geometry is the on-focus one calibrate tuned
        // against (its objective builds the feed with `FeedParametersBuilder::at_focus`).
        // Aiming the feed elsewhere serves a coma-displaced antenna — 30-plus dB down here —
        // and every comparison in this file would be meaningless.
        feed_pointing_location: target.clone(),
        emitter_position: target,
        frequency_mhz,
        pointing_frequency_mhz: None,
        include_reference: false,
        vehicle_attitude: None,
    }
}

fn serve(run: &RealDataRun, frequency_mhz: f64) -> GainResponse {
    let mut repo = CalibrationRepository::new();
    repo.add_calibration(run.load());
    compute_gain_from_request(&boresight_request(run, frequency_mhz), &repo).unwrap_or_else(|e| {
        panic!(
            "serving {frequency_mhz} MHz at boresight must succeed: {e}\n{}",
            run.output()
        )
    })
}

fn correction_applied(response: &GainResponse) -> bool {
    response
        .calibration_status
        .as_ref()
        .map(|s| s.correction_applied)
        .unwrap_or(false)
}

// ============================================================================
// The fixtures say what the report says
// ============================================================================

fn assert_fixture_matches_published(csv: &str, published: &[(f64, f64)]) {
    let text = std::fs::read_to_string(fixture(csv)).expect("read fixture");
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .collect();

    assert_eq!(
        rows.first().map(|s| s.trim()),
        Some("frequency_mhz,g_over_t_db,temperature_k"),
        "{csv}: first non-comment line must be the column header"
    );
    assert_eq!(
        rows.len() - 1,
        published.len(),
        "{csv}: row count disagrees with the published table in this test"
    );

    for (row, &(frequency_mhz, gain_dbi)) in rows[1..].iter().zip(published) {
        let fields: Vec<f64> = row
            .split(',')
            .map(|f| f.trim().parse().expect("numeric CSV field"))
            .collect();
        assert_eq!(fields.len(), 3, "{csv}: expected 3 columns in {row:?}");
        assert_eq!(fields[0], frequency_mhz, "{csv}: frequency in {row:?}");
        assert_eq!(
            fields[2], T_SYS_K,
            "{csv}: the assumed system temperature must be constant across the sweep; \
             a varying T_sys would shape the frequency trend the tuner is fitting"
        );
        let expected = published_gain_to_g_over_t(gain_dbi);
        assert!(
            (fields[1] - expected).abs() < 1e-9,
            "{csv}: {frequency_mhz} MHz carries G/T {} dB/K, but the published gain \
             {gain_dbi} dBi at T_sys = {T_SYS_K} K is {expected} dB/K",
            fields[1]
        );
    }
}

#[test]
fn andrew_fixture_matches_the_published_table() {
    assert_fixture_matches_published(
        "ntia_84_164_andrew_43998_10m_boresight.csv",
        &ANDREW_PUBLISHED,
    );
}

#[test]
fn sa_8002a_fixture_matches_the_published_table() {
    assert_fixture_matches_published("ntia_84_164_sa_8002a_10m_boresight.csv", &SA_PUBLISHED);
}

// ============================================================================
// Andrew 43998 — the uncorrected branch
// ============================================================================

#[test]
fn andrew_43998_real_data_produces_a_service_loadable_artifact() {
    let run = andrew();
    let calibration = run.load();

    assert_eq!(calibration.antenna_id, run.antenna_id);
    assert_eq!(calibration.feed_id, run.feed_id);
    assert_eq!(
        calibration.metadata.format_version, CALIBRATION_SCHEMA_VERSION,
        "a real-data boresight artifact must carry this build's schema version"
    );
    assert!(
        matches!(
            calibration.calibration_status,
            Some(CalibrationStatus::PartiallyCalibrated { .. })
        ),
        "boresight calibration covers boresight only; got {:?}",
        calibration.calibration_status
    );

    // Coverage spans the measured sweep, so the published frequencies are interpolated
    // rather than extrapolated.
    let coverage = calibration
        .calibration_coverage
        .as_ref()
        .expect("a boresight artifact must record its coverage");
    assert_eq!(coverage.frequency_range, (3700.0, 6425.0));
    assert!(coverage.is_boresight_only());
}

/// The headline real-data claim: tuned physics reproduces six published gains.
///
/// `physics_only_rmse_db` is the fit at the *assumed* design-spec parameters and
/// `rmse_db` the fit after tuning; both are measured against genuinely published numbers,
/// which is the thing no synthetic fixture in this repo can assert.
#[test]
fn andrew_43998_tuning_fits_the_published_gains() {
    let calibration = andrew().load();
    let meta = &calibration.metadata;

    let tuned_rmse = meta.rmse_db;
    let untuned_rmse = meta
        .physics_only_rmse_db
        .expect("boresight metadata records the pre-tuning RMSE");

    // Measured: 0.4040 dB untuned -> 0.0828 dB tuned. The bounds are loose enough to survive
    // an integrator refinement and tight enough that a regression to "fits nothing" fails.
    assert!(
        untuned_rmse < 1.0,
        "the assumed design specs should already be within 1 dB of the published gains \
         (measured 0.404 dB); got {untuned_rmse:.4} dB — check the fixture's assumed f/D"
    );
    assert!(
        tuned_rmse < 0.20,
        "tuned physics must reproduce the six published Andrew 43998 gains to well under \
         the project's 1 dB main-lobe requirement (measured 0.0828 dB); got {tuned_rmse:.4} dB"
    );
    assert!(
        tuned_rmse < untuned_rmse,
        "tuning must improve the fit: {tuned_rmse:.4} dB is no better than the untuned \
         {untuned_rmse:.4} dB"
    );
}

/// Real data, real residuals: this sweep sits on the **no-correction** side of the 0.5 dB
/// threshold, so the served value is tuned physics alone.
///
/// If this ever flips, the tests below stop covering the uncorrected branch — and
/// `sa_8002a_...` stops being the only real-data coverage of the corrected one.
#[test]
fn andrew_43998_stays_below_the_correction_fit_threshold() {
    let calibration = andrew().load();
    assert!(
        calibration.correction_surface.is_none(),
        "the Andrew fixture is the real-data cover for the *uncorrected* boresight branch; \
         its residuals now exceed the 0.5 dB correction-fit threshold"
    );
    assert!(
        !correction_applied(&serve(andrew(), 3950.0)),
        "an artifact with no correction surface cannot report correction_applied"
    );
}

#[test]
fn andrew_43998_served_gain_lands_within_tolerance_of_the_published_gains() {
    let run = andrew();
    let mut worst = (0.0_f64, 0.0_f64);

    for &(frequency_mhz, published_dbi) in &ANDREW_PUBLISHED {
        let response = serve(run, frequency_mhz);
        let delta = response.gain_db - published_dbi;

        assert!(
            response
                .warnings
                .iter()
                .any(|w| w.is(WarningCode::PartiallyCalibrated)),
            "a boresight-only artifact must warn that it is partially calibrated; got {:?}",
            response.warnings
        );

        assert!(
            delta.abs() < ANDREW_SERVED_TOLERANCE_DB,
            "served gain at {frequency_mhz} MHz is {:.4} dBi against a published \
             {published_dbi} dBi ({delta:+.4} dB), outside the \
             {ANDREW_SERVED_TOLERANCE_DB} dB tolerance",
            response.gain_db
        );

        if delta.abs() > worst.1.abs() {
            worst = (frequency_mhz, delta);
        }
    }

    // The deviation is systematic, not scatter — see the next test for why. Asserting the
    // sign here is what makes a *sign flip* (a real physics change) visible rather than
    // silently absorbed by the tolerance.
    assert!(
        worst.1 < 0.0,
        "the served-vs-published deviation is expected to be negative at every frequency \
         (the spillover term the tuner never saw); worst was {:+.4} dB at {} MHz",
        worst.1,
        worst.0
    );
}

/// Isolates the calibrate/service model mismatch described in the module header.
///
/// `spillover_loss_db` is the loss the service folded into `gain_db` and the calibrator's
/// objective never applied. Remove it and the served values reproduce the published gains at
/// the tuner's own reported RMSE — which simultaneously proves the physics genuinely fits real
/// measurements to 0.16 dB per point, and that the entire served-vs-published gap above is
/// that one term.
#[test]
fn andrew_43998_matches_the_published_gains_exactly_once_the_spillover_term_is_removed() {
    let run = andrew();
    let calibration = run.load();
    let mut sum_sq = 0.0;

    for &(frequency_mhz, published_dbi) in &ANDREW_PUBLISHED {
        let response = serve(run, frequency_mhz);
        let spillover_db = response.metadata.spillover_loss_db.unwrap_or_else(|| {
            panic!(
                "an artifact with no correction surface is served WITH spillover, so the \
                 service must report the term it folded in (frequency {frequency_mhz} MHz)"
            )
        });
        assert!(
            spillover_db < 0.0,
            "spillover is a loss and must be reported negative; got {spillover_db}"
        );

        let spillover_free = response.gain_db - spillover_db;
        let delta = spillover_free - published_dbi;
        assert!(
            delta.abs() < ANDREW_SPILLOVER_FREE_TOLERANCE_DB,
            "with the {spillover_db:.4} dB spillover term removed, {frequency_mhz} MHz \
             serves {spillover_free:.4} dBi against a published {published_dbi} dBi \
             ({delta:+.4} dB) — outside {ANDREW_SPILLOVER_FREE_TOLERANCE_DB} dB"
        );
        sum_sq += delta * delta;
    }

    // The spillover-free residuals ARE what the tuner minimized, so their RMSE must be the
    // RMSE the artifact reports. Any drift means calibrate and the service have started
    // disagreeing about something *else* as well, which is the failure this pins.
    let rmse = (sum_sq / ANDREW_PUBLISHED.len() as f64).sqrt();
    assert!(
        (rmse - calibration.metadata.rmse_db).abs() < 0.01,
        "spillover-free served residuals give {rmse:.4} dB RMSE, but the artifact reports \
         {:.4} dB — calibrate and the service now differ by more than the spillover term",
        calibration.metadata.rmse_db
    );
}

// ============================================================================
// SA 8002A — the corrected branch, on real data
// ============================================================================

#[test]
fn sa_8002a_real_data_fits_and_carries_a_frequency_correction() {
    let calibration = sa_8002a().load();

    let correction = calibration.correction_surface.as_ref().unwrap_or_else(|| {
        panic!(
            "the SA 8002A fixture is the real-data cover for the *corrected* boresight \
             branch: its published Rx- and Tx-band gains are mutually inconsistent, the \
             residual must clear the 0.5 dB threshold, and a correction must be fitted\n{}",
            sa_8002a().output()
        )
    });

    assert_eq!(
        correction.shape[2],
        SA_PUBLISHED.len(),
        "the frequency axis carries one control point per measured row"
    );
    assert!(
        matches!(
            calibration.calibration_status,
            Some(CalibrationStatus::PartiallyCalibrated { .. })
        ),
        "a frequency correction does not upgrade boresight coverage; got {:?}",
        calibration.calibration_status
    );
}

/// The served-path proof the roadmap asks for, now on real data: the correction must be
/// *reached*, not merely carried.
///
/// Until 2026-07-31 a boresight artifact recorded `azimuth_range = (0, 0)`, and
/// `is_in_coverage` rejected the very boresight query that coverage described — the artifact
/// loaded, reported `PartiallyCalibrated`, carried its correction, and served raw physics. The
/// tolerance alone would not catch that regression: raw physics here is ~1.6 dB off, well
/// inside a lenient bound. `correction_applied` is the assertion that catches it.
#[test]
fn sa_8002a_served_gain_applies_the_correction_and_matches_the_published_gains() {
    let run = sa_8002a();

    for &(frequency_mhz, published_dbi) in &SA_PUBLISHED {
        let response = serve(run, frequency_mhz);

        assert!(
            correction_applied(&response),
            "the boresight correction was silently skipped at {frequency_mhz} MHz — the \
             coverage gate is rejecting a boresight query again (roadmap D13)"
        );
        assert!(
            response
                .warnings
                .iter()
                .any(|w| w.is(WarningCode::PartiallyCalibrated)),
            "a boresight-only artifact must warn that it is partially calibrated; got {:?}",
            response.warnings
        );
        assert!(
            response.metadata.spillover_loss_db.is_none(),
            "an artifact carrying a correction surface is served with spillover OFF (the \
             correction absorbs it empirically), so no spillover term should be reported"
        );

        let delta = response.gain_db - published_dbi;
        assert!(
            delta.abs() < SA_SERVED_TOLERANCE_DB,
            "served gain at {frequency_mhz} MHz is {:.4} dBi against a published \
             {published_dbi} dBi ({delta:+.4} dB), outside the {SA_SERVED_TOLERANCE_DB} dB \
             tolerance — the fitted correction is no longer reproducing the measured \
             residuals at their own knots",
            response.gain_db
        );
    }
}

/// The correction is load-bearing: without it the same artifact's physics misses the
/// published gains by more than the tolerance the corrected path meets.
///
/// This is what stops the test above from passing for the wrong reason — a correction that
/// evaluated to ~0 dB everywhere (the D15 silent-zero failure mode) would still be "applied".
#[test]
fn sa_8002a_correction_is_material_not_decorative() {
    let calibration = sa_8002a().load();
    let correction = calibration
        .correction_surface
        .as_ref()
        .expect("the SA 8002A artifact must carry a correction surface");

    // The residual the tuner could not remove IS what the correction must absorb. Measured
    // 0.6214 dB — comfortably larger than the corrected-path tolerance, so the test above
    // could not pass with a correction of zero.
    assert!(
        calibration.metadata.rmse_db > SA_SERVED_TOLERANCE_DB,
        "post-tuning residual is only {:.4} dB, within the {SA_SERVED_TOLERANCE_DB} dB \
         corrected-path tolerance — that test would now pass with a zero correction and \
         proves nothing",
        calibration.metadata.rmse_db
    );

    // And the surface must actually *evaluate* to that. A fix that only lengthened the
    // collapsed azimuth/elevation/temperature knot vectors would satisfy the loader and still
    // drive the basis to zero here (the D15 silent-zero failure mode).
    let mut max_abs = 0.0_f64;
    for &(frequency_mhz, _) in &SA_PUBLISHED {
        let value = antenna_model::model::evaluate_correction(
            correction,
            0.0,
            0.0,
            frequency_mhz,
            calibration.validity_ranges.temperature_const,
        )
        .expect("evaluate the fitted correction")
        .correction_db;
        assert!(
            value.is_finite(),
            "correction at {frequency_mhz} MHz is not finite: {value}"
        );
        max_abs = max_abs.max(value.abs());
    }
    assert!(
        max_abs > SA_SERVED_TOLERANCE_DB,
        "the fitted correction evaluates to at most {max_abs:.4} dB across the measured \
         band — too small to be absorbing the {:.4} dB residual it was fitted to",
        calibration.metadata.rmse_db
    );
}
