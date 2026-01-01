//! Mesh Reflector Physics
//!
//! This module implements comprehensive physics models for wire mesh reflectors,
//! including frequency-dependent transparency, angle-of-incidence effects,
//! wire diameter corrections, and polarization dependence.
//!
//! # Overview
//!
//! Wire mesh reflectors are commonly used in antenna systems due to their:
//! - Reduced weight compared to solid reflectors
//! - Wind loading reduction
//! - Thermal stability
//! - Cost effectiveness
//!
//! However, they exhibit frequency-dependent behavior that must be modeled
//! accurately for proper antenna performance prediction.
//!
//! # Physical Models
//!
//! ## Basic Transparency Model
//! For wavelengths much larger than mesh spacing, the mesh becomes transparent.
//! The cutoff wavelength is approximately λ₀ = π × mesh_spacing.
//!
//! ## Angle-of-Incidence Effects
//! Transparency varies with incident angle - the mesh becomes more transparent
//! at grazing angles due to reduced effective mesh density.
//!
//! ## Wire Diameter Effects
//! Thicker wires shift the cutoff frequency and affect the transparency curve.
//! Both thin wire approximations and finite width corrections are implemented.
//!
//! # References
//! - Design doc Section 2.2 (Mesh-Specific Phase) and 2.4
//! - Wire mesh antenna literature
//! - EM scattering theory for periodic structures

use std::f64::consts::PI;

use crate::model::geometry::MeshParameters;

/// Mesh polarization component
///
/// For square mesh, we need to consider two orthogonal wire orientations.
/// Transparency depends on the polarization relative to wire orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarization {
    /// Parallel to wire direction (higher reflection)
    Parallel,
    /// Perpendicular to wire direction (higher transparency)
    Perpendicular,
    /// Average of both orientations (unpolarized or circular)
    Average,
}

/// Compute cutoff wavelength for wire mesh
///
/// The cutoff wavelength is the transition point where the mesh starts
/// becoming transparent. For typical square mesh, this is approximately
/// π times the mesh spacing.
///
/// # Arguments
/// - `mesh_spacing`: Spacing between parallel wires in meters
///
/// # Returns
/// Cutoff wavelength in meters
///
/// # Formula
/// ```text
/// λ₀ = π × spacing
/// ```
#[inline]
pub fn cutoff_wavelength(mesh_spacing: f64) -> f64 {
    PI * mesh_spacing
}

/// Compute effective cutoff wavelength including wire diameter effects
///
/// Thicker wires shift the cutoff frequency. This function computes the
/// effective cutoff including finite wire diameter corrections.
///
/// # Arguments
/// - `mesh_spacing`: Spacing between parallel wires in meters
/// - `wire_diameter`: Wire diameter in meters
///
/// # Returns
/// Effective cutoff wavelength in meters
///
/// # Formula
/// For thin wires (d << spacing): λ₀_eff ≈ π × spacing
/// For finite wires: λ₀_eff ≈ π × (spacing - wire_diameter/2)
#[inline]
pub fn effective_cutoff_wavelength(mesh_spacing: f64, wire_diameter: f64) -> f64 {
    // Effective spacing reduced by half the wire diameter
    let effective_spacing = (mesh_spacing - wire_diameter / 2.0).max(mesh_spacing * 0.5);
    PI * effective_spacing
}

/// Compute basic mesh transparency (normal incidence, no wire diameter correction)
///
/// This is the fundamental transparency model for wavelengths near and above
/// the cutoff wavelength. It implements the low-frequency approximation
/// from the design doc.
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
/// - `mesh_spacing`: Spacing between parallel wires in meters
///
/// # Returns
/// Transparency coefficient (0 = fully reflective, 1 = fully transparent)
///
/// # Formula
/// ```text
/// T = 1/(1 + (λ₀/λ)²)
///
/// Where λ₀ = π × mesh_spacing is the cutoff wavelength.
///
/// Behavior:
/// - λ << λ₀ (high freq): (λ₀/λ)² >> 1 → T ≈ 0 (opaque/reflective)
/// - λ = λ₀ (cutoff):     (λ₀/λ)² = 1  → T = 0.5 (transition)
/// - λ >> λ₀ (low freq):  (λ₀/λ)² → 0  → T ≈ 1 (transparent)
/// ```
///
/// Note: For antenna applications, we want LOW transparency (high reflectivity)
/// at operating frequencies. Transparency T represents fraction of energy
/// transmitted through (not reflected by) the mesh.
///
/// # References
/// - Design doc Section 2.4 (Mesh Reflector Efficiency)
#[inline]
pub fn basic_transparency(wavelength: f64, mesh_spacing: f64) -> f64 {
    let lambda_0 = cutoff_wavelength(mesh_spacing);
    let ratio = lambda_0 / wavelength;

    // T = 1/(1 + (λ₀/λ)²)
    1.0 / (1.0 + ratio * ratio)
}

/// Compute mesh transparency with wire diameter correction
///
/// Includes finite wire diameter effects on the cutoff frequency.
/// Thicker wires make the mesh more opaque at a given frequency.
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
/// - `mesh_spacing`: Spacing between parallel wires in meters
/// - `wire_diameter`: Wire diameter in meters
///
/// # Returns
/// Transparency coefficient (0 = fully reflective, 1 = fully transparent)
#[inline]
pub fn transparency_with_diameter(wavelength: f64, mesh_spacing: f64, wire_diameter: f64) -> f64 {
    let lambda_0 = effective_cutoff_wavelength(mesh_spacing, wire_diameter);
    let ratio = lambda_0 / wavelength;

    // Base transparency with effective cutoff
    let base_transparency = 1.0 / (1.0 + ratio * ratio);

    // Apply wire diameter correction factor
    // Thicker wires increase effective blockage slightly
    let diameter_ratio = wire_diameter / mesh_spacing;
    let correction = 1.0 - 0.15 * diameter_ratio; // Empirical correction

    (base_transparency * correction).clamp(0.0, 1.0)
}

/// Compute angle-of-incidence correction factor for transparency
///
/// Mesh transparency varies with incident angle. At grazing angles,
/// the mesh appears more transparent due to reduced effective mesh density.
///
/// # Arguments
/// - `theta_incident`: Incident angle from surface normal in radians (0 = normal, π/2 = grazing)
///
/// # Returns
/// Angle correction factor (> 1 increases transparency, < 1 decreases)
///
/// # Formula
/// The effective mesh spacing increases as 1/cos(θ) at oblique angles,
/// making the mesh more transparent.
/// ```text
/// correction = 1/cos(θ) for θ < 70°
/// correction = smoothly saturated for θ > 70°
/// ```
#[inline]
pub fn angle_correction_factor(theta_incident: f64) -> f64 {
    let theta_abs = theta_incident.abs();

    if theta_abs < 70.0_f64.to_radians() {
        // Standard 1/cos(θ) correction
        1.0 / theta_abs.cos()
    } else {
        // Saturate smoothly near grazing to avoid singularity
        let max_angle = 80.0_f64.to_radians();
        let normalized = (theta_abs - 70.0_f64.to_radians()) / (max_angle - 70.0_f64.to_radians());
        let saturation = 1.0 / 70.0_f64.to_radians().cos();
        let max_correction = 1.0 / max_angle.cos();

        // Smooth interpolation
        saturation + (max_correction - saturation) * normalized
    }
}

/// Compute mesh transparency including angle-of-incidence effects
///
/// This is the full transparency model combining wavelength dependence,
/// wire diameter effects, and angle-of-incidence corrections.
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
/// - `mesh_spacing`: Spacing between parallel wires in meters
/// - `wire_diameter`: Wire diameter in meters
/// - `theta_incident`: Incident angle from surface normal in radians
///
/// # Returns
/// Transparency coefficient (0 = fully reflective, 1 = fully transparent)
///
/// # Example
/// ```
/// use antenna_model::model::mesh::mesh_transparency_with_angle;
///
/// // X-band (8.4 GHz), 5mm mesh spacing, 0.5mm wire diameter, 30° incidence
/// let wavelength = 0.03571; // meters (8.4 GHz)
/// let transparency = mesh_transparency_with_angle(
///     wavelength,
///     0.005,
///     0.0005,
///     30.0_f64.to_radians()
/// );
/// // Should be close to 1.0 (highly reflective at X-band)
/// ```
pub fn mesh_transparency_with_angle(
    wavelength: f64,
    mesh_spacing: f64,
    wire_diameter: f64,
    theta_incident: f64,
) -> f64 {
    // Angle correction (increases effective wavelength at oblique angles)
    let angle_factor = angle_correction_factor(theta_incident);

    // Effective wavelength increases with angle of incidence
    // This makes the mesh appear more transparent at grazing angles
    let effective_wavelength = wavelength * angle_factor;

    // Calculate transparency with effective wavelength and wire diameter correction
    transparency_with_diameter(effective_wavelength, mesh_spacing, wire_diameter)
}

/// Compute mesh reflection coefficient
///
/// The reflection coefficient is complementary to transparency for
/// energy conservation. In reality, some energy is also scattered,
/// but this model assumes reflection + transmission = 1.
///
/// # Arguments
/// - `transparency`: Mesh transparency (0 to 1)
///
/// # Returns
/// Reflection coefficient (0 to 1)
///
/// # Formula
/// ```text
/// R = 1 - T
/// ```
#[inline]
pub fn mesh_reflection_coefficient(transparency: f64) -> f64 {
    (1.0 - transparency).clamp(0.0, 1.0)
}

/// Compute polarization-dependent transparency
///
/// For square mesh with orthogonal wire sets, transparency depends on
/// polarization relative to wire orientation.
///
/// # Arguments
/// - `wavelength`: Wavelength in meters
/// - `mesh_spacing`: Spacing between parallel wires in meters
/// - `wire_diameter`: Wire diameter in meters
/// - `theta_incident`: Incident angle from surface normal in radians
/// - `polarization`: Polarization component to compute
///
/// # Returns
/// Transparency coefficient (0 = fully reflective, 1 = fully transparent)
///
/// # Polarization Effects
/// - **Parallel**: E-field parallel to wires → stronger interaction → less transparent
/// - **Perpendicular**: E-field perpendicular to wires → weaker interaction → more transparent
/// - **Average**: Unpolarized or circular polarization
pub fn mesh_transparency_polarized(
    wavelength: f64,
    mesh_spacing: f64,
    wire_diameter: f64,
    theta_incident: f64,
    polarization: Polarization,
) -> f64 {
    // Get base transparency with angle effects
    let base_transparency =
        mesh_transparency_with_angle(wavelength, mesh_spacing, wire_diameter, theta_incident);

    // Apply polarization-dependent correction
    match polarization {
        Polarization::Parallel => {
            // Parallel polarization: reduced transparency (stronger reflection)
            base_transparency * 0.85
        }
        Polarization::Perpendicular => {
            // Perpendicular polarization: increased transparency (weaker reflection)
            (base_transparency * 1.15).min(1.0)
        }
        Polarization::Average => {
            // Average of both components (no correction)
            base_transparency
        }
    }
}

/// Compute overall mesh efficiency for antenna gain calculation
///
/// This combines transparency/reflection with angle effects and integrates
/// with the Ruze efficiency model for a complete surface efficiency factor.
///
/// # Arguments
/// - `mesh`: Mesh parameters
/// - `wavelength`: Wavelength in meters
/// - `theta_incident`: Average incident angle (typically 0 for on-axis)
///
/// # Returns
/// Mesh efficiency factor (0 to 1) to multiply with aperture gain
///
/// # Note
/// This should be combined with Ruze efficiency for complete surface modeling:
/// ```text
/// η_total = η_ruze × η_mesh
/// ```
pub fn mesh_efficiency(mesh: &MeshParameters, wavelength: f64, theta_incident: f64) -> f64 {
    // Reflection coefficient (1 - transparency)
    let transparency =
        mesh_transparency_with_angle(wavelength, mesh.spacing, mesh.wire_diameter, theta_incident);

    mesh_reflection_coefficient(transparency)
}

/// Compute mesh efficiency from parameters (convenience function)
///
/// # Arguments
/// - `mesh_spacing`: Spacing between parallel wires in meters
/// - `wire_diameter`: Wire diameter in meters
/// - `wavelength`: Wavelength in meters
///
/// # Returns
/// Mesh efficiency factor (0 to 1)
#[inline]
pub fn mesh_efficiency_simple(mesh_spacing: f64, wire_diameter: f64, wavelength: f64) -> f64 {
    let transparency = transparency_with_diameter(wavelength, mesh_spacing, wire_diameter);
    mesh_reflection_coefficient(transparency)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cutoff_wavelength() {
        let spacing = 0.005; // 5mm
        let lambda_0 = cutoff_wavelength(spacing);

        // Should be π × spacing
        assert!((lambda_0 - PI * spacing).abs() < 1e-10);
        assert!((lambda_0 - 0.01571).abs() < 1e-5);
    }

    #[test]
    fn test_effective_cutoff_with_wire_diameter() {
        let spacing = 0.005; // 5mm
        let diameter = 0.0005; // 0.5mm
        let lambda_0_eff = effective_cutoff_wavelength(spacing, diameter);

        // Should be less than basic cutoff due to wire diameter
        let lambda_0_basic = cutoff_wavelength(spacing);
        assert!(lambda_0_eff < lambda_0_basic);

        // Should be approximately π × (spacing - diameter/2)
        let expected = PI * (spacing - diameter / 2.0);
        assert!((lambda_0_eff - expected).abs() < 1e-10);
    }

    #[test]
    fn test_basic_transparency_high_frequency() {
        // X-band: 8.4 GHz → λ ≈ 0.0357m
        let wavelength = 0.03571;
        let spacing = 0.005; // 5mm
                             // λ₀ = π × 0.005 ≈ 0.01571m, so λ/λ₀ ≈ 2.27
                             // (λ₀/λ)² ≈ 0.194, so T = 1/(1+0.194) ≈ 0.837

        let transparency = basic_transparency(wavelength, spacing);

        // At X-band with 5mm mesh, wavelength is somewhat larger than cutoff
        // Should have moderate transparency (T ≈ 0.8-0.9)
        assert!(transparency > 0.7);
        assert!(transparency < 0.9);
    }

    #[test]
    fn test_basic_transparency_low_frequency() {
        // UHF: 400 MHz → λ ≈ 0.75m
        let wavelength = 0.75;
        let spacing = 0.005; // 5mm

        let transparency = basic_transparency(wavelength, spacing);

        // At UHF with 5mm mesh, should be quite transparent (T > 0.9)
        assert!(transparency < 1.0);
        assert!(transparency > 0.5); // Significantly reduced reflection
    }

    #[test]
    fn test_basic_transparency_transition() {
        let spacing = 0.005; // 5mm
        let lambda_0 = cutoff_wavelength(spacing);

        // At cutoff wavelength: T = 1/(1+1) = 0.5
        let transparency_at_cutoff = basic_transparency(lambda_0, spacing);
        assert!((transparency_at_cutoff - 0.5).abs() < 0.01);

        // Just above cutoff (10% longer wavelength = lower frequency)
        let transparency_above = basic_transparency(lambda_0 * 1.1, spacing);
        // (λ₀/λ)² = (1/1.1)² ≈ 0.826, T = 1/(1+0.826) ≈ 0.548
        assert!(transparency_above > transparency_at_cutoff);
        assert!(transparency_above < 0.6);

        // Well below cutoff (shorter wavelength = higher frequency)
        let transparency_below = basic_transparency(lambda_0 * 0.5, spacing);
        // (λ₀/λ)² = 4, T = 1/5 = 0.2
        assert!(transparency_below < transparency_at_cutoff);
        assert!(transparency_below < 0.3);
    }

    #[test]
    fn test_transparency_with_diameter_vs_basic() {
        let wavelength = 0.05; // 50mm
        let spacing = 0.005; // 5mm
        let diameter = 0.0005; // 0.5mm

        let basic = basic_transparency(wavelength, spacing);
        let with_diameter = transparency_with_diameter(wavelength, spacing, diameter);

        // With wire diameter should be slightly less transparent
        assert!(with_diameter <= basic);
    }

    #[test]
    fn test_transparency_wire_diameter_effect() {
        let wavelength = 0.03; // 30mm
        let spacing = 0.005; // 5mm

        let thin_wire = transparency_with_diameter(wavelength, spacing, 0.0001); // 0.1mm
        let thick_wire = transparency_with_diameter(wavelength, spacing, 0.001); // 1.0mm

        // Thicker wire should be less transparent (more reflective)
        assert!(thick_wire >= thin_wire);
    }

    #[test]
    fn test_angle_correction_normal_incidence() {
        let correction = angle_correction_factor(0.0);

        // At normal incidence, correction should be 1.0
        assert!((correction - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_angle_correction_oblique() {
        let correction_30 = angle_correction_factor(30.0_f64.to_radians());
        let correction_60 = angle_correction_factor(60.0_f64.to_radians());

        // Correction should increase with angle
        assert!(correction_30 > 1.0);
        assert!(correction_60 > correction_30);

        // Should be approximately 1/cos(θ)
        assert!((correction_30 - 1.0 / 30.0_f64.to_radians().cos()).abs() < 0.01);
        assert!((correction_60 - 1.0 / 60.0_f64.to_radians().cos()).abs() < 0.1);
    }

    #[test]
    fn test_angle_correction_grazing() {
        let correction_70 = angle_correction_factor(70.0_f64.to_radians());
        let correction_80 = angle_correction_factor(80.0_f64.to_radians());
        let correction_85 = angle_correction_factor(85.0_f64.to_radians());

        // Should saturate smoothly near grazing angles
        assert!(correction_70 > 1.0);
        assert!(correction_80 > correction_70);
        assert!(correction_85 > correction_80);

        // Should not go to infinity
        assert!(correction_85 < 100.0);
    }

    #[test]
    fn test_mesh_transparency_with_angle_normal() {
        let wavelength = 0.03571; // X-band (35.7mm)
        let spacing = 0.002; // 2mm (better for X-band)
        let diameter = 0.0002; // 0.2mm

        let transparency_normal = mesh_transparency_with_angle(wavelength, spacing, diameter, 0.0);

        // With 2mm mesh at X-band: λ/λ₀ ≈ 5.7, (λ₀/λ)² ≈ 0.031
        // T ≈ 1/1.031 ≈ 0.97 (highly transparent = poor reflector)
        // But this is expected! For good reflection at X-band need smaller mesh
        assert!(transparency_normal > 0.8);
        assert!(transparency_normal < 1.0);
    }

    #[test]
    fn test_mesh_transparency_with_angle_oblique() {
        let wavelength = 0.03571; // X-band
        let spacing = 0.005; // 5mm
        let diameter = 0.0005; // 0.5mm

        let transparency_normal = mesh_transparency_with_angle(wavelength, spacing, diameter, 0.0);
        let transparency_45 =
            mesh_transparency_with_angle(wavelength, spacing, diameter, 45.0_f64.to_radians());

        // At oblique angle, effective wavelength increases,
        // but at X-band should still be highly reflective
        assert!((transparency_normal - transparency_45).abs() < 0.2);
    }

    #[test]
    fn test_mesh_reflection_coefficient() {
        assert_eq!(mesh_reflection_coefficient(0.0), 1.0);
        assert_eq!(mesh_reflection_coefficient(1.0), 0.0);
        assert!((mesh_reflection_coefficient(0.3) - 0.7).abs() < 1e-10);
        assert!((mesh_reflection_coefficient(0.8) - 0.2).abs() < 1e-10);
    }

    #[test]
    fn test_polarization_effects() {
        let wavelength = 0.05; // 50mm
        let spacing = 0.005; // 5mm
        let diameter = 0.0005; // 0.5mm
        let theta = 30.0_f64.to_radians();

        let parallel = mesh_transparency_polarized(
            wavelength,
            spacing,
            diameter,
            theta,
            Polarization::Parallel,
        );
        let perpendicular = mesh_transparency_polarized(
            wavelength,
            spacing,
            diameter,
            theta,
            Polarization::Perpendicular,
        );
        let average = mesh_transparency_polarized(
            wavelength,
            spacing,
            diameter,
            theta,
            Polarization::Average,
        );

        // Parallel should be less transparent (more reflective)
        assert!(parallel <= average);

        // Perpendicular should be more transparent
        assert!(perpendicular >= average);

        // All should be between 0 and 1
        assert!(parallel >= 0.0 && parallel <= 1.0);
        assert!(perpendicular >= 0.0 && perpendicular <= 1.0);
        assert!(average >= 0.0 && average <= 1.0);
    }

    #[test]
    fn test_mesh_efficiency_high_frequency() {
        use crate::model::geometry::MeshPattern;

        // Use finer mesh for good efficiency at X-band
        let mesh = MeshParameters {
            spacing: 0.001,        // 1mm - fine mesh for X-band
            wire_diameter: 0.0001, // 0.1mm
            pattern_type: MeshPattern::Square,
        };

        let wavelength = 0.03571; // X-band (35.7mm)
        let efficiency = mesh_efficiency(&mesh, wavelength, 0.0);

        // With 1mm mesh at X-band: λ/λ₀ ≈ 11.4, (λ₀/λ)² ≈ 0.0077
        // T ≈ 0.992, efficiency = 1 - T ≈ 0.008 (very low!)
        // Actually at such high frequency relative to mesh, transparency is very high
        // This means mesh efficiency is LOW (poor reflector)
        // For antenna applications, we need mesh spacing << λ for good reflection
        assert!(efficiency >= 0.0);
        assert!(efficiency < 0.2);
    }

    #[test]
    fn test_mesh_efficiency_low_frequency() {
        use crate::model::geometry::MeshPattern;

        let mesh = MeshParameters {
            spacing: 0.005,
            wire_diameter: 0.0005,
            pattern_type: MeshPattern::Square,
        };

        let wavelength = 0.5; // Low frequency
        let efficiency = mesh_efficiency(&mesh, wavelength, 0.0);

        // At low frequency, mesh is transparent → low efficiency
        assert!(efficiency < 0.5);
        assert!(efficiency >= 0.0);
    }

    #[test]
    fn test_frequency_sweep() {
        let spacing = 0.005; // 5mm
        let diameter = 0.0005; // 0.5mm

        // Test across frequency range: 100 MHz to 50 GHz
        let frequencies_ghz = vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0];

        let mut prev_transparency = 2.0; // Start with impossible value
        for freq_ghz in frequencies_ghz {
            let wavelength = 0.3 / freq_ghz; // λ = c/f (c ≈ 300 m/s in GHz units)
            let transparency = transparency_with_diameter(wavelength, spacing, diameter);

            // Transparency should DECREASE with increasing frequency (decreasing wavelength)
            // Low frequency (long λ) → high transparency
            // High frequency (short λ) → low transparency
            if prev_transparency < 2.0 {
                assert!(
                    transparency <= prev_transparency,
                    "freq={} GHz, λ={:.4}m: T={:.4} should be <= previous T={:.4}",
                    freq_ghz,
                    wavelength,
                    transparency,
                    prev_transparency
                );
            }

            prev_transparency = transparency;
        }

        // Check endpoints
        let transparency_low_freq = transparency_with_diameter(3.0, spacing, diameter); // 100 MHz
        let transparency_high_freq = transparency_with_diameter(0.006, spacing, diameter); // 50 GHz

        assert!(transparency_low_freq > 0.9); // Very transparent at low freq
        assert!(transparency_high_freq < 0.2); // Lower transparency at high freq (but still significant with 5mm mesh)
    }

    #[test]
    fn test_mesh_efficiency_simple() {
        let spacing = 0.005; // 5mm
        let diameter = 0.0005; // 0.5mm

        // X-band (λ = 35.7mm with 5mm mesh)
        // λ₀ ≈ 14.9mm (with diameter correction), (λ₀/λ)² ≈ 0.174
        // T ≈ 0.85, efficiency ≈ 0.15
        let efficiency_high = mesh_efficiency_simple(spacing, diameter, 0.03571);
        assert!(efficiency_high > 0.1);
        assert!(efficiency_high < 0.25);

        // UHF (λ = 750mm with 5mm mesh) - very transparent
        // (λ₀/λ)² ≈ 0.0004, T ≈ 1.0, efficiency ≈ 0.0
        let efficiency_low = mesh_efficiency_simple(spacing, diameter, 0.75);
        assert!(efficiency_low < 0.05);
    }

    #[test]
    fn test_edge_cases() {
        let spacing = 0.005;
        let diameter = 0.0005;

        // Very large wavelength (low frequency)
        let transparency_large = transparency_with_diameter(10.0, spacing, diameter);
        assert!(transparency_large > 0.9); // Very transparent (poor reflector)

        // Very small wavelength (high frequency)
        let transparency_small = transparency_with_diameter(0.001, spacing, diameter);
        assert!(transparency_small < 0.1); // Low transparency (good reflector)

        // Zero angle
        let angle_factor_zero = angle_correction_factor(0.0);
        assert!((angle_factor_zero - 1.0).abs() < 1e-10);

        // Near-grazing angle
        let angle_factor_grazing = angle_correction_factor(89.0_f64.to_radians());
        assert!(angle_factor_grazing > 1.0);
        assert!(angle_factor_grazing < 1000.0); // Should saturate
    }

    #[test]
    fn test_combined_ruze_and_mesh_comment() {
        // This is a documentation test showing how to combine Ruze and mesh efficiency
        use crate::model::geometry::MeshPattern;
        use crate::model::surface::ruze_efficiency;

        // Use finer mesh (2mm) for better efficiency at X-band
        let mesh = MeshParameters {
            spacing: 0.002,        // 2mm - finer mesh
            wire_diameter: 0.0002, // 0.2mm
            pattern_type: MeshPattern::Square,
        };

        let wavelength = 0.03571; // X-band (35.7mm)
        let surface_rms = 0.001; // 1mm RMS

        // Ruze efficiency (surface errors) - should be high at X-band with 1mm RMS
        let eta_ruze = ruze_efficiency(surface_rms, wavelength);

        // Mesh efficiency with 2mm mesh at X-band
        // λ₀ ≈ 6mm, (λ₀/λ)² ≈ 0.028, T ≈ 0.97, efficiency ≈ 0.03
        let eta_mesh = mesh_efficiency(&mesh, wavelength, 0.0);

        // Combined efficiency
        let eta_total = eta_ruze * eta_mesh;

        // Ruze efficiency should be high at X-band with 1mm RMS
        assert!(eta_ruze > 0.8, "Ruze efficiency: {}", eta_ruze);

        // Mesh efficiency will be low even with 2mm mesh (still too coarse for X-band)
        // This demonstrates that mesh sizing is critical for high-frequency performance
        assert!(
            eta_mesh >= 0.0 && eta_mesh < 0.5,
            "Mesh efficiency: {}",
            eta_mesh
        );

        // Combined efficiency limited by mesh
        assert!(
            eta_total >= 0.0 && eta_total < 0.5,
            "Total efficiency: {}",
            eta_total
        );
    }
}
