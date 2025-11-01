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
//! - Before/after comparison (model-only vs model+correction)
//! - Main lobe accuracy verification (<1 dB target)
//! - First sidelobe accuracy verification (<1 dB target)
//! - Outlier identification (>1 dB error cases)
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
//! println!("RMSE (corrected): {:.3} dB", report.corrected_rmse);
//! println!("Main lobe max error: {:.3} dB", report.main_lobe_max_error);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::correction_surface::{CorrectionSurface, CorrectionSurfaceError, CorrectionSurfaceParams};
use crate::parser::MeasurementPoint;
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

    /// Parameters for correction surface fitting during cross-validation
    pub correction_params: CorrectionSurfaceParams,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            num_folds: 5,
            main_lobe_beamwidths: 3.0,
            first_sidelobe_max_deg: 10.0,
            frequency_bands: vec![
                (100.0, 1000.0),     // VHF/UHF
                (1000.0, 3000.0),    // L/S band
                (3000.0, 12000.0),   // C/X band
                (12000.0, 50000.0),  // Ku/Ka/V band
            ],
            main_lobe_target_db: 1.0,
            first_sidelobe_target_db: 1.0,
            outlier_threshold_db: 1.0,
            correction_params: CorrectionSurfaceParams::default(),
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

    /// Corrected model performance (with correction surface)
    pub corrected_rmse: f64,
    pub corrected_max_error: f64,
    pub corrected_r_squared: f64,

    /// Improvement metrics
    pub rmse_improvement_percent: f64,
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

    /// Outlier analysis
    pub outliers: Vec<OutlierPoint>,
    pub num_outliers: usize,

    /// Error analysis by frequency band
    pub frequency_band_analysis: Vec<FrequencyBandStats>,

    /// Error analysis by angular region
    pub angular_region_analysis: Vec<AngularRegionStats>,

    /// Cross-validation results (if performed)
    pub cross_validation: Option<CrossValidationResults>,

    /// Overall success
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

/// Results from k-fold cross-validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossValidationResults {
    pub num_folds: usize,
    pub fold_rmse_values: Vec<f64>,
    pub mean_rmse: f64,
    pub std_rmse: f64,
    pub min_rmse: f64,
    pub max_rmse: f64,
}

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

    // Compute corrected predictions
    let corrected_predictions = compute_corrected_predictions(
        measurements,
        model_predictions,
        correction_surface,
    )?;

    // Extract measured values
    let measured: Vec<f64> = measurements.iter().map(|m| m.g_over_t_db).collect();

    // Model-only statistics
    let model_only_rmse = compute_rmse(&measured, model_predictions);
    let model_only_max_error = compute_max_error(&measured, model_predictions);
    let model_only_r_squared = compute_r_squared(&measured, model_predictions);

    // Corrected model statistics
    let corrected_rmse = compute_rmse(&measured, &corrected_predictions);
    let corrected_max_error = compute_max_error(&measured, &corrected_predictions);
    let corrected_r_squared = compute_r_squared(&measured, &corrected_predictions);

    // Improvement metrics
    let rmse_improvement_percent = if model_only_rmse > 0.0 {
        100.0 * (model_only_rmse - corrected_rmse) / model_only_rmse
    } else {
        0.0
    };

    let max_error_improvement_percent = if model_only_max_error > 0.0 {
        100.0 * (model_only_max_error - corrected_max_error) / model_only_max_error
    } else {
        0.0
    };

    info!(
        "Model-only RMSE: {:.3} dB, Corrected RMSE: {:.3} dB ({:.1}% improvement)",
        model_only_rmse, corrected_rmse, rmse_improvement_percent
    );

    // Classify points by region
    let (main_lobe_indices, first_sidelobe_indices) =
        classify_points_by_region(measurements, config);

    // Main lobe statistics
    let (main_lobe_rmse, main_lobe_max_error, main_lobe_num_points) =
        compute_region_stats(&measured, &corrected_predictions, &main_lobe_indices);
    let main_lobe_meets_target = main_lobe_max_error <= config.main_lobe_target_db;

    info!(
        "Main lobe: {} points, max error: {:.3} dB, RMSE: {:.3} dB (target: {:.1} dB, {})",
        main_lobe_num_points,
        main_lobe_max_error,
        main_lobe_rmse,
        config.main_lobe_target_db,
        if main_lobe_meets_target { "PASS" } else { "FAIL" }
    );

    // First sidelobe statistics
    let (first_sidelobe_rmse, first_sidelobe_max_error, first_sidelobe_num_points) =
        compute_region_stats(&measured, &corrected_predictions, &first_sidelobe_indices);
    let first_sidelobe_meets_target = first_sidelobe_max_error <= config.first_sidelobe_target_db;

    info!(
        "First sidelobe: {} points, max error: {:.3} dB, RMSE: {:.3} dB (target: {:.1} dB, {})",
        first_sidelobe_num_points,
        first_sidelobe_max_error,
        first_sidelobe_rmse,
        config.first_sidelobe_target_db,
        if first_sidelobe_meets_target { "PASS" } else { "FAIL" }
    );

    // Identify outliers
    let outliers = identify_outliers(
        measurements,
        &corrected_predictions,
        config.outlier_threshold_db,
        config,
    );
    let num_outliers = outliers.len();

    if num_outliers > 0 {
        warn!("Found {} outliers (error > {:.1} dB)", num_outliers, config.outlier_threshold_db);
    }

    // Frequency band analysis
    let frequency_band_analysis = analyze_by_frequency_band(
        measurements,
        &corrected_predictions,
        &config.frequency_bands,
    );

    // Angular region analysis
    let angular_region_analysis = analyze_by_angular_region(
        measurements,
        &corrected_predictions,
    );

    // Cross-validation (if requested)
    let cross_validation = if config.num_folds > 1 {
        Some(perform_cross_validation(
            measurements,
            model_predictions,
            config,
        )?)
    } else {
        None
    };

    // Overall assessment
    let meets_accuracy_requirements =
        main_lobe_meets_target && first_sidelobe_meets_target;

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
        corrected_rmse,
        corrected_max_error,
        corrected_r_squared,
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

/// Compute corrected predictions using correction surface
fn compute_corrected_predictions(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
    correction_surface: &CorrectionSurface,
) -> Result<Vec<f64>> {
    let mut corrected = Vec::with_capacity(measurements.len());

    for (meas, &model_pred) in measurements.iter().zip(model_predictions.iter()) {
        let correction = correction_surface.evaluate(
            meas.frequency_mhz,
            meas.e_cone_deg,
            meas.e_clock_deg,
        )?;
        corrected.push(model_pred + correction);
    }

    Ok(corrected)
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

/// Identify outlier points
fn identify_outliers(
    measurements: &[MeasurementPoint],
    corrected_predictions: &[f64],
    threshold_db: f64,
    config: &ValidationConfig,
) -> Vec<OutlierPoint> {
    let mut outliers = Vec::new();

    let (main_lobe_indices, first_sidelobe_indices) =
        classify_points_by_region(measurements, config);

    for (i, (meas, &pred)) in measurements.iter().zip(corrected_predictions.iter()).enumerate() {
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
    corrected_predictions: &[f64],
    bands: &[(f64, f64)],
) -> Vec<FrequencyBandStats> {
    let mut results = Vec::new();

    for &(band_min, band_max) in bands {
        let mut band_measured = Vec::new();
        let mut band_predicted = Vec::new();

        for (meas, &pred) in measurements.iter().zip(corrected_predictions.iter()) {
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
    corrected_predictions: &[f64],
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

        for (meas, &pred) in measurements.iter().zip(corrected_predictions.iter()) {
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

/// Perform k-fold cross-validation
fn perform_cross_validation(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
    config: &ValidationConfig,
) -> Result<CrossValidationResults> {
    let num_folds = config.num_folds;
    let n = measurements.len();

    if num_folds > n {
        return Err(ValidationError::InvalidParameter {
            param: "num_folds".to_string(),
            value: num_folds.to_string(),
            reason: format!("Cannot have more folds ({}) than data points ({})", num_folds, n),
        });
    }

    info!("Performing {}-fold cross-validation", num_folds);

    let fold_size = n / num_folds;
    let mut fold_rmse_values = Vec::new();

    for fold in 0..num_folds {
        let test_start = fold * fold_size;
        let test_end = if fold == num_folds - 1 {
            n
        } else {
            (fold + 1) * fold_size
        };

        // Split into training and test sets
        let mut train_measurements = Vec::new();
        let mut train_predictions = Vec::new();
        let mut test_measurements = Vec::new();
        let mut test_predictions = Vec::new();

        for i in 0..n {
            if i >= test_start && i < test_end {
                test_measurements.push(measurements[i].clone());
                test_predictions.push(model_predictions[i]);
            } else {
                train_measurements.push(measurements[i].clone());
                train_predictions.push(model_predictions[i]);
            }
        }

        // Fit correction surface on training set
        let correction_surface = crate::correction_surface::fit_correction_surface(
            &train_measurements,
            &train_predictions,
            &config.correction_params,
        )?;

        // Evaluate on test set
        let mut test_corrected = Vec::new();
        for (meas, &model_pred) in test_measurements.iter().zip(test_predictions.iter()) {
            let correction = correction_surface.evaluate(
                meas.frequency_mhz,
                meas.e_cone_deg,
                meas.e_clock_deg,
            )?;
            test_corrected.push(model_pred + correction);
        }

        let test_measured: Vec<f64> = test_measurements.iter().map(|m| m.g_over_t_db).collect();
        let fold_rmse = compute_rmse(&test_measured, &test_corrected);

        debug!("Fold {}: RMSE = {:.3} dB ({} test points)", fold + 1, fold_rmse, test_measurements.len());
        fold_rmse_values.push(fold_rmse);
    }

    let mean_rmse = fold_rmse_values.iter().sum::<f64>() / num_folds as f64;
    let variance = fold_rmse_values
        .iter()
        .map(|x| (x - mean_rmse).powi(2))
        .sum::<f64>()
        / num_folds as f64;
    let std_rmse = variance.sqrt();
    let min_rmse = fold_rmse_values.iter().copied().fold(f64::INFINITY, f64::min);
    let max_rmse = fold_rmse_values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    info!(
        "Cross-validation complete: mean RMSE = {:.3} ± {:.3} dB (min: {:.3}, max: {:.3})",
        mean_rmse, std_rmse, min_rmse, max_rmse
    );

    Ok(CrossValidationResults {
        num_folds,
        fold_rmse_values,
        mean_rmse,
        std_rmse,
        min_rmse,
        max_rmse,
    })
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
        s.push_str(&format!("Model-only RMSE:        {:.3} dB\n", self.model_only_rmse));
        s.push_str(&format!("Model-only max error:   {:.3} dB\n", self.model_only_max_error));
        s.push_str(&format!("Model-only R²:          {:.4}\n\n", self.model_only_r_squared));

        s.push_str(&format!("Corrected RMSE:         {:.3} dB\n", self.corrected_rmse));
        s.push_str(&format!("Corrected max error:    {:.3} dB\n", self.corrected_max_error));
        s.push_str(&format!("Corrected R²:           {:.4}\n\n", self.corrected_r_squared));

        s.push_str(&format!("RMSE improvement:       {:.1}%\n", self.rmse_improvement_percent));
        s.push_str(&format!("Max error improvement:  {:.1}%\n\n", self.max_error_improvement_percent));

        s.push_str("Regional Analysis:\n");
        s.push_str("------------------\n");
        s.push_str(&format!(
            "Main lobe ({} points):\n",
            self.main_lobe_num_points
        ));
        s.push_str(&format!("  RMSE:       {:.3} dB\n", self.main_lobe_rmse));
        s.push_str(&format!("  Max error:  {:.3} dB\n", self.main_lobe_max_error));
        s.push_str(&format!(
            "  Target:     ≤1.0 dB [{}]\n\n",
            if self.main_lobe_meets_target { "PASS" } else { "FAIL" }
        ));

        s.push_str(&format!(
            "First sidelobe ({} points):\n",
            self.first_sidelobe_num_points
        ));
        s.push_str(&format!("  RMSE:       {:.3} dB\n", self.first_sidelobe_rmse));
        s.push_str(&format!("  Max error:  {:.3} dB\n", self.first_sidelobe_max_error));
        s.push_str(&format!(
            "  Target:     ≤1.0 dB [{}]\n\n",
            if self.first_sidelobe_meets_target { "PASS" } else { "FAIL" }
        ));

        if self.num_outliers > 0 {
            s.push_str(&format!("Outliers (error >1 dB): {} points\n\n", self.num_outliers));
        }

        if let Some(ref cv) = self.cross_validation {
            s.push_str("Cross-Validation:\n");
            s.push_str("------------------\n");
            s.push_str(&format!("{}-fold cross-validation\n", cv.num_folds));
            s.push_str(&format!("Mean RMSE:  {:.3} ± {:.3} dB\n", cv.mean_rmse, cv.std_rmse));
            s.push_str(&format!("Range:      {:.3} - {:.3} dB\n\n", cv.min_rmse, cv.max_rmse));
        }

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
}
