//! Calibration Artifact Serialization Module
//!
//! DEPRECATED: This module serializes the legacy `CalibrationArtifact` (3D
//! `CorrectionSurface`), which the antenna-model service **cannot** load. Full
//! calibration mode now emits a service-loadable `AntennaCalibration` via
//! `crate::artifact_export`. This module is retained only for the optional
//! `--metadata`/`--report` JSON sidecars and existing tests.
//!
//! This module handles serialization and deserialization of calibration artifacts
//! for deployment. Artifacts contain:
//! - Antenna configuration (class reference + tuned parameters)
//! - Correction surface (B-spline coefficients and knots)
//! - Metadata (calibration date, quality metrics, provenance)
//! - Version header and integrity checksums
//!
//! # Binary Format
//!
//! The artifact format is:
//! ```text
//! [Magic Number: 4 bytes: "ANTC"]
//! [Version: u32]
//! [CRC32 Checksum: u32]
//! [Data Length: u64]
//! [Serialized CalibrationArtifact using bincode]
//! ```
//!
//! # Example
//!
//! ```ignore
//! use calibrate::serializer::{CalibrationArtifact, save_artifact, load_artifact};
//! use calibrate::antenna_config::AntennaConfiguration;
//! use calibrate::correction_surface::CorrectionSurface;
//! use calibrate::validator::ValidationReport;
//!
//! let config = /* ... */;
//! let correction = /* ... */;
//! let validation = /* ... */;
//!
//! let artifact = CalibrationArtifact::new(
//!     config,
//!     correction,
//!     validation,
//!     "measurements/antenna_1.csv".to_string(),
//! );
//!
//! save_artifact(&artifact, "calibration_data/antenna_1.bin")?;
//! let loaded = load_artifact("calibration_data/antenna_1.bin")?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::antenna_config::AntennaConfiguration;
use crate::correction_surface::CorrectionSurface;
use crate::validator::ValidationReport;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use thiserror::Error;
use tracing::{debug, info};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum SerializationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Invalid artifact: {reason}")]
    InvalidArtifact { reason: String },

    #[error("Version mismatch: file version {file_version}, expected {expected_version}")]
    VersionMismatch {
        file_version: u32,
        expected_version: u32,
    },

    #[error("Checksum mismatch: expected {expected:#x}, got {actual:#x}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    #[error("Invalid magic number: expected 'ANTC', got {actual:?}")]
    InvalidMagicNumber { actual: Vec<u8> },
}

pub type Result<T> = std::result::Result<T, SerializationError>;

// ============================================================================
// Constants
// ============================================================================

/// Magic number for antenna calibration files: "ANTC"
const MAGIC_NUMBER: &[u8; 4] = b"ANTC";

/// Current artifact format version
const ARTIFACT_VERSION: u32 = 1;

// ============================================================================
// Data Structures
// ============================================================================

/// Complete calibration artifact for deployment
///
/// This contains everything needed to use a calibrated antenna model in production:
/// - Antenna configuration (geometry + tuned parameters)
/// - Correction surface (B-spline coefficients)
/// - Validation metrics (quality assessment)
/// - Metadata (provenance, timestamps, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationArtifact {
    /// Antenna configuration (class + tunable parameters)
    pub antenna_config: AntennaConfiguration,

    /// Correction surface (B-spline model for residuals)
    pub correction_surface: CorrectionSurface,

    /// Validation report (quality metrics)
    pub validation_report: ValidationReport,

    /// Metadata
    pub metadata: ArtifactMetadata,
}

/// Metadata about the calibration artifact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    /// Creation timestamp (ISO 8601 format)
    pub created_at: String,

    /// Measurement data source (file path or S3 URL)
    pub measurement_source: String,

    /// Was parameter tuning performed?
    pub parameters_tuned: bool,

    /// Number of measurement points used for calibration
    pub num_measurement_points: usize,

    /// Calibration tool version
    pub tool_version: String,

    /// Additional notes
    pub notes: Option<String>,

    /// Frequency range covered (MHz)
    pub frequency_range: (f64, f64),

    /// Angular range covered (E-cone degrees)
    pub angular_range: (f64, f64),
}

impl CalibrationArtifact {
    /// Create a new calibration artifact
    pub fn new(
        antenna_config: AntennaConfiguration,
        correction_surface: CorrectionSurface,
        validation_report: ValidationReport,
        measurement_source: String,
    ) -> Self {
        // Extract frequency and angular ranges from validation report
        let frequency_range = extract_frequency_range(&validation_report);
        let angular_range = extract_angular_range(&validation_report);

        let metadata = ArtifactMetadata {
            created_at: chrono::Utc::now().to_rfc3339(),
            measurement_source,
            parameters_tuned: antenna_config.metadata.parameters_tuned,
            num_measurement_points: validation_report.num_points,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            notes: antenna_config.metadata.notes.clone(),
            frequency_range,
            angular_range,
        };

        Self {
            antenna_config,
            correction_surface,
            validation_report,
            metadata,
        }
    }

    /// Get a summary of the artifact
    pub fn summary(&self) -> String {
        format!(
            "Antenna: {} ({})\nPoints: {}\nRMSE: {:.3} dB\nMain lobe max error: {:.3} dB\nMeets requirements: {}",
            self.antenna_config.antenna_id,
            self.antenna_config.class_id,
            self.metadata.num_measurement_points,
            self.validation_report.corrected_rmse,
            self.validation_report.main_lobe_max_error,
            if self.validation_report.meets_accuracy_requirements { "YES" } else { "NO" }
        )
    }
}

// ============================================================================
// Serialization Functions
// ============================================================================

/// Save a calibration artifact to a binary file
///
/// The file format includes:
/// - Magic number ("ANTC")
/// - Version number
/// - CRC32 checksum
/// - Data length
/// - Serialized artifact (bincode format)
///
/// # Arguments
/// * `artifact` - The calibration artifact to save
/// * `path` - Output file path
pub fn save_artifact<P: AsRef<Path>>(artifact: &CalibrationArtifact, path: P) -> Result<()> {
    let path = path.as_ref();
    info!("Saving calibration artifact to: {}", path.display());

    // Serialize the artifact using bincode serde compat
    let config = bincode::config::standard();
    let data = bincode::serde::encode_to_vec(artifact, config)
        .map_err(|e| SerializationError::Serialization(e.to_string()))?;
    let data_len = data.len() as u64;

    // Compute CRC32 checksum
    let checksum = crc32fast::hash(&data);

    debug!(
        "Serialized artifact: {} bytes, checksum: {:#x}",
        data_len, checksum
    );

    // Write to file
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    // Write header
    writer.write_all(MAGIC_NUMBER)?;
    writer.write_all(&ARTIFACT_VERSION.to_le_bytes())?;
    writer.write_all(&checksum.to_le_bytes())?;
    writer.write_all(&data_len.to_le_bytes())?;

    // Write data
    writer.write_all(&data)?;
    writer.flush()?;

    info!("Successfully saved artifact ({} bytes)", data_len);
    Ok(())
}

/// Load a calibration artifact from a binary file
///
/// This function validates:
/// - Magic number
/// - Version compatibility
/// - CRC32 checksum
///
/// # Arguments
/// * `path` - Input file path
///
/// # Returns
/// The deserialized calibration artifact
pub fn load_artifact<P: AsRef<Path>>(path: P) -> Result<CalibrationArtifact> {
    let path = path.as_ref();
    info!("Loading calibration artifact from: {}", path.display());

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    // Read and validate magic number
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC_NUMBER {
        return Err(SerializationError::InvalidMagicNumber {
            actual: magic.to_vec(),
        });
    }

    // Read version
    let mut version_bytes = [0u8; 4];
    reader.read_exact(&mut version_bytes)?;
    let version = u32::from_le_bytes(version_bytes);

    if version != ARTIFACT_VERSION {
        return Err(SerializationError::VersionMismatch {
            file_version: version,
            expected_version: ARTIFACT_VERSION,
        });
    }

    // Read checksum
    let mut checksum_bytes = [0u8; 4];
    reader.read_exact(&mut checksum_bytes)?;
    let expected_checksum = u32::from_le_bytes(checksum_bytes);

    // Read data length
    let mut len_bytes = [0u8; 8];
    reader.read_exact(&mut len_bytes)?;
    let data_len = u64::from_le_bytes(len_bytes);

    debug!(
        "Loading {} bytes, expected checksum: {:#x}",
        data_len, expected_checksum
    );

    // Read data
    let mut data = vec![0u8; data_len as usize];
    reader.read_exact(&mut data)?;

    // Verify checksum
    let actual_checksum = crc32fast::hash(&data);
    if actual_checksum != expected_checksum {
        return Err(SerializationError::ChecksumMismatch {
            expected: expected_checksum,
            actual: actual_checksum,
        });
    }

    // Deserialize using bincode serde compat
    let config = bincode::config::standard();
    let artifact: CalibrationArtifact = bincode::serde::decode_from_slice(&data, config)
        .map_err(|e| SerializationError::Serialization(e.to_string()))?
        .0;

    info!(
        "Successfully loaded artifact for antenna: {}",
        artifact.antenna_config.antenna_id
    );

    Ok(artifact)
}

/// Export artifact metadata to JSON for inspection
pub fn export_metadata_json<P: AsRef<Path>>(artifact: &CalibrationArtifact, path: P) -> Result<()> {
    let json = serde_json::to_string_pretty(&artifact.metadata).map_err(|e| {
        SerializationError::InvalidArtifact {
            reason: format!("JSON serialization failed: {}", e),
        }
    })?;

    std::fs::write(path, json)?;
    Ok(())
}

/// Export validation report to JSON
pub fn export_validation_json<P: AsRef<Path>>(
    artifact: &CalibrationArtifact,
    path: P,
) -> Result<()> {
    let json = serde_json::to_string_pretty(&artifact.validation_report).map_err(|e| {
        SerializationError::InvalidArtifact {
            reason: format!("JSON serialization failed: {}", e),
        }
    })?;

    std::fs::write(path, json)?;
    Ok(())
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract frequency range from validation report
fn extract_frequency_range(report: &ValidationReport) -> (f64, f64) {
    if report.frequency_band_analysis.is_empty() {
        return (0.0, 0.0);
    }

    let min = report
        .frequency_band_analysis
        .iter()
        .map(|b| b.band_min_mhz)
        .fold(f64::INFINITY, f64::min);

    let max = report
        .frequency_band_analysis
        .iter()
        .map(|b| b.band_max_mhz)
        .fold(f64::NEG_INFINITY, f64::max);

    (min, max)
}

/// Extract angular range from validation report
fn extract_angular_range(report: &ValidationReport) -> (f64, f64) {
    if report.angular_region_analysis.is_empty() {
        return (0.0, 0.0);
    }

    let min = report
        .angular_region_analysis
        .iter()
        .map(|r| r.cone_min_deg)
        .fold(f64::INFINITY, f64::min);

    let max = report
        .angular_region_analysis
        .iter()
        .map(|r| r.cone_max_deg)
        .fold(f64::NEG_INFINITY, f64::max);

    (min, max)
}

// ============================================================================
// File Format Information
// ============================================================================

/// Get information about the artifact file format
pub fn artifact_format_info() -> String {
    format!(
        "Antenna Calibration Artifact Format\n\
         ====================================\n\
         Magic Number: {:?}\n\
         Version: {}\n\
         Encoding: bincode (binary)\n\
         Integrity: CRC32 checksum\n\
         \n\
         File Structure:\n\
         - Magic number (4 bytes)\n\
         - Version (4 bytes, little-endian u32)\n\
         - CRC32 checksum (4 bytes, little-endian u32)\n\
         - Data length (8 bytes, little-endian u64)\n\
         - Serialized data (bincode format)\n\
         \n\
         Contents:\n\
         - Antenna configuration (geometry + tuned parameters)\n\
         - Correction surface (3D B-spline coefficients)\n\
         - Validation metrics (RMSE, R², max error, etc.)\n\
         - Metadata (timestamps, provenance, quality flags)\n",
        std::str::from_utf8(MAGIC_NUMBER).unwrap_or("<invalid UTF-8>"),
        ARTIFACT_VERSION
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    // Helper function to create a minimal test artifact
    fn create_test_artifact() -> CalibrationArtifact {
        use crate::antenna_config::{AntennaConfiguration, AntennaMetadata, TunableParameters};
        use crate::correction_surface::{CorrectionSurface, FitStatistics};
        use crate::validator::ValidationReport;

        let antenna_config = AntennaConfiguration {
            antenna_id: "test_antenna".to_string(),
            name: "Test Antenna".to_string(),
            class_id: "TestClass".to_string(),
            tunable_parameters: TunableParameters::default_from_class(),
            metadata: AntennaMetadata::default(),
        };

        let correction_surface = CorrectionSurface {
            coefficients: vec![0.0; 10],
            shape: [2, 2, 2],
            knots_frequency: vec![0.0, 1.0],
            knots_econe: vec![0.0, 1.0],
            knots_eclock: vec![0.0, 1.0],
            spline_order: 4,
            fit_stats: FitStatistics {
                num_points: 10,
                rmse_db: 0.5,
                max_residual_db: 1.0,
                r_squared: 0.95,
                cross_validation_rmse: None,
                improvement_percent: 50.0,
            },
        };

        let validation_report = ValidationReport {
            num_points: 10,
            model_only_rmse: 1.0,
            model_only_max_error: 2.0,
            model_only_r_squared: 0.8,
            corrected_rmse: 0.5,
            corrected_max_error: 1.0,
            corrected_r_squared: 0.95,
            rmse_improvement_percent: 50.0,
            max_error_improvement_percent: 50.0,
            main_lobe_num_points: 5,
            main_lobe_max_error: 0.8,
            main_lobe_rmse: 0.4,
            main_lobe_meets_target: true,
            first_sidelobe_num_points: 3,
            first_sidelobe_max_error: 0.9,
            first_sidelobe_rmse: 0.5,
            first_sidelobe_meets_target: true,
            outliers: vec![],
            num_outliers: 0,
            frequency_band_analysis: vec![],
            angular_region_analysis: vec![],
            cross_validation: None,
            meets_accuracy_requirements: true,
        };

        CalibrationArtifact::new(
            antenna_config,
            correction_surface,
            validation_report,
            "test_measurements.csv".to_string(),
        )
    }

    #[test]
    fn test_save_and_load_artifact() {
        let artifact = create_test_artifact();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Save
        save_artifact(&artifact, path).expect("Failed to save artifact");

        // Load
        let loaded = load_artifact(path).expect("Failed to load artifact");

        // Verify
        assert_eq!(
            loaded.antenna_config.antenna_id,
            artifact.antenna_config.antenna_id
        );
        assert_eq!(
            loaded.metadata.num_measurement_points,
            artifact.metadata.num_measurement_points
        );
        assert_eq!(
            loaded.correction_surface.coefficients.len(),
            artifact.correction_surface.coefficients.len()
        );
    }

    #[test]
    fn test_checksum_validation() {
        let artifact = create_test_artifact();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Save
        save_artifact(&artifact, path).expect("Failed to save artifact");

        // Corrupt the file by modifying a byte in the data section
        let mut file_data = std::fs::read(path).unwrap();
        if file_data.len() > 20 {
            file_data[20] ^= 0xFF; // Flip bits
        }
        std::fs::write(path, file_data).unwrap();

        // Try to load - should fail with checksum error
        let result = load_artifact(path);
        assert!(matches!(
            result,
            Err(SerializationError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn test_invalid_magic_number() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Write invalid magic number
        let mut file = File::create(path).unwrap();
        file.write_all(b"XXXX").unwrap();
        file.write_all(&[0u8; 16]).unwrap(); // padding

        // Try to load - should fail with invalid magic number
        let result = load_artifact(path);
        assert!(matches!(
            result,
            Err(SerializationError::InvalidMagicNumber { .. })
        ));
    }

    #[test]
    fn test_version_mismatch() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Write valid magic but wrong version
        let mut file = File::create(path).unwrap();
        file.write_all(MAGIC_NUMBER).unwrap();
        file.write_all(&999u32.to_le_bytes()).unwrap(); // Wrong version
        file.write_all(&[0u8; 12]).unwrap(); // padding

        // Try to load - should fail with version mismatch
        let result = load_artifact(path);
        assert!(matches!(
            result,
            Err(SerializationError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn test_artifact_summary() {
        let artifact = create_test_artifact();
        let summary = artifact.summary();
        assert!(summary.contains("test_antenna"));
        assert!(summary.contains("TestClass"));
        assert!(summary.contains("YES")); // meets requirements
    }

    #[test]
    fn test_format_info() {
        let info = artifact_format_info();
        assert!(info.contains("ANTC"));
        assert!(info.contains("bincode"));
        assert!(info.contains("CRC32"));
    }
}
