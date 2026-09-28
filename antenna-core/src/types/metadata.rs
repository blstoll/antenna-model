use serde::{Deserialize, Serialize};

use super::{AngularResolution, MeasurementDensity, ParameterSource};

/// Semantic schema version of the calibration payload this build reads and writes,
/// formatted `MAJOR.MINOR`.
///
/// This is the **schema** axis: what a decoded [`super::AntennaCalibration`] means. The
/// **container** axis, [`crate::artifact::ANTC_ARTIFACT_VERSION`], says how file
/// bytes become a payload.
///
/// - **MAJOR** — the field set, field order, or the meaning of an existing field
///   changed. The loader **rejects** a foreign major: postcard is positional, so a
///   mismatched payload can decode "successfully" into wrong values.
/// - **MINOR** — byte layout and every field's meaning intact (documentation, tightened
///   validation). The loader **warns** and loads.
///
/// A layout change bumps MAJOR here **and** the container version. Producers stamp this
/// constant, never a literal. Bump history and procedure:
/// `docs/calibration-workflow-guide.md` §10.5.1.
pub const CALIBRATION_SCHEMA_VERSION: &str = "5.1";

/// Provenance and fit-quality metadata for one calibration artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalibrationMetadata {
    /// Human-readable antenna name.
    pub antenna_name: String,

    /// ISO 8601 timestamp of calibration.
    pub calibration_date: String,

    /// Schema version this artifact was authored against, `MAJOR.MINOR`; see
    /// [`CALIBRATION_SCHEMA_VERSION`].
    pub format_version: String,

    /// Source of measurement data (e.g. S3 path, file name).
    pub data_source: String,

    /// RMSE of the combined model (physics + correction), dB.
    pub rmse_db: f64,

    /// R² of the combined model.
    pub r_squared: f64,

    /// Number of measurement points used in calibration.
    pub num_measurements: usize,

    /// Free-form notes about the calibration.
    pub notes: Option<String>,

    /// RMSE of the physics-only model before correction, dB.
    #[serde(default)]
    pub physics_only_rmse_db: Option<f64>,

    /// RMSE improvement from adding the correction surface, dB.
    #[serde(default)]
    pub correction_improvement_db: Option<f64>,

    /// Whether physical parameters were tuned against measurements.
    #[serde(default)]
    pub parameters_tuned: bool,

    /// Antenna class the shared physical parameters came from, if recorded.
    #[serde(default)]
    pub antenna_class: Option<String>,

    /// How the physical parameters were determined.
    #[serde(default)]
    pub parameters_source: Option<ParameterSource>,

    /// Spatial density of the measurement data.
    #[serde(default)]
    pub measurement_density: Option<MeasurementDensity>,

    /// The [`crate::model::PHYSICS_MODEL_VERSION`] the correction surface was fitted
    /// against; `0` means unknown. The loader warns on a mismatch.
    #[serde(default)]
    pub physics_model_version: u32,

    /// How finely the correction surface can vary in angle against this antenna's
    /// pattern scale. `None` when no angular surface was fitted (boresight mode fits
    /// frequency only). Recorded only; nothing on the served path branches on it.
    #[serde(default)]
    pub angular_resolution: Option<AngularResolution>,
}

impl CalibrationMetadata {
    /// Creates a new builder for constructing `CalibrationMetadata`.
    pub fn builder() -> CalibrationMetadataBuilder {
        CalibrationMetadataBuilder::default()
    }
}

/// Builder for [`CalibrationMetadata`]. `antenna_name`, `calibration_date`,
/// `data_source`, `rmse_db`, `r_squared` and `num_measurements` are required;
/// `format_version` defaults to [`CALIBRATION_SCHEMA_VERSION`] and
/// `physics_model_version` to `0`.
#[derive(Default)]
pub struct CalibrationMetadataBuilder {
    antenna_name: Option<String>,
    calibration_date: Option<String>,
    format_version: Option<String>,
    data_source: Option<String>,
    rmse_db: Option<f64>,
    r_squared: Option<f64>,
    num_measurements: Option<usize>,
    notes: Option<String>,
    physics_only_rmse_db: Option<f64>,
    correction_improvement_db: Option<f64>,
    parameters_tuned: bool,
    antenna_class: Option<String>,
    parameters_source: Option<ParameterSource>,
    measurement_density: Option<MeasurementDensity>,
    physics_model_version: Option<u32>,
    angular_resolution: Option<AngularResolution>,
}

impl CalibrationMetadataBuilder {
    pub fn antenna_name(mut self, name: impl Into<String>) -> Self {
        self.antenna_name = Some(name.into());
        self
    }

    pub fn calibration_date(mut self, date: impl Into<String>) -> Self {
        self.calibration_date = Some(date.into());
        self
    }

    pub fn format_version(mut self, version: impl Into<String>) -> Self {
        self.format_version = Some(version.into());
        self
    }

    pub fn data_source(mut self, source: impl Into<String>) -> Self {
        self.data_source = Some(source.into());
        self
    }

    pub fn rmse_db(mut self, rmse: f64) -> Self {
        self.rmse_db = Some(rmse);
        self
    }

    pub fn r_squared(mut self, r2: f64) -> Self {
        self.r_squared = Some(r2);
        self
    }

    pub fn num_measurements(mut self, num: usize) -> Self {
        self.num_measurements = Some(num);
        self
    }

    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    pub fn physics_only_rmse_db(mut self, rmse: f64) -> Self {
        self.physics_only_rmse_db = Some(rmse);
        self
    }

    pub fn correction_improvement_db(mut self, improvement: f64) -> Self {
        self.correction_improvement_db = Some(improvement);
        self
    }

    pub fn parameters_tuned(mut self, tuned: bool) -> Self {
        self.parameters_tuned = tuned;
        self
    }

    pub fn antenna_class(mut self, class: impl Into<String>) -> Self {
        self.antenna_class = Some(class.into());
        self
    }

    pub fn parameters_source(mut self, source: ParameterSource) -> Self {
        self.parameters_source = Some(source);
        self
    }

    pub fn measurement_density(mut self, density: MeasurementDensity) -> Self {
        self.measurement_density = Some(density);
        self
    }

    pub fn physics_model_version(mut self, version: u32) -> Self {
        self.physics_model_version = Some(version);
        self
    }

    /// Record the fitted surface's angular resolution. Leave unset when no angular
    /// surface was fitted.
    pub fn angular_resolution(mut self, resolution: AngularResolution) -> Self {
        self.angular_resolution = Some(resolution);
        self
    }

    pub fn build(self) -> Result<CalibrationMetadata, String> {
        Ok(CalibrationMetadata {
            antenna_name: self.antenna_name.ok_or("antenna_name is required")?,
            calibration_date: self
                .calibration_date
                .ok_or("calibration_date is required")?,
            format_version: self
                .format_version
                .unwrap_or_else(|| CALIBRATION_SCHEMA_VERSION.to_string()),
            data_source: self.data_source.ok_or("data_source is required")?,
            rmse_db: self.rmse_db.ok_or("rmse_db is required")?,
            r_squared: self.r_squared.ok_or("r_squared is required")?,
            num_measurements: self
                .num_measurements
                .ok_or("num_measurements is required")?,
            notes: self.notes,
            physics_only_rmse_db: self.physics_only_rmse_db,
            correction_improvement_db: self.correction_improvement_db,
            parameters_tuned: self.parameters_tuned,
            antenna_class: self.antenna_class,
            parameters_source: self.parameters_source,
            measurement_density: self.measurement_density,
            physics_model_version: self.physics_model_version.unwrap_or(0),
            angular_resolution: self.angular_resolution,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibration_metadata_builder() {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .notes("Test calibration")
            .build()
            .unwrap();

        assert_eq!(metadata.antenna_name, "Test Antenna");
        assert_eq!(metadata.rmse_db, 0.5);
        assert_eq!(metadata.r_squared, 0.98);
        assert_eq!(metadata.num_measurements, 1000);
        assert_eq!(metadata.notes, Some("Test calibration".to_string()));
        assert_eq!(metadata.format_version, CALIBRATION_SCHEMA_VERSION);
    }
}
