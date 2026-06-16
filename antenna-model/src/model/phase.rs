//! Phase Function Implementations
//!
//! This module implements the phase components for physical optics integration
//! over the reflector aperture. The total phase at each aperture point determines
//! the interference pattern in the far field.
//!
//! # Phase Components
//!
//! The total phase is a sum of several contributions:
//!
//! 1. **Path Phase** (`phase_path`): Standard parabolic reflector phase
//! 2. **Feed Displacement Phase** (`phase_feed_displacement`): Coma aberration from off-axis feed
//! 3. **Surface Error Phase** (`phase_surface_error`): Random and systematic surface deviations
//! 4. **Mesh Phase** (`phase_mesh`): Wire mesh reflector effects
//!
//! # Mathematical Foundation
//!
//! Based on design doc Section 2.2 (Phase Components)
//!
//! ## Total Phase
//! ```text
//! Ψ_total = Ψ_path + Ψ_feed_displacement + Ψ_surface + Ψ_mesh
//! ```

use std::f64::consts::PI;

use crate::model::coordinates::ApertureCoordinates;

/// Calculate wavenumber from wavelength
///
/// # Formula
/// ```text
/// k = 2π/λ
/// ```
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Wavenumber in radians per meter
#[inline]
pub fn wavenumber(wavelength: f64) -> f64 {
    2.0 * PI / wavelength
}

/// Calculate wavelength from frequency
///
/// # Formula
/// ```text
/// λ = c/f
/// ```
///
/// # Arguments
/// - `frequency_hz`: Frequency in Hz
///
/// # Returns
/// Wavelength in meters
#[inline]
pub fn wavelength_from_frequency(frequency_hz: f64) -> f64 {
    const SPEED_OF_LIGHT: f64 = 299_792_458.0; // m/s
    SPEED_OF_LIGHT / frequency_hz
}

/// Standard parabolic path phase
///
/// This is the phase contribution from the parabolic reflector geometry
/// when the feed is at the focal point and observing in direction (θ, φ).
///
/// # Derivation
///
/// For a parabola z = ρ²/(4f), the feed→surface optical path is k(f + z).
/// The far-field projection removes k(ρ·sinθ·cos(φ−φ') + z·cosθ). Dropping
/// the constant term kf:
///
/// ```text
/// Ψ_path = k·[z·(1−cosθ) − ρ·sinθ·cos(φ−φ')],  z = ρ²/(4f)
///        = k·[ρ²/(4f)·(1−cosθ) − ρ·sinθ·cos(φ−φ')]
/// ```
///
/// The `(1−cosθ)` factor is essential: it ensures the aperture is equiphase
/// at boresight (θ = 0), which is the defining optical property of a parabola.
/// Without it, a large spurious defocus phase is injected across the aperture,
/// corrupting the off-axis pattern.
///
/// # Arguments
/// - `rho`: Radial distance from axis in aperture plane (meters)
/// - `phi_prime`: Azimuthal angle in aperture plane (radians)
/// - `theta`: Polar angle in far field (radians, from boresight)
/// - `phi`: Azimuthal angle in far field (radians)
/// - `focal_length`: Reflector focal length (meters)
/// - `k`: Wavenumber (radians/meter)
///
/// # Returns
/// Phase in radians
#[inline]
pub fn phase_path(
    rho: f64,
    phi_prime: f64,
    theta: f64,
    phi: f64,
    focal_length: f64,
    k: f64,
) -> f64 {
    // Feed→surface path is k(f+z); far-field projection removes
    // k(ρ·sinθ·cos(φ−φ′) + z·cosθ). Dropping the constant kf:
    //   Ψ = k·[z·(1−cosθ) − ρ·sinθ·cos(φ−φ′)],  z = ρ²/(4f)
    let term1 = rho * rho / (4.0 * focal_length) * (1.0 - theta.cos());
    let term2 = rho * theta.sin() * (phi - phi_prime).cos();
    k * (term1 - term2)
}

/// Feed displacement phase (coma and defocus aberrations)
///
/// When the feed is displaced from the focal point, aberrations are introduced including
/// beam steering, coma (asymmetric sidelobes), defocus, and gain loss. This function
/// computes the exact phase difference using full path-length analysis.
///
/// # Algorithm
///
/// Computes the actual path length difference between:
/// - Path from ideal focal point `(0, 0, f)` to each aperture point on the parabolic surface
/// - Path from displaced feed position `(δ·cos(α), δ·sin(α), f + δz)` to each aperture point
///
/// This naturally includes all orders of aberration:
/// - First order (linear): Beam steering (θ ≈ δ/f)
/// - Second order: Defocus/astigmatism effects (both lateral and axial displacements)
/// - Third order: True coma with asymmetric sidelobes
/// - Higher orders: Additional aberrations for large displacements
///
/// # Geometry
///
/// For a parabolic reflector with equation z = ρ²/(4f):
/// - Aperture point in Cartesian: (x, y, z) where x = ρ·cos(φ'), y = ρ·sin(φ')
/// - Ideal focus at: (0, 0, f)
/// - Displaced feed at: (δ·cos(α), δ·sin(α), f + δz)
///
/// When `delta_z = 0` this function is identical to the previous 6-argument model.
///
/// # Arguments
/// - `rho`: Radial distance from axis in aperture plane (meters)
/// - `phi_prime`: Azimuthal angle in aperture plane (radians)
/// - `delta_feed`: Lateral feed displacement magnitude from focal axis (meters)
/// - `alpha`: Direction angle of lateral feed displacement (radians)
/// - `delta_z`: Axial feed offset from focal point (meters, positive = away from vertex)
/// - `focal_length`: Reflector focal length (meters)
/// - `k`: Wavenumber (radians/meter)
///
/// # Returns
/// Phase difference in radians (positive when displaced path is longer)
///
/// # References
/// - Rusch & Potter, "Analysis of Reflector Antennas" (1970)
/// - Love, "Electromagnetic Horn Antennas" (1976), Ch 10
/// - Silver, "Microwave Antenna Theory and Design" (1949), Ch 12
#[inline]
pub fn phase_feed_displacement(
    rho: f64,
    phi_prime: f64,
    delta_feed: f64,
    alpha: f64,
    delta_z: f64,
    focal_length: f64,
    k: f64,
) -> f64 {
    // Convert aperture point from polar to Cartesian coordinates
    let x = rho * phi_prime.cos();
    let y = rho * phi_prime.sin();

    // Surface height on parabola: z = ρ²/(4f)
    let z = rho * rho / (4.0 * focal_length);

    // Lateral feed displacement in Cartesian (in focal plane)
    let dx = delta_feed * alpha.cos();
    let dy = delta_feed * alpha.sin();

    // Distance from ideal focal point (0, 0, f) to surface point
    // For parabola, all paths from focus to surface to aperture plane are equal,
    // but we need the actual geometric distance for phase calculation
    let dz_ideal = z - focal_length;
    let path_ideal = (x * x + y * y + dz_ideal * dz_ideal).sqrt();

    // Distance from displaced feed (dx, dy, f + delta_z) to surface point
    // The axial offset delta_z shifts the z-position of the feed, producing
    // a ρ-dependent (defocus) phase when delta_z != 0
    let dz_displaced = z - (focal_length + delta_z);
    let path_displaced = ((x - dx).powi(2) + (y - dy).powi(2) + dz_displaced * dz_displaced).sqrt();

    // Phase difference: k × (displaced_path - ideal_path)
    // Positive phase when displaced feed creates longer path
    k * (path_displaced - path_ideal)
}

/// Surface error phase contribution
///
/// Random and systematic deviations of the reflector surface from the ideal
/// parabolic shape introduce phase errors that degrade antenna performance.
///
/// # Formula (from design doc Section 2.2)
/// ```text
/// Ψ_surface = (4π/λ)·ε(ρ,φ')·cos(θ_incident)
/// ```
///
/// The factor of 4π/λ = 2k accounts for the round-trip path (incident + reflected).
/// The cos(θ_incident) factor accounts for the angle of incidence on the surface.
///
/// # Arguments
/// - `epsilon`: Surface deviation from ideal at this point (meters, positive = away from vertex)
/// - `theta_incident`: Angle of incidence on surface (radians)
/// - `k`: Wavenumber (radians/meter)
///
/// # Returns
/// Phase in radians
#[inline]
pub fn phase_surface_error(epsilon: f64, theta_incident: f64, k: f64) -> f64 {
    2.0 * k * epsilon * theta_incident.cos()
}

/// Wire mesh reflector phase contribution
///
/// For mesh reflectors, the periodic wire structure introduces frequency-dependent
/// phase shifts, especially at low frequencies where the wavelength approaches
/// the mesh spacing.
///
/// # Formula (from design doc Section 2.2)
/// ```text
/// Ψ_mesh = arctan[(2π·d_mesh/λ)·sin(θ_incident)]
/// ```
///
/// This is a simplified model; full analysis requires Floquet mode decomposition.
/// At high frequencies (λ << d_mesh), the phase shift becomes negligible.
///
/// # Arguments
/// - `mesh_spacing`: Distance between parallel wires (meters)
/// - `theta_incident`: Angle of incidence on surface (radians)
/// - `k`: Wavenumber (radians/meter)
///
/// # Returns
/// Phase in radians
#[inline]
pub fn phase_mesh(mesh_spacing: f64, theta_incident: f64, k: f64) -> f64 {
    let wavelength = 2.0 * PI / k;
    let normalized_spacing = 2.0 * PI * mesh_spacing / wavelength;
    (normalized_spacing * theta_incident.sin()).atan()
}

/// Total phase combining all contributions
///
/// Computes the complete phase at an aperture point, including all physical effects:
/// path phase, coma aberration (lateral feed displacement), axial defocus (feed axial
/// offset from focus), surface errors, and mesh effects.
///
/// # Arguments
/// - `aperture`: Aperture coordinates (ρ, φ')
/// - `theta`: Far-field polar angle (radians)
/// - `phi`: Far-field azimuthal angle (radians)
/// - `focal_length`: Reflector focal length (meters)
/// - `feed_displacement`: Lateral feed displacement magnitude from focal axis (meters)
/// - `feed_displacement_angle`: Direction of lateral feed displacement (radians)
/// - `feed_axial_offset`: Axial feed offset from focal point (meters, positive = away from vertex)
/// - `surface_error`: Surface deviation at this point (meters)
/// - `theta_incident`: Angle of incidence (radians)
/// - `mesh_spacing`: Mesh spacing (meters), or 0.0 for solid reflector
/// - `k`: Wavenumber (radians/meter)
///
/// # Returns
/// Total phase in radians
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn phase_total(
    aperture: ApertureCoordinates,
    theta: f64,
    phi: f64,
    focal_length: f64,
    feed_displacement: f64,
    feed_displacement_angle: f64,
    feed_axial_offset: f64,
    surface_error: f64,
    theta_incident: f64,
    mesh_spacing: f64,
    k: f64,
) -> f64 {
    let mut total = phase_path(
        aperture.rho,
        aperture.phi_prime,
        theta,
        phi,
        focal_length,
        k,
    );

    // Add feed displacement phase (lateral coma + axial defocus) if feed is displaced
    if feed_displacement > 0.0 || feed_axial_offset.abs() > 0.0 {
        total += phase_feed_displacement(
            aperture.rho,
            aperture.phi_prime,
            feed_displacement,
            feed_displacement_angle,
            feed_axial_offset,
            focal_length,
            k,
        );
    }

    // Add surface error phase if surface is not ideal
    if surface_error.abs() > 0.0 {
        total += phase_surface_error(surface_error, theta_incident, k);
    }

    // Add mesh phase if mesh reflector
    if mesh_spacing > 0.0 {
        total += phase_mesh(mesh_spacing, theta_incident, k);
    }

    total
}

/// Surface error model trait
///
/// Defines interface for different surface error models (Gaussian random,
/// Zernike polynomials, measured surface maps, etc.)
pub trait SurfaceErrorModel {
    /// Get surface deviation at aperture point (ρ, φ')
    ///
    /// # Arguments
    /// - `rho`: Radial distance from axis (meters)
    /// - `phi_prime`: Azimuthal angle (radians)
    ///
    /// # Returns
    /// Surface deviation in meters (positive = away from vertex)
    fn error_at(&self, rho: f64, phi_prime: f64) -> f64;

    /// Get RMS surface error over the aperture
    fn rms(&self) -> f64;
}

/// Ideal surface (no errors)
#[derive(Debug, Clone, Copy)]
pub struct IdealSurface;

impl SurfaceErrorModel for IdealSurface {
    fn error_at(&self, _rho: f64, _phi_prime: f64) -> f64 {
        0.0
    }

    fn rms(&self) -> f64 {
        0.0
    }
}

/// Gaussian random surface errors
///
/// Models random surface deviations with Gaussian distribution.
/// Useful for Monte Carlo simulations and testing.
#[derive(Debug, Clone)]
pub struct GaussianSurface {
    /// RMS surface deviation (meters)
    pub rms: f64,
    /// Spatial correlation length (meters)
    pub correlation_length: f64,
    /// Random seed for reproducibility
    seed: u64,
}

impl GaussianSurface {
    /// Create new Gaussian surface model
    pub fn new(rms: f64, correlation_length: f64, seed: u64) -> Self {
        Self {
            rms,
            correlation_length,
            seed,
        }
    }

    /// Simple hash function for pseudo-random generation
    /// This is a placeholder - in production, use a proper RNG
    fn hash(&self, rho: f64, phi_prime: f64) -> f64 {
        // Simple deterministic pseudo-random based on position
        // This is NOT cryptographically secure, just for testing
        let x = (rho * 1000.0) as u64;
        let y = (phi_prime * 1000.0) as u64;
        let h = x
            .wrapping_mul(2654435761)
            .wrapping_add(y.wrapping_mul(2654435789));
        let h = h.wrapping_add(self.seed);

        // Map to [-1, 1]
        (h % 1000) as f64 / 500.0 - 1.0
    }
}

impl SurfaceErrorModel for GaussianSurface {
    fn error_at(&self, rho: f64, phi_prime: f64) -> f64 {
        // Simple approximation: scale hash output by RMS
        // In production, use proper Gaussian random with spatial correlation
        self.rms * self.hash(rho, phi_prime)
    }

    fn rms(&self) -> f64 {
        self.rms
    }
}

/// Zernike polynomial surface errors
///
/// Models systematic aberrations using Zernike polynomials, which form
/// an orthonormal basis over a circular aperture.
///
/// Common Zernike terms:
/// - Piston (Z0): Constant offset
/// - Tilt (Z1, Z2): Linear tilt in x and y
/// - Defocus (Z3): Quadratic focus error
/// - Astigmatism (Z4, Z5): Cylindrical aberrations
/// - Coma (Z6, Z7): Third-order coma
/// - Spherical (Z8): Spherical aberration
///
/// This is a simplified implementation supporting up to 5th order.
#[derive(Debug, Clone)]
pub struct ZernikeSurface {
    /// Zernike coefficients (meters)
    /// Index corresponds to Noll ordering
    pub coefficients: Vec<f64>,
    /// Aperture radius for normalization (meters)
    pub aperture_radius: f64,
}

impl ZernikeSurface {
    /// Create new Zernike surface model
    pub fn new(coefficients: Vec<f64>, aperture_radius: f64) -> Self {
        Self {
            coefficients,
            aperture_radius,
        }
    }

    /// Evaluate Zernike polynomial at normalized coordinates
    ///
    /// This is a simplified implementation of common low-order terms.
    /// For production use, implement full Zernike recursion.
    fn zernike_at(&self, rho_norm: f64, phi: f64, j: usize) -> f64 {
        match j {
            0 => 1.0,                                                      // Piston
            1 => rho_norm * phi.cos(),                                     // Tilt X
            2 => rho_norm * phi.sin(),                                     // Tilt Y
            3 => 2.0 * rho_norm * rho_norm - 1.0,                          // Defocus
            4 => rho_norm * rho_norm * (2.0 * phi).cos(),                  // Astigmatism 0°
            5 => rho_norm * rho_norm * (2.0 * phi).sin(),                  // Astigmatism 45°
            6 => (3.0 * rho_norm * rho_norm - 2.0) * rho_norm * phi.cos(), // Coma X
            7 => (3.0 * rho_norm * rho_norm - 2.0) * rho_norm * phi.sin(), // Coma Y
            8 => 6.0 * rho_norm.powi(4) - 6.0 * rho_norm * rho_norm + 1.0, // Spherical
            _ => 0.0, // Higher orders not implemented
        }
    }
}

impl SurfaceErrorModel for ZernikeSurface {
    fn error_at(&self, rho: f64, phi_prime: f64) -> f64 {
        let rho_norm = rho / self.aperture_radius;

        // Sum contributions from all Zernike terms
        self.coefficients
            .iter()
            .enumerate()
            .map(|(j, &coeff)| coeff * self.zernike_at(rho_norm, phi_prime, j))
            .sum()
    }

    fn rms(&self) -> f64 {
        // For Zernike polynomials, RMS is approximately the L2 norm of coefficients
        // (exact calculation requires integration, this is an approximation)
        let sum_squares: f64 = self.coefficients.iter().skip(1).map(|c| c * c).sum();
        sum_squares.sqrt()
    }
}

/// Calculate angle of incidence for a point on the parabolic surface
///
/// For a parabolic reflector, rays from the feed to a surface point at (ρ, φ')
/// reflect toward the far field. The angle of incidence depends on the surface
/// normal at that point.
///
/// # Arguments
/// - `rho`: Radial distance in aperture plane (meters)
/// - `focal_length`: Reflector focal length (meters)
///
/// # Returns
/// Angle of incidence in radians (from surface normal)
pub fn angle_of_incidence(rho: f64, focal_length: f64) -> f64 {
    // For parabola z = ρ²/(4f), the surface normal makes angle θ with z-axis
    // where tan(θ) = dz/dρ = ρ/(2f)
    // The angle of incidence for axial illumination is approximately θ
    (rho / (2.0 * focal_length)).atan()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const EPSILON: f64 = 1e-10;

    #[test]
    fn test_wavenumber() {
        // λ = 3 cm (X-band, ~10 GHz)
        let wavelength = 0.03;
        let k = wavenumber(wavelength);
        let expected = 2.0 * PI / 0.03;
        assert!((k - expected).abs() < EPSILON);
    }

    #[test]
    fn test_wavelength_from_frequency() {
        // 10 GHz
        let freq = 10e9;
        let lambda = wavelength_from_frequency(freq);
        // λ = c/f = 299792458 / 1e10 ≈ 0.0299792458 m ≈ 3 cm
        let expected = 299_792_458.0 / freq;
        assert!((lambda - expected).abs() < 1e-10);

        // Verify it's approximately 3 cm
        assert!((lambda - 0.03).abs() < 0.001);
    }

    #[test]
    fn test_phase_path_on_axis() {
        // On-axis (θ=0): aperture must be equiphase (defining property of a parabola).
        // The (1−cosθ) factor ensures the quadratic term vanishes at boresight.
        let focal_length = 17.0;
        let k = wavenumber(0.03);

        let phase = phase_path(1.0, 0.0, 0.0, 0.0, focal_length, k);
        assert!(
            phase.abs() < EPSILON,
            "on-axis phase should be 0, got {phase}"
        );
    }

    #[test]
    fn test_phase_path_boresight_equiphase() {
        // For a parabola fed at focus, the aperture is equiphase at theta=0.
        // This must hold for all radii — it is the defining optical property of a parabola.
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        for rho in [0.0, 1.0, 5.0, 10.0, 17.0] {
            let phase = phase_path(rho, 0.7, 0.0, 0.0, focal_length, k);
            assert!(phase.abs() < EPSILON, "rho={rho}: phase={phase}");
        }
    }

    #[test]
    fn test_phase_path_off_axis() {
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let (rho, phi_prime, theta, phi) = (5.0, PI / 4.0, 0.1, PI / 3.0);

        let phase = phase_path(rho, phi_prime, theta, phi, focal_length, k);

        // Correct formula: Ψ = k·[ρ²/(4f)·(1−cosθ) − ρ·sinθ·cos(φ−φ')]
        let term1 = rho * rho / (4.0 * focal_length) * (1.0 - theta.cos());
        let term2 = rho * theta.sin() * (phi - phi_prime).cos();
        assert!((phase - k * (term1 - term2)).abs() < EPSILON);
    }

    #[test]
    fn test_phase_feed_displacement_zero() {
        // Zero displacement should give zero phase
        let k = wavenumber(0.03);
        let phase = phase_feed_displacement(5.0, 0.0, 0.0, 0.0, 0.0, 17.0, k);
        assert!(phase.abs() < EPSILON);
    }

    #[test]
    fn test_phase_feed_displacement_nonzero() {
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let rho = 5.0;
        let phi_prime = PI / 4.0;
        let delta_feed = 1.0; // 1 meter displacement
        let alpha = 0.0; // Displacement in x-direction

        let phase =
            phase_feed_displacement(rho, phi_prime, delta_feed, alpha, 0.0, focal_length, k);

        // Should be non-zero for off-axis feed
        assert!(phase.abs() > 0.0);

        // For small displacements, phase should be approximately linear: Ψ ≈ k·δ·ρ/f·cos(φ'-α)
        // The full path-length model gives slightly different values due to higher-order terms
        let linear_approx = k * delta_feed * rho / focal_length * (phi_prime - alpha).cos();

        // Phase should be in the same ballpark as linear approximation (within 50%)
        // but not exactly equal due to higher-order aberration terms
        assert!(
            (phase.abs() - linear_approx.abs()).abs() < linear_approx.abs() * 0.5,
            "Phase {:.6} should be similar to linear approx {:.6}",
            phase,
            linear_approx
        );
    }

    #[test]
    fn test_phase_feed_displacement_symmetry() {
        // Phase should be symmetric about the displacement direction
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let rho = 5.0;
        let delta_feed = 1.0;
        let alpha = 0.0; // Displacement in +x direction

        // Points at +phi_prime and -phi_prime should have equal magnitude phase
        let phase_pos =
            phase_feed_displacement(rho, PI / 4.0, delta_feed, alpha, 0.0, focal_length, k);
        let phase_neg =
            phase_feed_displacement(rho, -PI / 4.0, delta_feed, alpha, 0.0, focal_length, k);

        assert!(
            (phase_pos - phase_neg).abs() < 1e-10,
            "Phase should be symmetric: +φ'={:.6}, -φ'={:.6}",
            phase_pos,
            phase_neg
        );
    }

    #[test]
    fn test_phase_feed_displacement_opposite_sides() {
        // Points on opposite sides of the aperture (relative to displacement)
        // should have opposite sign phases
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let rho = 5.0;
        let delta_feed = 1.0;
        let alpha = 0.0; // Displacement in +x direction

        // Point in +x direction (φ'=0) - displaced feed is closer
        let phase_toward =
            phase_feed_displacement(rho, 0.0, delta_feed, alpha, 0.0, focal_length, k);

        // Point in -x direction (φ'=π) - displaced feed is farther
        let phase_away = phase_feed_displacement(rho, PI, delta_feed, alpha, 0.0, focal_length, k);

        // Phases should have opposite signs
        assert!(
            phase_toward * phase_away < 0.0,
            "Phases should have opposite signs: toward={:.6}, away={:.6}",
            phase_toward,
            phase_away
        );

        // Away should have larger magnitude (longer path)
        assert!(
            phase_away.abs() > phase_toward.abs(),
            "Away phase {:.6} should be larger than toward {:.6}",
            phase_away.abs(),
            phase_toward.abs()
        );
    }

    #[test]
    fn test_phase_feed_displacement_at_center() {
        // At the center of the aperture (ρ=0, vertex of parabola):
        // - Surface point: (0, 0, 0)
        // - Ideal focus: (0, 0, f)
        // - Displaced feed: (δ, 0, f)
        //
        // Path from ideal: f
        // Path from displaced: sqrt(δ² + f²)
        // Difference: sqrt(δ² + f²) - f ≈ δ²/(2f) for small δ
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let delta_feed = 1.0;
        let alpha = 0.0;

        let phase = phase_feed_displacement(0.0, 0.0, delta_feed, alpha, 0.0, focal_length, k);

        // Expected: k * (sqrt(δ² + f²) - f)
        let expected = k * ((delta_feed.powi(2) + focal_length.powi(2)).sqrt() - focal_length);

        assert!(
            (phase - expected).abs() < 1e-6,
            "Phase at center {:.6} should match expected {:.6}",
            phase,
            expected
        );
    }

    #[test]
    fn test_phase_feed_displacement_geometry_validation() {
        // Validate the geometry calculation directly
        let focal_length = 10.0;
        let k = 1.0; // Use k=1 for easy calculation
        let rho = 4.0; // Surface point at ρ=4
        let phi_prime = 0.0; // On x-axis
        let delta_feed = 1.0; // 1m displacement
        let alpha = 0.0; // In +x direction

        // Aperture point: (4, 0, z) where z = 16/(4*10) = 0.4
        // Ideal focus: (0, 0, 10)
        // Displaced feed: (1, 0, 10)
        //
        // Path from ideal: sqrt(16 + 0 + 92.16) = sqrt(108.16) ≈ 10.4
        // Path from displaced: sqrt(9 + 0 + 92.16) = sqrt(101.16) ≈ 10.058

        let phase =
            phase_feed_displacement(rho, phi_prime, delta_feed, alpha, 0.0, focal_length, k);

        // Expected: path_displaced - path_ideal ≈ 10.058 - 10.4 = -0.342
        let expected = (101.16_f64).sqrt() - (108.16_f64).sqrt();

        assert!(
            (phase - expected).abs() < 1e-6,
            "Phase {:.6} should match geometry calculation {:.6}",
            phase,
            expected
        );
    }

    #[test]
    fn test_phase_feed_displacement_increases_with_rho() {
        // Phase magnitude should generally increase with ρ
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let delta_feed = 1.0;
        let alpha = 0.0;
        let phi_prime = PI; // Away from displacement

        let phase_small =
            phase_feed_displacement(2.0, phi_prime, delta_feed, alpha, 0.0, focal_length, k);
        let phase_large =
            phase_feed_displacement(8.0, phi_prime, delta_feed, alpha, 0.0, focal_length, k);

        assert!(
            phase_large.abs() > phase_small.abs(),
            "Phase at larger ρ ({:.6}) should exceed phase at smaller ρ ({:.6})",
            phase_large.abs(),
            phase_small.abs()
        );
    }

    #[test]
    fn test_phase_surface_error_zero() {
        // Ideal surface (ε=0) should give zero phase
        let k = wavenumber(0.03);
        let phase = phase_surface_error(0.0, 0.0, k);
        assert!(phase.abs() < EPSILON);
    }

    #[test]
    fn test_phase_surface_error_nonzero() {
        let k = wavenumber(0.03);
        let epsilon = 0.001; // 1 mm surface error
        let theta_incident = PI / 6.0; // 30 degrees

        let phase = phase_surface_error(epsilon, theta_incident, k);
        let expected = 2.0 * k * epsilon * theta_incident.cos();

        assert!((phase - expected).abs() < EPSILON);
    }

    #[test]
    fn test_phase_mesh_small_spacing() {
        let k = wavenumber(0.03); // 3 cm wavelength
        let mesh_spacing = 0.005; // 5 mm mesh
        let theta_incident = PI / 4.0;

        let phase = phase_mesh(mesh_spacing, theta_incident, k);

        // Should be non-zero but small
        assert!(phase.abs() > 0.0);
        assert!(phase.abs() < PI / 2.0);
    }

    #[test]
    fn test_phase_total_ideal_on_axis() {
        // Ideal antenna on-axis: only path phase
        let aperture = ApertureCoordinates::new(5.0, 0.0);
        let focal_length = 17.0;
        let k = wavenumber(0.03);

        let phase = phase_total(
            aperture,
            0.0, // On-axis
            0.0,
            focal_length,
            0.0, // No feed displacement
            0.0,
            0.0, // No axial offset
            0.0, // No surface error
            0.0,
            0.0, // No mesh
            k,
        );

        // Should equal path phase only
        let expected = phase_path(5.0, 0.0, 0.0, 0.0, focal_length, k);
        assert!((phase - expected).abs() < EPSILON);
    }

    #[test]
    fn test_phase_total_with_all_contributions() {
        let aperture = ApertureCoordinates::new(5.0, PI / 4.0);
        let focal_length = 17.0;
        let k = wavenumber(0.03);
        let theta = 0.1;
        let phi = PI / 3.0;

        let phase = phase_total(
            aperture,
            theta,
            phi,
            focal_length,
            1.0,      // Feed displacement
            0.0,      // Displacement angle
            0.0,      // No axial offset
            0.001,    // Surface error
            PI / 6.0, // Incident angle
            0.005,    // Mesh spacing
            k,
        );

        // Should be sum of all components
        let p_path = phase_path(5.0, PI / 4.0, theta, phi, focal_length, k);
        let p_feed = phase_feed_displacement(5.0, PI / 4.0, 1.0, 0.0, 0.0, focal_length, k);
        let p_surf = phase_surface_error(0.001, PI / 6.0, k);
        let p_mesh = phase_mesh(0.005, PI / 6.0, k);
        let expected = p_path + p_feed + p_surf + p_mesh;

        assert!((phase - expected).abs() < EPSILON);
    }

    #[test]
    fn test_ideal_surface() {
        let surface = IdealSurface;
        assert_eq!(surface.error_at(5.0, 0.0), 0.0);
        assert_eq!(surface.rms(), 0.0);
    }

    #[test]
    fn test_gaussian_surface() {
        let surface = GaussianSurface::new(0.001, 0.1, 12345);

        // Error should be within reasonable bounds
        let error = surface.error_at(5.0, PI / 4.0);
        assert!(error.abs() <= 3.0 * surface.rms()); // Within 3 sigma

        // Same position should give same error (deterministic)
        let error2 = surface.error_at(5.0, PI / 4.0);
        assert_eq!(error, error2);

        // RMS should match specification
        assert_eq!(surface.rms(), 0.001);
    }

    #[test]
    fn test_zernike_surface_piston() {
        // Pure piston (constant offset)
        let coeffs = vec![0.001, 0.0, 0.0]; // 1 mm piston
        let surface = ZernikeSurface::new(coeffs, 17.0);

        // All points should have same error
        let error1 = surface.error_at(5.0, 0.0);
        let error2 = surface.error_at(10.0, PI / 2.0);
        assert!((error1 - 0.001).abs() < 1e-6);
        assert!((error2 - 0.001).abs() < 1e-6);
    }

    #[test]
    fn test_zernike_surface_tilt() {
        // Pure tilt in x-direction
        let coeffs = vec![0.0, 0.001, 0.0]; // 1 mm tilt coefficient
        let surface = ZernikeSurface::new(coeffs, 17.0);

        // Error should vary linearly with ρ·cos(φ')
        let error1 = surface.error_at(5.0, 0.0); // φ'=0, max positive
        let error2 = surface.error_at(5.0, PI); // φ'=π, max negative

        // Should be opposite signs
        assert!(error1 > 0.0);
        assert!(error2 < 0.0);
        assert!((error1 + error2).abs() < 1e-10); // Should be symmetric
    }

    #[test]
    fn test_angle_of_incidence() {
        let focal_length = 17.0;

        // On-axis (ρ=0), angle should be zero
        let angle = angle_of_incidence(0.0, focal_length);
        assert!(angle.abs() < EPSILON);

        // At ρ = 2f, angle should be atan(1) = 45 degrees
        let angle = angle_of_incidence(2.0 * focal_length, focal_length);
        assert!((angle - PI / 4.0).abs() < 1e-6);
    }

    #[test]
    fn test_axial_displacement_produces_defocus() {
        // Pure axial offset (no lateral): δ_lateral=0, δz=0.10 m behind focus.
        // This must produce a ρ-dependent (defocus) phase — center and edge differ.
        let (f, k) = (17.0, wavenumber(0.03));
        let p_center = phase_feed_displacement(0.0, 0.0, 0.0, 0.0, 0.10, f, k);
        let p_edge = phase_feed_displacement(8.0, 0.0, 0.0, 0.0, 0.10, f, k);
        assert!(
            (p_edge - p_center).abs() > 1.0,
            "no defocus: center={p_center} edge={p_edge}"
        );
    }

    #[test]
    fn test_zero_axial_matches_previous_model() {
        // δz = 0 must reproduce the old 6-arg model exactly.
        let (f, k) = (17.0, wavenumber(0.03));
        let with_z0 = phase_feed_displacement(5.0, std::f64::consts::PI / 4.0, 1.0, 0.0, 0.0, f, k);
        let (x, y) = (
            5.0 * (std::f64::consts::PI / 4.0).cos(),
            5.0 * (std::f64::consts::PI / 4.0).sin(),
        );
        let z = 25.0 / (4.0 * f);
        let dz = z - f;
        let ideal = (x * x + y * y + dz * dz).sqrt();
        let displaced = ((x - 1.0).powi(2) + y * y + dz * dz).sqrt();
        let expected = k * (displaced - ideal);
        assert!(
            (with_z0 - expected).abs() < 1e-10,
            "δz=0 regression: {with_z0} vs {expected}"
        );
    }
}
