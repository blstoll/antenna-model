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
//! of these ever fails, investigate before loosening a tolerance. Findings
//! this suite has produced and how they are handled:
//!
//! - **Ruze underflow** — at extreme rms/λ, `exp(-(4πrms/λ)²)` legitimately
//!   underflows to exactly 0.0, so the strict (0, 1] claim is scoped to the
//!   representable regime and a broad [0, 1] test covers the rest.
//! - **`normalize_angle` 2π boundary rounding** — for inputs within ~1 ulp of a
//!   negative-multiple-of-2π wrap, `%` leaves a tiny negative remainder and
//!   `+ 2π` rounds back up to exactly 2π, violating the documented [0, 2π)
//!   contract. Filed, not fixed (per the charter); the strict bound is kept as
//!   the contract a gross regression would violate.
//! - **Aiming at boresight on a steered feed makes a bound property vacuous**
//!   — a lateral feed offset δ steers the beam ≈`0.9·δ/f` rad off-axis, so θ≈0
//!   samples deep sidelobes 60–108 dB under the ideal-aperture bound. The
//!   coma property aims into the steered beam instead, which brings its
//!   measured detection threshold from ~60 dB down to +6 dB.
//! - **The −60 dBi sidelobe nulls are real** — electrically large dishes have
//!   genuine nulls that reach the floor (measured at θ = 21°/32°/42° for a
//!   2.64 m @ 3.29 GHz dish), so the NaN-catching `> MIN_GAIN_FLOOR` assertion
//!   lives only in the main-lobe test whose domain provably clears it.

use antenna_core::model::{
    compute_gain, normalize_angle, normalize_angle_symmetric, ruze_efficiency,
};
use antenna_core::model::{
    ecef_to_enu_rotation, ecef_to_geodetic, geodetic_to_ecef, theoretical_max_gain,
    wavelength_from_frequency, AntennaConfiguration, ApertureCoordinates, EClockConeCoordinates,
    FarFieldCoordinates, FeedParameters, FeedParametersBuilder, FeedPosition, IntegrationParams,
    ReflectorGeometry, MIN_GAIN_FLOOR,
};
use proptest::prelude::*;
use std::f64::consts::PI;

/// Tolerance for coordinate round-trip angle drift. Used for both degree
/// (lon/lat in the ECEF↔Geodetic round-trip) and radian (E-clock/E-cone)
/// comparisons: 1e-6 degrees ≈ 1.7e-8 rad, 1e-6 rad ≈ 0.2 arcsec. It is far
/// looser than the transform errors (Bowring's iteration converges below float
/// noise) and exists to absorb two-way float rounding, not model error.
const ANGLE_TOL: f64 = 1e-6;

/// Builds the antenna half of a gain-case generator: a physically-valid
/// reflector (`surface_rms` derived *as a fraction of λ* so Ruze loss is
/// representable) plus a feed with the given builder tweaks. Returns the
/// config and the frequency that fixes λ.
fn build_gain_case(
    diameter: f64,
    f_over_d: f64,
    freq: f64,
    rms_frac: f64,
    feed_tweak: impl Fn(FeedParametersBuilder) -> FeedParametersBuilder,
) -> (AntennaConfiguration, f64) {
    let wavelength = wavelength_from_frequency(freq);
    let refl = ReflectorGeometry::builder()
        .diameter(diameter)
        .focal_length(diameter * f_over_d)
        .surface_rms(rms_frac * wavelength)
        .build()
        .expect("reflector generated inside its validated domain");
    let feed = feed_tweak(
        FeedParameters::builder()
            .at_focus(refl.focal_length)
            .q_factor(8.0),
    )
    .build()
    .expect("feed generated inside its validated domain");
    let config = AntennaConfiguration::builder()
        .id("prop")
        .name("property-test")
        .reflector(refl)
        .feed(feed)
        .build()
        .expect("config within validated domain");
    (config, freq)
}

/// A physically-valid antenna with the feed at focus (routes the cheap
/// symmetric integrator branch). Frequency is inside the model's validated
/// band [100, 50,000] MHz and diameter is capped so the D/λ-driven aperture
/// sweep stays bounded. The tuple components are drawn independently, so the
/// *joint* worst case is 4 m × 8.4 GHz → D/λ ≈ 112.
fn gain_config() -> impl Strategy<Value = (AntennaConfiguration, f64)> {
    (0.5f64..4.0, 0.2f64..1.0, 100.0e6f64..8.4e9, 0.001f64..0.1).prop_map(
        |(diameter, f_over_d, freq, rms_frac)| {
            build_gain_case(diameter, f_over_d, freq, rms_frac, |b| b)
        },
    )
}

/// A physically-valid antenna whose non-unity `asymmetry_factor` routes the
/// integrator to the **azimuthal-mode (Jₘ) branch**, where P12's radial-budget
/// defect lived (worst measured 7.08 dB): the answer is a residue of mode
/// integrals that cancel 59–111×, so ~1% per-mode error becomes ~10% of the
/// result. Feed kept at focus and sizes kept small so this stays cheap;
/// `asymmetry_factor` is a declared design property (horn geometry), so it is
/// drawn directly rather than tuned.
///
/// This reaches the branch but **not** its φ' sizing path — δ = 0 here, so the
/// bandwidth is the constant `asym_bandwidth = 6.0` and `n_phi` sits at
/// `MODE_PHI_MIN`. See [`steered_config`] for the generator that drives
/// `spread = k·δ·(R/f)`. (Neither screens P10-perf's +82 dB φ' aliasing — that
/// needs the differential check in `integration.rs`; see [`steered_case`].)
fn asymmetric_config() -> impl Strategy<Value = (AntennaConfiguration, f64)> {
    (
        0.5f64..1.5,
        0.2f64..1.0,
        300.0e6f64..2.0e9,
        1.2f64..2.5,
        0.001f64..0.05,
    )
        .prop_map(|(diameter, f_over_d, freq, asymmetry, rms_frac)| {
            build_gain_case(diameter, f_over_d, freq, rms_frac, |b| {
                b.asymmetry_factor(asymmetry)
            })
        })
}

/// A **laterally offset (comaed) feed**, aimed at its own steered beam peak.
/// Yields `(config, frequency, theta)` because the aim point is derived from
/// the drawn `δ/f` — it cannot be an independent component.
///
/// **Why the aim point matters.** A lateral offset δ steers the beam off
/// boresight by ≈ `0.9·δ/f` radians, *away* from the offset, so the peak sits
/// at φ = π when the feed is displaced along +x. Sampling θ near 0 on such a
/// geometry lands in the deep sidelobes — measured 60–108 dB below the
/// ideal-aperture bound, which would make a bound assertion there effectively
/// vacuous. Aiming into the steered beam instead leaves a measured worst-case
/// headroom of **2.87 dB** across this generator's box (corners probed:
/// D ∈ {2.5, 3.0} m × f/D ∈ {0.35, 0.55} × {6.0, 8.4} GHz × δ/f ∈ {0.08, 0.15},
/// tightest at f/D = 0.55).
///
/// f/D is capped at 0.55 because a longer focal length means less coma loss and
/// a higher peak: past that the headroom closes on the assertion's own 0.49 dB
/// slack and the property starts failing on correct physics.
fn steered_case() -> impl Strategy<Value = (AntennaConfiguration, f64, f64)> {
    (
        2.5f64..3.0,
        0.35f64..0.55,
        6.0e9f64..8.4e9,
        0.08f64..0.15,
        0.001f64..0.02,
        0.75f64..1.05,
    )
        .prop_map(
            |(diameter, f_over_d, freq, delta_ratio, rms_frac, beam_factor)| {
                let focal_length = diameter * f_over_d;
                let (config, freq) =
                    build_gain_case(diameter, f_over_d, freq, rms_frac, move |b| {
                        // Vertex-origin position: lateral x = δ, axially at the focus.
                        b.position(FeedPosition::new(
                            delta_ratio * focal_length,
                            0.0,
                            focal_length,
                        ))
                    });
                // Bracket the steered peak (measured at ≈0.9·δ/f rad).
                (config, freq, beam_factor * delta_ratio)
            },
        )
}

/// A small, low-frequency antenna (feed at focus or asymmetric) whose main
/// lobe is broad enough that *every* drawn angle provably clears
/// [`MIN_GAIN_FLOOR`]: D ∈ [0.5, 1.0] m and f ∈ [100, 800] MHz put the first
/// null beyond ~26° even at the top of the band, so θ ≤ 15° stays inside the
/// main lobe where the gain is ≥ ~−20 dBi — four orders above the −60 dBi
/// floor. This is the only domain where the *floor-clearance* assertion is
/// honest: at deep sidelobe nulls a genuine −60 dBi value and a NaN collapsed
/// by `apply_gain_floor` are numerically identical, so finiteness is only
/// detectable where the physics cannot legitimately sit at the floor.
fn main_lobe_case() -> impl Strategy<Value = (AntennaConfiguration, f64)> {
    prop_oneof![
        (0.5f64..1.0, 0.2f64..1.0, 100.0e6f64..800.0e6, 0.001f64..0.1).prop_map(
            |(diameter, f_over_d, freq, rms_frac)| {
                build_gain_case(diameter, f_over_d, freq, rms_frac, |b| b)
            }
        ),
        (
            0.5f64..1.0,
            0.2f64..1.0,
            300.0e6f64..800.0e6,
            1.2f64..2.5,
            0.001f64..0.1
        )
            .prop_map(|(diameter, f_over_d, freq, asymmetry, rms_frac)| {
                build_gain_case(diameter, f_over_d, freq, rms_frac, |b| {
                    b.asymmetry_factor(asymmetry)
                })
            })
    ]
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
        alt_m in -1000.0f64..400_000_000.0,
    ) {
        let (x, y, z) = geodetic_to_ecef(lon_deg, lat_deg, alt_m).unwrap();
        let (lon2, lat2, alt2) = ecef_to_geodetic(x, y, z).unwrap();
        // Bowring's iteration converges below float noise; the tolerance absorbs
        // two-way float rounding of the degree values.
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
    fn ecef_geodetic_roundtrip_at_poles(
        lat_deg in prop_oneof![89.995f64..=90.0, -90.0f64..=-89.995],
        alt_m in -1000.0f64..400_000_000.0,
    ) {
        // The polar cap is where `ecef_to_geodetic` switches to the z-based
        // altitude branch (`cos_lat ≤ 1e-4`, i.e. |lat| ≥ 89.99427°); uniform
        // latitude draws hit it with probability ~6e-5, so this test samples it
        // directly. The ±89.995° bound sits inside the threshold — cos(89.995°)
        // = 8.7e-5 — so *every* draw takes the branch, rather than the ~57% a
        // range of ±89.99° would have given.
        // Longitude is degenerate at the exact pole and is not asserted.
        let (x, y, z) = geodetic_to_ecef(123.456, lat_deg, alt_m).unwrap();
        let (_, lat2, alt2) = ecef_to_geodetic(x, y, z).unwrap();
        prop_assert!((lat2 - lat_deg).abs() < ANGLE_TOL, "latitude drifted {lat_deg} -> {lat2}");
        prop_assert!(
            (alt2 - alt_m).abs() < 1e-3 + 1e-9 * alt_m.abs(),
            "altitude drifted {alt_m} -> {alt2}"
        );
    }

    #[test]
    fn enu_rotation_is_orthogonal_and_right_handed(
        lat_rad in -PI / 2.0f64..PI / 2.0,
        lon_rad in -PI..PI,
    ) {
        // R·Rᵀ must be the identity for every lat/lon ...
        let r = ecef_to_enu_rotation(lat_rad, lon_rad);
        for (i, row) in r.iter().enumerate() {
            for (j, col) in r.iter().enumerate() {
                let dot: f64 = row.iter().zip(col.iter()).map(|(a, b)| a * b).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                prop_assert!(
                    (dot - expected).abs() < 1e-9,
                    "R·Rᵀ not identity at [{i}][{j}] (lat={lat_rad}, lon={lon_rad}): {dot}"
                );
            }
        }
        // ... and it must be a *proper* rotation: det(R) = +1. Orthogonality
        // alone admits a sign-flipped ENU basis (det = −1), the "ENU axis
        // direction" gotcha the domain contract warns about. (A cyclic row
        // permutation keeps det = +1 and is pinned by the anchored test below.)
        let det = r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0]);
        prop_assert!((det - 1.0).abs() < 1e-9, "det(R) = {det}, expected +1 (lat={lat_rad})");
    }

    #[test]
    fn normalize_angle_lands_in_bounds(angle in -1000.0f64..1000.0) {
        let n = normalize_angle(angle);
        // Documented contract is [0, 2π). Known 1-ulp corner (filed in the D7
        // closeout, not fixed here per the charter): for inputs within ~4.4e-16
        // of a multiple of 2π from *below*, `%` leaves a tiny negative
        // remainder and `+ 2π` rounds back up to exactly 2π, e.g.
        // `normalize_angle(-1e-17) == 2π`. Uniform draws hit that window with
        // probability ~1e-14 per draw, so the strict bound below is what the
        // function is contracted to and what a gross regression would violate.
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
        prop_assert!(
            (-PI..PI).contains(&n),
            "normalize_angle_symmetric({angle}) = {n} out of [-π,π)"
        );
        let turns = (angle - n) / (2.0 * PI);
        prop_assert!((turns - turns.round()).abs() < 1e-6, "{angle} not congruent to {n} mod 2π");
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

// ==== Physics bounds =====================================================
// The aperture integral is O(D/λ) and the hot off-axis branch is the
// expensive one, so the two gain properties run their own capped batches.

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Gain is finite and never exceeds the ideal-aperture bound (uniform
    /// illumination, 100% aperture efficiency) for any valid input on the
    /// symmetric branch.
    ///
    /// The bound is guaranteed by Cauchy–Schwarz: `|∫A e^{jψ} dA|² ≤ ∫|A|² dA ·
    /// area`, so `D(θ,φ) = (4π/λ²)·|I|²/∫|A|² ≤ (4π/λ²)·area = G_ideal`, and
    /// taper (≤ 1), obliquity (≤ 1) and Ruze/mesh efficiency (≤ 1) only lower
    /// it further. A 0.5 dB slack absorbs floating-point and quadrature slop
    /// while still catching the class of aliasing/overshoot bugs (the historic
    /// +20 to +82 dB) D7 is chartered to screen for.
    ///
    /// **This is the suite's tightest bound, and its power sits in one corner.**
    /// Measured detection threshold is +1.0 dB (it misses +0.5 dB), which is
    /// better than the other three bound properties by 1–5 dB — but only because
    /// `f_over_d` reaches 1.0. Boresight headroom against the bound, measured at
    /// D = 0.5 m / 100 MHz with the shipped q = 8 feed:
    ///
    /// | f/D | 0.2 | 0.4 | 0.6 | 0.8 | 1.0 |
    /// |---|---|---|---|---|---|
    /// | margin | −11.26 | −5.25 | −2.15 | −0.86 | **−0.38** dB |
    ///
    /// A long focal length with a fixed feed taper approaches uniform
    /// illumination, so aperture efficiency approaches 1 and the Cauchy–Schwarz
    /// bound becomes nearly exact. It is scale-invariant — the same −4.83 dB at
    /// f/D = 0.42 for every D/λ from 0.17 to 26.7 — so **narrowing `f_over_d`
    /// away from 1.0 would silently cost this property most of its power**
    /// without failing anything. Do not narrow it without re-measuring the
    /// table above.
    ///
    /// **Why this test cannot also assert `gain > MIN_GAIN_FLOOR`:** the
    /// electrically large dishes here (up to 28.9λ) have genuine sidelobe nulls
    /// below −60 dBi — measured dips to exactly the floor at θ = 21°/32°/42°
    /// for a 2.64 m @ 3.29 GHz dish, with a smooth continuous pattern — so a
    /// value pinned at `MIN_GAIN_FLOOR` is *not* proof of a NaN there.
    /// `gain.is_finite()` still catches +Inf, the one non-finite value the
    /// floor lets through; the NaN/−Inf signature is asserted in the
    /// main-lobe test, whose domain provably clears the floor.
    #[test]
    fn gain_is_finite_and_bounded_by_ideal_aperture(
        (config, freq) in gain_config(),
        theta_deg in 0.0f64..45.0,
        phi in 0.0f64..(2.0 * PI),
    ) {
        let theta = theta_deg.to_radians();
        let result = compute_gain(theta, phi, &config, freq, &IntegrationParams::default());
        prop_assert!(result.is_ok(), "compute_gain failed: {:?}", result.err());
        let gain = result.unwrap().gain;
        prop_assert!(gain.is_finite(), "gain is not finite: {gain}");
        let wavelength = wavelength_from_frequency(freq);
        let bound = theoretical_max_gain(config.reflector.diameter, wavelength, 1.0);
        prop_assert!(
            gain <= bound * 1.12 + 1e-9,
            "gain {gain} exceeds ideal-aperture bound {bound} (D={}m, f={}Hz, θ={theta} rad)",
            config.reflector.diameter,
            freq
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]

    /// The same physics bounds on the **azimuthal-mode (Jₘ) branch**: an
    /// asymmetric-illumination feed (`asymmetry_factor != 1.0`) routes
    /// `integrate_aperture` off the symmetric J₀ path (`is_symmetric` requires
    /// `asymmetry_factor == 1.0`) and into the mode expansion, whose per-mode
    /// errors cancel 59–111× and so used to turn ~1% per-mode error into
    /// silently wrong totals — P12's radial-budget defect, worst measured
    /// 7.08 dB. This test screens **that** defect class.
    ///
    /// It does **not** screen the φ' sizing path: with the feed at focus δ = 0,
    /// so `coma_bandwidth = 0` and `mode_count_for` uses the constant
    /// `asym_bandwidth = 6.0`, pinning `n_phi` at `MODE_PHI_MIN`. P10-perf's
    /// +82 dB aliasing lived in `spread = k·δ·(R/f)`, which is identically zero
    /// here — and [`steered_beam_gain_is_bounded_by_ideal_aperture`] does not
    /// cover it either; see its docs for the measured reason and the real guard.
    ///
    /// The bound proof is unchanged — the same Cauchy–Schwarz argument holds on
    /// any branch — and, as in the symmetric test, the floor-clearance claim is
    /// left to the main-lobe test because this angular range reaches genuine
    /// nulls below the floor.
    #[test]
    fn mode_branch_gain_is_finite_and_bounded_by_ideal_aperture(
        (config, freq) in asymmetric_config(),
        theta_deg in 0.0f64..30.0,
        phi in 0.0f64..(2.0 * PI),
    ) {
        let theta = theta_deg.to_radians();
        let result = compute_gain(theta, phi, &config, freq, &IntegrationParams::default());
        prop_assert!(result.is_ok(), "compute_gain failed: {:?}", result.err());
        let gain = result.unwrap().gain;
        prop_assert!(gain.is_finite(), "gain is not finite: {gain}");
        let wavelength = wavelength_from_frequency(freq);
        let bound = theoretical_max_gain(config.reflector.diameter, wavelength, 1.0);
        prop_assert!(
            gain <= bound * 1.12 + 1e-9,
            "gain {gain} exceeds ideal-aperture bound {bound} (D={}m, f={}Hz, θ={theta} rad)",
            config.reflector.diameter,
            freq
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// The NaN/−Inf catcher. `compute_gain` unconditionally ends in
    /// `apply_gain_floor` (edge_cases.rs), which clamps to
    /// `MIN_GAIN_FLOOR = 1e-6` via `f64::max` — and `f64::max` ignores NaN, so
    /// a NaN or −Inf out of the integrator comes back as *exactly* 1e-6, while
    /// only +Inf survives to fail `is_finite`. The only place through the
    /// public API where a floored value is unambiguous evidence of a non-finite
    /// computation is where genuine physics provably clears the floor: the
    /// broad-beamed small dishes of [`main_lobe_case`] at θ ≤ 15° (gain ≥
    /// ~−20 dBi, four orders above the floor). NaN/−Inf collapse lands at
    /// exactly 1e-6 and fails the strict `>`. This domain also covers both
    /// integrator branches via `prop_oneof!`.
    #[test]
    fn main_lobe_gain_clears_the_numerical_floor(
        (config, freq) in main_lobe_case(),
        theta_deg in 0.0f64..15.0,
        phi in 0.0f64..(2.0 * PI),
    ) {
        let theta = theta_deg.to_radians();
        let result = compute_gain(theta, phi, &config, freq, &IntegrationParams::default());
        prop_assert!(result.is_ok(), "compute_gain failed: {:?}", result.err());
        let gain = result.unwrap().gain;
        prop_assert!(gain.is_finite(), "gain is not finite: {gain}");
        prop_assert!(
            gain > MIN_GAIN_FLOOR,
            "gain pinned at the numerical floor ({MIN_GAIN_FLOOR}): NaN/−Inf collapse lands here"
        );
        // The bound must hold here too — the main lobe still obeys it, and a
        // mode-branch overshoot at a moderate angle is caught even at small size.
        let wavelength = wavelength_from_frequency(freq);
        let bound = theoretical_max_gain(config.reflector.diameter, wavelength, 1.0);
        prop_assert!(
            gain <= bound * 1.12 + 1e-9,
            "gain {gain} exceeds ideal-aperture bound {bound} (D={}m, f={}Hz, θ={theta} rad)",
            config.reflector.diameter,
            freq
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// The ideal-aperture bound on a **comaed** geometry (δ ≠ 0), evaluated at
    /// the geometry's own steered beam peak. Its reason to exist is coverage,
    /// not tightness: it is the only property here that exercises the
    /// lateral-offset phase path at all (`gain_config` and `asymmetric_config`
    /// both sit the feed at focus, so `phase_feed_displacement` sees δ = 0).
    ///
    /// **Measured power.** Injecting a uniform gain multiplier into
    /// `apply_gain_floor` and re-running the suite gives each bound property's
    /// detection threshold:
    ///
    /// | property | catches | misses |
    /// |---|---|---|
    /// | `gain_is_finite_and_bounded_by_ideal_aperture` | +1.0 dB | — |
    /// | `main_lobe_gain_clears_the_numerical_floor` | +2.0 dB | +1.0 dB |
    /// | `mode_branch_gain_is_finite_and_bounded_by_ideal_aperture` | +3.0 dB | +2.0 dB |
    /// | this one | +6.0 dB | +4.0 dB |
    ///
    /// So this is the **loosest** of the four — 2.87 dB of physical headroom
    /// plus 0.49 dB of assertion slack sets the floor, and only the worst corner
    /// of the box is that tight. Do not cite it as a tight bound; cite it as the
    /// coma-path one. All four remain comfortably inside the `+20…+82 dB`
    /// aliasing class D7 is chartered against.
    ///
    /// **What this does NOT screen — measured, not assumed.** It is not a guard
    /// on the φ' sampling axis. Reintroducing the retired `MODE_PHI_STEERED_MAX`
    /// (clamp `n_phi ≤ 64` when `δ/f > 0.05`) as a negative control moved these
    /// geometries by **≤ 0.09 dB**, nowhere near the bound. Two reasons, and the
    /// second is structural: the documented +28.67 dB error needed `δ/f = 0.4`
    /// (bandwidth ≈ 106, `n_phi ≥ 256`), far past the 0.15 this generator can
    /// reach before coma loss opens the headroom back up; and φ' aliasing
    /// inflates *sidelobes*, which start 40–100 dB below the ideal-aperture
    /// bound, so Cauchy–Schwarz structurally cannot see it. That axis is guarded
    /// by `integration::…::served_n_phi_sizing_is_sufficient_on_every_asymmetric_geometry`,
    /// which compares the served `n_phi` against a 2× denser grid with the
    /// radial density held fixed — a differential check that needs crate-private
    /// `mode_count_for`, and so cannot be reproduced from an integration test.
    #[test]
    fn steered_beam_gain_is_bounded_by_ideal_aperture(
        (config, freq, theta) in steered_case(),
    ) {
        // The beam steers *opposite* the +x feed offset, so its peak is at φ = π.
        let result = compute_gain(theta, PI, &config, freq, &IntegrationParams::default());
        prop_assert!(result.is_ok(), "compute_gain failed: {:?}", result.err());
        let gain = result.unwrap().gain;
        prop_assert!(gain.is_finite(), "gain is not finite: {gain}");
        let wavelength = wavelength_from_frequency(freq);
        let bound = theoretical_max_gain(config.reflector.diameter, wavelength, 1.0);
        prop_assert!(
            gain <= bound * 1.12 + 1e-9,
            "gain {:.2} dBi exceeds ideal-aperture bound {:.2} dBi (D={} m, f={} Hz, \
             δ/f={:.3}, θ={:.3}°)",
            10.0 * gain.log10(),
            10.0 * bound.log10(),
            config.reflector.diameter,
            freq,
            config.feed.position.radial_displacement() / config.reflector.focal_length,
            theta.to_degrees()
        );
    }
}

/// At the equator on the prime meridian, ECEF +X points through the observer,
/// so the ENU basis must be East=+Y, North=+Z, Up=+X. This pins the *cyclic
/// orientation* of the basis, which `R·Rᵀ = I` and `det(R) = +1` cannot: a
/// cyclic row permutation (East/North/Up relabelled) satisfies both while
/// pointing the wrong way — the "ENU axis direction" gotcha the domain
/// contract warns about.
#[test]
fn enu_basis_orientation_is_anchored() {
    let r = ecef_to_enu_rotation(0.0, 0.0);
    assert_row_near(&r[0], [0.0, 1.0, 0.0], "East");
    assert_row_near(&r[1], [0.0, 0.0, 1.0], "North");
    assert_row_near(&r[2], [1.0, 0.0, 0.0], "Up");
}

fn assert_row_near(row: &[f64; 3], expected: [f64; 3], name: &str) {
    for (a, b) in row.iter().zip(expected.iter()) {
        assert!(
            (a - b).abs() < 1e-12,
            "ENU {name} row {row:?} != expected {expected:?} at lat=0, lon=0"
        );
    }
}
