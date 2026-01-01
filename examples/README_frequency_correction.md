# Frequency Correction Surface Usage

This example demonstrates how to use the frequency correction module to fit a 1D frequency-only correction surface for boresight calibration.

## Overview

The frequency correction module (`calibrate/src/frequency_correction.rs`) provides functionality to:

1. **Determine if correction is needed**: Check if residuals exceed 0.5 dB threshold
2. **Fit 1D B-spline**: Create a cubic B-spline correction across frequency dimension
3. **Convert to degenerate 4D**: Package as a 4D B-spline for service compatibility

## When to Use

Frequency correction is **optional** and should be applied only when:
- Boresight parameter tuning is complete
- Residual errors (measured - physics model) exceed 0.5 dB
- You want to further improve boresight accuracy

## Example: Boresight Calibration with Frequency Correction

```rust
use calibrate::frequency_correction::{should_fit_correction, fit_frequency_correction};
use calibrate::boresight_calibration::{calibrate_boresight, build_calibration_artifact};

// Step 1: Load measurements and design specs
let measurements = BoresightMeasurements::from_csv("boresight_data.csv")?;
let design_specs = DesignSpecs::load_from_file("antenna_design.yaml")?;

// Step 2: Tune physical parameters
let calibration_result = calibrate_boresight(&measurements, &design_specs)?;

// Step 3: Compute residuals after parameter tuning
let mut residuals_by_freq = Vec::new();
let mut frequencies = Vec::new();

for measurement in &measurements.measurements {
    let predicted_gt = physics_model(
        &calibration_result.tuned_params,
        measurement.frequency_mhz,
        measurement.temperature_k,
    )?;

    let residual = measurement.g_over_t_db - predicted_gt;
    residuals_by_freq.push(residual);
    frequencies.push(measurement.frequency_mhz);
}

// Step 4: Check if frequency correction is beneficial
if should_fit_correction(&residuals_by_freq) {
    println!("Residuals exceed threshold, fitting frequency correction...");

    // Step 5: Fit frequency-only correction
    let correction_surface = fit_frequency_correction(&frequencies, &residuals_by_freq)?;

    println!("Fitted correction surface:");
    println!("  Shape: {:?}", correction_surface.shape);
    println!("  Frequency range: {:.1} - {:.1} MHz",
             correction_surface.knots_frequency.first().unwrap(),
             correction_surface.knots_frequency.last().unwrap());

    // Step 6: Build calibration artifact with correction
    let calibration = build_calibration_artifact(
        &calibration_result,
        &design_specs,
        Some(correction_surface),  // Include correction
    )?;

    println!("Boresight calibration complete with frequency correction");
} else {
    println!("Residuals below threshold, skipping frequency correction");

    // Build calibration artifact without correction
    let calibration = build_calibration_artifact(
        &calibration_result,
        &design_specs,
        None,  // No correction
    )?;

    println!("Boresight calibration complete (physics model only)");
}
```

## Degenerate 4D B-spline Structure

The fitted frequency correction is stored as a degenerate 4D B-spline:

```rust
BSplineModel4D {
    shape: [1, 1, N_freq, 1],  // Single spatial point, N frequency points, single temperature

    // Degenerate dimensions (single point)
    knots_azimuth: [0.0, 0.0, 0.0],      // Boresight azimuth
    knots_elevation: [0.0, 0.0, 0.0],    // Boresight elevation
    knots_temperature: [290.0, 290.0, 290.0],  // Typical temperature

    // Frequency dimension (proper B-spline)
    knots_frequency: [f_min, ..., f_max],  // Clamped cubic B-spline

    coefficients: [c1, c2, ..., cN],  // N correction values in dB
    spline_order: 3,  // Cubic
}
```

## Service Evaluation

The service automatically evaluates the correction surface at query time:

1. **Query at boresight** (az≈0, el≈0):
   - Correction is interpolated at the query frequency
   - Applied: `gain_final = gain_physics + correction(freq)`

2. **Query off-axis** (az≠0 or el≠0):
   - Correction is extrapolated (or returns zero, depending on implementation)
   - Warning generated for out-of-coverage query

## Expected Accuracy Improvement

With frequency correction:
- **Boresight accuracy**: ±0.5-0.8 dB (improved from ±1 dB)
- **Off-axis accuracy**: Still ±2-3 dB (physics model only)
- **Loss accuracy**: ±0.8-1.2 dB (improved from ±1-2 dB)

## Validation

After fitting, validate the correction:

```rust
// Compute RMSE with correction
let mut corrected_errors = Vec::new();

for (i, measurement) in measurements.measurements.iter().enumerate() {
    let predicted_gt = physics_model(&tuned_params, measurement.frequency_mhz)?;
    let correction = evaluate_correction(&correction_surface, measurement.frequency_mhz)?;
    let predicted_with_correction = predicted_gt + correction;

    let error = measurement.g_over_t_db - predicted_with_correction;
    corrected_errors.push(error);
}

let rmse = compute_rmse(&corrected_errors);
println!("Boresight RMSE after correction: {:.3} dB", rmse);
```

## CLI Usage

The calibrate tool will automatically fit frequency correction when appropriate:

```bash
cargo run --release --bin calibrate -- \
  --calibration-mode boresight \
  --input examples/boresight_measurements_xband.csv \
  --design-specs design_specs/small_groundstation.yaml \
  --output calibration_data/antenna_1_xband_boresight.bin \
  --antenna-id antenna_1 \
  --feed-id x_band \
  --verbose

# Output will indicate if correction was fitted:
# "Fitted frequency correction (max residual: 0.73 dB)"
# or
# "Skipping frequency correction (max residual: 0.42 dB < 0.5 dB threshold)"
```

## Notes

- **Optional enhancement**: Boresight calibration works without frequency correction
- **Threshold**: Only fit if `max(abs(residuals)) > 0.5 dB`
- **Compatible**: Uses standard `BSplineModel4D` format for service compatibility
- **Performance**: Minimal overhead (single spatial point, simple interpolation)
