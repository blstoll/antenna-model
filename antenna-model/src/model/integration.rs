//! Aperture Integration Engine
//!
//! This module implements numerical integration over the reflector aperture
//! to compute far-field antenna patterns using physical optics.
//!
//! # Mathematical Foundation
//!
//! The far-field electric field is computed via aperture integration:
//! ```text
//! E(θ,φ) = (jk·exp(-jkr))/(2λr) ∬_Aperture A(ρ,φ') · exp[jΨ(ρ,φ')] · ρ dρ dφ'
//! ```
//!
//! where:
//! - A(ρ,φ') is the aperture illumination amplitude (from feed pattern)
//! - Ψ(ρ,φ') is the total phase (path + coma + surface + mesh)
//! - Integration limits: ρ ∈ [0, D/2], φ' ∈ [0, 2π]
//!
//! # Numerical Methods
//!
//! Uses composite Simpson's rule with adaptive refinement:
//! - 2D integration via nested 1D integration
//! - Adaptive grid refinement for accuracy
//! - Convergence monitoring
//!
//! # References
//! - Design doc Section 2.1 (Core Physical Optics Model)
//! - Implementation plan Sprint 2, Task 2.4

use num_complex::Complex64;
use std::f64::consts::PI;

use crate::error::{ComputationError, ComputationResult};
use crate::model::{
    coordinates::ApertureCoordinates, edge_cases::higher_order_aberrations,
    geometry::AntennaConfiguration, illumination::illumination_amplitude, phase::phase_total,
    wavelength_from_frequency, wavenumber,
};

/// Complex integration result
///
/// The aperture integration produces a complex-valued field in the far zone.
/// Both real and imaginary parts are needed for phase information.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntegrationResult {
    /// Complex electric field value
    pub field: Complex64,

    /// Estimated integration error (magnitude)
    ///
    /// On successful convergence this is the inter-iteration difference that
    /// satisfied the tolerance.  On non-convergence it is the last computed
    /// inter-iteration difference (an honest, if pessimistic, error estimate).
    pub error_estimate: f64,

    /// Number of function evaluations performed
    pub num_evaluations: usize,

    /// Whether the integration converged within the allowed iterations.
    ///
    /// `true`  – the refinement loop exited because the relative error fell below
    ///           `relative_tolerance` (or the absolute difference below
    ///           `absolute_tolerance`).
    /// `false` – the loop exhausted `max_iterations` (or hit the maximum grid
    ///           size) without meeting tolerance.  The returned `field` is the
    ///           best available estimate; `error_estimate` holds the last
    ///           inter-iteration difference.
    ///
    /// When `error_estimate` is `f64::INFINITY` the loop ran only one iteration
    /// and no inter-iteration comparison was possible.  A finite `error_estimate`
    /// with `converged: false` means the maximum grid size was reached but the
    /// difference between the last two iterations still exceeded the tolerance.
    pub converged: bool,
}

/// Integration parameters for convergence control
#[derive(Debug, Clone)]
pub struct IntegrationParams {
    /// Minimum number of radial integration points
    pub min_rho_points: usize,

    /// Maximum number of radial integration points (for adaptive refinement)
    pub max_rho_points: usize,

    /// Minimum number of azimuthal integration points
    pub min_phi_points: usize,

    /// Maximum number of azimuthal integration points
    pub max_phi_points: usize,

    /// Relative error tolerance for adaptive refinement
    pub relative_tolerance: f64,

    /// Absolute error tolerance
    pub absolute_tolerance: f64,

    /// Maximum number of refinement iterations
    pub max_iterations: usize,

    /// Include higher-order Seidel aberrations in phase computation
    ///
    /// When true, adds astigmatism, field curvature, and distortion terms
    /// for feeds with moderate offsets (0.3f - 0.5f).
    pub use_higher_order_aberrations: bool,

    /// Fold physical feed-spillover efficiency into the returned gain.
    ///
    /// Decided by the SERVICE layer (set only for antennas with no correction
    /// surface — the surface otherwise absorbs spillover empirically). The model
    /// itself never inspects calibration; it only reads this bool.
    pub apply_spillover: bool,
}

impl Default for IntegrationParams {
    fn default() -> Self {
        Self {
            min_rho_points: 32,       // Minimum for radial direction
            max_rho_points: 128,      // Maximum for adaptive refinement
            min_phi_points: 64,       // Azimuthal (full 2π circle)
            max_phi_points: 256,      // Maximum azimuthal points
            relative_tolerance: 1e-4, // 0.01% relative error
            absolute_tolerance: 1e-8, // Absolute error floor
            max_iterations: 5,        // Refinement iteration limit
            use_higher_order_aberrations: false,
            apply_spillover: false,
        }
    }
}

impl IntegrationParams {
    /// Create fast integration parameters (lower accuracy, faster)
    pub fn fast() -> Self {
        Self {
            min_rho_points: 16,
            max_rho_points: 64,
            min_phi_points: 32,
            max_phi_points: 128,
            relative_tolerance: 1e-3,
            absolute_tolerance: 1e-7,
            max_iterations: 3,
            use_higher_order_aberrations: false,
            apply_spillover: false,
        }
    }

    /// Create high-accuracy integration parameters (slower, more accurate)
    pub fn high_accuracy() -> Self {
        Self {
            min_rho_points: 64,
            max_rho_points: 256,
            min_phi_points: 128,
            max_phi_points: 512,
            relative_tolerance: 1e-6,
            absolute_tolerance: 1e-10,
            max_iterations: 8,
            use_higher_order_aberrations: false,
            apply_spillover: false,
        }
    }

    /// Enable higher-order aberrations for moderate feed offsets
    pub fn with_higher_order_aberrations(mut self) -> Self {
        self.use_higher_order_aberrations = true;
        self
    }

    /// Create adaptive integration parameters with doubled sampling density
    ///
    /// Used near pattern nulls where rapid phase changes require finer sampling
    /// to maintain numerical accuracy.
    pub fn with_adaptive_refinement(&self) -> Self {
        Self {
            min_rho_points: self.min_rho_points * 2,
            max_rho_points: self.max_rho_points * 2,
            min_phi_points: self.min_phi_points * 2,
            max_phi_points: self.max_phi_points * 2,
            relative_tolerance: self.relative_tolerance / 2.0, // Tighter tolerance
            ..self.clone()
        }
    }
}

/// Integrate aperture field to compute far-field pattern
///
/// Performs 2D numerical integration over the reflector aperture using
/// composite Simpson's rule with adaptive refinement.
///
/// # Arguments
/// - `theta`: Polar angle in far field (radians, from boresight)
/// - `phi`: Azimuthal angle in far field (radians)
/// - `config`: Antenna configuration (geometry, feed, mesh)
/// - `frequency_hz`: Operating frequency in Hz
/// - `params`: Integration parameters (convergence tolerances, grid sizes)
///
/// # Returns
/// `IntegrationResult` containing complex field value, error estimate, and evaluation count
///
/// # Errors
/// Returns `ComputationError` if:
/// - Integration fails to converge within max iterations
/// - Invalid antenna configuration
///
/// # Examples
/// ```
/// use antenna_model::model::integration::{integrate_aperture, IntegrationParams};
/// use antenna_model::model::geometry::{AntennaConfiguration, ReflectorGeometry, FeedParameters};
///
/// // Example integration at boresight (θ=0)
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let reflector = ReflectorGeometry::builder()
/// #     .diameter(1.0)
/// #     .focal_length(0.5)
/// #     .surface_rms(0.001)
/// #     .build()?;
/// # let feed = FeedParameters::builder()
/// #     .at_focus(0.5)
/// #     .q_factor(8.0)
/// #     .build()?;
/// # let config = AntennaConfiguration::builder()
/// #     .id("test")
/// #     .name("Test")
/// #     .reflector(reflector)
/// #     .feed(feed)
/// #     .build()?;
/// let result = integrate_aperture(
///     0.0,               // theta (boresight)
///     0.0,               // phi
///     &config,
///     8.4e9,             // 8.4 GHz
///     &IntegrationParams::default(),
/// )?;
///
/// println!("Field magnitude: {}", result.field.norm());
/// println!("Error estimate: {}", result.error_estimate);
/// # Ok(())
/// # }
/// ```
pub fn integrate_aperture(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<IntegrationResult> {
    // Validate inputs
    if !theta.is_finite() || !phi.is_finite() || !frequency_hz.is_finite() {
        return Err(ComputationError::NumericalInstability {
            operation: "integrate_aperture".to_string(),
            reason: "Angles and frequency must be finite".to_string(),
        });
    }

    if frequency_hz <= 0.0 {
        return Err(ComputationError::NumericalInstability {
            operation: "integrate_aperture".to_string(),
            reason: format!("Frequency must be positive, got {}", frequency_hz),
        });
    }

    // Calculate wavelength and wavenumber
    let wavelength = wavelength_from_frequency(frequency_hz);
    let k = wavenumber(wavelength);

    // Integration limits
    let rho_max = config.reflector.diameter / 2.0;
    let phi_min = 0.0;
    let phi_max = 2.0 * PI;

    // Start with minimum grid size
    let mut n_rho = params.min_rho_points;
    let mut n_phi = params.min_phi_points;

    let mut previous_result = Complex64::new(0.0, 0.0);
    let mut num_evaluations = 0;
    // Tracks the last inter-iteration difference computed inside the loop.
    // Initialised to INFINITY so that if the loop exits before any convergence
    // check (e.g. max_iterations == 1, or the grid hits max on iteration 0),
    // the non-converged error estimate is still a conservative, honest value.
    let mut last_difference = f64::INFINITY;

    // Adaptive refinement loop: progressively refine the integration grid
    // until the result converges or we reach the maximum grid density.
    //
    // Strategy:
    // 1. Start with coarse grid (min_rho_points × min_phi_points)
    // 2. Compute integral with current grid
    // 3. Compare with previous iteration to estimate error
    // 4. If converged, return; otherwise refine grid by 50% and repeat
    // 5. Stop when grid reaches maximum size or convergence achieved
    for iteration in 0..params.max_iterations {
        // Perform 2D integration using composite Simpson's rule
        // on the current grid (n_rho × n_phi points)
        let (result, evals) = integrate_2d_simpson(
            theta,
            phi,
            config,
            k,
            wavelength,
            0.0,     // rho starts at aperture center
            rho_max, // rho extends to aperture edge (D/2)
            phi_min, // azimuthal angle: 0
            phi_max, // azimuthal angle: 2π (full circle)
            n_rho,
            n_phi,
            params.use_higher_order_aberrations,
        );

        num_evaluations += evals;

        // Check convergence by comparing with previous iteration
        // (skip on first iteration since we have no previous result)
        if iteration > 0 {
            // Compute difference between current and previous result
            let difference = (result - previous_result).norm();
            let magnitude = result.norm();

            // Track the most recent inter-iteration difference for the
            // non-converged fall-through error estimate below.
            last_difference = difference;

            // Calculate relative error (or absolute if magnitude is too small)
            // This handles both large and small field values correctly
            let relative_error = if magnitude > params.absolute_tolerance {
                difference / magnitude
            } else {
                difference
            };

            // Convergence test: relative error < tolerance OR absolute difference < tolerance
            // We use OR because for very small fields, absolute error is more meaningful
            if relative_error < params.relative_tolerance || difference < params.absolute_tolerance
            {
                return Ok(IntegrationResult {
                    field: result,
                    error_estimate: difference,
                    num_evaluations,
                    converged: true,
                });
            }
        }

        // Store current result for next iteration's convergence check
        previous_result = result;

        // Refine grid for next iteration: increase both dimensions by 50%
        // (grid grows from min → max over iterations)
        // Clamp to maximum to avoid unbounded growth
        n_rho = (n_rho * 3 / 2).min(params.max_rho_points);
        n_phi = (n_phi * 3 / 2).min(params.max_phi_points);

        // If we've reached maximum grid size in both dimensions,
        // stop refinement (we can't improve further)
        if n_rho >= params.max_rho_points && n_phi >= params.max_phi_points {
            break;
        }
    }

    // Did not converge within max iterations.
    // Use the last inter-iteration difference as the error estimate — it is an
    // honest (pessimistic) value rather than a fabricated |result| × tolerance.
    // If the loop ran only a single iteration, no convergence check was ever
    // performed so last_difference remains INFINITY, which correctly signals
    // that the accuracy is completely unknown.
    tracing::warn!(
        theta,
        phi,
        last_difference,
        "aperture integration did not converge within {} iterations",
        params.max_iterations
    );

    Ok(IntegrationResult {
        field: previous_result,
        error_estimate: last_difference,
        num_evaluations,
        converged: false,
    })
}

/// Perform 2D integration using composite Simpson's rule
///
/// Integrates over rectangular domain [rho_min, rho_max] × [phi_min, phi_max]
/// using nested 1D Simpson's rule.
///
/// Returns (integrated_value, num_evaluations)
#[allow(clippy::too_many_arguments)]
fn integrate_2d_simpson(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    k: f64,
    wavelength: f64,
    rho_min: f64,
    rho_max: f64,
    phi_min: f64,
    phi_max: f64,
    n_rho: usize,
    n_phi: usize,
    use_higher_order_aberrations: bool,
) -> (Complex64, usize) {
    // Ensure odd number of points for Simpson's rule
    let n_rho = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let n_phi = if n_phi.is_multiple_of(2) {
        n_phi + 1
    } else {
        n_phi
    };

    let h_rho = (rho_max - rho_min) / (n_rho - 1) as f64;
    let h_phi = (phi_max - phi_min) / (n_phi - 1) as f64;

    let mut sum = Complex64::new(0.0, 0.0);
    let mut num_evaluations = 0;

    // Outer integral over φ' using Simpson's rule
    for j in 0..n_phi {
        let phi_prime = phi_min + j as f64 * h_phi;
        let phi_weight = simpson_weight(j, n_phi);

        // Inner integral over ρ using Simpson's rule
        let mut inner_sum = Complex64::new(0.0, 0.0);

        for i in 0..n_rho {
            let rho = rho_min + i as f64 * h_rho;
            let rho_weight = simpson_weight(i, n_rho);

            // Evaluate integrand
            let integrand_value = aperture_integrand(
                rho,
                phi_prime,
                theta,
                phi,
                config,
                k,
                wavelength,
                use_higher_order_aberrations,
            );

            num_evaluations += 1;

            // Accumulate with weights and Jacobian (ρ for polar coordinates)
            inner_sum += integrand_value * rho * rho_weight;
        }

        // Accumulate outer integral
        sum += inner_sum * phi_weight;
    }

    // Apply Simpson's rule scaling factors
    let integral = sum * h_rho * h_phi / 9.0; // 1/9 = (1/3) * (1/3) for 2D Simpson's

    (integral, num_evaluations)
}

/// Simpson's rule weight for index i in array of n points
///
/// Returns:
/// - 1 for first and last points
/// - 4 for odd interior indices
/// - 2 for even interior indices
#[inline]
fn simpson_weight(i: usize, n: usize) -> f64 {
    if i == 0 || i == n - 1 {
        1.0
    } else if i % 2 == 1 {
        4.0
    } else {
        2.0
    }
}

/// Aperture integrand function
///
/// Computes the integrand at a single aperture point (ρ, φ') for observation
/// direction (θ, φ).
///
/// # Formula
/// ```text
/// Integrand = A(ρ,φ') · exp[j·Ψ(ρ,φ')]
/// ```
///
/// where:
/// - A(ρ,φ') is the illumination amplitude from the feed
/// - Ψ(ρ,φ') is the total phase (path + coma + surface + mesh)
///
/// # Arguments
/// - `rho`: Radial coordinate in aperture (meters)
/// - `phi_prime`: Azimuthal coordinate in aperture (radians)
/// - `theta`: Observation polar angle (radians)
/// - `phi`: Observation azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `k`: Wavenumber (rad/m)
/// - `wavelength`: Wavelength (meters)
///
/// # Returns
/// Complex integrand value
#[inline]
#[allow(clippy::too_many_arguments)]
fn aperture_integrand(
    rho: f64,
    phi_prime: f64,
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    k: f64,
    _wavelength: f64,
    use_higher_order_aberrations: bool,
) -> Complex64 {
    // Calculate illumination amplitude
    let amplitude =
        illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);

    // Create aperture coordinates
    let aperture = ApertureCoordinates { rho, phi_prime };

    // Calculate feed displacement from position.
    // Lateral (xy-plane) displacement drives coma; axial (z) offset drives defocus.
    let feed_displacement = config.feed.position.radial_displacement();
    let feed_displacement_angle = config.feed.position.y.atan2(config.feed.position.x);
    // Axial offset of the feed's PHASE CENTER from the focal point:
    // physical z-offset plus the phase-center offset along the feed axis
    // (positive = away from the vertex, matching phase_feed_displacement's delta_z).
    let feed_axial_offset =
        config.feed.position.z - config.reflector.focal_length + config.feed.phase_center_offset;

    // Calculate angle of incidence (simplified - assumes small angles)
    // For parabolic reflector, theta_incident ≈ ρ/(2f)
    let theta_incident = rho / (2.0 * config.reflector.focal_length);

    // Get mesh spacing (0.0 if no mesh)
    let mesh_spacing = config.mesh.as_ref().map_or(0.0, |m| m.spacing);

    // Surface error at this point (ρ, φ')
    //
    // FUTURE ENHANCEMENT: Spatially-varying surface error model
    // Currently uses ideal surface (surface_error = 0.0) for all aperture points.
    // The Ruze efficiency factor in pattern.rs handles the statistical effect
    // of surface RMS on overall gain, which is sufficient for most applications.
    //
    // For higher fidelity modeling of specific antennas with measured surface maps:
    // - Option 1: Zernike polynomial expansion of measured surface
    // - Option 2: Interpolate from measured surface map (x, y, z points)
    // - Option 3: Use correction surface from calibration (already implemented)
    //
    // Rationale for current approach:
    // - Calibration correction surface (B-spline) captures measured deviations
    // - Ruze statistical model is accurate for random surface errors
    // - Explicit surface modeling adds complexity with marginal accuracy gain
    let surface_error = 0.0;

    // Calculate total phase
    let mut total_phase = phase_total(
        aperture,
        theta,
        phi,
        config.reflector.focal_length,
        feed_displacement,
        feed_displacement_angle,
        feed_axial_offset,
        surface_error,
        theta_incident,
        mesh_spacing,
        k,
    );

    // Add higher-order Seidel aberrations if enabled
    // These include astigmatism, field curvature, and distortion terms
    if use_higher_order_aberrations && feed_displacement > 0.0 {
        total_phase += higher_order_aberrations(
            rho,
            phi_prime,
            feed_displacement,
            feed_displacement_angle,
            config.reflector.focal_length,
            k,
        );
    }

    // Combine: A(ρ,φ') · exp(j·Ψ)
    let phase_factor = Complex64::new(0.0, total_phase).exp();

    amplitude * phase_factor
}

/// ∬ |A(ρ,φ')|² ρ dρ dφ' over the aperture — denominator of the aperture-directivity
/// formula. Uses the same illumination model and Simpson scheme as the field integral.
///
/// The directivity of an aperture is
/// ```text
/// D(θ,φ) = (4π/λ²) · |∬ A e^{jΨ} ρ dρ dφ'|² / ∬ |A|² ρ dρ dφ'
/// ```
/// This function computes the (real, phase-free) denominator. The numerator is the
/// raw aperture integral from [`integrate_aperture`] (i.e. `IntegrationResult::field`),
/// NOT the normalized [`compute_far_field`] value.
pub fn integrate_amplitude_squared(
    config: &AntennaConfiguration,
    n_rho: usize,
    n_phi: usize,
) -> f64 {
    let rho_max = config.reflector.diameter / 2.0;

    // Ensure odd number of points for Simpson's rule.
    let n_rho = if n_rho.is_multiple_of(2) {
        n_rho + 1
    } else {
        n_rho
    };
    let n_phi = if n_phi.is_multiple_of(2) {
        n_phi + 1
    } else {
        n_phi
    };

    let h_rho = rho_max / (n_rho - 1) as f64;
    let h_phi = 2.0 * PI / (n_phi - 1) as f64;

    let mut sum = 0.0;
    for j in 0..n_phi {
        let phi_prime = j as f64 * h_phi;
        let wj = simpson_weight(j, n_phi);
        let mut inner = 0.0;
        for i in 0..n_rho {
            let rho = i as f64 * h_rho;
            let a =
                illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);
            inner += a * a * rho * simpson_weight(i, n_rho);
        }
        sum += inner * wj;
    }

    sum * h_rho * h_phi / 9.0
}

/// Compute far-field normalization factor
///
/// The complete far-field formula includes a normalization factor:
/// ```text
/// E(θ,φ) = (jk·exp(-jkr))/(2λr) × [aperture integral]
/// ```
///
/// This function computes the normalization factor, excluding the r-dependent
/// terms which are typically omitted in pattern calculations (relative patterns).
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Complex normalization factor (jk)/(2λ)
pub fn far_field_normalization(wavelength: f64) -> Complex64 {
    let k = wavenumber(wavelength);

    // (jk) / (2λ) = (j * 2π/λ) / (2λ) = jπ/λ²
    Complex64::new(0.0, 1.0) * k / (2.0 * wavelength)
}

/// Compute normalized far-field electric field
///
/// Combines aperture integration with normalization factor to produce
/// the complete far-field electric field (excluding r-dependent terms).
///
/// # Arguments
/// - `theta`: Polar angle (radians)
/// - `phi`: Azimuthal angle (radians)
/// - `config`: Antenna configuration
/// - `frequency_hz`: Frequency in Hz
/// - `params`: Integration parameters
///
/// # Returns
/// Complex electric field value (normalized, excluding 1/r factor)
pub fn compute_far_field(
    theta: f64,
    phi: f64,
    config: &AntennaConfiguration,
    frequency_hz: f64,
    params: &IntegrationParams,
) -> ComputationResult<Complex64> {
    let wavelength = wavelength_from_frequency(frequency_hz);
    let integration_result = integrate_aperture(theta, phi, config, frequency_hz, params)?;

    let normalization = far_field_normalization(wavelength);

    Ok(normalization * integration_result.field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{FeedParameters, FeedPosition, MeshParameters, ReflectorGeometry};

    /// Create a simple test antenna configuration
    fn test_antenna() -> AntennaConfiguration {
        use crate::model::geometry::MeshPattern;

        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(); // 1m diameter, f/D=0.5, ideal surface
        let feed_pos = FeedPosition::at_focus(0.5);
        let feed = FeedParameters::new(feed_pos, 8.0, 0.0, 1.0).unwrap(); // q=8, no offset, symmetric
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap(); // 5mm spacing, 0.5mm wire

        AntennaConfiguration::new(
            "test_antenna".to_string(),
            "Test Antenna".to_string(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap()
    }

    #[test]
    fn test_simpson_weight() {
        // Test Simpson's rule weights
        let n = 5; // 5 points

        assert_eq!(simpson_weight(0, n), 1.0); // First point
        assert_eq!(simpson_weight(1, n), 4.0); // Odd interior
        assert_eq!(simpson_weight(2, n), 2.0); // Even interior
        assert_eq!(simpson_weight(3, n), 4.0); // Odd interior
        assert_eq!(simpson_weight(4, n), 1.0); // Last point
    }

    #[test]
    fn test_integration_params_default() {
        let params = IntegrationParams::default();

        assert!(params.min_rho_points > 0);
        assert!(params.max_rho_points >= params.min_rho_points);
        assert!(params.relative_tolerance > 0.0);
        assert!(params.max_iterations > 0);
    }

    #[test]
    fn test_integration_params_fast() {
        let params = IntegrationParams::fast();
        let default_params = IntegrationParams::default();

        // Fast should use fewer points
        assert!(params.min_rho_points <= default_params.min_rho_points);
        assert!(params.max_rho_points <= default_params.max_rho_points);
    }

    #[test]
    fn test_integration_params_high_accuracy() {
        let params = IntegrationParams::high_accuracy();
        let default_params = IntegrationParams::default();

        // High accuracy should use more points and tighter tolerance
        assert!(params.max_rho_points >= default_params.max_rho_points);
        assert!(params.relative_tolerance <= default_params.relative_tolerance);
    }

    #[test]
    fn test_aperture_integrand_on_axis() {
        let config = test_antenna();
        let wavelength = 0.0357; // ~8.4 GHz
        let k = wavenumber(wavelength);

        // On-axis (θ=0, φ=0), center of aperture (ρ=0)
        let integrand = aperture_integrand(0.0, 0.0, 0.0, 0.0, &config, k, wavelength, false);

        // At center, amplitude should be near maximum, phase should be well-defined
        assert!(integrand.norm() > 0.0);
        assert!(integrand.norm() <= 1.0);
    }

    #[test]
    fn test_aperture_integrand_symmetry() {
        let config = test_antenna();
        let wavelength = 0.0357;
        let k = wavenumber(wavelength);

        // For symmetric feed and ideal surface, pattern should have azimuthal symmetry
        let rho = 0.2;
        let theta = 0.1;

        let integrand_0 = aperture_integrand(rho, 0.0, theta, 0.0, &config, k, wavelength, false);
        let integrand_90 = aperture_integrand(
            rho,
            PI / 2.0,
            theta,
            PI / 2.0,
            &config,
            k,
            wavelength,
            false,
        );

        // Magnitudes should be equal due to symmetry
        assert!((integrand_0.norm() - integrand_90.norm()).abs() < 1e-6);
    }

    #[test]
    fn test_integrate_aperture_on_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast(); // Use fast for quicker tests

        let result = integrate_aperture(
            0.0, // theta (on-axis)
            0.0, // phi
            &config, 8.4e9, // 8.4 GHz
            &params,
        )
        .unwrap();

        // On-axis field should be non-zero
        assert!(result.field.norm() > 0.0);

        // Should have performed evaluations
        assert!(result.num_evaluations > 0);

        // On-axis integration with fast params must converge (smooth, no phase oscillation).
        assert!(result.converged, "on-axis fast integration must converge");
        // A converged result must have a finite, non-negative error estimate.
        assert!(
            result.error_estimate.is_finite(),
            "converged error_estimate must be finite"
        );
        assert!(result.error_estimate >= 0.0);
    }

    #[test]
    fn test_integrate_aperture_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // Small off-axis angle
        let result = integrate_aperture(
            0.05, // theta (small angle ~2.9°)
            0.0,  // phi
            &config, 8.4e9, &params,
        )
        .unwrap();

        // Off-axis field should be non-zero but smaller than on-axis
        assert!(result.field.norm() > 0.0);
    }

    #[test]
    fn test_integrate_aperture_convergence() {
        let config = test_antenna();

        // Test that higher accuracy params give better results
        let fast_params = IntegrationParams::fast();
        let accurate_params = IntegrationParams::high_accuracy();

        let fast_result = integrate_aperture(0.0, 0.0, &config, 8.4e9, &fast_params).unwrap();
        let accurate_result =
            integrate_aperture(0.0, 0.0, &config, 8.4e9, &accurate_params).unwrap();

        // Both must converge so the error-estimate comparison below is meaningful.
        assert!(
            fast_result.converged,
            "fast on-axis integration must converge"
        );
        assert!(
            accurate_result.converged,
            "accurate on-axis integration must converge"
        );

        // High accuracy should have lower error estimate
        assert!(accurate_result.error_estimate <= fast_result.error_estimate * 2.0);

        // Results should be similar
        let difference = (fast_result.field - accurate_result.field).norm();
        let magnitude = accurate_result.field.norm();
        assert!(difference / magnitude < 0.1); // Within 10%
    }

    #[test]
    fn test_integrate_aperture_invalid_inputs() {
        let config = test_antenna();
        let params = IntegrationParams::default();

        // Invalid frequency
        let result = integrate_aperture(0.0, 0.0, &config, -1.0, &params);
        assert!(result.is_err());

        // Invalid angle (NaN)
        let result = integrate_aperture(f64::NAN, 0.0, &config, 8.4e9, &params);
        assert!(result.is_err());
    }

    #[test]
    fn test_integrate_amplitude_squared_positive_finite() {
        let config = test_antenna();

        // The denominator of the directivity formula must be a positive, finite
        // real number for a physically-illuminated aperture.
        let amp_sq = integrate_amplitude_squared(&config, 33, 65);
        assert!(amp_sq.is_finite());
        assert!(amp_sq > 0.0);

        // Sanity upper bound: |A| <= 1 everywhere, so the integral is at most the
        // area integral ∬ ρ dρ dφ' = π(D/2)² = π·0.25 ≈ 0.785 for the 1m test dish.
        let area = PI * (config.reflector.diameter / 2.0).powi(2);
        assert!(amp_sq <= area + 1e-9);
    }

    #[test]
    fn test_far_field_normalization() {
        let wavelength = 0.0357; // ~8.4 GHz
        let norm = far_field_normalization(wavelength);

        // Should be purely imaginary (j factor)
        assert!(norm.re.abs() < 1e-10);
        assert!(norm.im != 0.0);

        // Magnitude should be k/(2λ) = π/λ²
        let expected_magnitude = PI / (wavelength * wavelength);
        assert!((norm.norm() - expected_magnitude).abs() < 1e-6);
    }

    #[test]
    fn test_compute_far_field() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        let field = compute_far_field(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Far field should be non-zero
        assert!(field.norm() > 0.0);

        // Should be complex-valued
        // (May have both real and imaginary parts depending on phase)
    }

    #[test]
    fn test_pattern_decreases_off_axis() {
        let config = test_antenna();
        let params = IntegrationParams::fast();

        // On-axis field
        let field_on_axis = compute_far_field(0.0, 0.0, &config, 8.4e9, &params).unwrap();

        // Off-axis field (5 degrees)
        let field_off_axis =
            compute_far_field(5.0_f64.to_radians(), 0.0, &config, 8.4e9, &params).unwrap();

        // Pattern should decrease off-axis
        assert!(field_off_axis.norm() < field_on_axis.norm());
    }

    #[test]
    fn test_non_convergence_is_reported() {
        let config = test_antenna();
        let params = IntegrationParams {
            max_iterations: 1, // cannot converge: convergence check needs iteration > 0
            relative_tolerance: 1e-15,
            ..IntegrationParams::fast()
        };
        let result = integrate_aperture(0.3, 0.0, &config, 8.4e9, &params).unwrap();
        assert!(!result.converged);
        // With max_iterations == 1 the loop runs a single iteration and the convergence
        // check (iteration > 0) is never reached, so no inter-iteration difference is
        // ever computed.  last_difference remains at its INFINITY sentinel value.
        assert_eq!(result.error_estimate, f64::INFINITY);
    }

    #[test]
    fn test_integration_2d_simpson_basic() {
        let config = test_antenna();
        let wavelength = 0.0357;
        let k = wavenumber(wavelength);

        // Simple integration test
        let (result, evals) = integrate_2d_simpson(
            0.0, // theta
            0.0, // phi
            &config,
            k,
            wavelength,
            0.0,      // rho_min
            0.5,      // rho_max (half diameter)
            0.0,      // phi_min
            2.0 * PI, // phi_max
            17,       // n_rho (odd)
            33,       // n_phi (odd)
            false,    // use_higher_order_aberrations
        );

        // Should produce non-zero result
        assert!(result.norm() > 0.0);

        // Should have performed expected number of evaluations
        assert_eq!(evals, 17 * 33);
    }

    /// phase_center_offset must act as an axial defocus. Before the fix it was
    /// parsed and validated but never entered the physics, so both gains were
    /// identical and this test failed.
    #[test]
    fn test_phase_center_offset_produces_defocus_loss() {
        let feed_focused = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let feed_pco = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.05, 1.0).unwrap();

        let mk = |feed| {
            AntennaConfiguration::new(
                "t".into(),
                "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(),
                feed,
                None,
            )
            .unwrap()
        };

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused =
            crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
                .unwrap()
                .gain;
        let g_pco = crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_pco), 8.4e9, &params)
            .unwrap()
            .gain;

        assert!(
            g_focused - g_pco > 1.0,
            "5 cm phase-center offset must cost >1 dB defocus at 8.4 GHz: focused={g_focused:.2}, pco={g_pco:.2}"
        );
    }
}
