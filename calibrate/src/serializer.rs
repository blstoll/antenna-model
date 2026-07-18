//! Calibration Artifact Metadata Module
//!
//! Defines the legacy `CalibrationArtifact` (3D `CorrectionSurface`) type and its
//! optional JSON sidecar exporters (`--metadata`/`--report`). Full calibration mode
//! emits a service-loadable `AntennaCalibration` via `crate::artifact_export`; this
//! type is retained only to drive those optional JSON sidecars.
//!
//! The former `save_artifact`/`load_artifact` binary path (an ANTC-framed bincode
//! blob the service could never load) was removed on the bincode → postcard
//! migration (2026-07-18). The service-loadable binary path lives in
//! `crate::main::write_antc_artifact` (postcard payload).

use crate::antenna_config::AntennaConfiguration;
use crate::correction_surface::CorrectionSurface;
use crate::validator::ValidationReport;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

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
// JSON Sidecar Exporters
// ============================================================================

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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_artifact_summary() {
        let artifact = create_test_artifact();
        let summary = artifact.summary();
        assert!(summary.contains("test_antenna"));
        assert!(summary.contains("TestClass"));
        assert!(summary.contains("YES")); // meets requirements
    }
}
