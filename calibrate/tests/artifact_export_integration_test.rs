//! Integration test: full-mode export produces a service-loadable artifact.
//!
//! This exercises the on-disk round-trip that matters for the service:
//! build a 3D correction surface, convert + assemble an `AntennaCalibration`,
//! write it with the ANTC header used by full mode, then load it back through
//! the service loader (`antenna_model::data::loader::load_calibration_artifact`).

use antenna_model::data::loader::load_calibration_artifact;
use calibrate::artifact_export::{export_full_calibration, ExportPhysicalParams};
use calibrate::correction_surface::{fit_correction_surface, CorrectionSurfaceParams};
use calibrate::parser::MeasurementPoint;

/// Smooth synthetic residual over (clock, cone, freq).
fn residual(clock_deg: f64, cone_deg: f64, freq_mhz: f64) -> f64 {
    0.1 * (clock_deg * std::f64::consts::PI / 180.0).sin()
        + 0.02 * cone_deg
        + 0.005 * (freq_mhz - 8000.0)
}

fn build_measurements() -> Vec<MeasurementPoint> {
    let clocks = [0.0, 70.0, 140.0, 210.0, 280.0, 350.0];
    let cones = [0.0, 2.5, 5.0, 7.5, 10.0];
    let freqs = [8000.0, 8100.0, 8200.0, 8300.0, 8400.0];
    let mut v = Vec::new();
    for &k in &clocks {
        for &c in &cones {
            for &f in &freqs {
                v.push(MeasurementPoint::new(k, c, f, residual(k, c, f), 290.0));
            }
        }
    }
    v
}

/// Write an `AntennaCalibration` with the ANTC header (matching full mode).
fn write_antc(
    calibration: &antenna_model::data::types::AntennaCalibration,
    path: &std::path::Path,
) {
    let payload = bincode::encode_to_vec(calibration, bincode::config::standard()).expect("encode");
    let crc = crc32fast::hash(&payload);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ANTC");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&crc.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&payload);
    std::fs::write(path, &bytes).expect("write");
}

#[test]
fn test_full_export_loads_via_service() {
    let measurements = build_measurements();
    let predictions = vec![0.0; measurements.len()];

    let params = CorrectionSurfaceParams {
        spline_order: 4,
        num_knots_frequency: 1,
        num_knots_econe: 2,
        num_knots_eclock: 2,
        regularization: 1e-3,
        adaptive_knots: false,
        cross_validation_folds: 0,
        min_knot_spacing_frequency: 50.0,
        min_knot_spacing_econe: 1.0,
        min_knot_spacing_eclock: 5.0,
    };

    let surface =
        fit_correction_surface(&measurements, &predictions, &params).expect("surface fit");

    let physical = ExportPhysicalParams {
        diameter_m: 3.7,
        focal_length_m: 1.85,
        f_over_d_ratio: 0.5,
        surface_rms_mm: 1.2,
        feed_position_m: (0.0, 0.0, 1.85),
        q_factor: 8.0,
        phase_center_offset_m: 0.0,
        mesh: Some((5.0, 0.5)),
    };

    let calibration = export_full_calibration(
        "integ_antenna",
        "x_band",
        "Integ 3.7m",
        "file://integ.csv".to_string(),
        &physical,
        &surface,
        &measurements,
        0.4,
        0.99,
        0.9,
        true,
    )
    .expect("export");

    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    write_antc(&calibration, tmp.path());

    // Load via the service loader (exercises ANTC + CRC + bincode + validate).
    let loaded = load_calibration_artifact(tmp.path()).expect("service load");

    assert_eq!(loaded.antenna_id, "integ_antenna");
    assert_eq!(loaded.feed_id, "x_band");
    assert_eq!(loaded.metadata.format_version, "2.0");

    let correction = loaded
        .correction_surface
        .as_ref()
        .expect("correction surface present");

    // Shape: spatial axes top-padded by one, temperature = order + 1.
    let [n_freq, n_cone, n_clock] = surface.shape;
    assert_eq!(correction.shape[0], n_clock + 1, "azimuth control points");
    assert_eq!(correction.shape[1], n_cone + 1, "elevation control points");
    assert_eq!(correction.shape[2], n_freq + 1, "frequency control points");
    assert_eq!(
        correction.shape[3],
        surface.spline_order + 1,
        "temperature layers"
    );

    // Coverage round-trips with the measurement count.
    let coverage = loaded.calibration_coverage.expect("coverage present");
    assert_eq!(coverage.num_measurements, measurements.len());
    assert!(coverage.has_correction_surface);

    // Status is FullyCalibrated.
    assert!(matches!(
        loaded.calibration_status,
        Some(antenna_model::data::types::CalibrationStatus::FullyCalibrated { .. })
    ));
}

#[test]
fn test_full_export_correction_evaluates_against_3d() {
    // End-to-end: after a service load, the 4D correction reproduces the 3D
    // calibrate evaluation at interior points (round-trip through disk).
    let measurements = build_measurements();
    let predictions = vec![0.0; measurements.len()];
    let params = CorrectionSurfaceParams {
        spline_order: 4,
        num_knots_frequency: 1,
        num_knots_econe: 2,
        num_knots_eclock: 2,
        regularization: 1e-3,
        adaptive_knots: false,
        cross_validation_folds: 0,
        min_knot_spacing_frequency: 50.0,
        min_knot_spacing_econe: 1.0,
        min_knot_spacing_eclock: 5.0,
    };
    let surface =
        fit_correction_surface(&measurements, &predictions, &params).expect("surface fit");

    let physical = ExportPhysicalParams {
        diameter_m: 3.7,
        focal_length_m: 1.85,
        f_over_d_ratio: 0.5,
        surface_rms_mm: 1.2,
        feed_position_m: (0.0, 0.0, 1.85),
        q_factor: 8.0,
        phase_center_offset_m: 0.0,
        mesh: Some((5.0, 0.5)),
    };
    let calibration = export_full_calibration(
        "integ_antenna",
        "x_band",
        "Integ 3.7m",
        "file://integ.csv".to_string(),
        &physical,
        &surface,
        &measurements,
        0.4,
        0.99,
        0.9,
        true,
    )
    .expect("export");

    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    write_antc(&calibration, tmp.path());
    let loaded = load_calibration_artifact(tmp.path()).expect("service load");
    let model = loaded.correction_surface.expect("correction");

    // Temperature interval is [t_meas-1, t_meas+1] = [289, 291]; midpoint 290.
    let t_mid = 290.0;
    let mut max_err = 0.0_f64;
    for &k in &[10.0, 90.0, 180.0, 270.0, 349.0] {
        for &c in &[0.5, 5.0, 9.5] {
            for &f in &[8050.0, 8200.0, 8350.0] {
                let expected = surface.evaluate(f, c, k).expect("3D eval");
                let got = antenna_model::model::evaluate_correction(&model, k, c, f, t_mid)
                    .expect("4D eval")
                    .correction_db;
                max_err = max_err.max((got - expected).abs());
            }
        }
    }
    assert!(
        max_err < 1e-9,
        "post-load round-trip max error {max_err:e} exceeds 1e-9"
    );
}
