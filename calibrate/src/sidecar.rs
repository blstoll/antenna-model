//! Optional JSON sidecar exporters (`--metadata` / `--report`)
//!
//! Full calibration mode writes one service-loadable binary artifact (an
//! `AntennaCalibration` built by [`crate::artifact_export`], framed by
//! `crate::main::write_antc_artifact`). This module holds the *optional*
//! human-readable JSON files written alongside it: provenance
//! ([`ArtifactMetadata`]) and the validation report.
//!
//! History: this file was `serializer.rs` and additionally defined a
//! `CalibrationArtifact` type with a `save_artifact`/`load_artifact` binary
//! path — an ANTC-framed bincode blob of a *3D* correction surface that the
//! service could never load. The binary path was removed on the
//! bincode → postcard migration (2026-07-18); the `CalibrationArtifact`
//! wrapper that outlived it was removed by roadmap unit D1 (2026-07-29), since
//! neither exporter ever read its `antenna_config` or `correction_surface`
//! fields. Nothing here writes a binary artifact.

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

    #[error("Invalid artifact: {reason}")]
    InvalidArtifact { reason: String },
}

pub type Result<T> = std::result::Result<T, SerializationError>;

// ============================================================================
// Data Structures
// ============================================================================

/// Provenance for a calibration run, written by `--metadata`.
///
/// This is inspection output only — it is not part of the binary artifact the
/// service loads (that carries its own `CalibrationMetadata`).
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

// ============================================================================
// JSON Sidecar Exporters
// ============================================================================

/// Export calibration metadata to JSON for inspection (`--metadata`).
pub fn export_metadata_json<P: AsRef<Path>>(metadata: &ArtifactMetadata, path: P) -> Result<()> {
    write_json(metadata, path)
}

/// Export the validation report to JSON (`--report`).
pub fn export_validation_json<P: AsRef<Path>>(report: &ValidationReport, path: P) -> Result<()> {
    write_json(report, path)
}

fn write_json<T: Serialize, P: AsRef<Path>>(value: &T, path: P) -> Result<()> {
    let json =
        serde_json::to_string_pretty(value).map_err(|e| SerializationError::InvalidArtifact {
            reason: format!("JSON serialization failed: {}", e),
        })?;

    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata() -> ArtifactMetadata {
        ArtifactMetadata {
            created_at: "2026-07-29T00:00:00Z".to_string(),
            measurement_source: "test_measurements.csv".to_string(),
            parameters_tuned: true,
            num_measurement_points: 10,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            notes: Some("calibrated with class: TestClass".to_string()),
            frequency_range: (2000.0, 8000.0),
            angular_range: (0.0, 5.0),
        }
    }

    fn test_validation_report() -> ValidationReport {
        ValidationReport {
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
        }
    }

    #[test]
    fn test_export_metadata_json_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("metadata.json");

        export_metadata_json(&test_metadata(), &path).expect("export metadata");

        let text = std::fs::read_to_string(&path).expect("read metadata");
        let parsed: ArtifactMetadata = serde_json::from_str(&text).expect("parse metadata");
        assert_eq!(parsed.measurement_source, "test_measurements.csv");
        assert_eq!(parsed.num_measurement_points, 10);
        assert!(parsed.parameters_tuned);
        assert_eq!(parsed.frequency_range, (2000.0, 8000.0));
    }

    #[test]
    fn test_export_validation_json_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.json");

        export_validation_json(&test_validation_report(), &path).expect("export report");

        let text = std::fs::read_to_string(&path).expect("read report");
        let parsed: ValidationReport = serde_json::from_str(&text).expect("parse report");
        assert_eq!(parsed.num_points, 10);
        assert!((parsed.corrected_rmse - 0.5).abs() < 1e-12);
        assert!(parsed.meets_accuracy_requirements);
    }
}
