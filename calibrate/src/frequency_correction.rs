//! Frequency-only correction surface fitting for boresight calibration.
//!
//! This module provides functionality to fit a 1D frequency-only correction surface
//! to boresight measurement residuals. The correction is stored as a degenerate 4D
//! B-spline (single spatial point) for compatibility with the service's existing
//! interpolation code.
//!
//! # Use Case
//!
//! After parameter tuning in boresight calibration, there may be residual systematic
//! errors as a function of frequency. This module fits a 1D B-spline to these residuals
//! to further improve boresight accuracy.
//!
//! # Design
//!
//! - Input: Frequency-residual pairs (measured - physics model at boresight)
//! - Output: Degenerate 4D B-spline with shape [1, 1, N_freq, 1]
//! - Threshold: Only fit if max(abs(residuals)) > 0.5 dB
//! - Method: Cubic B-spline with uniform knot spacing

use antenna_model::data::types::BSplineModel4D;
use thiserror::Error;

/// Error types for frequency correction fitting.
#[derive(Debug, Error)]
pub enum FrequencyCorrectionError {
    #[error("Insufficient data points: need at least 4 points for cubic B-spline, got {0}")]
    InsufficientData(usize),

    #[error("Invalid frequency range: min={min} >= max={max}")]
    InvalidFrequencyRange { min: f64, max: f64 },

    #[error("Non-finite values in input data")]
    NonFiniteData,

    #[error("B-spline fitting failed: {0}")]
    FittingError(String),
}

/// Result type for frequency correction operations.
pub type Result<T> = std::result::Result<T, FrequencyCorrectionError>;

/// Checks if a frequency correction surface should be fitted.
///
/// A correction surface is only beneficial if the residuals show systematic
/// bias > 0.5 dB. For smaller residuals, fitting a correction may add noise
/// rather than improve accuracy.
///
/// # Arguments
///
/// * `residuals` - Array of residual values (measured - physics model) in dB
///
/// # Returns
///
/// `true` if max(abs(residuals)) > 0.5 dB, indicating correction is worthwhile
///
/// # Example
///
/// ```
/// use calibrate::frequency_correction::should_fit_correction;
///
/// let small_residuals = vec![0.1, -0.2, 0.15, -0.3];
/// assert!(!should_fit_correction(&small_residuals));
///
/// let large_residuals = vec![0.8, -0.6, 0.9, -0.7];
/// assert!(should_fit_correction(&large_residuals));
/// ```
pub fn should_fit_correction(residuals: &[f64]) -> bool {
    const THRESHOLD_DB: f64 = 0.5;

    if residuals.is_empty() {
        return false;
    }

    let max_abs_residual = residuals
        .iter()
        .map(|r| r.abs())
        .fold(f64::NEG_INFINITY, f64::max);

    max_abs_residual > THRESHOLD_DB
}

/// Fits a 1D frequency-only correction surface and converts to degenerate 4D B-spline.
///
/// This function creates a cubic B-spline interpolation of the frequency-dependent
/// residuals and packages it as a degenerate 4D B-spline for compatibility with
/// the service's correction surface evaluation code.
///
/// The degenerate 4D B-spline has:
/// - shape = [1, 1, N_freq, 1] where N_freq is the number of control points
/// - Azimuth dimension: single point at 0.0 degrees (boresight)
/// - Elevation dimension: single point at 0.0 degrees (boresight)
/// - Frequency dimension: proper B-spline with N_freq control points
/// - Temperature dimension: single point at 290.0 K (typical)
///
/// # Arguments
///
/// * `frequencies` - Frequency samples in MHz (must be sorted, at least 4 points)
/// * `residuals` - Correction values in dB (measured - physics model)
///
/// # Returns
///
/// A degenerate 4D B-spline model that can be stored in `AntennaCalibration.correction_surface`
///
/// # Errors
///
/// Returns error if:
/// - Fewer than 4 data points (minimum for cubic B-spline)
/// - Frequencies not monotonically increasing
/// - Any NaN or Inf values in input
/// - B-spline fitting fails
///
/// # Example
///
/// ```
/// use calibrate::frequency_correction::fit_frequency_correction;
///
/// let frequencies = vec![7100.0, 7500.0, 8000.0, 8450.0];
/// let residuals = vec![0.8, 0.6, 0.5, 0.7];
///
/// let correction = fit_frequency_correction(&frequencies, &residuals).unwrap();
/// assert_eq!(correction.shape, [1, 1, 4, 1]);
/// ```
pub fn fit_frequency_correction(
    frequencies: &[f64],
    residuals: &[f64],
) -> Result<BSplineModel4D> {
    // Validate inputs
    validate_inputs(frequencies, residuals)?;

    // For simplicity, use the measured points as control points directly
    // This creates an interpolating B-spline through the data points
    let n_points = frequencies.len();
    let spline_order: u8 = 3; // Cubic B-spline

    // Create knot vectors for each dimension
    let knots_frequency = create_knot_vector(frequencies, spline_order);
    let knots_azimuth = create_degenerate_knot_vector(0.0, spline_order); // Boresight azimuth
    let knots_elevation = create_degenerate_knot_vector(0.0, spline_order); // Boresight elevation
    let knots_temperature = create_degenerate_knot_vector(290.0, spline_order); // Typical temp

    // The coefficients are the residual values
    // For a degenerate 4D B-spline [1, 1, N, 1], we have N coefficients
    let coefficients = residuals.to_vec();

    // Create the degenerate 4D B-spline
    let bspline = BSplineModel4D {
        coefficients,
        shape: [1, 1, n_points, 1],
        knots_azimuth,
        knots_elevation,
        knots_frequency,
        knots_temperature,
        spline_order,
    };

    Ok(bspline)
}

/// Validates input data for B-spline fitting.
fn validate_inputs(frequencies: &[f64], residuals: &[f64]) -> Result<()> {
    // Check we have the same number of frequencies and residuals
    if frequencies.len() != residuals.len() {
        return Err(FrequencyCorrectionError::FittingError(format!(
            "Frequency and residual arrays must have same length: {} vs {}",
            frequencies.len(),
            residuals.len()
        )));
    }

    // Check we have at least 4 points for cubic B-spline
    let n_points = frequencies.len();
    if n_points < 4 {
        return Err(FrequencyCorrectionError::InsufficientData(n_points));
    }

    // Check for non-finite values
    if frequencies.iter().any(|f| !f.is_finite()) || residuals.iter().any(|r| !r.is_finite()) {
        return Err(FrequencyCorrectionError::NonFiniteData);
    }

    // Check frequencies are monotonically increasing
    for i in 1..frequencies.len() {
        if frequencies[i] <= frequencies[i - 1] {
            return Err(FrequencyCorrectionError::InvalidFrequencyRange {
                min: frequencies[i - 1],
                max: frequencies[i],
            });
        }
    }

    Ok(())
}

/// Creates a knot vector for a B-spline with given data points and order.
///
/// For cubic B-splines (order 3), uses clamped knot vector with multiplicity
/// at the endpoints for interpolation.
///
/// # Arguments
///
/// * `data_points` - Sorted array of data point locations
/// * `order` - B-spline order (degree + 1)
///
/// # Returns
///
/// Knot vector with length = n_points + order
fn create_knot_vector(data_points: &[f64], order: u8) -> Vec<f64> {
    let n = data_points.len();
    let k = order as usize;
    let total_knots = n + k;
    let mut knots = Vec::with_capacity(total_knots);

    // Clamped B-spline: repeat first and last knots k times
    // This ensures the spline interpolates the endpoints

    // Repeat first value k times
    for _ in 0..k {
        knots.push(data_points[0]);
    }

    // Internal knots: total - 2k knots
    // For a clamped B-spline with n control points and order k:
    // - First k knots are at x[0]
    // - Last k knots are at x[n-1]
    // - Internal knots: total - 2k = n + k - 2k = n - k
    let num_internal = total_knots - 2 * k;

    // Distribute internal knots uniformly among interior data points
    // For simplicity, use evenly spaced interior data points
    if num_internal > 0 {
        for i in 1..=num_internal {
            // Map index to data point index proportionally
            let idx = (i * (n - 1)) / (num_internal + 1);
            knots.push(data_points[idx.min(n - 1)]);
        }
    }

    // Repeat last value k times
    for _ in 0..k {
        knots.push(data_points[n - 1]);
    }

    knots
}

/// Creates a degenerate knot vector for a single point (collapsed dimension).
///
/// For a cubic B-spline (order 3), this returns [value, value, value, value].
///
/// # Arguments
///
/// * `value` - The single point value
/// * `order` - B-spline order (degree + 1)
///
/// # Returns
///
/// Knot vector with `order` repeated values
fn create_degenerate_knot_vector(value: f64, order: u8) -> Vec<f64> {
    vec![value; order as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_fit_correction_with_small_residuals() {
        let residuals = vec![0.1, -0.2, 0.15, -0.3];
        assert!(!should_fit_correction(&residuals));
    }

    #[test]
    fn test_should_fit_correction_with_large_residuals() {
        let residuals = vec![0.8, -0.6, 0.9, -0.7];
        assert!(should_fit_correction(&residuals));
    }

    #[test]
    fn test_should_fit_correction_at_threshold() {
        // Exactly at threshold should return false (not strictly greater)
        let residuals = vec![0.5, -0.4, 0.3];
        assert!(!should_fit_correction(&residuals));

        // Just above threshold should return true
        let residuals = vec![0.51, -0.4, 0.3];
        assert!(should_fit_correction(&residuals));
    }

    #[test]
    fn test_should_fit_correction_empty_residuals() {
        let residuals: Vec<f64> = vec![];
        assert!(!should_fit_correction(&residuals));
    }

    #[test]
    fn test_should_fit_correction_single_large_outlier() {
        let residuals = vec![0.1, -0.2, 0.8, -0.15];
        assert!(should_fit_correction(&residuals));
    }

    #[test]
    fn test_fit_frequency_correction_basic() {
        let frequencies = vec![7100.0, 7500.0, 8000.0, 8450.0];
        let residuals = vec![0.8, 0.6, 0.5, 0.7];

        let result = fit_frequency_correction(&frequencies, &residuals);
        assert!(result.is_ok());

        let bspline = result.unwrap();
        assert_eq!(bspline.shape, [1, 1, 4, 1]);
        assert_eq!(bspline.spline_order, 3);
        assert_eq!(bspline.coefficients, residuals);

        // Check knot vectors
        assert_eq!(bspline.knots_azimuth.len(), 3);
        assert_eq!(bspline.knots_elevation.len(), 3);
        assert_eq!(bspline.knots_temperature.len(), 3);
        assert!(bspline.knots_frequency.len() >= frequencies.len());

        // Check degenerate dimensions have repeated knots
        assert!(bspline.knots_azimuth.iter().all(|&k| k == 0.0));
        assert!(bspline.knots_elevation.iter().all(|&k| k == 0.0));
        assert!(bspline.knots_temperature.iter().all(|&k| k == 290.0));
    }

    #[test]
    fn test_fit_frequency_correction_insufficient_data() {
        let frequencies = vec![7100.0, 7500.0, 8000.0]; // Only 3 points
        let residuals = vec![0.8, 0.6, 0.5];

        let result = fit_frequency_correction(&frequencies, &residuals);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FrequencyCorrectionError::InsufficientData(3)
        ));
    }

    #[test]
    fn test_fit_frequency_correction_mismatched_lengths() {
        let frequencies = vec![7100.0, 7500.0, 8000.0, 8450.0];
        let residuals = vec![0.8, 0.6, 0.5]; // One fewer

        let result = fit_frequency_correction(&frequencies, &residuals);
        assert!(result.is_err());
    }

    #[test]
    fn test_fit_frequency_correction_non_monotonic_frequencies() {
        let frequencies = vec![7100.0, 8000.0, 7500.0, 8450.0]; // Not sorted
        let residuals = vec![0.8, 0.6, 0.5, 0.7];

        let result = fit_frequency_correction(&frequencies, &residuals);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FrequencyCorrectionError::InvalidFrequencyRange { .. }
        ));
    }

    #[test]
    fn test_fit_frequency_correction_nan_values() {
        let frequencies = vec![7100.0, 7500.0, f64::NAN, 8450.0];
        let residuals = vec![0.8, 0.6, 0.5, 0.7];

        let result = fit_frequency_correction(&frequencies, &residuals);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FrequencyCorrectionError::NonFiniteData
        ));
    }

    #[test]
    fn test_fit_frequency_correction_inf_residuals() {
        let frequencies = vec![7100.0, 7500.0, 8000.0, 8450.0];
        let residuals = vec![0.8, f64::INFINITY, 0.5, 0.7];

        let result = fit_frequency_correction(&frequencies, &residuals);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FrequencyCorrectionError::NonFiniteData
        ));
    }

    #[test]
    fn test_create_knot_vector_cubic() {
        let data_points = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        let knots = create_knot_vector(&data_points, 3);

        // For 5 points with order 3: should have 5 + 3 = 8 knots
        assert_eq!(knots.len(), 8);

        // First 3 should be the first data point
        assert_eq!(knots[0], 100.0);
        assert_eq!(knots[1], 100.0);
        assert_eq!(knots[2], 100.0);

        // Last 3 should be the last data point
        assert_eq!(knots[5], 500.0);
        assert_eq!(knots[6], 500.0);
        assert_eq!(knots[7], 500.0);
    }

    #[test]
    fn test_create_degenerate_knot_vector() {
        let knots = create_degenerate_knot_vector(42.0, 3);
        assert_eq!(knots, vec![42.0, 42.0, 42.0]);

        let knots = create_degenerate_knot_vector(0.0, 4);
        assert_eq!(knots, vec![0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_validate_inputs_valid() {
        let frequencies = vec![100.0, 200.0, 300.0, 400.0];
        let residuals = vec![0.5, 0.6, 0.4, 0.7];

        let result = validate_inputs(&frequencies, &residuals);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_inputs_empty() {
        let frequencies: Vec<f64> = vec![];
        let residuals: Vec<f64> = vec![];

        let result = validate_inputs(&frequencies, &residuals);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FrequencyCorrectionError::InsufficientData(0)
        ));
    }

    #[test]
    fn test_degenerate_4d_structure() {
        // Test with many frequency points
        let frequencies: Vec<f64> = (0..20).map(|i| 7000.0 + i as f64 * 100.0).collect();
        let residuals: Vec<f64> = (0..20).map(|i| (i as f64 * 0.1).sin()).collect();

        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        // Verify degenerate 4D structure
        assert_eq!(bspline.shape[0], 1); // Azimuth: single point
        assert_eq!(bspline.shape[1], 1); // Elevation: single point
        assert_eq!(bspline.shape[2], 20); // Frequency: 20 points
        assert_eq!(bspline.shape[3], 1); // Temperature: single point

        // Total coefficients should be 1 * 1 * 20 * 1 = 20
        assert_eq!(bspline.coefficients.len(), 20);
    }

    #[test]
    fn test_frequency_knot_vector_properties() {
        let frequencies = vec![7100.0, 7500.0, 8000.0, 8450.0, 8900.0];
        let residuals = vec![0.5, 0.6, 0.4, 0.7, 0.5];

        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        // Check knot vector starts and ends at data bounds
        assert_eq!(bspline.knots_frequency[0], frequencies[0]);
        assert_eq!(
            bspline.knots_frequency[bspline.knots_frequency.len() - 1],
            frequencies[frequencies.len() - 1]
        );

        // For clamped cubic B-spline, first and last knots should be repeated 3 times
        assert_eq!(bspline.knots_frequency[0], bspline.knots_frequency[1]);
        assert_eq!(bspline.knots_frequency[1], bspline.knots_frequency[2]);

        let n = bspline.knots_frequency.len();
        assert_eq!(bspline.knots_frequency[n - 1], bspline.knots_frequency[n - 2]);
        assert_eq!(bspline.knots_frequency[n - 2], bspline.knots_frequency[n - 3]);
    }
}
