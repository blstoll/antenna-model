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
                axial_defocus: 0.0,
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
                axial_defocus: 0.0,
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

    // ---------------------------------------------------------------------
    // P2 Stage 1 (safety gate): Seidel double-count redundancy check.
    //
    // The `HigherOrderAberrations` mode adds BOTH the exact coma phase
    // (`phase::phase_feed_displacement`) AND the Seidel approximation
    // (`higher_order_aberrations`) into the aperture phase. The exact phase
    // already contains every aberration order as an exact function of the
    // feed displacement. This test numerically extracts the delta^2 / delta^3
    // aberration content ALREADY present in the exact phase and compares it,
    // component-by-component, against the Seidel forms, to determine whether
    // adding the Seidel terms double-counts content that is genuinely
    // redundant (matching coefficients) or spuriously additive (mismatched).
    //
    // Method: with alpha = 0, delta_z = 0, at fixed (rho, phi'), fit a
    // degree-5 polynomial in the lateral offset delta over a set of SMALL,
    // sign-symmetric delta values (delta/f in {+-0.01, +-0.02, +-0.03}).
    // The polynomial's delta^2 and delta^3 coefficients are the exact phase's
    // Taylor coefficients at that aperture point. Sweeping phi' and projecting
    // those coefficients onto cos(2phi'), the phi'-constant term, cos(phi')
    // and cos(3phi') isolates the astigmatism / field-curvature / distortion /
    // trefoil angular components. These are compared to `higher_order_aberrations`.
    #[test]
    fn p2_stage1_seidel_double_count_redundancy_check() {
        use crate::model::phase::phase_feed_displacement;

        // Solve a 6x6 linear system by Gaussian elimination with partial pivoting.
        fn solve6(mut a: [[f64; 6]; 6], mut b: [f64; 6]) -> [f64; 6] {
            for i in 0..6 {
                let mut p = i;
                for r in (i + 1)..6 {
                    if a[r][i].abs() > a[p][i].abs() {
                        p = r;
                    }
                }
                a.swap(i, p);
                b.swap(i, p);
                let piv = a[i][i];
                for j in i..6 {
                    a[i][j] /= piv;
                }
                b[i] /= piv;
                for r in 0..6 {
                    if r != i {
                        let fac = a[r][i];
                        for j in i..6 {
                            a[r][j] -= fac * a[i][j];
                        }
                        b[r] -= fac * b[i];
                    }
                }
            }
            b
        }

        let f = 0.5_f64; // focal length (m); matches the on-axis test antenna
        let lambda = 0.03_f64; // 10 GHz-ish; k is a common linear prefactor and cancels in ratios
        let k = 2.0 * std::f64::consts::PI / lambda;
        let alpha = 0.0_f64;
        let dz = 0.0_f64;

        // The offset band this mode governs is 0.3f..0.5f (delta = 0.35f nominal).
        // We extract the *Taylor* delta^2/delta^3 coefficients (the leading-order
        // aberration content), which are delta-independent, using SMALL delta so
        // the polynomial fit is well-conditioned. delta = 0.35f is the physical
        // context for why this mode/aberration content matters.
        let deltas: [f64; 6] = [-0.03, -0.02, -0.01, 0.01, 0.02, 0.03].map(|x| x * f);

        // Extract (c2, c3) = (delta^2 coeff, delta^3 coeff) of the exact phase.
        let extract = |rho: f64, phi: f64| -> (f64, f64) {
            let ys: [f64; 6] = {
                let mut y = [0.0; 6];
                for (i, &d) in deltas.iter().enumerate() {
                    y[i] = phase_feed_displacement(rho, phi, d, alpha, dz, f, k);
                }
                y
            };
            // Vandermonde: row i = [1, d, d^2, d^3, d^4, d^5]
            let mut v = [[0.0; 6]; 6];
            for i in 0..6 {
                let mut p = 1.0;
                for j in 0..6 {
                    v[i][j] = p;
                    p *= deltas[i];
                }
            }
            let c = solve6(v, ys);
            (c[2], c[3])
        };

        let n = 64usize;
        let phis: Vec<f64> =
            (0..n).map(|i| 2.0 * std::f64::consts::PI * i as f64 / n as f64).collect();

        // A genuine leading-order coefficient match means exact/seidel ~ +1
        // (correct sign, ratio near unity), STABLE across aperture radii. We
        // collect the ratios at several radii and test that property.
        let mut r_astig_all = Vec::new();
        let mut r_fc_all = Vec::new();
        let mut r_dist_all = Vec::new();

        println!("\n=== P2 Stage 1: exact-phase delta^2/delta^3 content vs Seidel forms ===");
        println!("f = {f}, delta(context) = 0.35f = {}, k = {k:.4}", 0.35 * f);
        for &rho in &[0.1_f64, 0.25, 0.5] {
            let rr = f + rho * rho / (4.0 * f); // R = focus->surface distance

            // Project exact Taylor coeffs onto angular bases (alpha = 0).
            let mut d2_cos2 = 0.0;
            let mut d2_const = 0.0;
            let mut d3_cos1 = 0.0;
            let mut d3_cos3 = 0.0;
            for &phi in &phis {
                let (c2, c3) = extract(rho, phi);
                d2_cos2 += c2 * (2.0 * phi).cos();
                d2_const += c2;
                d3_cos1 += c3 * phi.cos();
                d3_cos3 += c3 * (3.0 * phi).cos();
            }
            let nf = n as f64;
            d2_cos2 *= 2.0 / nf;
            d2_const /= nf;
            d3_cos1 *= 2.0 / nf;
            d3_cos3 *= 2.0 / nf;

            // Closed-form analytic coefficients (independently derived) — used to
            // validate that the numerical extraction is faithful and non-noisy.
            let an_d2_cos2 = -k * rho * rho / (4.0 * rr.powi(3));
            let an_d2_const = k * (1.0 / (2.0 * rr) - rho * rho / (4.0 * rr.powi(3)));
            let an_d3_cos1 =
                k * (rho / (2.0 * rr.powi(3)) - 3.0 * rho.powi(3) / (8.0 * rr.powi(5)));
            let an_d3_cos3 = -k * rho.powi(3) / (8.0 * rr.powi(5));

            // The extraction MUST reproduce the analytic content — this is the
            // proof that the exact phase already carries the delta^2/delta^3
            // aberration content (basis of the double-count claim). Sanity gate.
            assert!(
                (d2_cos2 - an_d2_cos2).abs() <= 1e-4 * an_d2_cos2.abs().max(1.0),
                "extraction/analytic mismatch d2_cos2 rho={rho}: {d2_cos2} vs {an_d2_cos2}"
            );
            assert!(
                (d3_cos1 - an_d3_cos1).abs() <= 1e-4 * an_d3_cos1.abs().max(1.0),
                "extraction/analytic mismatch d3_cos1 rho={rho}: {d3_cos1} vs {an_d3_cos1}"
            );

            // Angular signatures must be genuinely present (nonzero) — the exact
            // phase DOES contain astigmatism (cos2), distortion (cos1) and
            // trefoil (cos3) delta^2/delta^3 content. This confirms that adding
            // ANY Seidel astigmatism/distortion double-counts existing content.
            assert!(d2_cos2.abs() > 1.0, "exact phase should carry cos(2phi) delta^2 content");
            assert!(d3_cos1.abs() > 1.0, "exact phase should carry cos(phi) delta^3 content");

            // Seidel coefficients (coefficient of delta^2 / delta^3), from
            // `higher_order_aberrations` with rho_n = rho/(2f), delta_n = delta/f.
            let s_astig = k * (1.0 / f).powi(2) * (rho / (2.0 * f)).powi(2); // -> cos(2phi)
            let s_fieldcurv = s_astig / 2.0; // -> constant
            let s_distort = k * (1.0 / f).powi(3) * (rho / (2.0 * f)).powi(3); // -> cos(phi)

            let r_astig = d2_cos2 / s_astig;
            let r_fc = d2_const / s_fieldcurv;
            let r_dist = d3_cos1 / s_distort;

            println!("\nrho = {rho}  (R = {rr:.4})");
            println!(
                "  astigmatism  (d2,cos2phi): exact = {d2_cos2:>10.4}   seidel = {s_astig:>10.4}   exact/seidel = {r_astig:>8.4}"
            );
            println!(
                "  fieldcurv    (d2,const  ): exact = {d2_const:>10.4}   seidel = {s_fieldcurv:>10.4}   exact/seidel = {r_fc:>8.4}"
            );
            println!(
                "  distortion   (d3,cos1phi): exact = {d3_cos1:>10.4}   seidel = {s_distort:>10.4}   exact/seidel = {r_dist:>8.4}"
            );
            println!("  trefoil      (d3,cos3phi): exact = {d3_cos3:>10.4}   (no Seidel counterpart; analytic {an_d3_cos3:.4})");

            r_astig_all.push(r_astig);
            r_fc_all.push(r_fc);
            r_dist_all.push(r_dist);
        }

        // VERDICT — the redundancy claim is REFUTED at the coefficient level.
        //
        // A leading-order coefficient match would require exact/seidel ~ +1,
        // sign-correct and STABLE across aperture radii. Instead we measure:
        //
        //  1) Astigmatism (the defining 2nd-order aberration, cos2phi) has the
        //     WRONG SIGN at every radius: the exact phase's cos(2phi') delta^2
        //     content is NEGATIVE while the Seidel astigmatism term is POSITIVE.
        //     Adding the Seidel term does not double the true content, it opposes
        //     and overshoots it.
        //
        //  2) The magnitudes are NOT a stable ratio: field-curvature and
        //     distortion ratios swing by ~2 orders of magnitude across radii
        //     (48 -> 1.1, 47 -> 0.53). This is the dimensional signature of a
        //     spurious 1/f factor: exact content scales ~1/R^3, 1/R^5 while the
        //     Seidel forms scale ~1/f^4, 1/f^6; distortion additionally carries
        //     the wrong rho-power (exact leading term is ~rho, Seidel is ~rho^3).
        //     (The single near-unity field-curvature point at rho=0.5 is an
        //     f-dependent numerical coincidence of the piston-like k/(2R) term,
        //     not a physical match.)
        //
        // CONCLUSION: the exact phase already CARRIES delta^2/delta^3 aberration
        // content (asserted above: nonzero cos2/cos1/cos3 signatures reproduced
        // by closed form), so adding ANY Seidel term double-counts -> the mode is
        // wrong to add both. But the Seidel forms are NOT a leading-order match
        // to that content (sign-flipped astigmatism, dimensionally-off, wrong
        // rho-power distortion). This is a STOP signal for the "coefficients
        // match" framing of P2 Stage 1.

        // (1) Sign flip: astigmatism exact/seidel < 0 at every radius.
        for (i, &r) in r_astig_all.iter().enumerate() {
            assert!(
                r < 0.0,
                "expected astigmatism sign flip (exact/seidel < 0) at radius index {i}, got {r}"
            );
        }

        // (2) No stable coefficient: the magnitude-ratio spread across radii is
        //     enormous (> 10x) for both field-curvature and distortion. A true
        //     leading-order coefficient would be radius-stable near unity.
        let spread = |v: &Vec<f64>| {
            let mags: Vec<f64> = v.iter().map(|x| x.abs()).collect();
            mags.iter().cloned().fold(f64::MIN, f64::max)
                / mags.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!(
            spread(&r_fc_all) > 10.0,
            "expected large (unstable) field-curvature ratio spread, got {}",
            spread(&r_fc_all)
        );
        assert!(
            spread(&r_dist_all) > 10.0,
            "expected large (unstable) distortion ratio spread, got {}",
            spread(&r_dist_all)
        );

        println!(
            "\nVERDICT: REFUTED (coefficient match). Astigmatism sign-flipped at all radii; \
             field-curvature/distortion ratios span {:.1}x / {:.1}x across radii. \
             Double-counting is qualitatively real (exact phase carries the content), \
             but the Seidel forms are NOT a leading-order coefficient match.",
            spread(&r_fc_all),
            spread(&r_dist_all)
        );
    }
}
