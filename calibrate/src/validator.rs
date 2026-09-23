//! Validation Module
//!
//! This module implements comprehensive validation of calibrated antenna models,
//! including the combined physics model + correction surface approach.
//!
//! # Overview
//!
//! The validator provides:
//! - K-fold cross-validation for robustness assessment
//! - Error metrics (RMSE, max error, R²) for model quality
//! - Before/after comparison (model-only vs served behavior)
//! - Main lobe accuracy verification (<1 dB target)
//! - First sidelobe accuracy verification (<1 dB target)
//! - High-error served-prediction identification (>1 dB error cases)
//! - Error analysis by frequency band and angular region
//!
//! # Example
//!
//! ```ignore
//! use calibrate::validator::{validate_calibration, ValidationConfig};
//! use calibrate::parser::MeasurementPoint;
//! use calibrate::correction_surface::CorrectionSurface;
//!
//! let measurements = vec![/* ... */];
//! let model_predictions = vec![/* ... */];
//! let correction_surface = /* ... */;
//! let config = ValidationConfig::default();
//!
//! let report = validate_calibration(
//!     &measurements,
//!     &model_predictions,
//!     &correction_surface,
//!     &config
//! )?;
//!
//! println!("RMSE (model only): {:.3} dB", report.model_only_rmse);
//! println!("RMSE (served behavior): {:.3} dB", report.served_behavior_rmse);
//! println!("Main lobe max error: {:.3} dB", report.main_lobe_max_error);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::correction_surface::{support_aware_metrics, CorrectionSurface, CorrectionSurfaceError};
use crate::parser::MeasurementPoint;
use antenna_core::model::CorrectionEvaluation;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Insufficient data for validation: need at least {min_required}, got {actual}")]
    InsufficientData { min_required: usize, actual: usize },

    #[error("Dimension mismatch: measurements ({measurements}) != predictions ({predictions})")]
    DimensionMismatch {
        measurements: usize,
        predictions: usize,
    },

    #[error("Cross-validation failed: {reason}")]
    CrossValidationError { reason: String },

    #[error("Invalid parameter: {param} = {value} ({reason})")]
    InvalidParameter {
        param: String,
        value: String,
        reason: String,
    },

    #[error("Correction surface error: {0}")]
    CorrectionSurfaceError(#[from] CorrectionSurfaceError),

    #[error("Computation error: {reason}")]
    ComputationError { reason: String },
}

pub type Result<T> = std::result::Result<T, ValidationError>;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for validation process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Number of folds for k-fold cross-validation (0 to skip)
    pub num_folds: usize,

    /// Main lobe definition: points within this many beamwidths from boresight
    pub main_lobe_beamwidths: f64,

    /// First sidelobe definition: between main_lobe and this angle (degrees)
    pub first_sidelobe_max_deg: f64,

    /// Frequency band boundaries for separate analysis (MHz)
    pub frequency_bands: Vec<(f64, f64)>,

    /// Accuracy target for main lobe (dB)
    pub main_lobe_target_db: f64,

    /// Accuracy target for first sidelobe (dB)
    pub first_sidelobe_target_db: f64,

    /// Outlier threshold (dB) - errors above this are flagged
    pub outlier_threshold_db: f64,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            num_folds: 5,
            main_lobe_beamwidths: 3.0,
            first_sidelobe_max_deg: 10.0,
            frequency_bands: vec![
                (100.0, 1000.0),    // VHF/UHF
                (1000.0, 3000.0),   // L/S band
                (3000.0, 12000.0),  // C/X band
                (12000.0, 50000.0), // Ku/Ka/V band
            ],
            main_lobe_target_db: 1.0,
            first_sidelobe_target_db: 1.0,
            outlier_threshold_db: 1.0,
        }
    }
}

// ============================================================================
// Data Structures
// ============================================================================

/// Complete validation report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Total number of measurement points
    pub num_points: usize,

    /// Model-only performance (no correction)
    pub model_only_rmse: f64,
    pub model_only_max_error: f64,
    pub model_only_r_squared: f64,

    /// Served-behavior RMSE over every validation point.
    ///
    /// Outside fitted support, this includes the physics-only prediction.
    pub served_behavior_rmse: f64,
    /// Correction RMSE over only points where the fitted surface returned `Applied`.
    ///
    /// `None` means no validation point was inside fitted support.
    pub in_support_correction_rmse: Option<f64>,
    /// Validation points served physics-only because they lay outside fitted support.
    pub out_of_support_points: usize,
    /// Proportion of validation points outside fitted support, in `[0, 1]`.
    pub out_of_support_proportion: f64,
    /// Served-behavior maximum error, with physics-only fallback outside fitted support.
    pub corrected_max_error: f64,
    /// Served-behavior coefficient of determination, with physics-only fallback outside support.
    pub corrected_r_squared: f64,

    /// Served-behavior RMSE improvement relative to physics-only predictions.
    pub rmse_improvement_percent: f64,
    /// Served-behavior maximum-error improvement relative to physics-only predictions.
    pub max_error_improvement_percent: f64,

    /// Main lobe statistics
    pub main_lobe_num_points: usize,
    pub main_lobe_max_error: f64,
    pub main_lobe_rmse: f64,
    pub main_lobe_meets_target: bool,

    /// First sidelobe statistics
    pub first_sidelobe_num_points: usize,
    pub first_sidelobe_max_error: f64,
    pub first_sidelobe_rmse: f64,
    pub first_sidelobe_meets_target: bool,

    /// High-error served predictions; this does not classify measurement quality.
    pub outliers: Vec<OutlierPoint>,
    /// Number of high-error served predictions.
    pub num_outliers: usize,

    /// Error analysis by frequency band
    pub frequency_band_analysis: Vec<FrequencyBandStats>,

    /// Error analysis by angular region
    pub angular_region_analysis: Vec<AngularRegionStats>,

    /// Cross-validation results (if performed)
    pub cross_validation: Option<CrossValidationResults>,

    /// Artifact acceptance under the main-lobe and first-sidelobe maximum-error policy.
    ///
    /// Served-behavior and in-support RMSE are diagnostics, not acceptance gates (#96).
    pub meets_accuracy_requirements: bool,
}

/// Information about an outlier point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutlierPoint {
    pub frequency_mhz: f64,
    pub e_cone_deg: f64,
    pub e_clock_deg: f64,
    pub measured_db: f64,
    pub predicted_db: f64,
    pub error_db: f64,
    pub region: String,
}

/// Statistics for a frequency band
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyBandStats {
    pub band_min_mhz: f64,
    pub band_max_mhz: f64,
    pub num_points: usize,
    pub rmse_db: f64,
    pub max_error_db: f64,
    pub mean_error_db: f64,
}

/// Statistics for an angular region
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AngularRegionStats {
    pub region_name: String,
    pub cone_min_deg: f64,
    pub cone_max_deg: f64,
    pub num_points: usize,
    pub rmse_db: f64,
    pub max_error_db: f64,
    pub mean_error_db: f64,
}

pub use crate::correction_surface::{
    CrossValidationFoldResult, CrossValidationResults, FoldFailure,
};

// ============================================================================
// Main Validation Function
// ============================================================================

/// Validate a calibrated antenna model
///
/// This function performs comprehensive validation of a calibrated model,
/// including the physics model and correction surface.
///
/// # Arguments
/// * `measurements` - Original measurement data points
/// * `model_predictions` - Physics model predictions (G/T in dB/K) for each measurement
/// * `correction_surface` - Fitted correction surface
/// * `config` - Validation configuration
///
/// # Returns
/// A comprehensive validation report
pub fn validate_calibration(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
    correction_surface: &CorrectionSurface,
    config: &ValidationConfig,
) -> Result<ValidationReport> {
    info!(
        "Starting validation with {} data points",
        measurements.len()
    );

    // Validate inputs
    if measurements.is_empty() {
        return Err(ValidationError::InsufficientData {
            min_required: 1,
            actual: 0,
        });
    }

    if measurements.len() != model_predictions.len() {
        return Err(ValidationError::DimensionMismatch {
            measurements: measurements.len(),
            predictions: model_predictions.len(),
        });
    }

    let num_points = measurements.len();

    // Compute served predictions while retaining whether each correction was applied.
    // The support-aware scorer below uses this typed outcome so OutsideSupport contributes
    // only to served behavior and never masquerades as an applied 0 dB correction (#96).
    let evaluated_predictions =
        compute_served_predictions(measurements, model_predictions, correction_surface)?;
    let served_predictions: Vec<f64> = evaluated_predictions
        .iter()
        .map(|prediction| prediction.served_db)
        .collect();
    // Extract measured values
    let measured: Vec<f64> = measurements.iter().map(|m| m.g_over_t_db).collect();
    let support_metrics = support_aware_metrics(measured.iter().zip(&evaluated_predictions).map(
        |(measured_db, prediction)| (measured_db - prediction.served_db, prediction.correction),
    ));
    if support_metrics.out_of_support_points > 0 {
        warn!(
            out_of_support_points = support_metrics.out_of_support_points,
            out_of_support_proportion = support_metrics.out_of_support_proportion,
            "validation points outside fitted support use physics-only predictions"
        );
    }

    // Model-only statistics
    let model_only_rmse = compute_rmse(&measured, model_predictions);
    let model_only_max_error = compute_max_error(&measured, model_predictions);
    let model_only_r_squared = compute_r_squared(&measured, model_predictions);

    // Served-behavior and fitted-support statistics
    let served_behavior_rmse = support_metrics.served_behavior_rmse;
    let served_behavior_max_error = compute_max_error(&measured, &served_predictions);
    let served_behavior_r_squared = compute_r_squared(&measured, &served_predictions);

    // Improvement metrics
    let rmse_improvement_percent = if model_only_rmse > 0.0 {
        100.0 * (model_only_rmse - served_behavior_rmse) / model_only_rmse
    } else {
        0.0
    };

    let max_error_improvement_percent = if model_only_max_error > 0.0 {
        100.0 * (model_only_max_error - served_behavior_max_error) / model_only_max_error
    } else {
        0.0
    };

    info!(
        model_only_rmse_db = model_only_rmse,
        served_behavior_rmse_db = served_behavior_rmse,
        rmse_improvement_percent,
        "validation error metrics"
    );

    // Classify points by region
    let (main_lobe_indices, first_sidelobe_indices) =
        classify_points_by_region(measurements, config);

    // Main lobe statistics
    let (main_lobe_rmse, main_lobe_max_error, main_lobe_num_points) =
        compute_region_stats(&measured, &served_predictions, &main_lobe_indices);
    let main_lobe_meets_target = main_lobe_max_error <= config.main_lobe_target_db;

    info!(
        "Main lobe: {} points, max error: {:.3} dB, RMSE: {:.3} dB (target: {:.1} dB, {})",
        main_lobe_num_points,
        main_lobe_max_error,
        main_lobe_rmse,
        config.main_lobe_target_db,
        if main_lobe_meets_target {
            "PASS"
        } else {
            "FAIL"
        }
    );

    // First sidelobe statistics
    let (first_sidelobe_rmse, first_sidelobe_max_error, first_sidelobe_num_points) =
        compute_region_stats(&measured, &served_predictions, &first_sidelobe_indices);
    let first_sidelobe_meets_target = first_sidelobe_max_error <= config.first_sidelobe_target_db;

    info!(
        "First sidelobe: {} points, max error: {:.3} dB, RMSE: {:.3} dB (target: {:.1} dB, {})",
        first_sidelobe_num_points,
        first_sidelobe_max_error,
        first_sidelobe_rmse,
        config.first_sidelobe_target_db,
        if first_sidelobe_meets_target {
            "PASS"
        } else {
            "FAIL"
        }
    );

    // Identify outliers
    let outliers = identify_outliers(
        measurements,
        &served_predictions,
        config.outlier_threshold_db,
        config,
    );
    let num_outliers = outliers.len();

    if num_outliers > 0 {
        warn!(
            "Found {} outliers (error > {:.1} dB)",
            num_outliers, config.outlier_threshold_db
        );
    }

    // Frequency band analysis
    let frequency_band_analysis =
        analyze_by_frequency_band(measurements, &served_predictions, &config.frequency_bands);

    // Angular region analysis
    let angular_region_analysis = analyze_by_angular_region(measurements, &served_predictions);

    // Cross-validation is performed once by the fitter. Validation only reports that result;
    // it never assigns or refits folds independently (GitHub issue #96).
    let cross_validation = if config.num_folds > 1 {
        let result = correction_surface.cross_validation().ok_or_else(|| {
            ValidationError::InvalidParameter {
                param: "num_folds".to_string(),
                value: config.num_folds.to_string(),
                reason: "validation requested cross-validation, but the fitted surface carries no cross-validation result".to_string(),
            }
        })?;
        if result.num_folds() != config.num_folds {
            return Err(ValidationError::InvalidParameter {
                param: "num_folds".to_string(),
                value: config.num_folds.to_string(),
                reason: format!(
                    "validation requested {} folds, but the fitted surface carries {} folds",
                    config.num_folds,
                    result.num_folds()
                ),
            });
        }
        Some(result.clone())
    } else {
        None
    };

    // Overall assessment
    let meets_accuracy_requirements = main_lobe_meets_target && first_sidelobe_meets_target;

    if meets_accuracy_requirements {
        info!("✓ Calibration meets accuracy requirements (<1 dB in main lobe and first sidelobe)");
    } else {
        warn!("✗ Calibration does NOT meet accuracy requirements");
    }

    Ok(ValidationReport {
        num_points,
        model_only_rmse,
        model_only_max_error,
        model_only_r_squared,
        served_behavior_rmse,
        in_support_correction_rmse: support_metrics.in_support_correction_rmse,
        out_of_support_points: support_metrics.out_of_support_points,
        out_of_support_proportion: support_metrics.out_of_support_proportion,
        corrected_max_error: served_behavior_max_error,
        corrected_r_squared: served_behavior_r_squared,
        rmse_improvement_percent,
        max_error_improvement_percent,
        main_lobe_num_points,
        main_lobe_max_error,
        main_lobe_rmse,
        main_lobe_meets_target,
        first_sidelobe_num_points,
        first_sidelobe_max_error,
        first_sidelobe_rmse,
        first_sidelobe_meets_target,
        outliers,
        num_outliers,
        frequency_band_analysis,
        angular_region_analysis,
        cross_validation,
        meets_accuracy_requirements,
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
struct EvaluatedPrediction {
    served_db: f64,
    correction: CorrectionEvaluation,
}

/// Apply the served-gain support law without erasing the typed correction outcome.
fn evaluated_prediction(physics_db: f64, correction: CorrectionEvaluation) -> EvaluatedPrediction {
    let served_db = match correction {
        CorrectionEvaluation::Applied(correction_db) => physics_db + correction_db,
        CorrectionEvaluation::OutsideSupport => physics_db,
    };
    EvaluatedPrediction {
        served_db,
        correction,
    }
}

/// Compute served predictions using the fitted core correction surface.
fn compute_served_predictions(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
    correction_surface: &CorrectionSurface,
) -> Result<Vec<EvaluatedPrediction>> {
    let fitted = correction_surface.fitted();
    Ok(measurements
        .iter()
        .zip(model_predictions.iter())
        .map(|(measurement, &physics_db)| {
            evaluated_prediction(
                physics_db,
                fitted.evaluate(
                    measurement.e_clock_deg,
                    measurement.e_cone_deg,
                    measurement.frequency_mhz,
                ),
            )
        })
        .collect())
}

/// Compute root mean squared error
fn compute_rmse(measured: &[f64], predicted: &[f64]) -> f64 {
    if measured.is_empty() {
        return 0.0;
    }

    let sum_squared_errors: f64 = measured
        .iter()
        .zip(predicted.iter())
        .map(|(m, p)| (m - p).powi(2))
        .sum();

    (sum_squared_errors / measured.len() as f64).sqrt()
}

/// Compute maximum absolute error
fn compute_max_error(measured: &[f64], predicted: &[f64]) -> f64 {
    measured
        .iter()
        .zip(predicted.iter())
        .map(|(m, p)| (m - p).abs())
        .fold(0.0f64, f64::max)
}

/// Compute R-squared (coefficient of determination)
fn compute_r_squared(measured: &[f64], predicted: &[f64]) -> f64 {
    if measured.is_empty() {
        return 0.0;
    }

    let mean_measured: f64 = measured.iter().sum::<f64>() / measured.len() as f64;

    let ss_total: f64 = measured.iter().map(|m| (m - mean_measured).powi(2)).sum();
    let ss_residual: f64 = measured
        .iter()
        .zip(predicted.iter())
        .map(|(m, p)| (m - p).powi(2))
        .sum();

    if ss_total == 0.0 {
        return 0.0;
    }

    1.0 - (ss_residual / ss_total)
}

/// Classify measurement points into regions (main lobe, first sidelobe, far field)
fn classify_points_by_region(
    measurements: &[MeasurementPoint],
    config: &ValidationConfig,
) -> (Vec<usize>, Vec<usize>) {
    let mut main_lobe_indices = Vec::new();
    let mut first_sidelobe_indices = Vec::new();

    // Estimate beamwidth (rough approximation)
    // For most antennas, HPBW ≈ 70λ/D degrees (for parabolic dishes)
    // Here we use a simple threshold based on E-cone angle
    let main_lobe_threshold = config.main_lobe_beamwidths * 2.0; // degrees (rough estimate)

    for (i, meas) in measurements.iter().enumerate() {
        let cone_angle = meas.e_cone_deg.abs();

        if cone_angle <= main_lobe_threshold {
            main_lobe_indices.push(i);
        } else if cone_angle <= config.first_sidelobe_max_deg {
            first_sidelobe_indices.push(i);
        }
        // Points beyond first_sidelobe_max_deg are far field (not tracked separately)
    }

    debug!(
        "Classified {} main lobe points, {} first sidelobe points",
        main_lobe_indices.len(),
        first_sidelobe_indices.len()
    );

    (main_lobe_indices, first_sidelobe_indices)
}

/// Compute statistics for a specific region
fn compute_region_stats(
    measured: &[f64],
    predicted: &[f64],
    indices: &[usize],
) -> (f64, f64, usize) {
    if indices.is_empty() {
        return (0.0, 0.0, 0);
    }

    let region_measured: Vec<f64> = indices.iter().map(|&i| measured[i]).collect();
    let region_predicted: Vec<f64> = indices.iter().map(|&i| predicted[i]).collect();

    let rmse = compute_rmse(&region_measured, &region_predicted);
    let max_error = compute_max_error(&region_measured, &region_predicted);

    (rmse, max_error, indices.len())
}

/// Identify high-error served predictions.
///
/// These are model-error diagnostics, not a measurement-quality classification. A point outside
/// fitted support may appear here when its physics-only served prediction exceeds the threshold;
/// issue #96 will expose that support disposition in the report.
fn identify_outliers(
    measurements: &[MeasurementPoint],
    served_predictions: &[f64],
    threshold_db: f64,
    config: &ValidationConfig,
) -> Vec<OutlierPoint> {
    let mut outliers = Vec::new();

    let (main_lobe_indices, first_sidelobe_indices) =
        classify_points_by_region(measurements, config);

    for (i, (meas, &pred)) in measurements
        .iter()
        .zip(served_predictions.iter())
        .enumerate()
    {
        let error = (meas.g_over_t_db - pred).abs();

        if error > threshold_db {
            let region = if main_lobe_indices.contains(&i) {
                "Main Lobe"
            } else if first_sidelobe_indices.contains(&i) {
                "First Sidelobe"
            } else {
                "Far Field"
            };

            outliers.push(OutlierPoint {
                frequency_mhz: meas.frequency_mhz,
                e_cone_deg: meas.e_cone_deg,
                e_clock_deg: meas.e_clock_deg,
                measured_db: meas.g_over_t_db,
                predicted_db: pred,
                error_db: error,
                region: region.to_string(),
            });
        }
    }

    outliers
}

/// Analyze errors by frequency band
fn analyze_by_frequency_band(
    measurements: &[MeasurementPoint],
    served_predictions: &[f64],
    bands: &[(f64, f64)],
) -> Vec<FrequencyBandStats> {
    let mut results = Vec::new();

    for &(band_min, band_max) in bands {
        let mut band_measured = Vec::new();
        let mut band_predicted = Vec::new();

        for (meas, &pred) in measurements.iter().zip(served_predictions.iter()) {
            if meas.frequency_mhz >= band_min && meas.frequency_mhz < band_max {
                band_measured.push(meas.g_over_t_db);
                band_predicted.push(pred);
            }
        }

        if !band_measured.is_empty() {
            let rmse = compute_rmse(&band_measured, &band_predicted);
            let max_error = compute_max_error(&band_measured, &band_predicted);
            let mean_error: f64 = band_measured
                .iter()
                .zip(band_predicted.iter())
                .map(|(m, p)| m - p)
                .sum::<f64>()
                / band_measured.len() as f64;

            results.push(FrequencyBandStats {
                band_min_mhz: band_min,
                band_max_mhz: band_max,
                num_points: band_measured.len(),
                rmse_db: rmse,
                max_error_db: max_error,
                mean_error_db: mean_error,
            });
        }
    }

    results
}

/// Analyze errors by angular region (E-cone bins)
fn analyze_by_angular_region(
    measurements: &[MeasurementPoint],
    served_predictions: &[f64],
) -> Vec<AngularRegionStats> {
    // Define angular regions (E-cone bins)
    let regions = vec![
        ("Near boresight (0-2°)", 0.0, 2.0),
        ("Main lobe (2-5°)", 2.0, 5.0),
        ("Near sidelobes (5-10°)", 5.0, 10.0),
        ("Far sidelobes (10-20°)", 10.0, 20.0),
        ("Far field (>20°)", 20.0, 90.0),
    ];

    let mut results = Vec::new();

    for (region_name, cone_min, cone_max) in regions {
        let mut region_measured = Vec::new();
        let mut region_predicted = Vec::new();

        for (meas, &pred) in measurements.iter().zip(served_predictions.iter()) {
            let cone = meas.e_cone_deg.abs();
            if cone >= cone_min && cone < cone_max {
                region_measured.push(meas.g_over_t_db);
                region_predicted.push(pred);
            }
        }

        if !region_measured.is_empty() {
            let rmse = compute_rmse(&region_measured, &region_predicted);
            let max_error = compute_max_error(&region_measured, &region_predicted);
            let mean_error: f64 = region_measured
                .iter()
                .zip(region_predicted.iter())
                .map(|(m, p)| m - p)
                .sum::<f64>()
                / region_measured.len() as f64;

            results.push(AngularRegionStats {
                region_name: region_name.to_string(),
                cone_min_deg: cone_min,
                cone_max_deg: cone_max,
                num_points: region_measured.len(),
                rmse_db: rmse,
                max_error_db: max_error,
                mean_error_db: mean_error,
            });
        }
    }

    results
}

// ============================================================================
// Report Formatting
// ============================================================================

impl ValidationReport {
    /// Format the validation report as a human-readable string
    pub fn format_summary(&self) -> String {
        let mut s = String::new();
        s.push_str("=================================================\n");
        s.push_str("        ANTENNA CALIBRATION VALIDATION REPORT    \n");
        s.push_str("=================================================\n\n");

        s.push_str(&format!("Total data points: {}\n\n", self.num_points));

        s.push_str("Model Performance:\n");
        s.push_str("------------------\n");
        s.push_str(&format!(
            "Model-only RMSE:        {:.3} dB\n",
            self.model_only_rmse
        ));
        s.push_str(&format!(
            "Model-only max error:   {:.3} dB\n",
            self.model_only_max_error
        ));
        s.push_str(&format!(
            "Model-only R²:          {:.4}\n\n",
            self.model_only_r_squared
        ));

        s.push_str(&format!(
            "Served-behavior RMSE: {:.3} dB\n",
            self.served_behavior_rmse
        ));
        match self.in_support_correction_rmse {
            Some(rmse) => s.push_str(&format!("In-support correction RMSE: {rmse:.3} dB\n")),
            None => s.push_str("In-support correction RMSE: n/a (no point had fitted support)\n"),
        }
        s.push_str(&format!(
            "Out of support: {} points ({:.1}%)\n",
            self.out_of_support_points,
            100.0 * self.out_of_support_proportion
        ));
        s.push_str(&format!(
            "Served-behavior max error: {:.3} dB\n",
            self.corrected_max_error
        ));
        s.push_str(&format!(
            "Served-behavior R²: {:.4}\n\n",
            self.corrected_r_squared
        ));

        s.push_str(&format!(
            "RMSE improvement:       {:.1}%\n",
            self.rmse_improvement_percent
        ));
        s.push_str(&format!(
            "Max error improvement:  {:.1}%\n\n",
            self.max_error_improvement_percent
        ));

        s.push_str("Regional Analysis:\n");
        s.push_str("------------------\n");
        s.push_str(&format!(
            "Main lobe ({} points):\n",
            self.main_lobe_num_points
        ));
        s.push_str(&format!("  RMSE:       {:.3} dB\n", self.main_lobe_rmse));
        s.push_str(&format!(
            "  Max error:  {:.3} dB\n",
            self.main_lobe_max_error
        ));
        s.push_str(&format!(
            "  Target:     ≤1.0 dB [{}]\n\n",
            if self.main_lobe_meets_target {
                "PASS"
            } else {
                "FAIL"
            }
        ));

        s.push_str(&format!(
            "First sidelobe ({} points):\n",
            self.first_sidelobe_num_points
        ));
        s.push_str(&format!(
            "  RMSE:       {:.3} dB\n",
            self.first_sidelobe_rmse
        ));
        s.push_str(&format!(
            "  Max error:  {:.3} dB\n",
            self.first_sidelobe_max_error
        ));
        s.push_str(&format!(
            "  Target:     ≤1.0 dB [{}]\n\n",
            if self.first_sidelobe_meets_target {
                "PASS"
            } else {
                "FAIL"
            }
        ));

        if self.num_outliers > 0 {
            s.push_str(&format!(
                "Outliers (error >1 dB): {} points\n\n",
                self.num_outliers
            ));
        }

        if let Some(ref cv) = self.cross_validation {
            s.push_str("Cross-Validation:\n");
            s.push_str("------------------\n");
            s.push_str(&format!(
                "{}-fold cross-validation (strided folds: point i is held out by fold \
                 i % {})\n",
                cv.num_folds(),
                cv.num_folds()
            ));
            match (cv.mean_rmse(), cv.std_rmse()) {
                (Some(mean), Some(std)) => s.push_str(&format!(
                    "Served-behavior mean RMSE:  {mean:.3} ± {std:.3} dB\n"
                )),
                _ => s.push_str("Served-behavior mean RMSE:  n/a (no fold could be scored)\n"),
            }
            match (cv.min_rmse(), cv.max_rmse()) {
                (Some(min), Some(max)) => {
                    s.push_str(&format!("Range:      {min:.3} - {max:.3} dB\n"))
                }
                _ => s.push_str("Range:      n/a\n"),
            }

            // Per-fold values, not just the summary (roadmap D22). A mean of 4.45 ± 4.92 dB
            // reads as one noisy number; the folds behind it were 10.07 / 0.56 / 0.12 /
            // 0.64 / 10.86, which reads as two populations and is what exposed the defect.
            //
            // Each value is labelled with its own fold NUMBER, not its position:
            // `fold_rmse_values` is dense and skips folds that could not be scored, so with
            // folds 1 and 2 failing, printing positionally would report fold 3's RMSE as
            // "fold 1".
            for fold in cv.scored_folds() {
                let in_support = fold
                    .in_support_correction_rmse()
                    .map(|rmse| format!("{rmse:.3} dB"))
                    .unwrap_or_else(|| "n/a".to_string());
                s.push_str(&format!(
                    "Fold #{}: served {:.3} dB; in-support {}; out of support {} / {} ({:.1}%)\n",
                    fold.fold(),
                    fold.served_behavior_rmse(),
                    in_support,
                    fold.out_of_support_points(),
                    fold.validation_points(),
                    100.0 * fold.out_of_support_proportion()
                ));
            }

            if !cv.is_complete() {
                s.push_str(&format!(
                    "\n⚠ SUPPORT INCOMPLETE: {} of {} folds failed and {} held-out points were \
                     outside their fold's fitted support. The figures above cover {} scored \
                     folds and use physics-only predictions for those unsupported points. The \
                     artifact is still written — its own fit succeeded on the full dataset.\n",
                    cv.failed_folds().len(),
                    cv.num_folds(),
                    cv.out_of_support_points(),
                    cv.fold_rmse_values().len()
                ));
                for failure in cv.failed_folds() {
                    s.push_str(&format!("    {}\n", failure.reason));
                }
            }
            s.push('\n');
        }

        s.push_str("Diagnostic RMSEs above are not acceptance gates; the result below uses regional maximum error.\n");
        s.push_str("=================================================\n");
        s.push_str(&format!(
            "OVERALL RESULT: {}\n",
            if self.meets_accuracy_requirements {
                "✓ PASS - Meets accuracy requirements"
            } else {
                "✗ FAIL - Does not meet accuracy requirements"
            }
        ));
        s.push_str("=================================================\n");

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correction_surface::CorrectionSurfaceParams;
    use antenna_core::model::{CorrectionSurfaceLayout, FittedCorrectionSurface};

    #[test]
    fn test_compute_rmse() {
        let measured = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let predicted = vec![1.1, 2.1, 2.9, 4.2, 4.8];
        let rmse = compute_rmse(&measured, &predicted);
        assert!((rmse - 0.152).abs() < 0.01);
    }

    #[test]
    fn test_compute_max_error() {
        let measured = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let predicted = vec![1.1, 2.1, 2.9, 4.2, 4.8];
        let max_error = compute_max_error(&measured, &predicted);
        assert!((max_error - 0.2).abs() < 1e-10);
    }

    #[test]
    fn test_compute_r_squared() {
        let measured = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let predicted = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // Perfect prediction
        let r_squared = compute_r_squared(&measured, &predicted);
        assert!((r_squared - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_validation_config_default() {
        let config = ValidationConfig::default();
        assert_eq!(config.num_folds, 5);
        assert_eq!(config.main_lobe_target_db, 1.0);
        assert_eq!(config.first_sidelobe_target_db, 1.0);
    }

    #[test]
    fn outside_support_uses_the_physics_prediction_during_validation() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
        )
        .expect("a clamped unit layout");
        let surface = CorrectionSurface::new(
            FittedCorrectionSurface::new(layout, vec![1.0; 8]).expect("eight coefficients"),
            crate::correction_surface::FitStatistics {
                num_points: 2,
                rmse_db: 0.0,
                max_residual_db: 0.0,
                r_squared: 1.0,
                cross_validation_rmse: None,
                improvement_percent: 0.0,
            },
        );
        let measurements = vec![
            MeasurementPoint::new(0.5, 0.5, 0.5, 11.0, 290.0),
            MeasurementPoint::new(2.0, 0.5, 0.5, 13.0, 290.0),
        ];
        let predictions = vec![10.0, 10.0];
        let config = ValidationConfig {
            num_folds: 0,
            frequency_bands: vec![],
            outlier_threshold_db: 2.0,
            main_lobe_target_db: 4.0,
            first_sidelobe_target_db: 4.0,
            ..ValidationConfig::default()
        };

        let evaluated = compute_served_predictions(&measurements, &predictions, &surface)
            .expect("the fitted core surface must return typed outcomes");
        assert_eq!(
            evaluated,
            vec![
                EvaluatedPrediction {
                    served_db: 11.0,
                    correction: CorrectionEvaluation::Applied(1.0),
                },
                EvaluatedPrediction {
                    served_db: 10.0,
                    correction: CorrectionEvaluation::OutsideSupport,
                },
            ],
            "physics-only fallback must retain its typed support disposition internally"
        );

        let report = validate_calibration(&measurements, &predictions, &surface, &config)
            .expect("outside support must fall back to the physics prediction");

        let expected_served_rmse = 3.0 / 2.0_f64.sqrt();
        assert!((report.served_behavior_rmse - expected_served_rmse).abs() < 1e-12);
        assert_eq!(report.in_support_correction_rmse, Some(0.0));
        assert_eq!(report.out_of_support_points, 1);
        assert_eq!(report.out_of_support_proportion, 0.5);
        assert_ne!(
            report.in_support_correction_rmse,
            Some(expected_served_rmse),
            "negative control: treating OutsideSupport as Applied(0.0) would put its 3 dB error in the in-support denominator"
        );
        assert_eq!(report.corrected_max_error, 3.0);
        assert_eq!(report.outliers.len(), 1);
        assert_eq!(report.outliers[0].predicted_db, 10.0);
        assert!(report.served_behavior_rmse > 1.0);
        assert!(
            report.meets_accuracy_requirements,
            "diagnostic RMSE must not replace the configured regional max-error policy"
        );
    }

    #[test]
    fn all_out_of_support_points_have_no_in_support_rmse() {
        let layout = CorrectionSurfaceLayout::new(
            [2, 2, 2],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
        )
        .expect("a clamped unit layout");
        let surface = CorrectionSurface::new(
            FittedCorrectionSurface::new(layout, vec![1.0; 8]).expect("eight coefficients"),
            crate::correction_surface::FitStatistics {
                num_points: 1,
                rmse_db: 0.0,
                max_residual_db: 0.0,
                r_squared: 1.0,
                cross_validation_rmse: None,
                improvement_percent: 0.0,
            },
        );
        let measurements = vec![MeasurementPoint::new(2.0, 2.0, 2.0, 13.0, 290.0)];
        let predictions = vec![10.0];
        let config = ValidationConfig {
            num_folds: 0,
            frequency_bands: vec![],
            ..ValidationConfig::default()
        };

        let report = validate_calibration(&measurements, &predictions, &surface, &config)
            .expect("physics-only served behavior remains scoreable");

        assert_eq!(report.served_behavior_rmse, 3.0);
        assert_eq!(report.in_support_correction_rmse, None);
        assert_eq!(report.out_of_support_points, 1);
        assert_eq!(report.out_of_support_proportion, 1.0);
    }

    // ========================================================================
    // D10 — the cross-validation fold refit must score the surface that ships
    // ========================================================================

    /// A grid large enough that a 5-fold split still covers the fitted **coefficient
    /// count**, which is what roadmap D20 made the binding quantity — not the old
    /// `(spline_order + 1)³ = 125` minimum, which depended on nothing about the model
    /// being fitted.
    ///
    /// `artifact_params` below declares 4 × 10 × 10 = 400 coefficients (the frequency axis
    /// has only two distinct values, so it places no interior knot and contributes `order`
    /// basis functions). 640 points leaves 512 in a 5-fold training split, 1.28× the
    /// coefficients. Growing the cone axis is what buys the margin: its knot count is
    /// already capped by the 6 requested, so more distinct cone values add data without
    /// adding coefficients.
    fn cv_fixture() -> (Vec<MeasurementPoint>, Vec<f64>) {
        cv_fixture_with_cone_values(40)
    }

    /// As [`cv_fixture`], with the cone axis length as a parameter.
    ///
    /// The cone axis is the one that adds points without adding coefficients (its knot count
    /// is capped by the 6 requested), so it is the knob for putting a fixture on either side
    /// of the coefficient count — which is what
    /// `a_fold_refit_failure_names_the_fold_and_both_point_counts` needs.
    fn cv_fixture_with_cone_values(cone_values: usize) -> (Vec<MeasurementPoint>, Vec<f64>) {
        let mut points = Vec::new();
        let mut predictions = Vec::new();
        for fi in 0..2 {
            let frequency_mhz = 8400.0 + 100.0 * fi as f64;
            for ci in 0..cone_values {
                let e_cone_deg = ci as f64;
                for ki in 0..8 {
                    let e_clock_deg = 45.0 * ki as f64;
                    // Main-lobe rolloff plus a clock-dependent ripple the physics model
                    // does not carry — the ripple is what the surface has to fit.
                    let ripple = 0.4 * e_clock_deg.to_radians().cos() * (1.0 + 0.05 * e_cone_deg);
                    let measured = 41.5 - 0.35 * e_cone_deg * e_cone_deg + ripple;
                    let model = 41.5 - 0.33 * e_cone_deg * e_cone_deg;
                    points.push(MeasurementPoint::new(
                        e_clock_deg,
                        e_cone_deg,
                        frequency_mhz,
                        measured,
                        50.0,
                    ));
                    predictions.push(model);
                }
            }
        }
        (points, predictions)
    }

    /// The parameters `calibrate` actually fits the shipped artifact with, read from their
    /// one owner. This helper used to hand-copy them (roadmap D26 exit criterion 6), which
    /// is how a test asserting "the CV folds score the shipped model family" could keep
    /// passing against a family the CLI had stopped shipping.
    fn artifact_params() -> CorrectionSurfaceParams {
        CorrectionSurfaceParams::shipped()
    }

    fn config_with_folds(num_folds: usize) -> ValidationConfig {
        ValidationConfig {
            num_folds,
            main_lobe_beamwidths: 1.0,
            first_sidelobe_max_deg: 5.0,
            frequency_bands: vec![],
            main_lobe_target_db: 1.0,
            first_sidelobe_target_db: 1.0,
            outlier_threshold_db: 3.0,
        }
    }

    fn run_cv(
        correction_params: CorrectionSurfaceParams,
        num_folds: usize,
    ) -> Result<CrossValidationResults> {
        let (points, predictions) = cv_fixture();
        let params = CorrectionSurfaceParams {
            cross_validation_folds: num_folds,
            ..correction_params
        };
        let surface =
            crate::correction_surface::fit_correction_surface(&points, &predictions, &params)?;
        let stored = surface
            .cross_validation()
            .cloned()
            .expect("num_folds > 1, so fitting must cross-validate");
        let report = validate_calibration(
            &points,
            &predictions,
            &surface,
            &config_with_folds(num_folds),
        )?;
        assert_eq!(
            report.cross_validation.as_ref(),
            Some(&stored),
            "validation must report the fitter's result without recomputing folds"
        );
        Ok(report
            .cross_validation
            .expect("num_folds > 1, so cross-validation must be reported"))
    }

    /// **Filed by D14's review, resolved by D22 (2026-08-03).** A dataset can clear the
    /// coefficient count on the whole set and miss it on a `(1 − 1/folds)` training split,
    /// so since roadmap D20 a fold refit can fail on data whose own fit succeeded.
    ///
    /// This test originally asserted that the whole run **failed** — which made `--validate`
    /// destructive: it removed an artifact that the same command without it produces. The
    /// maintainer's D22 call was to warn and still ship, so the assertion is inverted here:
    /// validation must *complete*, report the failure, and leave the surviving folds
    /// scored. What it kept is the diagnosis. Before the fold refit wrapped its error the
    /// failure surfaced as a bare `UnderdeterminedFit` quoting a point count *the caller
    /// never supplied* (the training split), immediately after a full-set fit at a larger
    /// count had succeeded; the three facts a reader needs — which fold, how big its split
    /// was, how big the real dataset is — are still asserted.
    #[test]
    fn a_fold_refit_failure_is_recorded_and_names_the_fold_and_both_point_counts() {
        // 448 points against the 400 coefficients `artifact_params` declares: the whole set
        // clears them, a 5-fold training split (358–359) does not.
        let (points, predictions) = cv_fixture_with_cone_values(28);
        assert_eq!(points.len(), 448);

        let params = CorrectionSurfaceParams {
            cross_validation_folds: 5,
            ..artifact_params()
        };
        let surface =
            crate::correction_surface::fit_correction_surface(&points, &predictions, &params)
                .expect("the whole set must fit — that is the premise of this test");

        let report = validate_calibration(&points, &predictions, &surface, &config_with_folds(5))
            .expect(
                "a fold that cannot refit must not fail the run: validation is not allowed to \
             withhold an artifact whose own fit succeeded (roadmap D22)",
            );

        let cv = report
            .cross_validation
            .as_ref()
            .expect("cross-validation ran");
        assert!(
            !cv.is_complete(),
            "premise broken: this fixture is sized so folds cannot refit"
        );
        assert_eq!(
            cv.failed_folds().len() + cv.fold_rmse_values().len(),
            cv.num_folds(),
            "every requested fold must be accounted for, scored or failed"
        );

        let reasons = cv
            .failed_folds()
            .iter()
            .map(|f| f.reason.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        for needle in ["fold 1/5", "training split of 358", "the full set has 448"] {
            assert!(
                reasons.contains(needle),
                "a fold refit failure must say {needle:?}; got: {reasons}"
            );
        }

        // The report a human reads has to say so too — a mean over the folds that happened
        // to survive, printed without that caveat, is the shape of claim D22 exists to stop.
        let summary = report.format_summary();
        assert!(
            summary.contains("INCOMPLETE"),
            "the summary must declare an incomplete cross-validation; got:\n{summary}"
        );
    }

    /// **Roadmap D22.** Folds are strided, not contiguous slices of the input file.
    ///
    /// Calls `correction_surface::is_held_out` — the crate's single fold-assignment
    /// definition used by the sole cross-validation implementation. An earlier version of
    /// this test re-implemented `i % num_folds != fold` inline, which made it a test of the
    /// *fixture* rather than of the code it guards: reverting the implementation to
    /// contiguous slices would have left it passing.
    ///
    /// The discriminating property: a grid-ordered file's contiguous first fold holds out an
    /// entire leading frequency slab, so its training set contains **no** point at that
    /// frequency and scoring it is an extrapolation. Under striding every fold's training
    /// set spans every frequency present.
    #[test]
    fn folds_are_strided_so_no_fold_holds_out_a_whole_frequency_slab() {
        use crate::correction_surface::is_held_out;

        let (points, _) = cv_fixture();
        let num_folds = 5;
        let all_frequencies: std::collections::BTreeSet<_> =
            points.iter().map(|p| p.frequency_mhz.to_bits()).collect();
        assert!(
            all_frequencies.len() > 1,
            "fixture must span several frequencies or this test is vacuous"
        );

        // The fixture must actually be grid-ordered, or the old blocked assignment would
        // have been harmless here and this test would prove nothing about it.
        let contiguous_first_fold: std::collections::BTreeSet<_> = points
            [..points.len() / num_folds]
            .iter()
            .map(|p| p.frequency_mhz.to_bits())
            .collect();
        assert!(
            contiguous_first_fold.len() < all_frequencies.len(),
            "negative control: this fixture is not grid-ordered, so it cannot demonstrate \
             what strided assignment fixes"
        );

        for fold in 0..num_folds {
            let train: std::collections::BTreeSet<_> = points
                .iter()
                .enumerate()
                .filter(|(i, _)| !is_held_out(*i, fold, num_folds))
                .map(|(_, p)| p.frequency_mhz.to_bits())
                .collect();
            assert_eq!(
                train, all_frequencies,
                "fold {fold}'s training set is missing a frequency present in the data, so \
                 scoring it extrapolates past the fitted knots"
            );
        }
    }

    /// **Roadmap D22.** Per-fold output is labelled by fold *number*, not by position.
    ///
    /// `fold_rmse_values` is dense — a fold that could not refit contributes no entry — so
    /// printing it positionally silently relabels the survivors: with fold 1 failing, the
    /// value shown as the first fold is really fold 2. A cross-validation report whose fold
    /// labels are wrong is worse than one that omits them, because the reader cannot tell.
    #[test]
    fn scored_fold_numbers_skip_the_folds_that_failed() {
        let cv = CrossValidationResults::from_fold_results(
            vec![
                CrossValidationFoldResult::new(2, 10, 0.30, Some(0.30), 0).expect("valid fold"),
                CrossValidationFoldResult::new(3, 10, 0.50, Some(0.50), 0).expect("valid fold"),
                CrossValidationFoldResult::new(5, 10, 0.40, Some(0.40), 0).expect("valid fold"),
            ],
            vec![
                FoldFailure {
                    fold: 1,
                    training_points: 10,
                    reason: "fold 1/5 could not refit".to_string(),
                },
                FoldFailure {
                    fold: 4,
                    training_points: 10,
                    reason: "fold 4/5 could not refit".to_string(),
                },
            ],
        )
        .expect("complete fold outcomes");

        assert_eq!(
            cv.scored_fold_numbers(),
            vec![2, 3, 5],
            "folds 1 and 4 failed, so the three scored values belong to folds 2, 3 and 5"
        );
        assert_eq!(cv.scored_fold_numbers().len(), cv.fold_rmse_values().len());

        let report = ValidationReport {
            cross_validation: Some(cv),
            ..minimal_report()
        };
        let summary = report.format_summary();
        assert!(
            summary.contains("Fold #2: served 0.300") && summary.contains("Fold #5: served 0.400"),
            "each fold RMSE must be labelled with its own fold number; got:\n{summary}"
        );
        assert!(
            !summary.contains("Fold #1: served 0.300"),
            "fold 1 failed — its number must not be attached to fold 2's value:\n{summary}"
        );
    }

    /// A report with everything zeroed, for tests that only care about one section.
    fn minimal_report() -> ValidationReport {
        ValidationReport {
            num_points: 0,
            model_only_rmse: 0.0,
            model_only_max_error: 0.0,
            model_only_r_squared: 0.0,
            served_behavior_rmse: 0.0,
            in_support_correction_rmse: None,
            out_of_support_points: 0,
            out_of_support_proportion: 0.0,
            corrected_max_error: 0.0,
            corrected_r_squared: 0.0,
            rmse_improvement_percent: 0.0,
            max_error_improvement_percent: 0.0,
            main_lobe_num_points: 0,
            main_lobe_max_error: 0.0,
            main_lobe_rmse: 0.0,
            main_lobe_meets_target: true,
            first_sidelobe_num_points: 0,
            first_sidelobe_max_error: 0.0,
            first_sidelobe_rmse: 0.0,
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
    fn served_behavior_fields_are_labeled_by_their_semantics() {
        let report = ValidationReport {
            served_behavior_rmse: 0.25,
            ..minimal_report()
        };

        let summary = report.format_summary();
        assert!(
            summary.contains("Served-behavior RMSE: 0.250 dB"),
            "served_behavior_rmse includes physics-only fallback and must be labeled by its served-behavior semantics; got:\n{summary}"
        );
        assert!(
            !summary.contains("Corrected RMSE"),
            "unsupported points are not corrected, so this label is misleading; got:\n{summary}"
        );
    }

    /// Correction support is complete only when every held-out observation has a
    /// correction value. The served-behavior RMSE still includes physics-only predictions
    /// outside support, while the count keeps those outcomes distinguishable.
    #[test]
    fn a_partially_unsupported_fold_is_recorded_as_incomplete() {
        let (mut points, predictions) = cv_fixture();
        points[0].e_clock_deg = 359.0;
        points[0].g_over_t_db = predictions[0] + 50.0;
        let params = CorrectionSurfaceParams {
            cross_validation_folds: 5,
            ..artifact_params()
        };
        let surface =
            crate::correction_surface::fit_correction_surface(&points, &predictions, &params)
                .expect("the full fit includes the unique extreme point");

        let report = validate_calibration(&points, &predictions, &surface, &config_with_folds(5))
            .expect("an incomplete fold is reportable and does not withhold the artifact");
        let cv = report.cross_validation.expect("cross-validation ran");

        assert!(!cv.is_complete());
        assert!(cv.failed_folds().is_empty());
        assert_eq!(cv.out_of_support_points(), 1);
        assert_eq!(cv.scored_folds().len(), 5);

        // Point 0 belongs to fold 1 and lies outside that fold's fitted support. Its
        // physics-only error is exactly 50 dB, so including it puts a hard lower bound on
        // the fold RMSE. Omitting it (the pre-#93 behavior) falls below this bound, while
        // the support count above proves the value was not relabelled Applied(0.0).
        let fold_one = &cv.scored_folds()[0];
        let physics_only_lower_bound = 50.0 / (fold_one.validation_points() as f64).sqrt();
        assert_eq!(fold_one.fold(), 1);
        assert_eq!(fold_one.out_of_support_points(), 1);
        assert_eq!(
            fold_one.out_of_support_proportion(),
            1.0 / fold_one.validation_points() as f64
        );
        assert!(fold_one.in_support_correction_rmse().is_some());
        assert!(
            fold_one.served_behavior_rmse() >= physics_only_lower_bound,
            "fold 1 served RMSE {} omitted the 50 dB physics-only error; expected at least {}",
            fold_one.served_behavior_rmse(),
            physics_only_lower_bound
        );
    }

    #[test]
    fn a_fold_with_no_fitted_support_reports_no_in_support_rmse() {
        let (mut points, predictions) = cv_fixture();
        for (index, point) in points.iter_mut().enumerate() {
            if crate::correction_surface::is_held_out(index, 0, 5) {
                point.e_clock_deg = 359.0;
            }
        }
        let params = CorrectionSurfaceParams {
            cross_validation_folds: 5,
            ..artifact_params()
        };
        let surface =
            crate::correction_surface::fit_correction_surface(&points, &predictions, &params)
                .expect("the full fit includes both clock domains");
        let report = validate_calibration(&points, &predictions, &surface, &config_with_folds(5))
            .expect("a fold without support remains reportable");
        let fold_one = &report
            .cross_validation
            .as_ref()
            .expect("cross-validation ran")
            .scored_folds()[0];

        assert_eq!(fold_one.fold(), 1);
        assert_eq!(fold_one.in_support_correction_rmse(), None);
        assert_eq!(
            fold_one.out_of_support_points(),
            fold_one.validation_points()
        );
        assert_eq!(fold_one.out_of_support_proportion(), 1.0);
        assert!(fold_one.served_behavior_rmse().is_finite());
    }

    /// The behavioural counterpart: the sole fitting-side cross-validation implementation
    /// must produce folds that all score the same *kind* of question on a grid-ordered fixture.
    ///
    /// The test above proves the assignment function is strided; this proves cross-validation
    /// actually routes through it. Under the pre-D22 contiguous slicing the edge folds
    /// extrapolated a whole frequency slab and came out orders of magnitude worse than the
    /// interior ones — 89× between best and worst on D14's artifact. Requiring the spread to
    /// stay inside one order of magnitude distinguishes "every fold interpolates" from "these
    /// numbers all happen to be small".
    #[test]
    fn cross_validation_folds_all_score_comparably_on_a_grid_ordered_fixture() {
        let cv = run_cv(artifact_params(), 5).expect("cv on the grid-ordered fixture");
        assert!(
            cv.is_complete(),
            "premise broken: this fixture is sized so every fold refits"
        );

        let best = cv.min_rmse().expect("a scored fold has a min");
        let worst = cv.max_rmse().expect("a scored fold has a max");
        assert!(
            worst < 10.0 * best,
            "fold RMSEs show two populations (worst {worst:.4} dB vs best {best:.4} dB, folds \
             {:?}) — the signature of contiguous folds holding out a whole axis slab. Check \
             that correction_surface::cross_validate still routes through \
             its shared is_held_out definition.",
            cv.fold_rmse_values()
        );
    }

    /// The fold refit must fit the *caller's* model family. Two configs that differ only
    /// in knot counts and regularization must therefore produce different CV numbers —
    /// if the refit fell back to `CorrectionSurfaceParams::default()` (the pre-D10 bug)
    /// both would fit the identical surface and report the identical RMSE.
    #[test]
    fn fold_refit_uses_caller_knot_counts_and_regularization() {
        let sparse = run_cv(artifact_params(), 5).expect("sparse config");
        let dense = run_cv(
            CorrectionSurfaceParams {
                num_knots_frequency: 8,
                num_knots_econe: 8,
                num_knots_eclock: 12,
                regularization: 1e-6,
                ..artifact_params()
            },
            5,
        )
        .expect("dense config");

        let sparse_mean = sparse
            .mean_rmse()
            .expect("sparse config must score every fold");
        let dense_mean = dense
            .mean_rmse()
            .expect("dense config must score every fold");
        assert!(
            (sparse_mean - dense_mean).abs() > 1e-3,
            "knot counts and regularization did not reach the fold refit: \
             sparse={sparse_mean:.6} dB, dense={dense_mean:.6} dB"
        );
    }

    /// Spline order reaches the refit too, proven through the fitter's coefficient count.
    /// The 960-point full set covers order 6's 864 coefficients, while each five-fold
    /// training split has only 768 points and therefore records a fold failure.
    #[test]
    fn fold_refit_uses_caller_spline_order() {
        let (points, predictions) = cv_fixture_with_cone_values(60);
        assert_eq!(points.len(), 960);
        let params = CorrectionSurfaceParams {
            spline_order: 6,
            cross_validation_folds: 5,
            ..artifact_params()
        };
        let surface =
            crate::correction_surface::fit_correction_surface(&points, &predictions, &params)
                .expect("the full set covers the order-6 coefficient count");
        let report = validate_calibration(&points, &predictions, &surface, &config_with_folds(5))
            .expect("a fold that cannot refit no longer fails the run (roadmap D22)");
        let cv = report
            .cross_validation
            .expect("cross-validation is reported");

        assert!(
            !cv.is_complete(),
            "premise broken: order 6 must be unfittable from this fixture's training splits"
        );

        // Each axis contributes `placed_knots + order` basis functions, so the caller's
        // spline order is visible in the coefficient count: order 4 gives 4x10x10 = 400
        // (which this fixture covers), order 6 gives 6x12x12 = 864 (which it does not).
        // Before roadmap D20 this keyed on `(order + 1)^3 = 343`, a data minimum that
        // depended on the order but on nothing else about the model being fitted.
        let reasons = cv
            .failed_folds()
            .iter()
            .map(|f| f.reason.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            reasons.contains("864"),
            "expected the caller's spline order to set the coefficient count, got: {reasons}"
        );
    }

    /// `--cv-folds N` is threaded through `num_folds` and must be visible in the report.
    #[test]
    fn num_folds_controls_the_reported_fold_count() {
        for folds in [3usize, 5, 8] {
            let results =
                run_cv(artifact_params(), folds).unwrap_or_else(|e| panic!("{folds} folds: {e}"));

            assert_eq!(results.num_folds(), folds);
            assert_eq!(
                results.fold_rmse_values().len() + results.failed_folds().len(),
                folds,
                "every requested fold must be represented as scored or unsupported"
            );
        }
    }
}
