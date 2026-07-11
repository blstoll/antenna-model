//! Design Specifications Loader
//!
//! This module provides functionality to load antenna design specifications from YAML files.
//! Design specs provide initial parameter estimates for boresight calibration when full
//! measurement data is not available.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Design specifications for an antenna reflector and feed system.
///
/// These specifications can come from:
/// - Manufacturer specifications
/// - Engineering drawings
/// - CAD models
/// - Visual inspection estimates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignSpecs {
    /// Antenna identifier
    pub antenna_id: String,

    /// Human-readable antenna name
    pub antenna_name: String,

    /// Reflector geometry
    pub reflector: ReflectorSpecs,

    /// Feed specifications (one or more feeds)
    pub feeds: Vec<FeedSpecs>,

    /// Optional mesh parameters (for mesh reflectors)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshSpecs>,
}

/// Reflector geometry specifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectorSpecs {
    /// Dish diameter in meters
    pub diameter_m: f64,

    /// Focal length in meters
    pub focal_length_m: f64,

    /// Surface RMS error in millimeters (estimate)
    pub surface_rms_mm: f64,
}

/// Feed horn specifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedSpecs {
    /// Feed identifier (e.g., "x_band", "s_band")
    pub feed_id: String,

    /// Feed name
    pub name: String,

    /// Feed position [x, y, z] in meters
    pub position: [f64; 3],

    /// q-factor for cos^q illumination pattern (estimate)
    pub q_factor: f64,

    /// Phase center offset in meters
    pub phase_center_offset_m: f64,

    /// Frequency range [min, max] in MHz
    pub frequency_range: [f64; 2],
}

/// Mesh reflector specifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshSpecs {
    /// Mesh spacing (hole size) in millimeters
    pub mesh_spacing_mm: f64,

    /// Wire diameter in millimeters
    pub wire_diameter_mm: f64,
}

impl DesignSpecs {
    /// Load design specs from a YAML file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the design specs YAML file
    ///
    /// # Example YAML format
    ///
    /// ```yaml
    /// antenna_id: "antenna_1"
    /// antenna_name: "3.7m Ground Station"
    /// reflector:
    ///   diameter_m: 3.7
    ///   focal_length_m: 1.85
    ///   surface_rms_mm: 1.5
    /// feeds:
    ///   - feed_id: "x_band"
    ///     name: "X-Band Primary Feed"
    ///     position: [0.0, 0.0, 0.0]
    ///     q_factor: 8.0
    ///     phase_center_offset_m: 0.0
    ///     frequency_range: [7100.0, 8500.0]
    /// mesh:
    ///   mesh_spacing_mm: 5.0
    ///   wire_diameter_mm: 0.5
    /// ```
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read design specs file: {}", path.display()))?;

        let specs: DesignSpecs = serde_yaml::from_str(&contents)
            .with_context(|| format!("Failed to parse design specs YAML: {}", path.display()))?;

        // Validate the specs
        specs.validate()?;

        Ok(specs)
    }

    /// Validate that design specs are physically reasonable.
    pub fn validate(&self) -> Result<()> {
        // Validate antenna ID
        if self.antenna_id.is_empty() {
            anyhow::bail!("antenna_id cannot be empty");
        }

        // Validate reflector specs
        self.reflector
            .validate()
            .context("Invalid reflector specs")?;

        // Validate feed specs
        if self.feeds.is_empty() {
            anyhow::bail!("At least one feed specification is required");
        }

        for (idx, feed) in self.feeds.iter().enumerate() {
            feed.validate()
                .with_context(|| format!("Invalid feed spec at index {}", idx))?;
        }

        // Check for duplicate feed IDs
        let mut feed_ids = std::collections::HashSet::new();
        for feed in &self.feeds {
            if !feed_ids.insert(&feed.feed_id) {
                anyhow::bail!("Duplicate feed_id: {}", feed.feed_id);
            }
        }

        // Validate mesh specs if present
        if let Some(ref mesh) = self.mesh {
            mesh.validate().context("Invalid mesh specs")?;
        }

        Ok(())
    }

    /// Get the f/D ratio from reflector specs.
    pub fn f_over_d_ratio(&self) -> f64 {
        self.reflector.focal_length_m / self.reflector.diameter_m
    }

    /// Find feed specs by feed ID.
    pub fn get_feed(&self, feed_id: &str) -> Option<&FeedSpecs> {
        self.feeds.iter().find(|f| f.feed_id == feed_id)
    }

    /// Get tuning bounds for boresight calibration.
    ///
    /// Returns reasonable bounds for parameter optimization based on design specs.
    /// These bounds allow parameters to vary within ±50% of nominal values.
    pub fn get_tuning_bounds(&self, feed_id: &str) -> Option<TuningBounds> {
        let feed = self.get_feed(feed_id)?;

        Some(TuningBounds {
            surface_rms_mm_range: (
                self.reflector.surface_rms_mm * 0.3,
                self.reflector.surface_rms_mm * 3.0,
            ),
            q_factor_range: (feed.q_factor * 0.5, feed.q_factor * 2.0),
            mesh_spacing_mm_range: self
                .mesh
                .as_ref()
                .map(|m| (m.mesh_spacing_mm * 0.5, m.mesh_spacing_mm * 2.0)),
            wire_diameter_mm_range: self
                .mesh
                .as_ref()
                .map(|m| (m.wire_diameter_mm * 0.5, m.wire_diameter_mm * 2.0)),
        })
    }
}

/// Parameter tuning bounds for boresight calibration.
#[derive(Debug, Clone)]
pub struct TuningBounds {
    /// Surface RMS range (min, max) in millimeters
    pub surface_rms_mm_range: (f64, f64),

    /// q-factor range (min, max)
    pub q_factor_range: (f64, f64),

    /// Optional mesh spacing range (min, max) in millimeters
    pub mesh_spacing_mm_range: Option<(f64, f64)>,

    /// Optional wire diameter range (min, max) in millimeters
    pub wire_diameter_mm_range: Option<(f64, f64)>,
}

impl ReflectorSpecs {
    /// Validate reflector specifications.
    fn validate(&self) -> Result<()> {
        if self.diameter_m <= 0.0 {
            anyhow::bail!("diameter_m must be positive, got {}", self.diameter_m);
        }

        if self.focal_length_m <= 0.0 {
            anyhow::bail!(
                "focal_length_m must be positive, got {}",
                self.focal_length_m
            );
        }

        if self.surface_rms_mm < 0.0 {
            anyhow::bail!(
                "surface_rms_mm must be non-negative, got {}",
                self.surface_rms_mm
            );
        }

        // Check f/D ratio is reasonable (typically 0.3 - 1.5)
        let f_over_d = self.focal_length_m / self.diameter_m;
        if !(0.2..=2.0).contains(&f_over_d) {
            anyhow::bail!(
                "f/D ratio {:.3} is outside reasonable range [0.2, 2.0]",
                f_over_d
            );
        }

        Ok(())
    }
}

impl FeedSpecs {
    /// Validate feed specifications.
    fn validate(&self) -> Result<()> {
        if self.feed_id.is_empty() {
            anyhow::bail!("feed_id cannot be empty");
        }

        if self.q_factor <= 0.0 {
            anyhow::bail!("q_factor must be positive, got {}", self.q_factor);
        }

        // Typical q_factor range is 3-15
        if self.q_factor < 1.0 || self.q_factor > 30.0 {
            anyhow::bail!(
                "q_factor {} is outside typical range [1.0, 30.0]",
                self.q_factor
            );
        }

        // Validate frequency range
        if self.frequency_range[0] >= self.frequency_range[1] {
            anyhow::bail!(
                "Invalid frequency range: [{}, {}] - min must be < max",
                self.frequency_range[0],
                self.frequency_range[1]
            );
        }

        if self.frequency_range[0] < 100.0 || self.frequency_range[1] > 50000.0 {
            anyhow::bail!(
                "Frequency range [{}, {}] MHz is outside supported range [100, 50000] MHz",
                self.frequency_range[0],
                self.frequency_range[1]
            );
        }

        Ok(())
    }
}

impl MeshSpecs {
    /// Validate mesh specifications.
    fn validate(&self) -> Result<()> {
        if self.mesh_spacing_mm <= 0.0 {
            anyhow::bail!(
                "mesh_spacing_mm must be positive, got {}",
                self.mesh_spacing_mm
            );
        }

        if self.wire_diameter_mm <= 0.0 {
            anyhow::bail!(
                "wire_diameter_mm must be positive, got {}",
                self.wire_diameter_mm
            );
        }

        // Wire diameter should be less than mesh spacing
        if self.wire_diameter_mm >= self.mesh_spacing_mm {
            anyhow::bail!(
                "wire_diameter_mm ({}) must be less than mesh_spacing_mm ({})",
                self.wire_diameter_mm,
                self.mesh_spacing_mm
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_valid_design_specs_yaml() -> String {
        r#"
antenna_id: "test_antenna"
antenna_name: "Test 3.7m Ground Station"
reflector:
  diameter_m: 3.7
  focal_length_m: 1.85
  surface_rms_mm: 1.5
feeds:
  - feed_id: "x_band"
    name: "X-Band Primary Feed"
    position: [0.0, 0.0, 0.0]
    q_factor: 8.0
    phase_center_offset_m: 0.0
    frequency_range: [7100.0, 8500.0]
  - feed_id: "s_band"
    name: "S-Band Feed"
    position: [0.05, 0.0, 0.0]
    q_factor: 7.0
    phase_center_offset_m: 0.0
    frequency_range: [2000.0, 2300.0]
mesh:
  mesh_spacing_mm: 5.0
  wire_diameter_mm: 0.5
"#
        .to_string()
    }

    #[test]
    fn test_load_valid_design_specs() {
        let yaml_content = create_valid_design_specs_yaml();
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let specs = DesignSpecs::load_from_file(temp_file.path()).unwrap();

        assert_eq!(specs.antenna_id, "test_antenna");
        assert_eq!(specs.antenna_name, "Test 3.7m Ground Station");
        assert_eq!(specs.reflector.diameter_m, 3.7);
        assert_eq!(specs.reflector.focal_length_m, 1.85);
        assert_eq!(specs.reflector.surface_rms_mm, 1.5);
        assert_eq!(specs.feeds.len(), 2);
        assert!(specs.mesh.is_some());
    }

    #[test]
    fn test_f_over_d_ratio() {
        let yaml_content = create_valid_design_specs_yaml();
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let specs = DesignSpecs::load_from_file(temp_file.path()).unwrap();

        let f_over_d = specs.f_over_d_ratio();
        assert!((f_over_d - 0.5).abs() < 1e-6); // 1.85 / 3.7 = 0.5
    }

    #[test]
    fn test_get_feed() {
        let yaml_content = create_valid_design_specs_yaml();
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let specs = DesignSpecs::load_from_file(temp_file.path()).unwrap();

        let x_band = specs.get_feed("x_band");
        assert!(x_band.is_some());
        assert_eq!(x_band.unwrap().q_factor, 8.0);

        let s_band = specs.get_feed("s_band");
        assert!(s_band.is_some());
        assert_eq!(s_band.unwrap().q_factor, 7.0);

        let nonexistent = specs.get_feed("ka_band");
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_get_tuning_bounds() {
        let yaml_content = create_valid_design_specs_yaml();
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let specs = DesignSpecs::load_from_file(temp_file.path()).unwrap();

        let bounds = specs.get_tuning_bounds("x_band").unwrap();

        // Surface RMS bounds: 1.5 * [0.3, 3.0] = [0.45, 4.5]
        assert!((bounds.surface_rms_mm_range.0 - 0.45).abs() < 1e-6);
        assert!((bounds.surface_rms_mm_range.1 - 4.5).abs() < 1e-6);

        // q_factor bounds: 8.0 * [0.5, 2.0] = [4.0, 16.0]
        assert!((bounds.q_factor_range.0 - 4.0).abs() < 1e-6);
        assert!((bounds.q_factor_range.1 - 16.0).abs() < 1e-6);

        // Mesh spacing bounds: 5.0 * [0.5, 2.0] = [2.5, 10.0]
        assert!(bounds.mesh_spacing_mm_range.is_some());
        let mesh_bounds = bounds.mesh_spacing_mm_range.unwrap();
        assert!((mesh_bounds.0 - 2.5).abs() < 1e-6);
        assert!((mesh_bounds.1 - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_invalid_diameter() {
        let yaml_content = r#"
antenna_id: "test"
antenna_name: "Test"
reflector:
  diameter_m: -1.0
  focal_length_m: 1.0
  surface_rms_mm: 1.0
feeds:
  - feed_id: "x_band"
    name: "X-Band"
    position: [0.0, 0.0, 0.0]
    q_factor: 8.0
    phase_center_offset_m: 0.0
    frequency_range: [7100.0, 8500.0]
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = DesignSpecs::load_from_file(temp_file.path());
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("diameter_m must be positive"));
    }

    #[test]
    fn test_invalid_f_over_d_ratio() {
        let yaml_content = r#"
antenna_id: "test"
antenna_name: "Test"
reflector:
  diameter_m: 1.0
  focal_length_m: 3.0  # f/D = 3.0, too high
  surface_rms_mm: 1.0
feeds:
  - feed_id: "x_band"
    name: "X-Band"
    position: [0.0, 0.0, 0.0]
    q_factor: 8.0
    phase_center_offset_m: 0.0
    frequency_range: [7100.0, 8500.0]
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = DesignSpecs::load_from_file(temp_file.path());
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("f/D ratio"));
    }

    #[test]
    fn test_duplicate_feed_ids() {
        let yaml_content = r#"
antenna_id: "test"
antenna_name: "Test"
reflector:
  diameter_m: 3.7
  focal_length_m: 1.85
  surface_rms_mm: 1.5
feeds:
  - feed_id: "x_band"
    name: "X-Band Primary"
    position: [0.0, 0.0, 0.0]
    q_factor: 8.0
    phase_center_offset_m: 0.0
    frequency_range: [7100.0, 8500.0]
  - feed_id: "x_band"  # Duplicate!
    name: "X-Band Secondary"
    position: [0.05, 0.0, 0.0]
    q_factor: 7.0
    phase_center_offset_m: 0.0
    frequency_range: [7100.0, 8500.0]
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = DesignSpecs::load_from_file(temp_file.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate feed_id"));
    }

    #[test]
    fn test_invalid_frequency_range() {
        let yaml_content = r#"
antenna_id: "test"
antenna_name: "Test"
reflector:
  diameter_m: 3.7
  focal_length_m: 1.85
  surface_rms_mm: 1.5
feeds:
  - feed_id: "x_band"
    name: "X-Band"
    position: [0.0, 0.0, 0.0]
    q_factor: 8.0
    phase_center_offset_m: 0.0
    frequency_range: [8500.0, 7100.0]  # Reversed!
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = DesignSpecs::load_from_file(temp_file.path());
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("Invalid frequency range"));
    }

    #[test]
    fn test_wire_diameter_exceeds_mesh_spacing() {
        let yaml_content = r#"
antenna_id: "test"
antenna_name: "Test"
reflector:
  diameter_m: 3.7
  focal_length_m: 1.85
  surface_rms_mm: 1.5
feeds:
  - feed_id: "x_band"
    name: "X-Band"
    position: [0.0, 0.0, 0.0]
    q_factor: 8.0
    phase_center_offset_m: 0.0
    frequency_range: [7100.0, 8500.0]
mesh:
  mesh_spacing_mm: 2.0
  wire_diameter_mm: 3.0  # Exceeds spacing!
"#;
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(yaml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = DesignSpecs::load_from_file(temp_file.path());
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("wire_diameter_mm"));
    }

    /// The shipped example that the docs point `--design-specs` at must actually parse and
    /// validate against this loader's schema. Guards against the file silently rotting out of
    /// sync with `DesignSpecs` (the exact failure that left three orphaned files here before).
    #[test]
    fn example_design_specs_file_loads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../calibration_data/design_specs/small_groundstation.yaml");
        let specs = DesignSpecs::load_from_file(&path)
            .unwrap_or_else(|e| panic!("example design specs {} must load: {e:?}", path.display()));
        assert_eq!(specs.antenna_id, "gs_3.7m");
        assert_eq!(specs.feeds.len(), 2);
        // load_from_file already calls validate(); assert once more for intent.
        specs
            .validate()
            .expect("example design specs must validate");
    }
}
