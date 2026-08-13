//! Coordinate System Transformations
//!
//! This module provides coordinate transformations between different reference frames
//! used in antenna modeling:
//!
//! - **Aperture coordinates** (ρ, φ'): Polar coordinates in the reflector aperture plane
//! - **Far-field coordinates** (θ, φ): Spherical coordinates for radiation patterns
//! - **E-clock/E-cone**: Pointing coordinates used in antenna control systems
//! - **Cartesian coordinates**: Standard (x, y, z) for feed positions
//!
//! # Coordinate System Conventions
//!
//! ## Aperture Coordinates (ρ, φ')
//! - ρ: Radial distance from reflector axis (0 to aperture_radius)
//! - φ': Azimuthal angle in aperture plane (0 to 2π)
//!
//! ## Far-field Coordinates (θ, φ)
//! - θ: **Polar angle from boresight** (0 at boresight, π/2 perpendicular to boresight)
//! - φ: Azimuthal angle (0 to 2π)
//! - **NOTE**: θ is NOT horizon-based elevation (which would be 0° at horizon, 90° at zenith)
//!
//! ## E-clock/E-cone Coordinates
//! - **E-cone**: Cone angle from boresight = polar angle θ (0° at boresight)
//! - **E-clock**: Clock angle around boresight = azimuthal angle φ (0 to 2π)
//! - Compatible with far-field coordinates: E-cone = θ, E-clock = φ
//!
//! ## Cartesian Coordinates (x, y, z)
//! - Origin at reflector vertex
//! - z-axis along reflector axis (positive toward reflector, boresight direction)
//! - x, y axes define aperture plane
//! - Feed at (0, 0, f) is at focal point (f = focal length)
//!
//! # Critical Convention Note (Fixed 2025-11-19)
//!
//! **All angular coordinates in this module use POLAR ANGLE from boresight**, where:
//! - 0° = boresight (perfect alignment)
//! - 90° = perpendicular to boresight
//!
//! This is the physics/math convention for spherical coordinates and is used consistently
//! throughout the codebase. It is **NOT** horizon-based elevation (0° = horizon, 90° = zenith).

use std::f64::consts::PI;

/// Aperture coordinates in polar form
///
/// Used for integration over the reflector aperture
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApertureCoordinates {
    /// Radial distance from axis (meters)
    pub rho: f64,
    /// Azimuthal angle (radians, 0 to 2π)
    pub phi_prime: f64,
}

impl ApertureCoordinates {
    /// Create new aperture coordinates
    pub fn new(rho: f64, phi_prime: f64) -> Self {
        Self { rho, phi_prime }
    }

    /// Convert to Cartesian coordinates in aperture plane
    pub fn to_cartesian(&self) -> (f64, f64) {
        let x = self.rho * self.phi_prime.cos();
        let y = self.rho * self.phi_prime.sin();
        (x, y)
    }

    /// Create from Cartesian coordinates in aperture plane
    pub fn from_cartesian(x: f64, y: f64) -> Self {
        let rho = (x * x + y * y).sqrt();
        let phi_prime = y.atan2(x);
        Self::new(rho, phi_prime)
    }
}

/// Far-field spherical coordinates
///
/// Used for antenna radiation patterns
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FarFieldCoordinates {
    /// Polar angle from boresight (radians, 0 to π)
    pub theta: f64,
    /// Azimuthal angle (radians, 0 to 2π)
    pub phi: f64,
}

impl FarFieldCoordinates {
    /// Create new far-field coordinates
    pub fn new(theta: f64, phi: f64) -> Self {
        Self { theta, phi }
    }

    /// Convert to unit direction vector in Cartesian coordinates
    pub fn to_direction_vector(&self) -> (f64, f64, f64) {
        let x = self.theta.sin() * self.phi.cos();
        let y = self.theta.sin() * self.phi.sin();
        let z = self.theta.cos();
        (x, y, z)
    }

    /// Create from Cartesian direction vector (assumes normalized)
    pub fn from_direction_vector(x: f64, y: f64, z: f64) -> Self {
        let theta = z.acos();
        let phi = y.atan2(x);
        Self::new(theta, phi)
    }
}

/// E-clock/E-cone coordinates
///
/// Pointing coordinates commonly used in antenna control systems.
/// E-cone is the angular distance from boresight, E-clock is the
/// rotation angle around the boresight axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EClockConeCoordinates {
    /// Cone angle from boresight (radians, 0 to π)
    pub e_cone: f64,
    /// Clock angle around boresight (radians, 0 to 2π)
    pub e_clock: f64,
}

impl EClockConeCoordinates {
    /// Create new E-clock/E-cone coordinates
    pub fn new(e_cone: f64, e_clock: f64) -> Self {
        Self { e_cone, e_clock }
    }

    /// Create from degrees (convenience function)
    pub fn from_degrees(e_cone_deg: f64, e_clock_deg: f64) -> Self {
        Self {
            e_cone: e_cone_deg.to_radians(),
            e_clock: e_clock_deg.to_radians(),
        }
    }

    /// Convert to degrees (convenience function)
    pub fn to_degrees(&self) -> (f64, f64) {
        (self.e_cone.to_degrees(), self.e_clock.to_degrees())
    }

    /// Convert to far-field coordinates
    ///
    /// E-clock/E-cone is essentially the same as (θ, φ) far-field coordinates
    pub fn to_far_field(&self) -> FarFieldCoordinates {
        FarFieldCoordinates::new(self.e_cone, self.e_clock)
    }

    /// Create from far-field coordinates
    pub fn from_far_field(far_field: FarFieldCoordinates) -> Self {
        Self::new(far_field.theta, far_field.phi)
    }

    /// Create from azimuth/elevation coordinates (degrees).
    ///
    /// Converts standard antenna pointing coordinates (azimuth, elevation) to
    /// E-cone/E-clock coordinates.
    ///
    /// **CORRECTED Coordinate Convention:**
    /// - Boresight is at azimuth=0°, elevation=0°
    /// - Azimuth: 0° = +X axis, 90° = +Y axis (antenna frame)
    /// - **Elevation: POLAR ANGLE from boresight (+Z axis)**
    ///   - 0° = boresight (along +Z axis)
    ///   - 90° = perpendicular to boresight (in XY plane)
    ///
    /// This matches the output convention of `antenna_frame_to_spherical()`.
    ///
    /// **Conversion:**
    /// For spherical coordinates with boresight at origin:
    /// - E-cone = elevation (polar angle from boresight)
    /// - E-clock = azimuth (rotation angle around boresight)
    ///
    /// # Arguments
    /// - `azimuth_deg`: Azimuth angle in degrees
    /// - `elevation_deg`: Elevation angle in degrees (POLAR ANGLE from boresight)
    ///
    /// # Returns
    /// E-cone/E-clock coordinates with angles in radians
    pub fn from_azimuth_elevation(azimuth_deg: f64, elevation_deg: f64) -> Self {
        let az_rad = azimuth_deg.to_radians();
        let el_rad = elevation_deg.to_radians();

        // For spherical coordinates with boresight at origin:
        // E-cone is the polar angle from boresight (elevation)
        // E-clock is the azimuthal angle around boresight
        let e_cone = el_rad;
        let e_clock = az_rad;

        Self::new(e_cone, e_clock)
    }

    /// `to_feed_displacement` with an explicit beam deviation factor.
    ///
    /// This is the key transformation from pointing angles to physical feed position.
    /// A lateral feed offset steers the beam to the OPPOSITE side of boresight
    /// (beam deviation): to point the beam at clock angle φ, the feed must be
    /// displaced at clock angle φ+π. Hence the negated x/y components below.
    ///
    /// Additionally, the physical-optics beam peak deviates by only BDF·ψ for a
    /// feed displaced by angle ψ (see `beam_deviation_factor` in
    /// `coordinates_3d.rs`). Dividing the displacement by `bdf` corrects for
    /// this so the beam lands at `e_cone` rather than `BDF·e_cone`. Pass `1.0`
    /// to reproduce the geometric (no-BDF) mapping.
    /// ```text
    /// displacement = 2·f·tan(cone_angle/2) / bdf
    /// x_feed = -displacement·cos(clock_angle)
    /// y_feed = -displacement·sin(clock_angle)
    /// z_feed = -displacement²/(4f)   (defocus term, keeps feed near focal surface)
    /// ```
    ///
    /// # Arguments
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    /// - `bdf`: Beam deviation factor (see `beam_deviation_factor`); use `1.0` for
    ///   the uncorrected geometric mapping
    ///
    /// # Returns
    /// Feed position (x, y, z) in Cartesian coordinates relative to focal point
    pub fn to_feed_displacement_with_bdf(&self, focal_length: f64, bdf: f64) -> (f64, f64, f64) {
        // Radial displacement in xy-plane, corrected for beam deviation
        let displacement = 2.0 * focal_length * (self.e_cone / 2.0).tan() / bdf;

        // Cartesian components — NEGATED: beam deviation puts the feed on the
        // side opposite the desired beam direction.
        let x_feed = -displacement * self.e_clock.cos();
        let y_feed = -displacement * self.e_clock.sin();

        // For large displacements, include z-component (defocus)
        let z_feed = -displacement * displacement / (4.0 * focal_length);

        (x_feed, y_feed, z_feed)
    }

    /// Calculate feed displacement from focal point (geometric mapping, BDF=1).
    ///
    /// See `to_feed_displacement_with_bdf` for the beam-deviation-corrected variant.
    pub fn to_feed_displacement(&self, focal_length: f64) -> (f64, f64, f64) {
        self.to_feed_displacement_with_bdf(focal_length, 1.0)
    }

    /// `to_feed_position` with an explicit beam deviation factor (see
    /// `to_feed_displacement_with_bdf`).
    ///
    /// Returns absolute feed position (not relative to focal point)
    ///
    /// # Arguments
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    /// - `bdf`: Beam deviation factor; use `1.0` for the uncorrected geometric mapping
    ///
    /// # Returns
    /// Feed position (x, y, z) with origin at reflector vertex
    pub fn to_feed_position_with_bdf(&self, focal_length: f64, bdf: f64) -> (f64, f64, f64) {
        let (dx, dy, dz) = self.to_feed_displacement_with_bdf(focal_length, bdf);
        (dx, dy, focal_length + dz)
    }

    /// Calculate feed position from E-clock/E-cone (geometric mapping, BDF=1).
    ///
    /// Returns absolute feed position (not relative to focal point)
    ///
    /// # Arguments
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    ///
    /// # Returns
    /// Feed position (x, y, z) with origin at reflector vertex
    pub fn to_feed_position(&self, focal_length: f64) -> (f64, f64, f64) {
        self.to_feed_position_with_bdf(focal_length, 1.0)
    }

    /// Calculate E-clock/E-cone from feed position
    ///
    /// Inverse of `to_feed_position()`. Given a feed position in Cartesian coordinates,
    /// calculate the corresponding E-clock/E-cone pointing angles.
    ///
    /// # Arguments
    /// - `x`, `y`, `z`: Feed position in Cartesian coordinates (meters)
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    ///
    /// # Returns
    /// E-clock/E-cone coordinates
    pub fn from_feed_position(x: f64, y: f64, z: f64, focal_length: f64) -> Self {
        // Calculate displacement from focal point
        let _dz = z - focal_length;

        // For small displacements, ignore z-component
        // For large displacements, use full formula
        let radial_displacement = (x * x + y * y).sqrt();

        // Solve for cone angle from displacement formula:
        // displacement = 2·f·tan(cone/2)
        // cone = 2·atan(displacement / (2f))
        let e_cone = 2.0 * (radial_displacement / (2.0 * focal_length)).atan();

        // Clock angle: the feed sits opposite the beam direction, so the beam's
        // clock angle is the direction from the feed BACK through the axis.
        let e_clock = (-y).atan2(-x);

        Self::new(e_cone, e_clock)
    }
}

/// Normalize angle to [0, 2π) range
pub fn normalize_angle(angle: f64) -> f64 {
    let mut normalized = angle % (2.0 * PI);
    if normalized < 0.0 {
        normalized += 2.0 * PI;
    }
    normalized
}

/// Normalize angle to [-π, π) range
pub fn normalize_angle_symmetric(angle: f64) -> f64 {
    let mut normalized = angle % (2.0 * PI);
    if normalized >= PI {
        normalized -= 2.0 * PI;
    } else if normalized < -PI {
        normalized += 2.0 * PI;
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const EPSILON: f64 = 1e-10;

    #[test]
    fn test_aperture_coordinates_cartesian_conversion() {
        let aperture = ApertureCoordinates::new(5.0, PI / 4.0);
        let (x, y) = aperture.to_cartesian();

        assert!((x - 5.0 * (PI / 4.0).cos()).abs() < EPSILON);
        assert!((y - 5.0 * (PI / 4.0).sin()).abs() < EPSILON);

        // Round-trip conversion
        let aperture2 = ApertureCoordinates::from_cartesian(x, y);
        assert!((aperture2.rho - aperture.rho).abs() < EPSILON);
        assert!((aperture2.phi_prime - aperture.phi_prime).abs() < EPSILON);
    }

    #[test]
    fn test_far_field_direction_vector() {
        // Boresight (θ=0) should point along +z
        let ff = FarFieldCoordinates::new(0.0, 0.0);
        let (x, y, z) = ff.to_direction_vector();
        assert!(x.abs() < EPSILON);
        assert!(y.abs() < EPSILON);
        assert!((z - 1.0).abs() < EPSILON);

        // θ=π/2, φ=0 should point along +x
        let ff = FarFieldCoordinates::new(PI / 2.0, 0.0);
        let (x, y, z) = ff.to_direction_vector();
        assert!((x - 1.0).abs() < EPSILON);
        assert!(y.abs() < EPSILON);
        assert!(z.abs() < EPSILON);
    }

    #[test]
    fn test_e_clock_cone_to_far_field() {
        let ecc = EClockConeCoordinates::new(PI / 4.0, PI / 3.0);
        let ff = ecc.to_far_field();

        assert!((ff.theta - PI / 4.0).abs() < EPSILON);
        assert!((ff.phi - PI / 3.0).abs() < EPSILON);
    }

    #[test]
    fn test_e_clock_cone_feed_displacement_on_axis() {
        let focal_length = 17.0;

        // On-axis (e_cone = 0) should give zero displacement
        let ecc = EClockConeCoordinates::new(0.0, 0.0);
        let (x, y, z) = ecc.to_feed_displacement(focal_length);

        assert!(x.abs() < EPSILON);
        assert!(y.abs() < EPSILON);
        assert!(z.abs() < EPSILON);
    }

    #[test]
    fn test_e_clock_cone_feed_displacement_small_angle() {
        let focal_length = 17.0;
        let e_cone = 0.1; // Small angle (~5.7 degrees)
        let e_clock = 0.0;

        let ecc = EClockConeCoordinates::new(e_cone, e_clock);
        let (x, y, z) = ecc.to_feed_displacement(focal_length);

        // Feed is displaced OPPOSITE the aim direction (beam deviation)
        let expected_displacement = 2.0 * focal_length * (e_cone / 2.0).tan();
        assert!((x + expected_displacement).abs() < 0.01);
        assert!(y.abs() < EPSILON);

        // z-component should be small for small displacements
        assert!(z.abs() < 0.1);
    }

    #[test]
    fn test_e_clock_cone_feed_position() {
        let focal_length = 17.0;
        let e_cone = 0.1;
        let e_clock = PI / 4.0;

        let ecc = EClockConeCoordinates::new(e_cone, e_clock);
        let (x, y, z) = ecc.to_feed_position(focal_length);

        // z should be close to focal_length for small displacements
        assert!((z - focal_length).abs() < 0.1);

        // x, y should reflect the clock angle, on the OPPOSITE side of the axis
        let _radial = (x * x + y * y).sqrt();
        let angle = (-y).atan2(-x);
        assert!((angle - e_clock).abs() < EPSILON);
    }

    #[test]
    fn test_e_clock_cone_roundtrip() {
        let focal_length = 17.0;

        // Test various angles
        let test_cases = vec![
            (0.0, 0.0),       // On-axis
            (0.05, 0.0),      // Small offset, 0 degrees
            (0.05, PI / 2.0), // Small offset, 90 degrees
            (0.1, PI / 4.0),  // Moderate offset, 45 degrees
            (0.2, PI),        // Larger offset, 180 degrees
        ];

        for (e_cone, e_clock) in test_cases {
            let ecc1 = EClockConeCoordinates::new(e_cone, e_clock);
            let (x, y, z) = ecc1.to_feed_position(focal_length);
            let ecc2 = EClockConeCoordinates::from_feed_position(x, y, z, focal_length);

            assert!(
                (ecc2.e_cone - ecc1.e_cone).abs() < 1e-6,
                "Cone angle mismatch: {} vs {}",
                ecc1.e_cone,
                ecc2.e_cone
            );

            // Clock angle can wrap around, so normalize before comparing
            let clock_diff = normalize_angle_symmetric(ecc2.e_clock - ecc1.e_clock);
            assert!(
                clock_diff.abs() < 1e-6,
                "Clock angle mismatch: {} vs {}",
                ecc1.e_clock,
                ecc2.e_clock
            );
        }
    }

    #[test]
    fn test_normalize_angle() {
        assert!((normalize_angle(0.0) - 0.0).abs() < EPSILON);
        assert!((normalize_angle(PI) - PI).abs() < EPSILON);
        assert!((normalize_angle(2.0 * PI) - 0.0).abs() < EPSILON);
        assert!((normalize_angle(-PI) - PI).abs() < EPSILON);
        assert!((normalize_angle(3.0 * PI) - PI).abs() < EPSILON);
    }

    #[test]
    fn test_normalize_angle_symmetric() {
        assert!((normalize_angle_symmetric(0.0) - 0.0).abs() < EPSILON);
        assert!((normalize_angle_symmetric(PI / 2.0) - PI / 2.0).abs() < EPSILON);
        assert!((normalize_angle_symmetric(-PI / 2.0) + PI / 2.0).abs() < EPSILON);
        assert!(
            (normalize_angle_symmetric(PI) - PI).abs() < EPSILON
                || (normalize_angle_symmetric(PI) + PI).abs() < EPSILON
        );
    }

    #[test]
    fn test_feed_displacement_formula() {
        // Verify the formula: displacement = 2·f·tan(cone/2)
        let focal_length = 17.0;
        let cone_angles = vec![0.05, 0.1, 0.2, 0.3];

        for cone in cone_angles {
            let ecc = EClockConeCoordinates::new(cone, 0.0);
            let (x, y, _z) = ecc.to_feed_displacement(focal_length);
            let displacement = (x * x + y * y).sqrt();
            let expected = 2.0 * focal_length * (cone / 2.0).tan();

            assert!(
                (displacement - expected).abs() < 1e-10,
                "Displacement formula mismatch at cone={}: {} vs {}",
                cone,
                displacement,
                expected
            );
        }
    }

    // ========================================================================
    // COMPREHENSIVE COORDINATE CONVERSION TESTS (Added 2025-11-19)
    // ========================================================================

    #[test]
    fn test_from_azimuth_elevation_polar_angle_convention() {
        // Test that from_azimuth_elevation uses polar angle convention
        // (0° = boresight, not horizon)

        // Test 1: Point 5° from boresight at azimuth 0°
        let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 5.0);

        // With polar angle convention, cone angle should be exactly 5°
        assert!(
            (ecc.e_cone - 5.0f64.to_radians()).abs() < 1e-10,
            "E-cone should equal elevation (polar angle): got {:.6} rad, expected {:.6} rad",
            ecc.e_cone,
            5.0f64.to_radians()
        );

        // Clock angle should be 0°
        assert!(
            ecc.e_clock.abs() < 1e-10,
            "E-clock should be 0 for azimuth=0: got {:.6} rad",
            ecc.e_clock
        );
    }

    #[test]
    fn test_from_azimuth_elevation_matches_direct_construction() {
        // Verify that from_azimuth_elevation(az, el) == new(el, az)
        // when elevation is interpreted as polar angle

        let test_cases = vec![
            (0.0, 0.0),    // Boresight
            (0.0, 5.0),    // Pure elevation offset
            (90.0, 0.0),   // Pure azimuth offset
            (45.0, 30.0),  // Diagonal offset
            (180.0, 10.0), // Behind
        ];

        for (az_deg, el_deg) in test_cases {
            let ecc_from_func = EClockConeCoordinates::from_azimuth_elevation(az_deg, el_deg);
            let ecc_direct = EClockConeCoordinates::new(el_deg.to_radians(), az_deg.to_radians());

            assert!(
                (ecc_from_func.e_cone - ecc_direct.e_cone).abs() < 1e-10,
                "E-cone mismatch for (az={}, el={}): got {:.6}, expected {:.6}",
                az_deg,
                el_deg,
                ecc_from_func.e_cone,
                ecc_direct.e_cone
            );

            assert!(
                (ecc_from_func.e_clock - ecc_direct.e_clock).abs() < 1e-10,
                "E-clock mismatch for (az={}, el={}): got {:.6}, expected {:.6}",
                az_deg,
                el_deg,
                ecc_from_func.e_clock,
                ecc_direct.e_clock
            );
        }
    }

    #[test]
    fn test_from_azimuth_elevation_feed_position_roundtrip() {
        // Test that converting angles -> feed position -> angles works correctly
        let focal_length = 13.6;

        let test_cases = vec![
            (0.0, 0.0),  // Boresight
            (0.0, 1.0),  // Small elevation offset
            (0.0, 5.0),  // Larger elevation offset
            (90.0, 5.0), // Offset in Y direction
            (45.0, 3.0), // Diagonal
        ];

        for (az_deg, el_deg) in test_cases {
            // Convert to E-cone/E-clock
            let ecc = EClockConeCoordinates::from_azimuth_elevation(az_deg, el_deg);

            // Convert to feed position
            let (x, y, z) = ecc.to_feed_position(focal_length);

            // Convert back to E-cone/E-clock
            let ecc_back = EClockConeCoordinates::from_feed_position(x, y, z, focal_length);

            // Should match original (within tolerance)
            let cone_error = (ecc_back.e_cone - ecc.e_cone).abs();
            let clock_error = normalize_angle_symmetric(ecc_back.e_clock - ecc.e_clock).abs();

            assert!(
                cone_error < 1e-6,
                "Cone angle roundtrip error for (az={}, el={}): {:.9} rad",
                az_deg,
                el_deg,
                cone_error
            );

            // Clock angle is ambiguous at boresight (el=0)
            if el_deg > 0.1 {
                assert!(
                    clock_error < 1e-6,
                    "Clock angle roundtrip error for (az={}, el={}): {:.9} rad",
                    az_deg,
                    el_deg,
                    clock_error
                );
            }
        }
    }

    #[test]
    fn test_from_azimuth_elevation_5deg_offset_physical_displacement() {
        // Specific test for the scenario in geo_feed_emitter_colocated_offset.json
        // Feed aimed 5° from boresight should produce ~1.19m physical displacement

        let focal_length = 13.6; // 34m dish, f/D=0.4
        let az_deg = 0.0;
        let el_deg = 5.0; // 5° polar angle from boresight

        let ecc = EClockConeCoordinates::from_azimuth_elevation(az_deg, el_deg);
        let (x, y, _z) = ecc.to_feed_position(focal_length);

        // Calculate lateral displacement
        let lateral_displacement = (x * x + y * y).sqrt();

        // For small angles: displacement ≈ f * tan(θ)
        // For θ = 5° = 0.0873 rad: displacement ≈ 13.6 * tan(0.0873) ≈ 1.19m
        let expected_displacement = focal_length * el_deg.to_radians().tan();

        assert!(
            (lateral_displacement - expected_displacement).abs() < 0.01,
            "5° offset should produce ~{:.3}m lateral displacement, got {:.3}m",
            expected_displacement,
            lateral_displacement
        );

        // Verify it's approximately the expected value
        assert!(
            lateral_displacement > 1.18 && lateral_displacement < 1.20,
            "Expected ~1.19m displacement, got {:.3}m",
            lateral_displacement
        );
    }

    #[test]
    fn test_from_azimuth_elevation_boresight_gives_focal_point() {
        // When azimuth=0 and elevation=0 (boresight), feed should be at focal point
        let focal_length = 13.6;

        let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 0.0);
        let (x, y, z) = ecc.to_feed_position(focal_length);

        assert!(
            x.abs() < 1e-10,
            "Feed at boresight should have x=0, got {:.6}",
            x
        );
        assert!(
            y.abs() < 1e-10,
            "Feed at boresight should have y=0, got {:.6}",
            y
        );
        assert!(
            (z - focal_length).abs() < 1e-10,
            "Feed at boresight should have z=focal_length, got {:.6} vs {:.6}",
            z,
            focal_length
        );
    }

    #[test]
    fn test_from_azimuth_elevation_azimuth_direction() {
        // Verify azimuth controls the direction in XY plane
        let focal_length = 13.6;
        let el_deg = 5.0;

        // Azimuth 0° aims the beam along +X, so the feed goes to -X
        let ecc_0 = EClockConeCoordinates::from_azimuth_elevation(0.0, el_deg);
        let (x0, y0, _) = ecc_0.to_feed_position(focal_length);
        assert!(x0 < -1.0, "Azimuth 0° should have large negative x");
        assert!(y0.abs() < 0.01, "Azimuth 0° should have y≈0");

        // Azimuth 90° aims along +Y, so the feed goes to -Y
        let ecc_90 = EClockConeCoordinates::from_azimuth_elevation(90.0, el_deg);
        let (x90, y90, _) = ecc_90.to_feed_position(focal_length);
        assert!(x90.abs() < 0.01, "Azimuth 90° should have x≈0");
        assert!(y90 < -1.0, "Azimuth 90° should have large negative y");

        // Azimuth 180° aims along -X, so the feed goes to +X
        let ecc_180 = EClockConeCoordinates::from_azimuth_elevation(180.0, el_deg);
        let (x180, y180, _) = ecc_180.to_feed_position(focal_length);
        assert!(x180 > 1.0, "Azimuth 180° should have large positive x");
        assert!(y180.abs() < 0.01, "Azimuth 180° should have y≈0");
    }

    #[test]
    fn test_to_degrees_returns_degrees() {
        let ecc = EClockConeCoordinates::new(PI / 2.0, PI);
        let (cone_deg, clock_deg) = ecc.to_degrees();
        assert!((cone_deg - 90.0).abs() < EPSILON, "cone_deg={cone_deg}");
        assert!((clock_deg - 180.0).abs() < EPSILON, "clock_deg={clock_deg}");
    }
}
