use serde::{Deserialize, Serialize};

use super::{
    BSplineModel4D, CalibrationCoverage, CalibrationMetadata, CalibrationStatus,
    PhysicalAntennaConfig, ValidationError, ValidityRanges,
};

/// Complete calibration data for one antenna-feed combination — the payload of one
/// `.bin` artifact.
///
/// Served gain is `physics(physical_config) + correction_surface(freq, cone, clock)`,
/// with the correction applied only inside the coverage the artifact records. An
/// antenna with several feeds has one artifact per feed; the repository keys them by
/// `(antenna_id, feed_id)`.
///
/// Artifact invariants ([`crate::artifact`]): non-empty ids; every nested value valid;
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
}

/// Builder for [`AntennaCalibration`], the only way to construct one outside a decode.
/// `antenna_id`, `feed_id`, `metadata`, `physical_config` and `validity_ranges` are
/// required. `build` is defined in [`crate::artifact`], which owns validation.
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

    /// The artifact as set, unvalidated; fails naming the first required field left unset.
    pub(crate) fn assemble(self) -> Result<AntennaCalibration, ValidationError> {
        let required = ValidationError::MissingField;
        Ok(AntennaCalibration {
            antenna_id: self.antenna_id.ok_or(required("antenna_id"))?,
            feed_id: self.feed_id.ok_or(required("feed_id"))?,
            metadata: self.metadata.ok_or(required("metadata"))?,
            physical_config: self.physical_config.ok_or(required("physical_config"))?,
            correction_surface: self.correction_surface,
            validity_ranges: self.validity_ranges.ok_or(required("validity_ranges"))?,
            calibration_status: self.calibration_status,
            calibration_coverage: self.calibration_coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::fixtures;
    use crate::types::{CalibrationCoverage, MeasurementDensity, ParameterSource};

    #[test]
    fn test_antenna_calibration_builder() {
        let calibration = fixtures::builder().build().unwrap();

        assert_eq!(calibration.antenna_id, "test_antenna");
        assert_eq!(calibration.feed_id, "x_band");
        assert!(calibration.correction_surface.is_none());
        assert!(calibration.physics_is_uncorrected());
    }

    #[test]
    fn test_serialization_round_trip() {
        let original = fixtures::calibrated().build().unwrap();

        let encoded = postcard::to_allocvec(&original).unwrap();
        let decoded: AntennaCalibration = postcard::from_bytes(&encoded).unwrap();

        assert_eq!(original, decoded);
        assert!(decoded.correction_surface.is_some());
    }

    #[test]
    fn test_serialization_round_trip_json() {
        let original = fixtures::builder().build().unwrap();

        let json = serde_json::to_string(&original).unwrap();
        let decoded: AntennaCalibration = serde_json::from_str(&json).unwrap();

        assert_eq!(original, decoded);
    }

    #[test]
    fn test_serialization_without_optional_records() {
        let calibration = fixtures::builder().build().unwrap();
        assert!(calibration.calibration_status.is_none());
        assert!(calibration.calibration_coverage.is_none());

        let encoded = postcard::to_allocvec(&calibration).unwrap();
        let decoded: AntennaCalibration = postcard::from_bytes(&encoded).unwrap();

        assert_eq!(calibration, decoded);
    }

    #[test]
    fn test_antenna_calibration_with_partial_calibration_fields() {
        let metadata = CalibrationMetadata {
            parameters_source: Some(ParameterSource::BoresightTuning {
                num_measurements: 28,
            }),
            measurement_density: Some(MeasurementDensity::BoresightOnly),
            ..fixtures::metadata()
        };
        let coverage = CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, false);

        let calibration = fixtures::builder()
            .metadata(metadata)
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
        assert_eq!(calibration.coverage(), Ok(Some(&coverage)));
    }

    /// Guards the single coverage claim on a value mutated after construction (#97).
    #[test]
    fn coverage_refuses_disagreeing_duplicate_records() {
        let mut calibration = fixtures::builder()
            .partially_calibrated(
                1.5,
                CalibrationCoverage::boresight_cone((7_100.0, 8_500.0), 28, false),
            )
            .build()
            .unwrap();
        calibration.calibration_coverage = None;

        assert_eq!(
            calibration.coverage(),
            Err(ValidationError::CoverageRecordsDisagree)
        );
    }
}
