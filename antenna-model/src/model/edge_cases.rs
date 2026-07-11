//! Edge case detection and handling for antenna pattern computation
//!
//! This module identifies edge cases where standard physical optics approximations
//! may be inaccurate and selects appropriate computational methods:
//!
//! - **Large feed offsets** (> 0.3f): Switch to ray tracing
//! - **Pattern nulls**: Apply numerical stability measures
//!
//! # References
//! - Design doc Section 3.1 (Edge Cases)
//! - Hopkins, H.H. "Wave Theory of Aberrations"

use tracing::info;

use crate::model::geometry::AntennaConfiguration;
use std::f64::consts::PI;

/// Threshold for large feed offset detection (fraction of focal length)
pub const LARGE_OFFSET_THRESHOLD: f64 = 0.3;

/// Threshold for severe feed offset requiring ray tracing (fraction of focal length)
pub const SEVERE_OFFSET_THRESHOLD: f64 = 0.5;

/// Minimum gain floor to prevent numerical instabilities (in linear units)
/// Corresponds to -60 dB
pub const MIN_GAIN_FLOOR: f64 = 1e-6;

/// Minimum gain floor in dB
pub const MIN_GAIN_FLOOR_DB: f64 = -60.0;

/// Classification of antenna configuration computational requirements
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputationMode {
    /// Standard physical optics (small feed offsets)
    StandardPhysicalOptics,

    /// Physical optics with higher-order aberration terms (moderate feed offsets)
    HigherOrderAberrations,

    /// Ray tracing required (large feed offsets > 0.5f)
    RayTracing,
}

/// Edge case detection result
#[derive(Debug, Clone)]
pub struct EdgeCaseAnalysis {
    /// Recommended computation mode
    pub mode: ComputationMode,

    /// Feed offset magnitude (meters)
    pub feed_offset_magnitude: f64,

    /// Feed offset as fraction of focal length
    pub feed_offset_ratio: f64,

    /// Expected spillover fraction (0.0 to 1.0)
    pub spillover_fraction: f64,

    /// Warnings for the user
    pub warnings: Vec<String>,
}

/// Detect edge cases and recommend computation mode
///
/// # Arguments
/// * `config` - Antenna configuration
/// * `theta` - Far-field elevation angle (radians)
/// * `phi` - Far-field azimuth angle (radians)
///
/// # Returns
/// Analysis of edge cases and recommended computation approach
pub fn analyze_edge_cases(
    config: &AntennaConfiguration,
    _theta: f64,
    _phi: f64,
) -> EdgeCaseAnalysis {
    let _focal_length = config.reflector.focal_length;

    // Calculate feed offset magnitude
    let (offset_mag, offset_ratio) = calculate_feed_offset(config);

    // Debug output for feed offset analysis
    info!(
        "Edge case analysis: offset_mag={:.4}m, offset_ratio={:.4}, focal_length={:.4}m",
        offset_mag, offset_ratio, config.reflector.focal_length
    );

    // Estimate spillover
    let spillover = estimate_spillover(config);

    // Determine computation mode
    let mode = select_computation_mode(offset_ratio);

    // Generate warnings
    let mut warnings = Vec::new();

    if offset_ratio > SEVERE_OFFSET_THRESHOLD {
        warnings.push(format!(
            "Feed offset ({:.2}f = {:.3} m) exceeds severe threshold ({:.1}f). Ray tracing recommended.",
            offset_ratio, offset_mag, SEVERE_OFFSET_THRESHOLD
        ));
    } else if offset_ratio > LARGE_OFFSET_THRESHOLD {
        warnings.push(format!(
            "Feed offset ({:.2}f = {:.3} m) exceeds standard threshold ({:.1}f). Higher-order aberrations included.",
            offset_ratio, offset_mag, LARGE_OFFSET_THRESHOLD
        ));
    }

    if spillover > 0.1 {
        warnings.push(format!(
            "Estimated spillover {:.1}% may reduce aperture efficiency.",
            spillover * 100.0
        ));
    }

    EdgeCaseAnalysis {
        mode,
        feed_offset_magnitude: offset_mag,
        feed_offset_ratio: offset_ratio,
        spillover_fraction: spillover,
        warnings,
    }
}

/// Calculate feed offset magnitude and ratio to focal length
fn calculate_feed_offset(config: &AntennaConfiguration) -> (f64, f64) {
    let focal_length = config.reflector.focal_length;

    // Feed displacement from focal point
    let offset_magnitude = config.feed.position.displacement_from_focus(focal_length);

    let offset_ratio = offset_magnitude / focal_length;
    (offset_magnitude, offset_ratio)
}

/// Select appropriate computation mode based on edge case analysis
fn select_computation_mode(offset_ratio: f64) -> ComputationMode {
    // Priority: ray tracing > higher-order > standard
    if offset_ratio > SEVERE_OFFSET_THRESHOLD {
        info!(
            "Severe offset threshold detected ({:.3} > {:.1}), switching to ray-tracing",
            offset_ratio, SEVERE_OFFSET_THRESHOLD
        );
        ComputationMode::RayTracing
    } else if offset_ratio > LARGE_OFFSET_THRESHOLD {
        info!(
            "Large offset threshold detected ({:.3} > {:.1}), computing higher-order aberrations",
            offset_ratio, LARGE_OFFSET_THRESHOLD
        );
        ComputationMode::HigherOrderAberrations
    } else {
        info!(
            "Using standard physical optics (offset_ratio={:.3} <= threshold={:.1})",
            offset_ratio, LARGE_OFFSET_THRESHOLD
        );
        ComputationMode::StandardPhysicalOptics
    }
}

/// Estimate spillover fraction for given antenna configuration
///
/// Spillover is the fraction of feed power that misses the reflector.
/// For a parabolic reflector, spillover depends on:
/// - Feed pattern (q-factor)
/// - f/D ratio
/// - Feed offset
///
/// # Returns
/// Fraction of power spilled over (0.0 to 1.0)
fn estimate_spillover(config: &AntennaConfiguration) -> f64 {
    let f_over_d = config.reflector.f_over_d();
    let q = config.feed.q_factor;

    // Edge angle subtended by reflector at focus (radians)
    // For parabola: tan(ψ_edge/2) = 1/(4·f/D)
    let edge_angle: f64 = 2.0 * (1.0 / (4.0 * f_over_d)).atan();

    // Feed offset increases spillover
    let (_, offset_ratio) = calculate_feed_offset(config);
    let offset_factor = 1.0 + 2.0 * offset_ratio; // Empirical approximation

    // Effective edge angle accounting for offset
    let effective_edge_angle = edge_angle * offset_factor;

    // Power beyond edge angle for cos^q pattern
    // Captured power ≈ ∫[0 to ψ_edge] cos^q(ψ) sin(ψ) dψ / ∫[0 to π/2] cos^q(ψ) sin(ψ) dψ
    // Spillover = 1 - captured power

    if effective_edge_angle >= PI / 2.0 {
        // Feed pattern extends beyond reflector edge - high spillover
        return 1.0;
    }

    // Analytical integral for cos^q pattern
    // ∫ cos^q(ψ) sin(ψ) dψ = -cos^(q+1)(ψ) / (q+1)
    // Evaluated from 0 to ψ_edge: [1 - cos^(q+1)(ψ_edge)]

    let cos_edge = effective_edge_angle.cos();
    // Use powf, not powi(q as i32): q is fractional in the live uncalibrated path
    // (e.g. 1.14, 3.15 after the 2026-07-10 feed-taper fix). Truncating q to an integer
    // exponent mis-estimates the captured/spilled fractions by ~0.1 dB.
    let captured_fraction = 1.0 - cos_edge.powf(q + 1.0);
    let spillover = 1.0 - captured_fraction;

    // Clamp to [0, 1]
    spillover.clamp(0.0, 1.0)
}

/// Apply minimum gain floor to prevent numerical instabilities
///
/// This is particularly important near pattern nulls where the computed
/// gain may become extremely small or numerically unstable.
///
/// # Arguments
/// * `gain_linear` - Computed gain in linear units
///
/// # Returns
/// Gain with floor applied
#[inline]
pub fn apply_gain_floor(gain_linear: f64) -> f64 {
    gain_linear.max(MIN_GAIN_FLOOR)
}

/// Apply minimum gain floor in dB
///
/// # Arguments
/// * `gain_db` - Computed gain in dB
///
/// # Returns
/// Gain with floor applied
#[inline]
pub fn apply_gain_floor_db(gain_db: f64) -> f64 {
    gain_db.max(MIN_GAIN_FLOOR_DB)
}

/// Higher-order Seidel aberration coefficients for large feed offsets
///
/// For offsets > 0.3f, include additional aberration terms beyond coma:
/// - Astigmatism
/// - Field curvature
/// - Distortion
///
/// # Arguments
/// * `rho` - Aperture radius coordinate (meters)
/// * `phi_prime` - Aperture azimuth coordinate (radians)
/// * `delta_feed` - Feed displacement magnitude (meters)
/// * `alpha` - Feed displacement angle (radians)
/// * `focal_length` - Focal length (meters)
/// * `wavenumber` - Wave number k = 2π/λ (rad/m)
///
/// # Returns
/// Additional phase contribution from higher-order aberrations (radians)
pub fn higher_order_aberrations(
    rho: f64,
    phi_prime: f64,
    delta_feed: f64,
    alpha: f64,
    focal_length: f64,
    wavenumber: f64,
) -> f64 {
    // Normalized aperture coordinate
    let rho_norm = rho / (2.0 * focal_length);
    let delta_norm = delta_feed / focal_length;

    // Astigmatism (proportional to δ²·ρ²)
    let astigmatism =
        wavenumber * delta_norm.powi(2) * rho_norm.powi(2) * (2.0 * phi_prime - 2.0 * alpha).cos();

    // Field curvature (proportional to δ²·ρ²)
    let field_curvature = wavenumber * delta_norm.powi(2) * rho_norm.powi(2) / 2.0;

    // Distortion (proportional to δ³·ρ³)
    let distortion = wavenumber * delta_norm.powi(3) * rho_norm.powi(3) * (phi_prime - alpha).cos();

    astigmatism + field_curvature + distortion
}

/// Check if adaptive integration is needed near pattern null
///
/// Pattern nulls occur when the aperture phase distribution causes
/// destructive interference. Near nulls, standard integration may
/// be inaccurate due to rapid phase variations.
///
/// # Arguments
/// * `theta` - Far-field elevation angle (radians)
/// * `phi` - Far-field azimuth angle (radians)
/// * `config` - Antenna configuration
///
/// # Returns
/// True if adaptive integration is recommended
pub fn needs_adaptive_integration(theta: f64, _phi: f64, config: &AntennaConfiguration) -> bool {
    // Estimate angular position of first null
    // For uniformly illuminated aperture: θ_null ≈ 1.22·λ/D
    // For tapered illumination, multiplier depends on q-factor

    let _diameter = config.reflector.diameter;

    // Estimate beamwidth from q-factor (higher q = narrower beam)
    let q = config.feed.q_factor;
    let beamwidth_factor = 1.0 + 0.1 * q; // Empirical approximation

    // First null is approximately at 1.2-1.5 times HPBW
    // HPBW ≈ λ/D for large apertures
    let _null_angle_estimate = beamwidth_factor * 1.5; // Radians per (λ/D)

    // Near null if within ±20% of estimated null position
    // (This is a heuristic; exact null positions require full pattern computation)

    // Check if we're near a null region (requires knowing wavelength)
    // For now, use conservative approach: angles > 0.1 rad may have nulls
    theta > 0.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::{FeedParameters, FeedPosition, ReflectorGeometry};

    fn test_antenna_on_axis() -> AntennaConfiguration {
        let focal_length = 0.5;
        AntennaConfiguration {
            id: "test_antenna".to_string(),
            name: "Test Antenna On-Axis".to_string(),
            reflector: ReflectorGeometry {
                diameter: 1.0,
                focal_length,
                surface_rms: 0.0005, // 0.5 mm
            },
            feed: FeedParameters {
                position: FeedPosition::at_focus(focal_length),
                q_factor: 8.0,
                phase_center_offset: 0.0,
                asymmetry_factor: 1.0,
            },
            mesh: None,
        }
    }

    fn test_antenna_offset(e_cone_deg: f64) -> AntennaConfiguration {
        use crate::model::coordinates::EClockConeCoordinates;
        let focal_length = 0.5; // f/D = 0.5, D = 1.0 -> f = 0.5
        let ecc = EClockConeCoordinates::from_degrees(e_cone_deg, 0.0);
        let (x, y, z) = ecc.to_feed_position(focal_length);

        AntennaConfiguration {
            id: "test_antenna".to_string(),
            name: format!("Test Antenna {} deg", e_cone_deg),
            reflector: ReflectorGeometry {
                diameter: 1.0,
                focal_length,
                surface_rms: 0.0005,
            },
            feed: FeedParameters {
                position: FeedPosition::new(x, y, z),
                q_factor: 8.0,
                phase_center_offset: 0.0,
                asymmetry_factor: 1.0,
            },
            mesh: None,
        }
    }

    #[test]
    fn test_on_axis_standard_mode() {
        let config = test_antenna_on_axis();
        let analysis = analyze_edge_cases(&config, 0.0, 0.0);

        assert_eq!(analysis.mode, ComputationMode::StandardPhysicalOptics);
        assert_eq!(analysis.feed_offset_ratio, 0.0);
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn test_small_offset_standard_mode() {
        // E-cone = 1 degree -> offset/f ≈ 0.017 << 0.3
        let config = test_antenna_offset(1.0);
        let analysis = analyze_edge_cases(&config, 0.0, 0.0);

        assert_eq!(analysis.mode, ComputationMode::StandardPhysicalOptics);
        assert!(analysis.feed_offset_ratio < LARGE_OFFSET_THRESHOLD);
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn test_moderate_offset_higher_order() {
        // E-cone = 20 degrees -> offset/f ≈ 0.35 > 0.3
        let config = test_antenna_offset(20.0);
        let analysis = analyze_edge_cases(&config, 0.05, 0.0); // 2.86 degrees

        assert_eq!(analysis.mode, ComputationMode::HigherOrderAberrations);
        assert!(analysis.feed_offset_ratio > LARGE_OFFSET_THRESHOLD);
        assert!(analysis.feed_offset_ratio < SEVERE_OFFSET_THRESHOLD);
        assert!(!analysis.warnings.is_empty());
    }

    #[test]
    fn test_large_offset_ray_tracing() {
        // E-cone = 35 degrees -> offset/f ≈ 0.6 > 0.5
        let config = test_antenna_offset(35.0);
        let analysis = analyze_edge_cases(&config, 0.05, 0.0);

        assert_eq!(analysis.mode, ComputationMode::RayTracing);
        assert!(analysis.feed_offset_ratio > SEVERE_OFFSET_THRESHOLD);
        assert!(analysis.warnings.iter().any(|w| w.contains("Ray tracing")));
    }

    #[test]
    fn test_near_boresight_moderate_offset_uses_standard_po() {
        // θ < 1° with a 0.1f-0.3f offset previously selected the (removed)
        // direct-path mode; it must now fall through to standard PO.
        // E-cone = 10 degrees -> offset/f ≈ 0.175, within the 0.1f-0.3f band.
        let config = test_antenna_offset(10.0);
        let theta = 0.01; // ~0.57 degrees, well under 1 degree
        let analysis = analyze_edge_cases(&config, theta, 0.0);

        assert_eq!(analysis.mode, ComputationMode::StandardPhysicalOptics);
    }

    #[test]
    fn test_spillover_estimation() {
        let config_on_axis = test_antenna_on_axis();
        let spillover_on_axis = estimate_spillover(&config_on_axis);

        // On-axis should have low spillover for f/D=0.5, q=8
        assert!(
            spillover_on_axis < 0.05,
            "On-axis spillover: {}",
            spillover_on_axis
        );

        let config_offset = test_antenna_offset(20.0);
        let spillover_offset = estimate_spillover(&config_offset);

        // Offset feed should have higher spillover
        assert!(spillover_offset > spillover_on_axis);
    }

    /// Regression: `estimate_spillover` must honor a FRACTIONAL q (not truncate it to
    /// an integer exponent). For f/D=0.5 the rim half-angle is 53.13° (cos=0.6); with an
    /// on-axis feed at q=1.5 the analytic spillover is cos^(q+1) = 0.6^2.5 ≈ 0.279.
    /// The old `powi((q as i32)+1)` truncated the exponent to 2 → 0.6^2 = 0.36, which this
    /// test rejects.
    #[test]
    fn test_spillover_honors_fractional_q() {
        let mut config = test_antenna_on_axis(); // D=1.0, f/D=0.5, feed at focus (no offset)
        config.feed.q_factor = 1.5;

        let spillover = estimate_spillover(&config);
        let expected = 0.6_f64.powf(2.5); // ≈ 0.2788

        assert!(
            (spillover - expected).abs() < 5e-3,
            "fractional-q spillover should be ~{expected:.4} (0.6^2.5), got {spillover:.4}; \
             a value near 0.36 means q was truncated to an integer exponent"
        );
    }

    #[test]
    fn test_gain_floor_linear() {
        assert_eq!(apply_gain_floor(1e-3), 1e-3);
        assert_eq!(apply_gain_floor(1e-8), MIN_GAIN_FLOOR);
        assert_eq!(apply_gain_floor(-1.0), MIN_GAIN_FLOOR);
    }

    #[test]
    fn test_gain_floor_db() {
        assert_eq!(apply_gain_floor_db(-30.0), -30.0);
        assert_eq!(apply_gain_floor_db(-80.0), MIN_GAIN_FLOOR_DB);
        assert_eq!(apply_gain_floor_db(-100.0), MIN_GAIN_FLOOR_DB);
    }

    #[test]
    fn test_higher_order_aberrations_zero_offset() {
        // Zero offset should give zero aberrations
        let phase = higher_order_aberrations(0.1, 0.0, 0.0, 0.0, 0.5, 100.0);
        assert_eq!(phase, 0.0);
    }

    #[test]
    fn test_higher_order_aberrations_nonzero() {
        // Nonzero offset should give nonzero aberrations
        let phase = higher_order_aberrations(0.1, 0.0, 0.1, 0.0, 0.5, 100.0);
        assert!(phase.abs() > 0.0);
    }

    #[test]
    fn test_adaptive_integration_needed() {
        let config = test_antenna_on_axis();

        // Near boresight: standard integration OK
        assert!(!needs_adaptive_integration(0.01, 0.0, &config));

        // Far from boresight: may need adaptive integration
        assert!(needs_adaptive_integration(0.2, 0.0, &config));
    }

    #[test]
    fn test_offset_calculation_on_axis() {
        let config = test_antenna_on_axis();
        let (offset_mag, offset_ratio) = calculate_feed_offset(&config);
        assert_eq!(offset_mag, 0.0);
        assert_eq!(offset_ratio, 0.0);
    }

    #[test]
    fn test_offset_calculation_e_cone() {
        let config = test_antenna_offset(10.0); // 10 degrees
        let (offset_mag, offset_ratio) = calculate_feed_offset(&config);

        // Approximate check: displacement = 2f·tan(cone/2)
        // f = 0.5, cone = 10° = 0.1745 rad
        // displacement ≈ 2·0.5·tan(0.0873) ≈ 0.0875
        assert!((offset_mag - 0.0875).abs() < 0.01);
        assert!((offset_ratio - 0.175).abs() < 0.02);
    }

    #[test]
    fn test_mode_selection_priority() {
        // Ray tracing for severe offsets
        let mode = select_computation_mode(0.6);
        assert_eq!(mode, ComputationMode::RayTracing);

        // Higher-order when moderate offset
        let mode = select_computation_mode(0.35);
        assert_eq!(mode, ComputationMode::HigherOrderAberrations);

        // Standard when small offset
        let mode = select_computation_mode(0.1);
        assert_eq!(mode, ComputationMode::StandardPhysicalOptics);
    }
}
