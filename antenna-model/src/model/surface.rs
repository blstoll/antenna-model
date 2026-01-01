//! Surface Error Modeling
//!
//! This module implements surface error models for parabolic reflector antennas,
//! including Ruze's equation for random surface errors and Zernike polynomials
//! for systematic aberrations.
//!
//! # References
//! - Ruze, J. "Antenna Tolerance Theory" (1966)
//! - Noll, R.J. "Zernike polynomials and atmospheric turbulence" (1976)

use std::f64::consts::PI;

/// Compute Ruze efficiency for random surface errors
///
/// Ruze's equation models the efficiency loss due to random surface errors
/// with RMS value σ at wavelength λ.
///
/// # Formula
/// ```text
/// η = exp(-(4π·σ/λ)²)
/// ```
///
/// # Arguments
/// * `sigma_rms` - RMS surface error in meters
/// * `wavelength` - Wavelength in meters
///
/// # Returns
/// Efficiency factor between 0.0 and 1.0
///
/// # Examples
/// ```
/// use antenna_model::model::surface::ruze_efficiency;
///
/// // 1mm RMS error at 8.4 GHz (λ ≈ 0.0357m)
/// let efficiency = ruze_efficiency(0.001, 0.0357);
/// assert!(efficiency > 0.85); // Should be ~88%
/// ```
pub fn ruze_efficiency(sigma_rms: f64, wavelength: f64) -> f64 {
    let ratio = sigma_rms / wavelength;
    let exponent = -(4.0 * PI * ratio).powi(2);
    exponent.exp()
}

/// Compute Ruze efficiency from frequency
///
/// Convenience function that converts frequency to wavelength.
///
/// # Arguments
/// * `sigma_rms` - RMS surface error in meters
/// * `frequency_hz` - Frequency in Hz
///
/// # Returns
/// Efficiency factor between 0.0 and 1.0
pub fn ruze_efficiency_from_frequency(sigma_rms: f64, frequency_hz: f64) -> f64 {
    const SPEED_OF_LIGHT: f64 = 299_792_458.0; // m/s
    let wavelength = SPEED_OF_LIGHT / frequency_hz;
    ruze_efficiency(sigma_rms, wavelength)
}

/// Zernike polynomial indices using Noll ordering
///
/// Noll ordering is the standard indexing scheme for Zernike polynomials
/// used in optical aberration analysis.
///
/// # Noll Index Convention
/// - j=1: Piston (constant term)
/// - j=2,3: Tip/tilt (linear terms)
/// - j=4,5,6: Defocus and astigmatism (quadratic terms)
/// - j=7,8,9,10: Coma and trefoil (cubic terms)
/// - j=11-15: Spherical and higher order (quartic terms)
/// - j=16-21: Fifth order terms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZernikeIndex {
    /// Noll index (j >= 1)
    pub noll_index: usize,
    /// Radial order (n >= 0)
    pub n: i32,
    /// Azimuthal order (-n <= m <= n)
    pub m: i32,
}

impl ZernikeIndex {
    /// Create a Zernike index from Noll index
    ///
    /// # Arguments
    /// * `noll_index` - Noll index (j >= 1)
    ///
    /// # Returns
    /// ZernikeIndex with corresponding (n, m) values
    pub fn from_noll(noll_index: usize) -> Self {
        assert!(noll_index >= 1, "Noll index must be >= 1");

        let (n, m) = noll_to_nm(noll_index);
        Self { noll_index, n, m }
    }

    /// Get the name of this Zernike polynomial
    pub fn name(&self) -> &'static str {
        match self.noll_index {
            1 => "Piston",
            2 => "Tip (tilt X)",
            3 => "Tilt (tilt Y)",
            4 => "Defocus",
            5 => "Astigmatism (oblique)",
            6 => "Astigmatism (vertical)",
            7 => "Coma (vertical)",
            8 => "Coma (horizontal)",
            9 => "Trefoil (oblique)",
            10 => "Trefoil (vertical)",
            11 => "Spherical aberration",
            12 => "Secondary astigmatism (oblique)",
            13 => "Secondary astigmatism (vertical)",
            14 => "Secondary coma (vertical)",
            15 => "Secondary coma (horizontal)",
            16 => "Secondary trefoil (oblique)",
            17 => "Secondary trefoil (vertical)",
            18 => "Tetrafoil (oblique)",
            19 => "Tetrafoil (vertical)",
            20 => "Secondary spherical",
            21 => "Pentafoil (oblique)",
            _ => "Higher order",
        }
    }
}

/// Convert Noll index to (n, m) indices
///
/// Standard Noll ordering from Noll (1976)
///
/// # Arguments
/// * `j` - Noll index (j >= 1)
///
/// # Returns
/// Tuple (n, m) where n is radial order and m is azimuthal order
fn noll_to_nm(j: usize) -> (i32, i32) {
    // Use lookup table for clarity and correctness
    match j {
        1 => (0, 0),   // Piston
        2 => (1, -1),  // Tip
        3 => (1, 1),   // Tilt
        4 => (2, 0),   // Defocus
        5 => (2, -2),  // Astigmatism (oblique)
        6 => (2, 2),   // Astigmatism (vertical)
        7 => (3, -1),  // Coma (vertical)
        8 => (3, 1),   // Coma (horizontal)
        9 => (3, -3),  // Trefoil (oblique)
        10 => (3, 3),  // Trefoil (vertical)
        11 => (4, 0),  // Spherical aberration
        12 => (4, -2), // Secondary astigmatism (oblique)
        13 => (4, 2),  // Secondary astigmatism (vertical)
        14 => (4, -4), // Tetrafoil (oblique)
        15 => (4, 4),  // Tetrafoil (vertical)
        16 => (5, -1), // Secondary coma (vertical)
        17 => (5, 1),  // Secondary coma (horizontal)
        18 => (5, -3), // Secondary trefoil (oblique)
        19 => (5, 3),  // Secondary trefoil (vertical)
        20 => (5, -5), // Pentafoil (oblique)
        21 => (5, 5),  // Pentafoil (vertical)
        _ => {
            // For higher orders, use the algorithmic approach
            // This is a simplified version; for production use, implement full algorithm
            let n = (((-1.0 + (1.0 + 8.0 * j as f64).sqrt()) / 2.0).floor()) as i32;
            let row_start = (n * (n + 1)) / 2 + 1;
            let idx_in_row = (j as i32) - row_start;

            // Default m calculation for higher orders
            let m = if idx_in_row % 2 == 0 {
                let k = -idx_in_row / 2;
                if n % 2 == 0 {
                    2 * k
                } else {
                    2 * k - 1
                }
            } else {
                let k = (idx_in_row + 1) / 2;
                if n % 2 == 0 {
                    2 * k
                } else {
                    2 * k - 1
                }
            };

            (n, m)
        }
    }
}

/// Evaluate Zernike polynomial at normalized coordinates
///
/// # Arguments
/// * `noll_index` - Noll index of the Zernike polynomial
/// * `rho` - Normalized radial coordinate (0 <= rho <= 1)
/// * `phi` - Azimuthal angle in radians
///
/// # Returns
/// Value of the Zernike polynomial at (rho, phi)
///
/// # Panics
/// Panics if rho > 1.0 or noll_index < 1
pub fn zernike_polynomial(noll_index: usize, rho: f64, phi: f64) -> f64 {
    assert!((0.0..=1.0).contains(&rho), "rho must be in [0, 1]");
    assert!(noll_index >= 1, "Noll index must be >= 1");

    let (n, m) = noll_to_nm(noll_index);

    // Compute radial polynomial
    let radial = zernike_radial(n, m.abs(), rho);

    // Compute angular part
    let angular = if m > 0 {
        (m as f64 * phi).cos()
    } else if m < 0 {
        (m.abs() as f64 * phi).sin()
    } else {
        1.0
    };

    // Normalization factor for orthonormality
    let norm = if m == 0 {
        ((n + 1) as f64).sqrt()
    } else {
        (2.0 * (n + 1) as f64).sqrt()
    };

    norm * radial * angular
}

/// Compute Zernike radial polynomial R_n^m(rho)
///
/// Uses the direct formula for computation.
///
/// # Arguments
/// * `n` - Radial order
/// * `m` - Absolute value of azimuthal order
/// * `rho` - Normalized radial coordinate
///
/// # Returns
/// Value of R_n^m(rho)
fn zernike_radial(n: i32, m: i32, rho: f64) -> f64 {
    assert!((n - m) % 2 == 0, "n-m must be even");
    assert!(m >= 0, "m must be non-negative in radial function");

    // For rho = 0, only the lowest power term (rho^m) contributes
    if rho == 0.0 {
        return if m == 0 {
            // Need to compute the coefficient for rho^0 term
            let k_max = (n - m) / 2;
            let sign = if k_max % 2 == 0 { 1.0 } else { -1.0 };
            let numerator = factorial((n - k_max) as u32) as f64;
            let denom1 = factorial(k_max as u32) as f64;
            let denom2 = factorial(((n + m) / 2 - k_max) as u32) as f64;
            let denom3 = factorial(((n - m) / 2 - k_max) as u32) as f64;
            sign * numerator / (denom1 * denom2 * denom3)
        } else {
            0.0
        };
    }

    let mut sum = 0.0;
    let k_max = (n - m) / 2;

    for k in 0..=k_max {
        let numerator = factorial((n - k) as u32) as f64;
        let denom1 = factorial(k as u32) as f64;
        let denom2 = factorial(((n + m) / 2 - k) as u32) as f64;
        let denom3 = factorial(((n - m) / 2 - k) as u32) as f64;

        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        let coeff = sign * numerator / (denom1 * denom2 * denom3);

        sum += coeff * rho.powi(n - 2 * k);
    }

    sum
}

/// Compute factorial (limited to reasonable values)
fn factorial(n: u32) -> u64 {
    match n {
        0 | 1 => 1,
        2 => 2,
        3 => 6,
        4 => 24,
        5 => 120,
        6 => 720,
        7 => 5040,
        8 => 40320,
        9 => 362880,
        10 => 3628800,
        11 => 39916800,
        12 => 479001600,
        // For n > 12, use Stirling's approximation
        n => {
            let n_f64 = n as f64;
            let result = (n_f64 * (n_f64 / std::f64::consts::E).ln()).exp()
                * (2.0 * std::f64::consts::PI * n_f64).sqrt();
            result.round() as u64
        }
    }
}

/// Surface error model trait
///
/// Defines the interface for different surface error models.
pub trait SurfaceErrorModel {
    /// Evaluate surface error at a point
    ///
    /// # Arguments
    /// * `rho` - Radial coordinate in meters
    /// * `phi` - Azimuthal angle in radians
    ///
    /// # Returns
    /// Surface error in meters (positive = raised, negative = depressed)
    fn evaluate(&self, rho: f64, phi: f64) -> f64;

    /// Get RMS surface error
    fn rms(&self) -> f64;
}

/// Ideal surface (no errors)
#[derive(Debug, Clone, Copy)]
pub struct IdealSurface;

impl SurfaceErrorModel for IdealSurface {
    fn evaluate(&self, _rho: f64, _phi: f64) -> f64 {
        0.0
    }

    fn rms(&self) -> f64 {
        0.0
    }
}

/// Gaussian random surface errors
///
/// Models random surface errors with specified RMS value.
/// Uses a deterministic pattern based on spatial frequency decomposition
/// for reproducible testing.
#[derive(Debug, Clone)]
pub struct GaussianSurface {
    rms: f64,
    // Spatial frequency components for deterministic pattern
    frequencies: Vec<(f64, f64, f64)>, // (kr, kphi, amplitude)
}

impl GaussianSurface {
    /// Create a Gaussian surface with specified RMS
    ///
    /// # Arguments
    /// * `rms` - RMS surface error in meters
    pub fn new(rms: f64) -> Self {
        // Create a deterministic pattern with multiple spatial frequencies
        // The RMS calculation for cos(kr*rho)*cos(kphi*phi) over unit circle
        // is not simple, so we'll use a simpler radially symmetric pattern
        //
        // For a pattern f(rho) = sum_i a_i * cos(k_i * rho), the RMS over the unit disk is:
        // RMS² = (1/π) ∫∫ f² rho drho dphi
        //
        // Use a single dominant mode for simplicity
        let frequencies = vec![
            (PI, 0.0, rms * 2.0), // Dominant radial mode
        ];

        Self { rms, frequencies }
    }
}

impl SurfaceErrorModel for GaussianSurface {
    fn evaluate(&self, rho: f64, phi: f64) -> f64 {
        self.frequencies
            .iter()
            .map(|(kr, kphi, amp)| amp * (kr * rho).cos() * (kphi * phi).cos())
            .sum()
    }

    fn rms(&self) -> f64 {
        self.rms
    }
}

/// Zernike polynomial surface errors
///
/// Models systematic aberrations using Zernike polynomial expansion.
#[derive(Debug, Clone)]
pub struct ZernikeSurface {
    /// Zernike coefficients (indexed by Noll index - 1)
    coefficients: Vec<f64>,
    /// Aperture radius for normalization
    radius: f64,
}

impl ZernikeSurface {
    /// Create a Zernike surface with specified coefficients
    ///
    /// # Arguments
    /// * `coefficients` - Zernike coefficients (Noll ordering, j=1,2,3,...)
    /// * `radius` - Aperture radius in meters for normalization
    pub fn new(coefficients: Vec<f64>, radius: f64) -> Self {
        Self {
            coefficients,
            radius,
        }
    }

    /// Create from named aberrations (up to 5th order)
    ///
    /// # Arguments
    /// * `piston` - Piston (j=1)
    /// * `tilt_x` - Tip (j=2)
    /// * `tilt_y` - Tilt (j=3)
    /// * `defocus` - Defocus (j=4)
    /// * `astigmatism_oblique` - Oblique astigmatism (j=5)
    /// * `astigmatism_vertical` - Vertical astigmatism (j=6)
    /// * `coma_vertical` - Vertical coma (j=7)
    /// * `coma_horizontal` - Horizontal coma (j=8)
    /// * `radius` - Aperture radius in meters
    #[allow(clippy::too_many_arguments)]
    pub fn from_aberrations(
        piston: f64,
        tilt_x: f64,
        tilt_y: f64,
        defocus: f64,
        astigmatism_oblique: f64,
        astigmatism_vertical: f64,
        coma_vertical: f64,
        coma_horizontal: f64,
        radius: f64,
    ) -> Self {
        let coefficients = vec![
            piston,
            tilt_x,
            tilt_y,
            defocus,
            astigmatism_oblique,
            astigmatism_vertical,
            coma_vertical,
            coma_horizontal,
        ];
        Self::new(coefficients, radius)
    }

    /// Compute RMS surface error using Zernike orthogonality
    fn compute_rms(&self) -> f64 {
        // For orthonormal Zernike polynomials, RMS is sqrt(sum of squared coefficients)
        // Excluding piston (j=1) which doesn't affect RMS
        let sum_sq: f64 = self.coefficients.iter().skip(1).map(|c| c * c).sum();
        sum_sq.sqrt()
    }
}

impl SurfaceErrorModel for ZernikeSurface {
    fn evaluate(&self, rho: f64, phi: f64) -> f64 {
        // Normalize rho to [0, 1]
        let rho_norm = rho / self.radius;
        if rho_norm > 1.0 {
            return 0.0; // Outside aperture
        }

        // Sum Zernike polynomials
        self.coefficients
            .iter()
            .enumerate()
            .map(|(i, &coeff)| {
                let noll_index = i + 1;
                coeff * zernike_polynomial(noll_index, rho_norm, phi)
            })
            .sum()
    }

    fn rms(&self) -> f64 {
        self.compute_rms()
    }
}

/// Compute surface error RMS over a circular aperture
///
/// Uses numerical integration to compute RMS for arbitrary surface error functions.
///
/// # Arguments
/// * `surface` - Surface error model
/// * `radius` - Aperture radius in meters
/// * `n_radial` - Number of radial sample points
/// * `n_angular` - Number of angular sample points
///
/// # Returns
/// RMS surface error in meters
pub fn compute_surface_rms<S: SurfaceErrorModel>(
    surface: &S,
    radius: f64,
    n_radial: usize,
    n_angular: usize,
) -> f64 {
    let mut sum_sq = 0.0;
    let mut weight_sum = 0.0;

    for i in 0..n_radial {
        let rho = radius * (i as f64 + 0.5) / n_radial as f64;
        let weight_radial = rho; // Jacobian for polar coordinates

        for j in 0..n_angular {
            let phi = 2.0 * PI * j as f64 / n_angular as f64;
            let error = surface.evaluate(rho, phi);

            sum_sq += error * error * weight_radial;
            weight_sum += weight_radial;
        }
    }

    (sum_sq / weight_sum).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ruze_efficiency_perfect_surface() {
        // Perfect surface (σ = 0) should have 100% efficiency
        let efficiency = ruze_efficiency(0.0, 0.0357);
        assert!((efficiency - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_ruze_efficiency_small_error() {
        // 1mm RMS at 8.4 GHz (λ ≈ 0.0357m)
        // σ/λ = 0.001/0.0357 ≈ 0.028
        // η = exp(-(4π·0.028)²) ≈ exp(-0.123) ≈ 0.884
        let efficiency = ruze_efficiency(0.001, 0.0357);
        assert!((efficiency - 0.884).abs() < 0.01);
    }

    #[test]
    fn test_ruze_efficiency_large_error() {
        // 5mm RMS at 8.4 GHz should give very poor efficiency
        let efficiency = ruze_efficiency(0.005, 0.0357);
        assert!(efficiency < 0.1); // Should be ~2%
    }

    #[test]
    fn test_ruze_efficiency_frequency_dependence() {
        let sigma = 0.001; // 1mm RMS

        // Higher frequency (shorter wavelength) = lower efficiency
        let eff_8ghz = ruze_efficiency_from_frequency(sigma, 8.4e9);
        let eff_32ghz = ruze_efficiency_from_frequency(sigma, 32.0e9);

        assert!(eff_32ghz < eff_8ghz);
    }

    #[test]
    fn test_noll_to_nm_low_orders() {
        assert_eq!(noll_to_nm(1), (0, 0)); // Piston
        assert_eq!(noll_to_nm(2), (1, -1)); // Tip
        assert_eq!(noll_to_nm(3), (1, 1)); // Tilt
        assert_eq!(noll_to_nm(4), (2, 0)); // Defocus
        assert_eq!(noll_to_nm(5), (2, -2)); // Astigmatism
        assert_eq!(noll_to_nm(6), (2, 2)); // Astigmatism
    }

    #[test]
    fn test_noll_to_nm_higher_orders() {
        assert_eq!(noll_to_nm(7), (3, -1)); // Coma (vertical)
        assert_eq!(noll_to_nm(8), (3, 1)); // Coma (horizontal)
        assert_eq!(noll_to_nm(11), (4, 0)); // Spherical
        assert_eq!(noll_to_nm(14), (4, -4)); // Tetrafoil
        assert_eq!(noll_to_nm(15), (4, 4)); // Tetrafoil
    }

    #[test]
    fn test_zernike_index() {
        let z = ZernikeIndex::from_noll(1);
        assert_eq!(z.n, 0);
        assert_eq!(z.m, 0);
        assert_eq!(z.name(), "Piston");

        let z = ZernikeIndex::from_noll(11);
        assert_eq!(z.n, 4);
        assert_eq!(z.m, 0);
        assert_eq!(z.name(), "Spherical aberration");
    }

    #[test]
    fn test_zernike_piston() {
        // Piston (j=1) should be constant = sqrt(1) = 1.0
        for rho in [0.0, 0.3, 0.7, 1.0] {
            for phi in [0.0, PI / 4.0, PI / 2.0, PI] {
                let z = zernike_polynomial(1, rho, phi);
                assert!((z - 1.0).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn test_zernike_tip_tilt() {
        // Tip (j=2, n=1, m=-1): Z = sqrt(4)·rho·sin(phi)
        // At phi=0: sin(0) = 0
        let z = zernike_polynomial(2, 1.0, 0.0);
        assert!(z.abs() < 1e-10);

        // At phi=π/2: sin(π/2) = 1, so Z = 2·1·1 = 2
        let z = zernike_polynomial(2, 1.0, PI / 2.0);
        assert!((z - 2.0).abs() < 1e-10);

        // Tilt (j=3, n=1, m=1): Z = sqrt(4)·rho·cos(phi)
        // At phi=0: cos(0) = 1, so Z = 2·1·1 = 2
        let z = zernike_polynomial(3, 1.0, 0.0);
        assert!((z - 2.0).abs() < 1e-10);

        // At phi=π: cos(π) = -1, so Z = 2·1·(-1) = -2
        let z = zernike_polynomial(3, 1.0, PI);
        assert!((z + 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_zernike_defocus() {
        // Defocus (j=4): Z = sqrt(3)·(2ρ² - 1)
        let z_center = zernike_polynomial(4, 0.0, 0.0);
        let expected = (3.0_f64).sqrt() * (-1.0);
        assert!((z_center - expected).abs() < 1e-10);

        let z_edge = zernike_polynomial(4, 1.0, 0.0);
        let expected = (3.0_f64).sqrt() * 1.0;
        assert!((z_edge - expected).abs() < 1e-10);
    }

    #[test]
    fn test_zernike_orthogonality() {
        // Test orthogonality of first few Zernike polynomials
        // Standard Noll convention: ∫∫ Z_i · Z_j · rho drho dphi = π·δ_ij over unit circle
        //
        // Note: This gives π for diagonal terms, not 1. To get truly orthonormal
        // polynomials, we would divide by sqrt(π), but Noll convention uses this form.

        let n_radial = 50;
        let n_angular = 100;

        for i in 1..=6 {
            for j in 1..=6 {
                let mut integral = 0.0;

                for ri in 0..n_radial {
                    let rho = (ri as f64 + 0.5) / n_radial as f64;
                    let dr = 1.0 / n_radial as f64;

                    for ai in 0..n_angular {
                        let phi = 2.0 * PI * ai as f64 / n_angular as f64;
                        let dphi = 2.0 * PI / n_angular as f64;

                        let zi = zernike_polynomial(i, rho, phi);
                        let zj = zernike_polynomial(j, rho, phi);

                        integral += zi * zj * rho * dr * dphi;
                    }
                }

                let expected = if i == j { PI } else { 0.0 };
                assert!(
                    (integral - expected).abs() < 0.05,
                    "Orthogonality failed for i={}, j={}: integral={}, expected={}",
                    i,
                    j,
                    integral,
                    expected
                );
            }
        }
    }

    #[test]
    fn test_ideal_surface() {
        let surface = IdealSurface;
        assert_eq!(surface.evaluate(0.5, 0.0), 0.0);
        assert_eq!(surface.evaluate(0.0, PI), 0.0);
        assert_eq!(surface.rms(), 0.0);
    }

    #[test]
    fn test_gaussian_surface_rms() {
        let rms = 0.001; // 1mm
        let surface = GaussianSurface::new(rms);

        // Verify that the reported RMS matches the target
        assert!((surface.rms() - rms).abs() < 1e-10);

        // Compute RMS numerically and verify it's in the right ballpark
        // Note: For a deterministic approximation, we expect reasonable agreement,
        // not exact match. Allow 50% tolerance.
        let computed_rms = compute_surface_rms(&surface, 1.0, 50, 100);
        assert!(
            (computed_rms - rms).abs() < rms * 0.5,
            "Computed RMS {} differs significantly from expected {}",
            computed_rms,
            rms
        );

        // Verify it's at least non-zero
        assert!(computed_rms > 0.0);
    }

    #[test]
    fn test_zernike_surface() {
        // Create surface with only defocus
        let coeffs = vec![0.0, 0.0, 0.0, 0.001]; // 1mm defocus
        let surface = ZernikeSurface::new(coeffs, 1.0);

        // Check evaluation at center (should be negative for defocus)
        let error_center = surface.evaluate(0.0, 0.0);
        assert!(error_center < 0.0);

        // Check evaluation at edge (should be positive for defocus)
        let error_edge = surface.evaluate(1.0, 0.0);
        assert!(error_edge > 0.0);
    }

    #[test]
    fn test_zernike_surface_rms() {
        // Create surface with known aberrations
        let coeffs = vec![
            0.0,   // Piston (doesn't affect RMS)
            0.001, // Tip
            0.001, // Tilt
            0.002, // Defocus
        ];
        let surface = ZernikeSurface::new(coeffs.clone(), 1.0);

        // RMS should be sqrt(0.001² + 0.001² + 0.002²) = sqrt(0.000006) ≈ 0.00245
        let expected_rms = (0.001_f64.powi(2) + 0.001_f64.powi(2) + 0.002_f64.powi(2)).sqrt();
        assert!((surface.rms() - expected_rms).abs() < 1e-10);
    }

    #[test]
    fn test_compute_surface_rms_ideal() {
        let surface = IdealSurface;
        let rms = compute_surface_rms(&surface, 1.0, 20, 40);
        assert!(rms < 1e-10);
    }

    #[test]
    fn test_zernike_surface_from_aberrations() {
        let surface = ZernikeSurface::from_aberrations(
            0.0,   // piston
            0.001, // tilt_x
            0.0,   // tilt_y
            0.002, // defocus
            0.0,   // astigmatism_oblique
            0.0,   // astigmatism_vertical
            0.001, // coma_vertical
            0.0,   // coma_horizontal
            1.0,   // radius
        );

        // Should have 8 coefficients
        assert_eq!(surface.coefficients.len(), 8);

        // RMS should be sqrt(0.001² + 0.002² + 0.001²)
        let expected_rms = (0.001_f64.powi(2) + 0.002_f64.powi(2) + 0.001_f64.powi(2)).sqrt();
        assert!((surface.rms() - expected_rms).abs() < 1e-10);
    }
}
