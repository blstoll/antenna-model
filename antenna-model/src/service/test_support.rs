//! Shared test-only calibration fixtures for the service layer.

use crate::model::{CorrectionSurfaceLayout, FittedCorrectionSurface};
use antenna_core::types::{
    AntennaCalibration, AntennaCalibrationBuilder, CalibrationCoverage, CalibrationMetadata,
    CalibrationStatus, FeedParameters, MeshParameters, PhysicalAntennaConfig, ReflectorGeometry,
    ValidityRanges,
};
/// The one builder for antenna-model test artifacts: a 10 m / f/D 0.5 dish with a mesh, an
/// on-axis feed, and validity ranges wide enough that nothing extrapolates by accident.
/// Override ids, status or physics before `build()`.
pub(crate) fn calibration_builder() -> AntennaCalibrationBuilder {
    let metadata = CalibrationMetadata::builder()
        .antenna_name("Test Antenna")
        .calibration_date("2025-01-01T00:00:00Z")
        .data_source("test")
        .rmse_db(0.5)
        .r_squared(0.99)
        .num_measurements(1000)
        .build()
        .unwrap();

    AntennaCalibration::builder()
        .antenna_id("test_antenna")
        .feed_id("test_feed")
        .metadata(metadata)
        .physical_config(PhysicalAntennaConfig {
            reflector: ReflectorGeometry {
                diameter_m: 10.0,
                focal_length_m: 5.0,
                f_over_d_ratio: 0.5,
                surface_rms_mm: 0.5,
            },
            feed: FeedParameters {
                // Focus-relative: an on-axis feed is the origin (C13).
                position: (0.0, 0.0, 0.0),
                q_factor: 8.0,
                phase_center_offset_m: 0.0,
                axial_defocus_m: 0.0,
                asymmetry_factor: 1.0,
            },
            mesh: Some(MeshParameters {
                mesh_spacing_mm: 5.0,
                wire_diameter_mm: 0.5,
            }),
        })
        .validity_ranges(ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (1000.0, 10000.0),
            temperature_const: 290.0,
        })
}

/// [`calibration_builder`]'s artifact with `status`. A `PartiallyCalibrated` status also
/// installs its coverage, so the two records cannot disagree.
pub(crate) fn create_test_calibration(status: CalibrationStatus) -> AntennaCalibration {
    let builder = match &status {
        CalibrationStatus::PartiallyCalibrated { coverage, .. } => {
            calibration_builder().calibration_coverage(coverage.clone())
        }
        _ => calibration_builder(),
    };
    builder.calibration_status(status).build().unwrap()
}

/// A surface over the given clamped knot vectors whose every coefficient is `value_db`, so
/// it evaluates to exactly `value_db` everywhere in its support (partition of unity). Each
/// axis's coefficient count is its knot count minus `spline_order`.
pub(crate) fn constant_surface(
    spline_order: u8,
    knots_e_clock: Vec<f64>,
    knots_e_cone: Vec<f64>,
    knots_frequency: Vec<f64>,
    value_db: f64,
) -> FittedCorrectionSurface {
    let order = usize::from(spline_order);
    let shape = [
        knots_e_clock.len() - order,
        knots_e_cone.len() - order,
        knots_frequency.len() - order,
    ];
    let layout = CorrectionSurfaceLayout::new(
        shape,
        knots_e_clock,
        knots_e_cone,
        knots_frequency,
        spline_order,
    )
    .expect("fixture correction surface must be valid");
    FittedCorrectionSurface::new(layout, vec![value_db; shape.iter().product()])
        .expect("fixture coefficient count matches its layout")
}

/// A correction surface that evaluates to exactly `correction_db` anywhere in its
/// span. Shaped the way `calibrate::fit_frequency_correction` writes a flat-axis
/// frequency correction — `order + 1` identical layers per axis over the full
/// queryable span — because a B-spline is only a partition of unity when its
/// coefficient count actually matches its knot vectors. (`dummy_correction_surface`
/// is deliberately not reused here: it exists to make a surface *present*, and its
/// all-zero coefficients hide the shape mismatch.)
pub(crate) fn constant_correction_surface(correction_db: f64) -> FittedCorrectionSurface {
    constant_surface(
        3,
        vec![0.0, 0.0, 0.0, 180.0, 360.0, 360.0, 360.0],
        vec![0.0, 0.0, 0.0, 90.0, 180.0, 180.0, 180.0],
        vec![8000.0, 8000.0, 8000.0, 8300.0, 9000.0, 9000.0, 9000.0],
        correction_db,
    )
}

/// A valid, evaluable correction surface for tests that need to represent
/// "corrected physics" (surface present ⇒ off-axis warning silent, spillover
/// off). All coefficients are zero, so it contributes 0 dB wherever it is
/// evaluated; its clamped knot vectors span the full validity range so an
/// end-to-end query does not extrapolate. Only its PRESENCE matters for the
/// P11 predicate, but making it evaluable lets it also be used in the
/// end-to-end `compute_gain` tests.
pub(crate) fn dummy_correction_surface() -> FittedCorrectionSurface {
    constant_surface(
        2,
        vec![0.0, 0.0, 360.0, 360.0],
        vec![0.0, 0.0, 90.0, 90.0],
        vec![1000.0, 1000.0, 10000.0, 10000.0],
        0.0,
    )
}

/// Attach `surface` to `calibration` the way a valid artifact carries one: with a coverage
/// record its fitted support contains (issue #97).
///
/// A fixture that already records coverage — a partially calibrated one, or a test that
/// narrowed it deliberately — keeps it. Otherwise the coverage is the surface's whole
/// support, the relationship a full-mode artifact has.
pub(crate) fn install_correction_surface(
    calibration: &mut AntennaCalibration,
    surface: FittedCorrectionSurface,
) {
    let support = surface.layout().support();
    calibration
        .calibration_coverage
        .get_or_insert_with(|| CalibrationCoverage::from_domain(support, 1_000, true));
    calibration.correction_surface = Some(surface);
}
