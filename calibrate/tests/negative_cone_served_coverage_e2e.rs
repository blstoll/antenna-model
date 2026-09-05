//! A negative-E-cone measurement set must produce an artifact the service can actually use.
//!
//! **Roadmap D26 finding 1**, and the test the filing asked for: not "the range looks right"
//! but *serve* such an artifact and show the correction reaching the answer.
//!
//! E-clock/E-cone are spherical coordinates about boresight, and `MeasurementPoint::validate`
//! admits E-cone over `[-90, 90]` — a one-sided pattern cut recorded on a fixed clock plane
//! is legal, first-class input. The served side has no such freedom: the service's elevation
//! is a polar angle from boresight and is never negative.
//!
//! Before D26 the two sides were bridged by a silent clamp in `export_full_calibration`
//! (`el_lo = min.max(0.0)`, `el_hi = max.min(90.0)`). For a `-14°…0°` cut that produced the
//! elevation range `(0.0, 0.0)`, and every consequence was invisible:
//!
//! - `CalibrationCoverage::is_boresight_only()` became true over thousands of measurements;
//! - `contains()` admitted no elevation but exactly 0.0, so the service applied **no
//!   correction at all** and served raw physics while reporting the artifact healthy;
//! - a wholly-negative span such as `-14°…-1°` produced the *inverted* range `(0.0, -1.0)`,
//!   rejecting everything by construction.
//!
//! The fix is a convention, not a clamp: the parser reflects `(φ, −θ)` onto the identical
//! direction `(φ + 180°, θ)` on the way in, so predictions, residuals, knots, extents and
//! coverage all speak the polar convention the service does — and the export now *refuses*
//! an out-of-convention extent instead of quietly truncating it.
//!
//! The control in `correction_is_applied_for_a_negative_cone_measurement_set` is the
//! pre-D26 artifact itself: the same calibration with its coverage collapsed to `(0, 0)`.
//! Serving both isolates the correction term exactly, and pins that it is evaluated at the
//! served direction rather than clamped to a knot-vector edge.

use antenna_model::api::schemas::{GainRequest, GainResponse, Position3D};
use antenna_model::data::repository::CalibrationRepository;
use antenna_model::data::types::AntennaCalibration;
use antenna_model::model::geodetic_to_ecef;
use antenna_model::service::compute_gain_from_request;
use calibrate::artifact_export::{export_full_calibration, ExportPhysicalParams};
use calibrate::correction_surface::{
    fit_correction_surface, CorrectionSurface, CorrectionSurfaceParams,
};
use calibrate::parser::{parse_measurements_sync, MeasurementPoint};

const ANTENNA_ID: &str = "negative_cone_cut";
const FEED_ID: &str = "primary";

/// A deliberately cheap geometry: 1.0 m at ~2 GHz is `D/λ ≈ 7`, so the aperture integral is
/// inexpensive at every angle this test serves. The physics is not what is under test.
const DIAMETER_M: f64 = 1.0;
const F_OVER_D: f64 = 0.4;
const TEMPERATURE_K: f64 = 290.0;

/// Clock planes chosen to be closed under the +180° reflection, so the normalized data stays
/// on a rectangular grid and the fit is not asked to extrapolate into an empty half-plane.
const CLOCKS_DEG: [f64; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];
/// The cut runs *below* boresight, which is the case the clamp destroyed.
const CONES_DEG: [f64; 6] = [-10.0, -8.0, -6.0, -4.0, -2.0, 0.0];
const FREQS_MHZ: [f64; 6] = [2000.0, 2080.0, 2160.0, 2240.0, 2320.0, 2400.0];

/// The residual the fit has to recover. Smooth and clock-dependent, so a correction evaluated
/// at the wrong clock or clamped to a cone edge does not accidentally agree.
fn residual_db(clock_deg: f64, cone_deg: f64, freq_mhz: f64) -> f64 {
    0.30 * (clock_deg.to_radians()).sin() + 0.05 * cone_deg.abs() + 0.004 * (freq_mhz - 2000.0)
}

/// The measurement CSV, written with **signed** cone exactly as a one-sided cut is recorded.
fn negative_cone_csv() -> String {
    let mut csv = String::from("e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n");
    for &clock in &CLOCKS_DEG {
        for &cone in &CONES_DEG {
            for &freq in &FREQS_MHZ {
                // The reflection sends (clock, −|cone|) to (clock + 180°, |cone|), so the
                // residual is written against the direction the row actually names.
                let (norm_clock, norm_cone) = if cone < 0.0 {
                    ((clock + 180.0) % 360.0, -cone)
                } else {
                    (clock, cone)
                };
                let g_over_t = residual_db(norm_clock, norm_cone, freq);
                csv.push_str(&format!(
                    "{clock:.6},{cone:.6},{freq:.6},{g_over_t:.9},{TEMPERATURE_K:.6}\n"
                ));
            }
        }
    }
    csv
}

fn fitting_params() -> CorrectionSurfaceParams {
    // 1/2/2 interior knots at order 4 declare 5 × 6 × 6 = 180 coefficients against this
    // grid's 288 points, so the fit is determined (roadmap D20).
    CorrectionSurfaceParams {
        spline_order: 4,
        num_knots_frequency: 1,
        num_knots_econe: 2,
        num_knots_eclock: 2,
        regularization: 1e-4,
        adaptive_knots: false,
        cross_validation_folds: 0,
        min_knot_spacing_frequency: 50.0,
        min_knot_spacing_econe: 1.0,
        min_knot_spacing_eclock: 5.0,
    }
}

/// Parse the signed-cone CSV through the real parser (which is where the reflection happens),
/// fit, and export.
fn build_artifact() -> (AntennaCalibration, CorrectionSurface, Vec<MeasurementPoint>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let csv_path = dir.path().join("negative_cone_cut.csv");
    std::fs::write(&csv_path, negative_cone_csv()).expect("write csv");

    let data = parse_measurements_sync(csv_path.to_str().expect("utf-8 path")).expect("parse");

    // The parser is what puts the data in the polar convention.
    assert!(
        data.points.iter().all(|p| p.e_cone_deg >= 0.0),
        "the parser must reflect every signed-cone row before anything downstream sees it"
    );
    assert_eq!(
        data.e_cone_range(),
        (0.0, 10.0),
        "the reflected cut covers 0°…10° off boresight"
    );

    // Predictions are zero, so the fitted surface *is* the residual function. That keeps the
    // assertion below about the correction term and not about the physics model.
    let predictions = vec![0.0; data.points.len()];
    let surface =
        fit_correction_surface(&data.points, &predictions, &fitting_params()).expect("surface fit");

    let physical = ExportPhysicalParams {
        diameter_m: DIAMETER_M,
        focal_length_m: DIAMETER_M * F_OVER_D,
        f_over_d_ratio: F_OVER_D,
        surface_rms_mm: 0.3,
        // The feed's offset **from the focal point**; an on-axis feed is the origin (C13).
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: 2.0,
        phase_center_offset_m: 0.0,
        asymmetry_factor: 1.0,
        mesh: None,
    };

    let calibration = export_full_calibration(
        ANTENNA_ID,
        FEED_ID,
        "Negative-cone cut 1.0m",
        "file://negative_cone_cut.csv".to_string(),
        &physical,
        &surface,
        &data.points,
        0.05,
        0.999,
        0.5,
        false,
    )
    .expect("export must succeed for a legal negative-cone measurement set");

    calibration.validate().expect("artifact must validate");
    (calibration, surface, data.points)
}

fn repository_with(calibration: AntennaCalibration) -> CalibrationRepository {
    let mut repo = CalibrationRepository::new();
    repo.add_calibration(calibration);
    repo
}

/// Aim a request at polar angle `cone_deg`, clock `clock_deg` in the antenna frame.
///
/// With no attitude quaternion the azimuth-zero reference is the Earth-Z cross product
/// (`coordinates_3d::antenna_frame_axes`), replicated here — the only way to aim at a chosen
/// clock angle from outside the crate. `served_geometry_lands_on_the_requested_angles`
/// asserts the replication still agrees with the service.
fn request_for(cone_deg: f64, clock_deg: f64, frequency_mhz: f64) -> GainRequest {
    const VEHICLE: (f64, f64, f64) = (-118.0, 34.0, 100.0);
    const TARGET: (f64, f64, f64) = (-117.0, 35.0, 400_000.0);

    let vehicle = geodetic_to_ecef(VEHICLE.0, VEHICLE.1, VEHICLE.2).expect("vehicle ECEF");
    let target = geodetic_to_ecef(TARGET.0, TARGET.1, TARGET.2).expect("target ECEF");

    let bore = (
        target.0 - vehicle.0,
        target.1 - vehicle.1,
        target.2 - vehicle.2,
    );
    let range = (bore.0.powi(2) + bore.1.powi(2) + bore.2.powi(2)).sqrt();
    let z = (bore.0 / range, bore.1 / range, bore.2 / range);
    assert!(
        z.2.abs() < 0.99,
        "the test geometry must stay in the Earth-Z branch of the azimuth reference"
    );
    let x_raw = (-z.1, z.0, 0.0_f64);
    let x_mag = (x_raw.0.powi(2) + x_raw.1.powi(2)).sqrt();
    let x = (x_raw.0 / x_mag, x_raw.1 / x_mag, x_raw.2 / x_mag);
    let y = (
        z.1 * x.2 - z.2 * x.1,
        z.2 * x.0 - z.0 * x.2,
        z.0 * x.1 - z.1 * x.0,
    );

    let (theta, phi) = (cone_deg.to_radians(), clock_deg.to_radians());
    let (st, ct) = (theta.sin(), theta.cos());
    let (sp, cp) = (phi.sin(), phi.cos());
    let direction = (
        x.0 * st * cp + y.0 * st * sp + z.0 * ct,
        x.1 * st * cp + y.1 * st * sp + z.1 * ct,
        x.2 * st * cp + y.2 * st * sp + z.2 * ct,
    );

    let boresight_target = Position3D::ecef(target.0, target.1, target.2);
    GainRequest {
        antenna_id: ANTENNA_ID.to_string(),
        feed_id: FEED_ID.to_string(),
        vehicle_position: Position3D::ecef(vehicle.0, vehicle.1, vehicle.2),
        reflector_boresight: boresight_target.clone(),
        // Aim the feed at boresight: zero steering displacement, so the served antenna is
        // the focused one the calibration describes.
        feed_pointing_location: boresight_target,
        emitter_position: Position3D::ecef(
            vehicle.0 + direction.0 * range,
            vehicle.1 + direction.1 * range,
            vehicle.2 + direction.2 * range,
        ),
        frequency_mhz,
        pointing_frequency_mhz: None,
        include_reference: false,
        vehicle_attitude: None,
    }
}

fn serve(
    repo: &CalibrationRepository,
    cone_deg: f64,
    clock_deg: f64,
    freq_mhz: f64,
) -> GainResponse {
    compute_gain_from_request(&request_for(cone_deg, clock_deg, freq_mhz), repo)
        .unwrap_or_else(|e| panic!("serving cone {cone_deg}° clock {clock_deg}° failed: {e}"))
}

// ============================================================================

/// The reflection the parser performs must be **physics-preserving**, or D26 fixed a
/// coverage range by silently editing the measurements.
///
/// `(φ, −θ)` and `(φ + 180°, θ)` name the same direction, and the far-field computation
/// agrees: it carries `sin θ` signed through `Jₘ(kR sin θ)`, and `Jₘ(−u) = (−1)ᵐ Jₘ(u)`
/// cancels against `e^{im(φ+π)} = (−1)ᵐ e^{imφ}` mode by mode, while the obliquity factor
/// depends on the even `cos θ`. The residual difference below is quadrature, not physics.
///
/// Checked on an **asymmetric** feed with a lateral offset — the azimuthal-mode branch, where
/// the identity is a real cancellation rather than a triviality. On the symmetric branch the
/// integrand does not depend on φ at all, so it would pass without testing anything.
#[test]
fn negative_cone_measurements_predict_the_same_gain_as_their_reflection() {
    use antenna_model::model::{
        compute_gain_db, AntennaConfigurationBuilder, FeedParametersBuilder, IntegrationParams,
        MeshParametersBuilder, ReflectorGeometryBuilder,
    };

    let focal_length = DIAMETER_M * F_OVER_D;
    let config = AntennaConfigurationBuilder::default()
        .id("polar-identity")
        .name("polar-identity")
        .reflector(
            ReflectorGeometryBuilder::default()
                .diameter(DIAMETER_M)
                .focal_length(focal_length)
                .surface_rms(0.0003)
                .build()
                .expect("reflector"),
        )
        .feed(
            FeedParametersBuilder::default()
                .at_focus(focal_length)
                .q_factor(2.0)
                // Both of the things that put this on the azimuthal-mode branch.
                .asymmetry_factor(1.4)
                .build()
                .expect("feed"),
        )
        .mesh(
            MeshParametersBuilder::default()
                .spacing(0.005)
                .wire_diameter(0.0005)
                .build()
                .expect("mesh"),
        )
        .build()
        .expect("config");

    let params = IntegrationParams::default();
    let mut worst = 0.0_f64;
    for &(cone_deg, clock_deg) in &[(2.0_f64, 0.0_f64), (6.0, 37.0), (10.0, 120.0)] {
        let signed = compute_gain_db(
            -cone_deg.to_radians(),
            clock_deg.to_radians(),
            &config,
            2_400e6,
            &params,
        )
        .expect("signed-cone gain")
        .gain;
        let reflected = compute_gain_db(
            cone_deg.to_radians(),
            (clock_deg + 180.0).to_radians(),
            &config,
            2_400e6,
            &params,
        )
        .expect("reflected gain")
        .gain;

        let err = (signed - reflected).abs();
        worst = worst.max(err);
        assert!(
            err < 1e-3,
            "cone {cone_deg}° clock {clock_deg}°: signed {signed:.9} dB vs reflected \
             {reflected:.9} dB (Δ {err:.3e}) — the reflection must not move the physics"
        );
    }

    // Negative control: a reflection that is *not* the identity must move the answer, or the
    // tolerance above is simply wider than the pattern's variation and proves nothing.
    let here = compute_gain_db(6.0_f64.to_radians(), 0.0, &config, 2_400e6, &params)
        .expect("control")
        .gain;
    let elsewhere = compute_gain_db(
        6.0_f64.to_radians(),
        90.0_f64.to_radians(),
        &config,
        2_400e6,
        &params,
    )
    .expect("control")
    .gain;
    assert!(
        (here - elsewhere).abs() > 1e-2,
        "an asymmetric feed must vary with clock ({here:.6} vs {elsewhere:.6} dB), or this \
         geometry cannot detect a wrong reflection"
    );
    eprintln!("worst signed-vs-reflected gain difference: {worst:e} dB");
}

/// The replication of the antenna frame above must still agree with the service, so a
/// convention change surfaces here rather than as a mysterious correction mismatch below.
#[test]
fn served_geometry_lands_on_the_requested_angles() {
    let (calibration, _, _) = build_artifact();
    let repo = repository_with(calibration);

    for &(cone, clock) in &[(6.0, 45.0), (3.0, 225.0), (9.0, 315.0)] {
        let response = serve(&repo, cone, clock, 2200.0);
        assert!(
            (response.geometry.emitter_elevation_deg - cone).abs() < 1e-6,
            "requested cone {cone}°, served elevation {}°",
            response.geometry.emitter_elevation_deg
        );
        let served_clock = response.geometry.emitter_azimuth_deg;
        assert!(
            (served_clock - clock).abs() < 1e-6 || (served_clock - clock).abs() > 359.999,
            "requested clock {clock}°, served azimuth {served_clock}°"
        );
    }
}

/// Exit criterion 1: the coverage a negative-cone set exports contains its calibrated region,
/// and the service applies the correction inside it.
#[test]
fn correction_is_applied_for_a_negative_cone_measurement_set() {
    let (calibration, surface, points) = build_artifact();

    let coverage = calibration
        .calibration_coverage
        .clone()
        .expect("full-mode artifacts carry coverage");
    assert_eq!(
        coverage.elevation_range,
        (0.0, 10.0),
        "the coverage must be the polar-angle extent of the cut; the pre-D26 clamp gave (0, 0)"
    );
    assert!(
        !coverage.is_boresight_only(),
        "a {}-measurement cut reaching 10° off boresight is not a boresight-only artifact",
        points.len()
    );
    assert_eq!(
        calibration.validity_ranges.elevation_min_max,
        (0.0, 10.0),
        "the validity range must agree with coverage"
    );

    // The control: the same artifact with the coverage the pre-D26 clamp produced. Its
    // correction surface is still present, so both artifacts take the identical physics
    // branch (`physics_is_uncorrected()` is false for both) and the only difference between
    // the two served numbers is the correction term.
    let mut clamped = calibration.clone();
    if let Some(cov) = clamped.calibration_coverage.as_mut() {
        cov.elevation_range = (0.0, 0.0);
    }
    assert!(
        clamped
            .calibration_coverage
            .as_ref()
            .expect("coverage")
            .is_boresight_only(),
        "the control must reproduce the defect it stands for"
    );

    let fixed_repo = repository_with(calibration);
    let clamped_repo = repository_with(clamped);

    // Probe well inside coverage, on several clock planes and frequencies — including the
    // 180°-side planes that only exist because the negative rows were reflected onto them.
    //
    // Kept off the exact azimuth maximum on purpose: the aim geometry reproduces a requested
    // 315° as 315.00000000000017, and `CalibrationCoverage::contains_direction_at_frequency`
    // is a closed comparison,
    // so a probe *on* the boundary tests floating-point luck rather than this fix. That
    // knife-edge is a pre-existing property of coverage tests, not something D26 introduces.
    let probes = [
        (2.0, 45.0, 2000.0),
        (5.0, 225.0, 2200.0),
        (7.5, 135.0, 2400.0),
        (9.5, 300.0, 2080.0),
    ];

    let mut worst = 0.0_f64;
    for &(cone, clock, freq) in &probes {
        let with = serve(&fixed_repo, cone, clock, freq);
        let without = serve(&clamped_repo, cone, clock, freq);

        let status = with
            .calibration_status
            .as_ref()
            .expect("calibration status present");
        assert!(
            status.correction_applied,
            "the correction must be applied at cone {cone}° clock {clock}° — this is the \
             assertion the pre-D26 clamp failed"
        );
        assert!(
            !without
                .calibration_status
                .as_ref()
                .expect("status")
                .correction_applied,
            "the control must serve raw physics, or it is not isolating the correction"
        );

        // The isolated correction term, against the surface evaluated at the served
        // direction. Evaluating the *3D* surface here is deliberate: if the served value had
        // been clamped to a knot-vector edge, or read at the unreflected clock, this is what
        // would catch it.
        let served_correction = with.gain_db - without.gain_db;
        let expected = surface
            .evaluate(
                freq,
                with.geometry.emitter_elevation_deg,
                with.geometry.emitter_azimuth_deg,
            )
            .expect("3D surface evaluation");
        let err = (served_correction - expected).abs();
        worst = worst.max(err);
        assert!(
            err < 1e-6,
            "cone {cone}° clock {clock}° {freq} MHz: served correction {served_correction:.9} dB \
             vs surface {expected:.9} dB (err {err:.3e})"
        );

        // And the correction is a real term, not a rounding artifact that would make the
        // comparison above vacuous.
        assert!(
            served_correction.abs() > 0.01,
            "the residual at cone {cone}° clock {clock}° is {served_correction:.6} dB — too \
             small for this test to distinguish an applied correction from none"
        );
    }
    eprintln!(
        "worst served-vs-surface correction error over {} probes: {worst:e} dB",
        probes.len()
    );
}

/// A cut that never reaches boresight — the case the clamp *inverted* into `(0.0, -1.0)`,
/// a range that rejects everything by construction and fails artifact validation outright.
#[test]
fn a_wholly_negative_cut_exports_a_range_that_contains_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let csv_path = dir.path().join("wholly_negative.csv");
    let mut csv = String::from("e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n");
    for &clock in &CLOCKS_DEG {
        for cone_step in 0..6 {
            // -10°…-1°, never touching boresight.
            let cone = -10.0 + 1.8 * cone_step as f64;
            for &freq in &FREQS_MHZ {
                let g = residual_db((clock + 180.0) % 360.0, -cone, freq);
                csv.push_str(&format!(
                    "{clock:.6},{cone:.6},{freq:.6},{g:.9},{TEMPERATURE_K:.6}\n"
                ));
            }
        }
    }
    std::fs::write(&csv_path, csv).expect("write csv");

    let data = parse_measurements_sync(csv_path.to_str().expect("utf-8")).expect("parse");
    let (lo, hi) = data.e_cone_range();
    assert!(lo > 0.0, "the reflected cut must not reach boresight: {lo}");
    assert!(
        (hi - 10.0).abs() < 1e-9,
        "outermost angle should be 10°, got {hi}"
    );

    let predictions = vec![0.0; data.points.len()];
    let surface =
        fit_correction_surface(&data.points, &predictions, &fitting_params()).expect("surface fit");

    let physical = ExportPhysicalParams {
        diameter_m: DIAMETER_M,
        focal_length_m: DIAMETER_M * F_OVER_D,
        f_over_d_ratio: F_OVER_D,
        surface_rms_mm: 0.3,
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: 2.0,
        phase_center_offset_m: 0.0,
        asymmetry_factor: 1.0,
        mesh: None,
    };

    let calibration = export_full_calibration(
        ANTENNA_ID,
        FEED_ID,
        "Wholly negative cut 1.0m",
        "file://wholly_negative.csv".to_string(),
        &physical,
        &surface,
        &data.points,
        0.05,
        0.999,
        0.5,
        false,
    )
    .expect("a wholly-negative cut is legal input and must export");

    calibration
        .validate()
        .expect("the pre-D26 inverted range (0.0, -1.0) failed this outright");

    let coverage = calibration
        .calibration_coverage
        .expect("coverage")
        .elevation_range;
    assert!(
        coverage.0 <= lo && coverage.1 >= hi,
        "coverage {coverage:?} must contain the calibrated region [{lo}, {hi}]"
    );
    assert!(coverage.0 < coverage.1, "the range must not be inverted");
}

/// The export refuses an extent that is not in the polar convention rather than clamping it.
///
/// The clamp could not distinguish "already correct" from "silently truncated", which is
/// exactly how the defect survived. This is the guard that makes the parser's normalization a
/// precondition instead of an assumption.
#[test]
fn export_refuses_measurements_that_never_went_through_the_normalization() {
    let dir = tempfile::tempdir().expect("tempdir");
    let csv_path = dir.path().join("negative_cone_cut.csv");
    std::fs::write(&csv_path, negative_cone_csv()).expect("write csv");
    let data = parse_measurements_sync(csv_path.to_str().expect("utf-8")).expect("parse");
    let predictions = vec![0.0; data.points.len()];
    let surface =
        fit_correction_surface(&data.points, &predictions, &fitting_params()).expect("surface fit");

    // Hand the exporter the *unnormalized* rows, as a library caller bypassing the parser
    // would.
    let raw: Vec<MeasurementPoint> = data
        .points
        .iter()
        .map(|p| {
            MeasurementPoint::new(
                p.e_clock_deg,
                -p.e_cone_deg.abs(),
                p.frequency_mhz,
                p.g_over_t_db,
                p.temperature_k,
            )
        })
        .collect();

    let physical = ExportPhysicalParams {
        diameter_m: DIAMETER_M,
        focal_length_m: DIAMETER_M * F_OVER_D,
        f_over_d_ratio: F_OVER_D,
        surface_rms_mm: 0.3,
        feed_position_m: (0.0, 0.0, 0.0),
        q_factor: 2.0,
        phase_center_offset_m: 0.0,
        asymmetry_factor: 1.0,
        mesh: None,
    };

    let err = export_full_calibration(
        ANTENNA_ID,
        FEED_ID,
        "Unnormalized 1.0m",
        "file://raw.csv".to_string(),
        &physical,
        &surface,
        &raw,
        0.05,
        0.999,
        0.5,
        false,
    )
    .expect_err("an out-of-convention E-cone extent must be refused, not clamped");
    let message = err.to_string();
    assert!(
        message.contains("polar convention"),
        "the error must say what is wrong and how to fix it, got: {message}"
    );
}
