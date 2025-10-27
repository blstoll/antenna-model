//! Antenna Geometry Data Structures
//!
//! This module defines the physical geometry and parameters for parabolic dish antennas
//! with steerable feeds. These structures are used by the physical optics computation engine
//! to model antenna radiation patterns.
//!
//! # Overview
//!
//! The antenna model consists of three main components:
//! - **Reflector Geometry**: Physical parameters of the parabolic dish
//! - **Feed Parameters**: Position and pattern characteristics of the feed horn
//! - **Mesh Parameters**: Properties of wire mesh reflectors
//!
//! These are combined into an `AntennaConfiguration` that provides a complete
//! description of the antenna system.

use crate::error::{ValidationError, ValidationResult};
use serde::{Deserialize, Serialize};

/// Reflector geometry for a parabolic dish antenna
///
/// This structure captures the physical dimensions and surface quality
/// of the parabolic reflector. The focal length and diameter determine
/// the f/D ratio, which affects feed illumination and aperture efficiency.
///
/// # Physical Constraints
/// - Diameter must be positive (> 0)
/// - Focal length must be positive (> 0)
/// - f/D ratio is typically in range [0.3, 0.5] for practical antennas
/// - Surface RMS should be much smaller than the shortest wavelength
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReflectorGeometry {
    /// Reflector diameter in meters
    pub diameter: f64,

    /// Focal length in meters (distance from vertex to focal point)
    pub focal_length: f64,

    /// Surface RMS error in meters (deviation from ideal parabolic shape)
    /// Used in Ruze's equation: η = exp(-(4π·σ/λ)²)
    pub surface_rms: f64,
}

impl ReflectorGeometry {
    /// Create a new reflector geometry with validation
    ///
    /// # Arguments
    /// - `diameter`: Reflector diameter in meters (must be > 0)
    /// - `focal_length`: Focal length in meters (must be > 0)
    /// - `surface_rms`: Surface RMS error in meters (must be >= 0)
    ///
    /// # Errors
    /// Returns `ValidationError` if parameters are out of valid physical ranges
    pub fn new(diameter: f64, focal_length: f64, surface_rms: f64) -> ValidationResult<Self> {
        let geometry = Self {
            diameter,
            focal_length,
            surface_rms,
        };
        geometry.validate()?;
        Ok(geometry)
    }

    /// Get the f/D ratio (focal length / diameter)
    ///
    /// This is a key parameter affecting illumination efficiency and feed design.
    /// Typical values are 0.3-0.5 for parabolic antennas.
    pub fn f_over_d(&self) -> f64 {
        self.focal_length / self.diameter
    }

    /// Get the aperture radius (half the diameter)
    pub fn aperture_radius(&self) -> f64 {
        self.diameter / 2.0
    }

    /// Validate physical constraints
    pub fn validate(&self) -> ValidationResult<()> {
        if self.diameter <= 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "diameter".to_string(),
                reason: format!("Diameter must be positive, got {}", self.diameter),
            });
        }

        if self.focal_length <= 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "focal_length".to_string(),
                reason: format!("Focal length must be positive, got {}", self.focal_length),
            });
        }

        if self.surface_rms < 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "surface_rms".to_string(),
                reason: format!("Surface RMS must be non-negative, got {}", self.surface_rms),
            });
        }

        // Check f/D ratio is in reasonable range (warn if unusual)
        let f_over_d = self.f_over_d();
        if !(0.2..=1.0).contains(&f_over_d) {
            // This is unusual but not necessarily invalid - could be a specialized design
            // We don't error, but this might be logged at a higher level
        }

        Ok(())
    }

    /// Create a builder for constructing reflector geometry
    pub fn builder() -> ReflectorGeometryBuilder {
        ReflectorGeometryBuilder::default()
    }
}

/// Builder for `ReflectorGeometry`
#[derive(Debug, Default)]
pub struct ReflectorGeometryBuilder {
    diameter: Option<f64>,
    focal_length: Option<f64>,
    surface_rms: Option<f64>,
}

impl ReflectorGeometryBuilder {
    /// Set the reflector diameter in meters
    pub fn diameter(mut self, diameter: f64) -> Self {
        self.diameter = Some(diameter);
        self
    }

    /// Set the focal length in meters
    pub fn focal_length(mut self, focal_length: f64) -> Self {
        self.focal_length = Some(focal_length);
        self
    }

    /// Set the surface RMS error in meters
    pub fn surface_rms(mut self, surface_rms: f64) -> Self {
        self.surface_rms = Some(surface_rms);
        self
    }

    /// Build the `ReflectorGeometry` with validation
    pub fn build(self) -> ValidationResult<ReflectorGeometry> {
        let diameter = self.diameter.ok_or_else(|| ValidationError::InvalidValue {
            param: "diameter".to_string(),
            reason: "Diameter not specified".to_string(),
        })?;

        let focal_length = self
            .focal_length
            .ok_or_else(|| ValidationError::InvalidValue {
                param: "focal_length".to_string(),
                reason: "Focal length not specified".to_string(),
            })?;

        let surface_rms = self.surface_rms.unwrap_or(0.0); // Default to ideal surface

        ReflectorGeometry::new(diameter, focal_length, surface_rms)
    }
}

/// Feed horn parameters for antenna illumination
///
/// Describes the position and radiation pattern of the feed horn.
/// The feed pattern is modeled using a cos^q approximation, where q
/// controls the edge taper (higher q = more focused beam).
///
/// # Physical Constraints
/// - Q-factor typically in range [6, 12] for practical feeds
/// - Phase center offset typically < λ/4
/// - Feed position relative to focal point affects aberrations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedParameters {
    /// Feed position in Cartesian coordinates (meters)
    /// Origin is at the reflector vertex, +z is toward the reflector
    pub position: FeedPosition,

    /// Q-factor for cos^q feed pattern approximation
    /// Higher values → more focused beam (less spillover, higher edge taper)
    /// Typical range: 6-10 for 10 dB edge taper, 10-12 for 12 dB edge taper
    pub q_factor: f64,

    /// Phase center offset in meters (distance from physical feed to phase center)
    /// Typically ±λ/4, frequency-dependent
    pub phase_center_offset: f64,

    /// Asymmetry factor for E-plane vs H-plane patterns (1.0 = symmetric)
    /// Values > 1.0 indicate broader E-plane pattern
    pub asymmetry_factor: f64,
}

impl FeedParameters {
    /// Create new feed parameters with validation
    pub fn new(
        position: FeedPosition,
        q_factor: f64,
        phase_center_offset: f64,
        asymmetry_factor: f64,
    ) -> ValidationResult<Self> {
        let params = Self {
            position,
            q_factor,
            phase_center_offset,
            asymmetry_factor,
        };
        params.validate()?;
        Ok(params)
    }

    /// Validate feed parameters
    pub fn validate(&self) -> ValidationResult<()> {
        if self.q_factor <= 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "q_factor".to_string(),
                reason: format!("Q-factor must be positive, got {}", self.q_factor),
            });
        }

        if self.asymmetry_factor <= 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "asymmetry_factor".to_string(),
                reason: format!(
                    "Asymmetry factor must be positive, got {}",
                    self.asymmetry_factor
                ),
            });
        }

        self.position.validate()?;
        Ok(())
    }

    /// Create a builder for constructing feed parameters
    pub fn builder() -> FeedParametersBuilder {
        FeedParametersBuilder::default()
    }
}

/// Feed position in Cartesian coordinates
///
/// The coordinate system has origin at the reflector vertex:
/// - x-axis: horizontal (arbitrary reference)
/// - y-axis: horizontal (perpendicular to x)
/// - z-axis: along reflector axis (positive toward reflector)
///
/// The focal point is at (0, 0, f) where f is the focal length.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct FeedPosition {
    /// X coordinate in meters
    pub x: f64,
    /// Y coordinate in meters
    pub y: f64,
    /// Z coordinate in meters
    pub z: f64,
}

impl FeedPosition {
    /// Create a new feed position
    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Create feed position at focal point
    pub fn at_focus(focal_length: f64) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: focal_length,
        }
    }

    /// Calculate displacement from focal point
    pub fn displacement_from_focus(&self, focal_length: f64) -> f64 {
        let dx = self.x;
        let dy = self.y;
        let dz = self.z - focal_length;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    /// Calculate radial displacement in xy-plane from axis
    pub fn radial_displacement(&self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Validate position is finite
    pub fn validate(&self) -> ValidationResult<()> {
        if !self.x.is_finite() || !self.y.is_finite() || !self.z.is_finite() {
            return Err(ValidationError::InvalidValue {
                param: "feed_position".to_string(),
                reason: "Feed position coordinates must be finite".to_string(),
            });
        }
        Ok(())
    }
}

/// Builder for `FeedParameters`
#[derive(Debug, Default)]
pub struct FeedParametersBuilder {
    position: Option<FeedPosition>,
    q_factor: Option<f64>,
    phase_center_offset: Option<f64>,
    asymmetry_factor: Option<f64>,
}

impl FeedParametersBuilder {
    /// Set feed position
    pub fn position(mut self, position: FeedPosition) -> Self {
        self.position = Some(position);
        self
    }

    /// Set feed position at focal point
    pub fn at_focus(mut self, focal_length: f64) -> Self {
        self.position = Some(FeedPosition::at_focus(focal_length));
        self
    }

    /// Set q-factor
    pub fn q_factor(mut self, q_factor: f64) -> Self {
        self.q_factor = Some(q_factor);
        self
    }

    /// Set phase center offset
    pub fn phase_center_offset(mut self, offset: f64) -> Self {
        self.phase_center_offset = Some(offset);
        self
    }

    /// Set asymmetry factor
    pub fn asymmetry_factor(mut self, factor: f64) -> Self {
        self.asymmetry_factor = Some(factor);
        self
    }

    /// Build the `FeedParameters` with validation
    pub fn build(self) -> ValidationResult<FeedParameters> {
        let position = self.position.ok_or_else(|| ValidationError::InvalidValue {
            param: "position".to_string(),
            reason: "Feed position not specified".to_string(),
        })?;

        let q_factor = self.q_factor.ok_or_else(|| ValidationError::InvalidValue {
            param: "q_factor".to_string(),
            reason: "Q-factor not specified".to_string(),
        })?;

        let phase_center_offset = self.phase_center_offset.unwrap_or(0.0);
        let asymmetry_factor = self.asymmetry_factor.unwrap_or(1.0); // Default to symmetric

        FeedParameters::new(position, q_factor, phase_center_offset, asymmetry_factor)
    }
}

/// Wire mesh reflector parameters
///
/// For mesh reflectors (common in large radio telescopes), the mesh introduces
/// frequency-dependent transparency and phase effects. At low frequencies
/// (λ > 10·mesh_spacing), significant power is transmitted through the mesh.
///
/// # Physical Constraints
/// - Mesh spacing typically 1-10 mm
/// - Wire diameter typically 0.05-1 mm (much smaller than spacing)
/// - Wire diameter must be less than mesh spacing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeshParameters {
    /// Mesh spacing in meters (distance between parallel wires)
    pub spacing: f64,

    /// Wire diameter in meters
    pub wire_diameter: f64,

    /// Mesh pattern type (for future extension - e.g., square, triangular, hexagonal)
    pub pattern_type: MeshPattern,
}

impl MeshParameters {
    /// Create new mesh parameters with validation
    pub fn new(
        spacing: f64,
        wire_diameter: f64,
        pattern_type: MeshPattern,
    ) -> ValidationResult<Self> {
        let params = Self {
            spacing,
            wire_diameter,
            pattern_type,
        };
        params.validate()?;
        Ok(params)
    }

    /// Validate mesh parameters
    pub fn validate(&self) -> ValidationResult<()> {
        if self.spacing <= 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "mesh_spacing".to_string(),
                reason: format!("Mesh spacing must be positive, got {}", self.spacing),
            });
        }

        if self.wire_diameter <= 0.0 {
            return Err(ValidationError::InvalidValue {
                param: "wire_diameter".to_string(),
                reason: format!("Wire diameter must be positive, got {}", self.wire_diameter),
            });
        }

        if self.wire_diameter >= self.spacing {
            return Err(ValidationError::InvalidValue {
                param: "wire_diameter".to_string(),
                reason: format!(
                    "Wire diameter ({}) must be less than mesh spacing ({})",
                    self.wire_diameter, self.spacing
                ),
            });
        }

        Ok(())
    }

    /// Create a builder for constructing mesh parameters
    pub fn builder() -> MeshParametersBuilder {
        MeshParametersBuilder::default()
    }

    /// Calculate mesh transparency at given wavelength
    ///
    /// Low-frequency approximation: T = 1/(1 + (λ₀/λ)²)
    /// where λ₀ is the cutoff wavelength related to mesh spacing
    ///
    /// This is a simplified model; full analysis requires Floquet mode analysis
    pub fn transparency_at_wavelength(&self, wavelength: f64) -> f64 {
        let lambda_0 = 10.0 * self.spacing; // Cutoff wavelength approximation
        1.0 / (1.0 + (lambda_0 / wavelength).powi(2))
    }
}

/// Mesh pattern types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MeshPattern {
    /// Square mesh (orthogonal wires)
    Square,
    /// Triangular mesh
    Triangular,
    /// Hexagonal mesh
    Hexagonal,
}

/// Builder for `MeshParameters`
#[derive(Debug)]
pub struct MeshParametersBuilder {
    spacing: Option<f64>,
    wire_diameter: Option<f64>,
    pattern_type: Option<MeshPattern>,
}

impl Default for MeshParametersBuilder {
    fn default() -> Self {
        Self {
            spacing: None,
            wire_diameter: None,
            pattern_type: Some(MeshPattern::Square), // Default to square
        }
    }
}

impl MeshParametersBuilder {
    /// Set mesh spacing
    pub fn spacing(mut self, spacing: f64) -> Self {
        self.spacing = Some(spacing);
        self
    }

    /// Set wire diameter
    pub fn wire_diameter(mut self, diameter: f64) -> Self {
        self.wire_diameter = Some(diameter);
        self
    }

    /// Set mesh pattern type
    pub fn pattern_type(mut self, pattern: MeshPattern) -> Self {
        self.pattern_type = Some(pattern);
        self
    }

    /// Build the `MeshParameters` with validation
    pub fn build(self) -> ValidationResult<MeshParameters> {
        let spacing = self.spacing.ok_or_else(|| ValidationError::InvalidValue {
            param: "spacing".to_string(),
            reason: "Mesh spacing not specified".to_string(),
        })?;

        let wire_diameter = self
            .wire_diameter
            .ok_or_else(|| ValidationError::InvalidValue {
                param: "wire_diameter".to_string(),
                reason: "Wire diameter not specified".to_string(),
            })?;

        let pattern_type = self.pattern_type.unwrap_or(MeshPattern::Square);

        MeshParameters::new(spacing, wire_diameter, pattern_type)
    }
}

/// Complete antenna configuration combining all physical parameters
///
/// This structure represents a complete antenna system including reflector,
/// feed, and mesh properties. It serves as the input to the physical optics
/// computation engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AntennaConfiguration {
    /// Unique identifier for this antenna
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Reflector geometry
    pub reflector: ReflectorGeometry,

    /// Feed parameters
    pub feed: FeedParameters,

    /// Mesh parameters (None for solid reflectors)
    pub mesh: Option<MeshParameters>,
}

impl AntennaConfiguration {
    /// Create new antenna configuration with validation
    pub fn new(
        id: String,
        name: String,
        reflector: ReflectorGeometry,
        feed: FeedParameters,
        mesh: Option<MeshParameters>,
    ) -> ValidationResult<Self> {
        let config = Self {
            id,
            name,
            reflector,
            feed,
            mesh,
        };
        config.validate()?;
        Ok(config)
    }

    /// Validate the complete antenna configuration
    pub fn validate(&self) -> ValidationResult<()> {
        if self.id.is_empty() {
            return Err(ValidationError::InvalidValue {
                param: "id".to_string(),
                reason: "Antenna ID cannot be empty".to_string(),
            });
        }

        self.reflector.validate()?;
        self.feed.validate()?;

        if let Some(ref mesh) = self.mesh {
            mesh.validate()?;
        }

        Ok(())
    }

    /// Check if feed displacement is large (might need special handling)
    ///
    /// Large feed offsets (> 0.3f) may require ray tracing instead of
    /// simple physical optics approximation
    pub fn has_large_feed_offset(&self) -> bool {
        let displacement = self
            .feed
            .position
            .displacement_from_focus(self.reflector.focal_length);
        displacement > 0.3 * self.reflector.focal_length
    }

    /// Create a builder for constructing antenna configuration
    pub fn builder() -> AntennaConfigurationBuilder {
        AntennaConfigurationBuilder::default()
    }
}

/// Builder for `AntennaConfiguration`
#[derive(Debug, Default)]
pub struct AntennaConfigurationBuilder {
    id: Option<String>,
    name: Option<String>,
    reflector: Option<ReflectorGeometry>,
    feed: Option<FeedParameters>,
    mesh: Option<MeshParameters>,
}

impl AntennaConfigurationBuilder {
    /// Set antenna ID
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Set antenna name
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set reflector geometry
    pub fn reflector(mut self, reflector: ReflectorGeometry) -> Self {
        self.reflector = Some(reflector);
        self
    }

    /// Set feed parameters
    pub fn feed(mut self, feed: FeedParameters) -> Self {
        self.feed = Some(feed);
        self
    }

    /// Set mesh parameters
    pub fn mesh(mut self, mesh: MeshParameters) -> Self {
        self.mesh = Some(mesh);
        self
    }

    /// Build the `AntennaConfiguration` with validation
    pub fn build(self) -> ValidationResult<AntennaConfiguration> {
        let id = self.id.ok_or_else(|| ValidationError::InvalidValue {
            param: "id".to_string(),
            reason: "Antenna ID not specified".to_string(),
        })?;

        let name = self.name.ok_or_else(|| ValidationError::InvalidValue {
            param: "name".to_string(),
            reason: "Antenna name not specified".to_string(),
        })?;

        let reflector = self
            .reflector
            .ok_or_else(|| ValidationError::InvalidValue {
                param: "reflector".to_string(),
                reason: "Reflector geometry not specified".to_string(),
            })?;

        let feed = self.feed.ok_or_else(|| ValidationError::InvalidValue {
            param: "feed".to_string(),
            reason: "Feed parameters not specified".to_string(),
        })?;

        AntennaConfiguration::new(id, name, reflector, feed, self.mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflector_geometry_valid() {
        let reflector = ReflectorGeometry::new(34.0, 17.0, 0.001).unwrap();
        assert_eq!(reflector.diameter, 34.0);
        assert_eq!(reflector.focal_length, 17.0);
        assert!((reflector.f_over_d() - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_reflector_geometry_invalid_diameter() {
        let result = ReflectorGeometry::new(-1.0, 17.0, 0.001);
        assert!(result.is_err());
    }

    #[test]
    fn test_reflector_geometry_invalid_focal_length() {
        let result = ReflectorGeometry::new(34.0, -1.0, 0.001);
        assert!(result.is_err());
    }

    #[test]
    fn test_reflector_geometry_builder() {
        let reflector = ReflectorGeometry::builder()
            .diameter(34.0)
            .focal_length(17.0)
            .surface_rms(0.001)
            .build()
            .unwrap();

        assert_eq!(reflector.diameter, 34.0);
        assert_eq!(reflector.focal_length, 17.0);
        assert_eq!(reflector.surface_rms, 0.001);
    }

    #[test]
    fn test_feed_position_at_focus() {
        let pos = FeedPosition::at_focus(17.0);
        assert_eq!(pos.x, 0.0);
        assert_eq!(pos.y, 0.0);
        assert_eq!(pos.z, 17.0);
        assert!((pos.displacement_from_focus(17.0)).abs() < 1e-10);
    }

    #[test]
    fn test_feed_position_displacement() {
        let pos = FeedPosition::new(1.0, 0.0, 17.0);
        let disp = pos.displacement_from_focus(17.0);
        assert!((disp - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_feed_parameters_valid() {
        let pos = FeedPosition::at_focus(17.0);
        let feed = FeedParameters::new(pos, 8.0, 0.01, 1.0).unwrap();
        assert_eq!(feed.q_factor, 8.0);
    }

    #[test]
    fn test_feed_parameters_builder() {
        let feed = FeedParameters::builder()
            .at_focus(17.0)
            .q_factor(8.0)
            .phase_center_offset(0.01)
            .build()
            .unwrap();

        assert_eq!(feed.q_factor, 8.0);
        assert_eq!(feed.asymmetry_factor, 1.0); // Default
    }

    #[test]
    fn test_mesh_parameters_valid() {
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap();
        assert_eq!(mesh.spacing, 0.005);
        assert_eq!(mesh.wire_diameter, 0.0005);
    }

    #[test]
    fn test_mesh_parameters_invalid_wire_too_large() {
        let result = MeshParameters::new(0.005, 0.006, MeshPattern::Square);
        assert!(result.is_err());
    }

    #[test]
    fn test_mesh_transparency() {
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap();

        // At very long wavelengths, transparency should be high
        let transparency_long = mesh.transparency_at_wavelength(1.0);
        assert!(transparency_long > 0.9);

        // At short wavelengths, transparency should be low
        let transparency_short = mesh.transparency_at_wavelength(0.01);
        assert!(transparency_short < 0.5);
    }

    #[test]
    fn test_antenna_configuration_valid() {
        let reflector = ReflectorGeometry::new(34.0, 17.0, 0.001).unwrap();
        let feed = FeedParameters::builder()
            .at_focus(17.0)
            .q_factor(8.0)
            .build()
            .unwrap();
        let mesh = MeshParameters::new(0.005, 0.0005, MeshPattern::Square).unwrap();

        let config = AntennaConfiguration::new(
            "test_antenna".to_string(),
            "Test 34m Dish".to_string(),
            reflector,
            feed,
            Some(mesh),
        )
        .unwrap();

        assert_eq!(config.id, "test_antenna");
        assert!(!config.has_large_feed_offset());
    }

    #[test]
    fn test_antenna_configuration_builder() {
        let reflector = ReflectorGeometry::builder()
            .diameter(34.0)
            .focal_length(17.0)
            .surface_rms(0.001)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .at_focus(17.0)
            .q_factor(8.0)
            .build()
            .unwrap();

        let config = AntennaConfiguration::builder()
            .id("test")
            .name("Test Antenna")
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        assert_eq!(config.id, "test");
        assert!(config.mesh.is_none());
    }

    #[test]
    fn test_large_feed_offset_detection() {
        let reflector = ReflectorGeometry::new(34.0, 17.0, 0.001).unwrap();

        // Small offset (at focus)
        let feed_small = FeedParameters::builder()
            .at_focus(17.0)
            .q_factor(8.0)
            .build()
            .unwrap();

        let config_small = AntennaConfiguration::builder()
            .id("test")
            .name("Test")
            .reflector(reflector.clone())
            .feed(feed_small)
            .build()
            .unwrap();

        assert!(!config_small.has_large_feed_offset());

        // Large offset (> 0.3f)
        let pos_large = FeedPosition::new(6.0, 0.0, 17.0); // 6m offset > 0.3*17 = 5.1m
        let feed_large = FeedParameters::builder()
            .position(pos_large)
            .q_factor(8.0)
            .build()
            .unwrap();

        let config_large = AntennaConfiguration::builder()
            .id("test")
            .name("Test")
            .reflector(reflector)
            .feed(feed_large)
            .build()
            .unwrap();

        assert!(config_large.has_large_feed_offset());
    }
}
