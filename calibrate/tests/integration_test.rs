//! Integration tests for calibration tool

use calibrate::{AntennaClassRegistry, AntennaConfiguration};

#[test]
fn test_load_antenna_classes() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    // Check that all expected classes are loaded
    let class_ids = registry.list_class_ids();
    assert!(class_ids.contains(&"DSN_34m".to_string()));
    assert!(class_ids.contains(&"DSN_70m".to_string()));
    assert!(class_ids.contains(&"GroundStation_13m".to_string()));
    assert!(class_ids.contains(&"TestAntenna_1m".to_string()));
    assert!(class_ids.contains(&"UHF_Array_Element".to_string()));
}

#[test]
fn test_antenna_class_properties() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    let dsn_34m = registry.get_class("DSN_34m").expect("DSN_34m class not found");

    // Verify geometry
    assert_eq!(dsn_34m.geometry.diameter_m, 34.0);
    assert!((dsn_34m.geometry.f_over_d - 0.4285).abs() < 1e-6);

    // Verify focal length calculation
    let focal_length = dsn_34m.focal_length_m();
    assert!((focal_length - 14.569).abs() < 0.01);

    // Verify feed parameters
    assert_eq!(dsn_34m.feed.q_factor, 8.0);
    assert_eq!(dsn_34m.feed.phase_center_offset_wavelengths, 0.0);
    assert_eq!(dsn_34m.feed.asymmetry_factor, 1.0);

    // Verify mesh parameters
    assert_eq!(dsn_34m.mesh.spacing_mm, 3.0);
    assert_eq!(dsn_34m.mesh.wire_diameter_mm, 0.3);

    // Verify surface parameters
    assert_eq!(dsn_34m.surface.rms_mm, 0.5);

    // Verify temperature
    assert_eq!(dsn_34m.system_noise_temperature_k, 50.0);
}

#[test]
fn test_load_antenna_configuration_with_tuning() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    let config = AntennaConfiguration::load_from_file("examples/antenna_config_example.yaml")
        .expect("Failed to load antenna configuration");

    // Verify basic properties
    assert_eq!(config.antenna_id, "dsn_34m_feed_x");
    assert_eq!(config.class_id, "DSN_34m");

    // Verify tunable parameters
    assert!(config.tunable_parameters.has_tuned_values());
    assert_eq!(config.tunable_parameters.surface_rms_mm, Some(0.6));
    assert_eq!(config.tunable_parameters.mesh_spacing_mm, None);

    // Verify metadata
    assert!(config.metadata.parameters_tuned);
    assert_eq!(config.metadata.calibration_date, Some("2025-10-30".to_string()));

    // Validate configuration against registry
    config.validate(&registry).expect("Configuration validation failed");
}

#[test]
fn test_load_antenna_configuration_no_tuning() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    let config = AntennaConfiguration::load_from_file("examples/antenna_config_no_tuning.yaml")
        .expect("Failed to load antenna configuration");

    // Verify basic properties
    assert_eq!(config.antenna_id, "groundstation_13m_ka_feed");
    assert_eq!(config.class_id, "GroundStation_13m");

    // Verify no tuning
    assert!(!config.tunable_parameters.has_tuned_values());
    assert!(!config.metadata.parameters_tuned);

    // Validate configuration
    config.validate(&registry).expect("Configuration validation failed");
}

#[test]
fn test_effective_parameters() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    let config = AntennaConfiguration::load_from_file("examples/antenna_config_example.yaml")
        .expect("Failed to load antenna configuration");

    let class = registry.get_class(&config.class_id).unwrap();

    // Test effective parameters (tuned takes precedence, then class default)
    let effective_rms = config.tunable_parameters.effective_surface_rms(class);
    assert_eq!(effective_rms, 0.6); // Tuned value

    let effective_spacing = config.tunable_parameters.effective_mesh_spacing(class);
    assert_eq!(effective_spacing, 3.0); // Class default

    let effective_diameter = config.tunable_parameters.effective_wire_diameter(class);
    assert_eq!(effective_diameter, 0.3); // Class default
}

#[test]
fn test_antenna_validation_invalid_class() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    let config = AntennaConfiguration::new(
        "invalid_antenna".to_string(),
        "Invalid Antenna".to_string(),
        "NonExistentClass".to_string(),
    );

    // Should fail validation due to non-existent class
    let result = config.validate(&registry);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn test_parameter_bounds_validation() {
    let registry = AntennaClassRegistry::load_from_file("antenna_classes.yaml")
        .expect("Failed to load antenna classes");

    let mut config = AntennaConfiguration::new(
        "test_antenna".to_string(),
        "Test Antenna".to_string(),
        "DSN_34m".to_string(),
    );

    // Set invalid parameter (out of bounds)
    config.tunable_parameters.surface_rms_mm = Some(100.0); // Way too high

    let result = config.validate(&registry);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("outside bounds"));
}
