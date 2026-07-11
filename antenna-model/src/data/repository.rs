//! Calibration data repository
//!
//! This module provides thread-safe access to loaded calibration data.

use crate::config::{AntennaConfig, AntennaConfigEntry, CalibrationConfig, FeedSpecConfig};
use crate::data::loader::load_calibration_artifact;
use crate::data::types::{
    AntennaCalibration, BSplineModel4D, CalibrationMetadata, CalibrationStatus, FeedParameters,
    MeshParameters, PhysicalAntennaConfig, ReflectorGeometry, ValidityRanges,
};
use crate::error::DataError;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Thread-safe repository for calibration data
///
/// Manages loaded calibration data for multiple antennas and feeds.
/// Uses composite (antenna_id, feed_id) identifiers for lookups.
///
/// # Thread Safety
/// All methods use read-write locks for thread-safe concurrent access.
/// Multiple readers can access data simultaneously, but writes are exclusive.
///
/// # Example
/// ```no_run
/// use antenna_model::config::CalibrationConfig;
/// use antenna_model::data::repository::CalibrationRepository;
///
/// let config = CalibrationConfig::default();
/// let repo = CalibrationRepository::load_from_config(&config)?;
///
/// // List all antennas
/// for antenna_id in repo.list_antennas() {
///     println!("Antenna: {}", antenna_id);
///     for feed_id in repo.list_feeds(&antenna_id) {
///         println!("  Feed: {}", feed_id);
///     }
/// }
///
/// // Get calibration for specific antenna-feed
/// if let Some(cal) = repo.get_calibration("antenna_1", "x_band") {
///     println!("Found calibration for antenna_1:x_band");
/// }
/// # Ok::<(), antenna_model::error::DataError>(())
/// ```
#[derive(Clone)]
pub struct CalibrationRepository {
    /// Nested map: antenna_id -> feed_id -> calibration
    data: Arc<RwLock<HashMap<String, HashMap<String, AntennaCalibration>>>>,
}

impl CalibrationRepository {
    /// Create a new empty repository
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load calibrations from configuration
    ///
    /// Reads the antenna configuration file and loads all enabled calibration artifacts.
    ///
    /// # Arguments
    /// * `config` - Calibration configuration specifying data directory and antenna list
    ///
    /// # Returns
    /// * `Ok(CalibrationRepository)` - Successfully loaded repository
    /// * `Err(DataError)` - Failed to load calibrations
    ///
    /// # Behavior
    /// - If `fail_fast` is true, any loading error will abort the entire operation
    /// - If `fail_fast` is false, errors are logged as warnings and loading continues
    pub fn load_from_config(config: &CalibrationConfig) -> Result<Self, DataError> {
        info!(
            "Loading calibration data from: {}",
            config.data_directory.display()
        );

        // Load antenna configuration
        let antenna_config =
            AntennaConfig::from_file(config.antenna_config_file.to_string_lossy().as_ref())?;

        let enabled_antennas = antenna_config.enabled_antennas();
        info!(
            "Found {} enabled antenna(s) in configuration",
            enabled_antennas.len()
        );

        let mut repository = Self::new();
        let mut loaded_count = 0;
        let mut error_count = 0;

        for entry in enabled_antennas {
            // Determine how to load this antenna based on calibration_file presence
            match repository.load_antenna(entry, config) {
                Ok(count) => {
                    loaded_count += count;
                }
                Err(e) => {
                    if config.fail_fast {
                        return Err(e);
                    } else {
                        warn!("Failed to load antenna '{}': {}", entry.id, e);
                        error_count += 1;
                    }
                }
            }
        }

        info!(
            "Loaded {} calibration(s), {} error(s)",
            loaded_count, error_count
        );

        if loaded_count == 0 {
            return Err(DataError::ConfigurationError {
                reason: "No calibrations loaded".to_string(),
            });
        }

        Ok(repository)
    }

    /// Load a single antenna from configuration (supports all calibration statuses)
    ///
    /// # Arguments
    /// * `entry` - Antenna configuration entry
    /// * `config` - Calibration configuration (for data directory path)
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of calibrations loaded (equals number of feeds)
    /// * `Err(DataError)` - Failed to load antenna
    ///
    /// # Behavior
    /// - If `calibration_file` is present: loads from binary file (fully/partially calibrated)
    /// - If `calibration_file` is absent: constructs from design specs (uncalibrated)
    fn load_antenna(
        &mut self,
        entry: &AntennaConfigEntry,
        config: &CalibrationConfig,
    ) -> Result<usize, DataError> {
        match &entry.calibration_file {
            Some(calibration_file) => {
                // Load from calibration file (fully or partially calibrated)
                debug!(
                    "Loading calibration for antenna '{}' from '{}'",
                    entry.id, calibration_file
                );

                let calibration_path = config.data_directory.join(calibration_file);
                let calibration = load_calibration_artifact(&calibration_path)?;

                // Verify antenna_id matches configuration
                if calibration.antenna_id != entry.id {
                    return Err(DataError::ConfigurationError {
                        reason: format!(
                            "Antenna ID mismatch: config='{}', file='{}'",
                            entry.id, calibration.antenna_id
                        ),
                    });
                }

                self.add_calibration(calibration);
                Ok(1)
            }
            None => {
                // Load from design specs (uncalibrated)
                debug!(
                    "Loading uncalibrated antenna '{}' from design specs",
                    entry.id
                );
                self.load_uncalibrated_antenna(entry)
            }
        }
    }

    /// Load uncalibrated antenna from design specifications
    ///
    /// # Arguments
    /// * `entry` - Antenna configuration entry with design specs
    ///
    /// # Returns
    /// * `Ok(usize)` - Number of calibrations loaded (equals number of feeds)
    /// * `Err(DataError)` - Failed to load antenna
    ///
    /// # Behavior
    /// - Constructs `PhysicalAntennaConfig` from design specs for each feed
    /// - Builds `AntennaCalibration` with `Uncalibrated` status
    /// - No correction surface (physics model only)
    fn load_uncalibrated_antenna(
        &mut self,
        entry: &AntennaConfigEntry,
    ) -> Result<usize, DataError> {
        let design = entry
            .design_specs
            .as_ref()
            .ok_or_else(|| DataError::ConfigurationError {
                reason: format!("Uncalibrated antenna '{}' requires design_specs", entry.id),
            })?;

        let mut loaded_count = 0;

        // Build calibration for each feed
        for feed_spec in &design.feeds {
            debug!(
                "Loading feed '{}' for uncalibrated antenna '{}'",
                feed_spec.id, entry.id
            );

            // Build PhysicalAntennaConfig from design specs
            let reflector = ReflectorGeometry {
                diameter_m: design.diameter_m,
                focal_length_m: design.focal_length_m,
                f_over_d_ratio: design.f_over_d_ratio,
                surface_rms_mm: design.surface_rms_mm,
            };

            let feed = FeedParameters {
                position: (
                    feed_spec.position[0],
                    feed_spec.position[1],
                    feed_spec.position[2],
                ),
                q_factor: feed_spec.q_factor,
                phase_center_offset_m: feed_spec.phase_center_offset_m,
            };

            let mesh = design.mesh.as_ref().map(|m| MeshParameters {
                mesh_spacing_mm: m.mesh_spacing_mm,
                wire_diameter_mm: m.wire_diameter_mm,
            });

            let physical_config = PhysicalAntennaConfig {
                reflector,
                feed,
                mesh,
            };

            // Build calibration metadata
            let metadata = CalibrationMetadata {
                antenna_name: format!("{} - {}", entry.name, feed_spec.name),
                calibration_date: "N/A".to_string(),
                format_version: "2.0".to_string(),
                data_source: "design_specifications".to_string(),
                rmse_db: f64::NAN,
                r_squared: f64::NAN,
                num_measurements: 0,
                notes: Some("Uncalibrated - using design specifications".to_string()),
                physics_only_rmse_db: None,
                correction_improvement_db: None,
                parameters_tuned: false,
                antenna_class: None,
                parameters_source: Some(crate::data::types::ParameterSource::DesignSpecifications),
                measurement_density: Some(crate::data::types::MeasurementDensity::None),
                physics_model_version: crate::model::PHYSICS_MODEL_VERSION,
            };

            // Build validity ranges (use config override or default from design)
            let validity_ranges = build_validity_ranges(entry, feed_spec);

            // Build calibration with Uncalibrated status
            let calibration = AntennaCalibration {
                antenna_id: entry.id.clone(),
                feed_id: feed_spec.id.clone(),
                metadata,
                physical_config,
                correction_surface: None,
                validity_ranges,
                calibration_status: Some(CalibrationStatus::Uncalibrated {
                    accuracy_estimate_db: 3.0,
                    loss_accuracy_estimate_db: 2.0,
                }),
                calibration_coverage: None,
            };

            self.add_calibration(calibration);
            loaded_count += 1;
        }

        info!(
            "Loaded {} feed(s) for uncalibrated antenna '{}'",
            loaded_count, entry.id
        );

        Ok(loaded_count)
    }

    /// Add a calibration to the repository
    ///
    /// # Arguments
    /// * `calibration` - Calibration to add
    pub fn add_calibration(&mut self, calibration: AntennaCalibration) {
        let antenna_id = calibration.antenna_id.clone();
        let feed_id = calibration.feed_id.clone();

        let mut data = self.data.write();
        let antenna_map = data.entry(antenna_id.clone()).or_default();
        antenna_map.insert(feed_id.clone(), calibration);

        debug!("Added calibration: {}:{}", antenna_id, feed_id);
    }

    /// Get a calibration for a specific antenna and feed
    ///
    /// # Arguments
    /// * `antenna_id` - Antenna identifier
    /// * `feed_id` - Feed identifier
    ///
    /// # Returns
    /// * `Some(AntennaCalibration)` - Calibration found
    /// * `None` - No calibration for this antenna-feed combination
    pub fn get_calibration(&self, antenna_id: &str, feed_id: &str) -> Option<AntennaCalibration> {
        let data = self.data.read();
        data.get(antenna_id)
            .and_then(|feeds| feeds.get(feed_id))
            .cloned()
    }

    /// Get the physical antenna configuration for a specific antenna and feed
    ///
    /// # Arguments
    /// * `antenna_id` - Antenna identifier
    /// * `feed_id` - Feed identifier
    ///
    /// # Returns
    /// * `Some(PhysicalAntennaConfig)` - Physical configuration found
    /// * `None` - No calibration for this antenna-feed combination
    pub fn get_antenna_config(
        &self,
        antenna_id: &str,
        feed_id: &str,
    ) -> Option<PhysicalAntennaConfig> {
        self.get_calibration(antenna_id, feed_id)
            .map(|cal| cal.physical_config)
    }

    /// Get the correction surface for a specific antenna and feed
    ///
    /// # Arguments
    /// * `antenna_id` - Antenna identifier
    /// * `feed_id` - Feed identifier
    ///
    /// # Returns
    /// * `Some(Option<BSplineModel4D>)` - Calibration found (may or may not have correction surface)
    /// * `None` - No calibration for this antenna-feed combination
    pub fn get_correction_surface(
        &self,
        antenna_id: &str,
        feed_id: &str,
    ) -> Option<Option<BSplineModel4D>> {
        self.get_calibration(antenna_id, feed_id)
            .map(|cal| cal.correction_surface)
    }

    /// Get the validity ranges for a specific antenna and feed
    ///
    /// # Arguments
    /// * `antenna_id` - Antenna identifier
    /// * `feed_id` - Feed identifier
    ///
    /// # Returns
    /// * `Some(ValidityRanges)` - Validity ranges found
    /// * `None` - No calibration for this antenna-feed combination
    pub fn get_validity_ranges(&self, antenna_id: &str, feed_id: &str) -> Option<ValidityRanges> {
        self.get_calibration(antenna_id, feed_id)
            .map(|cal| cal.validity_ranges)
    }

    /// List all antenna IDs in the repository
    ///
    /// # Returns
    /// Vector of antenna IDs, sorted alphabetically
    pub fn list_antennas(&self) -> Vec<String> {
        let data = self.data.read();
        let mut antennas: Vec<String> = data.keys().cloned().collect();
        antennas.sort();
        antennas
    }

    /// List all feed IDs for a specific antenna
    ///
    /// # Arguments
    /// * `antenna_id` - Antenna identifier
    ///
    /// # Returns
    /// Vector of feed IDs for this antenna, sorted alphabetically.
    /// Returns empty vector if antenna not found.
    pub fn list_feeds(&self, antenna_id: &str) -> Vec<String> {
        let data = self.data.read();
        data.get(antenna_id)
            .map(|feeds| {
                let mut feed_ids: Vec<String> = feeds.keys().cloned().collect();
                feed_ids.sort();
                feed_ids
            })
            .unwrap_or_default()
    }

    /// Check if a specific antenna-feed combination exists
    ///
    /// # Arguments
    /// * `antenna_id` - Antenna identifier
    /// * `feed_id` - Feed identifier
    ///
    /// # Returns
    /// * `true` - Calibration exists
    /// * `false` - No calibration for this combination
    pub fn has_calibration(&self, antenna_id: &str, feed_id: &str) -> bool {
        let data = self.data.read();
        data.get(antenna_id)
            .map(|feeds| feeds.contains_key(feed_id))
            .unwrap_or(false)
    }

    /// Get the total number of loaded calibrations
    pub fn calibration_count(&self) -> usize {
        let data = self.data.read();
        data.values().map(|feeds| feeds.len()).sum()
    }

    /// Get the number of antennas
    pub fn antenna_count(&self) -> usize {
        let data = self.data.read();
        data.len()
    }
}

impl Default for CalibrationRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// Build validity ranges for an uncalibrated antenna feed
///
/// Uses config override if present, otherwise constructs from design specs.
fn build_validity_ranges(entry: &AntennaConfigEntry, feed_spec: &FeedSpecConfig) -> ValidityRanges {
    if let Some(ref validity_config) = entry.validity_ranges {
        // Use explicit validity ranges from config
        ValidityRanges {
            azimuth_min_max: (
                validity_config.azimuth_range[0],
                validity_config.azimuth_range[1],
            ),
            elevation_min_max: (
                validity_config.elevation_range[0],
                validity_config.elevation_range[1],
            ),
            frequency_min_max: (
                validity_config.frequency_range[0],
                validity_config.frequency_range[1],
            ),
            temperature_const: validity_config.temperature_k,
        }
    } else {
        // Default validity ranges from feed frequency range
        ValidityRanges {
            azimuth_min_max: (0.0, 360.0),
            elevation_min_max: (0.0, 90.0),
            frequency_min_max: (feed_spec.frequency_range[0], feed_spec.frequency_range[1]),
            temperature_const: 290.0, // Assume room temperature
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::types::{CalibrationMetadata, FeedParameters, ReflectorGeometry};
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn create_test_calibration(antenna_id: &str, feed_id: &str) -> AntennaCalibration {
        let metadata = CalibrationMetadata::builder()
            .antenna_name(format!("{} {}", antenna_id, feed_id))
            .calibration_date("2025-01-15T00:00:00Z")
            .data_source("test_data.csv")
            .rmse_db(0.5)
            .r_squared(0.98)
            .num_measurements(1000)
            .build()
            .unwrap();

        let reflector = ReflectorGeometry::builder()
            .diameter_m(34.0)
            .focal_length_m(13.6)
            .f_over_d_ratio(0.4)
            .surface_rms_mm(0.5)
            .build()
            .unwrap();

        let feed = FeedParameters::builder()
            .position(0.0, 0.0, 0.1)
            .q_factor(8.0)
            .phase_center_offset_m(0.0)
            .build()
            .unwrap();

        let physical_config = PhysicalAntennaConfig::builder()
            .reflector(reflector)
            .feed(feed)
            .build()
            .unwrap();

        let ranges = ValidityRanges::builder()
            .azimuth_range(0.0, 360.0)
            .elevation_range(10.0, 80.0)
            .frequency_range(8000.0, 8500.0)
            .temperature(290.0)
            .build()
            .unwrap();

        AntennaCalibration::builder()
            .antenna_id(antenna_id)
            .feed_id(feed_id)
            .metadata(metadata)
            .physical_config(physical_config)
            .validity_ranges(ranges)
            .build()
            .unwrap()
    }

    fn write_calibration_file(calibration: &AntennaCalibration) -> NamedTempFile {
        let mut temp_file = NamedTempFile::new().unwrap();
        let config = bincode::config::standard();
        let encoded = bincode::encode_to_vec(calibration, config).unwrap();
        temp_file.write_all(&encoded).unwrap();
        temp_file.flush().unwrap();
        temp_file
    }

    #[test]
    fn test_repository_new() {
        let repo = CalibrationRepository::new();
        assert_eq!(repo.calibration_count(), 0);
        assert_eq!(repo.antenna_count(), 0);
    }

    #[test]
    fn test_add_calibration() {
        let mut repo = CalibrationRepository::new();
        let cal = create_test_calibration("antenna_1", "x_band");

        repo.add_calibration(cal);

        assert_eq!(repo.calibration_count(), 1);
        assert_eq!(repo.antenna_count(), 1);
        assert!(repo.has_calibration("antenna_1", "x_band"));
    }

    #[test]
    fn test_add_multiple_feeds_same_antenna() {
        let mut repo = CalibrationRepository::new();
        let cal1 = create_test_calibration("antenna_1", "x_band");
        let cal2 = create_test_calibration("antenna_1", "s_band");

        repo.add_calibration(cal1);
        repo.add_calibration(cal2);

        assert_eq!(repo.calibration_count(), 2);
        assert_eq!(repo.antenna_count(), 1);
        assert!(repo.has_calibration("antenna_1", "x_band"));
        assert!(repo.has_calibration("antenna_1", "s_band"));
    }

    #[test]
    fn test_add_multiple_antennas() {
        let mut repo = CalibrationRepository::new();
        let cal1 = create_test_calibration("antenna_1", "x_band");
        let cal2 = create_test_calibration("antenna_2", "s_band");

        repo.add_calibration(cal1);
        repo.add_calibration(cal2);

        assert_eq!(repo.calibration_count(), 2);
        assert_eq!(repo.antenna_count(), 2);
    }

    #[test]
    fn test_get_calibration() {
        let mut repo = CalibrationRepository::new();
        let cal = create_test_calibration("antenna_1", "x_band");
        repo.add_calibration(cal.clone());

        let retrieved = repo.get_calibration("antenna_1", "x_band").unwrap();
        assert_eq!(retrieved.antenna_id, "antenna_1");
        assert_eq!(retrieved.feed_id, "x_band");
    }

    #[test]
    fn test_get_calibration_not_found() {
        let repo = CalibrationRepository::new();
        assert!(repo.get_calibration("nonexistent", "feed").is_none());
    }

    #[test]
    fn test_get_antenna_config() {
        let mut repo = CalibrationRepository::new();
        let cal = create_test_calibration("antenna_1", "x_band");
        repo.add_calibration(cal);

        let config = repo.get_antenna_config("antenna_1", "x_band").unwrap();
        assert_eq!(config.reflector.diameter_m, 34.0);
    }

    #[test]
    fn test_get_correction_surface() {
        let mut repo = CalibrationRepository::new();
        let cal = create_test_calibration("antenna_1", "x_band");
        repo.add_calibration(cal);

        let correction = repo.get_correction_surface("antenna_1", "x_band").unwrap();
        assert!(correction.is_none()); // Test calibration has no correction surface
    }

    #[test]
    fn test_get_validity_ranges() {
        let mut repo = CalibrationRepository::new();
        let cal = create_test_calibration("antenna_1", "x_band");
        repo.add_calibration(cal);

        let ranges = repo.get_validity_ranges("antenna_1", "x_band").unwrap();
        assert_eq!(ranges.frequency_min_max, (8000.0, 8500.0));
    }

    #[test]
    fn test_list_antennas() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration("antenna_2", "x_band"));
        repo.add_calibration(create_test_calibration("antenna_1", "s_band"));
        repo.add_calibration(create_test_calibration("antenna_3", "ka_band"));

        let antennas = repo.list_antennas();
        assert_eq!(antennas.len(), 3);
        // Should be sorted
        assert_eq!(antennas, vec!["antenna_1", "antenna_2", "antenna_3"]);
    }

    #[test]
    fn test_list_feeds() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration("antenna_1", "x_band"));
        repo.add_calibration(create_test_calibration("antenna_1", "s_band"));
        repo.add_calibration(create_test_calibration("antenna_1", "ka_band"));

        let feeds = repo.list_feeds("antenna_1");
        assert_eq!(feeds.len(), 3);
        // Should be sorted
        assert_eq!(feeds, vec!["ka_band", "s_band", "x_band"]);
    }

    #[test]
    fn test_list_feeds_empty() {
        let repo = CalibrationRepository::new();
        let feeds = repo.list_feeds("nonexistent");
        assert_eq!(feeds.len(), 0);
    }

    #[test]
    fn test_has_calibration() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration("antenna_1", "x_band"));

        assert!(repo.has_calibration("antenna_1", "x_band"));
        assert!(!repo.has_calibration("antenna_1", "s_band"));
        assert!(!repo.has_calibration("antenna_2", "x_band"));
    }

    #[test]
    fn test_load_from_config() {
        // Create temporary directory structure
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        // Create calibration files
        let cal1 = create_test_calibration("antenna_1", "x_band");
        let cal2 = create_test_calibration("antenna_2", "s_band");

        let file1 = write_calibration_file(&cal1);
        let file2 = write_calibration_file(&cal2);

        // Copy to temp directory
        std::fs::copy(file1.path(), data_dir.join("antenna_1.bin")).unwrap();
        std::fs::copy(file2.path(), data_dir.join("antenna_2.bin")).unwrap();

        // Create antenna config YAML
        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_1"
    name: "Test Antenna 1"
    calibration_file: "antenna_1.bin"
    enabled: true
  - id: "antenna_2"
    name: "Test Antenna 2"
    calibration_file: "antenna_2.bin"
    enabled: true
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        // Create calibration config
        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        // Load repository
        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        assert_eq!(repo.calibration_count(), 2);
        assert_eq!(repo.antenna_count(), 2);
        assert!(repo.has_calibration("antenna_1", "x_band"));
        assert!(repo.has_calibration("antenna_2", "s_band"));
    }

    #[test]
    fn test_load_from_config_with_disabled_antenna() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let cal1 = create_test_calibration("antenna_1", "x_band");
        let file1 = write_calibration_file(&cal1);
        std::fs::copy(file1.path(), data_dir.join("antenna_1.bin")).unwrap();

        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_1"
    name: "Test Antenna 1"
    calibration_file: "antenna_1.bin"
    enabled: false
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        // Should fail because no enabled antennas
        let result = CalibrationRepository::load_from_config(&calibration_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_from_config_fail_fast() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_1"
    name: "Test Antenna 1"
    calibration_file: "nonexistent.bin"
    enabled: true
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        // Should fail fast on missing file
        let result = CalibrationRepository::load_from_config(&calibration_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_repository_clone() {
        let mut repo = CalibrationRepository::new();
        repo.add_calibration(create_test_calibration("antenna_1", "x_band"));

        // Clone the repository
        let repo_clone = repo.clone();

        // Both should have the same data
        assert_eq!(repo.calibration_count(), 1);
        assert_eq!(repo_clone.calibration_count(), 1);
        assert!(repo_clone.has_calibration("antenna_1", "x_band"));
    }

    // ============================================================================
    // Task 6.6 Tests: Uncalibrated Antenna Loading
    // ============================================================================

    #[test]
    fn test_load_uncalibrated_antenna_single_feed() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_uncal_1"
    name: "Uncalibrated Test Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.5
      feeds:
        - id: "x_band"
          name: "X-Band Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [7100.0, 8500.0]
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        // Verify repository loaded uncalibrated antenna
        assert_eq!(repo.calibration_count(), 1);
        assert_eq!(repo.antenna_count(), 1);
        assert!(repo.has_calibration("antenna_uncal_1", "x_band"));

        // Verify calibration details
        let cal = repo.get_calibration("antenna_uncal_1", "x_band").unwrap();
        assert_eq!(cal.antenna_id, "antenna_uncal_1");
        assert_eq!(cal.feed_id, "x_band");
        assert_eq!(cal.physical_config.reflector.diameter_m, 3.7);
        assert_eq!(cal.physical_config.reflector.focal_length_m, 1.85);
        assert_eq!(cal.physical_config.reflector.f_over_d_ratio, 0.5);
        assert_eq!(cal.physical_config.reflector.surface_rms_mm, 1.5);
        assert_eq!(cal.physical_config.feed.q_factor, 8.0);
        assert!(cal.physical_config.mesh.is_none());
        assert!(cal.correction_surface.is_none());

        // Verify calibration status
        let status = cal.calibration_status.as_ref().unwrap();
        match status {
            CalibrationStatus::Uncalibrated {
                accuracy_estimate_db,
                loss_accuracy_estimate_db,
            } => {
                assert_eq!(*accuracy_estimate_db, 3.0);
                assert_eq!(*loss_accuracy_estimate_db, 2.0);
            }
            _ => panic!("Expected Uncalibrated status"),
        }

        // Verify validity ranges
        assert_eq!(cal.validity_ranges.azimuth_min_max, (0.0, 360.0));
        assert_eq!(cal.validity_ranges.elevation_min_max, (0.0, 90.0));
        assert_eq!(cal.validity_ranges.frequency_min_max, (7100.0, 8500.0));
        assert_eq!(cal.validity_ranges.temperature_const, 290.0);

        // Verify metadata
        assert_eq!(cal.metadata.data_source, "design_specifications");
        assert_eq!(cal.metadata.num_measurements, 0);
        assert!(!cal.metadata.parameters_tuned);
    }

    #[test]
    fn test_load_uncalibrated_antenna_multi_feed() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_multi"
    name: "Multi-Feed Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 12.0
      focal_length_m: 4.8
      f_over_d_ratio: 0.4
      surface_rms_mm: 0.8
      feeds:
        - id: "s_band"
          name: "S-Band"
          position: [0.05, 0.0, 0.0]
          q_factor: 7.0
          phase_center_offset_m: 0.0
          frequency_range: [2000.0, 2300.0]
        - id: "x_band"
          name: "X-Band"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.5
          phase_center_offset_m: 0.01
          frequency_range: [8000.0, 8500.0]
        - id: "ka_band"
          name: "Ka-Band"
          position: [-0.03, 0.0, 0.0]
          q_factor: 10.0
          phase_center_offset_m: 0.005
          frequency_range: [32000.0, 34000.0]
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        // Verify all feeds loaded
        assert_eq!(repo.calibration_count(), 3);
        assert_eq!(repo.antenna_count(), 1);
        assert!(repo.has_calibration("antenna_multi", "s_band"));
        assert!(repo.has_calibration("antenna_multi", "x_band"));
        assert!(repo.has_calibration("antenna_multi", "ka_band"));

        // Verify each feed has correct parameters
        let s_band = repo.get_calibration("antenna_multi", "s_band").unwrap();
        assert_eq!(s_band.physical_config.feed.position.0, 0.05);
        assert_eq!(s_band.physical_config.feed.q_factor, 7.0);
        assert_eq!(s_band.validity_ranges.frequency_min_max, (2000.0, 2300.0));

        let x_band = repo.get_calibration("antenna_multi", "x_band").unwrap();
        assert_eq!(x_band.physical_config.feed.position.0, 0.0);
        assert_eq!(x_band.physical_config.feed.q_factor, 8.5);
        assert_eq!(x_band.physical_config.feed.phase_center_offset_m, 0.01);
        assert_eq!(x_band.validity_ranges.frequency_min_max, (8000.0, 8500.0));

        let ka_band = repo.get_calibration("antenna_multi", "ka_band").unwrap();
        assert_eq!(ka_band.physical_config.feed.position.0, -0.03);
        assert_eq!(ka_band.physical_config.feed.q_factor, 10.0);
        assert_eq!(
            ka_band.validity_ranges.frequency_min_max,
            (32000.0, 34000.0)
        );

        // All should share same reflector geometry
        assert_eq!(s_band.physical_config.reflector.diameter_m, 12.0);
        assert_eq!(x_band.physical_config.reflector.diameter_m, 12.0);
        assert_eq!(ka_band.physical_config.reflector.diameter_m, 12.0);
    }

    #[test]
    fn test_load_uncalibrated_antenna_with_mesh() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "mesh_antenna"
    name: "Mesh Reflector Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 5.0
      focal_length_m: 2.0
      f_over_d_ratio: 0.4
      surface_rms_mm: 2.0
      feeds:
        - id: "primary"
          name: "Primary Feed"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [2000.0, 2500.0]
      mesh:
        mesh_spacing_mm: 5.0
        wire_diameter_mm: 0.5
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        let cal = repo.get_calibration("mesh_antenna", "primary").unwrap();

        // Verify mesh parameters loaded
        assert!(cal.physical_config.mesh.is_some());
        let mesh = cal.physical_config.mesh.as_ref().unwrap();
        assert_eq!(mesh.mesh_spacing_mm, 5.0);
        assert_eq!(mesh.wire_diameter_mm, 0.5);
    }

    #[test]
    fn test_load_uncalibrated_antenna_with_validity_ranges() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "custom_ranges"
    name: "Custom Validity Ranges"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 4.0
      focal_length_m: 1.6
      f_over_d_ratio: 0.4
      surface_rms_mm: 1.0
      feeds:
        - id: "feed1"
          name: "Feed 1"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 9000.0]
    validity_ranges:
      azimuth_range: [0.0, 180.0]
      elevation_range: [10.0, 80.0]
      frequency_range: [7500.0, 9500.0]
      temperature_k: 300.0
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        let cal = repo.get_calibration("custom_ranges", "feed1").unwrap();

        // Verify custom validity ranges override defaults
        assert_eq!(cal.validity_ranges.azimuth_min_max, (0.0, 180.0));
        assert_eq!(cal.validity_ranges.elevation_min_max, (10.0, 80.0));
        assert_eq!(cal.validity_ranges.frequency_min_max, (7500.0, 9500.0));
        assert_eq!(cal.validity_ranges.temperature_const, 300.0);
    }

    #[test]
    fn test_load_uncalibrated_missing_design_specs() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "bad_antenna"
    name: "Missing Design Specs"
    calibration_status: "uncalibrated"
    enabled: true
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        // Should fail validation during config parsing
        let result = CalibrationRepository::load_from_config(&calibration_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_mixed_calibrated_and_uncalibrated() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        // Create a calibration file for the calibrated antenna
        let cal = create_test_calibration("antenna_calibrated", "x_band");
        let file = write_calibration_file(&cal);
        std::fs::copy(file.path(), data_dir.join("antenna_cal.bin")).unwrap();

        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_calibrated"
    name: "Fully Calibrated Antenna"
    calibration_file: "antenna_cal.bin"
    enabled: true
  - id: "antenna_uncalibrated"
    name: "Uncalibrated Antenna"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.0
      focal_length_m: 1.2
      f_over_d_ratio: 0.4
      surface_rms_mm: 1.0
      feeds:
        - id: "primary"
          name: "Primary"
          position: [0.0, 0.0, 0.0]
          q_factor: 7.5
          phase_center_offset_m: 0.0
          frequency_range: [2000.0, 2500.0]
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        // Both should be loaded
        assert_eq!(repo.calibration_count(), 2);
        assert_eq!(repo.antenna_count(), 2);
        assert!(repo.has_calibration("antenna_calibrated", "x_band"));
        assert!(repo.has_calibration("antenna_uncalibrated", "primary"));

        // Verify calibrated antenna
        let cal_calibrated = repo
            .get_calibration("antenna_calibrated", "x_band")
            .unwrap();
        assert!(cal_calibrated.calibration_status.is_none()); // Old format

        // Verify uncalibrated antenna
        let cal_uncalibrated = repo
            .get_calibration("antenna_uncalibrated", "primary")
            .unwrap();
        assert!(cal_uncalibrated.calibration_status.is_some());
        assert!(matches!(
            cal_uncalibrated.calibration_status.as_ref().unwrap(),
            CalibrationStatus::Uncalibrated { .. }
        ));
    }

    #[test]
    fn test_load_from_config_fail_fast_uncalibrated() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        // Invalid design specs (negative diameter)
        let antenna_config_yaml = r#"
antennas:
  - id: "bad_antenna"
    name: "Invalid Design Specs"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: -1.0
      focal_length_m: 1.0
      f_over_d_ratio: 0.4
      surface_rms_mm: 1.0
      feeds:
        - id: "feed1"
          name: "Feed 1"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 9000.0]
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        // Should fail during config validation
        let result = CalibrationRepository::load_from_config(&calibration_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_uncalibrated_antenna_list_operations() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        let antenna_config_yaml = r#"
antennas:
  - id: "antenna_1"
    name: "Antenna 1"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.0
      focal_length_m: 1.2
      f_over_d_ratio: 0.4
      surface_rms_mm: 1.0
      feeds:
        - id: "feed_a"
          name: "Feed A"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 9000.0]
        - id: "feed_b"
          name: "Feed B"
          position: [0.05, 0.0, 0.0]
          q_factor: 7.0
          phase_center_offset_m: 0.0
          frequency_range: [2000.0, 3000.0]
  - id: "antenna_2"
    name: "Antenna 2"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 4.0
      focal_length_m: 1.6
      f_over_d_ratio: 0.4
      surface_rms_mm: 1.5
      feeds:
        - id: "primary"
          name: "Primary"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.5
          phase_center_offset_m: 0.0
          frequency_range: [7000.0, 8000.0]
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        // Test list operations
        let antennas = repo.list_antennas();
        assert_eq!(antennas.len(), 2);
        assert_eq!(antennas, vec!["antenna_1", "antenna_2"]);

        let feeds_1 = repo.list_feeds("antenna_1");
        assert_eq!(feeds_1.len(), 2);
        assert_eq!(feeds_1, vec!["feed_a", "feed_b"]);

        let feeds_2 = repo.list_feeds("antenna_2");
        assert_eq!(feeds_2.len(), 1);
        assert_eq!(feeds_2, vec!["primary"]);
    }

    #[test]
    fn test_backward_compatibility_explicit_uncalibrated() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path();

        // Even without explicit calibration_status, if no calibration_file is present
        // and design_specs are provided, the validation should fail unless
        // calibration_status is explicitly set to "uncalibrated"
        // This test verifies that explicit status is required for clarity
        let antenna_config_yaml = r#"
antennas:
  - id: "explicit_uncal"
    name: "Explicitly Uncalibrated"
    calibration_status: "uncalibrated"
    enabled: true
    design_specs:
      diameter_m: 3.5
      focal_length_m: 1.4
      f_over_d_ratio: 0.4
      surface_rms_mm: 1.2
      feeds:
        - id: "feed1"
          name: "Feed 1"
          position: [0.0, 0.0, 0.0]
          q_factor: 8.0
          phase_center_offset_m: 0.0
          frequency_range: [8000.0, 9000.0]
"#;
        let config_path = data_dir.join("antennas.yaml");
        std::fs::write(&config_path, antenna_config_yaml).unwrap();

        let calibration_config = CalibrationConfig {
            data_directory: data_dir.to_path_buf(),
            antenna_config_file: config_path,
            fail_fast: true,
        };

        let repo = CalibrationRepository::load_from_config(&calibration_config).unwrap();

        // Should load as uncalibrated
        assert_eq!(repo.calibration_count(), 1);
        assert!(repo.has_calibration("explicit_uncal", "feed1"));

        let cal = repo.get_calibration("explicit_uncal", "feed1").unwrap();
        assert!(matches!(
            cal.calibration_status.as_ref().unwrap(),
            CalibrationStatus::Uncalibrated { .. }
        ));
    }
}
