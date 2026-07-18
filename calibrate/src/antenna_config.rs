//! Antenna configuration types for calibration tool
//!
//! This module defines the antenna configuration system with a hybrid parameter approach:
//! - **Antenna Classes**: Shared physical/geometric parameters for antenna types
//! - **Tunable Parameters**: Small set of per-antenna optimizable parameters
//! - **Antenna Configuration**: Complete config combining class + tunable params
//!
//! The philosophy:
//! 1. Physical geometry (diameter, f/D) is shared across antenna class
//! 2. A few key parameters (surface RMS, mesh spacing) can be tuned per-antenna
//! 3. Correction surfaces (Task 4.4) handle band-splits and feed-specific losses

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Shared parameters defining an antenna class
///
/// These parameters are common to all antennas of this class and define
/// the nominal design characteristics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntennaClass {
    /// Unique identifier for this antenna class (e.g., "DSN_34m", "GroundStation_13m")
    pub class_id: String,

    /// Human-readable description
    pub description: String,

    /// Reflector geometry
    pub geometry: ReflectorGeometry,

    /// Feed parameters
    pub feed: FeedParameters,

    /// Mesh parameters (nominal values)
    pub mesh: MeshParameters,

    /// Surface quality (nominal value)
    pub surface: SurfaceParameters,

    /// System noise temperature for G/T to gain conversion
    pub system_noise_temperature_k: f64,
}

/// Reflector geometry parameters (shared across antenna class)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectorGeometry {
    /// Reflector diameter in meters
    pub diameter_m: f64,

    /// Focal length to diameter ratio (typically ~0.5)
    pub f_over_d: f64,
}

/// Feed parameters (shared across antenna class)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedParameters {
    /// Q-factor for cos^q illumination pattern (typically 6-10)
    pub q_factor: f64,

    /// Phase center offset in wavelengths (typically ±0.25)
    pub phase_center_offset_wavelengths: f64,

    /// Asymmetry factor for E-plane vs H-plane (1.0 = symmetric)
    pub asymmetry_factor: f64,
}

/// Mesh parameters (nominal values)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshParameters {
    /// Mesh spacing in millimeters (typically 1-10 mm)
    pub spacing_mm: f64,

    /// Wire diameter in millimeters (typically 0.05-1 mm)
    pub wire_diameter_mm: f64,
}

/// Surface quality parameters (nominal value)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceParameters {
    /// Surface RMS error in millimeters (typically 0.1-2 mm)
    pub rms_mm: f64,
}

/// Tunable parameters that can be optimized per-antenna
///
/// These parameters can be adjusted during calibration to improve fit.
/// The optimization is optional - if skipped, nominal values from the
/// antenna class are used.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunableParameters {
    /// Surface RMS error (mm)
    pub surface_rms_mm: Option<f64>,

    /// Mesh spacing (mm)
    pub mesh_spacing_mm: Option<f64>,

    /// Mesh wire diameter (mm)
    pub mesh_wire_diameter_mm: Option<f64>,
}

impl TunableParameters {
    /// Create tunable parameters with no overrides (use class defaults)
    pub fn default_from_class() -> Self {
        Self {
            surface_rms_mm: None,
            mesh_spacing_mm: None,
            mesh_wire_diameter_mm: None,
        }
    }

    /// Check if any parameters have been tuned
    pub fn has_tuned_values(&self) -> bool {
        self.surface_rms_mm.is_some()
            || self.mesh_spacing_mm.is_some()
            || self.mesh_wire_diameter_mm.is_some()
    }

    /// Get the effective surface RMS (tuned or default)
    pub fn effective_surface_rms(&self, class: &AntennaClass) -> f64 {
        self.surface_rms_mm.unwrap_or(class.surface.rms_mm)
    }

    /// Get the effective mesh spacing (tuned or default)
    pub fn effective_mesh_spacing(&self, class: &AntennaClass) -> f64 {
        self.mesh_spacing_mm.unwrap_or(class.mesh.spacing_mm)
    }

    /// Get the effective wire diameter (tuned or default)
    pub fn effective_wire_diameter(&self, class: &AntennaClass) -> f64 {
        self.mesh_wire_diameter_mm
            .unwrap_or(class.mesh.wire_diameter_mm)
    }
}

/// Parameter bounds for optimization
///
/// Defines valid ranges for tunable parameters during optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterBounds {
    /// Surface RMS bounds (mm)
    pub surface_rms_mm: (f64, f64),

    /// Mesh spacing bounds (mm)
    pub mesh_spacing_mm: (f64, f64),

    /// Wire diameter bounds (mm)
    pub wire_diameter_mm: (f64, f64),
}

impl Default for ParameterBounds {
    fn default() -> Self {
        Self {
            surface_rms_mm: (0.1, 2.0),
            mesh_spacing_mm: (1.0, 10.0),
            wire_diameter_mm: (0.05, 1.0),
        }
    }
}

impl ParameterBounds {
    /// Validate that parameters are within bounds
    pub fn validate(&self, params: &TunableParameters, class: &AntennaClass) -> Result<(), String> {
        let surface_rms = params.effective_surface_rms(class);
        let mesh_spacing = params.effective_mesh_spacing(class);
        let wire_diameter = params.effective_wire_diameter(class);

        if surface_rms < self.surface_rms_mm.0 || surface_rms > self.surface_rms_mm.1 {
            return Err(format!(
                "Surface RMS {:.3} mm outside bounds [{:.3}, {:.3}]",
                surface_rms, self.surface_rms_mm.0, self.surface_rms_mm.1
            ));
        }

        if mesh_spacing < self.mesh_spacing_mm.0 || mesh_spacing > self.mesh_spacing_mm.1 {
            return Err(format!(
                "Mesh spacing {:.3} mm outside bounds [{:.3}, {:.3}]",
                mesh_spacing, self.mesh_spacing_mm.0, self.mesh_spacing_mm.1
            ));
        }

        if wire_diameter < self.wire_diameter_mm.0 || wire_diameter > self.wire_diameter_mm.1 {
            return Err(format!(
                "Wire diameter {:.3} mm outside bounds [{:.3}, {:.3}]",
                wire_diameter, self.wire_diameter_mm.0, self.wire_diameter_mm.1
            ));
        }

        Ok(())
    }
}

/// Complete antenna configuration for calibration
///
/// Combines antenna class reference with per-antenna tunable parameters
/// and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntennaConfiguration {
    /// Unique antenna identifier
    pub antenna_id: String,

    /// Human-readable name
    pub name: String,

    /// Reference to antenna class
    pub class_id: String,

    /// Tunable parameters (None values use class defaults)
    pub tunable_parameters: TunableParameters,

    /// Optional metadata
    pub metadata: AntennaMetadata,
}

/// Metadata for antenna configuration
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AntennaMetadata {
    /// Calibration date (ISO 8601 format)
    pub calibration_date: Option<String>,

    /// Measurement source (e.g., S3 URL, file path)
    pub measurement_source: Option<String>,

    /// Was parameter tuning performed?
    pub parameters_tuned: bool,

    /// Additional notes
    pub notes: Option<String>,
}

/// Container for antenna class definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntennaClassRegistry {
    /// Map of class_id -> AntennaClass
    pub classes: HashMap<String, AntennaClass>,
}

impl AntennaClassRegistry {
    /// Load antenna classes from YAML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let contents = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read antenna classes file: {}", e))?;

        let registry: AntennaClassRegistry = serde_yaml::from_str(&contents)
            .map_err(|e| format!("Failed to parse antenna classes YAML: {}", e))?;

        // Validate all classes
        for (class_id, class) in &registry.classes {
            if class.class_id != *class_id {
                return Err(format!(
                    "Class ID mismatch: key '{}' vs class_id '{}'",
                    class_id, class.class_id
                ));
            }
            class.validate()?;
        }

        Ok(registry)
    }

    /// Get antenna class by ID
    pub fn get_class(&self, class_id: &str) -> Option<&AntennaClass> {
        self.classes.get(class_id)
    }

    /// List all class IDs
    pub fn list_class_ids(&self) -> Vec<String> {
        self.classes.keys().cloned().collect()
    }
}

impl AntennaClass {
    /// Validate antenna class parameters
    pub fn validate(&self) -> Result<(), String> {
        // Validate geometry
        if self.geometry.diameter_m <= 0.0 {
            return Err(format!(
                "Invalid diameter: {} m (must be > 0)",
                self.geometry.diameter_m
            ));
        }

        {
            use antenna_model::model::geometry::{F_OVER_D_MAX, F_OVER_D_MIN};
            if !(F_OVER_D_MIN..=F_OVER_D_MAX).contains(&self.geometry.f_over_d) {
                return Err(format!(
                    "Invalid f/D ratio: {} (must be in [{}, {}])",
                    self.geometry.f_over_d, F_OVER_D_MIN, F_OVER_D_MAX
                ));
            }
        }

        // Validate feed
        if self.feed.q_factor < 0.0 {
            return Err(format!(
                "Invalid q_factor: {} (must be >= 0)",
                self.feed.q_factor
            ));
        }

        if self.feed.asymmetry_factor <= 0.0 {
            return Err(format!(
                "Invalid asymmetry_factor: {} (must be > 0)",
                self.feed.asymmetry_factor
            ));
        }

        // Validate mesh
        if self.mesh.spacing_mm <= 0.0 {
            return Err(format!(
                "Invalid mesh spacing: {} mm (must be > 0)",
                self.mesh.spacing_mm
            ));
        }

        if self.mesh.wire_diameter_mm <= 0.0 {
            return Err(format!(
                "Invalid wire diameter: {} mm (must be > 0)",
                self.mesh.wire_diameter_mm
            ));
        }

        if self.mesh.wire_diameter_mm >= self.mesh.spacing_mm {
            return Err(format!(
                "Wire diameter {} mm >= mesh spacing {} mm (physically impossible)",
                self.mesh.wire_diameter_mm, self.mesh.spacing_mm
            ));
        }

        // Validate surface
        if self.surface.rms_mm < 0.0 {
            return Err(format!(
                "Invalid surface RMS: {} mm (must be >= 0)",
                self.surface.rms_mm
            ));
        }

        // Validate temperature
        if self.system_noise_temperature_k <= 0.0 {
            return Err(format!(
                "Invalid system noise temperature: {} K (must be > 0)",
                self.system_noise_temperature_k
            ));
        }

        Ok(())
    }

    /// Calculate focal length from diameter and f/D ratio
    pub fn focal_length_m(&self) -> f64 {
        self.geometry.diameter_m * self.geometry.f_over_d
    }
}

impl AntennaConfiguration {
    /// Create a new antenna configuration with default parameters
    pub fn new(antenna_id: String, name: String, class_id: String) -> Self {
        Self {
            antenna_id,
            name,
            class_id,
            tunable_parameters: TunableParameters::default_from_class(),
            metadata: AntennaMetadata::default(),
        }
    }

    /// Load configuration from YAML file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let contents = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read antenna configuration file: {}", e))?;

        let config: AntennaConfiguration = serde_yaml::from_str(&contents)
            .map_err(|e| format!("Failed to parse antenna configuration YAML: {}", e))?;

        Ok(config)
    }

    /// Save configuration to YAML file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let yaml = serde_yaml::to_string(self)
            .map_err(|e| format!("Failed to serialize antenna configuration: {}", e))?;

        fs::write(path.as_ref(), yaml)
            .map_err(|e| format!("Failed to write antenna configuration file: {}", e))?;

        Ok(())
    }

    /// Validate configuration against antenna class
    pub fn validate(&self, registry: &AntennaClassRegistry) -> Result<(), String> {
        // Check that class exists
        let class = registry
            .get_class(&self.class_id)
            .ok_or_else(|| format!("Antenna class '{}' not found", self.class_id))?;

        // Validate tunable parameters are within reasonable bounds
        let bounds = ParameterBounds::default();
        bounds.validate(&self.tunable_parameters, class)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_class() -> AntennaClass {
        AntennaClass {
            class_id: "test_34m".to_string(),
            description: "Test 34m antenna class".to_string(),
            geometry: ReflectorGeometry {
                diameter_m: 34.0,
                f_over_d: 0.5,
            },
            feed: FeedParameters {
                q_factor: 8.0,
                phase_center_offset_wavelengths: 0.0,
                asymmetry_factor: 1.0,
            },
            mesh: MeshParameters {
                spacing_mm: 5.0,
                wire_diameter_mm: 0.5,
            },
            surface: SurfaceParameters { rms_mm: 1.0 },
            system_noise_temperature_k: 50.0,
        }
    }

    #[test]
    fn test_antenna_class_validation() {
        let class = create_test_class();
        assert!(class.validate().is_ok());
    }

    #[test]
    fn test_antenna_class_invalid_diameter() {
        let mut class = create_test_class();
        class.geometry.diameter_m = -1.0;
        assert!(class.validate().is_err());
    }

    #[test]
    fn test_antenna_class_invalid_f_over_d() {
        let mut class = create_test_class();
        class.geometry.f_over_d = 1.5;
        assert!(class.validate().is_err());
    }

    #[test]
    fn test_antenna_class_f_over_d_below_range_rejected() {
        let mut class = create_test_class();
        class.geometry.f_over_d = 0.15;
        assert!(class.validate().is_err());
    }

    #[test]
    fn test_antenna_class_invalid_mesh() {
        let mut class = create_test_class();
        class.mesh.wire_diameter_mm = 10.0; // Greater than spacing
        assert!(class.validate().is_err());
    }

    #[test]
    fn test_focal_length_calculation() {
        let class = create_test_class();
        assert_eq!(class.focal_length_m(), 17.0); // 34 * 0.5
    }

    #[test]
    fn test_tunable_parameters_default() {
        let params = TunableParameters::default_from_class();
        assert!(!params.has_tuned_values());
    }

    #[test]
    fn test_tunable_parameters_with_values() {
        let params = TunableParameters {
            surface_rms_mm: Some(1.5),
            mesh_spacing_mm: None,
            mesh_wire_diameter_mm: None,
        };
        assert!(params.has_tuned_values());
    }

    #[test]
    fn test_effective_parameters() {
        let class = create_test_class();
        let params = TunableParameters {
            surface_rms_mm: Some(1.5),
            mesh_spacing_mm: None,
            mesh_wire_diameter_mm: None,
        };

        assert_eq!(params.effective_surface_rms(&class), 1.5);
        assert_eq!(params.effective_mesh_spacing(&class), 5.0); // From class
        assert_eq!(params.effective_wire_diameter(&class), 0.5); // From class
    }

    #[test]
    fn test_parameter_bounds_validation() {
        let class = create_test_class();
        let bounds = ParameterBounds::default();

        let valid_params = TunableParameters {
            surface_rms_mm: Some(1.0),
            mesh_spacing_mm: Some(5.0),
            mesh_wire_diameter_mm: Some(0.5),
        };
        assert!(bounds.validate(&valid_params, &class).is_ok());

        let invalid_params = TunableParameters {
            surface_rms_mm: Some(10.0), // Out of bounds
            mesh_spacing_mm: None,
            mesh_wire_diameter_mm: None,
        };
        assert!(bounds.validate(&invalid_params, &class).is_err());
    }

    #[test]
    fn test_antenna_configuration_new() {
        let config = AntennaConfiguration::new(
            "ant_1".to_string(),
            "Antenna 1".to_string(),
            "test_34m".to_string(),
        );

        assert_eq!(config.antenna_id, "ant_1");
        assert_eq!(config.class_id, "test_34m");
        assert!(!config.tunable_parameters.has_tuned_values());
    }

    #[test]
    fn test_antenna_configuration_serialization() {
        let config = AntennaConfiguration::new(
            "ant_1".to_string(),
            "Antenna 1".to_string(),
            "test_34m".to_string(),
        );

        let yaml = serde_yaml::to_string(&config).unwrap();
        let deserialized: AntennaConfiguration = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(config.antenna_id, deserialized.antenna_id);
        assert_eq!(config.class_id, deserialized.class_id);
    }
}
