//! End-to-end test of the `calibrate` binary in **boresight** mode.
//!
//! Companion to `cli_full_mode_e2e.rs`. Together the two files satisfy roadmap unit D2's
//! "a round-trip test covers **both** producers": full mode goes through
//! `artifact_export::export_full_calibration`, boresight mode through
//! `boresight_calibration::build_calibration_artifact`, and both must now write the same
//! ANTC container framing and load through the *service's* loader.
//!
//! This file exists because they did not. Until 2026-07-30 the boresight path wrote a bare
//! `postcard::to_allocvec` — no magic, no container version, no CRC — which the service
//! accepted only via its legacy headerless fallback. Nothing was visibly broken, which is
//! precisely the hazard: the artifact carried no container stamp for
//! `ANTC_ARTIFACT_VERSION` to check, so a future framing change would have mis-decoded it
//! silently instead of being rejected, and no checksum, so corruption surfaced as wrong
//! numbers rather than a load failure. Both are now asserted below.

use antenna_model::data::loader::{ANTC_ARTIFACT_VERSION, ANTC_HEADER_LEN, ANTC_MAGIC};
use antenna_model::data::types::{CalibrationStatus, CALIBRATION_SCHEMA_VERSION};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Boresight measurements: `frequency_mhz,g_over_t_db,temperature_k` (the boresight parser
/// is separate from full mode's and takes exactly these three columns).
///
/// The values are a plausible X-band sweep for the 3.7 m design spec below, chosen so the
/// tuner converges to a **max |residual| under 0.5 dB** — the threshold at which
/// `frequency_correction::should_fit_correction` fits a correction surface. Keeping this
/// fixture on the near side of the threshold is what points the framing tests below at the
/// *uncorrected* boresight artifact specifically; `RIPPLED_BORESIGHT_CSV` covers the other
/// side of the branch.
const BORESIGHT_CSV: &str = "\
frequency_mhz,g_over_t_db,temperature_k
7100,21.5,290
7450,21.9,290
7800,22.2,290
8150,22.5,290
8500,22.7,290
";

/// The same sweep with a ±1 dB frequency ripple superimposed. Parameter tuning cannot absorb
/// it — surface RMS, q-factor and mesh geometry are all smooth in frequency — so the residual
/// clears `should_fit_correction`'s 0.5 dB threshold and the run attaches a frequency
/// correction surface.
///
/// That branch was **unloadable** until 2026-07-31 (roadmap D13): `fit_frequency_correction`
/// built its azimuth/elevation/temperature axes as `order` equal knots over a single
/// coefficient layer, which `BSplineModel4D::validate` rejects — so the service refused every
/// boresight artifact that carried a correction. D2 could only pin the framing on the
/// no-correction path for exactly that reason. This fixture exists to hold the other path
/// open.
const RIPPLED_BORESIGHT_CSV: &str = "\
frequency_mhz,g_over_t_db,temperature_k
7100,22.5,290
7450,20.9,290
7800,23.2,290
8150,21.5,290
8500,23.7,290
";

const ANTENNA_ID: &str = "gs_3.7m";
const FEED_ID: &str = "x_band_feed";

struct CalibrateRun {
    stdout: String,
    stderr: String,
    artifact: PathBuf,
    // Kept alive so the temp dir outlives the artifact path above.
    _dir: tempfile::TempDir,
}

impl CalibrateRun {
    fn output(&self) -> String {
        format!(
            "--- stdout ---\n{}\n--- stderr ---\n{}",
            self.stdout, self.stderr
        )
    }
}

/// The shipped design-spec example, reached from `CARGO_MANIFEST_DIR` so the test does not
/// depend on the process CWD. Using the shipped file rather than a private copy means this
/// test also exercises the example the docs tell users to run.
fn design_specs_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../calibration_data/design_specs/small_groundstation.yaml")
}

/// Run the real `calibrate` binary in boresight mode over `BORESIGHT_CSV`.
fn run_boresight() -> CalibrateRun {
    run_boresight_over(BORESIGHT_CSV)
}

/// Run the real `calibrate` binary in boresight mode over an arbitrary fixture CSV.
fn run_boresight_over(csv: &str) -> CalibrateRun {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("boresight.csv");
    let artifact = dir.path().join("antenna.bin");

    std::fs::write(&input, csv).expect("write fixture CSV");

    let specs = design_specs_path();
    assert!(
        specs.exists(),
        "design specs not found at {}",
        specs.display()
    );

    let out = Command::new(env!("CARGO_BIN_EXE_calibrate"))
        .args(["--calibration-mode", "boresight"])
        .arg("--input")
        .arg(&input)
        .arg("--output")
        .arg(&artifact)
        .args(["--antenna-id", ANTENNA_ID])
        .args(["--feed-id", FEED_ID])
        .arg("--design-specs")
        .arg(&specs)
        .output()
        .expect("run the calibrate binary");

    let run = CalibrateRun {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        artifact,
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

#[test]
fn cli_boresight_mode_writes_an_antc_framed_artifact() {
    let run = run_boresight();
    let bytes = std::fs::read(&run.artifact).expect("read artifact");

    assert!(
        bytes.len() > ANTC_HEADER_LEN,
        "artifact is shorter than an ANTC header ({} bytes)",
        bytes.len()
    );
    assert_eq!(
        &bytes[0..4],
        ANTC_MAGIC,
        "boresight artifact is missing the ANTC magic; first bytes: {:?}",
        &bytes[..bytes.len().min(8)]
    );

    // The container version stamp — the thing a headerless artifact could not carry, and
    // therefore the thing `ANTC_ARTIFACT_VERSION` could never check on one.
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    assert_eq!(
        version, ANTC_ARTIFACT_VERSION,
        "boresight artifact must stamp the container version this build writes"
    );

    // The declared payload length must describe the rest of the file exactly.
    let payload_len = u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
    assert_eq!(
        payload_len,
        bytes.len() - ANTC_HEADER_LEN,
        "declared payload length disagrees with the file size"
    );
}

#[test]
fn cli_boresight_mode_writes_a_service_loadable_artifact() {
    let run = run_boresight();

    // The point of this assertion: the artifact must load through the SERVICE's loader,
    // not just calibrate's own round-trip code.
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("the service loader must accept a freshly written boresight artifact");

    assert_eq!(calibration.antenna_id, ANTENNA_ID);
    assert_eq!(calibration.feed_id, FEED_ID);

    // The schema axis rides inside the payload and must be stamped by the boresight
    // builder just as it is by the full-mode one.
    assert_eq!(
        calibration.metadata.format_version, CALIBRATION_SCHEMA_VERSION,
        "boresight artifact must carry this build's schema version"
    );

    assert!(
        matches!(
            calibration.calibration_status,
            Some(CalibrationStatus::PartiallyCalibrated { .. })
        ),
        "boresight calibration covers boresight only, so the artifact must say \
         PartiallyCalibrated; got {:?}",
        calibration.calibration_status
    );
}

#[test]
fn boresight_fixture_stays_below_the_correction_fit_threshold() {
    let run = run_boresight();
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("load the boresight artifact");

    // See BORESIGHT_CSV's comment. The two tests above are about the framing of a
    // *no-correction* boresight artifact; if this fixture drifts across the 0.5 dB
    // threshold they quietly become duplicates of the rippled ones below.
    assert!(
        calibration.correction_surface.is_none(),
        "fixture drifted: the tuner's residual now exceeds the 0.5 dB correction-fit \
         threshold, so this artifact no longer covers the no-correction branch"
    );
}

/// The D13 regression test at CLI level: a boresight run whose residuals trip the fitting
/// threshold must produce an artifact the **service's own loader** accepts.
///
/// Before the fix this failed at `load_calibration_artifact` with an `InvalidKnotVector`
/// on the azimuth axis — the whole boresight-plus-correction path was dead on arrival.
#[test]
fn a_boresight_artifact_carrying_a_frequency_correction_loads() {
    let run = run_boresight_over(RIPPLED_BORESIGHT_CSV);

    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact).expect(
        "the service loader must accept a boresight artifact that carries a frequency \
             correction (roadmap D13 — degenerate correction axes)",
    );

    let correction = calibration.correction_surface.as_ref().unwrap_or_else(|| {
        panic!(
            "the rippled fixture must trip the 0.5 dB correction-fit threshold, otherwise \
             this test is vacuous\n{}",
            run.output()
        )
    });

    // One control point per measured frequency; the other three axes are flat.
    assert_eq!(
        correction.shape[2],
        RIPPLED_BORESIGHT_CSV.lines().count() - 1,
        "frequency axis should carry one control point per measurement row"
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

/// The correction the artifact carries must also *evaluate* to something real at boresight.
///
/// A "fix" that only lengthened the degenerate knot vectors would satisfy the loader and
/// still fail here: an axis whose evaluable span `[knots[order-1], knots[len-order]]` is
/// empty drives its basis functions to zero and collapses the correction to 0 dB, which
/// would look exactly like a correctly-loaded surface that happens to correct nothing.
#[test]
fn the_carried_frequency_correction_evaluates_to_a_real_value() {
    let run = run_boresight_over(RIPPLED_BORESIGHT_CSV);
    let calibration = antenna_model::data::loader::load_calibration_artifact(&run.artifact)
        .expect("load the rippled boresight artifact");
    let correction = calibration
        .correction_surface
        .as_ref()
        .expect("rippled fixture must carry a correction surface");

    // The ripple is ±1 dB about the smooth sweep, so the fitted correction must be
    // materially nonzero somewhere across the band. Sample every measured frequency.
    let frequencies = [7100.0, 7450.0, 7800.0, 8150.0, 8500.0];
    let mut max_abs = 0.0_f64;
    for freq in frequencies {
        let value = antenna_model::model::evaluate_correction(
            correction,
            0.0,
            0.0,
            freq,
            calibration.validity_ranges.temperature_const,
        )
        .expect("evaluate the boresight correction")
        .correction_db;
        assert!(
            value.is_finite(),
            "correction at {freq} MHz is not finite: {value}"
        );
        max_abs = max_abs.max(value.abs());
    }

    assert!(
        max_abs > 0.5,
        "the fitted correction is ~0 dB across the whole band (max |c| = {max_abs:.4} dB); \
         the collapsed axes are evaluating their basis to zero"
    );
}

#[test]
fn corrupting_a_boresight_artifact_is_detected() {
    let run = run_boresight();
    let mut bytes = std::fs::read(&run.artifact).expect("read artifact");

    // Flip one bit deep in the payload, leaving magic, version and declared length intact.
    // A headerless artifact had no way to notice this: postcard would decode the corrupted
    // bytes into a plausible-looking number. The CRC in the header is what turns it into a
    // load failure.
    let victim = ANTC_HEADER_LEN + (bytes.len() - ANTC_HEADER_LEN) / 2;
    bytes[victim] ^= 0b0000_0001;

    let corrupted = run.artifact.with_extension("corrupt.bin");
    std::fs::write(&corrupted, &bytes).expect("write corrupted artifact");

    let err = antenna_model::data::loader::load_calibration_artifact(&corrupted)
        .expect_err("a corrupted payload must not load");
    assert!(
        err.to_string().contains("CRC32 mismatch"),
        "expected a CRC failure, got: {err}"
    );
}
