/// Integration tests for measurement data parser
use calibrate::parser::{parse_measurements_sync, MeasurementData, MeasurementPoint};

#[test]
fn test_parse_sample_measurements() {
    let fixture_path = "tests/fixtures/sample_measurements.csv";
    let result = parse_measurements_sync(fixture_path);

    assert!(result.is_ok(), "Failed to parse sample measurements");
    let data = result.unwrap();

    // Should have 41 measurement points in the sample file
    assert_eq!(data.len(), 41, "Expected 41 measurement points");

    // Check frequency range
    let (freq_min, freq_max) = data.frequency_range();
    assert_eq!(freq_min, 2200.0);
    assert_eq!(freq_max, 12000.0);

    // Check E-cone range
    let (cone_min, cone_max) = data.e_cone_range();
    assert_eq!(cone_min, 0.0);
    assert_eq!(cone_max, 15.0);
}

#[test]
fn test_quality_report_on_sample_data() {
    let fixture_path = "tests/fixtures/sample_measurements.csv";
    let data = parse_measurements_sync(fixture_path).unwrap();

    // Generate quality report with estimated 2° beamwidth
    let report = data.quality_report(2.0);

    assert_eq!(report.total_points, 41);
    assert_eq!(report.unique_frequencies, 3); // 2200, 8400, 12000 MHz

    // Should have main lobe and sidelobe points
    assert!(report.main_lobe_points > 0);
    assert!(report.sidelobe_points > 0);

    // Format the report (should not panic)
    let formatted = report.format();
    assert!(formatted.contains("Data Quality Report"));
    assert!(formatted.contains("Total Points: 41"));
}

#[test]
fn test_gain_extraction() {
    let fixture_path = "tests/fixtures/sample_measurements.csv";
    let data = parse_measurements_sync(fixture_path).unwrap();

    // Check gain extraction for first point
    let first_point = &data.points[0];
    let gain = first_point.gain_db();

    // G/T = 41.5 dB/K, T = 50 K
    // Gain = 41.5 + 10*log10(50) = 41.5 + 16.99 ≈ 58.49 dB
    assert!((gain - 58.49).abs() < 0.1, "Gain extraction incorrect");
}

#[test]
fn test_outlier_detection() {
    // Create data with a clear outlier
    let points = vec![
        MeasurementPoint::new(0.0, 0.0, 8400.0, 41.0, 50.0),
        MeasurementPoint::new(45.0, 1.0, 8400.0, 41.1, 50.0),
        MeasurementPoint::new(90.0, 2.0, 8400.0, 41.2, 50.0),
        MeasurementPoint::new(135.0, 3.0, 8400.0, 41.0, 50.0),
        MeasurementPoint::new(180.0, 4.0, 8400.0, 41.3, 50.0),
        MeasurementPoint::new(225.0, 5.0, 8400.0, 60.0, 50.0), // Clear outlier
    ];

    let data = MeasurementData::new(points, "test".to_string());
    let outliers = data.detect_outliers(3.5);

    assert_eq!(outliers.len(), 1, "Should detect one outlier");
    assert_eq!(outliers[0], 5, "Outlier should be at index 5");
}

#[test]
fn test_frequency_distribution() {
    let fixture_path = "tests/fixtures/sample_measurements.csv";
    let data = parse_measurements_sync(fixture_path).unwrap();

    let freq_dist = data.frequency_distribution();

    // Should have 3 frequencies
    assert_eq!(freq_dist.len(), 3);

    // X-band (8400 MHz) should have the most points
    let x_band_count = freq_dist.get("8400.0").unwrap_or(&0);
    assert!(*x_band_count > 15, "X-band should have > 15 points");
}

#[test]
fn test_main_lobe_sidelobe_classification() {
    let fixture_path = "tests/fixtures/sample_measurements.csv";
    let data = parse_measurements_sync(fixture_path).unwrap();

    let beamwidth = 2.0; // 2° beamwidth

    // Count main lobe vs sidelobe points
    let main_lobe_count = data
        .points
        .iter()
        .filter(|p| p.is_main_lobe(beamwidth))
        .count();

    let sidelobe_count = data.points.len() - main_lobe_count;

    // Should have both types
    assert!(main_lobe_count > 0, "Should have main lobe points");
    assert!(sidelobe_count > 0, "Should have sidelobe points");

    // Main lobe should be larger portion for this dataset
    assert!(
        main_lobe_count > sidelobe_count / 2,
        "Main lobe should be significant portion"
    );
}

#[test]
fn test_invalid_csv_handling() {
    // Create a CSV with invalid data
    let invalid_csv = "e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n\
                       500.0,0.0,8400.0,41.5,50.0\n\
                       0.0,200.0,8400.0,41.5,50.0\n";

    use std::io::Write;
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("invalid_test.csv");
    let mut file = std::fs::File::create(&temp_file).unwrap();
    file.write_all(invalid_csv.as_bytes()).unwrap();
    drop(file);

    let result = parse_measurements_sync(temp_file.to_str().unwrap());

    // Should fail because all points are invalid
    assert!(result.is_err(), "Should fail with all invalid points");

    // Cleanup
    std::fs::remove_file(temp_file).ok();
}

#[test]
fn test_partial_invalid_csv_handling() {
    // Create a CSV with some valid and some invalid data
    let partial_csv = "e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k\n\
                       0.0,0.0,8400.0,41.5,50.0\n\
                       invalid,10.0,8400.0,40.5,50.0\n\
                       90.0,5.0,2200.0,39.5,50.0\n";

    use std::io::Write;
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("partial_invalid_test.csv");
    let mut file = std::fs::File::create(&temp_file).unwrap();
    file.write_all(partial_csv.as_bytes()).unwrap();
    drop(file);

    let result = parse_measurements_sync(temp_file.to_str().unwrap());

    // Should succeed with valid points only
    assert!(result.is_ok(), "Should succeed with some valid points");
    let data = result.unwrap();
    assert_eq!(data.len(), 2, "Should have 2 valid points");

    // Cleanup
    std::fs::remove_file(temp_file).ok();
}
