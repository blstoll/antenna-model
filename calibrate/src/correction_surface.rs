//! Correction Surface Fitting Module
//!
//! This module implements 3D B-spline surface fitting to model residuals between
//! measured antenna performance and physical optics predictions. The correction
//! surface captures systematic errors that the physics-based model cannot explain.
//!
//! # Overview
//!
//! The correction surface is a 3D B-spline interpolation over:
//! - Frequency (MHz)
//! - E-cone angle (degrees)
//! - E-clock angle (degrees)
//!
//! The fitting process:
//! 1. Compute residuals: Δ = measured_G/T - model_G/T
//! 2. Select knots adaptively based on measurement density
//! 3. Fit cubic B-spline coefficients using least squares
//! 4. Validate with cross-validation to prevent overfitting
//!
//! # Example
//!
//! ```no_run
//! use calibrate::correction_surface::{fit_correction_surface, CorrectionSurfaceParams};
//! use calibrate::parser::MeasurementPoint;
//!
//! let measurements = vec![/* ... */];
//! let model_predictions = vec![/* ... */];
//! let params = CorrectionSurfaceParams::default();
//!
//! let surface = fit_correction_surface(&measurements, &model_predictions, &params)?;
//! let correction = surface.evaluate(8400.0, 10.0, 45.0)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::parser::MeasurementPoint;
use ndarray::{Array1, Array2};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum CorrectionSurfaceError {
    #[error("Insufficient data for fitting: need at least {min_required}, got {actual}")]
    InsufficientData { min_required: usize, actual: usize },

    #[error("Dimension mismatch: measurements ({measurements}) != predictions ({predictions})")]
    DimensionMismatch {
        measurements: usize,
        predictions: usize,
    },

    #[error("Invalid knot vector: {reason}")]
    InvalidKnotVector { reason: String },

    #[error("Singular matrix in least squares fitting: {reason}")]
    SingularMatrix { reason: String },

    #[error("Invalid parameter value: {param} = {value} ({reason})")]
    InvalidParameter {
        param: String,
        value: f64,
        reason: String,
    },

    #[error("Interpolation failed: {reason}")]
    InterpolationError { reason: String },

    #[error("Cross-validation failed: {reason}")]
    CrossValidationError { reason: String },
}

pub type Result<T> = std::result::Result<T, CorrectionSurfaceError>;

// ============================================================================
// Data Structures
// ============================================================================

/// Parameters for correction surface fitting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionSurfaceParams {
    /// Spline order (degree + 1). Default is 4 for cubic splines.
    pub spline_order: usize,

    /// Target number of knots for frequency dimension
    pub num_knots_frequency: usize,

    /// Target number of knots for E-cone dimension
    pub num_knots_econe: usize,

    /// Target number of knots for E-clock dimension
    pub num_knots_eclock: usize,

    /// Regularization parameter (lambda). Higher values = smoother surface.
    /// Set to 0.0 to disable regularization.
    pub regularization: f64,

    /// Use adaptive knot placement based on measurement density
    pub adaptive_knots: bool,

    /// Number of folds for cross-validation (0 to skip)
    pub cross_validation_folds: usize,

    /// Minimum spacing between knots (prevents overfitting)
    pub min_knot_spacing_frequency: f64, // MHz
    pub min_knot_spacing_econe: f64,  // degrees
    pub min_knot_spacing_eclock: f64, // degrees
}

impl Default for CorrectionSurfaceParams {
    fn default() -> Self {
        Self {
            spline_order: 4, // cubic splines
            num_knots_frequency: 8,
            num_knots_econe: 8,
            num_knots_eclock: 12, // More for 360-degree coverage
            regularization: 1e-6,
            adaptive_knots: true,
            cross_validation_folds: 5,
            min_knot_spacing_frequency: 50.0, // 50 MHz
            min_knot_spacing_econe: 2.0,      // 2 degrees
            min_knot_spacing_eclock: 5.0,     // 5 degrees
        }
    }
}

/// Represents a residual data point (measurement - model prediction)
#[derive(Debug, Clone)]
pub struct ResidualPoint {
    pub frequency_mhz: f64,
    pub e_cone_deg: f64,
    pub e_clock_deg: f64,
    pub residual_db: f64,
}

/// A fitted 3D B-spline correction surface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionSurface {
    /// B-spline coefficients (flattened 3D array)
    /// Indexing: coeff[i_freq + n_freq * (i_cone + n_cone * i_clock)]
    pub coefficients: Vec<f64>,

    /// Shape: [n_frequency, n_cone, n_clock]
    pub shape: [usize; 3],

    /// Knot vectors for each dimension
    pub knots_frequency: Vec<f64>,
    pub knots_econe: Vec<f64>,
    pub knots_eclock: Vec<f64>,

    /// Spline order (degree + 1)
    pub spline_order: usize,

    /// Fitting statistics
    pub fit_stats: FitStatistics,
}

/// Statistics about the fitted correction surface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitStatistics {
    /// Number of data points used for fitting
    pub num_points: usize,

    /// Root mean squared error (RMSE) of fit
    pub rmse_db: f64,

    /// Maximum absolute residual after correction
    pub max_residual_db: f64,

    /// R-squared value (goodness of fit)
    pub r_squared: f64,

    /// Cross-validation RMSE (if performed)
    pub cross_validation_rmse: Option<f64>,

    /// Improvement over uncorrected model (% reduction in RMSE)
    pub improvement_percent: f64,
}

// ============================================================================
// Main API Functions
// ============================================================================

/// Compute residuals between measurements and model predictions
///
/// # Arguments
/// * `measurements` - Measured data points with G/T values
/// * `model_predictions` - Model predictions (G/T in dB/K) for each measurement point
///
/// # Returns
/// Vector of residual points (measured - predicted)
pub fn compute_residuals(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
) -> Result<Vec<ResidualPoint>> {
    if measurements.len() != model_predictions.len() {
        return Err(CorrectionSurfaceError::DimensionMismatch {
            measurements: measurements.len(),
            predictions: model_predictions.len(),
        });
    }

    let residuals: Vec<ResidualPoint> = measurements
        .iter()
        .zip(model_predictions.iter())
        .map(|(meas, pred)| ResidualPoint {
            frequency_mhz: meas.frequency_mhz,
            e_cone_deg: meas.e_cone_deg,
            e_clock_deg: meas.e_clock_deg,
            residual_db: meas.g_over_t_db - pred,
        })
        .collect();

    debug!(
        "Computed {} residuals. Mean residual: {:.3} dB, Std: {:.3} dB",
        residuals.len(),
        mean_residual(&residuals),
        std_residual(&residuals)
    );

    Ok(residuals)
}

/// Fit a 3D B-spline correction surface to residuals
///
/// This is the main entry point for correction surface fitting.
///
/// # Arguments
/// * `measurements` - Original measurement data
/// * `model_predictions` - Model predictions for each measurement
/// * `params` - Fitting parameters
///
/// # Returns
/// A fitted CorrectionSurface that can be used to evaluate corrections
pub fn fit_correction_surface(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
    params: &CorrectionSurfaceParams,
) -> Result<CorrectionSurface> {
    info!(
        "Starting correction surface fitting with {} data points",
        measurements.len()
    );

    // Validate inputs
    validate_fitting_inputs(measurements, model_predictions, params)?;

    // Compute residuals
    let residuals = compute_residuals(measurements, model_predictions)?;

    // Compute initial RMSE (before correction)
    let initial_rmse = compute_rmse(&residuals.iter().map(|r| r.residual_db).collect::<Vec<_>>());
    info!("Initial RMSE (model only): {:.3} dB", initial_rmse);

    // Generate knot vectors
    let knots_freq = generate_knot_vector(
        &residuals
            .iter()
            .map(|r| r.frequency_mhz)
            .collect::<Vec<_>>(),
        params.num_knots_frequency,
        params.spline_order,
        params.adaptive_knots,
        params.min_knot_spacing_frequency,
    )?;

    let knots_cone = generate_knot_vector(
        &residuals.iter().map(|r| r.e_cone_deg).collect::<Vec<_>>(),
        params.num_knots_econe,
        params.spline_order,
        params.adaptive_knots,
        params.min_knot_spacing_econe,
    )?;

    let knots_clock = generate_knot_vector(
        &residuals.iter().map(|r| r.e_clock_deg).collect::<Vec<_>>(),
        params.num_knots_eclock,
        params.spline_order,
        params.adaptive_knots,
        params.min_knot_spacing_eclock,
    )?;

    info!(
        "Generated knot vectors: freq={}, cone={}, clock={}",
        knots_freq.len(),
        knots_cone.len(),
        knots_clock.len()
    );

    // Compute number of B-spline basis functions
    let n_freq = knots_freq.len() - params.spline_order;
    let n_cone = knots_cone.len() - params.spline_order;
    let n_clock = knots_clock.len() - params.spline_order;

    debug!(
        "Number of basis functions: freq={}, cone={}, clock={} (total: {})",
        n_freq,
        n_cone,
        n_clock,
        n_freq * n_cone * n_clock
    );

    // Build design matrix and solve least squares
    let coefficients = fit_bspline_coefficients(
        &residuals,
        &knots_freq,
        &knots_cone,
        &knots_clock,
        params.spline_order,
        params.regularization,
    )?;

    // Create the surface
    let surface = CorrectionSurface {
        coefficients: coefficients.clone(),
        shape: [n_freq, n_cone, n_clock],
        knots_frequency: knots_freq,
        knots_econe: knots_cone,
        knots_eclock: knots_clock,
        spline_order: params.spline_order,
        fit_stats: FitStatistics {
            num_points: residuals.len(),
            rmse_db: 0.0, // Will be computed below
            max_residual_db: 0.0,
            r_squared: 0.0,
            cross_validation_rmse: None,
            improvement_percent: 0.0,
        },
    };

    // Compute fit statistics
    let fit_stats = compute_fit_statistics(&surface, &residuals, initial_rmse)?;

    // Update the surface with statistics
    let surface = CorrectionSurface {
        fit_stats,
        ..surface
    };

    info!(
        "Correction surface fitted successfully. RMSE: {:.3} dB, R²: {:.3}, Improvement: {:.1}%",
        surface.fit_stats.rmse_db,
        surface.fit_stats.r_squared,
        surface.fit_stats.improvement_percent
    );

    // Cross-validation if requested
    if params.cross_validation_folds > 1 {
        info!(
            "Running {}-fold cross-validation...",
            params.cross_validation_folds
        );
        let cv_rmse = cross_validate(&residuals, params)?;
        info!("Cross-validation RMSE: {:.3} dB", cv_rmse);

        let surface = CorrectionSurface {
            fit_stats: FitStatistics {
                cross_validation_rmse: Some(cv_rmse),
                ..surface.fit_stats
            },
            ..surface
        };

        Ok(surface)
    } else {
        Ok(surface)
    }
}

// ============================================================================
// B-Spline Basis Functions
// ============================================================================

/// Evaluate a single B-spline basis function using Cox-de Boor recursion
///
/// # Arguments
/// * `i` - Basis function index
/// * `k` - Order (degree + 1)
/// * `t` - Evaluation point
/// * `knots` - Knot vector
///
/// # Returns
/// Value of B_{i,k}(t)
fn bspline_basis(i: usize, k: usize, t: f64, knots: &[f64]) -> f64 {
    if k == 1 {
        // Base case: characteristic function
        if i < knots.len() - 1 && t >= knots[i] && t < knots[i + 1] {
            return 1.0;
        }
        // Special case for right endpoint
        if i == knots.len() - 2 && t == knots[i + 1] {
            return 1.0;
        }
        return 0.0;
    }

    // Recursive case
    let mut left = 0.0;
    let mut right = 0.0;

    // Left term
    if i + k <= knots.len() {
        let denom = knots[i + k - 1] - knots[i];
        if denom.abs() > 1e-10 {
            left = (t - knots[i]) / denom * bspline_basis(i, k - 1, t, knots);
        }
    }

    // Right term
    if i + 1 < knots.len() && i + k <= knots.len() {
        let denom = knots[i + k] - knots[i + 1];
        if denom.abs() > 1e-10 {
            right = (knots[i + k] - t) / denom * bspline_basis(i + 1, k - 1, t, knots);
        }
    }

    left + right
}

/// Evaluate all non-zero B-spline basis functions at a point
///
/// Returns a vector of (index, value) pairs for non-zero basis functions
fn evaluate_basis_functions(t: f64, knots: &[f64], order: usize) -> Vec<(usize, f64)> {
    let n_basis = knots.len() - order;
    let mut results = Vec::new();

    // Find the knot interval containing t
    let interval = find_knot_interval(t, knots, order);

    // Only evaluate basis functions that can be non-zero at t
    // For order k, at most k basis functions are non-zero at any point
    let start = interval.saturating_sub(order - 1);
    let end = (interval + 1).min(n_basis);

    for i in start..end {
        let value = bspline_basis(i, order, t, knots);
        if value.abs() > 1e-12 {
            results.push((i, value));
        }
    }

    results
}

/// Find the knot interval containing t
fn find_knot_interval(t: f64, knots: &[f64], order: usize) -> usize {
    let n = knots.len() - order;

    // Handle edge cases
    if t <= knots[order - 1] {
        return order - 1;
    }
    if t >= knots[n] {
        return n - 1;
    }

    // Binary search
    let mut left = order - 1;
    let mut right = n;

    while right - left > 1 {
        let mid = (left + right) / 2;
        if t < knots[mid] {
            right = mid;
        } else {
            left = mid;
        }
    }

    left
}

// ============================================================================
// Knot Vector Generation
// ============================================================================

/// Generate a knot vector for a given dimension
///
/// # Arguments
/// * `data` - Data points in this dimension
/// * `num_knots` - Target number of internal knots
/// * `order` - Spline order
/// * `adaptive` - Use adaptive placement based on data density
/// * `min_spacing` - Minimum spacing between knots
fn generate_knot_vector(
    data: &[f64],
    num_knots: usize,
    order: usize,
    adaptive: bool,
    min_spacing: f64,
) -> Result<Vec<f64>> {
    if data.is_empty() {
        return Err(CorrectionSurfaceError::InsufficientData {
            min_required: 1,
            actual: 0,
        });
    }

    let mut sorted_data = data.to_vec();
    sorted_data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let min_val = sorted_data[0];
    let max_val = sorted_data[sorted_data.len() - 1];

    if max_val - min_val < min_spacing {
        return Err(CorrectionSurfaceError::InvalidParameter {
            param: "data_range".to_string(),
            value: max_val - min_val,
            reason: format!("Data range too small (< {})", min_spacing),
        });
    }

    let mut internal_knots = if adaptive {
        generate_adaptive_knots(&sorted_data, num_knots, min_spacing)?
    } else {
        generate_uniform_knots(min_val, max_val, num_knots)
    };

    // Ensure minimum spacing
    internal_knots = enforce_min_spacing(&internal_knots, min_spacing);

    // Build full knot vector with repeated end knots
    let mut knots = vec![min_val; order];
    knots.extend_from_slice(&internal_knots);
    knots.extend(vec![max_val; order]);

    validate_knot_vector(&knots, order)?;

    Ok(knots)
}

/// Generate uniformly spaced internal knots
fn generate_uniform_knots(min: f64, max: f64, num_knots: usize) -> Vec<f64> {
    if num_knots == 0 {
        return vec![];
    }

    let step = (max - min) / (num_knots + 1) as f64;
    (1..=num_knots).map(|i| min + i as f64 * step).collect()
}

/// Generate adaptive knots based on data density
fn generate_adaptive_knots(
    sorted_data: &[f64],
    num_knots: usize,
    min_spacing: f64,
) -> Result<Vec<f64>> {
    if num_knots == 0 {
        return Ok(vec![]);
    }

    // Use quantile-based placement for adaptive knots
    let n = sorted_data.len();
    let step = n / (num_knots + 1);

    let mut knots = Vec::new();
    for i in 1..=num_knots {
        let idx = (i * step).min(n - 1);
        knots.push(sorted_data[idx]);
    }

    // Remove duplicates and enforce minimum spacing
    knots.dedup_by(|a, b| (*b - *a).abs() < min_spacing);

    Ok(knots)
}

/// Enforce minimum spacing between knots
fn enforce_min_spacing(knots: &[f64], min_spacing: f64) -> Vec<f64> {
    if knots.is_empty() {
        return vec![];
    }

    let mut result = vec![knots[0]];

    for &k in &knots[1..] {
        if k - result[result.len() - 1] >= min_spacing {
            result.push(k);
        }
    }

    result
}

/// Validate that a knot vector is valid for B-spline interpolation
fn validate_knot_vector(knots: &[f64], order: usize) -> Result<()> {
    if knots.len() < 2 * order {
        return Err(CorrectionSurfaceError::InvalidKnotVector {
            reason: format!(
                "Knot vector too short: {} knots for order {}",
                knots.len(),
                order
            ),
        });
    }

    // Check non-decreasing
    for i in 1..knots.len() {
        if knots[i] < knots[i - 1] {
            return Err(CorrectionSurfaceError::InvalidKnotVector {
                reason: format!(
                    "Knot vector not non-decreasing: knots[{}]={} > knots[{}]={}",
                    i - 1,
                    knots[i - 1],
                    i,
                    knots[i]
                ),
            });
        }
    }

    Ok(())
}

// ============================================================================
// Least Squares Fitting
// ============================================================================

/// Accumulate the normal equations `(B^T B + λI, B^T r)` for the tensor-product basis.
///
/// A B-spline basis function is non-zero only over `order` consecutive knot spans, so
/// each residual point activates at most `order` basis functions per dimension — `order^3`
/// of the `n_coeff` columns of its design-matrix row. Accumulating `B^T B` from those
/// active entries costs `O(n_data · order^6)` instead of the `O(n_data · n_coeff^2)` a dense
/// `B^T B` product would; the design matrix is never materialized.
fn accumulate_normal_equations(
    residuals: &[ResidualPoint],
    knots_freq: &[f64],
    knots_cone: &[f64],
    knots_clock: &[f64],
    order: usize,
    regularization: f64,
) -> (Array2<f64>, Array1<f64>) {
    let n_freq = knots_freq.len() - order;
    let n_cone = knots_cone.len() - order;
    let n_clock = knots_clock.len() - order;
    let n_coeff = n_freq * n_cone * n_clock;

    let mut normal_matrix = Array2::<f64>::zeros((n_coeff, n_coeff));
    let mut btr = Array1::<f64>::zeros(n_coeff);

    let mut active: Vec<(usize, f64)> = Vec::with_capacity(order * order * order);

    for res in residuals {
        let basis_freq = evaluate_basis_functions(res.frequency_mhz, knots_freq, order);
        let basis_cone = evaluate_basis_functions(res.e_cone_deg, knots_cone, order);
        let basis_clock = evaluate_basis_functions(res.e_clock_deg, knots_clock, order);

        // The non-zero entries of this point's design-matrix row.
        active.clear();
        for &(if_, vf) in &basis_freq {
            for &(ic, vc) in &basis_cone {
                for &(ik, vk) in &basis_clock {
                    let idx = if_ + n_freq * (ic + n_cone * ik);
                    active.push((idx, vf * vc * vk));
                }
            }
        }

        // Rank-1 update restricted to the active columns.
        for &(ia, va) in &active {
            btr[ia] += va * res.residual_db;
            for &(ib, vb) in &active {
                normal_matrix[[ia, ib]] += va * vb;
            }
        }
    }

    if regularization > 0.0 {
        for i in 0..n_coeff {
            normal_matrix[[i, i]] += regularization;
        }
    }

    (normal_matrix, btr)
}

/// Solve `A x = b` for symmetric positive-definite `A` by Cholesky factorization.
///
/// `a` is consumed as scratch: it is overwritten with its lower-triangular factor `L`
/// where `A = L L^T`. Returns `None` if `A` is not positive definite — for the normal
/// equations here that means the basis is rank-deficient over the supplied data, which
/// regularization (λ > 0) is what prevents.
fn cholesky_solve(a: &mut Array2<f64>, b: &Array1<f64>) -> Option<Array1<f64>> {
    /// Dot product over four independent accumulators.
    ///
    /// Floating-point addition is not associative, so a single-accumulator loop forces the
    /// compiler to keep the adds in order and it cannot vectorize. Splitting the sum into
    /// four independent chains lets it emit SIMD/FMA, which is worth several times the
    /// throughput in the O(n^3) factorization below. The regrouping perturbs results only
    /// at the ulp level.
    #[inline]
    fn dot(x: &[f64], y: &[f64]) -> f64 {
        let n = x.len();
        let y = &y[..n]; // Prove the lengths match once, so the loop is bounds-check free.
        let (mut s0, mut s1, mut s2, mut s3) = (0.0, 0.0, 0.0, 0.0);

        let tail = n % 4;
        let mut k = 0;
        while k < n - tail {
            s0 += x[k] * y[k];
            s1 += x[k + 1] * y[k + 1];
            s2 += x[k + 2] * y[k + 2];
            s3 += x[k + 3] * y[k + 3];
            k += 4;
        }
        let mut sum = (s0 + s2) + (s1 + s3);
        while k < n {
            sum += x[k] * y[k];
            k += 1;
        }
        sum
    }

    let n = b.len();
    debug_assert_eq!(a.dim(), (n, n));

    // Row-major contiguous storage: rows of `a` are then adjacent in memory, which is what
    // makes the inner dot products above vectorizable.
    let a = a
        .as_slice_mut()
        .expect("normal matrix is contiguous and row-major");

    // Factor A = L L^T in place, writing only the lower triangle.
    for j in 0..n {
        let row_j = &a[j * n..j * n + n];
        let diag = row_j[j] - dot(&row_j[..j], &row_j[..j]);

        // NaN is checked explicitly; `diag <= 0.0` alone would let it through.
        if diag.is_nan() || diag <= 0.0 {
            return None;
        }
        let ljj = diag.sqrt();
        a[j * n + j] = ljj;

        for i in (j + 1)..n {
            // Row j is fully computed and i > j, so the rows are disjoint: split the buffer
            // to borrow row j immutably while writing row i.
            let (head, tail) = a.split_at_mut(i * n);
            let row_j = &head[j * n..j * n + j];
            let row_i = &mut tail[..n];
            row_i[j] = (row_i[j] - dot(&row_i[..j], row_j)) / ljj;
        }
    }

    // Forward substitution: L y = b.
    let mut x = b.to_vec();
    for i in 0..n {
        let row_i = &a[i * n..i * n + n];
        x[i] = (x[i] - dot(&row_i[..i], &x[..i])) / row_i[i];
    }

    // Back substitution: L^T x = y. Walks column i of L, so it is strided rather than
    // contiguous — but this is O(n^2) and negligible beside the factorization.
    for i in (0..n).rev() {
        let mut sum = x[i];
        for k in (i + 1)..n {
            sum -= a[k * n + i] * x[k];
        }
        x[i] = sum / a[i * n + i];
    }

    Some(Array1::from_vec(x))
}

/// Fit B-spline coefficients using least squares
///
/// Solves the system: (B^T B + λI) c = B^T r
/// where B is the design matrix, r is the residual vector, and λ is regularization
fn fit_bspline_coefficients(
    residuals: &[ResidualPoint],
    knots_freq: &[f64],
    knots_cone: &[f64],
    knots_clock: &[f64],
    order: usize,
    regularization: f64,
) -> Result<Vec<f64>> {
    let n_coeff =
        (knots_freq.len() - order) * (knots_cone.len() - order) * (knots_clock.len() - order);

    info!(
        "Accumulating normal equations: {} data points, {} coefficients",
        residuals.len(),
        n_coeff
    );

    let (mut normal_matrix, btr) = accumulate_normal_equations(
        residuals,
        knots_freq,
        knots_cone,
        knots_clock,
        order,
        regularization,
    );

    let coefficients = cholesky_solve(&mut normal_matrix, &btr).ok_or_else(|| {
        CorrectionSurfaceError::SingularMatrix {
            reason: format!(
                "Normal equations are not positive definite: the {} basis functions are not \
                 identifiable from {} data points (increase regularization or reduce knot counts)",
                n_coeff,
                residuals.len()
            ),
        }
    })?;

    Ok(coefficients.to_vec())
}

// ============================================================================
// Correction Surface Evaluation
// ============================================================================

impl CorrectionSurface {
    /// Evaluate the correction at a given point
    ///
    /// # Arguments
    /// * `frequency_mhz` - Frequency in MHz
    /// * `e_cone_deg` - E-cone angle in degrees
    /// * `e_clock_deg` - E-clock angle in degrees
    ///
    /// # Returns
    /// Correction value in dB to add to the model prediction
    pub fn evaluate(&self, frequency_mhz: f64, e_cone_deg: f64, e_clock_deg: f64) -> Result<f64> {
        let basis_freq =
            evaluate_basis_functions(frequency_mhz, &self.knots_frequency, self.spline_order);
        let basis_cone = evaluate_basis_functions(e_cone_deg, &self.knots_econe, self.spline_order);
        let basis_clock =
            evaluate_basis_functions(e_clock_deg, &self.knots_eclock, self.spline_order);

        let [n_freq, n_cone, _n_clock] = self.shape;
        let mut correction = 0.0;

        for &(if_, vf) in &basis_freq {
            for &(ic, vc) in &basis_cone {
                for &(ik, vk) in &basis_clock {
                    let idx = if_ + n_freq * (ic + n_cone * ik);
                    if idx < self.coefficients.len() {
                        correction += self.coefficients[idx] * vf * vc * vk;
                    }
                }
            }
        }

        Ok(correction)
    }

    /// Evaluate corrections for multiple points (batch evaluation)
    pub fn evaluate_batch(
        &self,
        points: &[(f64, f64, f64)], // (freq, cone, clock)
    ) -> Result<Vec<f64>> {
        points
            .iter()
            .map(|(f, c, k)| self.evaluate(*f, *c, *k))
            .collect()
    }
}

// ============================================================================
// Statistics and Validation
// ============================================================================

/// Compute fit statistics for the correction surface
fn compute_fit_statistics(
    surface: &CorrectionSurface,
    residuals: &[ResidualPoint],
    initial_rmse: f64,
) -> Result<FitStatistics> {
    let mut corrected_residuals = Vec::with_capacity(residuals.len());
    let mut max_residual: f64 = 0.0;

    for res in residuals {
        let correction = surface.evaluate(res.frequency_mhz, res.e_cone_deg, res.e_clock_deg)?;
        let corrected = res.residual_db - correction;
        max_residual = max_residual.max(corrected.abs());
        corrected_residuals.push(corrected);
    }

    let rmse = compute_rmse(&corrected_residuals);
    let r_squared = compute_r_squared(
        &residuals.iter().map(|r| r.residual_db).collect::<Vec<_>>(),
        &corrected_residuals,
    );
    let improvement = if initial_rmse > 0.0 {
        ((initial_rmse - rmse) / initial_rmse) * 100.0
    } else {
        0.0
    };

    Ok(FitStatistics {
        num_points: residuals.len(),
        rmse_db: rmse,
        max_residual_db: max_residual,
        r_squared,
        cross_validation_rmse: None,
        improvement_percent: improvement,
    })
}

/// Compute root mean squared error
fn compute_rmse(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = values.iter().map(|v| v * v).sum();
    (sum_sq / values.len() as f64).sqrt()
}

/// Compute R-squared (coefficient of determination)
fn compute_r_squared(original: &[f64], corrected: &[f64]) -> f64 {
    if original.len() != corrected.len() || original.is_empty() {
        return 0.0;
    }

    let mean_original: f64 = original.iter().sum::<f64>() / original.len() as f64;
    let ss_tot: f64 = original.iter().map(|v| (v - mean_original).powi(2)).sum();
    let ss_res: f64 = corrected.iter().map(|v| v.powi(2)).sum();

    if ss_tot == 0.0 {
        return 1.0;
    }

    1.0 - (ss_res / ss_tot)
}

/// Perform k-fold cross-validation
fn cross_validate(residuals: &[ResidualPoint], params: &CorrectionSurfaceParams) -> Result<f64> {
    let k = params.cross_validation_folds;
    if k < 2 {
        return Err(CorrectionSurfaceError::CrossValidationError {
            reason: "Need at least 2 folds for cross-validation".to_string(),
        });
    }

    let n = residuals.len();
    let fold_size = n / k;

    let mut cv_errors = Vec::new();

    for fold in 0..k {
        let start = fold * fold_size;
        let end = if fold == k - 1 {
            n
        } else {
            (fold + 1) * fold_size
        };

        // Split into training and validation sets
        let mut training = Vec::new();
        let mut validation = Vec::new();

        for (i, res) in residuals.iter().enumerate() {
            if i >= start && i < end {
                validation.push(res.clone());
            } else {
                training.push(res.clone());
            }
        }

        // Fit on training data
        // Note: We need to reconstruct measurements and predictions from residuals
        // For simplicity, we'll use the residuals directly and fit to zero-mean
        let training_measurements: Vec<MeasurementPoint> = training
            .iter()
            .map(|r| MeasurementPoint {
                e_clock_deg: r.e_clock_deg,
                e_cone_deg: r.e_cone_deg,
                frequency_mhz: r.frequency_mhz,
                g_over_t_db: r.residual_db,
                temperature_k: 290.0, // Dummy value
            })
            .collect();

        let training_predictions = vec![0.0; training.len()]; // Zero mean for residuals

        // Fit surface on training fold
        let surface =
            fit_correction_surface(&training_measurements, &training_predictions, params)?;

        // Evaluate on validation fold
        for val_res in &validation {
            let correction = surface.evaluate(
                val_res.frequency_mhz,
                val_res.e_cone_deg,
                val_res.e_clock_deg,
            )?;
            let error = val_res.residual_db - correction;
            cv_errors.push(error);
        }
    }

    let cv_rmse = compute_rmse(&cv_errors);
    Ok(cv_rmse)
}

// ============================================================================
// Helper Functions
// ============================================================================

fn validate_fitting_inputs(
    measurements: &[MeasurementPoint],
    model_predictions: &[f64],
    params: &CorrectionSurfaceParams,
) -> Result<()> {
    // Check sufficient data
    let min_required = (params.spline_order + 1).pow(3);
    if measurements.len() < min_required {
        return Err(CorrectionSurfaceError::InsufficientData {
            min_required,
            actual: measurements.len(),
        });
    }

    // Check dimension match
    if measurements.len() != model_predictions.len() {
        return Err(CorrectionSurfaceError::DimensionMismatch {
            measurements: measurements.len(),
            predictions: model_predictions.len(),
        });
    }

    // Validate parameters
    if params.spline_order < 2 {
        return Err(CorrectionSurfaceError::InvalidParameter {
            param: "spline_order".to_string(),
            value: params.spline_order as f64,
            reason: "Must be at least 2".to_string(),
        });
    }

    if params.regularization < 0.0 {
        return Err(CorrectionSurfaceError::InvalidParameter {
            param: "regularization".to_string(),
            value: params.regularization,
            reason: "Must be non-negative".to_string(),
        });
    }

    Ok(())
}

fn mean_residual(residuals: &[ResidualPoint]) -> f64 {
    if residuals.is_empty() {
        return 0.0;
    }
    residuals.iter().map(|r| r.residual_db).sum::<f64>() / residuals.len() as f64
}

fn std_residual(residuals: &[ResidualPoint]) -> f64 {
    if residuals.is_empty() {
        return 0.0;
    }
    let mean = mean_residual(residuals);
    let variance = residuals
        .iter()
        .map(|r| (r.residual_db - mean).powi(2))
        .sum::<f64>()
        / residuals.len() as f64;
    variance.sqrt()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bspline_basis_order_1() {
        let knots = vec![0.0, 1.0, 2.0, 3.0];
        assert!((bspline_basis(0, 1, 0.5, &knots) - 1.0).abs() < 1e-10);
        assert!((bspline_basis(1, 1, 1.5, &knots) - 1.0).abs() < 1e-10);
        assert!(bspline_basis(0, 1, 1.5, &knots).abs() < 1e-10);
    }

    #[test]
    fn test_bspline_basis_order_2() {
        let knots = vec![0.0, 0.0, 1.0, 2.0, 2.0];
        let val = bspline_basis(0, 2, 0.5, &knots);
        assert!(val > 0.0 && val < 1.0);
    }

    #[test]
    fn test_generate_uniform_knots() {
        let knots = generate_uniform_knots(0.0, 10.0, 5);
        assert_eq!(knots.len(), 5);
        assert!((knots[0] - 1.666).abs() < 0.01);
        assert!((knots[4] - 8.333).abs() < 0.01);
    }

    #[test]
    fn test_enforce_min_spacing() {
        let knots = vec![0.0, 0.5, 0.6, 1.0, 1.5, 2.0];
        let result = enforce_min_spacing(&knots, 0.7);
        assert_eq!(result.len(), 3); // Should keep 0.0, 1.0, 2.0
    }

    #[test]
    fn test_compute_rmse() {
        let values = vec![1.0, -1.0, 2.0, -2.0];
        let rmse = compute_rmse(&values);
        assert!((rmse - 1.58113).abs() < 0.001);
    }

    #[test]
    fn test_compute_residuals() {
        let measurements = vec![
            MeasurementPoint {
                e_clock_deg: 0.0,
                e_cone_deg: 0.0,
                frequency_mhz: 8000.0,
                g_over_t_db: 40.0,
                temperature_k: 290.0,
            },
            MeasurementPoint {
                e_clock_deg: 45.0,
                e_cone_deg: 10.0,
                frequency_mhz: 8100.0,
                g_over_t_db: 38.0,
                temperature_k: 290.0,
            },
        ];
        let predictions = vec![39.5, 37.8];

        let residuals = compute_residuals(&measurements, &predictions).unwrap();
        assert_eq!(residuals.len(), 2);
        assert!((residuals[0].residual_db - 0.5).abs() < 1e-10);
        assert!((residuals[1].residual_db - 0.2).abs() < 1e-10);
    }

    #[test]
    fn test_validate_knot_vector() {
        let valid = vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0];
        assert!(validate_knot_vector(&valid, 3).is_ok());

        let invalid_short = vec![0.0, 0.0, 1.0];
        assert!(validate_knot_vector(&invalid_short, 3).is_err());

        let invalid_order = vec![0.0, 2.0, 1.0, 3.0];
        assert!(validate_knot_vector(&invalid_order, 2).is_err());
    }
}

#[cfg(test)]
mod least_squares_tests {
    use super::*;

    /// Deterministic synthetic residuals (no RNG) spanning the knot ranges below.
    fn fixture_residuals() -> Vec<ResidualPoint> {
        let mut pts = Vec::new();
        for i in 0..8 {
            for j in 0..6 {
                for k in 0..6 {
                    let f = 8000.0 + 100.0 * i as f64;
                    let c = 2.0 * j as f64;
                    let cl = 30.0 * k as f64;
                    let r = 0.5 * (f / 1000.0).sin()
                        + 0.25 * (c / 10.0).cos()
                        + 0.1 * (cl / 100.0).sin();
                    pts.push(ResidualPoint {
                        frequency_mhz: f,
                        e_cone_deg: c,
                        e_clock_deg: cl,
                        residual_db: r,
                    });
                }
            }
        }
        pts
    }

    /// (knots_freq, knots_cone, knots_clock, order) — clamped, as `generate_knot_vector` builds them.
    fn fixture_knots() -> (Vec<f64>, Vec<f64>, Vec<f64>, usize) {
        (
            vec![
                8000.0, 8000.0, 8000.0, 8200.0, 8400.0, 8700.0, 8700.0, 8700.0,
            ],
            vec![0.0, 0.0, 0.0, 4.0, 6.0, 10.0, 10.0, 10.0],
            vec![0.0, 0.0, 0.0, 60.0, 100.0, 150.0, 150.0, 150.0],
            3,
        )
    }

    /// Textbook dense construction of `(B^T B + λI, B^T r)`, materializing the design
    /// matrix. This is the definition `accumulate_normal_equations` optimizes away, kept
    /// here as an independent oracle.
    fn dense_normal_equations(
        residuals: &[ResidualPoint],
        knots_freq: &[f64],
        knots_cone: &[f64],
        knots_clock: &[f64],
        order: usize,
        regularization: f64,
    ) -> (Array2<f64>, Array1<f64>) {
        let n_freq = knots_freq.len() - order;
        let n_cone = knots_cone.len() - order;
        let n_clock = knots_clock.len() - order;
        let n_coeff = n_freq * n_cone * n_clock;

        let mut design = Array2::<f64>::zeros((residuals.len(), n_coeff));
        let mut rhs = Array1::<f64>::zeros(residuals.len());

        for (i, res) in residuals.iter().enumerate() {
            rhs[i] = res.residual_db;
            for &(if_, vf) in &evaluate_basis_functions(res.frequency_mhz, knots_freq, order) {
                for &(ic, vc) in &evaluate_basis_functions(res.e_cone_deg, knots_cone, order) {
                    for &(ik, vk) in &evaluate_basis_functions(res.e_clock_deg, knots_clock, order)
                    {
                        design[[i, if_ + n_freq * (ic + n_cone * ik)]] = vf * vc * vk;
                    }
                }
            }
        }

        let mut btb = design.t().dot(&design);
        let btr = design.t().dot(&rhs);
        for i in 0..n_coeff {
            btb[[i, i]] += regularization;
        }
        (btb, btr)
    }

    /// The sparse accumulation must reproduce the dense `B^T B` / `B^T r` exactly.
    #[test]
    fn normal_equations_match_dense_reference() {
        let residuals = fixture_residuals();
        let (kf, kc, kk, order) = fixture_knots();
        let lambda = 1e-3;

        let (sparse_a, sparse_b) =
            accumulate_normal_equations(&residuals, &kf, &kc, &kk, order, lambda);
        let (dense_a, dense_b) = dense_normal_equations(&residuals, &kf, &kc, &kk, order, lambda);

        assert_eq!(sparse_a.dim(), dense_a.dim());
        let max_a = (&sparse_a - &dense_a)
            .iter()
            .fold(0.0f64, |m, v| m.max(v.abs()));
        let max_b = (&sparse_b - &dense_b)
            .iter()
            .fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(max_a < 1e-9, "B^T B mismatch vs dense reference: {max_a:e}");
        assert!(max_b < 1e-9, "B^T r mismatch vs dense reference: {max_b:e}");
    }

    /// Cholesky must actually solve the system it is handed.
    #[test]
    fn cholesky_solves_spd_system() {
        let mut a = Array2::from_shape_vec(
            (3, 3),
            vec![4.0, 12.0, -16.0, 12.0, 37.0, -43.0, -16.0, -43.0, 98.0],
        )
        .unwrap();
        let b = Array1::from_vec(vec![1.0, 2.0, 3.0]);
        let original = a.clone();

        let x = cholesky_solve(&mut a, &b).expect("matrix is positive definite");

        let residual = original.dot(&x) - &b;
        let max = residual.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(max < 1e-10, "A·x != b, residual {max:e}");
    }

    #[test]
    fn cholesky_rejects_non_positive_definite() {
        // Indefinite: eigenvalues ±1.
        let mut a = Array2::from_shape_vec((2, 2), vec![0.0, 1.0, 1.0, 0.0]).unwrap();
        let b = Array1::from_vec(vec![1.0, 1.0]);
        assert!(cholesky_solve(&mut a, &b).is_none());
    }

    /// The fitted coefficients must satisfy the normal equations they were derived from.
    #[test]
    fn fit_satisfies_normal_equations() {
        let residuals = fixture_residuals();
        let (kf, kc, kk, order) = fixture_knots();
        let lambda = 1e-3;

        let coeffs = fit_bspline_coefficients(&residuals, &kf, &kc, &kk, order, lambda).unwrap();
        let (a, b) = accumulate_normal_equations(&residuals, &kf, &kc, &kk, order, lambda);

        let residual = a.dot(&Array1::from_vec(coeffs)) - &b;
        let max = residual.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(max < 1e-8, "(B^T B + λI)c != B^T r, residual {max:e}");
    }

    /// Regression guard: values captured from the previous OpenBLAS/LAPACK (`dgesv`)
    /// implementation, before the switch to sparse normal equations + Cholesky. The fit
    /// must not drift.
    #[test]
    fn fit_matches_openblas_golden() {
        let residuals = fixture_residuals();
        let (kf, kc, kk, order) = fixture_knots();

        let c = fit_bspline_coefficients(&residuals, &kf, &kc, &kk, order, 1e-3).unwrap();

        assert_eq!(c.len(), 125);
        let sum: f64 = c.iter().sum();
        let sumsq: f64 = c.iter().map(|v| v * v).sum();

        let close = |got: f64, want: f64, what: &str| {
            let tol = 1e-6 * want.abs().max(1.0);
            assert!(
                (got - want).abs() < tol,
                "{what}: got {got:.12e}, want {want:.12e}"
            );
        };
        close(sum, 8.154347510713e1, "sum");
        close(sumsq, 5.590390188922e1, "sumsq");
        close(c[0], 7.434343253931e-1, "c[0]");
        close(c[1], 7.358607285775e-1, "c[1]");
        close(c[c.len() / 2], 7.478379397133e-1, "c[mid]");
        close(c[c.len() - 1], 1.489868817277e-1, "c[last]");
    }

    /// Behavior change worth pinning: with λ = 0 and a basis the data cannot identify,
    /// `B^T B` is only positive *semi*-definite. Cholesky refuses it and we surface
    /// `SingularMatrix`, where LAPACK's general LU solve would have returned a
    /// numerically meaningless answer. All production call sites use λ > 0.
    #[test]
    fn unregularized_rank_deficient_fit_reports_singular_matrix() {
        let (kf, kc, kk, order) = fixture_knots();
        // 125 basis functions, 2 data points — hopelessly rank-deficient.
        let residuals = vec![
            ResidualPoint {
                frequency_mhz: 8100.0,
                e_cone_deg: 1.0,
                e_clock_deg: 20.0,
                residual_db: 0.5,
            },
            ResidualPoint {
                frequency_mhz: 8500.0,
                e_cone_deg: 8.0,
                e_clock_deg: 120.0,
                residual_db: -0.25,
            },
        ];

        let err = fit_bspline_coefficients(&residuals, &kf, &kc, &kk, order, 0.0).unwrap_err();
        assert!(
            matches!(err, CorrectionSurfaceError::SingularMatrix { .. }),
            "expected SingularMatrix, got {err:?}"
        );
    }
}
