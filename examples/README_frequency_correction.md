# Frequency Correction Surface Usage

This example demonstrates how to use the frequency correction module to fit a 1D frequency-only correction surface for boresight calibration.

## Overview

The frequency correction module (`calibrate/src/frequency_correction.rs`) provides functionality to:

1. **Determine if correction is needed**: Check if residuals exceed 0.5 dB threshold
2. **Fit 1D B-spline**: Create a cubic B-spline correction across frequency dimension
3. **Convert to a flat-axis 4D B-spline**: Package as a 4D B-spline for service compatibility

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

## Flat-axis 4D B-spline Structure

The fitted frequency correction is stored as a 4D B-spline that varies only along
frequency. The other three axes are **flat**, not degenerate:

```rust
BSplineModel4D {
    // order + 1 = 4 identical layers on each collapsed axis; N frequency control points
    shape: [4, 4, N_freq, 4],

    // Flat dimensions: a clamped knot vector over a real span, one interior knot
    knots_azimuth:     [0.0,   0.0,   0.0,   180.0, 360.0,  360.0,  360.0],
    knots_elevation:   [0.0,   0.0,   0.0,    90.0, 180.0,  180.0,  180.0],
    knots_temperature: [0.0,   0.0,   0.0,   500.0, 1000.0, 1000.0, 1000.0],

    // Frequency dimension (proper B-spline)
    knots_frequency: [f_min, ..., f_max],  // Clamped cubic B-spline

    // Each residual replicated across every flat layer, in the service's
    // idx = i_az + n_az * (i_el + n_el * (i_freq + n_freq * i_temp)) layout
    coefficients: vec![...; 4 * 4 * N_freq * 4],
    spline_order: 3,  // Cubic
}
```

### Why not a single degenerate point?

A one-layer axis over `order` equal knots looks like the obvious way to collapse a
dimension, and it is how this module worked until 2026-07-31. It is wrong twice over:

1. **The service refuses to load it.** `BSplineModel4D::validate` requires
   `knots.len() >= shape + order` on every axis, and the loader validates every artifact.
   Every boresight run whose residuals cleared the 0.5 dB threshold produced a `.bin` the
   service rejected outright.
2. **Lengthening the knot vector is not a fix.** The evaluator's span is
   `[knots[order-1], knots[len-order]]`, which stays empty for a single coefficient layer
   however long the vector is. An empty span drives every basis function to zero, so the
   correction evaluates to 0 dB — a silent failure that looks like a healthy artifact.

Both are avoided by growing the layer count alongside the knot vector, which is what
`artifact_export::flat_axis(lo, hi, order)` does. The flat spans deliberately cover the
whole queryable domain so a surface that is constant along an axis never reports a
spurious "extrapolated" warning; the boresight-only claim is carried by the artifact's
`calibration_coverage`, which is where the evaluator enforces it.

## Service Evaluation

The service automatically evaluates the correction surface at query time:

1. **Query at boresight**:
   - Correction is interpolated at the query frequency
   - Applied: `gain_final = gain_physics + correction(freq)`

2. **Query off-axis**:
   - `service::evaluator::is_in_coverage` finds the query outside the artifact's
     boresight-only `calibration_coverage`, so no correction is applied
   - The response is flagged extrapolated and carries the partial-calibration warning

> **Known defect (2026-07-31, roadmap D13):** case 1 does not currently fire. Boresight
> coverage is recorded as `azimuth_range = (0, 0)`, but at boresight the azimuth is
> undefined — `antenna_frame_to_spherical` computes it as `atan2(y, x)` on two components
> that are float noise, and a realistic ECEF geometry aimed exactly at the boresight point
> yields 63.43°. The elevation gate is safe (`acos(z/range)` saturates to exactly 0.0); the
> azimuth gate is not. Until this is resolved, a boresight artifact loads and carries its
> correction but serves uncorrected physics.

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
- **Performance**: Minimal overhead (`4 × 4 × N_freq × 4` coefficients, simple interpolation)
