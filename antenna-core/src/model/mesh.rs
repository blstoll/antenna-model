//! Mesh Reflector Physics
//!
//! Models the power reflection efficiency of a wire-mesh reflector using the
//! Wait/Marcuvitz inductive-grid shunt model. This is the single mesh effect
//! consumed by the gain pipeline (`pattern::overall_efficiency`); the older
//! transparency/angle/polarization helpers were removed as dead code once the
//! calibration correction surface subsumed their role.
//!
//! # References
//! - Wait, J. R. (1954). "Reflection from a wire grid parallel to a conducting plane."
//! - Marcuvitz, N. (1951). *Waveguide Handbook*, Section 6.8.
//! - Design doc Section 2.2 (Mesh-Specific Phase) and 2.4

/// Power reflection efficiency of a square wire mesh (inductive grid model).
///
/// Uses the Wait/Marcuvitz inductive-grid shunt model.  The normalized shunt
/// reactance for a square mesh (wire spacing `g`, wire radius `a = d/2`) is:
/// ```text
/// X = (g/λ) · ln(g / (π·d))
/// ```
/// and the power reflectivity (efficiency) is:
/// ```text
/// |R|² = 1 / (1 + 4·X²)
/// ```
///
/// # Behaviour
/// - λ → ∞ (low frequency): X → 0, |R|² → 1 (solid-reflector limit)
/// - Decreasing λ increases X, reducing efficiency monotonically
/// - No step discontinuity — continuous and physically correct
///
/// Falls back to solid-reflector behaviour (`1.0`) when:
/// - `mesh_spacing ≤ 0` or `wavelength ≤ 0` (invalid inputs)
/// - The log term is ≤ 0 (wire so thick the surface is effectively solid)
///
/// # Arguments
/// - `mesh_spacing`: Centre-to-centre spacing between parallel wires in metres
/// - `wire_diameter`: Wire diameter in metres
/// - `wavelength`: Free-space wavelength in metres
///
/// # Returns
/// Reflection efficiency factor in [0, 1]
pub fn mesh_reflection_efficiency(mesh_spacing: f64, wire_diameter: f64, wavelength: f64) -> f64 {
    if mesh_spacing <= 0.0 || wavelength <= 0.0 {
        return 1.0;
    }
    let log_term = (mesh_spacing / (std::f64::consts::PI * wire_diameter)).ln();
    if log_term <= 0.0 {
        return 1.0; // wire so thick the surface is effectively solid
    }
    let x = (mesh_spacing / wavelength) * log_term;
    1.0 / (1.0 + 4.0 * x * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflection_efficiency_low_frequency_is_high() {
        // 5mm mesh, 0.5mm wire at 100 MHz (λ=3m): excellent reflector
        let eff = mesh_reflection_efficiency(0.005, 0.0005, 3.0);
        assert!(eff > 0.99, "got {eff}");
    }

    #[test]
    fn test_reflection_efficiency_monotonic_in_wavelength() {
        let (g, d) = (0.005, 0.0005);
        let mut prev = 0.0;
        for lambda in [0.005, 0.01, 0.0357, 0.1, 1.0, 3.0] {
            let eff = mesh_reflection_efficiency(g, d, lambda);
            assert!(eff >= prev, "non-monotonic at λ={lambda}: {eff} < {prev}");
            prev = eff;
        }
    }

    #[test]
    fn test_reflection_efficiency_continuous_at_old_cutoff() {
        let (g, d) = (0.005, 0.0005);
        let cutoff = std::f64::consts::PI * g;
        let below = mesh_reflection_efficiency(g, d, cutoff * 0.999);
        let above = mesh_reflection_efficiency(g, d, cutoff * 1.001);
        assert!(
            (below - above).abs() < 0.01,
            "step at cutoff: {below} vs {above}"
        );
    }
}
