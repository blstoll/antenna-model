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
use tracing::{debug, info, warn};

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum CorrectionSurfaceError {
    #[error("Insufficient data for fitting: need at least {min_required}, got {actual}")]
    InsufficientData { min_required: usize, actual: usize },

    /// The fitted system has more coefficients than data points (roadmap D20).
    ///
    /// The remediation text says "measurements" first and names the knot counts second, in
    /// that order deliberately: the knot counts have **no CLI flag**. Full mode's are a
    /// private local in `calibrate/src/main.rs::surface_fitting_params` and boresight mode's
    /// are fixed by its axis construction, so "reduce the knots" is a code change, while
    /// supplying more data is something the caller can actually do. The message therefore
    /// states the number of points required rather than leaving it to be inferred from the
    /// coefficient count, and warns about the cross-validation multiplier — a training split
    /// is what has to clear the count, not the whole set.
    #[error(
        "Underdetermined fit: {n_coefficients} B-spline coefficients \
         ({n_freq}x{n_cone}x{n_clock}) against {n_points} data points. Supply at least \
         {n_coefficients} measurements (and at least {n_coefficients} in every \
         cross-validation *training* split, i.e. ~{n_coefficients} / (1 - 1/folds) overall \
         when --validate is used), or reduce the knot counts — which are compile-time, in \
         calibrate/src/main.rs::surface_fitting_params, not a CLI flag."
    )]
    UnderdeterminedFit {
        n_coefficients: usize,
        n_points: usize,
        n_freq: usize,
        n_cone: usize,
        n_clock: usize,
    },

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

impl CorrectionSurfaceParams {
    /// Return a copy of these parameters with cross-validation disabled.
    ///
    /// Used for every *inner* fit performed on behalf of an outer cross-validation —
    /// both the fold refits in [`crate::validator::validate_calibration`] and the fold
    /// fits inside [`cross_validate`] itself. Cross-validating a fold of a
    /// cross-validation is never wanted: it does not describe the surface being scored,
    /// and because each level re-enters `fit_correction_surface` with the same folds it
    /// recurses until the shrinking training set trips the
    /// `(spline_order + 1)³` minimum and the whole run fails.
    ///
    /// Every other field — knot counts, regularization, spline order, knot spacing — is
    /// preserved, so the refit fits the *same model family* as the surface being scored.
    pub fn without_nested_cross_validation(&self) -> Self {
        Self {
            cross_validation_folds: 0,
            ..self.clone()
        }
    }
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

    // The real data-sufficiency requirement (roadmap D20).
    //
    // `validate_fitting_inputs` ran a cheap `(spline_order + 1)^3` pre-check above, which
    // is a fixed 125 at order 4 and depends on nothing about the model actually being
    // fitted. The quantity that decides whether the least-squares system is determined is
    // the coefficient count, and it is only knowable *here*: the knot counts in
    // `params` are a request, and dedup / minimum-spacing / interior-only placement can
    // all reduce them.
    //
    // Below this line the system is underdetermined and the ridge term is the only thing
    // making it solvable — the surface then interpolates its data points almost exactly
    // while oscillating between them, which reads as an excellent RMSE and an inaccurate
    // surface. That is a hard error rather than a warning by decision: a warning here
    // repeats the class of defect D11 was, a real problem reported through a channel
    // nobody reads.
    let n_coefficients = n_freq * n_cone * n_clock;
    if residuals.len() < n_coefficients {
        return Err(CorrectionSurfaceError::UnderdeterminedFit {
            n_coefficients,
            n_points: residuals.len(),
            n_freq,
            n_cone,
            n_clock,
        });
    }

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
        // `None` when no fold could be refitted — reported as an absent figure, never as a
        // failed run (roadmap D22; see `cross_validate`'s docs for why this is the copy that
        // matters).
        let cv_rmse = cross_validate(&residuals, params)?;
        match cv_rmse {
            Some(rmse) => info!("Cross-validation RMSE: {:.3} dB", rmse),
            None => info!("Cross-validation RMSE: not available (no fold could be refitted)"),
        }

        let surface = CorrectionSurface {
            fit_stats: FitStatistics {
                cross_validation_rmse: cv_rmse,
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
///
/// # Degenerate axis
/// A fully degenerate knot vector (every knot equal, e.g. `[5.0; 8]`) has zero-width spans
/// everywhere, so every basis function — including the domain-maximum case above — returns
/// 0.0 rather than summing to a partition of unity. This is pre-existing, not something the
/// domain-maximum fix introduced or fixes: the half-open span can't fire on a zero-width
/// interval either. It is guarded upstream, not here — `generate_knot_vector` rejects
/// `max_val - min_val < min_spacing` before a degenerate vector can be built, so a caller
/// that bypasses that guard would need its own handling for this case.
fn bspline_basis(i: usize, k: usize, t: f64, knots: &[f64]) -> f64 {
    if k == 1 {
        // Base case: characteristic function of the half-open span [knots[i], knots[i+1]).
        if i < knots.len() - 1 && t >= knots[i] && t < knots[i + 1] {
            return 1.0;
        }
        // The domain maximum needs the last non-degenerate span to be closed on the
        // right, or no basis function is non-zero there at all.
        //
        // This is not cosmetic. `accumulate_normal_equations` evaluates the basis at
        // every measurement, so before this a point sitting exactly on an axis maximum
        // contributed an all-zero row: the last coefficient in that axis got no data
        // support and was driven to ~0 by the ridge term, corrupting the fit across the
        // entire top knot span rather than just at the endpoint. On a regular grid the
        // maximum always has data on it. See
        // docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md.
        //
        // The previous attempt at this keyed on `i == knots.len() - 2`, which for a
        // clamped knot vector is a padding index outside the valid basis range
        // `0..knots.len() - order`, so it never fired for a basis function that is
        // actually evaluated.
        if i + 1 < knots.len()
            && t == knots[knots.len() - 1]
            && knots[i + 1] == t
            && knots[i] < knots[i + 1]
        {
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
///
/// # Interior placement (roadmap D19)
///
/// Every knot returned here is *internal*: [`generate_knot_vector`] clamps the vector by
/// prepending and appending `order` copies of the data bounds, so a knot equal to a bound
/// would arrive at multiplicity `order + 1`. The basis function `B_{i,order}` has support
/// `[t_i, t_{i+order}]`, which at that multiplicity is zero-width — the function is
/// identically zero everywhere, its coefficient carries no information, and its row and
/// column of `B^T B` are exactly zero (leaving the system solvable only via the ridge term).
///
/// Quantile placement hits a bound whenever a bound value is common enough to own the
/// quantile: measured 2026-08-02 on D12's fixture, whose frequency axis has four distinct
/// values with 72 rows each, so index `288/5 = 57` selects the minimum and `4·57 = 228` the
/// maximum. Candidates on a bound are therefore **dropped**, not nudged inward — on an axis
/// with four distinct values there is no fourth distinct interior position to nudge one to,
/// and inventing a knot where the data has no support is what the adaptive placement exists
/// to avoid. The delivered count can consequently fall short of the request, which is
/// reported rather than absorbed silently.
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
    let min_val = sorted_data[0];
    let max_val = sorted_data[n - 1];

    let mut knots = Vec::new();
    for i in 1..=num_knots {
        let idx = (i * step).min(n - 1);
        let candidate = sorted_data[idx];

        // Strictly interior only — see the note above.
        if candidate > min_val && candidate < max_val {
            knots.push(candidate);
        }
    }

    // Remove duplicates and enforce minimum spacing
    knots.dedup_by(|a, b| (*b - *a).abs() < min_spacing);

    if knots.len() < num_knots {
        warn!(
            requested = num_knots,
            placed = knots.len(),
            min = min_val,
            max = max_val,
            "adaptive knot placement delivered fewer internal knots than requested: the \
             data does not support that many distinct interior positions"
        );
    }

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
///
/// # Multiplicity (roadmap D19)
///
/// Length and monotonicity were the only checks until 2026-08-02, which let the fitter's own
/// adaptive placement ship vectors with end multiplicity `order + 1` — see
/// [`generate_adaptive_knots`] for the mechanism and
/// `docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md` for the history. The
/// two rules added here are the ones that make a clamped vector well-formed:
///
/// * **Each end must repeat exactly `order` times.** Fewer leaves the spline unclamped (it
///   would not interpolate its end coefficient at the bound); more creates a zero-width
///   support and therefore an identically-zero basis function.
/// * **An interior knot may repeat at most `order - 1` times.** That is a legitimate
///   continuity reduction, down to C⁰ at `order - 1`. Multiplicity `order` splits the spline
///   into disconnected pieces, which this fitter never intends.
///
/// A fully degenerate vector (every knot equal) fails the first rule, which also closes the
/// gap documented on [`bspline_basis`] — previously guarded only upstream in
/// [`generate_knot_vector`].
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

    // Check multiplicity, run by run over the (now known non-decreasing) vector.
    let mut start = 0;
    while start < knots.len() {
        let mut end = start + 1;
        while end < knots.len() && knots[end] == knots[start] {
            end += 1;
        }
        let multiplicity = end - start;
        let is_end_run = start == 0 || end == knots.len();

        if is_end_run {
            if multiplicity != order {
                return Err(CorrectionSurfaceError::InvalidKnotVector {
                    reason: format!(
                        "Clamped knot vector must repeat each bound exactly {order} times: \
                         value {} at index {start} repeats {multiplicity} times. A bound \
                         repeated more than {order} times gives basis function B_{start} a \
                         zero-width support, making it identically zero (roadmap D19)",
                        knots[start]
                    ),
                });
            }
        } else if multiplicity >= order {
            return Err(CorrectionSurfaceError::InvalidKnotVector {
                reason: format!(
                    "Interior knot {} at index {start} repeats {multiplicity} times; the \
                     maximum for order {order} is {} (multiplicity {order} splits the spline)",
                    knots[start],
                    order - 1
                ),
            });
        }

        start = end;
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

/// Is point `index` held out by fold `fold` of `num_folds`?
///
/// **The single definition of fold assignment for this crate** — both cross-validation
/// implementations call it (`cross_validate` below, and `validator::perform_cross_validation`),
/// so they cannot report two numbers computed by two different partitions of the same data.
/// That is not hypothetical: until roadmap **D22** they *did*, and only the validator's copy
/// was ever examined.
///
/// **Strided**: point `i` is held out by fold `i % num_folds`. Folds used to be contiguous
/// slices, `[k·n/K, (k+1)·n/K)`. Measurement files are grid-ordered — frequency-major for both
/// calibrate fixtures and for any real swept measurement — so the first and last slices held
/// out an entire frequency slab, and scoring them made the fit **extrapolate past its own
/// knots**. Measured on D14's 3240-row artifact at 5 folds: 10.07 / 0.56 / 0.12 / 0.64 /
/// 10.86 dB, a mean of 4.45 ± 4.92 dB against an in-sample 0.027 dB. That number was neither
/// generalization error nor a deliberate extrapolation test but a mixture whose proportions
/// depended on how the input file happened to be sorted — re-sorting the same measurements
/// changed the headline quality claim of `--validate`. Strided, the same artifact scores
/// 0.029 / 0.031 / 0.031 / 0.060 / 0.046 dB.
///
/// Striding is deterministic (no RNG, no seed to record) and is invariant to which axis varies
/// fastest, which is the property that was actually missing. Its known bias is the opposite
/// one: on a dense grid every held-out point has near neighbours in the training set, so the
/// score leans **optimistic** and measures interpolation quality rather than extrapolation.
/// That is the right default for a surface whose job is interpolation, and unlike the old
/// behaviour it is a stated property rather than a side effect of row order. A deliberate
/// extrapolation test would have to hold out a named axis on purpose — see D22's option 3.
pub(crate) fn is_held_out(index: usize, fold: usize, num_folds: usize) -> bool {
    index % num_folds == fold
}

/// Perform k-fold cross-validation.
///
/// Returns `None` when no fold could be scored, rather than failing the caller: a fold refits
/// on `(1 − 1/folds)` of the data, and since roadmap **D20** an underdetermined fit is a hard
/// error, so a dataset can clear the coefficient count on the full set and miss it on a split.
/// Propagating that killed the whole run — and because `--validate` sets
/// `cross_validation_folds`, it killed it *here*, inside the fit, before
/// `validator::validate_calibration` or the artifact writer was ever reached. `--validate`
/// could therefore **remove an artifact that the same command without it produces**, which is
/// exactly what roadmap D22 decided it must not do. Fixing that only in the validator left
/// this copy as the reachable one; both are non-fatal now.
fn cross_validate(
    residuals: &[ResidualPoint],
    params: &CorrectionSurfaceParams,
) -> Result<Option<f64>> {
    let k = params.cross_validation_folds;
    if k < 2 {
        return Err(CorrectionSurfaceError::CrossValidationError {
            reason: "Need at least 2 folds for cross-validation".to_string(),
        });
    }

    let mut cv_errors = Vec::new();
    let mut scored_folds = 0usize;
    let mut failed_folds = 0usize;

    for fold in 0..k {
        // Split into training and validation sets, through the shared assignment above.
        let mut training = Vec::new();
        let mut validation = Vec::new();

        for (i, res) in residuals.iter().enumerate() {
            if is_held_out(i, fold, k) {
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

        // Fit surface on training fold. The fold fit must not cross-validate in turn —
        // `fit_correction_surface` would re-enter this function with the same fold count
        // and recurse until the training set falls below the fitting minimum.
        let surface = match fit_correction_surface(
            &training_measurements,
            &training_predictions,
            &params.without_nested_cross_validation(),
        ) {
            Ok(surface) => surface,
            Err(e) => {
                failed_folds += 1;
                warn!(
                    "cross-validation fold {}/{k} could not refit on its training split of {} \
                     points (the full set has {}, and its own fit succeeded — cross-validation \
                     trains on {:.0}% of it): {e}",
                    fold + 1,
                    training.len(),
                    residuals.len(),
                    100.0 * (1.0 - 1.0 / k as f64),
                );
                continue;
            }
        };

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
        scored_folds += 1;
    }

    if scored_folds == 0 {
        warn!(
            "cross-validation could not score any of the {k} folds; the surface's own fit on \
             the full dataset succeeded, so it is reported without a cross-validation figure"
        );
        return Ok(None);
    }
    if failed_folds > 0 {
        warn!(
            "cross-validation is INCOMPLETE: {scored_folds}/{k} folds scored; the figure below \
             covers only those"
        );
    }

    Ok(Some(compute_rmse(&cv_errors)))
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

    // ========================================================================
    // D19 — adaptive knots must land in the strict interior
    //
    // `generate_adaptive_knots` placed internal knots at data quantiles with no
    // constraint that the result be interior, and `generate_knot_vector` then clamps
    // by prepending/appending `order` copies of the bounds. A quantile that landed ON
    // a bound therefore became multiplicity `order + 1`, whose basis function
    // `B_{i,order}` has support `[t_i, t_{i+order}]` of ZERO width — identically zero
    // everywhere, contributing a dead coefficient and an empty row/column in `B^T B`.
    // ========================================================================

    /// The D12 full-mode fixture's axis data, in the shipped 4/6/8 configuration.
    ///
    /// The frequency axis is the sharp case: four distinct values (400/500/600/700 MHz)
    /// with 72 rows each, so quantile index `(1 * 288/5) = 57` lands on the minimum and
    /// `(4 * 288/5) = 228` lands on the maximum.
    fn d12_axis_data() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let freqs = [400.0, 500.0, 600.0, 700.0];
        let cones = [0.0, 2.0, 4.0, 6.0, 9.0, 12.0, 16.0, 20.0, 24.0];
        let clocks = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

        let (mut fd, mut cd, mut kd) = (Vec::new(), Vec::new(), Vec::new());
        for &f in &freqs {
            for &c in &cones {
                for &k in &clocks {
                    fd.push(f);
                    cd.push(c);
                    kd.push(k);
                }
            }
        }
        (fd, cd, kd)
    }

    /// Indices of basis functions whose support `[t_i, t_{i+order}]` has zero width.
    fn identically_zero_basis(knots: &[f64], order: usize) -> Vec<usize> {
        (0..knots.len() - order)
            .filter(|&i| knots[i + order] - knots[i] <= 0.0)
            .collect()
    }

    /// The negative control for the multiplicity guard, per P13: a check nobody has
    /// seen fail is not evidence of anything. These are the knot vectors the fitter
    /// ACTUALLY produced on 2026-08-02, before this fix — `validate_knot_vector` passed
    /// every one of them, and must now reject the two that are defective.
    #[test]
    fn multiplicity_guard_rejects_the_knot_vectors_the_fitter_used_to_produce() {
        let order = 4;

        // Measured pre-fix. End multiplicity 5 = order + 1 on both axes.
        let pre_fix_frequency = vec![
            400.0, 400.0, 400.0, 400.0, 400.0, 500.0, 600.0, 700.0, 700.0, 700.0, 700.0, 700.0,
        ];
        let pre_fix_clock = vec![
            0.0, 0.0, 0.0, 0.0, 0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0, 315.0, 315.0,
            315.0, 315.0,
        ];

        for (name, knots) in [("frequency", &pre_fix_frequency), ("clock", &pre_fix_clock)] {
            assert_eq!(
                identically_zero_basis(knots, order),
                vec![0, knots.len() - order - 1],
                "{name}: the pre-fix vector must have a dead basis function at each end, \
                 or this control is not testing what it claims"
            );
            assert!(
                validate_knot_vector(knots, order).is_err(),
                "{name}: validate_knot_vector must reject end multiplicity {} for order \
                 {order}; it accepted this vector before D19",
                order + 1
            );
        }

        // Positive control: the cone axis was already correct pre-fix and must stay ok.
        let pre_fix_cone = vec![
            0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 6.0, 12.0, 16.0, 20.0, 24.0, 24.0, 24.0, 24.0,
        ];
        assert!(identically_zero_basis(&pre_fix_cone, order).is_empty());
        assert!(
            validate_knot_vector(&pre_fix_cone, order).is_ok(),
            "the cone axis was never defective and must not be rejected"
        );
    }

    /// An interior knot may repeat up to `order - 1` times (that is a legitimate
    /// continuity reduction, down to C⁰); `order` times splits the spline into
    /// disconnected pieces and is not something this fitter ever intends to produce.
    #[test]
    fn multiplicity_guard_admits_legitimate_interior_repeats() {
        let order = 4;

        let c0_knot = vec![0.0, 0.0, 0.0, 0.0, 5.0, 5.0, 5.0, 10.0, 10.0, 10.0, 10.0];
        assert!(
            validate_knot_vector(&c0_knot, order).is_ok(),
            "interior multiplicity {} (= order - 1) is legitimate",
            order - 1
        );

        let split = vec![
            0.0, 0.0, 0.0, 0.0, 5.0, 5.0, 5.0, 5.0, 10.0, 10.0, 10.0, 10.0,
        ];
        assert!(
            validate_knot_vector(&split, order).is_err(),
            "interior multiplicity {order} (= order) splits the spline and must be rejected"
        );
    }

    /// A fully degenerate vector (every knot equal) makes every basis function return
    /// 0 rather than summing to a partition of unity — recorded as item 3 of
    /// `docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md`'s "Still
    /// open" and guarded only upstream until now. The multiplicity check closes it here
    /// too, at no extra cost.
    #[test]
    fn multiplicity_guard_rejects_a_fully_degenerate_axis() {
        assert!(validate_knot_vector(&[5.0; 8], 4).is_err());
    }

    #[test]
    fn adaptive_knots_are_strictly_interior_on_every_d12_axis() {
        let order = 4;
        let (fd, cd, kd) = d12_axis_data();

        for (name, data, num_knots, min_spacing) in [
            ("frequency", &fd, 4usize, 50.0),
            ("cone", &cd, 6, 2.0),
            ("clock", &kd, 8, 5.0),
        ] {
            let knots = generate_knot_vector(data, num_knots, order, true, min_spacing)
                .unwrap_or_else(|e| panic!("{name}: knot generation failed: {e}"));

            let lo = knots[0];
            let hi = knots[knots.len() - 1];

            assert_eq!(
                knots.iter().filter(|&&k| k == lo).count(),
                order,
                "{name}: end multiplicity must be exactly the order, got {knots:?}"
            );
            assert_eq!(
                knots.iter().filter(|&&k| k == hi).count(),
                order,
                "{name}: end multiplicity must be exactly the order, got {knots:?}"
            );
            assert!(
                identically_zero_basis(&knots, order).is_empty(),
                "{name}: every basis function must have non-zero support, got {knots:?}"
            );
            assert!(
                validate_knot_vector(&knots, order).is_ok(),
                "{name}: the generator must produce vectors its own validator accepts"
            );
        }
    }

    /// The headline number: what the dead basis functions cost the shipped configuration.
    ///
    /// Pre-D19 the D12 configuration declared **960** coefficients, of which **360 (37.5 %)**
    /// were attached to identically-zero basis functions — serialized into every artifact and
    /// read back by the service's 4D interpolator, carrying no information.
    ///
    /// The served surface is **unchanged** by their removal, and that is not a weaker result
    /// than it sounds: a basis function that is zero everywhere contributes zero to every
    /// evaluation, so removing it *cannot* move a value. D12's four known-answer probes
    /// reproduce bit-for-bit across this change (0.5928 / 0.0934 / 0.0365 / 0.0934 dB), which
    /// is the end-to-end confirmation. What D19 fixes is the representation: an honest
    /// `shape`, and a `B^T B` that is no longer structurally rank-deficient. The remaining
    /// probe error is underdetermination (600 coefficients against 288 points) — roadmap D20.
    #[test]
    fn the_shipped_configuration_no_longer_carries_dead_coefficients() {
        let order = 4;
        let (fd, cd, kd) = d12_axis_data();

        let axes = [
            ("frequency", &fd, 4usize, 50.0, 6usize),
            ("cone", &cd, 6, 2.0, 10),
            ("clock", &kd, 8, 5.0, 10),
        ];

        let mut n_coefficients = 1;
        for (name, data, num_knots, min_spacing, expected_basis) in axes {
            let knots = generate_knot_vector(data, num_knots, order, true, min_spacing)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let n_basis = knots.len() - order;

            assert_eq!(
                n_basis, expected_basis,
                "{name}: expected {expected_basis} basis functions, got {n_basis} from {knots:?}"
            );
            n_coefficients *= n_basis;
        }

        assert_eq!(
            n_coefficients, 600,
            "the D12 configuration should declare 600 live coefficients (was 960 declared, \
             360 of them identically zero)"
        );
    }

    /// The fix must not disturb an axis that was already placing knots correctly —
    /// otherwise it is changing more than the defect.
    #[test]
    fn adaptive_knots_are_unchanged_on_the_axis_that_was_already_correct() {
        let (_, cd, _) = d12_axis_data();
        let knots = generate_knot_vector(&cd, 6, 4, true, 2.0).expect("cone knots");

        assert_eq!(
            knots,
            vec![0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 6.0, 12.0, 16.0, 20.0, 24.0, 24.0, 24.0, 24.0],
            "the cone axis was correct before D19 and must be bit-identical after"
        );
    }

    // ========================================================================
    // D20 — the data-sufficiency check must test the coefficient count
    // ========================================================================

    /// Build `n` residual points on a regular grid spanning the given axes.
    fn grid_measurements(
        n_freq: usize,
        n_cone: usize,
        n_clock: usize,
    ) -> (Vec<MeasurementPoint>, Vec<f64>) {
        let mut measurements = Vec::new();
        let mut predictions = Vec::new();

        for i in 0..n_freq {
            for j in 0..n_cone {
                for k in 0..n_clock {
                    let frequency_mhz = 400.0 + 300.0 * i as f64 / (n_freq - 1).max(1) as f64;
                    let e_cone_deg = 24.0 * j as f64 / (n_cone - 1).max(1) as f64;
                    let e_clock_deg = 315.0 * k as f64 / (n_clock - 1).max(1) as f64;

                    measurements.push(MeasurementPoint::new(
                        e_clock_deg,
                        e_cone_deg,
                        frequency_mhz,
                        10.0,
                        290.0,
                    ));
                    predictions.push(9.0);
                }
            }
        }

        (measurements, predictions)
    }

    /// The check must fire on a system the old `(spline_order + 1)^3 = 125` minimum waved
    /// through. 216 points comfortably clears 125 and is nowhere near the 600 coefficients
    /// the shipped full-mode configuration declares.
    #[test]
    fn a_fit_with_fewer_points_than_coefficients_is_rejected() {
        let (measurements, predictions) = grid_measurements(6, 6, 6);
        assert!(
            measurements.len() > (4 + 1usize).pow(3),
            "the fixture must clear the old 125-point minimum, or this test proves nothing"
        );

        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 4,
            num_knots_econe: 6,
            num_knots_eclock: 8,
            regularization: 1e-3,
            adaptive_knots: true,
            cross_validation_folds: 0,
            ..CorrectionSurfaceParams::default()
        };

        let err = fit_correction_surface(&measurements, &predictions, &params)
            .expect_err("an underdetermined fit must be rejected");

        match err {
            CorrectionSurfaceError::UnderdeterminedFit {
                n_coefficients,
                n_points,
                ..
            } => {
                assert!(
                    n_points < n_coefficients,
                    "the error must report the two numbers that made it fire, got \
                     {n_points} points / {n_coefficients} coefficients"
                );
            }
            other => panic!("expected UnderdeterminedFit, got {other:?}"),
        }
    }

    /// ...and must not fire once the data covers the coefficients.
    #[test]
    fn a_fit_with_more_points_than_coefficients_is_accepted() {
        let (measurements, predictions) = grid_measurements(6, 6, 6);
        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 0,
            num_knots_econe: 0,
            num_knots_eclock: 0,
            regularization: 1e-3,
            adaptive_knots: true,
            cross_validation_folds: 0,
            ..CorrectionSurfaceParams::default()
        };

        // 4x4x4 = 64 coefficients against 216 points.
        fit_correction_surface(&measurements, &predictions, &params)
            .expect("a determined system must fit");
    }

    /// The old pre-check is kept as a cheap early guard, not replaced — it catches obvious
    /// garbage before any knot generation happens.
    #[test]
    fn the_cheap_pre_check_still_rejects_obviously_too_little_data() {
        let (measurements, predictions) = grid_measurements(2, 2, 2);
        let params = CorrectionSurfaceParams {
            cross_validation_folds: 0,
            ..CorrectionSurfaceParams::default()
        };

        let err = fit_correction_surface(&measurements, &predictions, &params)
            .expect_err("8 points must be rejected");
        assert!(
            matches!(err, CorrectionSurfaceError::InsufficientData { .. }),
            "expected the cheap InsufficientData pre-check, got {err:?}"
        );
    }

    // ========================================================================
    // D10 — cross-validation must not re-enter itself
    // ========================================================================

    #[test]
    fn without_nested_cross_validation_preserves_every_other_field() {
        let params = CorrectionSurfaceParams {
            spline_order: 5,
            num_knots_frequency: 4,
            num_knots_econe: 6,
            num_knots_eclock: 8,
            regularization: 1e-3,
            adaptive_knots: false,
            cross_validation_folds: 5,
            min_knot_spacing_frequency: 25.0,
            min_knot_spacing_econe: 1.0,
            min_knot_spacing_eclock: 3.0,
        };

        let refit = params.without_nested_cross_validation();

        assert_eq!(refit.cross_validation_folds, 0);
        assert_eq!(refit.spline_order, params.spline_order);
        assert_eq!(refit.num_knots_frequency, params.num_knots_frequency);
        assert_eq!(refit.num_knots_econe, params.num_knots_econe);
        assert_eq!(refit.num_knots_eclock, params.num_knots_eclock);
        assert_eq!(refit.regularization, params.regularization);
        assert_eq!(refit.adaptive_knots, params.adaptive_knots);
        assert_eq!(
            refit.min_knot_spacing_frequency,
            params.min_knot_spacing_frequency
        );
        assert_eq!(refit.min_knot_spacing_econe, params.min_knot_spacing_econe);
        assert_eq!(
            refit.min_knot_spacing_eclock,
            params.min_knot_spacing_eclock
        );
    }

    /// A fold fit inside `cross_validate` must not cross-validate in turn. The fixture is
    /// sized so the recursion is exactly what fails, restated against the quantity roadmap
    /// D20 made binding — the coefficient count, not the old `(4+1)³ = 125` minimum.
    ///
    /// These knots declare 4 × 10 × 10 = **400** coefficients (two distinct frequencies
    /// place no interior knot, so that axis contributes `order` basis functions). 512
    /// points leaves **410** in one 5-fold training split, which clears 400; a *second*
    /// level would train on **328** and trip the check. The window is deliberately narrow —
    /// a fixture much larger than this would let two levels of recursion succeed and the
    /// test would stop testing anything.
    #[test]
    fn cross_validation_does_not_recurse_into_itself() {
        let mut measurements = Vec::new();
        let mut predictions = Vec::new();
        for fi in 0..2 {
            let frequency_mhz = 8400.0 + 100.0 * fi as f64;
            for ci in 0..32 {
                let e_cone_deg = ci as f64;
                for ki in 0..8 {
                    let e_clock_deg = 45.0 * ki as f64;
                    let ripple = 0.4 * e_clock_deg.to_radians().cos();
                    measurements.push(MeasurementPoint::new(
                        e_clock_deg,
                        e_cone_deg,
                        frequency_mhz,
                        41.5 - 0.35 * e_cone_deg * e_cone_deg + ripple,
                        50.0,
                    ));
                    predictions.push(41.5 - 0.33 * e_cone_deg * e_cone_deg);
                }
            }
        }
        assert_eq!(measurements.len(), 512);

        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 4,
            num_knots_econe: 6,
            num_knots_eclock: 8,
            regularization: 1e-3,
            adaptive_knots: true,
            cross_validation_folds: 5,
            min_knot_spacing_frequency: 50.0,
            min_knot_spacing_econe: 2.0,
            min_knot_spacing_eclock: 5.0,
        };

        let surface = fit_correction_surface(&measurements, &predictions, &params)
            .expect("one level of cross-validation must fit; a second level would not");

        assert!(
            surface.fit_stats.cross_validation_rmse.is_some(),
            "the requested cross-validation should still have run"
        );
    }

    /// **Roadmap D22, the reachable half.** A fold that cannot refit must not fail the fit.
    ///
    /// This is the copy of cross-validation that `--validate` actually reaches first:
    /// `main::surface_fitting_params` sets `cross_validation_folds` from the flag, so
    /// `fit_correction_surface` cross-validates *before* `validator::validate_calibration`
    /// is ever called. While this propagated a fold failure, D22's decision — warn and still
    /// ship — was unreachable from the CLI no matter what the validator did: the run died
    /// here, and `--validate` **removed an artifact that the same command without it
    /// produces**.
    ///
    /// The fixture is sized to fail: 448 points clear the 400 coefficients the shipped knot
    /// counts declare, a 5-fold training split (358) does not.
    #[test]
    fn a_fold_that_cannot_refit_does_not_fail_the_fit() {
        let mut measurements = Vec::new();
        let mut predictions = Vec::new();
        for fi in 0..2 {
            let frequency_mhz = 8400.0 + 100.0 * fi as f64;
            for ci in 0..28 {
                let e_cone_deg = ci as f64;
                for ki in 0..8 {
                    let e_clock_deg = 45.0 * ki as f64;
                    let ripple = 0.4 * e_clock_deg.to_radians().cos();
                    measurements.push(MeasurementPoint::new(
                        e_clock_deg,
                        e_cone_deg,
                        frequency_mhz,
                        41.5 - 0.35 * e_cone_deg * e_cone_deg + ripple,
                        50.0,
                    ));
                    predictions.push(41.5 - 0.33 * e_cone_deg * e_cone_deg);
                }
            }
        }
        assert_eq!(measurements.len(), 448);

        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 4,
            num_knots_econe: 6,
            num_knots_eclock: 8,
            regularization: 1e-3,
            adaptive_knots: true,
            cross_validation_folds: 5,
            min_knot_spacing_frequency: 50.0,
            min_knot_spacing_econe: 2.0,
            min_knot_spacing_eclock: 5.0,
        };

        // Premise: the full set fits. If this ever stops holding the test below proves
        // nothing, because there would be no artifact to protect in the first place.
        fit_correction_surface(
            &measurements,
            &predictions,
            &params.without_nested_cross_validation(),
        )
        .expect("the whole set must fit — that is the premise");

        let surface = fit_correction_surface(&measurements, &predictions, &params).expect(
            "a fold that cannot refit must not fail the fit: the surface's own fit on the \
             full dataset succeeded, and --validate is not allowed to withhold an artifact \
             it would otherwise have produced (roadmap D22)",
        );

        assert!(
            surface.fit_stats.cross_validation_rmse.is_none(),
            "no fold could be scored here, so there is no cross-validation figure to report \
             — reporting one would describe folds that never ran"
        );
    }

    // ========================================================================
    // Endpoint evaluation — see
    // docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md
    // ========================================================================

    /// Clamped knot vector on `[lo, hi]` with `n_internal` evenly spaced internal knots.
    fn clamped_knots(lo: f64, hi: f64, n_internal: usize, order: usize) -> Vec<f64> {
        let mut k = vec![lo; order];
        for i in 1..=n_internal {
            k.push(lo + (hi - lo) * i as f64 / (n_internal + 1) as f64);
        }
        k.extend(std::iter::repeat_n(hi, order));
        k
    }

    /// A surface whose coefficients are all 1.0. A correct B-spline basis is a partition
    /// of unity, so this must evaluate to exactly 1.0 everywhere in its domain — including
    /// on the boundary.
    fn unit_surface(order: usize) -> CorrectionSurface {
        let knots_frequency = clamped_knots(400.0, 700.0, 2, order);
        let knots_econe = clamped_knots(0.0, 24.0, 4, order);
        let knots_eclock = clamped_knots(0.0, 315.0, 6, order);
        let shape = [
            knots_frequency.len() - order,
            knots_econe.len() - order,
            knots_eclock.len() - order,
        ];
        CorrectionSurface {
            coefficients: vec![1.0; shape[0] * shape[1] * shape[2]],
            shape,
            knots_frequency,
            knots_econe,
            knots_eclock,
            spline_order: order,
            fit_stats: FitStatistics {
                num_points: 0,
                rmse_db: 0.0,
                max_residual_db: 0.0,
                r_squared: 0.0,
                cross_validation_rmse: None,
                improvement_percent: 0.0,
            },
        }
    }

    /// The regression this fixes: the basis was a partition of unity everywhere *except*
    /// at the exact maximum of an axis, where every basis function evaluated to zero.
    /// Measured before the fix: 1.000000000 at t=0.9999, 0.000000000 at t=1.0.
    #[test]
    fn basis_is_a_partition_of_unity_on_every_face_and_corner() {
        let s = unit_surface(4);
        let (f_lo, f_hi) = (400.0, 700.0);
        let (c_lo, c_hi) = (0.0, 24.0);
        let (k_lo, k_hi) = (0.0, 315.0);
        let f_mid = 0.5 * (f_lo + f_hi);
        let c_mid = 0.5 * (c_lo + c_hi);
        let k_mid = 0.5 * (k_lo + k_hi);

        // Interior, all six faces, and all eight corners.
        let mut probes = vec![
            ("interior", f_mid, c_mid, k_mid),
            ("freq min face", f_lo, c_mid, k_mid),
            ("freq MAX face", f_hi, c_mid, k_mid),
            ("cone min face", f_mid, c_lo, k_mid),
            ("cone MAX face", f_mid, c_hi, k_mid),
            ("clock min face", f_mid, c_mid, k_lo),
            ("clock MAX face", f_mid, c_mid, k_hi),
        ];
        for &f in &[f_lo, f_hi] {
            for &c in &[c_lo, c_hi] {
                for &k in &[k_lo, k_hi] {
                    probes.push(("corner", f, c, k));
                }
            }
        }

        for (label, f, c, k) in probes {
            let got = s.evaluate(f, c, k).expect("evaluate");
            assert!(
                (got - 1.0).abs() < 1e-12,
                "{label} ({f}, {c}, {k}): basis summed to {got:.12}, not 1.0 — \
                 the B-spline basis is not a partition of unity there"
            );
        }
    }

    /// Approaching the maximum must not be discontinuous with reaching it.
    #[test]
    fn basis_is_continuous_up_to_the_maximum() {
        let s = unit_surface(4);
        for &f in &[699.0_f64, 699.9, 699.99, 699.999, 699.999_999, 700.0] {
            let got = s.evaluate(f, 12.0, 180.0).expect("evaluate");
            assert!(
                (got - 1.0).abs() < 1e-12,
                "at frequency {f} the basis summed to {got:.12}, not 1.0"
            );
        }
    }

    /// The consequence that actually mattered: because the fitter uses the same basis, a
    /// measurement sitting exactly on an axis maximum contributed an all-zero row to the
    /// normal equations, so the last coefficient got no data support and collapsed to ~0
    /// under regularization — corrupting the whole top knot span, not just the endpoint.
    /// A basis-only test would pass while the fit stayed broken, so assert on a FIT.
    #[test]
    fn a_fitted_constant_is_recovered_at_the_domain_maximum() {
        // Deliberately OVERdetermined: 7^3 = 343 points against
        // (2+4)^3 = 216 coefficients. The shipped 4/6/8 configuration is
        // underdetermined (288 points, 960 coefficients), which degrades the fit for a
        // separate, still-open reason — this test must isolate the endpoint behaviour,
        // so it must not also be starved of data.
        let freqs: Vec<f64> = (0..7).map(|i| 400.0 + 50.0 * i as f64).collect();
        let cones: Vec<f64> = (0..7).map(|i| 4.0 * i as f64).collect();
        let clocks: Vec<f64> = (0..7).map(|i| 52.5 * i as f64).collect();

        let mut measurements = Vec::new();
        let mut predictions = Vec::new();
        for &f in &freqs {
            for &c in &cones {
                for &k in &clocks {
                    // Residual is a constant 1.5 dB, which a B-spline represents exactly.
                    measurements.push(MeasurementPoint::new(k, c, f, 1.5, 100.0));
                    predictions.push(0.0);
                }
            }
        }
        assert_eq!(measurements.len(), 343);

        let params = CorrectionSurfaceParams {
            spline_order: 4,
            num_knots_frequency: 2,
            num_knots_econe: 2,
            num_knots_eclock: 2,
            regularization: 1e-9,
            adaptive_knots: false,
            cross_validation_folds: 0,
            min_knot_spacing_frequency: 50.0,
            min_knot_spacing_econe: 2.0,
            min_knot_spacing_eclock: 5.0,
        };
        let surface = fit_correction_surface(&measurements, &predictions, &params)
            .expect("fitting a constant must succeed");

        for (label, f, c, k) in [
            ("interior", 550.0, 12.0, 180.0),
            ("frequency at MAX", 700.0, 12.0, 180.0),
            ("frequency just under MAX", 699.99, 12.0, 180.0),
            ("cone at MAX", 550.0, 24.0, 180.0),
            ("clock at MAX", 550.0, 12.0, 315.0),
            ("all three at MAX", 700.0, 24.0, 315.0),
        ] {
            let got = surface.evaluate(f, c, k).expect("evaluate");
            assert!(
                (got - 1.5).abs() < 1e-3,
                "{label}: fitted constant recovered as {got:.6}, expected 1.5"
            );
        }
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

    /// Regression guard for **solver drift**: the sparse normal-equations + Cholesky path
    /// must keep agreeing with the original OpenBLAS/LAPACK (`dgesv`) implementation it
    /// replaced. It is not an oracle for the B-spline basis itself — see the re-pin note
    /// below.
    ///
    /// **Re-pinned 2026-07-30** after the `bspline_basis` domain-maximum fix (see the
    /// `k == 1` base case above and
    /// `docs/findings-2026-07-29-correction-surface-upper-edge-collapse.md`). This
    /// fixture's frequency axis (`fixture_residuals`, `i in 0..8` → 8000..8700 MHz in
    /// 100 MHz steps) reaches exactly 8700 MHz, the frequency knot vector's maximum
    /// (`fixture_knots`). Only `i == 7` reaches 8700 MHz, so that is 6×6 = 36 of the 288
    /// points (`j in 0..6` cone steps times `k in 0..6` clock steps at that one frequency
    /// step); before the fix, those 36 points evaluated the basis to all zero and
    /// contributed nothing to the normal equations, so the last frequency coefficient had
    /// no data support and was pulled toward zero by the ridge term alone. The fit below
    /// legitimately changed as a result — this is not solver drift, it is the fixture
    /// actually being fit correctly for the first time.
    ///
    /// Pre-fix (buggy basis) values, kept for the record:
    /// `sum = 8.154347510713e1`, `sumsq = 5.590390188922e1`, `c[0] = 7.434343253931e-1`,
    /// `c[1] = 7.358607285775e-1`, `c[mid] = 7.478379397133e-1`,
    /// `c[last] = 1.489868817277e-1`.
    ///
    /// Sanity check on the new values: `sum` rose (81.54 -> 87.17) and `c[last]` rose
    /// sharply (0.149 -> 0.566) — exactly the direction expected when a starved
    /// coefficient regains data support instead of being suppressed by the ridge term.
    /// This is coefficient-index-specific, not a uniform shift: flattening index is
    /// `i_freq + n_freq * (i_cone + n_cone * i_clock)`, and `c[0]`, `c[1]`, `c[mid]` all
    /// have `i_freq != 4` (the top frequency index), so none of them touch the
    /// frequency-max span and none move much — while `c[last]` (index 124) is the single
    /// coefficient at `i_freq = 4`, the one that was starved.
    ///
    /// The new values are corroborated by three independent checks, not just accepted
    /// as "whatever the code now emits": `fit_satisfies_normal_equations` (the solve is
    /// self-consistent, `(BᵀB + λI)c = Bᵀr`), `normal_equations_match_dense_reference`
    /// (the sparse accumulation matches a dense reference), and — the genuine basis
    /// oracle — `a_fitted_constant_is_recovered_at_the_domain_maximum`, which fits a
    /// constant (analytically exactly representable by any B-spline) and recovers it
    /// everywhere including the domain maximum, independent of any pinned number here.
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
        close(sum, 8.717338510919e1, "sum");
        close(sumsq, 6.171608452120e1, "sumsq");
        close(c[0], 7.444512507241e-1, "c[0]");
        close(c[1], 7.368946771474e-1, "c[1]");
        close(c[c.len() / 2], 7.472561519268e-1, "c[mid]");
        close(c[c.len() - 1], 5.661438211178e-1, "c[last]");
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
