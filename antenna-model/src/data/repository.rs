//! Calibration data repository
//!
//! This module provides thread-safe access to loaded calibration data.

use crate::config::{AntennaConfig, CalibrationConfig};
use crate::data::loader::load_calibration_artifact;
use crate::data::types::{
    AntennaCalibration, BSplineModel4D, PhysicalAntennaConfig, ValidityRanges,
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
        let antenna_config = AntennaConfig::from_file(
            config
                .antenna_config_file
                .to_string_lossy().as_ref(),
        )?;

        let enabled_antennas = antenna_config.enabled_antennas();
        info!(
            "Found {} enabled antenna(s) in configuration",
            enabled_antennas.len()
        );

        let mut repository = Self::new();
        let mut loaded_count = 0;
        let mut error_count = 0;

        for entry in enabled_antennas {
            debug!(
                "Loading calibration for antenna '{}' from '{}'",
                entry.id, entry.calibration_file
            );

            let calibration_path = config.data_directory.join(&entry.calibration_file);

            match load_calibration_artifact(&calibration_path) {
                Ok(calibration) => {
                    // Verify antenna_id matches configuration
                    if calibration.antenna_id != entry.id {
                        let msg = format!(
                            "Antenna ID mismatch: config='{}', file='{}'",
                            entry.id, calibration.antenna_id
                        );
                        if config.fail_fast {
                            return Err(DataError::ConfigurationError { reason: msg });
                        } else {
                            warn!("{}", msg);
                            error_count += 1;
                            continue;
                        }
                    }

                    repository.add_calibration(calibration);
                    loaded_count += 1;
                }
                Err(e) => {
                    if config.fail_fast {
                        return Err(e);
                    } else {
                        warn!(
                            "Failed to load calibration for '{}': {}",
                            entry.id, e
                        );
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
    pub fn get_calibration(
        &self,
        antenna_id: &str,
        feed_id: &str,
    ) -> Option<AntennaCalibration> {
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
    pub fn get_validity_ranges(
        &self,
        antenna_id: &str,
        feed_id: &str,
    ) -> Option<ValidityRanges> {
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
}
