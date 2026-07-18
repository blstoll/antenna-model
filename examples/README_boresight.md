# Boresight Calibration Examples

This directory contains example files for boresight calibration mode.

## Files

### Design Specifications (`design_specs/`)

- `small_groundstation.yaml` - 3.7m dish with X-band and S-band feeds
- `medium_groundstation.yaml` - 7.3m dish with X-band and Ka-band feeds
- `solid_reflector.yaml` - 13m DSN-class antenna with multiple feeds

### Boresight Measurements

- `boresight_measurements_xband.csv` - Example X-band frequency sweep at az=0, el=0

## Usage Example

```bash
# Boresight calibration for small ground station X-band feed
cargo run --release --bin calibrate -- \
  --calibration-mode boresight \
  --input examples/boresight_measurements_xband.csv \
  --design-specs design_specs/small_groundstation.yaml \
  --output calibration_data/antenna_1_xband_boresight.bin \
  --antenna-id antenna_1 \
  --feed-id x_band \
  --max-tuning-iterations 100 \
  --verbose
```

## Boresight Measurement Format

The boresight measurement CSV should have the following columns:

```csv
frequency_mhz,g_over_t_db,temperature_k
7100.0,40.2,290.0
7200.0,40.5,290.0
...
```

**Requirements:**
- Measurements at azimuth=0, elevation=0 (boresight pointing)
- Frequency sweep covering the feed's operating range
- Recommended: 10-50 frequency samples
- Temperature typically 290K (room temperature)

## Expected Results

After boresight calibration, you will get:

- **Calibration Status**: PartiallyCalibrated (boresight only)
- **Accuracy**:
  - Boresight: ±0.5-1 dB (tuned to measurements, with optional frequency correction)
  - Off-axis: ±2-3 dB (physics model extrapolation)
  - Loss (relative): ±1-2 dB (error cancellation)
- **Tuned Parameters**: surface_rms, q_factor, mesh_spacing (if applicable)
- **Optional Frequency Correction**: Automatically fitted if residuals exceed 0.5 dB threshold
- **Test Time**: ~1 hour vs ~8 hours for full calibration

## Design Specs Format

See `design_specs/small_groundstation.yaml` for the required format. Key fields:

- `reflector`: diameter_m, focal_length_m, surface_rms_mm
- `feeds[]`: feed_id, position, q_factor, frequency_range
- `mesh` (optional): mesh_spacing_mm, wire_diameter_mm

These provide initial parameter estimates that will be optimized during boresight calibration.

## Frequency Correction Surface

The boresight calibration tool includes automatic frequency correction surface fitting:

### How It Works

1. **Parameter Tuning**: First, physical parameters (surface_rms, q_factor, mesh) are optimized
2. **Residual Analysis**: After tuning, residuals (measured - predicted) are computed for each frequency
3. **Threshold Check**: If max(abs(residuals)) > 0.5 dB, a frequency correction is fitted
4. **B-spline Fitting**: A 1D cubic B-spline is fitted to the frequency-dependent residuals
5. **Degenerate 4D Format**: The correction is stored as a degenerate 4D B-spline for service compatibility

### When Is It Applied?

The frequency correction is **automatically** applied when:
- Maximum absolute residual > 0.5 dB after parameter tuning
- At least 4 frequency measurement points are available

If residuals are already < 0.5 dB from parameter tuning alone, no correction surface is needed.

### Expected Improvement

- **Without correction**: ±1 dB boresight accuracy (physics model only)
- **With correction**: ±0.5 dB boresight accuracy (physics + correction surface)

### Example Output

```
✓ Boresight calibration complete
  Checking if frequency correction is needed...
    Max residual exceeds 0.5 dB threshold, fitting frequency correction...
    ✓ Frequency correction fitted successfully
      Shape: [1, 1, 15, 1]
      Frequency control points: 15

✓ Calibration artifact built successfully
  Status: PartiallyCalibrated (boresight only)
  ✓ Frequency correction surface attached
  Accuracy estimate: ±0.5 dB at boresight (with frequency correction)
  Off-axis: ±2-3 dB (physics extrapolation)
```

## Interpreting Calibration Results

After running boresight calibration, you'll see output like:

```
Boresight Calibration Complete
==============================
Antenna ID: antenna_1
Feed ID: x_band
Measurements: 15 points

Frequency Range: 7100.0 - 8500.0 MHz

Parameter Tuning Results:
-------------------------
Initial RMSE: 2.34 dB (design specs)
Final RMSE: 0.42 dB (tuned)
Improvement: 82%

Tuned Parameters:
  surface_rms_mm: 1.85 (was 1.50, change: +23%)
  q_factor: 9.2 (was 8.0, change: +15%)

Expected Accuracy:
  Boresight: ±0.5-1.0 dB (±0.5 dB with frequency correction, ±1.0 dB without)
  Off-axis: ±2-3 dB (physics extrapolation)
  Loss (relative): ±1-2 dB

Output: calibration_data/antenna_1_xband.bin (554 bytes)
```

### Understanding the Metrics

**Initial RMSE (design specs):**
- Root-Mean-Square Error between measurements and physics model using design specifications
- High values (>2 dB) indicate design specs don't match actual hardware well
- Example: 2.34 dB means design specs predict G/T with ~2.3 dB average error

**Final RMSE (tuned):**
- RMSE after parameter optimization
- **Quality Indicators:**
  - **Excellent**: <0.5 dB (very good fit)
  - **Good**: 0.5-1.0 dB (acceptable fit)
  - **Fair**: 1.0-1.5 dB (review measurements or design specs)
  - **Poor**: >1.5 dB (possible measurement errors or physics model limitations)

**Improvement Percentage:**
- Reduction in RMSE from design specs to tuned parameters
- Example: 82% means tuned parameters reduce error by 82%
- Typical improvements: 60-90%

**Tuned Parameters:**
- **surface_rms_mm**: Actual surface quality (may differ from design by ±50%)
- **q_factor**: Actual feed illumination pattern (may differ by ±30%)
- **mesh_spacing_mm** and **wire_diameter_mm**: For mesh reflectors only

**Parameter Changes:**
- ±10-30% from design specs is normal (manufacturing tolerances, installation effects)
- >50% change warrants investigation (possible measurement error or wrong design specs)

### Calibration Quality Checklist

- ✅ **Final RMSE < 0.5 dB**: Excellent fit, proceed with confidence
- ✅ **Final RMSE 0.5-1.0 dB**: Good fit, acceptable for boresight calibration
- ⚠️ **Final RMSE 1.0-1.5 dB**: Review measurement quality and design specs
- ❌ **Final RMSE > 1.5 dB**: Investigate measurement errors or physics model issues

- ✅ **Improvement > 70%**: Parameter tuning working well
- ⚠️ **Improvement 50-70%**: Marginal improvement, design specs may be reasonable
- ❌ **Improvement < 50%**: Parameter tuning not effective, review setup

- ✅ **Parameter changes 10-30%**: Normal variation
- ⚠️ **Parameter changes 30-50%**: Investigate causes
- ❌ **Parameter changes > 50%**: Verify design specs and measurements

## Common Issues and Solutions

### Issue 1: High Final RMSE (>1.5 dB)

**Symptoms:**
```
Final RMSE: 2.1 dB (tuned)
Improvement: 25%
```

**Possible Causes:**
- Measurement errors (noise, interference, miscalibrated equipment)
- Wrong design specs file (antenna ID mismatch)
- Physics model limitations (e.g., blockage effects not modeled)
- Environmental factors (temperature gradients, wind loading)

**Solutions:**
1. **Review measurement quality:**
   - Check SNR (signal-to-noise ratio) during measurements
   - Verify calibration of measurement equipment
   - Ensure stable ambient temperature (±5°C variation max)

2. **Verify design specs:**
   - Confirm `antenna_id` matches actual antenna
   - Check diameter, focal length match installed hardware
   - Verify feed q-factor estimate is reasonable (typical: 5-15)

3. **Increase measurement density:**
   - Add more frequency points (try 20-30 instead of 10-15)
   - Ensure measurements cover full operating band

4. **Check for environmental factors:**
   - Avoid measurements during high winds (>15 mph)
   - Measure during stable temperature conditions
   - Check for nearby interference sources

### Issue 2: Parameter Bounds Violations

**Symptoms:**
```
Warning: Parameter out of bounds: surface_rms=4.8 mm (max: 4.5 mm)
  Clamping to maximum value
```

**Cause:**
- Optimizer trying to explore parameters outside reasonable range
- Indicates design specs may be far from actual hardware

**Solutions:**
1. **Review tuning bounds** (in `design_specs_loader.rs`):
   - Default bounds: surface_rms [nominal × 0.3, nominal × 3.0]
   - Default bounds: q_factor [nominal × 0.5, nominal × 2.0]

2. **Adjust design specs** if hardware is known to be non-standard:
   - Update surface_rms estimate based on manufacturer data
   - Adjust q-factor estimate based on feed design

3. **Consider wider bounds** if justified:
   - Some antennas have poor surface quality (increase upper bound)
   - Some feeds have unusual patterns (widen q-factor range)

### Issue 3: Convergence Failure

**Symptoms:**
```
Warning: Optimization did not converge after 100 iterations
  Final RMSE: 1.8 dB
```

**Solutions:**
1. **Increase iterations:**
   ```bash
   --max-tuning-iterations 200
   ```
   Try 200-500 iterations for difficult cases

2. **Review initial parameter estimates:**
   - Design specs should be reasonable starting point
   - Very poor initial guesses slow convergence

3. **Check measurement data quality:**
   - Noisy measurements make optimization difficult
   - Outliers can prevent convergence

4. **Consider alternative starting points:**
   - Manually adjust design specs to better estimates
   - Use measurements from similar antenna as reference

### Issue 4: CSV Parsing Errors

**Symptoms:**
```
Error: Failed to parse CSV: Missing column 'frequency_mhz'
```

**Solutions:**
1. **Verify CSV format:**
   ```csv
   frequency_mhz,g_over_t_db,temperature_k
   7100.0,40.2,290.0
   ```

2. **Check for common issues:**
   - Header row must be present and exact (case-sensitive)
   - No extra spaces in column names
   - No special characters (use UTF-8 encoding)
   - Comma delimiters (not semicolons or tabs)

3. **Validate data:**
   - All rows have 3 columns
   - Numeric values are valid (no text in numeric columns)
   - No missing values

### Issue 5: Artifact Incompatible with Service

**Symptoms:**
```
ERROR Service startup failed: Invalid calibration artifact
```

**Solutions:**
1. **Verify postcard format:**
   - Calibrate tool must use same postcard version as service
   - Check Cargo.toml versions match, and that the ANTC artifact version matches
     `ANTC_SUPPORTED_VERSION` in the service (currently 2)

2. **Check artifact integrity:**
   ```bash
   ls -lh calibration_data/antenna_1_xband_boresight.bin
   ```
   - File should be 500-1000 bytes typically
   - Zero-byte or very large files indicate corruption

3. **Regenerate artifact:**
   - Re-run calibration with `--verbose` flag
   - Review output for any errors during serialization

4. **Verify service configuration:**
   - Check `antennas.yaml` correctly references `.bin` file
   - Ensure file path is correct

### Issue 6: Unrealistic Parameter Values

**Symptoms:**
```
Tuned Parameters:
  surface_rms_mm: 0.05 (was 1.50, change: -97%)
  q_factor: 25.0 (was 8.0, change: +213%)
```

**Cause:**
- Optimizer finding local minimum with unrealistic parameters
- Measurements may have systematic bias

**Solutions:**
1. **Verify measurement calibration:**
   - Check if measurement equipment is properly calibrated
   - Verify temperature measurement is accurate

2. **Review design specs:**
   - Ensure design specs are representative
   - Check if antenna has been modified since manufacturing

3. **Add validation constraints:**
   - Very low surface_rms (<0.1 mm) unlikely for real antennas
   - Very high q-factor (>20) unusual except for horns

4. **Check for systematic measurement errors:**
   - Temperature errors affect all measurements similarly
   - Cable losses can introduce systematic bias

## Upgrading Calibration

You can upgrade from boresight to full calibration:

1. **Start**: Uncalibrated (design specs only) → ±3-5 dB absolute, ±2-3 dB loss
2. **Boresight**: Boresight measurements (~1 hour) → ±1 dB boresight, ±2-3 dB off-axis
3. **Full**: Full grid measurements (~8 hours) → ±1 dB everywhere

Each stage produces a valid `.bin` file that can be used in the antenna model service.

**Detailed Workflows:** See [Calibration Workflow Guide](../docs/calibration-workflow-guide.md) for complete documentation of all calibration workflows and troubleshooting.
