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
//! - θ: Polar angle from boresight (0 at boresight, π/2 at horizon)
//! - φ: Azimuthal angle (0 to 2π)
//!
//! ## E-clock/E-cone Coordinates
//! - E-cone: Cone angle from boresight (similar to θ)
//! - E-clock: Clock angle around boresight (0 to 2π)
//!
//! ## Cartesian Coordinates (x, y, z)
//! - Origin at reflector vertex
//! - z-axis along reflector axis (positive toward reflector)
//! - x, y axes define aperture plane

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
        (self.e_cone.to_radians(), self.e_clock.to_radians())
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

    /// Calculate feed displacement from focal point
    ///
    /// This is the key transformation from pointing angles to physical feed position.
    /// Based on design doc Section 2.5:
    /// ```text
    /// displacement = 2*f*tan(cone_angle/2)
    /// x_feed = displacement*cos(clock_angle)
    /// y_feed = displacement*sin(clock_angle)
    /// z_feed = -displacement^2/(4f)  for large displacements
    /// ```
    ///
    /// # Arguments
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    ///
    /// # Returns
    /// Feed position (x, y, z) in Cartesian coordinates relative to focal point
    pub fn to_feed_displacement(&self, focal_length: f64) -> (f64, f64, f64) {
        // Radial displacement in xy-plane
        let displacement = 2.0 * focal_length * (self.e_cone / 2.0).tan();

        // Cartesian components
        let x_feed = displacement * self.e_clock.cos();
        let y_feed = displacement * self.e_clock.sin();

        // For large displacements, include z-component (defocus)
        // This keeps the feed on the paraboloid surface
        let z_feed = -displacement * displacement / (4.0 * focal_length);

        (x_feed, y_feed, z_feed)
    }

    /// Calculate feed position from E-clock/E-cone
    ///
    /// Returns absolute feed position (not relative to focal point)
    ///
    /// # Arguments
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    ///
    /// # Returns
    /// Feed position (x, y, z) with origin at reflector vertex
    pub fn to_feed_position(&self, focal_length: f64) -> (f64, f64, f64) {
        let (dx, dy, dz) = self.to_feed_displacement(focal_length);
        (dx, dy, focal_length + dz)
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

        // Clock angle is simply the azimuthal angle in xy-plane
        let e_clock = y.atan2(x);

        Self::new(e_cone, e_clock)
    }
}

/// Azimuth/Elevation coordinates
///
/// Alternative pointing coordinate system using azimuth (horizontal angle)
/// and elevation (vertical angle from horizon)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AzElCoordinates {
    /// Azimuth angle (radians, 0 to 2π, typically 0 = North, π/2 = East)
    pub azimuth: f64,
    /// Elevation angle (radians, 0 = horizon, π/2 = zenith)
    pub elevation: f64,
}

impl AzElCoordinates {
    /// Create new azimuth/elevation coordinates
    pub fn new(azimuth: f64, elevation: f64) -> Self {
        Self { azimuth, elevation }
    }

    /// Create from degrees
    pub fn from_degrees(azimuth_deg: f64, elevation_deg: f64) -> Self {
        Self {
            azimuth: azimuth_deg.to_radians(),
            elevation: elevation_deg.to_radians(),
        }
    }

    /// Convert to degrees
    pub fn to_degrees(&self) -> (f64, f64) {
        (self.azimuth.to_degrees(), self.elevation.to_degrees())
    }

    /// Convert to far-field spherical coordinates (θ, φ)
    ///
    /// Note: θ is measured from zenith, elevation is measured from horizon
    /// So: θ = π/2 - elevation
    pub fn to_far_field(&self) -> FarFieldCoordinates {
        let theta = PI / 2.0 - self.elevation;
        let phi = self.azimuth;
        FarFieldCoordinates::new(theta, phi)
    }

    /// Create from far-field coordinates
    pub fn from_far_field(far_field: FarFieldCoordinates) -> Self {
        let elevation = PI / 2.0 - far_field.theta;
        let azimuth = far_field.phi;
        Self::new(azimuth, elevation)
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

        // For small angles, displacement ≈ f·cone
        let expected_displacement = 2.0 * focal_length * (e_cone / 2.0).tan();
        assert!((x - expected_displacement).abs() < 0.01);
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

        // x, y should reflect the clock angle
        let _radial = (x * x + y * y).sqrt();
        let angle = y.atan2(x);
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
    fn test_azel_to_far_field() {
        // Zenith (elevation = 90°) should give θ = 0
        let azel = AzElCoordinates::from_degrees(0.0, 90.0);
        let ff = azel.to_far_field();
        assert!(ff.theta.abs() < EPSILON);

        // Horizon (elevation = 0°) should give θ = π/2
        let azel = AzElCoordinates::from_degrees(0.0, 0.0);
        let ff = azel.to_far_field();
        assert!((ff.theta - PI / 2.0).abs() < EPSILON);

        // Round-trip
        let azel2 = AzElCoordinates::from_far_field(ff);
        assert!((azel2.elevation - azel.elevation).abs() < EPSILON);
        assert!((azel2.azimuth - azel.azimuth).abs() < EPSILON);
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
}
