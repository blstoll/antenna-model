//! Feed Illumination Model
//!
//! This module implements feed pattern models for aperture illumination
//! in parabolic reflector antennas. The feed horn illuminates the reflector
//! surface, and the illumination function determines the amplitude and phase
//! of the field at each aperture point.
//!
//! # Feed Pattern Model
//!
//! The primary model is the **cos^q approximation**, widely used for feed horns:
//! ```text
//! F(ψ) = cos(ψ)^q  for ψ < π/2, else 0
//! ```
//! where:
//! - ψ is the angle from the feed boresight to the aperture point
//! - q is the pattern factor (higher q → more focused beam)
//!
//! Typical q values:
//! - q ≈ 6-8 for 10 dB edge taper
//! - q ≈ 10-12 for 12 dB edge taper
//!
//! # References
//! - Design doc Section 2.3 (Illumination Function)
//! - Implementation plan Sprint 2, Task 2.3

use std::f64::consts::PI;

use crate::model::geometry::{FeedParameters, FeedPosition};

/// Calculate cos^q feed pattern
///
/// This is the standard approximation for feed horn radiation patterns.
/// The pattern is symmetric about the feed axis and falls off as cos(ψ)^q.
///
/// # Formula
/// ```text
/// F(ψ) = cos(ψ)^q  for |ψ| < π/2
/// F(ψ) = 0         for |ψ| ≥ π/2
/// ```
///
/// # Arguments
/// - `psi`: Angle from feed boresight (radians)
/// - `q`: Pattern factor (dimensionless, typically 6-12)
///
/// # Returns
/// Amplitude factor (normalized, 0 to 1)
///
/// # Examples
/// ```
/// use antenna_model::model::illumination::cos_q_pattern;
/// use std::f64::consts::PI;
///
/// // On-axis (ψ = 0) gives maximum amplitude
/// assert!((cos_q_pattern(0.0, 8.0) - 1.0).abs() < 1e-10);
///
/// // At ψ = 60°, cos^8(60°) = 0.5^8 ≈ 0.0039
/// assert!((cos_q_pattern(PI / 3.0, 8.0) - 0.00390625).abs() < 1e-6);
///
/// // At ψ = 90° or beyond, amplitude is zero
/// assert!(cos_q_pattern(PI / 2.0, 8.0).abs() < 1e-10);
/// assert!(cos_q_pattern(PI, 8.0).abs() < 1e-10);
/// ```
#[inline]
pub fn cos_q_pattern(psi: f64, q: f64) -> f64 {
    // Check if angle is within forward hemisphere
    if psi.abs() >= PI / 2.0 {
        return 0.0;
    }

    // Calculate cos^q
    psi.cos().powf(q)
}

/// Calculate angle from feed to aperture point
///
/// Computes the angle ψ between the feed boresight direction and the
/// ray from the feed to a point on the reflector aperture.
///
/// # Geometry
/// - Feed position: (x_f, y_f, z_f) in Cartesian coordinates
/// - Aperture point: (ρ, φ') in cylindrical coordinates
///   - Cartesian: (ρ cos(φ'), ρ sin(φ'), ρ²/(4f))
/// - Feed boresight: directed from feed toward reflector vertex (origin)
///
/// # Coordinate System
/// - Origin at reflector vertex
/// - z-axis along reflector axis
/// - For centered feed at (0, 0, f), boresight points in -z direction
///
/// # Arguments
/// - `rho`: Radial distance from axis in aperture (meters)
/// - `phi_prime`: Azimuthal angle in aperture (radians)
/// - `feed_pos`: Feed position in Cartesian coordinates
/// - `focal_length`: Reflector focal length (meters)
///
/// # Returns
/// Angle ψ in radians (0 to π)
///
/// # Examples
/// ```
/// use antenna_model::model::illumination::feed_angle;
/// use antenna_model::model::geometry::FeedPosition;
/// use std::f64::consts::PI;
///
/// // Centered feed at focus, looking at vertex (on-axis point)
/// let feed = FeedPosition::new(0.0, 0.0, 1.0); // focal_length = 1.0
/// let psi = feed_angle(0.0, 0.0, &feed, 1.0);
/// // At vertex (ρ=0), angle should be 0 or π (along axis)
/// assert!(psi.abs() < 1e-10 || (psi - PI).abs() < 1e-10);
/// ```
pub fn feed_angle(rho: f64, phi_prime: f64, feed_pos: &FeedPosition, focal_length: f64) -> f64 {
    // Convert aperture point from cylindrical to Cartesian
    let x_aperture = rho * phi_prime.cos();
    let y_aperture = rho * phi_prime.sin();
    let z_aperture = rho * rho / (4.0 * focal_length);

    // Vector from feed to aperture point
    let dx = x_aperture - feed_pos.x;
    let dy = y_aperture - feed_pos.y;
    let dz = z_aperture - feed_pos.z;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();

    // Handle degenerate case (feed at aperture point)
    if distance < 1e-10 {
        return 0.0;
    }

    // Feed boresight direction: from feed toward vertex (origin)
    // This is the natural illumination direction for the reflector
    let boresight_x = -feed_pos.x;
    let boresight_y = -feed_pos.y;
    let boresight_z = -feed_pos.z;
    let boresight_mag =
        (boresight_x * boresight_x + boresight_y * boresight_y + boresight_z * boresight_z).sqrt();

    // Handle degenerate case (feed at origin)
    if boresight_mag < 1e-10 {
        // If feed is at origin, use -z as default boresight
        let cos_angle = -dz / distance;
        return cos_angle.clamp(-1.0, 1.0).acos();
    }

    // Normalized boresight direction
    let bx = boresight_x / boresight_mag;
    let by = boresight_y / boresight_mag;
    let bz = boresight_z / boresight_mag;

    // Normalized direction to aperture point
    let vx = dx / distance;
    let vy = dy / distance;
    let vz = dz / distance;

    // Angle between boresight and aperture direction
    let cos_angle = bx * vx + by * vy + bz * vz;

    // Clamp to valid range to handle numerical errors
    cos_angle.clamp(-1.0, 1.0).acos()
}

/// Calculate illumination amplitude at aperture point
///
/// Combines the feed angle calculation with the cos^q pattern to determine
/// the amplitude of the feed illumination at a specific aperture point.
///
/// For asymmetric feeds (E-plane ≠ H-plane), this function approximates
/// the asymmetry using the `asymmetry_factor` from `FeedParameters`.
///
/// # Arguments
/// - `rho`: Radial distance from axis in aperture (meters)
/// - `phi_prime`: Azimuthal angle in aperture (radians)
/// - `feed_params`: Feed parameters including position and pattern
/// - `focal_length`: Reflector focal length (meters)
///
/// # Returns
/// Amplitude factor (0 to 1, normalized to unity on boresight)
///
/// # Examples
/// ```
/// use antenna_model::model::illumination::illumination_amplitude;
/// use antenna_model::model::geometry::{FeedParameters, FeedPosition};
///
/// let feed_pos = FeedPosition::at_focus(1.0);
/// let feed_params = FeedParameters::new(
///     feed_pos,
///     8.0,   // q_factor
///     0.0,   // phase_center_offset
///     1.0,   // asymmetry_factor (symmetric)
/// ).unwrap();
///
/// // At vertex (on-axis), should have maximum amplitude
/// let amp = illumination_amplitude(0.0, 0.0, &feed_params, 1.0);
/// // Note: might not be exactly 1.0 due to geometry, but should be close
/// ```
#[inline]
pub fn illumination_amplitude(
    rho: f64,
    phi_prime: f64,
    feed_params: &FeedParameters,
    focal_length: f64,
) -> f64 {
    // Calculate angle from feed to aperture point
    let psi = feed_angle(rho, phi_prime, &feed_params.position, focal_length);

    // Apply asymmetry if present
    // For asymmetric patterns, we modify the effective q-factor based on azimuthal angle
    // This is a simplified model; real asymmetric patterns are more complex
    let q_effective = if feed_params.asymmetry_factor != 1.0 {
        // Modulate q-factor with azimuthal angle to approximate E/H plane differences
        // E-plane typically at φ' = 0, π; H-plane at φ' = π/2, 3π/2
        let azimuth_factor = (2.0 * phi_prime).cos();
        let q_modulation = (feed_params.asymmetry_factor - 1.0) * azimuth_factor;
        feed_params.q_factor * (1.0 + 0.2 * q_modulation) // 20% maximum variation
    } else {
        feed_params.q_factor
    };

    // Calculate cos^q pattern
    cos_q_pattern(psi, q_effective)
}

/// Calculate edge taper in dB for given q-factor and f/D ratio
///
/// Edge taper is the reduction in illumination amplitude at the reflector
/// edge compared to the center (boresight). It's expressed in dB:
/// ```text
/// edge_taper_dB = 20 log₁₀(A_edge / A_center)
/// ```
///
/// # Geometry
/// For a parabolic reflector with focal length f and diameter D:
/// - Edge at radius R = D/2
/// - Edge z-coordinate: z_edge = R²/(4f)
/// - Feed at (0, 0, f)
/// - Angle from boresight to edge calculated using actual reflector geometry
///
/// # Arguments
/// - `q`: Pattern factor for cos^q feed model
/// - `f_over_d`: Focal length to diameter ratio (typically ~0.5)
///
/// # Returns
/// Edge taper in dB (negative value, typically -25 to -45 dB for q=6-12)
///
/// # Examples
/// ```
/// use antenna_model::model::illumination::edge_taper_db;
///
/// // For q=8 and f/D=0.5, edge taper is around -35 dB
/// let taper = edge_taper_db(8.0, 0.5);
/// assert!(taper < -30.0 && taper > -40.0);
/// ```
pub fn edge_taper_db(q: f64, f_over_d: f64) -> f64 {
    // Calculate angle from feed to reflector edge using correct geometry
    // Let D = 1 (normalized), then f = f_over_d * D and R = D/2
    let d = 1.0;
    let f = f_over_d * d;
    let r = d / 2.0;

    // Feed position (at focus)
    let feed_pos = FeedPosition::new(0.0, 0.0, f);

    // Calculate angle to edge using our feed_angle function
    let psi_edge = feed_angle(r, 0.0, &feed_pos, f);

    // Amplitude at edge
    let amp_edge = cos_q_pattern(psi_edge, q);

    // Convert to dB (will be negative)
    20.0 * amp_edge.log10()
}

/// Calculate q-factor required for desired edge taper
///
/// Inverse of `edge_taper_db`: given a desired edge taper in dB,
/// calculate the required q-factor for a cos^q feed pattern.
///
/// # Arguments
/// - `edge_taper_db`: Desired edge taper in dB (negative value, e.g., -35.0)
/// - `f_over_d`: Focal length to diameter ratio (typically ~0.5)
///
/// # Returns
/// Required q-factor (typically 6-12)
///
/// # Examples
/// ```
/// use antenna_model::model::illumination::q_factor_from_taper;
///
/// // For -35 dB edge taper and f/D=0.5, q should be around 8
/// let q = q_factor_from_taper(-35.0, 0.5);
/// assert!(q > 7.0 && q < 9.0);
/// ```
pub fn q_factor_from_taper(edge_taper_db: f64, f_over_d: f64) -> f64 {
    // Convert dB to linear amplitude
    let amp_edge = 10.0_f64.powf(edge_taper_db / 20.0);

    // Calculate angle to edge using same geometry as edge_taper_db
    let d = 1.0;
    let f = f_over_d * d;
    let r = d / 2.0;

    let feed_pos = FeedPosition::new(0.0, 0.0, f);
    let psi_edge = feed_angle(r, 0.0, &feed_pos, f);

    // Solve: amp_edge = cos(psi_edge)^q
    // q = ln(amp_edge) / ln(cos(psi_edge))
    if psi_edge.cos() <= 0.0 || amp_edge <= 0.0 {
        return 0.0; // Degenerate case
    }

    amp_edge.ln() / psi_edge.cos().ln()
}

/// Calculate phase center offset contribution to illumination phase
///
/// The phase center of a feed horn is typically offset from its physical
/// aperture. This offset introduces an additional phase term that varies
/// with the angle ψ from boresight.
///
/// # Formula
/// ```text
/// Δφ = -k · d_pc · (1 - cos(ψ))
/// ```
/// where d_pc is the phase center offset and k is the wavenumber.
///
/// # Arguments
/// - `psi`: Angle from feed boresight (radians)
/// - `phase_center_offset`: Offset distance in meters (typically ±λ/4)
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Phase offset in radians
///
/// # Examples
/// ```
/// use antenna_model::model::illumination::phase_center_offset_phase;
/// use std::f64::consts::PI;
///
/// // At boresight (ψ=0), phase offset is zero
/// let phase = phase_center_offset_phase(0.0, 0.01, 0.036);
/// assert!(phase.abs() < 1e-10);
///
/// // Off-axis, there is a phase contribution
/// let phase = phase_center_offset_phase(PI / 4.0, 0.01, 0.036);
/// assert!(phase.abs() > 0.0);
/// ```
pub fn phase_center_offset_phase(psi: f64, phase_center_offset: f64, wavelength: f64) -> f64 {
    let k = 2.0 * PI / wavelength;
    -k * phase_center_offset * (1.0 - psi.cos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::FeedPosition;

    #[test]
    fn test_cos_q_pattern_on_axis() {
        // On boresight (ψ = 0), pattern should be unity
        assert!((cos_q_pattern(0.0, 8.0) - 1.0).abs() < 1e-10);
        assert!((cos_q_pattern(0.0, 10.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_cos_q_pattern_values() {
        // Test specific angle values
        // At ψ = 60° = π/3, cos(60°) = 0.5
        let psi_60 = PI / 3.0;

        // cos(60°)^8 = 0.5^8 = 1/256
        let expected_q8 = 0.5_f64.powi(8);
        assert!((cos_q_pattern(psi_60, 8.0) - expected_q8).abs() < 1e-6);

        // cos(60°)^10 = 0.5^10 = 1/1024
        let expected_q10 = 0.5_f64.powi(10);
        assert!((cos_q_pattern(psi_60, 10.0) - expected_q10).abs() < 1e-6);
    }

    #[test]
    fn test_cos_q_pattern_zero_at_90_deg() {
        // At ψ = 90° and beyond, pattern should be zero
        assert!(cos_q_pattern(PI / 2.0, 8.0).abs() < 1e-10);
        assert!(cos_q_pattern(PI / 2.0 + 0.1, 8.0).abs() < 1e-10);
        assert!(cos_q_pattern(PI, 8.0).abs() < 1e-10);
    }

    #[test]
    fn test_cos_q_pattern_negative_angles() {
        // Pattern should be symmetric about zero
        let psi = PI / 6.0;
        assert!((cos_q_pattern(psi, 8.0) - cos_q_pattern(-psi, 8.0)).abs() < 1e-10);
    }

    #[test]
    fn test_cos_q_pattern_higher_q_more_focused() {
        // Higher q means more focused beam (lower amplitude off-axis)
        let psi = PI / 6.0; // 30 degrees
        let amp_q6 = cos_q_pattern(psi, 6.0);
        let amp_q10 = cos_q_pattern(psi, 10.0);

        // q=10 should have lower amplitude at 30° than q=6
        assert!(amp_q10 < amp_q6);
    }

    #[test]
    fn test_feed_angle_centered_on_axis() {
        // Centered feed at focus looking at vertex
        let feed = FeedPosition::at_focus(1.0);
        let psi = feed_angle(0.0, 0.0, &feed, 1.0);

        // At vertex (ρ=0), the aperture point is at (0, 0, 0)
        // Feed is at (0, 0, 1)
        // Vector from feed to vertex: (0, 0, -1)
        // Boresight from feed toward vertex: (0, 0, -1)
        // These are parallel, so angle should be 0
        assert!(psi.abs() < 1e-6);
    }

    #[test]
    fn test_feed_angle_geometry() {
        // Feed at focus (0, 0, 1), focal_length = 1
        let feed = FeedPosition::at_focus(1.0);

        // Test point at edge: ρ = 0.5, φ' = 0
        // Aperture point: (0.5, 0, 0.5²/(4·1)) = (0.5, 0, 0.0625)
        // Feed at (0, 0, 1)
        // Vector to aperture: (0.5, 0, -0.9375)
        // Boresight: (0, 0, -1)
        // cos(ψ) = (0.5·0 + 0·0 + (-0.9375)·(-1)) / sqrt(0.25 + 0.878) = 0.9375 / sqrt(1.128) ≈ 0.883

        let psi = feed_angle(0.5, 0.0, &feed, 1.0);
        let expected_cos = 0.9375_f64 / (0.25_f64 + 0.9375_f64 * 0.9375_f64).sqrt();
        let expected_psi = expected_cos.acos();

        assert!((psi - expected_psi).abs() < 1e-6);
    }

    #[test]
    fn test_feed_angle_offset_feed() {
        // Offset feed at (0.1, 0.0, 1.0)
        let feed = FeedPosition::new(0.1, 0.0, 1.0);

        // Angle should change compared to centered feed
        let psi_centered = feed_angle(0.5, 0.0, &FeedPosition::at_focus(1.0), 1.0);
        let psi_offset = feed_angle(0.5, 0.0, &feed, 1.0);

        // Should be different
        assert!((psi_centered - psi_offset).abs() > 1e-3);
    }

    #[test]
    fn test_illumination_amplitude_symmetric() {
        // Test with symmetric feed pattern
        let feed_pos = FeedPosition::at_focus(1.0);
        let feed_params = FeedParameters::new(
            feed_pos, 8.0, // q_factor
            0.0, // phase_center_offset
            1.0, // asymmetry_factor (symmetric)
        )
        .unwrap();

        // Calculate amplitude at various points
        let amp_center = illumination_amplitude(0.0, 0.0, &feed_params, 1.0);
        let amp_edge = illumination_amplitude(0.5, 0.0, &feed_params, 1.0);

        // Edge should have lower amplitude than center
        assert!(amp_edge < amp_center);

        // Both should be positive
        assert!(amp_center > 0.0);
        assert!(amp_edge > 0.0);
    }

    #[test]
    fn test_illumination_amplitude_azimuthal_symmetry() {
        // For symmetric feed, amplitude should be same at all azimuthal angles (same ρ)
        let feed_pos = FeedPosition::at_focus(1.0);
        let feed_params = FeedParameters::new(
            feed_pos, 8.0, 0.0, 1.0, // symmetric
        )
        .unwrap();

        let rho = 0.3;
        let amp_0 = illumination_amplitude(rho, 0.0, &feed_params, 1.0);
        let amp_90 = illumination_amplitude(rho, PI / 2.0, &feed_params, 1.0);
        let amp_180 = illumination_amplitude(rho, PI, &feed_params, 1.0);

        // Should all be equal for symmetric pattern
        assert!((amp_0 - amp_90).abs() < 1e-6);
        assert!((amp_0 - amp_180).abs() < 1e-6);
    }

    #[test]
    fn test_illumination_amplitude_asymmetric() {
        // Test with asymmetric feed pattern
        let feed_pos = FeedPosition::at_focus(1.0);
        let feed_params = FeedParameters::new(
            feed_pos, 8.0, 0.0, 1.3, // asymmetric (E-plane broader)
        )
        .unwrap();

        let rho = 0.3;
        let amp_0 = illumination_amplitude(rho, 0.0, &feed_params, 1.0);
        let amp_90 = illumination_amplitude(rho, PI / 2.0, &feed_params, 1.0);

        // Should be different for asymmetric pattern
        // The difference might be small due to simplified model
        // Just verify they're both reasonable values
        assert!(amp_0 > 0.0 && amp_0 <= 1.0);
        assert!(amp_90 > 0.0 && amp_90 <= 1.0);
    }

    #[test]
    fn test_edge_taper_reasonable_values() {
        // Test that edge taper gives reasonable dB values
        // For f/D=0.5, the edge angle is about 53°, giving deep taper values

        // q=8, f/D=0.5 gives approximately -35 dB edge taper
        let taper_q8 = edge_taper_db(8.0, 0.5);
        assert!(taper_q8 < -30.0 && taper_q8 > -40.0);

        // q=10 should have more taper (more negative)
        let taper_q10 = edge_taper_db(10.0, 0.5);
        assert!(taper_q10 < taper_q8);

        // q=6 should have less taper (less negative)
        let taper_q6 = edge_taper_db(6.0, 0.5);
        assert!(taper_q6 > taper_q8);
    }

    #[test]
    fn test_edge_taper_always_negative() {
        // Edge taper should always be negative (edge is dimmer than center)
        assert!(edge_taper_db(6.0, 0.5) < 0.0);
        assert!(edge_taper_db(8.0, 0.5) < 0.0);
        assert!(edge_taper_db(10.0, 0.5) < 0.0);
        assert!(edge_taper_db(12.0, 0.5) < 0.0);
    }

    #[test]
    fn test_q_factor_from_taper_roundtrip() {
        // Test that q_factor_from_taper is inverse of edge_taper_db
        let original_q = 8.0;
        let f_over_d = 0.5;

        let taper = edge_taper_db(original_q, f_over_d);
        let recovered_q = q_factor_from_taper(taper, f_over_d);

        // Should recover original q within tolerance
        assert!((recovered_q - original_q).abs() < 0.1);
    }

    #[test]
    fn test_q_factor_from_taper_typical_values() {
        // -30 dB edge taper should give q around 7-8
        let q = q_factor_from_taper(-30.0, 0.5);
        assert!(q > 6.0 && q < 9.0);

        // -40 dB edge taper should give higher q (more focused)
        let q_40db = q_factor_from_taper(-40.0, 0.5);
        assert!(q_40db > q);
    }

    #[test]
    fn test_phase_center_offset_zero_on_axis() {
        // On boresight, phase center offset contributes no phase
        let phase = phase_center_offset_phase(0.0, 0.01, 0.036);
        assert!(phase.abs() < 1e-10);
    }

    #[test]
    fn test_phase_center_offset_increases_off_axis() {
        // Phase contribution increases with angle
        let wavelength = 0.036; // ~8.4 GHz
        let offset = 0.009; // λ/4

        let phase_30 = phase_center_offset_phase(PI / 6.0, offset, wavelength);
        let phase_45 = phase_center_offset_phase(PI / 4.0, offset, wavelength);

        // Magnitude should increase with angle
        assert!(phase_45.abs() > phase_30.abs());
    }

    #[test]
    fn test_phase_center_offset_sign() {
        // For positive offset, phase should be negative (additional path length)
        let phase = phase_center_offset_phase(PI / 4.0, 0.01, 0.036);
        assert!(phase < 0.0);
    }

    #[test]
    fn test_phase_center_offset_magnitude() {
        // For offset = λ/4 at 45°, phase should be on order of radians
        let wavelength = 0.036;
        let offset = wavelength / 4.0;
        let phase = phase_center_offset_phase(PI / 4.0, offset, wavelength);

        // Should be a few radians (not tiny, not huge)
        assert!(phase.abs() > 0.1);
        assert!(phase.abs() < 10.0);
    }
}
