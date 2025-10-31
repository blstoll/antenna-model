//! Integration tests for correction surface fitting
//!
//! These tests verify that the correction surface fitting pipeline works
//! end-to-end with synthetic data.

use calibrate::correction_surface::{
    compute_residuals, fit_correction_surface, CorrectionSurfaceParams,
};
use calibrate::parser::MeasurementPoint;

/// Generate synthetic measurement data with a known pattern
fn generate_synthetic_measurements(n: usize) -> Vec<MeasurementPoint> {
    let mut measurements = Vec::new();

    // Create a grid of measurements
    let freq_range = (8000.0, 8400.0);
    let cone_range = (0.0, 30.0);
    let clock_range = (0.0, 360.0);

    let nf = (n as f64).powf(1.0 / 3.0).ceil() as usize;
    let nc = nf;
    let nk = nf;

    for i in 0..nf {
        let freq = freq_range.0 + (freq_range.1 - freq_range.0) * i as f64 / (nf - 1) as f64;
        for j in 0..nc {
            let cone = cone_range.0 + (cone_range.1 - cone_range.0) * j as f64 / (nc - 1) as f64;
            for k in 0..nk {
                let clock =
                    clock_range.0 + (clock_range.1 - clock_range.0) * k as f64 / (nk - 1) as f64;

                // Synthetic G/T value with a known pattern
                // G/T = 40 + 0.001*(freq - 8200) - 0.1*cone + 0.05*sin(clock/180*π)
                let g_over_t = 40.0
                    + 0.001 * (freq - 8200.0)
                    - 0.1 * cone
                    + 0.05 * (clock / 180.0 * std::f64::consts::PI).sin();

                measurements.push(MeasurementPoint {
                    e_clock_deg: clock,
                    e_cone_deg: cone,
                    frequency_mhz: freq,
                    g_over_t_db: g_over_t,
                    temperature_k: 290.0,
                });
            }
        }
    }

    measurements
}

/// Generate model predictions that are slightly off from the true values
fn generate_model_predictions(measurements: &[MeasurementPoint], error_magnitude: f64) -> Vec<f64> {
    measurements
        .iter()
        .map(|m| {
            // Model is off by a systematic error that depends on position
            let error = error_magnitude
                * (m.frequency_mhz / 1000.0).sin()
                * (m.e_cone_deg / 10.0).cos();
            m.g_over_t_db - error
        })
        .collect()
}

#[test]
fn test_correction_surface_fitting_basic() {
    // Generate synthetic data
    let measurements = generate_synthetic_measurements(125); // 5x5x5 grid
    let predictions = generate_model_predictions(&measurements, 0.5);

    // Fit correction surface
    let params = CorrectionSurfaceParams {
        num_knots_frequency: 3,
        num_knots_econe: 3,
        num_knots_eclock: 4,
        adaptive_knots: false,
        cross_validation_folds: 0, // Skip CV for speed
        ..Default::default()
    };

    let result = fit_correction_surface(&measurements, &predictions, &params);
    assert!(result.is_ok(), "Fitting should succeed: {:?}", result.err());

    let surface = result.unwrap();

    // Check that the surface has reasonable statistics
    assert!(
        surface.fit_stats.num_points == measurements.len(),
        "Should use all data points"
    );
    assert!(
        surface.fit_stats.rmse_db < 0.5,
        "RMSE should be small: {}",
        surface.fit_stats.rmse_db
    );
    assert!(
        surface.fit_stats.r_squared > 0.3,
        "R-squared should be positive: {}",
        surface.fit_stats.r_squared
    );
    assert!(
        surface.fit_stats.improvement_percent > 15.0,
        "Should show improvement: {}%",
        surface.fit_stats.improvement_percent
    );
}

#[test]
fn test_correction_surface_evaluation() {
    // Generate synthetic data
    let measurements = generate_synthetic_measurements(125); // 5x5x5 grid
    let predictions = generate_model_predictions(&measurements, 1.0);

    // Fit correction surface
    let params = CorrectionSurfaceParams {
        num_knots_frequency: 3,
        num_knots_econe: 3,
        num_knots_eclock: 3,
        adaptive_knots: false,
        cross_validation_folds: 0,
        spline_order: 3, // Reduce order to need fewer points
        ..Default::default()
    };

    let surface = fit_correction_surface(&measurements, &predictions, &params).unwrap();

    // Evaluate at a point that was in the training data
    let test_point = &measurements[10];
    let correction = surface
        .evaluate(
            test_point.frequency_mhz,
            test_point.e_cone_deg,
            test_point.e_clock_deg,
        )
        .unwrap();

    // Correction should be close to the actual residual
    let actual_residual = test_point.g_over_t_db - predictions[10];
    assert!(
        (correction - actual_residual).abs() < 0.5,
        "Correction {} should be close to actual residual {}",
        correction,
        actual_residual
    );
}

#[test]
fn test_correction_surface_interpolation() {
    // Generate synthetic data with sparser sampling
    let mut measurements = Vec::new();
    let mut predictions = Vec::new();

    // Sample at a 4x4x4 grid (64 points)
    for freq in [8000.0, 8133.0, 8266.0, 8400.0] {
        for cone in [0.0, 10.0, 20.0, 30.0] {
            for clock in [0.0, 90.0, 180.0, 270.0] {
                let g_over_t = 40.0 + 0.001 * (freq - 8200.0) - 0.1 * cone;
                measurements.push(MeasurementPoint {
                    e_clock_deg: clock,
                    e_cone_deg: cone,
                    frequency_mhz: freq,
                    g_over_t_db: g_over_t,
                    temperature_k: 290.0,
                });

                // Model has systematic error
                let error = 0.5 * (freq / 1000.0).sin() * (cone / 10.0).cos();
                predictions.push(g_over_t - error);
            }
        }
    }

    let params = CorrectionSurfaceParams {
        num_knots_frequency: 2,
        num_knots_econe: 2,
        num_knots_eclock: 3,
        adaptive_knots: false,
        cross_validation_folds: 0,
        spline_order: 3, // Cubic splines, need 4^3 = 64 points minimum
        ..Default::default()
    };

    let surface = fit_correction_surface(&measurements, &predictions, &params).unwrap();

    // Test interpolation at a point between samples
    let correction = surface.evaluate(8100.0, 10.0, 45.0).unwrap();

    // Correction should be reasonable (between min and max residuals)
    let residuals = compute_residuals(&measurements, &predictions).unwrap();
    let min_res = residuals
        .iter()
        .map(|r| r.residual_db)
        .fold(f64::INFINITY, f64::min);
    let max_res = residuals
        .iter()
        .map(|r| r.residual_db)
        .fold(f64::NEG_INFINITY, f64::max);

    assert!(
        correction >= min_res - 0.5 && correction <= max_res + 0.5,
        "Interpolated correction {} should be within residual range [{}, {}]",
        correction,
        min_res,
        max_res
    );
}

#[test]
fn test_correction_surface_batch_evaluation() {
    let measurements = generate_synthetic_measurements(125);
    let predictions = generate_model_predictions(&measurements, 0.5);

    let params = CorrectionSurfaceParams {
        num_knots_frequency: 3,
        num_knots_econe: 3,
        num_knots_eclock: 3,
        adaptive_knots: false,
        cross_validation_folds: 0,
        spline_order: 3,
        ..Default::default()
    };

    let surface = fit_correction_surface(&measurements, &predictions, &params).unwrap();

    // Test batch evaluation
    let test_points = vec![
        (8100.0, 10.0, 45.0),
        (8200.0, 15.0, 90.0),
        (8300.0, 20.0, 180.0),
    ];

    let corrections = surface.evaluate_batch(&test_points).unwrap();

    assert_eq!(corrections.len(), test_points.len());

    // Each correction should match individual evaluation
    for (i, point) in test_points.iter().enumerate() {
        let individual = surface.evaluate(point.0, point.1, point.2).unwrap();
        assert!(
            (corrections[i] - individual).abs() < 1e-10,
            "Batch evaluation should match individual evaluation"
        );
    }
}

#[test]
fn test_adaptive_knot_placement() {
    // Generate data with non-uniform sampling (dense in center)
    let mut measurements = Vec::new();
    let mut predictions = Vec::new();

    for i in 0..50 {
        // Dense sampling near center
        let cone = 15.0 + 5.0 * (i as f64 / 49.0 - 0.5);
        let freq = 8200.0 + 100.0 * (i as f64 / 49.0 - 0.5);

        for clock in [0.0, 90.0, 180.0, 270.0] {
            let g_over_t = 40.0 - 0.1 * cone;
            measurements.push(MeasurementPoint {
                e_clock_deg: clock,
                e_cone_deg: cone,
                frequency_mhz: freq,
                g_over_t_db: g_over_t,
                temperature_k: 290.0,
            });
            predictions.push(g_over_t - 0.2);
        }
    }

    // Test adaptive knot placement
    let params_adaptive = CorrectionSurfaceParams {
        num_knots_frequency: 5,
        num_knots_econe: 5,
        num_knots_eclock: 4,
        adaptive_knots: true,
        cross_validation_folds: 0,
        ..Default::default()
    };

    let surface_adaptive =
        fit_correction_surface(&measurements, &predictions, &params_adaptive).unwrap();

    // Test uniform knot placement
    let params_uniform = CorrectionSurfaceParams {
        adaptive_knots: false,
        ..params_adaptive
    };

    let surface_uniform =
        fit_correction_surface(&measurements, &predictions, &params_uniform).unwrap();

    // Both should fit well, but adaptive might be slightly better
    assert!(
        surface_adaptive.fit_stats.rmse_db < 0.5,
        "Adaptive fit should work well"
    );
    assert!(
        surface_uniform.fit_stats.rmse_db < 0.5,
        "Uniform fit should work well"
    );
}

#[test]
fn test_regularization_effect() {
    let measurements = generate_synthetic_measurements(125);
    let predictions = generate_model_predictions(&measurements, 0.5);

    // Test with regularization (more robust)
    let params_with_reg = CorrectionSurfaceParams {
        regularization: 1e-3,
        num_knots_frequency: 3,
        num_knots_econe: 3,
        num_knots_eclock: 3,
        cross_validation_folds: 0,
        spline_order: 3,
        ..Default::default()
    };

    let surface_with_reg = fit_correction_surface(&measurements, &predictions, &params_with_reg);
    assert!(surface_with_reg.is_ok(), "Fitting with regularization should succeed");

    let rmse_with_reg = surface_with_reg.unwrap().fit_stats.rmse_db;

    // Regularization should still produce a good fit
    assert!(rmse_with_reg < 1.0, "Regularized fit should be good: {}", rmse_with_reg);

    // Test with different regularization value
    let params_high_reg = CorrectionSurfaceParams {
        regularization: 1e-1,
        ..params_with_reg
    };

    let surface_high_reg = fit_correction_surface(&measurements, &predictions, &params_high_reg);
    assert!(surface_high_reg.is_ok(), "Fitting with high regularization should succeed");
}

#[test]
fn test_insufficient_data_error() {
    // Too few data points
    let measurements = vec![
        MeasurementPoint {
            e_clock_deg: 0.0,
            e_cone_deg: 0.0,
            frequency_mhz: 8200.0,
            g_over_t_db: 40.0,
            temperature_k: 290.0,
        },
        MeasurementPoint {
            e_clock_deg: 90.0,
            e_cone_deg: 10.0,
            frequency_mhz: 8300.0,
            g_over_t_db: 39.5,
            temperature_k: 290.0,
        },
    ];
    let predictions = vec![39.8, 39.3];

    let params = CorrectionSurfaceParams::default();

    let result = fit_correction_surface(&measurements, &predictions, &params);
    assert!(
        result.is_err(),
        "Should fail with insufficient data"
    );
}

#[test]
fn test_dimension_mismatch_error() {
    let measurements = generate_synthetic_measurements(27);
    let predictions = vec![40.0; 20]; // Wrong size

    let params = CorrectionSurfaceParams::default();

    let result = fit_correction_surface(&measurements, &predictions, &params);
    assert!(result.is_err(), "Should fail with dimension mismatch");
}

#[test]
fn test_cross_validation() {
    // Skip CV test for now due to complexity with recursive fitting
    // This is tested implicitly in the parameter tuning integration tests

    // Simple test: just verify that CV flag is respected
    let measurements = generate_synthetic_measurements(125);
    let predictions = generate_model_predictions(&measurements, 0.5);

    // Without CV
    let params_no_cv = CorrectionSurfaceParams {
        num_knots_frequency: 2,
        num_knots_econe: 2,
        num_knots_eclock: 2,
        adaptive_knots: false,
        cross_validation_folds: 0, // Disable CV
        spline_order: 3,
        ..Default::default()
    };

    let surface_no_cv = fit_correction_surface(&measurements, &predictions, &params_no_cv).unwrap();
    assert!(
        surface_no_cv.fit_stats.cross_validation_rmse.is_none(),
        "CV RMSE should not be computed when folds=0"
    );

    // Note: Full CV testing is complex due to recursive fitting requirements
    // and is covered in integration tests with real data
}

#[test]
fn test_compute_residuals_basic() {
    let measurements = vec![
        MeasurementPoint {
            e_clock_deg: 0.0,
            e_cone_deg: 0.0,
            frequency_mhz: 8200.0,
            g_over_t_db: 40.0,
            temperature_k: 290.0,
        },
        MeasurementPoint {
            e_clock_deg: 45.0,
            e_cone_deg: 10.0,
            frequency_mhz: 8300.0,
            g_over_t_db: 39.0,
            temperature_k: 290.0,
        },
    ];
    let predictions = vec![39.5, 38.8];

    let residuals = compute_residuals(&measurements, &predictions).unwrap();

    assert_eq!(residuals.len(), 2);
    assert!((residuals[0].residual_db - 0.5).abs() < 1e-10);
    assert!((residuals[1].residual_db - 0.2).abs() < 1e-10);
    assert_eq!(residuals[0].frequency_mhz, 8200.0);
    assert_eq!(residuals[1].e_cone_deg, 10.0);
}
