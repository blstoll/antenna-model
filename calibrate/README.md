# Antenna Calibration Tool

Command-line tool for generating calibration artifacts from antenna measurement data. This tool implements a hybrid calibration approach combining physics-based models with correction surfaces to achieve high accuracy (±1 dB) predictions.

## Overview

The calibration tool processes G/T (Gain-to-Temperature) measurements and generates binary calibration artifacts suitable for deployment in the Antenna Model Service. The workflow consists of:

1. **Measurement parsing** - Load and validate CSV measurement data
2. **Antenna class loading** - Load shared physical parameters
3. **Parameter tuning (optional)** - Optimize 2-3 physical parameters
4. **Model predictions** - Compute physics-based G/T predictions
5. **Correction surface fitting** - Fit B-spline surface to residuals
6. **Validation** - Cross-validation and quality metrics

## Installation

```bash
# Build from source
cargo build --release -p calibrate

# The binary will be at target/release/calibrate
```

## Quick Start

### Basic Usage (No Parameter Tuning)

```bash
calibrate \
  --input measurements.csv \
  --output calibration.bin \
  --antenna-id my_antenna_001 \
  --antenna-class DSN_34m
```

### With Parameter Tuning and Validation

```bash
calibrate \
  --input measurements.csv \
  --output calibration.bin \
  --antenna-id my_antenna_001 \
  --antenna-class DSN_34m \
  --tune-parameters \
  --validate \
  --report validation_report.json
```

### Full Example with All Options

```bash
calibrate \
  --input s3://bucket/antenna_measurements.csv \
  --output calibration_artifacts/antenna_001.bin \
  --antenna-id antenna_001 \
  --antenna-name "DSN 34m Antenna - Site 1" \
  --antenna-class DSN_34m \
  --classes-file calibrate/antenna_classes.yaml \
  --tune-parameters \
  --tuning-mode surface-and-mesh \
  --validate \
  --cv-folds 5 \
  --report reports/antenna_001_validation.json \
  --metadata reports/antenna_001_metadata.json \
  --verbose
```

## Input Format

### Measurement CSV Format

The input CSV file must contain the following columns:

```csv
e_clock_deg,e_cone_deg,frequency_mhz,g_over_t_db,temperature_k
0.0,0.0,8400.0,41.5,50.0
0.0,0.5,8400.0,41.4,50.0
0.0,1.0,8400.0,41.2,50.0
...
```

**Column Descriptions:**
- `e_clock_deg` - E-clock angle (azimuthal angle around boresight, 0-360 degrees)
- `e_cone_deg` - E-cone angle (polar angle from boresight, 0-90 degrees)
- `frequency_mhz` - Frequency in MHz
- `g_over_t_db` - Measured G/T in dB/K
- `temperature_k` - System noise temperature in Kelvin

**Data Requirements:**
- Minimum 50 measurement points recommended
- Good coverage of frequency range and angular range
- Main lobe coverage essential (e_cone < 5°)
- First sidelobe coverage recommended (5° < e_cone < 15°)

### Antenna Classes Configuration

The `antenna_classes.yaml` file defines shared parameters for antenna types:

```yaml
classes:
  DSN_34m:
    class_id: "DSN_34m"
    description: "Deep Space Network 34-meter beam-waveguide antenna"
    geometry:
      diameter_m: 34.0
      f_over_d: 0.4285
    feed:
      q_factor: 8.0
      phase_center_offset_wavelengths: 0.0
      asymmetry_factor: 1.0
    mesh:
      spacing_mm: 3.0
      wire_diameter_mm: 0.3
    surface:
      rms_mm: 0.5
    system_noise_temperature_k: 50.0
```

## Command-Line Options

### Required Arguments

- `--input, -i <PATH>` - Input measurement CSV file or S3 URL
- `--output, -o <PATH>` - Output calibration artifact path (.bin file)
- `--antenna-id, -a <ID>` - Unique antenna identifier

### Optional Arguments

- `--antenna-name, -n <NAME>` - Human-readable antenna name (default: "Untitled Antenna")
- `--antenna-class, -c <CLASS>` - Antenna class name (default: "DSN_34m")
- `--classes-file <PATH>` - Path to antenna classes YAML (default: "calibrate/antenna_classes.yaml")

### Parameter Tuning Options

- `--tune-parameters, -t` - Enable parameter optimization
- `--tuning-mode <MODE>` - Tuning mode: `surface-only`, `surface-and-mesh`, or `all` (default: surface-only)
- `--max-tuning-iterations <N>` - Maximum optimization iterations (default: 100)

**Tuning Modes:**
- `surface-only` - Tune surface RMS only (fastest, 1 parameter)
- `surface-and-mesh` - Tune surface RMS and mesh spacing (2 parameters)
- `all` - Tune surface RMS, mesh spacing, and wire diameter (3 parameters, slowest)

### Validation Options

- `--validate` - Run cross-validation after fitting
- `--cv-folds <N>` - Number of cross-validation folds (default: 5)
- `--report, -r <PATH>` - Export validation report to JSON
- `--metadata, -m <PATH>` - Export metadata to JSON

### Other Options

- `--verbose, -v` - Enable verbose logging (debug level)

## Output Files

### Calibration Artifact (.bin)

Binary file containing:
- Antenna configuration (class reference + tuned parameters)
- Correction surface (B-spline coefficients, knots, dimensions)
- Validation report (quality metrics)
- Metadata (timestamps, data sources, quality indicators)

This file is used by the Antenna Model Service for runtime predictions.

### Validation Report JSON (optional)

```json
{
  "num_points": 500,
  "model_only_rmse": 2.34,
  "corrected_rmse": 0.45,
  "rmse_improvement_percent": 80.8,
  "main_lobe_max_error": 0.78,
  "main_lobe_meets_target": true,
  "first_sidelobe_max_error": 0.92,
  "first_sidelobe_meets_target": true,
  "outliers": [...],
  "cross_validation": {
    "mean_rmse": 0.47,
    "std_rmse": 0.03,
    "min_rmse": 0.43,
    "max_rmse": 0.51
  }
}
```

### Metadata JSON (optional)

```json
{
  "created_at": "2025-01-15T10:30:45Z",
  "measurement_source": "measurements.csv",
  "parameters_tuned": true,
  "num_measurement_points": 500,
  "tool_version": "0.1.0",
  "frequency_range": [8000.0, 8400.0],
  "angular_range": [0.0, 30.0],
  "notes": "Calibrated with class: DSN_34m, R²=0.95"
}
```

## Workflow Examples

### Example 1: Quick Calibration (No Tuning)

Fastest workflow, uses nominal antenna class parameters:

```bash
calibrate \
  -i measurements/antenna_001.csv \
  -o artifacts/antenna_001.bin \
  -a antenna_001 \
  -c DSN_34m
```

**When to use:**
- Antenna matches class definition closely
- Fast turnaround needed
- Correction surface will compensate for parameter mismatches

### Example 2: High-Accuracy Calibration

Optimal accuracy with parameter tuning:

```bash
calibrate \
  -i measurements/antenna_002.csv \
  -o artifacts/antenna_002.bin \
  -a antenna_002 \
  -c DSN_34m \
  --tune-parameters \
  --tuning-mode all \
  --validate \
  --report reports/antenna_002.json
```

**When to use:**
- Maximum accuracy required
- Antenna may deviate from nominal class parameters
- Sufficient computation time available (5-10 minutes)

### Example 3: Batch Processing

Process multiple antennas:

```bash
#!/bin/bash
for antenna in antenna_001 antenna_002 antenna_003; do
  echo "Calibrating $antenna..."
  calibrate \
    -i measurements/${antenna}.csv \
    -o artifacts/${antenna}.bin \
    -a ${antenna} \
    -c DSN_34m \
    --tune-parameters \
    --validate \
    --report reports/${antenna}_validation.json \
    --metadata reports/${antenna}_metadata.json
done
```

## Calibration Quality Metrics

### Accuracy Targets

- **Main Lobe**: ≤1.0 dB maximum error
- **First Sidelobe**: ≤1.0 dB maximum error
- **Overall RMSE**: ≤0.5 dB (after correction)
- **R² (goodness of fit)**: ≥0.90

### Interpreting Results

**Good Calibration:**
```
✓ Main lobe meets accuracy target
✓ First sidelobe meets accuracy target
Corrected RMSE: 0.35 dB
Improvement: 85.2%
```

**Warning Signs:**
```
⚠ Main lobe max error exceeds threshold (1.24 > 1.00 dB)
⚠ Found 12 outlier points
```

**Possible issues:**
- Insufficient measurement coverage
- Noisy measurement data
- Antenna not well-represented by class definition
- Need parameter tuning (`--tune-parameters`)

## Troubleshooting

### Error: "Insufficient data for fitting"

**Cause:** Not enough measurement points

**Solution:** Collect more measurements, aim for at least 50 points

### Error: "Antenna class 'XYZ' not found"

**Cause:** Class not defined in antenna_classes.yaml

**Solution:** Check `--classes-file` path and class name spelling

### Warning: "Main lobe max error exceeds threshold"

**Cause:** Poor fit in main lobe region

**Solutions:**
1. Enable parameter tuning: `--tune-parameters`
2. Check measurement data quality
3. Increase cross-validation folds: `--cv-folds 10`

### Slow Performance

**Causes and solutions:**
- **Parameter tuning slow**: Use `--tuning-mode surface-only` (default)
- **Many measurement points**: Normal behavior, consider sampling
- **High cross-validation folds**: Reduce `--cv-folds` (default: 5)

## Advanced Topics

### Custom Antenna Classes

Create your own antenna class in `antenna_classes.yaml`:

```yaml
classes:
  MyCustomAntenna:
    class_id: "MyCustomAntenna"
    description: "My custom antenna configuration"
    geometry:
      diameter_m: 10.0
      f_over_d: 0.5
    feed:
      q_factor: 7.0
      phase_center_offset_wavelengths: 0.0
      asymmetry_factor: 1.0
    mesh:
      spacing_mm: 4.0
      wire_diameter_mm: 0.4
    surface:
      rms_mm: 1.0
    system_noise_temperature_k: 70.0
```

### S3 Integration

The tool supports S3 URLs for input measurements:

```bash
calibrate \
  --input s3://my-bucket/measurements/antenna_001.csv \
  --output artifacts/antenna_001.bin \
  --antenna-id antenna_001
```

Requires AWS credentials configured (`~/.aws/credentials` or environment variables).

### Parameter Bounds

Parameters are constrained during tuning:
- **Surface RMS**: 0.1 - 5.0 mm
- **Mesh spacing**: 0.5 - 20.0 mm
- **Wire diameter**: 0.05 - 2.0 mm

These bounds ensure physical plausibility.

## Integration with Antenna Model Service

After generating calibration artifacts:

1. Copy `.bin` files to service calibration directory:
   ```bash
   cp artifacts/*.bin antenna-model/calibration_data/
   ```

2. Update service configuration to register antennas:
   ```yaml
   # antenna-model/calibration_data/antennas.yaml
   antennas:
     configs:
       - id: "antenna_001"
         name: "My Antenna"
         calibration_file: "antenna_001.bin"
         enabled: true
   ```

3. Deploy updated service to Kubernetes

## Performance

**Typical calibration times** (500 measurement points, 8-core CPU):

| Mode | Time |
|------|------|
| No tuning | ~30 seconds |
| Surface-only tuning | ~2 minutes |
| All parameters tuning | ~5-10 minutes |

Add ~1-2 minutes for cross-validation (`--validate`).

## Version History

- **0.1.0** (Sprint 4) - Initial CLI implementation with full workflow

## See Also

- [Architecture Documentation](../docs/architecture.md)
- [Design Document](../docs/antenna-model-design-doc.md)
- [Implementation Plan](../docs/implementation-plan.md)
