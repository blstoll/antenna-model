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
/// `frequency_correction::should_fit_correction` would fit a correction surface. That is
/// deliberate and is asserted explicitly in `boresight_fixture_stays_below_the_correction_
/// fit_threshold`: a boresight artifact that *does* carry a frequency correction currently
/// fails to load, because `fit_frequency_correction` builds degenerate 4D axes that the
/// service-side `BSplineModel4D::validate` rejects. That defect is filed and pinned under
/// roadmap unit D13, which owns the fix; this unit is about container framing, so the
/// fixture stays on the near side of the threshold rather than papering over it.
const BORESIGHT_CSV: &str = "\
frequency_mhz,g_over_t_db,temperature_k
7100,21.5,290
7450,21.9,290
7800,22.2,290
8150,22.5,290
8500,22.7,290
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
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("boresight.csv");
    let artifact = dir.path().join("antenna.bin");

    std::fs::write(&input, BORESIGHT_CSV).expect("write fixture CSV");

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

    // See BORESIGHT_CSV's comment: a boresight artifact carrying a frequency correction
    // is currently rejected at load by `BSplineModel4D::validate` (degenerate axes from
    // `fit_frequency_correction`) — roadmap D13 owns that fix. If this assertion ever
    // fails, the fixture has drifted across the 0.5 dB threshold and the two tests above
    // are no longer testing what they claim; do not "fix" it by loosening the loader.
    assert!(
        calibration.correction_surface.is_none(),
        "fixture drifted: the tuner's residual now exceeds the 0.5 dB correction-fit \
         threshold, so this artifact exercises the D13 defect rather than D2's framing"
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
