//! Calibration artifact loader
//!
//! This module provides functionality for loading and validating calibration artifacts
//! from binary files.

use crate::data::types::AntennaCalibration;
use crate::error::DataError;
use bincode::config;
use std::path::Path;
use tracing::{debug, info, warn};

/// Load a calibration artifact from a binary file
///
/// Deserializes and validates a calibration artifact from a .bin file.
///
/// # Arguments
/// * `path` - Path to the calibration binary file
///
/// # Returns
/// * `Ok(AntennaCalibration)` - Successfully loaded and validated calibration
/// * `Err(DataError)` - Failed to load or validate
///
/// # Example
/// ```no_run
/// use antenna_model::data::loader::load_calibration_artifact;
///
/// let calibration = load_calibration_artifact("calibration_data/antenna_1.bin")?;
/// println!("Loaded antenna: {}, feed: {}", calibration.antenna_id, calibration.feed_id);
/// # Ok::<(), antenna_model::error::DataError>(())
/// ```
pub fn load_calibration_artifact<P: AsRef<Path>>(path: P) -> Result<AntennaCalibration, DataError> {
    let path = path.as_ref();

    debug!("Loading calibration artifact from: {}", path.display());

    // Read the binary file
    let bytes = std::fs::read(path).map_err(|e| DataError::LoadError {
        path: path.display().to_string(),
        reason: format!("Failed to read file: {}", e),
    })?;

    // Deserialize using bincode
    let config = config::standard();
    let (calibration, _): (AntennaCalibration, usize) =
        bincode::decode_from_slice(&bytes, config).map_err(|e| DataError::LoadError {
            path: path.display().to_string(),
            reason: format!("Failed to deserialize calibration data: {}", e),
        })?;

    // Validate the calibration
    calibration.validate().map_err(|e| DataError::ValidationError {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;

    // Log summary
    info!(
        "Loaded calibration: antenna_id={}, feed_id={}, format_version={}",
        calibration.antenna_id, calibration.feed_id, calibration.metadata.format_version
    );

    // Log physical parameters
    debug!(
        "Physical config: diameter={:.1}m, f/D={:.2}, surface_rms={:.2}mm",
        calibration.physical_config.reflector.diameter_m,
        calibration.physical_config.reflector.f_over_d_ratio,
        calibration.physical_config.reflector.surface_rms_mm
    );

    // Log correction surface presence
    if let Some(ref correction) = calibration.correction_surface {
        debug!(
            "Correction surface: shape={:?}, {} coefficients",
            correction.shape,
            correction.num_coefficients()
        );
    } else {
        debug!("No correction surface (physics-only model)");
    }

    // Log validity ranges
    debug!(
        "Validity ranges: az=[{:.1}, {:.1}]°, el=[{:.1}, {:.1}]°, freq=[{:.1}, {:.1}] MHz",
        calibration.validity_ranges.azimuth_min_max.0,
        calibration.validity_ranges.azimuth_min_max.1,
        calibration.validity_ranges.elevation_min_max.0,
        calibration.validity_ranges.elevation_min_max.1,
        calibration.validity_ranges.frequency_min_max.0,
        calibration.validity_ranges.frequency_min_max.1
    );

    // Warn about old format versions
    if calibration.metadata.format_version != "2.0" {
        warn!(
            "Calibration format version {} may be outdated (expected 2.0)",
            calibration.metadata.format_version
        );
    }

    Ok(calibration)
}

/// Validate a calibration artifact's internal consistency
///
/// Performs deep validation beyond the basic checks in `AntennaCalibration::validate()`.
///
/// # Arguments
/// * `calibration` - The calibration to validate
///
/// # Returns
/// * `Ok(())` - Calibration is valid
/// * `Err(DataError)` - Validation failed
pub fn validate_calibration(calibration: &AntennaCalibration) -> Result<(), DataError> {
    // Basic validation (already done in load, but can be called separately)
    calibration.validate().map_err(|e| DataError::ValidationError {
        path: format!("{}:{}", calibration.antenna_id, calibration.feed_id),
        reason: e.to_string(),
    })?;

    // Additional validation checks

    // Check that validity ranges are reasonable
    let freq_range = calibration.validity_ranges.frequency_min_max;
    if freq_range.0 < 100.0 || freq_range.1 > 50000.0 {
        warn!(
            "Frequency range [{:.1}, {:.1}] MHz is outside typical range [100, 50000] MHz",
            freq_range.0, freq_range.1
        );
    }

    // Check elevation range is physically reasonable
    let el_range = calibration.validity_ranges.elevation_min_max;
    if el_range.0 < 0.0 || el_range.1 > 90.0 {
        return Err(DataError::ValidationError {
            path: format!("{}:{}", calibration.antenna_id, calibration.feed_id),
            reason: format!(
                "Elevation range [{:.1}, {:.1}]° is outside physical bounds [0, 90]°",
                el_range.0, el_range.1
            ),
        });
    }

    // Check mesh parameters if present
    if let Some(ref mesh) = calibration.physical_config.mesh {
        if mesh.wire_diameter_mm >= mesh.mesh_spacing_mm {
            return Err(DataError::ValidationError {
                path: format!("{}:{}", calibration.antenna_id, calibration.feed_id),
                reason: format!(
                    "Wire diameter ({:.2} mm) must be less than mesh spacing ({:.2} mm)",
                    mesh.wire_diameter_mm, mesh.mesh_spacing_mm
                ),
            });
        }
    }

    // Check correction surface dimensions if present
    if let Some(ref correction) = calibration.correction_surface {
        let total_coeffs = correction.num_coefficients();
        if total_coeffs > 1_000_000 {
            warn!(
                "Correction surface has {} coefficients, which may impact performance",
                total_coeffs
            );
        }
    }

    // Check metadata quality metrics
    if calibration.metadata.rmse_db > 1.0 {
        warn!(
            "Calibration RMSE ({:.2} dB) exceeds 1 dB accuracy target",
            calibration.metadata.rmse_db
        );
    }

    if calibration.metadata.r_squared < 0.95 {
        warn!(
            "Calibration R² ({:.3}) is below 0.95, indicating poor fit quality",
            calibration.metadata.r_squared
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::types::{
        BSplineModel4D, CalibrationMetadata, FeedParameters, PhysicalAntennaConfig,
        ReflectorGeometry, ValidityRanges,
    };
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_calibration() -> AntennaCalibration {
        let metadata = CalibrationMetadata::builder()
            .antenna_name("Test Antenna")
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

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

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(10.0, 80.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        AntennaCalibration::builder()
            .antenna_id("test_antenna")
            .feed_id("x_band")
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap()
    }

    #[test]
    fn test_load_calibration_artifact_success() {
        let calibration = create_test_calibration();

        // Serialize to a temporary file
        let mut temp_file = NamedTempFile::new().unwrap();
        let config = config::standard();
        let encoded = bincode::encode_to_vec(&calibration, config).unwrap();
        temp_file.write_all(&encoded).unwrap();
        temp_file.flush().unwrap();

        // Load it back
        let loaded = load_calibration_artifact(temp_file.path()).unwrap();

        assert_eq!(loaded.antenna_id, "test_antenna");
        assert_eq!(loaded.feed_id, "x_band");
        assert_eq!(loaded.metadata.antenna_name, "Test Antenna");
    }

    #[test]
    fn test_load_calibration_artifact_file_not_found() {
        let result = load_calibration_artifact("/nonexistent/path/to/file.bin");
        assert!(result.is_err());
        match result {
            Err(DataError::LoadError { path, .. }) => {
                assert!(path.contains("nonexistent"));
            }
            _ => panic!("Expected LoadError"),
        }
    }

    #[test]
    fn test_load_calibration_artifact_invalid_data() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"invalid binary data").unwrap();
        temp_file.flush().unwrap();

        let result = load_calibration_artifact(temp_file.path());
        assert!(result.is_err());
        match result {
            Err(DataError::LoadError { reason, .. }) => {
                assert!(reason.contains("deserialize"));
            }
            _ => panic!("Expected LoadError with deserialization failure"),
        }
    }

    #[test]
    fn test_validate_calibration_success() {
        let calibration = create_test_calibration();
        assert!(validate_calibration(&calibration).is_ok());
    }

    #[test]
    fn test_validate_calibration_invalid_elevation_range() {
        let mut calibration = create_test_calibration();
        calibration.validity_ranges.elevation_min_max = (-10.0, 100.0);

        let result = validate_calibration(&calibration);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_calibration_with_correction_surface() {
        let mut calibration = create_test_calibration();

        let correction = BSplineModel4D::builder()
            .coefficients(vec![1.0; 24])
            .shape([2, 3, 2, 2])
            .knots_azimuth(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_elevation(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0])
            .knots_frequency(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .knots_temperature(vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .build()
            .unwrap();

        calibration.correction_surface = Some(correction);

        assert!(validate_calibration(&calibration).is_ok());
    }
}
