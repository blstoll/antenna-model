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

use crate::correction_surface::{
    CorrectionSurface, CorrectionSurfaceError, CorrectionSurfaceParams,
};
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
                (100.0, 1000.0),    // VHF/UHF
                (1000.0, 3000.0),   // L/S band
                (3000.0, 12000.0),  // C/X band
                (12000.0, 50000.0), // Ku/Ka/V band
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

/// A fold whose training split could not be fitted.
///
/// Roadmap D22: recorded and reported rather than aborting the run. Since D20 an
/// underdetermined fit is a hard error and a fold trains on `(1 − 1/folds)` of the data, so
/// a dataset can clear the coefficient count on the full set and miss it on a split.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldFailure {
    /// 1-based fold number.
    pub fold: usize,
    /// Size of the training split that could not be fitted.
    pub training_points: usize,
    /// The underlying fitting error, with both point counts.
    pub reason: String,
}

/// Results from k-fold cross-validation.
///
/// **Read `fold_rmse_values`, not just `mean_rmse`** (roadmap D22). The mean alone hid a
/// 100× spread across folds on D14's artifact, which is what led to the fold-assignment
/// defect being found at all. `format_summary` prints every fold for the same reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossValidationResults {
    /// Folds requested.
    pub num_folds: usize,
    /// Per-fold RMSE for each fold that was successfully scored, in fold order. Shorter than
    /// `num_folds` when `failed_folds` is non-empty — so **this vector is dense and its index
    /// is not the fold number**. Pair it with [`Self::scored_fold_numbers`] before labelling.
    pub fold_rmse_values: Vec<f64>,
    /// Folds whose training split could not be fitted. Empty in the normal case.
    #[serde(default)]
    pub failed_folds: Vec<FoldFailure>,
    /// Mean over the folds that were scored.
    ///
    /// `Option` rather than a NaN sentinel: these are serialized into the `--report` JSON,
    /// `serde_json` writes a non-finite `f64` as `null`, and a plain `f64` field cannot read
    /// that back — so a NaN here made the report un-round-trippable
    /// (`sidecar::tests` parses `ValidationReport` back). `None` means no fold scored.
    pub mean_rmse: Option<f64>,
    pub std_rmse: Option<f64>,
    pub min_rmse: Option<f64>,
    pub max_rmse: Option<f64>,
}

impl CrossValidationResults {
    /// True when every requested fold was scored.
    pub fn is_complete(&self) -> bool {
        self.failed_folds.is_empty()
    }

    /// The 1-based fold numbers behind `fold_rmse_values`, in the same order.
    ///
    /// `fold_rmse_values` skips folds that could not be refitted, so its *position* is not its
    /// fold number. Reporting it positionally silently relabels the survivors — with folds 1
    /// and 2 failing, the value printed as "fold 1" is really fold 3.
    pub fn scored_fold_numbers(&self) -> Vec<usize> {
        let failed: std::collections::BTreeSet<usize> =
            self.failed_folds.iter().map(|f| f.fold).collect();
        (1..=self.num_folds)
            .filter(|n| !failed.contains(n))
            .collect()
    }
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
    let corrected_predictions =
        compute_corrected_predictions(measurements, model_predictions, correction_surface)?;

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
        if main_lobe_meets_target {
            "PASS"
        } else {
            "FAIL"
        }
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
        if first_sidelobe_meets_target {
            "PASS"
        } else {
            "FAIL"
        }
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
        warn!(
            "Found {} outliers (error > {:.1} dB)",
            num_outliers, config.outlier_threshold_db
        );
    }

    // Frequency band analysis
    let frequency_band_analysis = analyze_by_frequency_band(
        measurements,
        &corrected_predictions,
        &config.frequency_bands,
    );

    // Angular region analysis
    let angular_region_analysis = analyze_by_angular_region(measurements, &corrected_predictions);

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
        let correction =
            correction_surface.evaluate(meas.frequency_mhz, meas.e_cone_deg, meas.e_clock_deg)?;
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

    for (i, (meas, &pred)) in measurements
        .iter()
        .zip(corrected_predictions.iter())
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
            reason: format!(
                "Cannot have more folds ({}) than data points ({})",
                num_folds, n
            ),
        });
    }

    info!("Performing {}-fold cross-validation", num_folds);

    // Forced unconditionally rather than relying on the caller passing params that
    // already have it: nested cross-validation under a CV fold is never wanted.
    let refit_params = config.correction_params.without_nested_cross_validation();

    let mut fold_rmse_values = Vec::new();
    let mut failed_folds: Vec<FoldFailure> = Vec::new();

    for fold in 0..num_folds {
        // Fold assignment comes from `correction_surface::is_held_out` — the crate's single
        // definition, shared with the cross-validation inside `fit_correction_surface`.
        // Restating `i % num_folds == fold` here is what let the two implementations drift
        // in the first place (roadmap D22); that function's docs carry the rationale and
        // the measurements.
        let mut train_measurements = Vec::new();
        let mut train_predictions = Vec::new();
        let mut test_measurements = Vec::new();
        let mut test_predictions = Vec::new();

        for i in 0..n {
            if crate::correction_surface::is_held_out(i, fold, num_folds) {
                test_measurements.push(measurements[i].clone());
                test_predictions.push(model_predictions[i]);
            } else {
                train_measurements.push(measurements[i].clone());
                train_predictions.push(model_predictions[i]);
            }
        }

        // Fit correction surface on training set, with the caller's knot counts,
        // regularization and spline order — the fold must score the same model family as
        // the artifact being blessed — but never with nested cross-validation.
        //
        // **A fold that cannot be fitted is recorded, not fatal** (roadmap D22, decided
        // 2026-08-03). It used to abort the whole run, which made `--validate` *destructive*:
        // since D20 an underdetermined fit is a hard error, and a fold trains on
        // `(1 − 1/folds)` of the data, so a dataset can clear the coefficient count on the
        // full set and miss it on a training split. The run then failed before the artifact
        // was written — `--validate` removed an artifact that the same command without it
        // produces. A fold failure is real information and is reported as such; it is not a
        // reason to withhold an artifact whose own fit succeeded.
        let refit = crate::correction_surface::fit_correction_surface(
            &train_measurements,
            &train_predictions,
            &refit_params,
        );
        let correction_surface = match refit {
            Ok(surface) => surface,
            Err(e) => {
                // Names the fold and BOTH point counts. Without them the most likely
                // failure — `UnderdeterminedFit` — surfaces as a bare complaint about a
                // point count the caller never supplied, on a dataset whose full-set fit
                // had just succeeded.
                let reason = format!(
                    "fold {}/{} could not refit on its training split of {} points (the full \
                     set has {n}, and its own fit succeeded — cross-validation trains on \
                     {:.0}% of it): {e}",
                    fold + 1,
                    num_folds,
                    train_measurements.len(),
                    100.0 * (1.0 - 1.0 / num_folds as f64),
                );
                warn!("{reason}");
                failed_folds.push(FoldFailure {
                    fold: fold + 1,
                    training_points: train_measurements.len(),
                    reason,
                });
                continue;
            }
        };

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

        debug!(
            "Fold {}: RMSE = {:.3} dB ({} test points)",
            fold + 1,
            fold_rmse,
            test_measurements.len()
        );
        fold_rmse_values.push(fold_rmse);
    }

    // Averaged over the folds that were actually SCORED, not over `num_folds`. Dividing by
    // the requested count would silently pull the mean toward zero for every fold that
    // failed to fit — reporting a *better* cross-validation the less of it ran.
    let scored = fold_rmse_values.len();
    let (mean_rmse, std_rmse, min_rmse, max_rmse) = if scored == 0 {
        (None, None, None, None)
    } else {
        let mean = fold_rmse_values.iter().sum::<f64>() / scored as f64;
        let variance = fold_rmse_values
            .iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f64>()
            / scored as f64;
        (
            Some(mean),
            Some(variance.sqrt()),
            Some(
                fold_rmse_values
                    .iter()
                    .copied()
                    .fold(f64::INFINITY, f64::min),
            ),
            Some(
                fold_rmse_values
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max),
            ),
        )
    };

    match (
        failed_folds.is_empty(),
        mean_rmse,
        std_rmse,
        min_rmse,
        max_rmse,
    ) {
        (true, Some(mean), Some(std), Some(min), Some(max)) => info!(
            "Cross-validation complete: mean RMSE = {mean:.3} ± {std:.3} dB \
             (min: {min:.3}, max: {max:.3})"
        ),
        (false, Some(mean), Some(std), _, _) => warn!(
            "Cross-validation INCOMPLETE: {scored}/{num_folds} folds scored (mean RMSE = \
             {mean:.3} ± {std:.3} dB over those); {} fold(s) could not refit on their \
             training split. The artifact is still written — its own fit succeeded — but \
             this figure describes only the folds that ran.",
            failed_folds.len()
        ),
        _ => warn!(
            "Cross-validation produced NO figure: none of the {num_folds} folds could refit \
             on its training split. The artifact is still written — its own fit on the full \
             dataset succeeded."
        ),
    }

    Ok(CrossValidationResults {
        num_folds,
        fold_rmse_values,
        failed_folds,
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
            "Corrected RMSE:         {:.3} dB\n",
            self.corrected_rmse
        ));
        s.push_str(&format!(
            "Corrected max error:    {:.3} dB\n",
            self.corrected_max_error
        ));
        s.push_str(&format!(
            "Corrected R²:           {:.4}\n\n",
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
                cv.num_folds, cv.num_folds
            ));
            match (cv.mean_rmse, cv.std_rmse) {
                (Some(mean), Some(std)) => {
                    s.push_str(&format!("Mean RMSE:  {mean:.3} ± {std:.3} dB\n"))
                }
                _ => s.push_str("Mean RMSE:  n/a (no fold could be scored)\n"),
            }
            match (cv.min_rmse, cv.max_rmse) {
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
            // `fold_rmse_values` is dense and skips folds that could not refit, so with folds
            // 1 and 2 failing, printing positionally would report fold 3's RMSE as "fold 1".
            if !cv.fold_rmse_values.is_empty() {
                s.push_str("Per fold:   ");
                for (i, (fold_no, rmse)) in cv
                    .scored_fold_numbers()
                    .iter()
                    .zip(cv.fold_rmse_values.iter())
                    .enumerate()
                {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(&format!("#{fold_no} {rmse:.3}"));
                }
                s.push_str(" dB\n");
            }

            if !cv.is_complete() {
                s.push_str(&format!(
                    "\n⚠ INCOMPLETE: {} of {} folds could not refit on their training \
                     split, so the figures above cover only the {} that ran. The artifact \
                     is still written — its own fit succeeded on the full dataset.\n",
                    cv.failed_folds.len(),
                    cv.num_folds,
                    cv.fold_rmse_values.len()
                ));
                for failure in &cv.failed_folds {
                    s.push_str(&format!("    {}\n", failure.reason));
                }
            }
            s.push('\n');
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

    /// The parameters `calibrate` actually fits the shipped artifact with.
    fn artifact_params() -> CorrectionSurfaceParams {
        CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 4,
            num_knots_econe: 6,
            num_knots_eclock: 8,
            regularization: 1e-3,
            adaptive_knots: true,
            cross_validation_folds: 0,
            min_knot_spacing_frequency: 50.0,
            min_knot_spacing_econe: 2.0,
            min_knot_spacing_eclock: 5.0,
        }
    }

    fn config_with(correction_params: CorrectionSurfaceParams) -> ValidationConfig {
        ValidationConfig {
            num_folds: 5,
            main_lobe_beamwidths: 1.0,
            first_sidelobe_max_deg: 5.0,
            frequency_bands: vec![],
            main_lobe_target_db: 1.0,
            first_sidelobe_target_db: 1.0,
            outlier_threshold_db: 3.0,
            correction_params,
        }
    }

    fn run_cv(config: &ValidationConfig) -> Result<CrossValidationResults> {
        let (points, predictions) = cv_fixture();
        let surface = crate::correction_surface::fit_correction_surface(
            &points,
            &predictions,
            &artifact_params(),
        )?;
        let report = validate_calibration(&points, &predictions, &surface, config)?;
        Ok(report
            .cross_validation
            .expect("num_folds > 1, so cross-validation must have run"))
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

        let surface = crate::correction_surface::fit_correction_surface(
            &points,
            &predictions,
            &artifact_params(),
        )
        .expect("the whole set must fit — that is the premise of this test");

        let report = validate_calibration(
            &points,
            &predictions,
            &surface,
            &config_with(artifact_params()),
        )
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
            cv.failed_folds.len() + cv.fold_rmse_values.len(),
            cv.num_folds,
            "every requested fold must be accounted for, scored or failed"
        );

        let reasons = cv
            .failed_folds
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
    /// definition, which both cross-validation implementations use. An earlier version of
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
        let cv = CrossValidationResults {
            num_folds: 5,
            fold_rmse_values: vec![0.30, 0.50, 0.40],
            failed_folds: vec![
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
            mean_rmse: Some(0.40),
            std_rmse: Some(0.0816),
            min_rmse: Some(0.30),
            max_rmse: Some(0.50),
        };

        assert_eq!(
            cv.scored_fold_numbers(),
            vec![2, 3, 5],
            "folds 1 and 4 failed, so the three scored values belong to folds 2, 3 and 5"
        );
        assert_eq!(cv.scored_fold_numbers().len(), cv.fold_rmse_values.len());

        let report = ValidationReport {
            cross_validation: Some(cv),
            ..minimal_report()
        };
        let summary = report.format_summary();
        assert!(
            summary.contains("#2 0.300") && summary.contains("#5 0.400"),
            "each fold RMSE must be labelled with its own fold number; got:\n{summary}"
        );
        assert!(
            !summary.contains("#1 0.300"),
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
            corrected_rmse: 0.0,
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

    /// The behavioural counterpart: `perform_cross_validation` itself must produce folds that
    /// all score the same *kind* of question on a grid-ordered fixture.
    ///
    /// The test above proves the assignment function is strided; this proves cross-validation
    /// actually routes through it. Under the pre-D22 contiguous slicing the edge folds
    /// extrapolated a whole frequency slab and came out orders of magnitude worse than the
    /// interior ones — 89× between best and worst on D14's artifact. Requiring the spread to
    /// stay inside one order of magnitude distinguishes "every fold interpolates" from "these
    /// numbers all happen to be small".
    #[test]
    fn cross_validation_folds_all_score_comparably_on_a_grid_ordered_fixture() {
        let cv = run_cv(&config_with(artifact_params())).expect("cv on the grid-ordered fixture");
        assert!(
            cv.is_complete(),
            "premise broken: this fixture is sized so every fold refits"
        );

        let best = cv.min_rmse.expect("a scored fold has a min");
        let worst = cv.max_rmse.expect("a scored fold has a max");
        assert!(
            worst < 10.0 * best,
            "fold RMSEs show two populations (worst {worst:.4} dB vs best {best:.4} dB, folds \
             {:?}) — the signature of contiguous folds holding out a whole axis slab. Check \
             that perform_cross_validation still routes through \
             correction_surface::is_held_out.",
            cv.fold_rmse_values
        );
    }

    /// The fold refit must fit the *caller's* model family. Two configs that differ only
    /// in knot counts and regularization must therefore produce different CV numbers —
    /// if the refit fell back to `CorrectionSurfaceParams::default()` (the pre-D10 bug)
    /// both would fit the identical surface and report the identical RMSE.
    #[test]
    fn fold_refit_uses_caller_knot_counts_and_regularization() {
        let sparse = run_cv(&config_with(artifact_params())).expect("sparse config");
        let dense = run_cv(&config_with(CorrectionSurfaceParams {
            num_knots_frequency: 8,
            num_knots_econe: 8,
            num_knots_eclock: 12,
            regularization: 1e-6,
            ..artifact_params()
        }))
        .expect("dense config");

        let sparse_mean = sparse
            .mean_rmse
            .expect("sparse config must score every fold");
        let dense_mean = dense.mean_rmse.expect("dense config must score every fold");
        assert!(
            (sparse_mean - dense_mean).abs() > 1e-3,
            "knot counts and regularization did not reach the fold refit: \
             sparse={sparse_mean:.6} dB, dense={dense_mean:.6} dB"
        );
    }

    /// Spline order reaches the refit too, proven through the fitter's data minimum:
    /// order 6 needs `(6+1)³ = 343` points, and a 5-fold split of 256 trains on 205.
    /// Under the pre-D10 default (order 4, minimum 125) this would have succeeded.
    #[test]
    fn fold_refit_uses_caller_spline_order() {
        // Since D22 a fold that cannot refit is recorded rather than fatal, so the evidence
        // that the caller's order reached the refit is in the recorded reason, not in an
        // `Err` from the run.
        let cv = run_cv(&config_with(CorrectionSurfaceParams {
            spline_order: 6,
            ..artifact_params()
        }))
        .expect("a fold that cannot refit no longer fails the run (roadmap D22)");

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
            .failed_folds
            .iter()
            .map(|f| f.reason.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            reasons.contains("864"),
            "expected the caller's spline order to set the coefficient count, got: {reasons}"
        );
    }

    /// Nested cross-validation must not run whatever the caller passes. Before D10 the
    /// `cross_validation_folds: 5` case did not merely report a different number — each
    /// level re-entered the fitter with the same fold count and recursed until the
    /// training set fell under the minimum, failing the whole run.
    #[test]
    fn no_nested_cross_validation_under_any_caller_configuration() {
        let without = run_cv(&config_with(artifact_params())).expect("folds = 0");
        let with = run_cv(&config_with(CorrectionSurfaceParams {
            cross_validation_folds: 5,
            ..artifact_params()
        }))
        .expect("folds = 5 must not recurse");

        assert_eq!(
            without.mean_rmse, with.mean_rmse,
            "the caller's cross_validation_folds changed the fold refit"
        );
    }

    /// `--cv-folds N` is threaded through `num_folds` and must be visible in the report.
    #[test]
    fn num_folds_controls_the_reported_fold_count() {
        for folds in [3usize, 5, 8] {
            let results = run_cv(&ValidationConfig {
                num_folds: folds,
                ..config_with(artifact_params())
            })
            .unwrap_or_else(|e| panic!("{folds} folds: {e}"));

            assert_eq!(results.num_folds, folds);
            assert_eq!(results.fold_rmse_values.len(), folds);
        }
    }
}
