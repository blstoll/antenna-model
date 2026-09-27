use serde::{Deserialize, Serialize};

use super::CalibrationCoverage;

/// Level of calibration data behind an artifact, with the accuracy it supports.
///
/// Whether the served gain is corrected is decided by the presence of a correction
/// surface ([`super::AntennaCalibration::physics_is_uncorrected`]), not by this status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CalibrationStatus {
    /// Dense measurement grid across azimuth, elevation, and frequency.
    FullyCalibrated {
        /// Expected accuracy, dB (typically ±1.0 in the main lobe and first sidelobe).
        accuracy_estimate_db: f64,
    },

    /// Limited coverage: boresight-only or a sparse grid. Typically ±1–1.5 dB in
    /// coverage and ±2–3 dB outside it.
    PartiallyCalibrated {
        /// Expected accuracy, dB.
        accuracy_estimate_db: f64,
        /// Measurement coverage. Must equal the artifact's `calibration_coverage`; write
        /// both with [`super::AntennaCalibrationBuilder::partially_calibrated`].
        coverage: CalibrationCoverage,
    },

    /// Design specifications only, no measurements.
    Uncalibrated {
        /// Expected absolute gain accuracy, dB (typically ±3.0).
        accuracy_estimate_db: f64,
        /// Expected relative-gain (loss) accuracy, dB (typically ±2.0) — better than
        /// absolute because systematic error cancels in the subtraction.
        loss_accuracy_estimate_db: f64,
    },
}

impl CalibrationStatus {
    /// Returns the accuracy estimate in dB for this calibration status.
    pub fn accuracy_estimate_db(&self) -> f64 {
        match self {
            CalibrationStatus::FullyCalibrated {
                accuracy_estimate_db,
            } => *accuracy_estimate_db,
            CalibrationStatus::PartiallyCalibrated {
                accuracy_estimate_db,
                ..
            } => *accuracy_estimate_db,
            CalibrationStatus::Uncalibrated {
                accuracy_estimate_db,
                ..
            } => *accuracy_estimate_db,
        }
    }

    /// Whether this status claims a fitted correction surface.
    pub fn has_correction_surface(&self) -> bool {
        match self {
            CalibrationStatus::FullyCalibrated { .. } => true,
            CalibrationStatus::PartiallyCalibrated { coverage, .. } => {
                coverage.has_correction_surface
            }
            CalibrationStatus::Uncalibrated { .. } => false,
        }
    }

    /// The status's wire name: `fully_calibrated`, `partially_calibrated` or
    /// `uncalibrated`.
    pub fn status_string(&self) -> &str {
        match self {
            CalibrationStatus::FullyCalibrated { .. } => "fully_calibrated",
            CalibrationStatus::PartiallyCalibrated { .. } => "partially_calibrated",
            CalibrationStatus::Uncalibrated { .. } => "uncalibrated",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibration_status_fully_calibrated() {
        let status = CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        };

        assert_eq!(status.accuracy_estimate_db(), 1.0);
        assert!(status.has_correction_surface());
        assert_eq!(status.status_string(), "fully_calibrated");
    }

    #[test]
    fn test_calibration_status_partially_calibrated() {
        let coverage = CalibrationCoverage {
            azimuth_range: (0.0, 0.0),
            elevation_range: (0.0, 0.0),
            frequency_range: (7100.0, 8500.0),
            num_measurements: 28,
            has_correction_surface: true,
        };

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage,
        };

        assert_eq!(status.accuracy_estimate_db(), 1.5);
        assert!(status.has_correction_surface());
        assert_eq!(status.status_string(), "partially_calibrated");
    }

    #[test]
    fn test_calibration_status_uncalibrated() {
        let status = CalibrationStatus::Uncalibrated {
            accuracy_estimate_db: 3.0,
            loss_accuracy_estimate_db: 2.0,
        };

        assert_eq!(status.accuracy_estimate_db(), 3.0);
        assert!(!status.has_correction_surface());
        assert_eq!(status.status_string(), "uncalibrated");
    }

    #[test]
    fn test_partial_calibration_serialization_round_trip() {
        let coverage = CalibrationCoverage::builder()
            .azimuth_range(0.0, 0.0)
            .elevation_range(0.0, 0.0)
            .frequency_range(7100.0, 8500.0)
            .num_measurements(28)
            .has_correction_surface(true)
            .build()
            .unwrap();

        let status = CalibrationStatus::PartiallyCalibrated {
            accuracy_estimate_db: 1.5,
            coverage: coverage.clone(),
        };

        // Test postcard serialization
        let encoded = postcard::to_allocvec(&status).unwrap();
        let decoded: CalibrationStatus = postcard::from_bytes(&encoded).unwrap();

        assert_eq!(status, decoded);

        // Test JSON serialization
        let json = serde_json::to_string(&status).unwrap();
        let decoded_json: CalibrationStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded_json);
    }
}
