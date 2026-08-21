//! Integration test: full-mode export produces a service-loadable artifact.
//!
//! This exercises the on-disk round-trip that matters for the service:
//! build a 3D correction surface, convert + assemble an `AntennaCalibration`,
//! write it with the ANTC header used by full mode, then load it back through
//! the service loader (`antenna_model::data::loader::load_calibration_artifact`).

use antenna_model::data::loader::load_calibration_artifact;
use antenna_model::data::types::CALIBRATION_SCHEMA_VERSION;
use calibrate::artifact_export::{export_full_calibration, ExportPhysicalParams};
use calibrate::correction_surface::{
    assess_angular_resolution, fit_correction_surface, CorrectionSurfaceParams,
};
use calibrate::parser::MeasurementPoint;

/// Smooth synthetic residual over (clock, cone, freq).
fn residual(clock_deg: f64, cone_deg: f64, freq_mhz: f64) -> f64 {
    0.1 * (clock_deg * std::f64::consts::PI / 180.0).sin()
        + 0.02 * cone_deg
        + 0.005 * (freq_mhz - 8000.0)
}

/// 8 x 6 x 6 = 288 points against the 5x6x6 = 180 coefficients that 1/2/2 knots at order 4
/// declare. Sized to the coefficient count, per roadmap D20 — the previous 6 x 5 x 5 = 150
/// grid was underdetermined and the fitter now rejects it rather than fitting it quietly.
/// The axis *ranges* are unchanged, so the domain every probe below samples is the same.
fn build_measurements() -> Vec<MeasurementPoint> {
    let clocks = [0.0, 50.0, 100.0, 150.0, 200.0, 250.0, 300.0, 350.0];
    let cones = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
    let freqs = [8000.0, 8080.0, 8160.0, 8240.0, 8320.0, 8400.0];
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
/// Write an artifact through the production writer.
///
/// This used to hand-roll the ANTC header here with a literal `b"ANTC"` and a literal
/// `2u32`, making it a fourth producer of the container format — the exact drift roadmap
/// **D2** collapsed into one writer. It went stale the moment D23 bumped the container
/// version to 3, failing with "unsupported ANTC artifact version 2" from a test whose
/// subject is the correction surface. Calling the real writer means the framing and both
/// version stamps can never be this test's problem again.
fn write_antc(
    calibration: &antenna_model::data::types::AntennaCalibration,
    path: &std::path::Path,
) {
    calibrate::artifact_export::write_calibration_artifact(calibration, path).expect("write");
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
        asymmetry_factor: 1.0,
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

    // Load via the service loader (exercises ANTC + CRC + postcard + validate).
    let loaded = load_calibration_artifact(tmp.path()).expect("service load");

    assert_eq!(loaded.antenna_id, "integ_antenna");
    assert_eq!(loaded.feed_id, "x_band");
    assert_eq!(loaded.metadata.format_version, CALIBRATION_SCHEMA_VERSION);

    let correction = loaded
        .correction_surface
        .as_ref()
        .expect("correction surface present");

    // Shape: spatial axes copy directly (no top-padding; service evaluator fixed),
    // temperature = order + 1.
    let [n_freq, n_cone, n_clock] = surface.shape;
    assert_eq!(correction.shape[0], n_clock, "azimuth control points");
    assert_eq!(correction.shape[1], n_cone, "elevation control points");
    assert_eq!(correction.shape[2], n_freq, "frequency control points");
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
        asymmetry_factor: 1.0,
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

/// D21: the angular-resolution assessment must survive producer → ANTC → service loader
/// with its measured values intact.
///
/// The negative control is the second half: every field is required to match the value the
/// producer measured from the fitted surface, **and** the assessment is required to be one
/// this antenna actually fails. Without that second requirement the test would pass just as
/// happily against a build that wrote a placeholder — an artifact claiming perfect resolution
/// for every antenna is exactly the silence D21 exists to end, and it would look identical
/// through an equality check alone.
#[test]
fn the_angular_resolution_assessment_round_trips_through_the_artifact() {
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
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: 8.0,
        phase_center_offset_m: 0.0,
        asymmetry_factor: 1.0,
        mesh: None,
    };

    let measured =
        assess_angular_resolution(&surface, physical.diameter_m).expect("angular resolution");

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

    let served = loaded
        .metadata
        .angular_resolution
        .expect("a full-mode artifact must carry its angular resolution");
    assert_eq!(
        served, measured,
        "the assessment must survive the round trip"
    );

    // The negative control: this really is an under-resolved geometry, so the round-tripped
    // value cannot be a well-resolved placeholder.
    assert!(
        !served.resolves_lobe_structure(),
        "a 3.7 m dish at X-band against these knots must not resolve its lobe structure: {}",
        served.summary()
    );
    assert!(
        served.cone_knots_per_lobe_period() < 1.0,
        "expected well under one knot per lobe period, got {:.4} — if this geometry has \
         become resolvable the control above is no longer doing anything",
        served.cone_knots_per_lobe_period()
    );
}

/// The artifact's `angular_resolution` and its `diameter_m` describe the **same** dish.
///
/// Roadmap **D26** finding 2. `export_full_calibration` used to take the assessment as a
/// parameter while the caller derived it from a second, independent read of the diameter
/// (`class.geometry.diameter_m`), so the two fields could describe different antennas — the
/// invariant C13 and D23 established two lines away in the same function. Nothing could
/// observe a divergence, because every call site happened to pass the matching value.
///
/// This test re-derives the assessment **from the artifact's own stamped diameter** and
/// requires it to reproduce the artifact's own stamped assessment. The negative control is
/// what gives it power: a different diameter must produce a different answer, so the equality
/// above is a real constraint rather than two ways of writing a constant.
#[test]
fn the_stamped_diameter_is_the_one_the_assessment_was_made_against() {
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

    const DIAMETER_M: f64 = 3.7;
    const OTHER_DIAMETER_M: f64 = 12.0;

    let physical = ExportPhysicalParams {
        diameter_m: DIAMETER_M,
        focal_length_m: 1.85,
        f_over_d_ratio: 0.5,
        surface_rms_mm: 1.2,
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: 8.0,
        phase_center_offset_m: 0.0,
        asymmetry_factor: 1.0,
        mesh: None,
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

    let stamped_diameter = loaded.physical_config.reflector.diameter_m;
    assert_eq!(stamped_diameter, DIAMETER_M);

    let stamped_resolution = loaded
        .metadata
        .angular_resolution
        .expect("a full-mode artifact must carry its angular resolution");

    // Re-derive from the artifact alone. Nothing outside the file is consulted.
    let from_the_artifact =
        assess_angular_resolution(&surface, stamped_diameter).expect("re-assessment");
    assert_eq!(
        stamped_resolution, from_the_artifact,
        "the artifact's angular resolution must be the one its own diameter implies"
    );

    // Negative control: a different dish gives a different answer, so the equality above is
    // a constraint on the diameter and not an identity that holds for anything.
    let from_another_dish =
        assess_angular_resolution(&surface, OTHER_DIAMETER_M).expect("control assessment");
    assert_ne!(
        stamped_resolution, from_another_dish,
        "a {OTHER_DIAMETER_M} m dish must not produce the same assessment as a \
         {DIAMETER_M} m one, or this test cannot detect a divergent diameter"
    );
}

// ---------------------------------------------------------------------------
// Roadmap D3 — the edge coverage the round trip did not have.
//
// The interior-point round trip above (`test_full_export_correction_evaluates_against_3d`)
// is the served leg of D3's exit criterion; what follows are the two edge cases the unit
// names, plus the stack guard that lets its `RUST_MIN_STACK` workaround be retired.
// ---------------------------------------------------------------------------

/// The knot configuration shared by the tests below — the same one the interior-point
/// round trip uses, so an edge failure cannot be blamed on a different surface shape.
fn round_trip_params() -> CorrectionSurfaceParams {
    CorrectionSurfaceParams {
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
    }
}

fn round_trip_physical() -> ExportPhysicalParams {
    ExportPhysicalParams {
        diameter_m: 3.7,
        focal_length_m: 1.85,
        f_over_d_ratio: 0.5,
        surface_rms_mm: 1.2,
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: 8.0,
        phase_center_offset_m: 0.0,
        asymmetry_factor: 1.0,
        mesh: None,
    }
}

/// Export, write through the production writer, and load back through the service loader.
///
/// Returns the loaded 4D correction surface — i.e. the object the *service* would hold,
/// not the one the producer built, so every assertion made against it has crossed postcard,
/// the ANTC container and `AntennaCalibration::validate`.
fn export_write_load(
    surface: &calibrate::correction_surface::CorrectionSurface,
    measurements: &[MeasurementPoint],
) -> antenna_model::data::types::BSplineModel4D {
    let calibration = export_full_calibration(
        "integ_antenna",
        "x_band",
        "Integ 3.7m",
        "file://integ.csv".to_string(),
        &round_trip_physical(),
        surface,
        measurements,
        0.4,
        0.99,
        0.9,
        true,
    )
    .expect("export");

    let tmp = tempfile::NamedTempFile::new().expect("tmp");
    write_antc(&calibration, tmp.path());
    load_calibration_artifact(tmp.path())
        .expect("service load")
        .correction_surface
        .expect("correction surface present")
}

/// Every axis **boundary** must survive the round trip, not just the interior.
///
/// `test_full_export_correction_evaluates_against_3d` samples interior points only, by
/// design — its probe grid stops at 10°/349° in clock, 0.5°/9.5° in cone and
/// 8050/8350 MHz. The in-crate unit test
/// (`calibrate::artifact_export::tests::test_round_trip_matches_3d_evaluation`) does sample
/// the exact domain bounds, but it compares two *in-process* objects and never crosses the
/// writer or the loader. So the combination that matters here — a domain edge, evaluated by
/// the service's own 4D interpolator, on an artifact that came off disk — was covered by
/// neither.
///
/// That combination is the one with history. **D15** was an upper-edge collapse in
/// `bspline_basis` at a domain maximum: the two implementations agree everywhere except at
/// an endpoint, and the fitted surface was corrupted across the whole top knot span while
/// every interior probe stayed clean. The 3D and 4D evaluators are *different code*
/// (`correction_surface::bspline_basis` versus `correction_interpolator`'s Cox-de Boor
/// loop), so their endpoint conventions are exactly the kind of thing that can diverge
/// without any interior sample noticing.
///
/// The `extrapolated` assertion is the second half: a point sitting **on** a domain bound is
/// in the domain. If an endpoint ever starts reporting extrapolation, the served response
/// gains a spurious `Extrapolated` warning for a query the artifact genuinely covers, which
/// no agreement check alone would catch.
///
/// The probe grid is **derived from the served knot vectors** rather than hand-listed, so it
/// lands exactly on every knot — the clamped ends *and* the interior ones — plus a midpoint
/// in each span. Interior knots earn their place independently of the endpoints: the two
/// implementations guard a vanishing Cox-de Boor denominator at **different thresholds**
/// (`1e-10` in `correction_surface::bspline_basis`, `1e-14` in
/// `correction_interpolator::evaluate_basis_functions`), and a knot is where that denominator
/// gets small. Deriving the grid also means it follows the fixture: change the knot counts and
/// the probes move with them instead of quietly going stale.
#[test]
fn the_round_trip_agrees_at_every_axis_boundary_after_a_service_load() {
    let measurements = build_measurements();
    let predictions = vec![0.0; measurements.len()];
    let surface = fit_correction_surface(&measurements, &predictions, &round_trip_params())
        .expect("surface fit");
    let model = export_write_load(&surface, &measurements);

    // Every distinct knot on an axis, plus the midpoint of each span between them. The
    // clamped end knots are the domain bounds, so this covers both edges of all four axes.
    fn probes(knots: &[f64]) -> Vec<f64> {
        let mut distinct: Vec<f64> = Vec::new();
        for &k in knots {
            if !distinct.iter().any(|&d: &f64| (d - k).abs() < 1e-12) {
                distinct.push(k);
            }
        }
        let mut out = distinct.clone();
        for w in distinct.windows(2) {
            out.push(0.5 * (w[0] + w[1]));
        }
        out
    }

    let clocks = probes(&model.knots_azimuth);
    let cones = probes(&model.knots_elevation);
    let freqs = probes(&model.knots_frequency);
    let temps = probes(&model.knots_temperature);

    // The fixture's knots: 4 distinct on each spatial angle (2 clamped ends + 2 interior),
    // 3 on frequency, 3 on the flat temperature axis. Asserted so that a fixture change
    // which silently collapses an axis cannot shrink this test's reach unnoticed.
    assert_eq!((clocks.len(), cones.len()), (7, 7), "clock/cone probes");
    assert_eq!(
        (freqs.len(), temps.len()),
        (5, 5),
        "frequency/temperature probes"
    );

    let mut max_err = 0.0_f64;
    let mut max_expected = 0.0_f64;
    let mut samples = 0;
    for &k in &clocks {
        for &c in &cones {
            for &f in &freqs {
                let expected = surface.evaluate(f, c, k).expect("3D eval");
                // Explicit rather than implied. `err < 1e-9` below does reject a NaN (every
                // comparison against NaN is false), but the aggregate `max_err.max(err)`
                // would discard one, so the finiteness requirement is stated where it is
                // meant rather than left to that subtlety.
                assert!(
                    expected.is_finite(),
                    "3D evaluation must be finite at clock={k}, cone={c}, freq={f}: {expected}"
                );
                max_expected = max_expected.max(expected.abs());
                for &t in &temps {
                    let got = antenna_model::model::evaluate_correction(&model, k, c, f, t)
                        .expect("4D eval");
                    let err = (got.correction_db - expected).abs();
                    max_err = max_err.max(err);
                    samples += 1;
                    assert!(
                        err < 1e-9,
                        "boundary mismatch at clock={k}, cone={c}, freq={f}, temp={t}: \
                         3D expected={expected}, served 4D got={}, err={err:e}",
                        got.correction_db
                    );
                    assert!(
                        !got.extrapolated,
                        "a point on the fitted domain bound must not report extrapolation: \
                         clock={k}, cone={c}, freq={f}, temp={t}"
                    );
                }
            }
        }
    }

    // Vacuity guard: an agreement check between two zeros passes just as happily as one
    // between two correct values. D15's collapse drove the top knot span toward zero, which
    // is exactly the shape of failure this would otherwise wave through.
    assert!(
        max_expected > 0.1,
        "the boundary probes must carry a real correction to compare, got max |correction| \
         = {max_expected:e} dB"
    );

    assert_eq!(
        samples,
        clocks.len() * cones.len() * freqs.len() * temps.len(),
        "the probe grid must visit every knot/midpoint combination"
    );
    eprintln!("D3 boundary round-trip max error over {samples} samples: {max_err:e}");
}

/// The **narrowest frequency axis full mode can express** must round-trip too.
///
/// D3 asks for a single-frequency edge test. **No producer in this tree can emit one**, and
/// both refusals are asserted below rather than asserted about:
///
/// - **Full mode** — a dataset whose rows all share a frequency has zero range on that axis,
///   and `generate_knot_vector` refuses it (`max_val - min_val < min_spacing`) rather than
///   building a degenerate knot vector.
/// - **Boresight mode** — `fit_frequency_correction` needs **≥ 4** frequencies for its cubic
///   B-spline and returns `InsufficientData` below that, so a single-frequency boresight run
///   writes an artifact with no correction surface at all.
///
/// An earlier version of this comment claimed boresight mode *is* the single-frequency
/// artifact, "collapsing the frequency axis with `flat_axis`". That was wrong, and worth
/// recording because it is an easy misreading: boresight collapses **azimuth, elevation and
/// temperature** with `flat_axis` and *fits* frequency like any other axis.
///
/// What both refusals are protecting is not obvious, so it belongs here. A degenerate axis —
/// `order` equal knots — would pass `BSplineModel4D::validate`, which checks knot-vector
/// *length* and not span width, and would then evaluate to a **zero correction at every
/// frequency**: the evaluable span `[knots[order-1], knots[len-order]]` is empty, so every
/// Cox-de Boor denominator vanishes and every basis value with it. That is the D13/D26
/// signature — an artifact that loads clean, reports healthy, and silently applies nothing.
/// It is exactly why `flat_axis` exists (see its doc comment) and why the fitter's range
/// check exists, and it is why these two refusals are the coverage this edge case admits.
///
/// What full mode *can* express is the minimum coefficient count: zero interior knots, so the
/// frequency axis carries exactly `spline_order` = 4 coefficients rather than 5. That is the
/// case where the clamped end-knot multiplicities meet in the middle with no interior knot
/// between them, and it exercises the reindex in `to_bspline_4d` at a different stride than
/// every other test here (4·6·6 = 144 coefficients, not 180).
#[test]
fn a_minimal_frequency_axis_round_trips_and_a_degenerate_one_is_refused() {
    let measurements = build_measurements();
    let predictions = vec![0.0; measurements.len()];

    let params = CorrectionSurfaceParams {
        num_knots_frequency: 0,
        ..round_trip_params()
    };
    let surface =
        fit_correction_surface(&measurements, &predictions, &params).expect("minimal-axis fit");

    assert_eq!(
        surface.shape[0], 4,
        "zero interior knots at order 4 must leave exactly `order` frequency coefficients"
    );

    let model = export_write_load(&surface, &measurements);
    assert_eq!(
        model.shape[2], 4,
        "the served frequency axis must carry the same 4"
    );

    // Asserted per sample, not only through the `max_err` aggregate. `f64::max` *returns the
    // other operand* when one side is NaN, so a non-finite correction would be silently
    // dropped from the aggregate and the final `max_err < 1e-9` would still pass — the exact
    // discard D26 found in `widest_knot_gap`'s `fold(0.0, f64::max)`, where a NaN axis gap
    // was thrown away and the *better*-resolved verdict reported out of corrupt input.
    // The probes sit on the axis bounds as well as inside, so `!extrapolated` is checked here
    // for the same reason as in the boundary test above.
    let mut max_err = 0.0_f64;
    let mut max_expected = 0.0_f64;
    for &k in &[0.0, 175.0, 350.0] {
        for &c in &[0.0, 5.0, 10.0] {
            for &f in &[8000.0, 8200.0, 8400.0] {
                let expected = surface.evaluate(f, c, k).expect("3D eval");
                assert!(
                    expected.is_finite(),
                    "3D evaluation must be finite at clock={k}, cone={c}, freq={f}: {expected}"
                );
                max_expected = max_expected.max(expected.abs());

                let got = antenna_model::model::evaluate_correction(&model, k, c, f, 290.0)
                    .expect("4D eval");
                assert!(
                    got.correction_db.is_finite(),
                    "served correction must be finite at clock={k}, cone={c}, freq={f}: {}",
                    got.correction_db
                );
                assert!(
                    !got.extrapolated,
                    "a point inside the fitted domain must not report extrapolation: \
                     clock={k}, cone={c}, freq={f}"
                );

                let err = (got.correction_db - expected).abs();
                assert!(
                    err < 1e-9,
                    "minimal-axis mismatch at clock={k}, cone={c}, freq={f}: \
                     3D expected={expected}, served 4D got={}, err={err:e}",
                    got.correction_db
                );
                max_err = max_err.max(err);
            }
        }
    }
    // Vacuity guard, as above: two zeros agree perfectly.
    assert!(
        max_expected > 0.1,
        "the probes must carry a real correction to compare, got max |correction| \
         = {max_expected:e} dB"
    );
    assert!(
        max_err < 1e-9,
        "minimal-frequency-axis round-trip max error {max_err:e} exceeds 1e-9"
    );

    // The degenerate case: one frequency, everything else unchanged. This must be refused,
    // not fitted — a zero-width axis has no evaluable span, and a surface built on one would
    // load and then return nothing useful.
    //
    // The grid is deliberately *denser* than `build_measurements()` on its two surviving
    // axes: 16 x 12 = 192 points clears the `(spline_order + 1)³ = 125` minimum-points
    // pre-check, so the refusal under test is the zero-width axis itself rather than the
    // point count. Slicing one frequency out of the 288-row grid gives only 48 rows and
    // stops at "need at least 125" — a refusal that says nothing about degenerate axes.
    let single_freq: Vec<MeasurementPoint> = (0..16)
        .flat_map(|ik| {
            (0..12).map(move |ic| {
                let k = ik as f64 * 22.0;
                let c = ic as f64 * 0.9;
                MeasurementPoint::new(k, c, 8000.0, residual(k, c, 8000.0), 290.0)
            })
        })
        .collect();
    assert_eq!(
        single_freq.len(),
        192,
        "the single-frequency grid must clear the 125-point pre-check"
    );
    let single_predictions = vec![0.0; single_freq.len()];
    let err = fit_correction_surface(&single_freq, &single_predictions, &round_trip_params())
        .expect_err("a single-frequency dataset must not produce a full-mode surface");
    let msg = err.to_string();
    assert!(
        msg.contains("range"),
        "the refusal should name the zero-width axis, got: {msg}"
    );

    // The other producer, for the same reason. Boresight mode does not reach the code above
    // at all — it fits the frequency axis in `fit_frequency_correction` — so its refusal is a
    // separate mechanism and needs its own assertion, or "no producer can emit a
    // single-frequency correction surface" rests on half a check.
    let one_frequency =
        calibrate::frequency_correction::fit_frequency_correction(&[8000.0], &[0.8])
            .expect_err("a single frequency must not produce a boresight correction surface");
    assert!(
        matches!(
            one_frequency,
            calibrate::frequency_correction::FrequencyCorrectionError::InsufficientData(1)
        ),
        "expected InsufficientData(1), got: {one_frequency}"
    );

    // Positive control: the same call with the minimum it does accept succeeds, so the
    // refusal above is about the point count and not about this call being broken.
    let four_frequencies = calibrate::frequency_correction::fit_frequency_correction(
        &[7100.0, 7500.0, 8000.0, 8450.0],
        &[0.8, 0.6, 0.5, 0.7],
    )
    .expect("four frequencies is the documented minimum and must fit");
    four_frequencies
        .validate()
        .expect("and the result must be one the service loader accepts");
}

/// The whole round trip must run inside a **small** thread stack.
///
/// This is the guard that lets `RUST_MIN_STACK=16777216` come out of `scripts/check.sh` and
/// `.github/workflows/ci.yml`. That variable was added on 2026-07-09 (commit `4b439c0`)
/// because the calibrate lib suite aborted with a stack overflow on the Linux debug build
/// while passing on macOS, and it was attributed to "the 3D→4D round-trip B-spline
/// evaluation", with an iterative rewrite filed as roadmap D3.
///
/// **Measurement (2026-08-20, macOS aarch64, debug) contradicts that diagnosis rather than
/// confirming it.** The complete round trip — fit, convert, export, write, load, evaluate —
/// completes in a **24 KiB** thread stack, and the whole `calibrate` lib suite passes with
/// `RUST_MIN_STACK=65536`. There is also nothing to rewrite iteratively: the 4D evaluator
/// (`correction_interpolator::evaluate_basis_functions`) was already an iterative Cox-de Boor
/// loop at `4b439c0` itself, and the one genuine recursion,
/// `correction_surface::bspline_basis`, is depth-bounded by `spline_order` (4 here), so its
/// worst case is 2⁴ tiny frames.
///
/// So the value of this test is not the rewrite D3 imagined — it is turning a Linux-only,
/// CI-only symptom into a property that fails on every platform. `GUARD_STACK_BYTES` is
/// **21× the measured floor**, and it is set explicitly on this thread rather than inherited,
/// so it holds whatever the harness gives the other tests: `RUST_MIN_STACK` when that is set,
/// and libtest's own default when it is not. A regression that would newly need the
/// workaround trips here first, locally and on every platform, instead of surfacing as an
/// abort in Linux CI.
///
/// Both harnesses this repo uses run a test on a worker thread named after it — verified
/// 2026-08-20 for `cargo test` and for `cargo nextest`, which is why `RUST_MIN_STACK` still
/// reaches the tests after D18 moved the gate onto nextest.
///
/// **Failure mode:** a stack overflow aborts the process (SIGABRT); it is not a catchable
/// assertion, and for that reason this test has no negative control — you cannot ask a
/// process to survive its own abort. The thread is named so the runtime's
/// `thread '<name>' has overflowed its stack` line points at this test rather than at
/// whichever unrelated test happened to be running, which is precisely the misattribution
/// that sent D3 looking for a recursion that was never there.
#[test]
fn the_round_trip_fits_in_a_small_thread_stack() {
    /// 21× the 24 KiB floor measured for this path on 2026-08-20 (macOS aarch64, debug).
    const GUARD_STACK_BYTES: usize = 512 * 1024;

    let handle = std::thread::Builder::new()
        .name("d3-stack-guard".to_string())
        .stack_size(GUARD_STACK_BYTES)
        .spawn(|| {
            let measurements = build_measurements();
            let predictions = vec![0.0; measurements.len()];
            let surface = fit_correction_surface(&measurements, &predictions, &round_trip_params())
                .expect("surface fit");
            let model = export_write_load(&surface, &measurements);

            let expected = surface.evaluate(8200.0, 5.0, 175.0).expect("3D eval");
            let got = antenna_model::model::evaluate_correction(&model, 175.0, 5.0, 8200.0, 290.0)
                .expect("4D eval")
                .correction_db;
            assert!(
                (got - expected).abs() < 1e-9,
                "round trip inside the guarded stack must still be correct: \
                 expected={expected}, got={got}"
            );
        })
        .expect("spawn guarded thread");

    handle
        .join()
        .expect("the round trip must complete in the guarded stack");
}
