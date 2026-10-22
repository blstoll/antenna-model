//! The one builder-based fixture for tests that need a valid artifact.

use super::{
    AntennaCalibration, AntennaCalibrationBuilder, CalibrationCoverage, CalibrationMetadata,
    CalibrationStatus, CorrectionDomain, FeedParameters, PhysicalAntennaConfig, ReflectorGeometry,
    ValidityRanges,
};
use crate::model::{ClampedAxis, CorrectionSurfaceLayout, FittedCorrectionSurface};

/// The domain [`calibrated`] covers and its surface supports.
pub(crate) const DOMAIN: CorrectionDomain = CorrectionDomain {
    e_clock_deg: (0.0, 360.0),
    e_cone_deg: (0.0, 30.0),
    frequency_mhz: (8_000.0, 8_500.0),
};

pub(crate) fn physical_config() -> PhysicalAntennaConfig {
    PhysicalAntennaConfig::builder()
        .reflector(
            ReflectorGeometry::builder()
                .diameter_m(34.0)
                .focal_length_m(13.6)
                .f_over_d_ratio(0.4)
                .surface_rms_mm(0.5)
                .build()
                .unwrap(),
        )
        .feed(
            FeedParameters::builder()
                .position(0.0, 0.0, 0.1)
                .q_factor(8.0)
                .build()
                .unwrap(),
        )
        .build()
        .unwrap()
}

pub(crate) fn metadata() -> CalibrationMetadata {
    CalibrationMetadata::builder()
        .antenna_name("Test Antenna")
        .calibration_date("2025-01-15T00:00:00Z")
        .data_source("test_data.csv")
        .rmse_db(0.5)
        .r_squared(0.98)
        .num_measurements(1000)
        .build()
        .unwrap()
}

pub(crate) fn validity_ranges() -> ValidityRanges {
    ValidityRanges::builder()
        .azimuth_range(0.0, 360.0)
        .elevation_range(0.0, 90.0)
        .frequency_range(8_000.0, 8_500.0)
        .temperature(290.0)
        .build()
        .unwrap()
}

/// A valid, uncalibrated artifact's builder: every required field set, no surface.
pub(crate) fn builder() -> AntennaCalibrationBuilder {
    AntennaCalibration::builder()
        .antenna_id("test_antenna")
        .feed_id("x_band")
        .metadata(metadata())
        .physical_config(physical_config())
        .validity_ranges(validity_ranges())
}

/// A linear (order 2) surface whose support is exactly `support`, with the given eight
/// coefficients in canonical E-clock-fastest order.
fn linear_surface_over(
    support: CorrectionDomain,
    coefficients: Vec<f64>,
) -> FittedCorrectionSurface {
    let axis = |(lower, upper): (f64, f64)| ClampedAxis::new(lower, upper, vec![]);
    let layout = CorrectionSurfaceLayout::clamped(
        axis(support.e_clock_deg),
        axis(support.e_cone_deg),
        axis(support.frequency_mhz),
        2,
    )
    .unwrap();
    FittedCorrectionSurface::new(layout, coefficients).unwrap()
}

/// A linear (order 2) surface whose support is exactly `support`, constant `value_db`.
pub(crate) fn surface_over(support: CorrectionDomain, value_db: f64) -> FittedCorrectionSurface {
    linear_surface_over(support, vec![value_db; 8])
}

/// A linear (order 2) surface whose support is exactly `support`, with the coefficient at
/// `(i_clock, i_cone, i_freq)` equal to `100·i_freq + 10·i_cone + i_clock`.
pub(crate) fn indexed_surface_over(support: CorrectionDomain) -> FittedCorrectionSurface {
    linear_surface_over(
        support,
        (0..2)
            .flat_map(|freq| {
                (0..2).flat_map(move |cone| (0..2).map(move |clock| (freq, cone, clock)))
            })
            .map(|(freq, cone, clock)| f64::from(100 * freq + 10 * cone + clock))
            .collect(),
    )
}

/// A valid fully-calibrated artifact's builder: a surface over [`DOMAIN`], covering it.
pub(crate) fn calibrated() -> AntennaCalibrationBuilder {
    builder()
        .correction_surface(surface_over(DOMAIN, 0.5))
        .calibration_status(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 0.5,
        })
        .calibration_coverage(CalibrationCoverage::from_domain(DOMAIN, 1000, true))
}
