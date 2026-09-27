use serde::{Deserialize, Serialize};

use super::{
    BSplineModel4D, CalibrationCoverage, CalibrationMetadata, CalibrationStatus,
    PhysicalAntennaConfig, ValidationError, ValidityRanges,
};
use crate::model::correction_surface::{CoveredCorrectionSurface, FittedCorrectionSurface};

/// Complete calibration data for one antenna-feed combination — the payload of one
/// `.bin` artifact.
///
/// Served gain is `physics(physical_config) + correction_surface(freq, cone, clock)`,
/// with the correction applied only inside the coverage the artifact records. An
/// antenna with several feeds has one artifact per feed; the repository keys them by
/// `(antenna_id, feed_id)`.
///
/// Invariants, checked by [`Self::validate`]: non-empty ids; every nested value valid;
/// a correction surface only together with a coverage record contained by its fitted
/// support; and a `PartiallyCalibrated` status carrying the same coverage as
/// `calibration_coverage`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AntennaCalibration {
    /// Unique identifier for this antenna.
    pub antenna_id: String,

    /// Identifier for this feed (e.g. `"x_band"`, `"primary"`), unique per antenna.
    pub feed_id: String,

    /// Provenance and fit-quality metadata.
    pub metadata: CalibrationMetadata,

    /// Physical parameters of the physics model.
    pub physical_config: PhysicalAntennaConfig,

    /// Residual correction surface fitted to `measured − physics`, if any.
    #[serde(default)]
    pub correction_surface: Option<BSplineModel4D>,

    /// Parameter ranges reported in antenna metadata.
    pub validity_ranges: ValidityRanges,

    /// Level of calibration data available.
    #[serde(default)]
    pub calibration_status: Option<CalibrationStatus>,

    /// Where measurements justify applying the correction surface. Required whenever
    /// `correction_surface` is present; read it through [`Self::coverage`].
    #[serde(default)]
    pub calibration_coverage: Option<CalibrationCoverage>,
}

impl AntennaCalibration {
    /// Creates a new builder for constructing an `AntennaCalibration`.
    pub fn builder() -> AntennaCalibrationBuilder {
        AntennaCalibrationBuilder::default()
    }

    /// True iff there is no correction surface, so the served gain is raw physics.
    ///
    /// This — not `CalibrationStatus::Uncalibrated` — is the gate for every
    /// uncorrected-physics behaviour: a `PartiallyCalibrated` artifact without a
    /// frequency correction is uncorrected too. Pass it to
    /// [`IntegrationParams::with_uncorrected_physics_gates`](crate::model::IntegrationParams::with_uncorrected_physics_gates)
    /// rather than setting the gated flags by hand, so the service and `calibrate`
    /// answer the question with the same code. See D17.
    pub fn physics_is_uncorrected(&self) -> bool {
        self.correction_surface.is_none()
    }

    /// The one calibration-coverage claim this artifact makes.
    ///
    /// `calibration_coverage` is the authority. A `PartiallyCalibrated` status carries a
    /// serialized duplicate, and an artifact whose two copies disagree is refused. See #97.
    pub fn coverage(&self) -> Result<Option<&CalibrationCoverage>, ValidationError> {
        match &self.calibration_status {
            Some(CalibrationStatus::PartiallyCalibrated { coverage, .. })
                if self.calibration_coverage.as_ref() != Some(coverage) =>
            {
                Err(ValidationError::CoverageRecordsDisagree)
            }
            _ => Ok(self.calibration_coverage.as_ref()),
        }
    }

    /// The executable correction surface paired with the coverage that gates it, or
    /// `None` when the artifact carries no correction surface.
    ///
    /// Refuses a surface with no coverage record (it is not "covered everywhere") and
    /// coverage beyond the surface's fitted support. See [`CoveredCorrectionSurface`].
    pub fn covered_correction_surface(
        &self,
    ) -> Result<Option<CoveredCorrectionSurface>, ValidationError> {
        self.correction_surface
            .as_ref()
            .map(|model| {
                let surface = FittedCorrectionSurface::from_model4d(model)?;
                let coverage = self
                    .coverage()?
                    .cloned()
                    .ok_or(ValidationError::MissingCoverage)?;
                CoveredCorrectionSurface::new(surface, coverage)
            })
            .transpose()
    }

    /// Checks every invariant listed on the type.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.antenna_id.is_empty() {
            return Err(ValidationError::EmptyField("antenna_id".to_string()));
        }
        if self.feed_id.is_empty() {
            return Err(ValidationError::EmptyField("feed_id".to_string()));
        }

        self.physical_config.validate()?;
        self.validity_ranges.validate()?;
        if let Some(ref coverage) = self.calibration_coverage {
            coverage.validate()?;
        }

        // Constructing the covered surface also validates the surface itself.
        self.coverage()?;
        self.covered_correction_surface()?;

        if let Some(ref resolution) = self.metadata.angular_resolution {
            resolution.validate()?;
        }

        Ok(())
    }
}

/// Builder for [`AntennaCalibration`]. `antenna_id`, `feed_id`, `metadata`,
/// `physical_config` and `validity_ranges` are required; `build` does not validate.
#[derive(Default)]
pub struct AntennaCalibrationBuilder {
    antenna_id: Option<String>,
    feed_id: Option<String>,
    metadata: Option<CalibrationMetadata>,
    physical_config: Option<PhysicalAntennaConfig>,
    correction_surface: Option<BSplineModel4D>,
    validity_ranges: Option<ValidityRanges>,
    calibration_status: Option<CalibrationStatus>,
    calibration_coverage: Option<CalibrationCoverage>,
}

impl AntennaCalibrationBuilder {
    pub fn antenna_id(mut self, id: impl Into<String>) -> Self {
        self.antenna_id = Some(id.into());
        self
    }

    pub fn feed_id(mut self, id: impl Into<String>) -> Self {
        self.feed_id = Some(id.into());
        self
    }

    pub fn metadata(mut self, metadata: CalibrationMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn physical_config(mut self, config: PhysicalAntennaConfig) -> Self {
        self.physical_config = Some(config);
        self
    }

    pub fn correction_surface(mut self, correction: BSplineModel4D) -> Self {
        self.correction_surface = Some(correction);
        self
    }

    pub fn validity_ranges(mut self, ranges: ValidityRanges) -> Self {
        self.validity_ranges = Some(ranges);
        self
    }

    pub fn calibration_status(mut self, status: CalibrationStatus) -> Self {
        self.calibration_status = Some(status);
        self
    }

    pub fn calibration_coverage(mut self, coverage: CalibrationCoverage) -> Self {
        self.calibration_coverage = Some(coverage);
        self
    }

    /// Mark the artifact `PartiallyCalibrated`, writing the status's coverage and
    /// `calibration_coverage` from the one `coverage` value so they cannot disagree.
    pub fn partially_calibrated(
        self,
        accuracy_estimate_db: f64,
        coverage: CalibrationCoverage,
    ) -> Self {
        self.calibration_status(CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db,
            coverage: coverage.clone(),
        })
        .calibration_coverage(coverage)
    }

    pub fn build(self) -> Result<AntennaCalibration, String> {
        Ok(AntennaCalibration {
            antenna_id: self.antenna_id.ok_or("antenna_id is required")?,
            feed_id: self.feed_id.ok_or("feed_id is required")?,
            metadata: self.metadata.ok_or("metadata is required")?,
            physical_config: self.physical_config.ok_or("physical_config is required")?,
            correction_surface: self.correction_surface,
            validity_ranges: self.validity_ranges.ok_or("validity_ranges is required")?,
            calibration_status: self.calibration_status,
            calibration_coverage: self.calibration_coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AngularResolution, FeedParameters, MeasurementDensity, ParameterSource, ReflectorGeometry,
    };

    fn create_test_physical_config() -> PhysicalAntennaConfig {
        let reflector = ReflectorGeometry::builder()
            .diameter_m(34.0)
            .focal_length_m(13.6)
            .f_over_d_ratio(0.4)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.1)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap()
    }

    #[test]
    fn test_antenna_calibration_builder() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("primary")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        assert_eq!(calibration.antenna_id, "test_antenna");
        assert_eq!(calibration.feed_id, "primary");
        assert!(calibration.validate().is_ok());
        assert!(calibration.correction_surface.is_none());
    }

    #[test]
    fn test_antenna_calibration_validate() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        // Invalid: empty antenna ID
        let invalid_calibration = AntennaCalibration {
            antenna_id: "".to_string(),
            feed_id: "primary".to_string(),
            metadata: metadata.clone(),
            physical_config: physical_config.clone(),
            correction_surface: None,
            validity_ranges: ranges.clone(),
            calibration_status: None,
            calibration_coverage: None,
        };
        assert!(invalid_calibration.validate().is_err());

        // Invalid: empty feed ID
        let invalid_calibration = AntennaCalibration {
            antenna_id: "test".to_string(),
            feed_id: "".to_string(),
            metadata: metadata.clone(),
            physical_config: physical_config.clone(),
            correction_surface: None,
            validity_ranges: ranges.clone(),
            calibration_status: None,
            calibration_coverage: None,
        };
        assert!(invalid_calibration.validate().is_err());

        // Valid calibration
        let valid_calibration = AntennaCalibration {
            antenna_id: "test".to_string(),
            feed_id: "primary".to_string(),
            metadata,
            physical_config,
            correction_surface: None,
            validity_ranges: ranges,
            calibration_status: None,
            calibration_coverage: None,
        };
        assert!(valid_calibration.validate().is_ok());
    }

    #[test]
    fn test_serialization_round_trip() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let correction = BSplineModel4D::builder()
            .coefficients(vec![1.0, 2.0, 3.0, 4.0])
            .shape([2, 2, 1, 1])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let original = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("x_band")
            .metadata(metadata)
            .physical_config(physical_config)
            .correction_surface(correction)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        // Test postcard serialization round-trip
        let encoded = postcard::to_allocvec(&original).unwrap();
        let decoded: AntennaCalibration = postcard::from_bytes(&encoded).unwrap();

        assert_eq!(original, decoded);
        assert_eq!(original.antenna_id, decoded.antenna_id);
        assert_eq!(original.feed_id, decoded.feed_id);
        assert_eq!(
            original.physical_config.reflector.diameter_m,
            decoded.physical_config.reflector.diameter_m
        );
        assert!(original.correction_surface.is_some());
        assert!(decoded.correction_surface.is_some());
    }

    #[test]
    fn test_serialization_round_trip_json() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let original = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("primary")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap();

        // Test JSON serialization
        let json = serde_json::to_string(&original).unwrap();
        let decoded: AntennaCalibration = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
        assert_eq!(decoded.feed_id, "primary");
    }

    #[test]
    fn test_antenna_calibration_with_partial_calibration_fields() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .parameters_source(ParameterSource::BoresightTuning {
                num_measurements: 28,
            })
            .measurement_density(MeasurementDensity::BoresightOnly)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 0.0)
            .elevation_range(0.0, 0.0)
            .frequency_range(7100.0, 8500.0)
            .num_measurements(28)
            .has_correction_surface(false)
            .build()
            .unwrap();

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        };

        let calibration = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("x_band")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .calibration_status(status)
            .calibration_coverage(coverage)
            .build()
            .unwrap();

        assert!(calibration.validate().is_ok());
        assert!(calibration.calibration_status.is_some());
        assert!(calibration.calibration_coverage.is_some());
    }

    #[test]
    fn test_serialization_backward_compatibility() {
        // Test that old calibrations (without new fields) can still be deserialized
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2025-01-15")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();

        let physical_config = create_test_physical_config();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(0.0, 90.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        // Build calibration without new optional fields
        let calibration = AntennaCalibration {
            antenna_id: "test_antenna".to_string(),
            feed_id: "primary".to_string(),
            metadata,
            physical_config,
            correction_surface: None,
            validity_ranges: ranges,
            calibration_status: None,   // Old format - no status
            calibration_coverage: None, // Old format - no coverage
        };

        // Should still be valid
        assert!(calibration.validate().is_ok());

        // Should serialize/deserialize correctly
        let encoded = postcard::to_allocvec(&calibration).unwrap();
        let decoded: AntennaCalibration = postcard::from_bytes(&encoded).unwrap();

        assert_eq!(calibration, decoded);
        assert!(decoded.calibration_status.is_none());
        assert!(decoded.calibration_coverage.is_none());
    }

    /// Guards against a meaningless deserialized angular-resolution assessment loading
    /// cleanly (D26 finding 5).
    #[test]
    fn an_artifact_carrying_an_uninterpretable_assessment_is_refused_at_validate() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test")
            .calibration_date("2026-08-13")
            .data_source("test.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(100)
            .build()
            .unwrap();
        let mut cal = AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("primary")
            .metadata(metadata)
            .physical_config(create_test_physical_config())
            .validity_ranges(
                ValidityRanges::builder()
                    .azimuth_range(0.0, 360.0)
                    .elevation_range(0.0, 90.0)
                    .frequency_range(8000.0, 8500.0)
                    .temperature(290.0)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        assert!(
            cal.validate().is_ok(),
            "the fixture must validate before the field is corrupted"
        );

        cal.metadata.angular_resolution = Some(AngularResolution {
            cone_knot_spacing_deg: 0.0,
            cone_lobe_period_deg: 1.16,
            clock_knot_spacing_deg: 40.0,
            clock_lobe_period_deg: 4.8,
        });
        let err = cal
            .validate()
            .expect_err("a zero knot spacing must not load as a valid artifact");
        assert!(
            matches!(err, ValidationError::InvalidAngularResolution { .. }),
            "expected InvalidAngularResolution, got {err:?}"
        );

        // A well-formed assessment — including the legal infinite clock period — still passes.
        cal.metadata.angular_resolution = Some(AngularResolution {
            cone_knot_spacing_deg: 2.0,
            cone_lobe_period_deg: 1.16,
            clock_knot_spacing_deg: 40.0,
            clock_lobe_period_deg: f64::INFINITY,
        });
        assert!(cal.validate().is_ok());
    }

    /// Coverage is an artifact invariant, contained by — not equal to — the correction
    /// surface's fitted support (#97).
    mod coverage_containment {
        use super::*;
        use crate::model::FittedCorrectionSurface;
        use crate::model::{ClampedAxis, CorrectionDomain, CorrectionSurfaceLayout};

        const FULL_MODE: CorrectionDomain = CorrectionDomain {
            e_clock_deg: (10.0, 80.0),
            e_cone_deg: (0.5, 30.0),
            frequency_mhz: (8_000.0, 8_500.0),
        };

        /// A flat surface whose support is exactly `support` on every axis.
        fn surface_over(support: CorrectionDomain) -> BSplineModel4D {
            let flat = |(lower, upper): (f64, f64)| ClampedAxis::flat(lower, upper);
            let layout = CorrectionSurfaceLayout::clamped(
                flat(support.e_clock_deg),
                flat(support.e_cone_deg),
                flat(support.frequency_mhz),
                4,
            )
            .unwrap();
            let coefficients = vec![0.5; layout.coefficient_count()];
            FittedCorrectionSurface::new(layout, coefficients)
                .unwrap()
                .to_model4d(280.0, 300.0)
                .unwrap()
        }

        fn metadata() -> CalibrationMetadata {
            CalibrationMetadata::builder()
                .antenna_name("Test")
                .calibration_date("2026-09-26")
                .data_source("test.csv")
                .rmse_db(0.5)
                .r_squared(0.98)
                .num_measurements(100)
                .build()
                .unwrap()
        }

        fn base() -> AntennaCalibrationBuilder {
            AntennaCalibration::builder()
                .antenna_id("test_antenna")
                .feed_id("primary")
                .metadata(metadata())
                .physical_config(create_test_physical_config())
                .validity_ranges(
                    ValidityRanges::builder()
                        .azimuth_range(0.0, 360.0)
                        .elevation_range(0.0, 90.0)
                        .frequency_range(8_000.0, 8_500.0)
                        .temperature(290.0)
                        .build()
                        .unwrap(),
                )
        }

        /// A full-mode artifact: surface support and coverage are one measured domain.
        fn full_mode(support: CorrectionDomain, coverage: CorrectionDomain) -> AntennaCalibration {
            base()
                .correction_surface(surface_over(support))
                .calibration_status(CalibrationStatus::FullyCalibrated {
                    accuracy_estimate_db: 0.5,
                })
                .calibration_coverage(CalibrationCoverage::from_domain(coverage, 100, true))
                .build()
                .unwrap()
        }

        fn assert_exceeds(calibration: &AntennaCalibration, axis: &str) {
            match calibration.validate() {
                Err(ValidationError::CoverageExceedsSupport { ref dimension, .. })
                    if dimension == axis => {}
                other => panic!("expected coverage to exceed support on {axis}, got {other:?}"),
            }
        }

        #[test]
        fn full_mode_coverage_equal_to_support_loads() {
            let calibration = full_mode(FULL_MODE, FULL_MODE);
            assert_eq!(calibration.validate(), Ok(()));
            let covered = calibration
                .covered_correction_surface()
                .unwrap()
                .expect("a surface is present");
            assert_eq!(covered.surface().layout().support(), FULL_MODE);
            assert_eq!(covered.coverage().domain(), FULL_MODE);
        }

        #[test]
        fn boresight_coverage_is_a_strict_subset_of_its_flat_support() {
            let support = CorrectionDomain {
                e_clock_deg: (0.0, 360.0),
                e_cone_deg: (0.0, 180.0),
                frequency_mhz: (7_100.0, 8_500.0),
            };
            let coverage = CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, true);
            assert!(coverage.is_boresight_only());
            assert_ne!(
                coverage.domain(),
                support,
                "boresight coverage is deliberately narrower"
            );

            let calibration = base()
                .correction_surface(surface_over(support))
                .partially_calibrated(1.5, coverage.clone())
                .build()
                .unwrap();

            assert_eq!(calibration.validate(), Ok(()));
            assert_eq!(calibration.coverage(), Ok(Some(&coverage)));
        }

        /// Containment is inclusive: a coverage bound equal to a support bound is inside,
        /// and the next representable value past it on any axis is not.
        #[test]
        fn containment_is_inclusive_at_every_support_bound() {
            assert_eq!(full_mode(FULL_MODE, FULL_MODE).validate(), Ok(()));

            type AxisOf = fn(&mut CorrectionDomain) -> &mut (f64, f64);
            let axes: [(&str, AxisOf); 3] = [
                ("azimuth (E-clock)", |d| &mut d.e_clock_deg),
                ("elevation (E-cone)", |d| &mut d.e_cone_deg),
                ("frequency", |d| &mut d.frequency_mhz),
            ];
            for (name, axis) in axes {
                let mut below = FULL_MODE;
                axis(&mut below).0 = axis(&mut below).0.next_down();
                assert_exceeds(&full_mode(FULL_MODE, below), name);

                let mut above = FULL_MODE;
                axis(&mut above).1 = axis(&mut above).1.next_up();
                assert_exceeds(&full_mode(FULL_MODE, above), name);
            }
        }

        #[test]
        fn non_finite_coverage_is_not_contained() {
            let mut coverage = FULL_MODE;
            coverage.frequency_mhz.1 = f64::NAN;
            assert_exceeds(&full_mode(FULL_MODE, coverage), "frequency");
        }

        #[test]
        fn a_correction_surface_without_coverage_is_rejected() {
            let calibration = base()
                .correction_surface(surface_over(FULL_MODE))
                .calibration_status(CalibrationStatus::FullyCalibrated {
                    accuracy_estimate_db: 0.5,
                })
                .build()
                .unwrap();

            let error = calibration.validate().unwrap_err();
            assert_eq!(error, ValidationError::MissingCoverage);
            let message = error.to_string();
            assert!(
                message.contains("calibration_coverage") && message.contains("correction_surface"),
                "the error must name the fields involved: {message}"
            );
        }

        #[test]
        fn coverage_is_not_required_without_a_correction_surface() {
            let calibration = base()
                .calibration_status(CalibrationStatus::Uncalibrated {
                    accuracy_estimate_db: 3.0,
                    loss_accuracy_estimate_db: 2.0,
                })
                .build()
                .unwrap();
            assert_eq!(calibration.validate(), Ok(()));
            assert_eq!(calibration.covered_correction_surface(), Ok(None));
        }

        #[test]
        fn the_exceeds_error_names_the_axis_and_both_intervals() {
            let mut coverage = FULL_MODE;
            coverage.e_cone_deg.1 = 45.0;
            let message = full_mode(FULL_MODE, coverage)
                .validate()
                .unwrap_err()
                .to_string();
            for expected in ["elevation (E-cone)", "45", "30", "calibration_coverage"] {
                assert!(
                    message.contains(expected),
                    "{expected:?} missing from: {message}"
                );
            }
        }

        #[test]
        fn disagreeing_duplicate_partial_coverage_is_rejected() {
            let status_coverage =
                CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, false);
            let other = CalibrationCoverage::from_domain(FULL_MODE, 28, false);

            let disagreeing = base()
                .calibration_status(CalibrationStatus::PartiallyCalibrated {
                    accuracy_estimate_db: 1.5,
                    coverage: status_coverage.clone(),
                })
                .calibration_coverage(other)
                .build()
                .unwrap();
            assert_eq!(
                disagreeing.validate(),
                Err(ValidationError::CoverageRecordsDisagree)
            );
            assert_eq!(
                disagreeing.coverage(),
                Err(ValidationError::CoverageRecordsDisagree)
            );

            let missing_top_level = base()
                .calibration_status(CalibrationStatus::PartiallyCalibrated {
                    accuracy_estimate_db: 1.5,
                    coverage: status_coverage,
                })
                .build()
                .unwrap();
            let error = missing_top_level.validate().unwrap_err();
            assert_eq!(error, ValidationError::CoverageRecordsDisagree);
            assert!(error.to_string().contains("calibration_status"));
        }

        #[test]
        fn partially_calibrated_construction_writes_both_records_from_one_value() {
            let coverage = CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, false);
            let calibration = base()
                .partially_calibrated(1.5, coverage.clone())
                .build()
                .unwrap();

            assert_eq!(
                calibration.calibration_status,
                Some(CalibrationStatus::PartiallyCalibrated {
                    accuracy_estimate_db: 1.5,
                    coverage: coverage.clone(),
                })
            );
            assert_eq!(calibration.calibration_coverage, Some(coverage));
            assert_eq!(calibration.validate(), Ok(()));
        }
    }
}
