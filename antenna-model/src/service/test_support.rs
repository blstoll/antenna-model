//! Shared test-only calibration fixtures for the service layer.

use crate::data::types::{
    AntennaCalibration, CalibrationMetadata, CalibrationStatus, FeedParameters, MeshParameters,
    PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
};
/// The canonical service-layer test artifact: a 10 m / f/D 0.5 dish with a mesh, an
/// on-axis feed, and validity ranges wide enough that nothing extrapolates by accident.
///
/// Shared by `service::evaluator` (end-to-end `/gain` behaviour) and
/// `service::served_gain` (the gain law itself) since issue #61 split the two — before
/// that both lived in one `mod tests` and this was private to it.
///
/// A `PartiallyCalibrated` status also installs its coverage on the artifact, so the
/// status and the coverage record cannot disagree in a fixture.
pub(crate) fn create_test_calibration(status: CalibrationStatus) -> AntennaCalibration {
    let metadata = CalibrationMetadata::builder()
        .antenna_name("Test Antenna")
        .calibration_date("2025-01-01T00:00:00Z")
        .format_version("2.0")
        .data_source("test")
        .rmse_db(0.5)
        .r_squared(0.99)
        .num_measurements(1000)
        .build()
        .unwrap();

    let mut builder = AntennaCalibration::builder()
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
                // Feed at focal point - zero offset from optical axis
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
        });

    builder = builder.calibration_status(status.clone());

    // Add coverage for partially calibrated
    if let CalibrationStatus::PartiallyCalibrated { ref coverage, .. } = status {
        builder = builder.calibration_coverage(coverage.clone());
    }

    builder.build().unwrap()
}

/// A valid, evaluable correction surface for tests that need to represent
/// "corrected physics" (surface present ⇒ off-axis warning silent, spillover
/// off). All coefficients are zero, so it contributes 0 dB wherever it is
/// evaluated; its clamped knot vectors span the full validity range so an
/// end-to-end query does not extrapolate. Only its PRESENCE matters for the
/// P11 predicate, but making it evaluable lets it also be used in the
/// end-to-end `compute_gain` tests.
pub(crate) fn dummy_correction_surface() -> crate::data::types::BSplineModel4D {
    crate::data::types::BSplineModel4D {
        coefficients: vec![0.0; 2 * 2 * 2],
        shape: [2, 2, 2, 1],
        knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
        knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
        knots_frequency: vec![1000.0, 1000.0, 1000.0, 10000.0, 10000.0, 10000.0],
        knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
        spline_order: 3,
    }
}
