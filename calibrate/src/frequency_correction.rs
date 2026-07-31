//! Frequency-only correction surface fitting for boresight calibration.
//!
//! This module provides functionality to fit a 1D frequency-only correction surface
//! to boresight measurement residuals. The correction is stored as a 4D B-spline
//! whose azimuth, elevation and temperature axes are *flat* — constant, but with a
//! real span — for compatibility with the service's existing interpolation code.
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
//! - Output: 4D B-spline that varies only along frequency, shape
//!   `[order+1, order+1, N_freq, order+1]`
//! - Threshold: Only fit if max(abs(residuals)) > 0.5 dB
//! - Method: Cubic B-spline with uniform knot spacing
//!
//! # Why the collapsed axes are *flat*, not degenerate
//!
//! Until 2026-07-31 the three non-frequency axes were built as `order` equal
//! knots over a single coefficient layer — a genuinely degenerate axis. Such an
//! artifact **could not be loaded by the service at all**: `BSplineModel4D::
//! validate` requires `knots.len() >= shape + order` per axis and the loader
//! runs it on every artifact, so any boresight run whose residuals tripped the
//! 0.5 dB threshold produced a `.bin` the service rejected. Lengthening the
//! degenerate vectors would have satisfied the length check while leaving the
//! evaluable span `[knots[order-1], knots[len-order]]` empty, so the axes are
//! now built by [`artifact_export::flat_axis`](crate::artifact_export) — the
//! same construction full mode uses for its temperature axis — which replicates
//! the coefficient layer over a real interval. See roadmap unit D13.

use antenna_model::data::types::BSplineModel4D;
use thiserror::Error;

use crate::artifact_export::flat_axis;

/// Span of the flat azimuth axis, in degrees.
///
/// The three constants below bound axes the fitted surface is **constant**
/// along, so their only job is to cover every value the service can ever query
/// — a query landing outside a knot span is reported as extrapolated, and there
/// is no interpolation error here to warn about. Azimuth is the full circle
/// because `coordinates_3d::normalize_azimuth_deg` maps into `[0, 360)`.
///
/// The claim that this correction is only *measured* at boresight is carried by
/// the artifact's `calibration_coverage` (azimuth and elevation both `0..=0`),
/// which is where `service::evaluator::is_in_coverage` enforces it — not by
/// pinching these knot spans.
const AZIMUTH_AXIS_DEG: (f64, f64) = (0.0, 360.0);

/// Span of the flat elevation axis, in degrees. Elevation reaches the service's
/// correction surface as a **polar angle from boresight** (0° on axis), so the
/// full range is `[0, 180]`. See [`AZIMUTH_AXIS_DEG`].
const ELEVATION_AXIS_DEG: (f64, f64) = (0.0, 180.0);

/// Span of the flat temperature axis, in Kelvin. The evaluator queries the
/// correction with `validity_ranges.temperature_const`, which boresight mode
/// sets to 290 K; this bracket covers any system noise temperature that value
/// could plausibly take. See [`AZIMUTH_AXIS_DEG`].
const TEMPERATURE_AXIS_K: (f64, f64) = (0.0, 1000.0);

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

/// Fits a 1D frequency-only correction surface and converts to a 4D B-spline
/// that is flat in every axis but frequency.
///
/// This function creates a cubic B-spline of the frequency-dependent residuals
/// and packages it as a `BSplineModel4D` for the service's correction-surface
/// evaluation code.
///
/// The resulting 4D B-spline has:
/// - shape = `[F, F, N_freq, F]` with `F = spline_order + 1`, where `N_freq` is
///   the number of frequency control points
/// - Frequency dimension: proper B-spline with `N_freq` control points
/// - Azimuth, elevation and temperature: **flat** axes (identical coefficient
///   layers over a real span, see the module docs) so the surface is exactly
///   constant along them
///
/// # Arguments
///
/// * `frequencies` - Frequency samples in MHz (must be sorted, at least 4 points)
/// * `residuals` - Correction values in dB (measured - physics model)
///
/// # Returns
///
/// A 4D B-spline model that can be stored in `AntennaCalibration.correction_surface`
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
/// assert_eq!(correction.shape, [4, 4, 4, 4]); // flat, flat, 4 frequencies, flat
/// correction.validate().expect("the service loader must accept this");
/// ```
pub fn fit_frequency_correction(frequencies: &[f64], residuals: &[f64]) -> Result<BSplineModel4D> {
    // Validate inputs
    validate_inputs(frequencies, residuals)?;

    // For simplicity, use the measured points as control points directly
    let n_freq = frequencies.len();
    let spline_order: u8 = 3; // Cubic B-spline
    let order = spline_order as usize;

    let knots_frequency = create_knot_vector(frequencies, spline_order);

    // The three axes this correction does not vary along. Flat, not degenerate:
    // identical coefficient layers over a real span (see the module docs).
    let (n_az, knots_azimuth) = flat_axis(AZIMUTH_AXIS_DEG.0, AZIMUTH_AXIS_DEG.1, order);
    let (n_el, knots_elevation) = flat_axis(ELEVATION_AXIS_DEG.0, ELEVATION_AXIS_DEG.1, order);
    let (n_temp, knots_temperature) = flat_axis(TEMPERATURE_AXIS_K.0, TEMPERATURE_AXIS_K.1, order);

    // Replicate the residual control points across every flat layer, in the 4D
    // flat-index layout the service evaluates:
    //   idx = i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp))
    let mut coefficients = vec![0.0_f64; n_az * n_el * n_freq * n_temp];
    for i_temp in 0..n_temp {
        for (i_freq, &residual) in residuals.iter().enumerate() {
            for i_el in 0..n_el {
                for i_az in 0..n_az {
                    let idx = i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp));
                    coefficients[idx] = residual;
                }
            }
        }
    }

    let bspline = BSplineModel4D {
        coefficients,
        shape: [n_az, n_el, n_freq, n_temp],
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

#[cfg(test)]
mod tests {
    use super::*;
    use antenna_model::model::evaluate_correction;

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
        // order + 1 = 4 layers on each flat axis; 4 frequency control points.
        assert_eq!(bspline.shape, [4, 4, 4, 4]);
        assert_eq!(bspline.spline_order, 3);
        assert_eq!(bspline.coefficients.len(), 4 * 4 * 4 * 4);

        // Knot vectors: 2*order + 1 on each flat axis, n + order on frequency.
        assert_eq!(bspline.knots_azimuth.len(), 7);
        assert_eq!(bspline.knots_elevation.len(), 7);
        assert_eq!(bspline.knots_temperature.len(), 7);
        assert!(bspline.knots_frequency.len() >= frequencies.len());

        // Each flat axis spans its full documented interval.
        assert_eq!(bspline.knots_azimuth.first(), Some(&AZIMUTH_AXIS_DEG.0));
        assert_eq!(bspline.knots_azimuth.last(), Some(&AZIMUTH_AXIS_DEG.1));
        assert_eq!(bspline.knots_elevation.first(), Some(&ELEVATION_AXIS_DEG.0));
        assert_eq!(bspline.knots_elevation.last(), Some(&ELEVATION_AXIS_DEG.1));
        assert_eq!(
            bspline.knots_temperature.first(),
            Some(&TEMPERATURE_AXIS_K.0)
        );
        assert_eq!(
            bspline.knots_temperature.last(),
            Some(&TEMPERATURE_AXIS_K.1)
        );
    }

    /// Regression pin, inverted 2026-07-31 (roadmap D13; defect filed 2026-07-30
    /// by the D15 review).
    ///
    /// The boresight-mode frequency correction used to be **structurally
    /// unloadable**: its azimuth/elevation/temperature axes were `order` equal
    /// knots over one coefficient layer, and `BSplineModel4D::validate` requires
    /// `knots.len() >= shape + order` on every axis. The service loader runs that
    /// validation on every artifact (`AntennaCalibration::validate` →
    /// `correction.validate()`), so any boresight run whose residuals tripped the
    /// 0.5 dB fitting threshold wrote a `.bin` the service refused to load.
    ///
    /// This test used to assert `is_err()` to pin the defect. It now asserts the
    /// contract the fix established, and must never be relaxed back.
    #[test]
    fn frequency_correction_is_accepted_by_the_service_side_validator() {
        let frequencies = vec![7100.0, 7500.0, 8000.0, 8450.0];
        let residuals = vec![0.8, 0.6, 0.5, 0.7];
        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        bspline.validate().expect(
            "fit_frequency_correction must produce a surface the service loader accepts; \
             the degenerate-axis defect has regressed",
        );
    }

    /// The three collapsed axes must be *flat*, not merely valid: the fitted
    /// correction is a function of frequency alone, so moving along azimuth,
    /// elevation or temperature must not change the evaluated value by so much
    /// as a rounding step.
    ///
    /// This is the assertion that a "just lengthen the degenerate knot vectors"
    /// fix would fail — an axis with an empty span evaluates its basis to zero
    /// and collapses the whole correction to 0 dB.
    #[test]
    fn collapsed_axes_are_flat_not_just_valid() {
        let frequencies = vec![3700.0, 3950.0, 4200.0, 5925.0, 6175.0, 6425.0];
        let residuals = vec![0.9, 0.7, 0.55, -0.8, -0.95, -0.6];
        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        let freq = 4000.0;
        let reference = evaluate_correction(&bspline, 0.0, 0.0, freq, 290.0)
            .expect("evaluate at boresight")
            .correction_db;

        assert!(
            reference.abs() > 0.1,
            "the reference value is ~0 dB, so a collapsed-to-zero surface would \
             pass the comparisons below vacuously; got {reference} dB"
        );

        for az in [0.0, 1.0, 45.0, 180.0, 359.0, 360.0] {
            for el in [0.0, 0.5, 30.0, 90.0, 179.0, 180.0] {
                for temp in [1.0, 100.0, 290.0, 500.0, 999.0] {
                    let result = evaluate_correction(&bspline, az, el, freq, temp)
                        .expect("evaluate off the collapsed axes' origin");
                    assert!(
                        (result.correction_db - reference).abs() < 1e-12,
                        "correction must not depend on az/el/temperature: \
                         ({az}, {el}, {temp}) gave {} dB vs {reference} dB at the origin",
                        result.correction_db
                    );
                    assert!(
                        !result.extrapolated,
                        "({az}, {el}, {temp}) is inside every flat axis span but was \
                         reported as extrapolated"
                    );
                }
            }
        }
    }

    /// A clamped B-spline interpolates its first and last control points, so the
    /// correction reproduces the measured residual exactly at the endpoints of
    /// the frequency sweep. (Interior control points are *not* interpolated —
    /// see the note in `frequency_control_points_are_not_interpolated`.)
    #[test]
    fn correction_reproduces_the_endpoint_residuals() {
        let frequencies = vec![3700.0, 3950.0, 4200.0, 5925.0, 6175.0, 6425.0];
        let residuals = vec![0.9, 0.7, 0.55, -0.8, -0.95, -0.6];
        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        for (freq, expected) in [
            (frequencies[0], residuals[0]),
            (
                frequencies[frequencies.len() - 1],
                residuals[residuals.len() - 1],
            ),
        ] {
            let got = evaluate_correction(&bspline, 0.0, 0.0, freq, 290.0)
                .expect("evaluate at a sweep endpoint")
                .correction_db;
            assert!(
                (got - expected).abs() < 1e-9,
                "at {freq} MHz the correction should reproduce the endpoint residual \
                 {expected} dB, got {got} dB"
            );
        }
    }

    /// Known limitation, pinned so it is not mistaken for a regression: the
    /// residuals are used **as control points**, not fitted, so at interior
    /// frequencies the correction is a smoothed version of the residual sequence
    /// rather than an interpolant of it. The deviation is bounded by how fast the
    /// residuals vary between samples. Not fixed here (this unit is about the
    /// artifact being loadable at all); recorded on roadmap D13.
    #[test]
    fn frequency_control_points_are_not_interpolated() {
        // A deliberately spiky residual sequence maximises the smoothing gap.
        let frequencies = vec![1000.0, 1100.0, 1200.0, 1300.0, 1400.0];
        let residuals = vec![0.0, 2.0, 0.0, 2.0, 0.0];
        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        let got = evaluate_correction(&bspline, 0.0, 0.0, frequencies[1], 290.0)
            .expect("evaluate at an interior control point")
            .correction_db;

        assert!(
            (got - residuals[1]).abs() > 0.1,
            "this test documents that interior residuals are NOT interpolated; if the \
             fitter has been changed to a true interpolating/least-squares fit, delete \
             this test rather than loosening it (got {got} dB at {} MHz for residual {})",
            frequencies[1],
            residuals[1]
        );
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
    fn test_flat_4d_structure() {
        // Test with many frequency points
        let frequencies: Vec<f64> = (0..20).map(|i| 7000.0 + i as f64 * 100.0).collect();
        let residuals: Vec<f64> = (0..20).map(|i| (i as f64 * 0.1).sin()).collect();

        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        // Flat axes carry order + 1 = 4 identical layers; frequency carries the data.
        assert_eq!(bspline.shape[0], 4); // Azimuth: flat
        assert_eq!(bspline.shape[1], 4); // Elevation: flat
        assert_eq!(bspline.shape[2], 20); // Frequency: 20 control points
        assert_eq!(bspline.shape[3], 4); // Temperature: flat

        assert_eq!(bspline.coefficients.len(), 4 * 4 * 20 * 4);
        bspline.validate().expect("structure must stay loadable");

        // Every flat layer of a given frequency index carries the same residual.
        let [n_az, n_el, n_freq, n_temp] = bspline.shape;
        for (i_freq, &residual) in residuals.iter().enumerate() {
            for i_temp in 0..n_temp {
                for i_el in 0..n_el {
                    for i_az in 0..n_az {
                        let idx = i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp));
                        assert_eq!(
                            bspline.coefficients[idx], residual,
                            "layer ({i_az}, {i_el}, {i_freq}, {i_temp}) is not a replica"
                        );
                    }
                }
            }
        }
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
        assert_eq!(
            bspline.knots_frequency[n - 1],
            bspline.knots_frequency[n - 2]
        );
        assert_eq!(
            bspline.knots_frequency[n - 2],
            bspline.knots_frequency[n - 3]
        );
    }
}
