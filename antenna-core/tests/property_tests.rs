//! Property-based tests (roadmap D7).
//!
//! Makes real the CLAUDE.md claim that the coordinate and physics model has
//! property-based round-trip and bounds coverage. The properties live here as
//! integration tests rather than in-module `#[cfg(test)]` units because they
//! drag in the `proptest` dev-dependency, which we keep out of the shipped
//! crate's normal unit-test surface.
//!
//! ## The one rule that keeps these meaningful
//!
//! **Every generator is constrained to the *validated physical domain*.** The
//! upstream entry points (`ReflectorGeometry::validate`, `validate_geodetic`,
//! `validate_ecef`, frequency bounds, etc.) reject unphysical input with a
//! `ValidationError` *before* any physics runs, so a naive `any::<f64>()`
//! strategy would "discover" inputs validation already filters — a property
//! that only passes because it never touches the physics. Here the strategies
//! build values through the same builders `main.rs` / `service` use, so they
//! are guaranteed valid and the properties actually exercise the transforms.
//!
//! Per the D7 charter, **a property failure is a finding to file in the
//! roadmap, not something to weaken the property or generator around.** If one
//! of these ever fails, investigate before loosening a tolerance.

use antenna_core::model::{
    compute_gain, normalize_angle, normalize_angle_symmetric, ruze_efficiency,
};
use antenna_core::model::{
    ecef_to_enu_rotation, ecef_to_geodetic, geodetic_to_ecef, theoretical_max_gain,
    wavelength_from_frequency, AntennaConfiguration, ApertureCoordinates, EClockConeCoordinates,
    FarFieldCoordinates, FeedParameters, IntegrationParams, ReflectorGeometry,
};
use proptest::prelude::*;
use std::f64::consts::PI;

/// Tight relative slack for coordinate round-trips (radians / meters).
const ANGLE_TOL: f64 = 1e-6;

/// A physically-valid reflector, generated inside the validated f/D and RMS
/// domains so `ReflectorGeometry::validate` always accepts it.
fn reflector() -> impl Strategy<Value = ReflectorGeometry> {
    // f/D must stay within [F_OVER_D_MIN, F_OVER_D_MAX] = [0.2, 1.0]; the
    // builder derives focal_length from diameter so the ratio is exact.
    (0.5f64..8.0, 0.25f64..0.9, 0.0f64..0.03).prop_map(|(diameter, f_over_d, surface_rms)| {
        ReflectorGeometry::builder()
            .diameter(diameter)
            .focal_length(diameter * f_over_d)
            .surface_rms(surface_rms)
            .build()
            .expect("reflector generated inside its validated domain")
    })
}

/// A valid antenna (feed at focus → cheap symmetric integrator branch) paired
/// with a frequency within the model's validated band [100, 50,000] MHz.
///
/// Diameter and frequency are jointly capped so `D/λ` stays modest (
/// ≤ ~180), keeping the on-axis physical-optics sweep fast enough for CI.
fn on_axis_config() -> impl Strategy<Value = (AntennaConfiguration, f64)> {
    (reflector(), 100.0e6f64..8.4e9).prop_map(|(refl, freq)| {
        let feed = FeedParameters::builder()
            .at_focus(refl.focal_length)
            .q_factor(8.0)
            .build()
            .expect("feed at focus is always valid");
        let config = AntennaConfiguration::builder()
            .id("prop")
            .name("property-test")
            .reflector(refl)
            .feed(feed)
            .build()
            .expect("config within validated domain");
        (config, freq)
    })
}

/// A surface-rms / wavelength pair where Ruze gain loss is *representable* in f64:
/// wavelength inside the model's validated band [100, 50,000] MHz and rms bounded to a
/// fraction of the wavelength (`4π·rms/λ ≤ ~5`, so `exp(-arg²) > 0`). Beyond this regime
/// the efficiency legitimately underflows to exactly `0.0`, which the broad closed-domain
/// test covers instead. Yields `(rms, wavelength)`.
fn ruze_representable() -> impl Strategy<Value = (f64, f64)> {
    (0.006f64..3.0, 0.0f64..0.4)
        .prop_map(|(wavelength, rms_frac)| (rms_frac * wavelength, wavelength))
}

proptest! {
    // ==== Coordinate round-trips =========================================
    // Cheap: run a full default batch (256 cases).

    #[test]
    fn ecef_geodetic_roundtrip(
        lon_deg in -180.0f64..180.0,
        lat_deg in -90.0f64..90.0,
        alt_m in -1000.0f64..100_000_000.0,
    ) {
        let (x, y, z) = geodetic_to_ecef(lon_deg, lat_deg, alt_m).unwrap();
        let (lon2, lat2, alt2) = ecef_to_geodetic(x, y, z).unwrap();
        // Longitude/latitude are exact angles; Bowring's method converges to <1e-12 rad.
        prop_assert!((lon2 - lon_deg).abs() < ANGLE_TOL, "longitude drifted {lon_deg} -> {lon2}");
        prop_assert!((lat2 - lat_deg).abs() < ANGLE_TOL, "latitude drifted {lat_deg} -> {lat2}");
        // Altitude is a length; allow a relative term so km-scale altitudes hold ~1mm abs.
        prop_assert!(
            (alt2 - alt_m).abs() < 1e-3 + 1e-9 * alt_m.abs(),
            "altitude drifted {alt_m} -> {alt2}"
        );
    }

    #[test]
    fn aperture_cartesian_roundtrip(
        rho in 0.001f64..50.0,
        phi_prime in 0.0f64..(2.0 * PI),
    ) {
        let ap = ApertureCoordinates::new(rho, phi_prime);
        let (x, y) = ap.to_cartesian();
        let back = ApertureCoordinates::from_cartesian(x, y);
        prop_assert!((back.rho - ap.rho).abs() < 1e-9, "rho drift {} -> {}", ap.rho, back.rho);
        let dphi = (back.phi_prime - ap.phi_prime).rem_euclid(2.0 * PI);
        prop_assert!(
            dphi < 1e-9 || (2.0 * PI - dphi) < 1e-9,
            "phi drift {} -> {}",
            ap.phi_prime,
            back.phi_prime
        );
    }

    #[test]
    fn far_field_direction_roundtrip(
        theta in 0.01f64..(PI - 0.01),
        phi in 0.0f64..(2.0 * PI),
    ) {
        let ff = FarFieldCoordinates::new(theta, phi);
        let (x, y, z) = ff.to_direction_vector();
        // A direction vector must be a unit vector.
        prop_assert!((x * x + y * y + z * z - 1.0).abs() < 1e-12, "direction not unit");
        let back = FarFieldCoordinates::from_direction_vector(x, y, z);
        prop_assert!((back.theta - theta).abs() < 1e-9, "theta drift {theta} -> {}", back.theta);
        let dphi = (back.phi - phi).rem_euclid(2.0 * PI);
        prop_assert!(
            dphi < 1e-9 || (2.0 * PI - dphi) < 1e-9,
            "phi drift {} -> {}",
            phi,
            back.phi
        );
    }

    #[test]
    fn e_clock_cone_feed_roundtrip(
        e_cone in 0.001f64..0.3,
        e_clock in 0.0f64..(2.0 * PI),
        focal_length in 1.0f64..20.0,
    ) {
        // Avoid e_cone == 0 where the clock angle is degenerate (any clock is
        // "the same" direction on-axis). The z-defocus term is small in this
        // cone range and `from_feed_position` correctly ignores it.
        let ecc = EClockConeCoordinates::new(e_cone, e_clock);
        let (x, y, z) = ecc.to_feed_position(focal_length);
        let back = EClockConeCoordinates::from_feed_position(x, y, z, focal_length);
        prop_assert!(
            (back.e_cone - e_cone).abs() < ANGLE_TOL,
            "cone drift {e_cone} -> {}",
            back.e_cone
        );
        let dphi = (back.e_clock - e_clock).rem_euclid(2.0 * PI);
        prop_assert!(
            dphi < ANGLE_TOL || (2.0 * PI - dphi) < ANGLE_TOL,
            "clock drift {} -> {}",
            e_clock,
            back.e_clock
        );
    }

    #[test]
    fn enu_rotation_is_orthogonal(
        lat_rad in -PI / 2.0f64..PI / 2.0,
        lon_rad in -PI..PI,
    ) {
        // R·Rᵀ must be the identity for every lat/lon.
        let r = ecef_to_enu_rotation(lat_rad, lon_rad);
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = r[i].iter().zip(r[j].iter()).map(|(a, b)| a * b).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                prop_assert!(
                    (dot - expected).abs() < 1e-9,
                    "R·Rᵀ not identity at [{i}][{j}] (lat={lat_rad}, lon={lon_rad}): {dot}"
                );
            }
        }
    }

    #[test]
    fn normalize_angle_lands_in_bounds(angle in -1000.0f64..1000.0) {
        let n = normalize_angle(angle);
        prop_assert!((0.0..2.0 * PI).contains(&n), "normalize_angle({angle}) = {n} out of [0,2π)");
        // It differs from the input by an integral number of full turns. Checked in
        // *turn* units: `fmod`-style rounding leaves `angle - n` a hair below an exact
        // multiple of 2π, which `rem_euclid` would wrap to ≈2π and falsely flag.
        let turns = (angle - n) / (2.0 * PI);
        prop_assert!((turns - turns.round()).abs() < 1e-6, "{angle} not congruent to {n} mod 2π");
    }

    #[test]
    fn normalize_angle_symmetric_lands_in_bounds(angle in -1000.0f64..1000.0) {
        let n = normalize_angle_symmetric(angle);
        prop_assert!((-PI..PI).contains(&n), "normalize_angle_symmetric({angle}) = {n} out of [-π,π)");
        let turns = (angle - n) / (2.0 * PI);
        prop_assert!((turns - turns.round()).abs() < 1e-6, "{angle} not congruent to {n} mod 2π");
    }

    // ==== Physics bounds =================================================
    // The aperture integral is O(D/λ) and the hot off-axis branch is the
    // expensive one, so the gain property runs a smaller batch (32 cases) with
    // the feed at focus and θ ≤ 45°.

    /// Gain is finite, positive, and never exceeds the ideal-aperture bound
    /// (uniform illumination, 100% aperture efficiency) for any valid input.
    ///
    /// The bound is guaranteed by Cauchy–Schwarz: `|∫A e^{jψ} dA|² ≤ ∫|A|² dA ·
    /// area`, so `D(θ,φ) = (4π/λ²)·|I|²/∫|A|² ≤ (4π/λ²)·area = G_ideal`, and
    /// taper (≤ 1), obliquity (≤ 1) and Ruze/mesh efficiency (≤ 1) only lower
    /// it further. A 0.5 dB slack absorbs floating-point and quadrature slop
    /// while still catching the class of aliasing/overshoot bugs (the historic
    /// +20 to +82 dB) D7 is chartered to screen for.
    #[test]
    fn gain_is_finite_and_bounded_by_ideal_aperture(
        (config, freq) in on_axis_config(),
        theta_deg in 0.0f64..45.0,
        phi in 0.0f64..(2.0 * PI),
    ) {
        let theta = theta_deg.to_radians();
        let result = compute_gain(theta, phi, &config, freq, &IntegrationParams::default());
        prop_assert!(result.is_ok(), "compute_gain failed: {:?}", result.err());
        let gain = result.unwrap().gain;
        prop_assert!(gain.is_finite(), "gain is not finite: {gain}");
        prop_assert!(gain > 0.0, "gain is not positive: {gain}");
        let wavelength = wavelength_from_frequency(freq);
        let bound = theoretical_max_gain(config.reflector.diameter, wavelength, 1.0);
        prop_assert!(
            gain <= bound * 1.12 + 1e-9,
            "gain {gain} exceeds ideal-aperture bound {bound} (D={}m, f={}Hz, θ={theta} rad)",
            config.reflector.diameter,
            freq
        );
    }

    // ==== Ruze efficiency ================================================

    /// In the representable physical regime, Ruze efficiency is strictly inside
    /// (0, 1]: never more than a perfect surface (≤ 1) and never zeroed out.
    #[test]
    fn ruze_efficiency_in_unit_interval((rms, wavelength) in ruze_representable()) {
        let eta = ruze_efficiency(rms, wavelength);
        prop_assert!(eta > 0.0 && eta <= 1.0, "ruze_efficiency = {eta} outside (0,1]");
    }

    /// Over the *whole* non-negative domain, Ruze efficiency can never exceed 1 or
    /// go negative. (At extreme rms/λ the argument² exceeds f64's exp range and the
    /// value underflows to exactly 0.0 — the closed lower bound here is the honest
    /// one for that physically-meaningless corner.)
    #[test]
    fn ruze_efficiency_stays_in_closed_unit_interval(
        surface_rms in 0.0f64..1.0,
        wavelength in 0.001f64..3.0,
    ) {
        let eta = ruze_efficiency(surface_rms, wavelength);
        prop_assert!((0.0..=1.0).contains(&eta), "ruze_efficiency = {eta} outside [0,1]");
    }

    /// Ruze efficiency is monotone (non-increasing) in surface RMS: a rougher
    /// surface can never scatter *more* coherent gain than a smoother one.
    #[test]
    fn ruze_efficiency_monotone_decreasing_in_surface_rms(
        (rms_lo, wavelength) in ruze_representable(),
        extra in 0.0f64..0.1,
    ) {
        let rms_hi = rms_lo + extra;
        let eta_lo = ruze_efficiency(rms_lo, wavelength);
        let eta_hi = ruze_efficiency(rms_hi, wavelength);
        prop_assert!(
            eta_lo >= eta_hi,
            "ruze not monotone: rms {rms_lo} -> {eta_lo} but rms {rms_hi} -> {eta_hi} (λ={wavelength})"
        );
    }
}
