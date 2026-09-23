//! Frequency-only correction surface fitting for boresight calibration.
//!
//! This module provides functionality to fit a 1D frequency-only correction surface
//! to boresight measurement residuals. The correction is stored in schema 5's 4D
//! wire type, whose E-clock, E-cone, and synthetic temperature axes are *flat* —
//! constant, but with a real span. Runtime evaluation adapts it to three dimensions.
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
//! - Method: **quadratic** B-spline (order 3) whose control points are the residuals
//!   themselves, with interior knots at evenly spaced interior measured frequencies
//!
//! # The spline is quadratic, and that is deliberate for now
//!
//! In this repository `order = degree + 1`, so the order 3 written here is **degree 2**.
//! Until GitHub issue #95 this module's comments called it cubic, and the served numbers
//! have always been quadratic. #95 preserved that behavior and fixed the description:
//! changing the boresight fit to cubic (order 4) changes served values, which is a separate
//! decision, not an incidental side effect of moving constructors. Full-mode fitting is
//! cubic, order 4 (`CorrectionSurfaceParams::shipped`).
//!
//! # Why the collapsed axes are *flat*, not degenerate
//!
//! The E-clock and E-cone axes are [`ClampedAxis::flat`]: `order + 1` identical coefficient
//! layers over a real span, so the surface is exactly constant along them. Until 2026-07-31
//! they were one layer over `order` equal knots — an axis with nothing to evaluate over,
//! which the service loader rejected, so every boresight run that tripped the 0.5 dB
//! threshold wrote a `.bin` the service refused (roadmap D13). The core layout now refuses
//! that construction itself (issue #95).

use antenna_core::data::types::BSplineModel4D;
use antenna_core::model::{ClampedAxis, CorrectionSurfaceLayout, FittedCorrectionSurface};
use thiserror::Error;

/// Span of the flat E-clock axis, in degrees.
///
/// The three constants below bound axes the fitted surface is **constant**
/// along, so their only job is to cover every value the service can ever query.
/// Outside fitted support the service applies no correction, and there is no reason
/// for a constant axis to create that outcome. E-clock spans the full circle
/// because `coordinates_3d::normalize_azimuth_deg` maps into `[0, 360)`.
///
/// The claim that this correction is only *measured* at boresight is carried by
/// the artifact's `calibration_coverage`, which is where
/// `service::served_gain::is_in_coverage` enforces it — not by pinching these knot
/// spans. That coverage is an on-axis **cone**, not a point: boresight is the pole
/// of the (azimuth, polar-angle) system, so azimuth is degenerate there and
/// coverage constrains elevation alone, to
/// [`BORESIGHT_COVERAGE_CONE_DEG`](antenna_core::data::types::BORESIGHT_COVERAGE_CONE_DEG).
/// Writing it as `az ∈ [0,0] ∧ el ∈ [0,0]` — as boresight mode did until
/// 2026-07-31 — constrains a coordinate that carries no information at the pole,
/// and so rejected the very point it was meant to cover: the azimuth of a
/// boresight-aimed query is `atan2` on float noise (measured: 63.43°). See
/// `boresight_calibration::build_calibration_artifact`.
const E_CLOCK_AXIS_DEG: (f64, f64) = (0.0, 360.0);

/// Span of the flat E-cone axis, in degrees. E-cone reaches the service's
/// correction surface as a **polar angle from boresight** (0° on axis), so the
/// full range is `[0, 180]`. See [`E_CLOCK_AXIS_DEG`].
const E_CONE_AXIS_DEG: (f64, f64) = (0.0, 180.0);

/// Span of the synthetic flat temperature wire axis, in Kelvin. Schema 5.1 keeps
/// these knots for byte compatibility, but the evaluator has no temperature query
/// coordinate. See [`E_CLOCK_AXIS_DEG`].
const TEMPERATURE_AXIS_K: (f64, f64) = (0.0, 1000.0);

/// The boresight frequency correction's spline order: 3, i.e. **quadratic** (degree 2).
///
/// Preserved, not chosen: see the module docs. Moving to cubic is a served-value change.
const BORESIGHT_SPLINE_ORDER: u8 = 3;

/// Error types for frequency correction fitting.
#[derive(Debug, Error)]
pub enum FrequencyCorrectionError {
    /// Four points, not the three an order-3 spline needs: a floor kept from when this fit
    /// was believed cubic, preserved with the rest of the served behavior (issue #95).
    #[error("Insufficient data points: need at least 4 frequency points, got {0}")]
    InsufficientData(usize),

    #[error("Invalid frequency range: min={min} >= max={max}")]
    InvalidFrequencyRange { min: f64, max: f64 },

    #[error("Non-finite values in input data")]
    NonFiniteData,

    #[error("B-spline fitting failed: {0}")]
    FittingError(String),

    /// The core layout refused the surface this module described (GitHub issue #95).
    #[error("Invalid correction-surface construction: {0}")]
    InvalidSurface(#[from] antenna_core::data::types::ValidationError),
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

/// Fits a 1D frequency-only correction surface and packages it as schema 5's 4D wire type,
/// flat in every axis but frequency.
///
/// The spline is **quadratic** (order 3 = degree 2; see the module docs), with the residuals
/// used directly as its frequency control points. The resulting 4D B-spline has:
/// - shape = `[F, F, N_freq, F]` with `F = spline_order + 1`, where `N_freq` is
///   the number of frequency control points
/// - Frequency dimension: proper B-spline with `N_freq` control points
/// - E-clock, E-cone and temperature: **flat** axes (identical coefficient layers over a
///   real span, see the module docs) so the surface is exactly constant along them
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
/// - Fewer than 4 data points
/// - Frequencies not monotonically increasing
/// - Any NaN or Inf values in input
/// - B-spline construction fails
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
/// assert_eq!(correction.spline_order, 3); // quadratic
/// assert_eq!(correction.shape, [4, 4, 4, 4]); // flat, flat, 4 frequencies, flat
/// correction.validate().expect("the service loader must accept this");
/// ```
pub fn fit_frequency_correction(frequencies: &[f64], residuals: &[f64]) -> Result<BSplineModel4D> {
    validate_inputs(frequencies, residuals)?;

    let layout = CorrectionSurfaceLayout::clamped(
        ClampedAxis::flat(E_CLOCK_AXIS_DEG.0, E_CLOCK_AXIS_DEG.1),
        ClampedAxis::flat(E_CONE_AXIS_DEG.0, E_CONE_AXIS_DEG.1),
        frequency_axis(frequencies, BORESIGHT_SPLINE_ORDER),
        BORESIGHT_SPLINE_ORDER,
    )?;

    // Replicate each frequency's control point across every flat angular layer, in the
    // canonical coefficient order the layout declares — E-clock fastest, then E-cone, then
    // frequency. The synthetic temperature axis is the wire adapter's business, not this
    // module's (GitHub issue #94).
    let [n_e_clock, n_e_cone, _] = layout.shape();
    let coefficients = residuals
        .iter()
        .flat_map(|&residual| std::iter::repeat_n(residual, n_e_clock * n_e_cone))
        .collect();
    let fitted = FittedCorrectionSurface::new(layout, coefficients)?;

    Ok(fitted.to_model4d(TEMPERATURE_AXIS_K.0, TEMPERATURE_AXIS_K.1)?)
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

    // At least 4 points (see `InsufficientData` for why four)
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

/// The frequency axis: clamped to the measured sweep, with one control point per sample.
///
/// `n` control points at order `k` need `n - k` interior knots; they are placed on evenly
/// spaced *interior* measured frequencies, which keeps them strictly inside the sweep
/// (the frequencies are strictly increasing). This is placement policy only — the core
/// layout builds and validates the knot vector (GitHub issue #95).
fn frequency_axis(frequencies: &[f64], spline_order: u8) -> ClampedAxis {
    let n = frequencies.len();
    let num_interior = n.saturating_sub(spline_order as usize);
    let interior = (1..=num_interior)
        .map(|i| frequencies[(i * (n - 1)) / (num_interior + 1)])
        .collect();
    ClampedAxis::new(frequencies[0], frequencies[n - 1], interior)
}

#[cfg(test)]
mod tests {
    use super::*;
    use antenna_core::model::FittedCorrectionSurface;

    fn applied_value(
        surface: &FittedCorrectionSurface,
        e_clock_deg: f64,
        e_cone_deg: f64,
        frequency_mhz: f64,
    ) -> f64 {
        surface
            .evaluate(e_clock_deg, e_cone_deg, frequency_mhz)
            .correction_db()
            .expect("test query left fitted support")
    }

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
        assert_eq!(bspline.knots_azimuth.first(), Some(&E_CLOCK_AXIS_DEG.0));
        assert_eq!(bspline.knots_azimuth.last(), Some(&E_CLOCK_AXIS_DEG.1));
        assert_eq!(bspline.knots_elevation.first(), Some(&E_CONE_AXIS_DEG.0));
        assert_eq!(bspline.knots_elevation.last(), Some(&E_CONE_AXIS_DEG.1));
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
    /// knots over one coefficient layer, and `BSplineModel4D::validate` required
    /// `knots.len() >= shape + order` on every axis (since issue #95: exactly
    /// `shape + order`, with a non-empty support). The service loader runs that
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
        let surface = FittedCorrectionSurface::from_model4d(&bspline).unwrap();

        let freq = 4000.0;
        let reference = applied_value(&surface, 0.0, 0.0, freq);

        assert!(
            reference.abs() > 0.1,
            "the reference value is ~0 dB, so a collapsed-to-zero surface would \
             pass the comparisons below vacuously; got {reference} dB"
        );

        for az in [0.0, 1.0, 45.0, 180.0, 359.0, 360.0] {
            for el in [0.0, 0.5, 30.0, 90.0, 179.0, 180.0] {
                let result = applied_value(&surface, az, el, freq);
                assert!(
                    (result - reference).abs() < 1e-12,
                    "correction must not depend on E-clock/E-cone: \
                     ({az}, {el}) gave {result} dB vs {reference} dB at the origin"
                );
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
        let surface = FittedCorrectionSurface::from_model4d(&bspline).unwrap();

        for (freq, expected) in [
            (frequencies[0], residuals[0]),
            (
                frequencies[frequencies.len() - 1],
                residuals[residuals.len() - 1],
            ),
        ] {
            let got = applied_value(&surface, 0.0, 0.0, freq);
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
        let surface = FittedCorrectionSurface::from_model4d(&bspline).unwrap();

        let got = applied_value(&surface, 0.0, 0.0, frequencies[1]);

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

    /// The frequency knot vector the boresight producer has always written, byte for byte:
    /// order 3, clamped to the sweep, interior knots on evenly spaced interior samples.
    /// Preserved through the move onto the core layout (GitHub issue #95).
    #[test]
    fn the_frequency_knot_vector_is_preserved_through_the_core_layout() {
        let frequencies = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        let residuals = vec![0.5, 0.6, 0.4, 0.7, 0.5];
        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();

        assert_eq!(
            bspline.knots_frequency,
            vec![100.0, 100.0, 100.0, 200.0, 300.0, 500.0, 500.0, 500.0]
        );
    }

    /// Pins the spline order this module intentionally preserves: **order 3, quadratic**
    /// (degree 2), not the cubic its comments claimed before issue #95. Moving to cubic is a
    /// served-value decision, so it must fail this test rather than slip in.
    ///
    /// Numerically, not just by the stamp: on one knot span a degree-2 polynomial has a
    /// vanishing third finite difference and a non-vanishing second one. A cubic surface
    /// fails the first check; a linear one fails the second.
    #[test]
    fn the_boresight_frequency_correction_is_quadratic_order_3() {
        let frequencies = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        let residuals = vec![0.0, 2.0, -1.0, 3.0, 0.5];
        let bspline = fit_frequency_correction(&frequencies, &residuals).unwrap();
        assert_eq!(bspline.spline_order, BORESIGHT_SPLINE_ORDER);
        assert_eq!(BORESIGHT_SPLINE_ORDER, 3);

        let surface = FittedCorrectionSurface::from_model4d(&bspline).unwrap();
        // Four equally spaced samples inside the single span [300, 500].
        let v: Vec<f64> = [320.0, 360.0, 400.0, 440.0]
            .iter()
            .map(|&f| applied_value(&surface, 0.0, 0.0, f))
            .collect();
        let second = v[2] - 2.0 * v[1] + v[0];
        let third = v[3] - 3.0 * v[2] + 3.0 * v[1] - v[0];
        assert!(
            third.abs() < 1e-12,
            "degree > 2 on one span: third difference {third}"
        );
        assert!(
            second.abs() > 1e-3,
            "degree < 2 on one span: second difference {second}"
        );
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

        // Clamped at order 3: first and last knots are repeated exactly 3 times
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
