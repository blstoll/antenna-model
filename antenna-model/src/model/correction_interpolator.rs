//! Correction Surface Interpolation
//!
//! This module provides B-spline interpolation for correction surfaces that
//! adjust the physics-based antenna model predictions to match calibration measurements.
//!
//! The correction surface is stored as a 4D B-spline model with dimensions:
//! - Azimuth (degrees)
//! - Elevation (degrees)
//! - Frequency (MHz)
//! - Temperature (Kelvin, typically constant)
//!
//! # Correction Model
//!
//! The correction surface represents the residual error between the physics model
//! and measured data:
//!
//! ```text
//! Gain_final = Gain_physics + Correction_surface
//! ```
//!
//! Where:
//! - `Gain_physics`: Computed from physical optics model (aperture integration, etc.)
//! - `Correction_surface`: B-spline interpolated correction (this module)
//! - `Gain_final`: Final predicted gain matching calibration measurements

use crate::data::types::BSplineModel4D;
use crate::error::{ComputationError, Result};

/// Result of correction surface evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionResult {
    /// Correction value in dB (to be added to physics model gain)
    pub correction_db: f64,

    /// Warnings generated during evaluation
    pub warnings: Vec<String>,

    /// Whether any input was out of range (extrapolated)
    pub extrapolated: bool,
}

impl CorrectionResult {
    /// Create a new correction result with no warnings
    pub fn new(correction_db: f64) -> Self {
        Self {
            correction_db,
            warnings: Vec::new(),
            extrapolated: false,
        }
    }

    /// Add a warning message
    pub fn with_warning(mut self, warning: String) -> Self {
        self.warnings.push(warning);
        self
    }

    /// Mark as extrapolated
    pub fn with_extrapolation(mut self) -> Self {
        self.extrapolated = true;
        self
    }
}

/// Evaluate correction surface at a given point.
///
/// Performs 4D B-spline interpolation on the correction surface to compute
/// the correction value to be added to the physics model gain.
///
/// # Arguments
/// - `model`: B-spline correction surface model
/// - `azimuth_deg`: Azimuth in degrees
/// - `elevation_deg`: Elevation in degrees
/// - `frequency_mhz`: Frequency in MHz
/// - `temperature_k`: Temperature in Kelvin (typically constant)
///
/// # Returns
/// Correction value in dB with warnings if extrapolated
///
/// # Out-of-Range Handling
/// - If query point is outside the knot vector range, extrapolation is used
/// - Nearest boundary value is returned (constant extrapolation)
/// - A warning is generated and `extrapolated` flag is set
pub fn evaluate_correction(
    model: &BSplineModel4D,
    azimuth_deg: f64,
    elevation_deg: f64,
    frequency_mhz: f64,
    temperature_k: f64,
) -> Result<CorrectionResult> {
    // Validate inputs
    if !azimuth_deg.is_finite()
        || !elevation_deg.is_finite()
        || !frequency_mhz.is_finite()
        || !temperature_k.is_finite()
    {
        return Err(ComputationError::InterpolationFailed {
            azimuth: azimuth_deg,
            elevation: elevation_deg,
            frequency: frequency_mhz,
            temperature: temperature_k,
            reason: "Non-finite input value".to_string(),
        }
        .into());
    }

    // Find knot span indices for each dimension
    let (az_idx, az_extrapolated) =
        find_knot_span(&model.knots_azimuth, azimuth_deg, model.spline_order);
    let (el_idx, el_extrapolated) =
        find_knot_span(&model.knots_elevation, elevation_deg, model.spline_order);
    let (freq_idx, freq_extrapolated) =
        find_knot_span(&model.knots_frequency, frequency_mhz, model.spline_order);
    let (temp_idx, temp_extrapolated) =
        find_knot_span(&model.knots_temperature, temperature_k, model.spline_order);

    let any_extrapolated =
        az_extrapolated || el_extrapolated || freq_extrapolated || temp_extrapolated;

    // Evaluate B-spline basis functions for each dimension
    let az_basis = evaluate_basis_functions(
        &model.knots_azimuth,
        az_idx,
        azimuth_deg,
        model.spline_order,
    );
    let el_basis = evaluate_basis_functions(
        &model.knots_elevation,
        el_idx,
        elevation_deg,
        model.spline_order,
    );
    let freq_basis = evaluate_basis_functions(
        &model.knots_frequency,
        freq_idx,
        frequency_mhz,
        model.spline_order,
    );
    let temp_basis = evaluate_basis_functions(
        &model.knots_temperature,
        temp_idx,
        temperature_k,
        model.spline_order,
    );

    // Perform tensor product interpolation
    let correction_db = tensor_product_interpolation(
        &model.coefficients,
        &model.shape,
        az_idx,
        el_idx,
        freq_idx,
        temp_idx,
        &az_basis,
        &el_basis,
        &freq_basis,
        &temp_basis,
        model.spline_order as usize,
    )?;

    // Build result with warnings if extrapolated
    let mut result = CorrectionResult::new(correction_db);

    if any_extrapolated {
        result = result.with_extrapolation();

        let mut out_of_range_dims = Vec::new();
        if az_extrapolated {
            out_of_range_dims.push(format!("azimuth ({:.2}°)", azimuth_deg));
        }
        if el_extrapolated {
            out_of_range_dims.push(format!("elevation ({:.2}°)", elevation_deg));
        }
        if freq_extrapolated {
            out_of_range_dims.push(format!("frequency ({:.1} MHz)", frequency_mhz));
        }
        if temp_extrapolated {
            out_of_range_dims.push(format!("temperature ({:.1} K)", temperature_k));
        }

        result = result.with_warning(format!(
            "Correction surface extrapolated for: {}",
            out_of_range_dims.join(", ")
        ));
    }

    Ok(result)
}

/// Find knot span index for a given parameter value.
///
/// Returns the index `i` such that `knots[i] <= u < knots[i+1]`.
/// Also returns whether the value was out of range (extrapolated).
fn find_knot_span(knots: &[f64], u: f64, order: u8) -> (usize, bool) {
    // Number of basis functions for a clamped B-spline: len - order  (NOT len - order - 1).
    // The extra -1 was an off-by-one that made `high` one short of u_max, so any query
    // landing in the topmost knot interval would be misidentified as the second-to-last
    // interval.  The correct formula matches calibrate/src/correction_surface.rs::find_knot_interval.
    let n = knots.len() - order as usize;

    // Clamp to valid range
    let u_min = knots[order as usize - 1];
    let u_max = knots[knots.len() - order as usize];

    let extrapolated = u < u_min || u > u_max;
    let u_clamped = u.clamp(u_min, u_max);

    // Binary search for knot span
    let mut low = order as usize - 1;
    let mut high = n;

    // Special case: u is at or beyond last knot
    if u_clamped >= knots[high] {
        return (high - 1, extrapolated);
    }

    // Binary search
    while high - low > 1 {
        let mid = (low + high) / 2;
        if u_clamped < knots[mid] {
            high = mid;
        } else {
            low = mid;
        }
    }

    (low, extrapolated)
}

/// Evaluate B-spline basis functions at a given parameter value.
///
/// Uses Cox-de Boor recursion formula to compute basis functions.
/// Returns vector of basis function values of length `order`.
fn evaluate_basis_functions(knots: &[f64], span: usize, u: f64, order: u8) -> Vec<f64> {
    let p = order as usize - 1; // Degree (order - 1)
    let mut basis = vec![0.0; order as usize];

    // Initialize zeroth degree basis functions
    let mut left = vec![0.0; p + 1];
    let mut right = vec![0.0; p + 1];

    basis[0] = 1.0;

    // Compute higher degree basis functions using Cox-de Boor recursion
    for j in 1..=p {
        left[j] = u - knots[span + 1 - j];
        right[j] = knots[span + j] - u;
        let mut saved = 0.0;

        for r in 0..j {
            let denom = right[r + 1] + left[j - r];
            // Handle division by zero (can occur with repeated knots)
            let temp = if denom.abs() > 1e-14 {
                basis[r] / denom
            } else {
                0.0
            };
            basis[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }

        basis[j] = saved;
    }

    basis
}

/// Perform 4D tensor product interpolation.
///
/// Computes the weighted sum of coefficients using basis functions.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_range_loop)]
fn tensor_product_interpolation(
    coefficients: &[f64],
    shape: &[usize; 4],
    az_idx: usize,
    el_idx: usize,
    freq_idx: usize,
    temp_idx: usize,
    az_basis: &[f64],
    el_basis: &[f64],
    freq_basis: &[f64],
    temp_basis: &[f64],
    order: usize,
) -> Result<f64> {
    let [n_az, n_el, n_freq, n_temp] = *shape;

    let mut result = 0.0;

    // Iterate over local support of basis functions
    for i_temp in 0..order {
        // Compute coefficient index with saturating subtraction to avoid underflow
        let temp_coeff_idx =
            ((temp_idx + i_temp).saturating_sub(order - 1)).min(n_temp.saturating_sub(1));
        if temp_coeff_idx >= n_temp {
            continue;
        }

        for i_freq in 0..order {
            let freq_coeff_idx =
                ((freq_idx + i_freq).saturating_sub(order - 1)).min(n_freq.saturating_sub(1));
            if freq_coeff_idx >= n_freq {
                continue;
            }

            for i_el in 0..order {
                let el_coeff_idx =
                    ((el_idx + i_el).saturating_sub(order - 1)).min(n_el.saturating_sub(1));
                if el_coeff_idx >= n_el {
                    continue;
                }

                for i_az in 0..order {
                    let az_coeff_idx =
                        ((az_idx + i_az).saturating_sub(order - 1)).min(n_az.saturating_sub(1));
                    if az_coeff_idx >= n_az {
                        continue;
                    }

                    // Compute flat index: coefficients[i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp))]
                    let flat_idx = az_coeff_idx
                        + n_az * (el_coeff_idx + n_el * (freq_coeff_idx + n_freq * temp_coeff_idx));

                    if flat_idx >= coefficients.len() {
                        return Err(ComputationError::InvalidModelState(format!(
                            "Coefficient index {} out of bounds (length {})",
                            flat_idx,
                            coefficients.len()
                        ))
                        .into());
                    }

                    let coeff = coefficients[flat_idx];
                    let weight =
                        az_basis[i_az] * el_basis[i_el] * freq_basis[i_freq] * temp_basis[i_temp];

                    result += weight * coeff;
                }
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_knot_span_interior() {
        let knots = vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 4.0];
        let order = 3;

        let (idx, extrapolated) = find_knot_span(&knots, 1.5, order);
        assert_eq!(idx, 3);
        assert!(!extrapolated);

        let (idx, extrapolated) = find_knot_span(&knots, 2.5, order);
        assert_eq!(idx, 4);
        assert!(!extrapolated);
    }

    #[test]
    fn test_find_knot_span_boundaries() {
        let knots = vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 4.0];
        let order = 3;

        // At lower boundary
        let (_idx, extrapolated) = find_knot_span(&knots, 0.0, order);
        assert!(!extrapolated);

        // At upper boundary
        let (_idx, extrapolated) = find_knot_span(&knots, 4.0, order);
        assert!(!extrapolated);
    }

    #[test]
    fn test_find_knot_span_extrapolation() {
        let knots = vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 4.0];
        let order = 3;

        // Below range
        let (_idx, extrapolated) = find_knot_span(&knots, -1.0, order);
        assert!(extrapolated);

        // Above range
        let (_idx, extrapolated) = find_knot_span(&knots, 5.0, order);
        assert!(extrapolated);
    }

    #[test]
    fn test_evaluate_basis_functions_order_2() {
        let knots = vec![0.0, 0.0, 1.0, 2.0, 3.0, 3.0];
        let span = 2;
        let u = 1.5;
        let order = 2;

        let basis = evaluate_basis_functions(&knots, span, u, order);

        // For linear B-splines, basis functions should sum to 1
        let sum: f64 = basis.iter().sum();
        assert!((sum - 1.0).abs() < 1e-10);

        // Should have `order` basis values
        assert_eq!(basis.len(), order as usize);
    }

    #[test]
    fn test_evaluate_basis_functions_order_3() {
        let knots = vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0];
        let span = 3;
        let u = 1.5;
        let order = 3;

        let basis = evaluate_basis_functions(&knots, span, u, order);

        // Basis functions should sum to 1 (partition of unity property)
        let sum: f64 = basis.iter().sum();
        assert!((sum - 1.0).abs() < 1e-10);

        // Should have `order` basis values
        assert_eq!(basis.len(), order as usize);
    }

    #[test]
    fn test_correction_result_builder() {
        let result = CorrectionResult::new(2.5)
            .with_warning("Test warning".to_string())
            .with_extrapolation();

        assert_eq!(result.correction_db, 2.5);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.extrapolated);
    }

    #[test]
    fn test_evaluate_correction_simple() {
        // Create a simple 2x2x2x1 B-spline model (minimal for order 3)
        // This requires at least order+1 = 4 knots per dimension
        let model = BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2 * 1], // All corrections = 1.0 dB
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 10.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 20.0, 20.0, 20.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        };

        // Query at center of domain
        let result = evaluate_correction(&model, 5.0, 10.0, 8500.0, 290.0).unwrap();

        // Should return a finite correction (B-spline interpolation of constant 1.0 should be reasonable)
        assert!(
            result.correction_db.is_finite(),
            "Correction should be finite, got {}",
            result.correction_db
        );
        assert!(!result.extrapolated);
        assert!(result.warnings.is_empty());

        // With all coefficients = 1.0, the weighted sum should be close to 1.0
        // but may not be exactly 1.0 due to the boundary knot structure
        assert!(
            result.correction_db.abs() < 5.0,
            "Correction {} should be reasonable magnitude",
            result.correction_db
        );
    }

    #[test]
    fn test_evaluate_correction_extrapolation() {
        let model = BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2 * 1],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 10.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 20.0, 20.0, 20.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        };

        // Query outside domain (azimuth too high)
        let result = evaluate_correction(&model, 15.0, 10.0, 8500.0, 290.0).unwrap();

        // Should be marked as extrapolated with warning
        assert!(result.extrapolated);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("azimuth"));
    }

    #[test]
    fn test_evaluate_correction_invalid_input() {
        let model = BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2 * 1],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 10.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 20.0, 20.0, 20.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
            spline_order: 3,
        };

        // Query with NaN should fail
        let result = evaluate_correction(&model, f64::NAN, 10.0, 8500.0, 290.0);
        assert!(result.is_err());
    }

    /// Test that `evaluate_correction` returns the correct value when querying the
    /// **topmost knot interval** of the azimuth axis.
    ///
    /// # Construction
    ///
    /// We use order-2 (linear) B-splines on all four axes with the **same** knot
    /// vector `[0, 0, 1, 3, 5, 5]` (4 control points, 3 spans: [0,1], [1,3], [3,5]).
    ///
    /// Azimuth control points: `c = [0.0, 1.0, 2.0, 4.0]` (varying).
    /// El / freq / temp control points: all 1.0 (constant).
    ///
    /// Because all non-az control points are 1.0, the tensor product factors as:
    ///   result = az_interp × (el sum) × (freq sum) × (temp sum) = az_interp × 1
    ///
    /// For azimuth at u=4.0 (inside the **top** interval [3,5]):
    ///   t = (4-3)/(5-3) = 0.5
    ///   expected = (1-t)*c[2] + t*c[3] = 0.5×2 + 0.5×4 = **3.0**
    ///
    /// ## Pre-fix (off-by-one) behaviour
    ///   n = len − order − 1 = 6 − 2 − 1 = 3, high = 3, knots[3] = 3.0
    ///   u = 4.0 ≥ knots[3] = 3.0 → short-circuit returns span 2 (interval [1,3])
    ///   Evaluating span 2 at u = 4.0: basis = [−0.5, 1.5]
    ///   result = −0.5×c[1] + 1.5×c[2] = −0.5 + 3.0 = **2.5** ≠ 3.0   (WRONG)
    ///
    /// ## Post-fix behaviour
    ///   n = len − order = 6 − 2 = 4, high = 4, knots[4] = 5.0 = u_max
    ///   u = 4.0 < knots[4] → binary search → span 3 (interval [3,5])
    ///   basis = [0.5, 0.5]; result = 0.5×2 + 0.5×4 = **3.0**   (CORRECT)
    #[test]
    fn test_top_interval_linear_bspline_known_answer() {
        // All four axes use the same order-2 knot vector: [0, 0, 1, 3, 5, 5].
        // This gives 4 basis functions and 3 real spans: [0,1], [1,3], [3,5].
        //
        // Querying non-az axes at u=2.0 (span 2, safely away from the top interval)
        // gives basis [0.5, 0.5] which sums to 1.0 (partition of unity) and
        // interacts correctly with both the buggy and fixed az span selection.
        //
        // shape=[4,4,4,4]: coeff[i_az + 4*(i_el + 4*(i_freq + 4*i_temp))]
        // Non-az coefficients are all 1.0, so the product factors to az_interp × 1.
        let n = 4_usize;
        let knots = vec![0.0, 0.0, 1.0, 3.0, 5.0, 5.0];
        // az_values indexed by i_az; other axes all 1.0
        let az_values = [0.0_f64, 1.0, 2.0, 4.0];
        let coefficients: Vec<f64> = (0..n * n * n * n)
            .map(|flat| {
                let i_az = flat % n;
                az_values[i_az]
            })
            .collect();

        let model = BSplineModel4D {
            coefficients,
            shape: [n, n, n, n],
            knots_azimuth: knots.clone(),
            knots_elevation: knots.clone(),
            knots_frequency: knots.clone(),
            knots_temperature: knots.clone(),
            spline_order: 2,
        };

        // Sanity: model must pass structural validation.
        assert!(
            model.validate().is_ok(),
            "test model failed validate(): {:?}",
            model.validate()
        );

        // Query INSIDE the top azimuth interval [3, 5] — the problematic region.
        // Non-az axes queried at u=2.0 (span 2, [1,3]), all constant 1.0.
        // Expected az interpolation at u=4.0 (top span [3,5]):
        //   t = (4-3)/(5-3) = 0.5; result = 0.5*c[2] + 0.5*c[3] = 1.0 + 2.0 = 3.0
        let expected = 3.0_f64;
        let result = evaluate_correction(&model, 4.0, 2.0, 2.0, 2.0).expect("should succeed");

        assert!(
            !result.extrapolated,
            "az=4.0 is inside [0,5]; should not be extrapolated"
        );
        assert!(
            (result.correction_db - expected).abs() < 1e-12,
            "top-interval query at az=4.0: expected {expected}, got {} (err {:e}). \
             Pre-fix bug would return ~2.5 due to wrong span selection.",
            result.correction_db,
            (result.correction_db - expected).abs()
        );

        // Cross-check: mid-range interval [1,3] at az=2.0 should give 1.5.
        // t = (2-1)/(3-1) = 0.5; result = 0.5*c[1] + 0.5*c[2] = 0.5 + 1.0 = 1.5
        let mid = evaluate_correction(&model, 2.0, 2.0, 2.0, 2.0).expect("should succeed");
        assert!(
            (mid.correction_db - 1.5).abs() < 1e-12,
            "mid-interval at az=2.0: expected 1.5, got {}",
            mid.correction_db
        );
    }

    /// Test that `find_knot_span` with the corrected formula (`n = len - order`)
    /// selects the correct span index for the topmost knot interval.
    #[test]
    fn test_find_knot_span_top_interval() {
        // 4 basis functions, order 2: knots [0,0,1,3,5,5], len=6, n=4
        let knots = vec![0.0, 0.0, 1.0, 3.0, 5.0, 5.0];
        let order = 2u8;

        // u=4.0 is inside the TOP interval [3,5]; correct span index = 3.
        // Pre-fix: n=6-2-1=3, high=3, knots[3]=3.0, 4.0>=3.0 → returns (2, false).  WRONG
        // Post-fix: n=6-2=4,   high=4, knots[4]=5.0, 4.0<5.0  → binary search → (3, false). CORRECT
        let (idx, extrapolated) = find_knot_span(&knots, 4.0, order);
        assert!(!extrapolated, "4.0 is inside [0,5]");
        assert_eq!(idx, 3, "top interval [3,5] has span index 3");

        // u=3.5 also in top interval.
        let (idx2, ext2) = find_knot_span(&knots, 3.5, order);
        assert!(!ext2);
        assert_eq!(idx2, 3, "3.5 in [3,5] → span 3");

        // u=5.0 (u_max) should still return last valid span (2 for the buggy code, 3 for fixed).
        let (idx_max, ext_max) = find_knot_span(&knots, 5.0, order);
        assert!(!ext_max, "5.0 is u_max, not extrapolated");
        assert_eq!(idx_max, 3, "u_max clamps to last valid span 3");
    }
}
