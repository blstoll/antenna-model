# Antenna Calibration Workflow Guide

**Version:** 2.0
**Last Updated:** 2025-12-08
**For:** Antenna Model Service Sprint 7+

---

## Table of Contents

### Part 1: Operational Workflows
1. [Introduction](#1-introduction)
2. [Uncalibrated Antennas (Design Specs Only)](#2-uncalibrated-antennas-design-specs-only)
3. [Boresight Calibration Workflow](#3-boresight-calibration-workflow)
4. [Full Grid Calibration Workflow](#4-full-grid-calibration-workflow)
5. [Calibration Upgrade Path](#5-calibration-upgrade-path)
6. [Using Calibrated Antennas in Service](#6-using-calibrated-antennas-in-service)

### Part 2: Technical Reference
7. [Calibration Status Types (Technical Detail)](#7-calibration-status-types-technical-detail)
8. [Accuracy Estimation Methods](#8-accuracy-estimation-methods)
9. [Boresight Optimization Algorithm](#9-boresight-optimization-algorithm)
10. [API Integration](#10-api-integration)
11. [Appendix: Design Specs Reference](#11-appendix-design-specs-reference)

---

# Part 1: Operational Workflows

## 1. Introduction

The Antenna Model Service supports three calibration levels, each with different accuracy characteristics and test time requirements. This guide provides complete workflows for all three levels and explains when to use each approach.

### 1.1 Three Calibration Levels Overview

The service uses a **hybrid physical optics + correction surface** model that gracefully degrades based on available calibration data:

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Calibration Levels                             │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌──────────────────┐     ┌──────────────────┐    ┌──────────────┐ │
│  │  Uncalibrated    │ --> │ Partially        │ -> │   Fully      │ │
│  │  (Design Specs)  │     │ Calibrated       │    │ Calibrated   │ │
│  │                  │     │ (Boresight)      │    │ (Grid)       │ │
│  └──────────────────┘     └──────────────────┘    └──────────────┘ │
│                                                                     │
│   Physics Model          Physics + Parameter      Physics +        │
│   (Design Specs)         Tuning (Boresight)       Correction       │
│                                                    Surface          │
│   Test Time: 0 hours     Test Time: ~1 hour       Test Time:       │
│                                                    ~8 hours         │
└─────────────────────────────────────────────────────────────────────┘
```

### 1.2 Accuracy Expectations

| Calibration Status | Absolute Gain Accuracy | Loss Accuracy | Test Time | Typical Use Case |
|-------------------|----------------------|---------------|-----------|------------------|
| **Fully Calibrated** | ±1.0 dB | ±1.0 dB | ~8 hours | Production operations, high-precision analysis |
| **Partially Calibrated (in-coverage)** | ±1.0-1.5 dB | ±1.0-1.5 dB | ~1 hour | Rapid commissioning, boresight verification |
| **Partially Calibrated (out-of-coverage)** | ±2-3 dB | ±2-3 dB | ~1 hour | Physics extrapolation from boresight |
| **Uncalibrated** | ±3-5 dB | ±2 dB | 0 hours | Loss analysis, planning, design validation |

**Key Insight:** Loss (relative gain) accuracy is better than absolute gain accuracy for uncalibrated antennas (±2 dB vs ±3-5 dB) due to systematic error cancellation when comparing two pointing directions.

### 1.3 When to Use Each Level

**Uncalibrated (Design Specs Only):**
- Initial antenna design validation
- Comparative loss analysis across pointing directions
- Rapid "what-if" scenario modeling
- Pre-deployment planning
- When absolute accuracy is not critical

**Partially Calibrated (Boresight):**
- Rapid antenna commissioning (~1 hour test vs ~8 hours)
- Boresight verification after installation
- Parameter tuning for new antenna designs
- Upgrading from uncalibrated to improve accuracy
- When full grid testing is not feasible

**Fully Calibrated (Grid):**
- Production operations requiring high accuracy
- Full field-of-view characterization
- Generating loss heatmaps
- Quality assurance and acceptance testing
- When ±1 dB accuracy is required everywhere

### 1.4 Calibration Artifact Format

All calibration levels produce binary `.bin` files containing:
- **Antenna metadata**: ID, name, feeds
- **Physical configuration**: Reflector geometry, feed parameters, mesh properties
- **Calibration status**: Fully/Partially/Uncalibrated with accuracy estimates
- **Correction surface** (optional): 4D B-spline interpolation model
- **Validity ranges**: Azimuth, elevation, frequency, temperature

The service loads these artifacts at startup from the `calibration_data/` directory.

---

## 2. Uncalibrated Antennas (Design Specs Only)

Uncalibrated antennas use **design specifications only** with no measured data. The service computes antenna performance using the physical optics model with nominal design parameters.

### 2.1 When to Use Uncalibrated Mode

**Ideal For:**
- Antenna design validation before hardware exists
- Comparative loss analysis (±2 dB loss accuracy)
- Rapid scenario modeling
- Initial system planning

**Not Suitable For:**
- Absolute gain predictions (±3-5 dB uncertainty)
- High-precision applications
- Regulatory compliance testing

### 2.2 Creating Design Specs YAML

Design specs define the physical antenna parameters. Here's a complete example for a 3.7m ground station:

```yaml
# design_specs/small_groundstation.yaml
antenna_id: "antenna_1"
antenna_name: "3.7m Ground Station - X/S-Band"

reflector:
  diameter_m: 3.7              # Dish diameter in meters
  focal_length_m: 1.85         # Focal length (f/D = 0.5)
  surface_rms_mm: 1.5          # Surface RMS error estimate

feeds:
  - feed_id: "x_band"          # Unique feed identifier
    name: "X-Band Primary Feed"
    position: [0.0, 0.0, 0.0]  # Position relative to focal point (x, y, z) in meters
    q_factor: 8.0              # Feed illumination pattern parameter
    phase_center_offset_m: 0.0 # Phase center offset along feed axis
    frequency_range: [7100.0, 8500.0]  # Operating frequency range in MHz

  - feed_id: "s_band"
    name: "S-Band Primary Feed"
    position: [0.0, 0.0, 0.0]
    q_factor: 7.0
    phase_center_offset_m: 0.0
    frequency_range: [2000.0, 2300.0]

mesh:  # Optional - omit this section for solid reflectors
  mesh_spacing_mm: 5.0         # Mesh hole size
  wire_diameter_mm: 0.5        # Wire diameter
```

**Required Fields:**
- `antenna_id`: Unique identifier (alphanumeric + underscores)
- `antenna_name`: Human-readable name
- `reflector.diameter_m`: Positive value, typical range 1-70 meters
- `reflector.focal_length_m`: Positive value, typical f/D ratio 0.3-1.5
- `reflector.surface_rms_mm`: Non-negative, typical 0.1-5.0 mm
- `feeds[]`: At least one feed, unique feed_id values
- `feeds[].q_factor`: Positive, typical 1.0-30.0 (higher = more focused beam)
- `feeds[].frequency_range`: Valid MHz range, min < max

**Optional Fields:**
- `mesh`: Include only for mesh reflectors (omit for solid)
- `feeds[].position`: Defaults to [0, 0, 0] (focal point)
- `feeds[].phase_center_offset_m`: Defaults to 0.0

### 2.3 Design Specs Validation Rules

The calibration tool validates all design specs against physical constraints:

1. **Antenna ID**: Non-empty, no spaces
2. **Diameter**: Must be > 0, typically 1-70 meters
3. **f/D Ratio**: Must be in range [0.2, 2.0]
   - Calculated as: f/D = focal_length_m / diameter_m
   - Typical parabolic reflectors: 0.3-1.5
4. **Surface RMS**: Must be >= 0, typically 0.1-5.0 mm
5. **Feed IDs**: Must be unique within antenna
6. **q-Factor**: Must be > 0, typically 1.0-30.0
7. **Frequency Range**: Must have min < max, typically 100-50000 MHz
8. **Mesh (if present)**:
   - `wire_diameter_mm` < `mesh_spacing_mm`
   - Both must be positive
   - Typical mesh_spacing: 1-10 mm
   - Typical wire_diameter: 0.1-2.0 mm

**Validation Error Example:**
```
Error: Invalid f/D ratio: 2.5 (must be between 0.2 and 2.0)
  focal_length_m: 5.0
  diameter_m: 2.0
  Suggested: Increase diameter or decrease focal length
```

### 2.4 Loading Uncalibrated Antennas into Service

**Step 1:** Place design specs in `design_specs/` directory

**Step 2:** Reference in service configuration `config/service.yaml`:

```yaml
antennas:
  - antenna_id: "antenna_1"
    calibration_status: "uncalibrated"
    design_specs_path: "design_specs/small_groundstation.yaml"
```

**Step 3:** Start service - it will load design specs automatically:

```bash
cargo run --release --bin antenna-model
```

**Service Log Output:**
```
INFO  Loading antenna configuration: antenna_1
INFO  Calibration status: Uncalibrated (design specs)
INFO  Feeds loaded: x_band, s_band
INFO  Accuracy: ±4.0 dB absolute, ±2.0 dB loss
```

### 2.5 Use Cases for Uncalibrated Antennas

#### Use Case 1: Loss Analysis

**Scenario:** Compare loss between two emitter positions for path planning

**Query:**
```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "feed_id": "x_band",
    "vehicle_position": {"x": 0, "y": 0, "z": 0},
    "vehicle_attitude": {"roll": 0, "pitch": 0, "yaw": 0},
    "antenna_pointing": {"azimuth": 45, "elevation": 30},
    "feed_position": {"x": -6400000, "y": 1000000, "z": 2000000},
    "emitter_position": {"x": -6500000, "y": 1100000, "z": 2100000},
    "reference_position": {"x": -6500000, "y": 1100000, "z": 2000000},
    "frequency_mhz": 7500,
    "compute_loss": true
  }'
```

**Response:**
```json
{
  "gain_db": 43.2,
  "loss_db": 2.5,
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 4.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_applied": false,
    "parameters_source": "design_specifications"
  },
  "warnings": [
    "Antenna 'antenna_1' is uncalibrated (using design specifications). Absolute gain accuracy: ±4.0 dB, Loss accuracy: ±2.0 dB"
  ]
}
```

**Key Observations:**
- `loss_db` (2.5 dB) has ±2.0 dB accuracy - useful for comparative analysis
- `gain_db` (43.2 dB) has ±4.0 dB accuracy - less reliable for absolute predictions
- Warning clearly communicates limitations

#### Use Case 2: Design Validation

**Scenario:** Validate antenna design before manufacturing

Compare performance across frequency band:
```bash
for freq in 7100 7500 8000 8500; do
  curl -X POST http://localhost:3000/api/v1/gain \
    -H "Content-Type: application/json" \
    -d "{...\"frequency_mhz\": $freq}" | jq '.gain_db'
done
```

Expected trend validation: gain should be stable across X-band (±2-3 dB variation is normal).

---

## 3. Boresight Calibration Workflow

Boresight calibration tunes physical antenna parameters from frequency sweep measurements taken at the antenna's boresight (azimuth=0°, elevation=0°). This provides improved accuracy at boresight and better physics-based extrapolation off-axis.

### 3.1 Test Setup and Data Collection

#### Equipment Needed
- Antenna under test (installed and operational)
- Calibrated satellite beacon or ground source at known position
- Spectrum analyzer or receiver for G/T measurements
- Antenna control system to point at boresight

#### Test Procedure

1. **Point antenna at boresight** (azimuth=0°, elevation=0°)
   - Verify antenna is at mechanical boresight
   - Record ambient temperature

2. **Sweep frequency across operating band**
   - Take 10-50 measurements across feed's frequency range
   - Measure G/T (Gain-to-Temperature ratio) at each frequency
   - Typical spacing: 50-200 MHz

3. **Record measurements** in CSV format

**Typical Test Time:** 30-60 minutes (vs 6-8 hours for full grid)

### 3.2 Boresight Measurement CSV Format

Create a CSV file with three columns:

```csv
frequency_mhz,g_over_t_db,temperature_k
7100.0,40.2,290.0
7200.0,40.5,290.0
7300.0,40.8,290.0
7400.0,41.0,290.0
7500.0,41.2,290.0
7600.0,41.3,290.0
7700.0,41.4,290.0
7800.0,41.5,290.0
7900.0,41.5,290.0
8000.0,41.6,290.0
8100.0,41.5,290.0
8200.0,41.4,290.0
8300.0,41.3,290.0
8400.0,41.2,290.0
8500.0,41.0,290.0
```

**Column Descriptions:**
- `frequency_mhz`: Measurement frequency in MHz (must match feed's operating range)
- `g_over_t_db`: Measured G/T in dB/K
- `temperature_k`: Ambient temperature in Kelvin (typically 290K = 17°C)

**Requirements:**
- Header row required
- Minimum 4 measurement points (for cubic B-spline fitting)
- Recommended: 10-50 points for good frequency coverage
- All measurements at azimuth=0°, elevation=0° (boresight)
- Frequency range should cover or be within feed's operating range

### 3.3 Running Boresight Calibration

**Command:**
```bash
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

**Required Flags:**
- `--calibration-mode boresight`: Selects boresight calibration workflow
- `--input <path>`: Path to boresight measurements CSV
- `--design-specs <path>`: Path to design specs YAML (provides initial parameter estimates)
- `--output <path>`: Output calibration artifact (.bin file)
- `--antenna-id <id>`: Antenna identifier (must match design specs)
- `--feed-id <id>`: Feed identifier to calibrate (must exist in design specs)

**Optional Flags:**
- `--max-tuning-iterations <n>`: Maximum optimization iterations (default: 100)
- `--antenna-name <name>`: Human-readable antenna name (default: from design specs)
- `--metadata <path>`: Export calibration metadata as JSON
- `--verbose`: Enable debug logging
- `--validate`: Run cross-validation (optional for boresight)

### 3.4 Interpreting Calibration Results

**Successful Calibration Output:**
```
Boresight Calibration Tool
==========================

Loading design specifications from: design_specs/small_groundstation.yaml
  Antenna: antenna_1 (3.7m Ground Station - X/S-Band)
  Feed: x_band
  Initial surface_rms: 1.50 mm
  Initial q_factor: 8.0

Parsing boresight measurements from: examples/boresight_measurements_xband.csv
  Measurements loaded: 15 points
  Frequency range: 7100.0 - 8500.0 MHz
  Temperature: 290.0 K

Initial Physics Model Evaluation:
  RMSE (design specs): 2.34 dB

Starting parameter optimization (Nelder-Mead)...
Iteration 10: surface_rms=1.72, q_factor=8.9, RMSE=1.15 dB
Iteration 20: surface_rms=1.81, q_factor=9.1, RMSE=0.68 dB
Iteration 30: surface_rms=1.85, q_factor=9.2, RMSE=0.45 dB
Iteration 38: surface_rms=1.85, q_factor=9.2, RMSE=0.42 dB (converged)

Optimization complete!
  Iterations: 38
  Function evaluations: 156
  Final RMSE: 0.42 dB
  Improvement: 82% (from 2.34 dB)

Tuned Parameters:
  surface_rms_mm: 1.85 (was 1.50, change: +23%)
  q_factor: 9.2 (was 8.0, change: +15%)
  mesh_spacing_mm: 5.0 (not tuned - solid reflector)
  wire_diameter_mm: N/A (solid reflector)

Building calibration artifact...
  Calibration status: PartiallyCalibrated (boresight only)
  Coverage: azimuth [0.0, 0.0], elevation [0.0, 0.0], frequency [7100.0, 8500.0] MHz
  Measurements: 15 points
  Has correction surface: false (physics model only)

Expected Accuracy:
  Boresight: ±1.0 dB (at measured frequencies)
  Off-axis: ±2-3 dB (physics model extrapolation)
  Loss (relative): ±1-2 dB

Calibration artifact saved: calibration_data/antenna_1_xband_boresight.bin
  File size: 554 bytes
  Format: bincode v2.0 (AntennaCalibration)

Boresight Calibration Complete!
```

### 3.5 Understanding Calibration Metrics

**Initial RMSE (design specs):**
- Root-Mean-Square Error between measurements and physics model using design specifications
- High values (>2 dB) indicate design specs are not well-matched to actual hardware
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

### 3.6 Generated Calibration Artifact Structure

The `.bin` file contains:

**Metadata:**
- Antenna ID and name
- Feed ID and configuration
- Calibration timestamp
- Measurement count and quality metrics (RMSE, R²)

**Physical Configuration:**
- Reflector geometry (diameter, focal length, tuned surface RMS)
- Feed parameters (tuned q-factor, position, frequency range)
- Mesh properties (if applicable)

**Calibration Status:**
```rust
PartiallyCalibrated {
    accuracy_estimate_db: 1.5,  // Expected accuracy at boresight
    coverage: CalibrationCoverage {
        azimuth_range: (0.0, 0.0),      // Boresight only
        elevation_range: (0.0, 0.0),    // Boresight only
        frequency_range: (7100.0, 8500.0),  // Measured range
        num_measurements: 15,
        has_correction_surface: false,  // Optional frequency correction
    }
}
```

**Validity Ranges:**
- Physics model is valid for all azimuth/elevation angles
- Frequency range from measurements
- Temperature range (typically ±20K from measurement temperature)

### 3.7 Using Boresight Calibration in Service

**Service Configuration** (`calibration_data/antennas.toml`):
```toml
[[antennas]]
antenna_id = "antenna_1"
feeds = [
    { feed_id = "x_band", calibration_file = "antenna_1_xband_boresight.bin" }
]
```

**Service Startup:**
```bash
cargo run --release --bin antenna-model
```

**Log Output:**
```
INFO  Loading antenna: antenna_1
INFO  Feed: x_band
INFO    Calibration: PartiallyCalibrated (boresight)
INFO    Accuracy: ±1.5 dB (boresight), ±2-3 dB (off-axis)
INFO    Coverage: az=[0.0, 0.0], el=[0.0, 0.0], freq=[7100.0, 8500.0] MHz
INFO    Tuned parameters: surface_rms=1.85mm, q_factor=9.2
```

**API Query at Boresight:**
```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "feed_id": "x_band",
    ...
    "antenna_pointing": {"azimuth": 0, "elevation": 0},
    "frequency_mhz": 7500
  }'
```

**Response:**
```json
{
  "gain_db": 44.8,
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 1.5,
    "correction_applied": false,
    "parameters_source": "boresight_tuned",
    "coverage": {
      "azimuth_range_deg": [0.0, 0.0],
      "elevation_range_deg": [0.0, 0.0],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 15,
      "is_boresight_only": true
    }
  },
  "warnings": []
}
```

**API Query Off-Axis:**
```json
{
  "antenna_pointing": {"azimuth": 5, "elevation": 10}
}
```

**Response:**
```json
{
  "gain_db": 42.3,
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 2.5,
    ...
  },
  "warnings": [
    "Antenna 'antenna_1' is partially calibrated. Accuracy estimate: ±1.5 dB",
    "Query is outside calibrated region - using physics model extrapolation"
  ]
}
```

### 3.8 Common Issues and Solutions

**Issue 1: High Final RMSE (>1.5 dB)**

**Possible Causes:**
- Measurement errors (noise, interference)
- Wrong design specs file
- Physics model limitations (e.g., blockage effects not modeled)

**Solutions:**
- Review measurement quality (SNR, calibration)
- Verify design specs match actual hardware
- Increase measurement density (more frequency points)
- Check for environmental factors (temperature gradients, wind)

**Issue 2: Parameter Bounds Violations**

**Error:**
```
Warning: Parameter out of bounds: surface_rms=4.8 mm (max: 4.5 mm)
  Clamping to maximum value
```

**Cause:** Optimizer trying to explore parameters outside reasonable range

**Solution:**
- Review tuning bounds in design specs loader
- May indicate design specs are far from actual hardware
- Consider wider bounds if hardware is known to be non-standard

**Issue 3: Convergence Failure**

**Error:**
```
Warning: Optimization did not converge after 100 iterations
  Final RMSE: 1.8 dB
```

**Solutions:**
- Increase `--max-tuning-iterations` (try 200-500)
- Review initial parameter estimates (design specs)
- Check measurement data quality
- Consider starting with better initial guesses

**Issue 4: CSV Parsing Errors**

**Error:**
```
Error: Failed to parse CSV: Missing column 'frequency_mhz'
```

**Solution:**
- Verify CSV has header row: `frequency_mhz,g_over_t_db,temperature_k`
- Check for typos in column names (case-sensitive)
- Ensure no extra spaces or special characters

---

## 4. Full Grid Calibration Workflow

Full grid calibration fits a 4D correction surface from dense measurements across azimuth, elevation, and frequency. This provides ±1 dB accuracy everywhere in the calibrated region.

### 4.1 Test Planning

**Grid Density Recommendations:**
- **Azimuth sampling**: Every 10-30° (12-36 points over 360°)
- **Elevation sampling**: Every 3-10° (9-30 points over 0-90°)
- **Frequency sampling**: 5-20 points across operating band
- **Total measurements**: 500-3000 points typical

**Coverage Requirements:**
- Cover full operational field of view
- Denser sampling near boresight (higher gain gradient)
- Coarser sampling at high elevation angles (low gain region)

**Example Grid:**
```
Azimuth: [0, 10, 20, ..., 350] (36 points, 10° spacing)
Elevation: [0, 3, 6, 10, 15, 20, 30, 40, 50, 60, 70, 80, 90] (13 points)
Frequency: [7100, 7300, 7500, 7700, 7900, 8100, 8300, 8500] MHz (8 points)
Total: 36 × 13 × 8 = 3744 measurements
```

### 4.2 Full Grid Measurement CSV Format

Full calibration CSV uses antenna-frame coordinates:

```csv
e_clock,e_cone,frequency_mhz,g_over_t_db,temperature_k
0.0,0.0,7100.0,45.2,290.0
0.0,0.0,7300.0,45.5,290.0
...
10.0,5.0,7100.0,44.1,290.0
10.0,5.0,7300.0,44.3,290.0
...
```

**Column Descriptions:**
- `e_clock`: Azimuth angle in degrees (0-360)
- `e_cone`: Cone angle (off-axis) in degrees (0-90)
- `frequency_mhz`: Measurement frequency
- `g_over_t_db`: Measured G/T in dB/K
- `temperature_k`: Ambient temperature

**Coordinate Convention:**
- `e_clock=0, e_cone=0`: Boresight
- `e_clock`: Azimuth rotation around boresight axis
- `e_cone`: Off-axis angle from boresight

### 4.3 Running Full Grid Calibration

**Command:**
```bash
cargo run --release --bin calibrate -- \
  --calibration-mode full \
  --input measurements/antenna_1_full_grid.csv \
  --output calibration_data/antenna_1_xband_full.bin \
  --antenna-id antenna_1 \
  --feed-id x_band \
  --antenna-name "3.7m Ground Station - X-Band" \
  --validate
```

**Required Flags:**
- `--calibration-mode full` (or omit, full is default)
- `--input <path>`: Full grid measurements CSV
- `--output <path>`: Output artifact
- `--antenna-id <id>`: Antenna identifier

**Optional But Recommended:**
- `--validate`: Cross-validation to verify ±1 dB accuracy
- `--metadata <path>`: Export detailed calibration report

**Typical Runtime:** 30-120 seconds depending on grid density

### 4.4 Calibration Process Steps

The full calibration tool performs:

1. **Parse measurements** (CSV → data structures)
2. **Tune physical parameters** (differential evolution optimizer)
   - Optimizes surface RMS, q-factor, mesh properties
   - Uses same physics model as boresight calibration
3. **Compute residuals**: `residual = measured - physics_model`
4. **Fit correction surface** (4D B-spline to residuals)
   - Azimuth, elevation, frequency, (temperature)
   - Cubic B-spline interpolation
5. **Cross-validation** (if `--validate` flag)
   - Split data into training/test sets
   - Verify <1 dB error in main lobe and first sidelobe
6. **Generate artifact** with `FullyCalibrated` status

### 4.5 Validation and Quality Metrics

**Validation Report:**
```
Full Grid Calibration Complete
==============================

Measurements: 3744 points
  Azimuth: [0.0, 360.0] (36 samples)
  Elevation: [0.0, 90.0] (13 samples)
  Frequency: [7100.0, 8500.0] MHz (8 samples)

Parameter Tuning:
  Initial RMSE: 3.12 dB (design specs)
  Final RMSE: 0.68 dB (tuned)
  Improvement: 78%

Correction Surface Fitting:
  B-spline order: 3 (cubic)
  Control points: 48 × 15 × 10 = 7200
  Knot spacing: Adaptive (denser near boresight)
  Final residual RMSE: 0.32 dB

Cross-Validation (70% train / 30% test):
  Main lobe error (±3° from boresight): 0.45 dB RMS
  First sidelobe error (3-10° off-axis): 0.72 dB RMS
  Full coverage error: 0.88 dB RMS

Quality Metrics:
  R² score: 0.982 (excellent fit)
  Max absolute error: 1.8 dB (at elevation=85°, edge of coverage)

✓ All validation criteria met (<1 dB in main lobe and first sidelobe)

Output: calibration_data/antenna_1_xband_full.bin (128 KB)
```

**Quality Thresholds:**
- **Main lobe** (±3° from boresight): Must be <1.0 dB RMS
- **First sidelobe** (3-10° off-axis): Must be <1.0 dB RMS
- **Full coverage**: Target <1.5 dB RMS, <3 dB max absolute

### 4.6 Using Fully Calibrated Antennas

**API Response:**
```json
{
  "gain_db": 45.3,
  "loss_db": 2.1,
  "calibration_status": {
    "status": "fully_calibrated",
    "accuracy_estimate_db": 1.0,
    "correction_applied": true,
    "parameters_source": "measurement_tuned"
  }
}
```

**Key Differences from Boresight:**
- `"correction_applied": true` (B-spline correction surface used)
- No coverage warnings (full field of view calibrated)
- No extrapolation warnings (everywhere in-coverage)

---

## 5. Calibration Upgrade Path

The service supports a clear upgrade path from uncalibrated to fully calibrated, allowing incremental investment in test time.

### 5.1 Upgrade Workflow

```
┌──────────────────────────────────────────────────────────────┐
│                    Calibration Upgrade Path                  │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  Step 1: Uncalibrated (Design Specs)                        │
│  ├─ Input: Design specifications YAML                       │
│  ├─ Test time: 0 hours                                      │
│  ├─ Accuracy: ±3-5 dB absolute, ±2 dB loss                  │
│  └─ Use case: Initial planning, loss analysis               │
│                                                              │
│  Step 2: Boresight Calibration                              │
│  ├─ Input: Design specs + boresight measurements (15 pts)   │
│  ├─ Test time: ~1 hour                                      │
│  ├─ Accuracy: ±1 dB @ boresight, ±2-3 dB off-axis          │
│  ├─ Benefit: Improved physics model for extrapolation       │
│  └─ Use case: Rapid commissioning, verification             │
│                                                              │
│  Step 3: Full Grid Calibration                              │
│  ├─ Input: Full grid measurements (500-3000 pts)            │
│  ├─ Test time: ~8 hours                                     │
│  ├─ Accuracy: ±1 dB everywhere                              │
│  ├─ Benefit: Correction surface for all pointing angles     │
│  └─ Use case: Production operations, high precision         │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

### 5.2 When to Upgrade

**Uncalibrated → Boresight:**
- Need better absolute gain accuracy (from ±4 dB to ±1 dB at boresight)
- Commissioning after antenna installation
- Validating antenna performance vs design
- ~1 hour test time investment available

**Boresight → Full Grid:**
- Need ±1 dB accuracy at all pointing angles (not just boresight)
- Generating loss heatmaps for operational planning
- Acceptance testing for delivery
- ~8 hour test time investment available

### 5.3 Cost-Benefit Analysis

| Upgrade | Test Time Δ | Accuracy Gain | Typical Cost | ROI Scenario |
|---------|------------|---------------|--------------|--------------|
| None → Boresight | +1 hour | ±4 dB → ±1.5 dB @ boresight | $500-1000 | High - rapid validation |
| Boresight → Full | +7 hours | ±2-3 dB → ±1 dB everywhere | $3000-5000 | Medium - production operations |
| None → Full | +8 hours | ±4 dB → ±1 dB everywhere | $3500-6000 | Low - skip boresight |

**Recommendation:** Most users should start with boresight calibration, then upgrade to full grid only if needed.

### 5.4 Backward Compatibility

Each calibration level produces a valid `.bin` artifact that works in the service:
- Old artifacts continue to work (loaded as `FullyCalibrated` if no status field)
- API clients receive optional `calibration_status` field (backward compatible)
- Accuracy degrades gracefully based on calibration level

---

## 6. Using Calibrated Antennas in Service

### 6.1 Loading Calibration Artifacts

**Service Configuration** (`calibration_data/antennas.toml`):
```toml
[[antennas]]
antenna_id = "antenna_1"
feeds = [
    { feed_id = "x_band", calibration_file = "antenna_1_xband_boresight.bin" },
    { feed_id = "s_band", calibration_file = "antenna_1_sband_full.bin" }
]

[[antennas]]
antenna_id = "antenna_2"
feeds = [
    { feed_id = "primary", calibration_file = "antenna_2_primary.bin" }
]

# Uncalibrated antenna (design specs only)
[[antennas]]
antenna_id = "antenna_3"
design_specs_path = "design_specs/antenna_3.yaml"
```

**Service Startup:**
```bash
cargo run --release --bin antenna-model
```

**Log Output:**
```
INFO  Antenna Model Service starting...
INFO  Loading calibration data...
INFO
INFO  Antenna: antenna_1
INFO    Feed: x_band (partially calibrated - boresight)
INFO      Accuracy: ±1.5 dB @ boresight, ±2-3 dB off-axis
INFO      Coverage: freq=[7100, 8500] MHz
INFO    Feed: s_band (fully calibrated)
INFO      Accuracy: ±1.0 dB
INFO      Coverage: az=[0, 360], el=[0, 90], freq=[2000, 2300] MHz
INFO
INFO  Antenna: antenna_2
INFO    Feed: primary (fully calibrated)
INFO      Accuracy: ±1.0 dB
INFO
INFO  Antenna: antenna_3 (uncalibrated - design specs)
INFO    Accuracy: ±4.0 dB absolute, ±2.0 dB loss
INFO
INFO  Loaded 3 antennas, 4 feeds total
INFO  Service ready on http://localhost:3000
```

### 6.2 API Response Interpretation

**Fully Calibrated Response:**
```json
{
  "gain_db": 45.3,
  "loss_db": 2.1,
  "calibration_status": {
    "status": "fully_calibrated",
    "accuracy_estimate_db": 1.0,
    "correction_applied": true,
    "parameters_source": "measurement_tuned"
  }
}
```

**Interpretation:**
- `status`: "fully_calibrated" - highest accuracy level
- `accuracy_estimate_db`: 1.0 - expect ±1.0 dB error
- `correction_applied`: true - B-spline correction surface was used
- No warnings - query is within calibrated coverage

**Partially Calibrated (In-Coverage):**
```json
{
  "gain_db": 44.8,
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 1.5,
    "correction_applied": false,
    "coverage": {
      "azimuth_range_deg": [0.0, 0.0],
      "elevation_range_deg": [0.0, 0.0],
      "frequency_range_mhz": [7100.0, 8500.0],
      "is_boresight_only": true
    }
  },
  "warnings": [
    "Antenna 'antenna_1' is partially calibrated. Accuracy estimate: ±1.5 dB"
  ]
}
```

**Interpretation:**
- Query is at or near boresight (within coverage)
- Physics model with tuned parameters (no correction surface for boresight-only)
- Warning informs about calibration limitation

**Partially Calibrated (Out-of-Coverage):**
```json
{
  "gain_db": 42.1,
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 2.5,
    ...
  },
  "warnings": [
    "Antenna 'antenna_1' is partially calibrated. Accuracy estimate: ±1.5 dB",
    "Query is outside calibrated region - using physics model extrapolation"
  ]
}
```

**Interpretation:**
- Query is off-axis (outside boresight coverage)
- Physics model extrapolation (±2-3 dB accuracy)
- Additional warning about extrapolation

**Uncalibrated:**
```json
{
  "gain_db": 43.2,
  "loss_db": 2.5,
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 4.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_applied": false,
    "parameters_source": "design_specifications"
  },
  "warnings": [
    "Antenna 'antenna_1' is uncalibrated (using design specifications). Absolute gain accuracy: ±4.0 dB, Loss accuracy: ±2.0 dB"
  ]
}
```

**Interpretation:**
- Design specs only (no measured data)
- Lower absolute accuracy (±4 dB) but better loss accuracy (±2 dB)
- Use loss values for comparative analysis

### 6.3 Understanding Warnings

**Warning Types:**

1. **Uncalibrated Warning:**
   ```
   "Antenna 'antenna_1' is uncalibrated (using design specifications).
    Absolute gain accuracy: ±4.0 dB, Loss accuracy: ±2.0 dB"
   ```
   - Always present for uncalibrated antennas
   - Informs about accuracy limitations
   - Loss accuracy is better than absolute

2. **Partial Calibration Warning:**
   ```
   "Antenna 'antenna_1' is partially calibrated. Accuracy estimate: ±1.5 dB"
   ```
   - Present for all partially calibrated queries
   - Accuracy varies by location (in vs out of coverage)

3. **Extrapolation Warning:**
   ```
   "Query is outside calibrated region - using physics model extrapolation"
   ```
   - Query outside calibrated coverage
   - Accuracy degrades to ±2-3 dB
   - Physics model is still valid, just less accurate

4. **Correction Not Applied Warning:**
   ```
   "Correction surface not applied (out of coverage)"
   ```
   - Correction surface exists but not applicable to this query
   - Using physics model only

**Best Practices:**
- Always check `calibration_status` field in responses
- Use `accuracy_estimate_db` to assess reliability
- For uncalibrated: prefer loss over absolute gain
- For partial: verify coverage matches your query region

### 6.4 Calibration Status in Batch and Heatmap Endpoints

**Batch Endpoint:**
Each result in the batch includes calibration status:

```json
{
  "results": [
    {
      "request_id": 0,
      "gain_db": 45.2,
      "calibration_status": { ... }
    },
    {
      "request_id": 1,
      "gain_db": 44.8,
      "calibration_status": { ... }
    }
  ]
}
```

**Heatmap Endpoint:**
Single calibration status for the entire heatmap (same antenna/feed):

```json
{
  "heatmap": [
    {"azimuth": 0, "elevation": 0, "loss_db": 0.0},
    {"azimuth": 5, "elevation": 0, "loss_db": 0.5},
    ...
  ],
  "calibration_status": {
    "status": "fully_calibrated",
    "accuracy_estimate_db": 1.0
  }
}
```

---

# Part 2: Technical Reference

## 7. Calibration Status Types (Technical Detail)

### 7.1 CalibrationStatus Enum

The `CalibrationStatus` enum in `antenna-model/src/data/types.rs` defines three variants:

```rust
pub enum CalibrationStatus {
    FullyCalibrated {
        accuracy_estimate_db: f64,  // Typically ±1.0 dB
    },

    PartiallyCalibrated {
        accuracy_estimate_db: f64,          // Varies by coverage (±1.0-1.5 dB)
        coverage: CalibrationCoverage,      // Spatial/frequency coverage
    },

    Uncalibrated {
        accuracy_estimate_db: f64,           // ±3-5 dB (absolute gain)
        loss_accuracy_estimate_db: f64,      // ±2 dB (better than absolute)
    },
}
```

### 7.2 CalibrationCoverage Structure

Describes the spatial, frequency, and measurement density of partial calibration:

```rust
pub struct CalibrationCoverage {
    pub azimuth_range: (f64, f64),      // Min, max in degrees
    pub elevation_range: (f64, f64),    // Min, max in degrees
    pub frequency_range: (f64, f64),    // Min, max in MHz
    pub num_measurements: usize,        // Total measurement points
    pub has_correction_surface: bool,   // Whether correction surface fitted
}
```

**Methods:**
- `is_boresight_only() -> bool`: Returns true if azimuth and elevation ranges are both (0, 0)
- `contains(az, el, freq) -> bool`: Checks if query point is within coverage
- `validate() -> Result<()>`: Ensures range consistency (min ≤ max)

**Example Coverage Scenarios:**

**Boresight-only:**
```rust
CalibrationCoverage {
    azimuth_range: (0.0, 0.0),
    elevation_range: (0.0, 0.0),
    frequency_range: (7100.0, 8500.0),
    num_measurements: 15,
    has_correction_surface: false,  // Physics model only
}
```

**Sparse grid:**
```rust
CalibrationCoverage {
    azimuth_range: (0.0, 360.0),
    elevation_range: (30.0, 60.0),     // Limited elevation
    frequency_range: (8000.0, 8500.0),  // Limited frequency
    num_measurements: 324,
    has_correction_surface: true,       // Sparse B-spline
}
```

**Dense grid (full calibration):**
```rust
CalibrationCoverage {
    azimuth_range: (0.0, 360.0),
    elevation_range: (0.0, 90.0),
    frequency_range: (7100.0, 8500.0),
    num_measurements: 3744,
    has_correction_surface: true,
}
```

### 7.3 ParameterSource Enum

Indicates how physical antenna parameters were determined:

```rust
pub enum ParameterSource {
    DesignSpecifications,            // ±20-30% accuracy on parameters
    BoresightTuning { num_measurements: usize },  // ±5-10% accuracy
    PartialGridTuning { num_measurements: usize }, // ±5-10% accuracy
    FullGridTuning { num_measurements: usize },    // ±3-5% accuracy
}
```

**Relationship to Calibration Status:**
- **Uncalibrated** → `DesignSpecifications`
- **PartiallyCalibrated (boresight)** → `BoresightTuning`
- **PartiallyCalibrated (sparse grid)** → `PartialGridTuning`
- **FullyCalibrated** → `FullGridTuning`

### 7.4 MeasurementDensity Enum

Describes spatial measurement density relative to antenna beamwidth:

```rust
pub enum MeasurementDensity {
    None,                              // No measurements
    BoresightOnly,                     // Single spatial point
    Sparse { points_per_beam: f64 },   // 2-5 points per beamwidth
    Dense { points_per_beam: f64 },    // >10 points per beamwidth
}
```

**Impact on Accuracy:**
- **Dense** (>10 pts/beam): Can fit high-quality correction surface (±1 dB)
- **Sparse** (2-5 pts/beam): Parameter tuning only, limited correction (±1-2 dB)
- **BoresightOnly**: Parameter tuning at single point (±1.5 dB @ boresight, ±2-3 dB off-axis)
- **None**: Physics model with design specs (±3-5 dB)

---

## 8. Accuracy Estimation Methods

### 8.1 How accuracy_estimate_db is Calculated

**Fully Calibrated:**
```rust
accuracy_estimate_db = 1.0  // Fixed, based on validation criteria
```
- Cross-validation enforces <1 dB RMS in main lobe and first sidelobe
- Conservative estimate: may be better than 1 dB in practice

**Partially Calibrated (Boresight):**
```rust
if is_in_coverage(query) {
    accuracy_estimate_db = 1.5  // Boresight region
} else {
    accuracy_estimate_db = 2.5  // Off-axis (physics extrapolation)
}
```
- Boresight accuracy based on RMSE from parameter tuning (typically 0.4-0.8 dB)
- Conservative 1.5 dB estimate includes modeling uncertainties
- Off-axis uses physics model extrapolation (2-3 dB typical)

**Uncalibrated:**
```rust
accuracy_estimate_db = 4.0       // Absolute gain
loss_accuracy_estimate_db = 2.0  // Relative gain (loss)
```
- Absolute accuracy limited by parameter uncertainties (±20-30% on each parameter)
- Loss accuracy better due to systematic error cancellation
- Based on empirical validation with multiple antenna designs

### 8.2 Coverage-Dependent Accuracy

For partially calibrated antennas, accuracy depends on query location:

```rust
fn get_accuracy(
    status: &CalibrationStatus,
    query_az: f64,
    query_el: f64,
    query_freq: f64,
) -> f64 {
    match status {
        CalibrationStatus::FullyCalibrated { accuracy_estimate_db } => {
            *accuracy_estimate_db  // Same everywhere
        },

        CalibrationStatus::PartiallyCalibrated { coverage, .. } => {
            if coverage.contains(query_az, query_el, query_freq) {
                1.5  // In-coverage: tuned physics model
            } else {
                2.5  // Out-of-coverage: physics extrapolation
            }
        },

        CalibrationStatus::Uncalibrated { accuracy_estimate_db, .. } => {
            *accuracy_estimate_db  // Same everywhere (design specs)
        },
    }
}
```

### 8.3 Loss vs Absolute Gain Accuracy

**Why Loss Has Better Accuracy for Uncalibrated:**

Loss is computed as:
```
Loss(dB) = Gain(reference) - Gain(target)
```

**Systematic errors cancel:**
- Surface RMS error affects both directions similarly → cancels
- Feed q-factor error affects both directions similarly → cancels
- Mesh transparency affects both directions similarly → cancels

**Random errors remain:**
- Pointing errors
- Frequency-dependent effects
- Spatial variations

**Result:** Uncalibrated loss accuracy (±2 dB) is better than absolute gain (±3-5 dB).

**Example:**
```
True:        Gain_ref = 45.0 dB,  Gain_target = 42.0 dB  →  Loss = 3.0 dB
Predicted:   Gain_ref = 43.0 dB,  Gain_target = 40.0 dB  →  Loss = 3.0 dB
                       ↑                    ↑
                 -2 dB error          -2 dB error
                                            ↓
                                    Loss error = 0 dB (canceled!)
```

### 8.4 Error Cancellation in Uncalibrated Mode

**Parameters with Good Cancellation (loss):**
- `surface_rms_mm`: Affects all directions via Ruze efficiency
- `q_factor`: Affects all directions via feed pattern
- `mesh_spacing_mm`, `wire_diameter_mm`: Affect all directions via mesh transparency

**Parameters with Partial Cancellation:**
- `focal_length_m`: Affects coma aberration (direction-dependent)
- `diameter_m`: Affects beamwidth (some cancellation)

**No Cancellation:**
- Feed position errors (coma lobe direction-dependent)
- Blockage effects (highly directional)

**Conclusion:** Loss predictions for uncalibrated antennas are reliable (±2 dB) for most scenarios.

---

## 9. Boresight Optimization Algorithm

### 9.1 Nelder-Mead Simplex Method

Boresight calibration uses the **Nelder-Mead simplex** algorithm (from `argmin` crate) to tune physical parameters:

**Algorithm Overview:**
1. Create initial simplex: n+1 vertices for n parameters
   - Start from design spec values
   - Perturb each parameter by ±10% to create simplex vertices
2. Evaluate cost function at each vertex
3. Identify best, worst, second-worst vertices
4. Perform simplex operations: reflection, expansion, contraction, shrinkage
5. Repeat until convergence (standard deviation < tolerance)

**Advantages:**
- Derivative-free (no gradients needed)
- Robust to noisy cost functions
- Fast convergence for small parameter sets (2-4 parameters)
- Well-suited for physics models (non-linear, potentially non-smooth)

### 9.2 Tuned Parameters

**For All Antennas:**
- `surface_rms_mm`: Surface quality (RMS error)
  - Affects: Ruze efficiency, gain loss at all angles
  - Tuning bounds: [nominal × 0.3, nominal × 3.0]
  - Typical change: ±20-50%

- `q_factor`: Feed illumination pattern parameter
  - Affects: Beamwidth, sidelobe levels, spillover
  - Tuning bounds: [nominal × 0.5, nominal × 2.0]
  - Typical change: ±10-30%

**For Mesh Reflectors Only:**
- `mesh_spacing_mm`: Hole size in wire mesh
  - Affects: Mesh transparency (frequency-dependent)
  - Tuning bounds: [nominal × 0.5, nominal × 2.0]
  - Typical change: ±10-20%

- `wire_diameter_mm`: Wire thickness
  - Affects: Mesh transparency
  - Tuning bounds: [nominal × 0.5, nominal × 2.0]
  - Constraint: wire_diameter < mesh_spacing

**Fixed Parameters (Not Tuned):**
- `diameter_m`, `focal_length_m`: Measured geometrically
- Feed position: Assumed at focal point for boresight calibration
- Temperature: Assumed constant

### 9.3 Parameter Tuning Bounds

Bounds prevent optimizer from exploring unrealistic parameter values:

```rust
pub fn compute_tuning_bounds(design_specs: &DesignSpecs) -> ParameterBounds {
    ParameterBounds {
        surface_rms_mm: (
            design_specs.reflector.surface_rms_mm * 0.3,
            design_specs.reflector.surface_rms_mm * 3.0,
        ),

        q_factor: (
            design_specs.feed.q_factor * 0.5,
            design_specs.feed.q_factor * 2.0,
        ),

        mesh_spacing_mm: design_specs.mesh.map(|m| (
            m.mesh_spacing_mm * 0.5,
            m.mesh_spacing_mm * 2.0,
        )),

        wire_diameter_mm: design_specs.mesh.map(|m| (
            m.wire_diameter_mm * 0.5,
            m.wire_diameter_mm * 2.0,
        )),
    }
}
```

**Out-of-Bounds Penalty:**
If optimizer proposes parameters outside bounds, cost function returns very high value (1e6) to discourage exploration in that region.

### 9.4 Cost Function (RMSE Computation)

The cost function computes **Root-Mean-Square Error** between measurements and physics model predictions:

```rust
pub fn compute_rmse(
    parameters: &TunableParameters,
    measurements: &BoresightMeasurements,
    geometry: &ReflectorGeometry,
) -> f64 {
    let mut squared_errors = Vec::new();

    for measurement in &measurements.points {
        // Predict G/T using physics model with candidate parameters
        let predicted_g_over_t = compute_g_over_t(
            geometry,
            parameters,
            measurement.frequency_mhz,
            0.0,  // azimuth (boresight)
            0.0,  // elevation (boresight)
        );

        // Compute error
        let error = measurement.g_over_t_db - predicted_g_over_t;
        squared_errors.push(error * error);
    }

    // RMSE
    let mean_squared_error = squared_errors.iter().sum::<f64>() / squared_errors.len() as f64;
    mean_squared_error.sqrt()
}
```

**Optimization Goal:** Minimize RMSE

**Typical RMSE Values:**
- Initial (design specs): 1.5-3.0 dB
- Final (tuned): 0.3-0.8 dB
- Excellent: <0.5 dB
- Good: 0.5-1.0 dB

### 9.5 Convergence Criteria

**Nelder-Mead Convergence:**
```rust
NelderMead::new()
    .with_sd_tolerance(1e-4)  // Standard deviation of simplex function values
```

**Early Stopping:**
```rust
if rmse < 0.1 {
    // Excellent fit achieved, stop early
    break;
}
```

**Maximum Iterations:**
- Default: 100 iterations
- Configurable via `--max-tuning-iterations` flag
- Typical convergence: 20-50 iterations

**Convergence Indicators:**
- Simplex standard deviation < 1e-4
- RMSE improvement < 0.01 dB per iteration
- Maximum iterations reached

### 9.6 Optimization Logging

**Verbose Mode** (`--verbose` flag):
```
Iteration 10: surface_rms=1.72, q_factor=8.9, RMSE=1.15 dB
Iteration 20: surface_rms=1.81, q_factor=9.1, RMSE=0.68 dB
Iteration 30: surface_rms=1.85, q_factor=9.2, RMSE=0.45 dB
Iteration 38: surface_rms=1.85, q_factor=9.2, RMSE=0.42 dB (converged)
```

**Final Report:**
```
Optimization complete!
  Iterations: 38
  Function evaluations: 156
  Final RMSE: 0.42 dB
  Improvement: 82% (from 2.34 dB)
```

---

## 10. API Integration

### 10.1 CalibrationStatusInfo Structure

API responses include calibration status via `CalibrationStatusInfo`:

```rust
pub struct CalibrationStatusInfo {
    pub status: String,                           // "fully_calibrated", "partially_calibrated", "uncalibrated"
    pub accuracy_estimate_db: f64,               // Expected accuracy
    pub loss_accuracy_estimate_db: Option<f64>,  // For uncalibrated only
    pub coverage: Option<CoverageInfo>,          // For partially calibrated only
    pub correction_applied: bool,                // Whether correction surface was used
    pub parameters_source: String,               // "measurement_tuned", "design_specifications"
}
```

**Conversion from CalibrationStatus:**
```rust
impl From<&CalibrationStatus> for CalibrationStatusInfo {
    fn from(status: &CalibrationStatus) -> Self {
        match status {
            CalibrationStatus::FullyCalibrated { accuracy_estimate_db } => {
                CalibrationStatusInfo {
                    status: "fully_calibrated".to_string(),
                    accuracy_estimate_db: *accuracy_estimate_db,
                    loss_accuracy_estimate_db: None,
                    coverage: None,
                    correction_applied: false,  // Updated during evaluation
                    parameters_source: "measurement_tuned".to_string(),
                }
            },

            CalibrationStatus::PartiallyCalibrated { accuracy_estimate_db, coverage } => {
                CalibrationStatusInfo {
                    status: "partially_calibrated".to_string(),
                    accuracy_estimate_db: *accuracy_estimate_db,
                    loss_accuracy_estimate_db: None,
                    coverage: Some(coverage.into()),
                    correction_applied: false,
                    parameters_source: if coverage.is_boresight_only() {
                        "boresight_tuned".to_string()
                    } else {
                        "partial_grid_tuned".to_string()
                    },
                }
            },

            CalibrationStatus::Uncalibrated { accuracy_estimate_db, loss_accuracy_estimate_db } => {
                CalibrationStatusInfo {
                    status: "uncalibrated".to_string(),
                    accuracy_estimate_db: *accuracy_estimate_db,
                    loss_accuracy_estimate_db: Some(*loss_accuracy_estimate_db),
                    coverage: None,
                    correction_applied: false,
                    parameters_source: "design_specifications".to_string(),
                }
            },
        }
    }
}
```

### 10.2 Correction Surface Application Logic

**Service Layer** (`src/service/evaluator.rs`):

```rust
// Step 1: Compute physics model gain
let physics_gain_db = compute_physics_model(
    &calibration.physical_config,
    azimuth_deg,
    elevation_deg,
    frequency_mhz,
);

// Step 2: Check if correction surface should be applied
let correction_applied = calibration.correction_surface.is_some()
    && is_in_coverage(&calibration.calibration_coverage, azimuth_deg, elevation_deg, frequency_mhz);

// Step 3: Apply correction if applicable
let final_gain_db = if correction_applied {
    let correction_db = interpolate_correction_surface(
        &calibration.correction_surface.unwrap(),
        azimuth_deg,
        elevation_deg,
        frequency_mhz,
        temperature_k,
    );
    physics_gain_db + correction_db
} else {
    physics_gain_db
};

// Step 4: Update calibration status info
calibration_status_info.correction_applied = correction_applied;
```

**Coverage Check:**
```rust
fn is_in_coverage(
    coverage: &Option<CalibrationCoverage>,
    azimuth_deg: f64,
    elevation_deg: f64,
    frequency_mhz: f64,
) -> bool {
    match coverage {
        Some(cov) => cov.contains(azimuth_deg, elevation_deg, frequency_mhz),
        None => false,  // Uncalibrated: no coverage
    }
}
```

### 10.3 Warning Generation Rules

**Uncalibrated Antennas:**
```rust
if status == "uncalibrated" {
    warnings.push(format!(
        "Antenna '{}' is uncalibrated (using design specifications). \
         Absolute gain accuracy: ±{:.1} dB, Loss accuracy: ±{:.1} dB",
        antenna_id,
        accuracy_estimate_db,
        loss_accuracy_estimate_db.unwrap_or(accuracy_estimate_db)
    ));
}
```

**Partially Calibrated Antennas:**
```rust
if status == "partially_calibrated" {
    // Main warning
    warnings.push(format!(
        "Antenna '{}' is partially calibrated. Accuracy estimate: ±{:.1} dB",
        antenna_id,
        accuracy_estimate_db
    ));

    // Out-of-coverage warning
    if !is_in_coverage {
        warnings.push(
            "Query is outside calibrated region - using physics model extrapolation"
                .to_string()
        );
    }

    // Correction not applied warning
    if has_correction_surface && !correction_applied {
        warnings.push("Correction surface not applied (out of coverage)".to_string());
    }
}
```

**Fully Calibrated:**
```rust
// No warnings (backward compatible)
```

### 10.4 Backward Compatibility

**v1.0 Calibration Files (No calibration_status field):**
```rust
// In data loader
if calibration.calibration_status.is_none() {
    // Assume fully calibrated for backward compatibility
    calibration.calibration_status = Some(CalibrationStatus::FullyCalibrated {
        accuracy_estimate_db: 1.0,
    });
}
```

**API Responses:**
```rust
// calibration_status field is Option<CalibrationStatusInfo>
// Serialization skips None values (backward compatible)
#[serde(skip_serializing_if = "Option::is_none")]
pub calibration_status: Option<CalibrationStatusInfo>
```

**Old API Clients:**
- Receive responses without `calibration_status` field (omitted when None)
- Forward compatible: new fields ignored by old parsers
- No breaking changes

**New API Clients:**
- Should check for presence of `calibration_status` field
- If present: use accuracy estimates and warnings
- If absent: assume fully calibrated or unknown quality

---

## 11. Appendix: Design Specs Reference

### 11.1 Complete Field Reference

```yaml
# Antenna identification
antenna_id: string (required)          # Unique identifier, alphanumeric + underscores
antenna_name: string (optional)        # Human-readable name

# Reflector geometry
reflector:
  diameter_m: float (required)         # Dish diameter in meters, >0
  focal_length_m: float (required)     # Focal length in meters, >0
  surface_rms_mm: float (required)     # Surface RMS error in mm, ≥0

# Feeds (array, at least one required)
feeds:
  - feed_id: string (required)         # Unique feed identifier
    name: string (optional)            # Human-readable feed name
    position: [x, y, z] (optional)     # Position relative to focal point (meters), default: [0, 0, 0]
    q_factor: float (required)         # Feed illumination pattern, >0, typical: 1-30
    phase_center_offset_m: float (optional)  # Phase center offset along feed axis, default: 0.0
    frequency_range: [min, max] (required)   # Operating frequency range in MHz

# Mesh properties (optional, omit for solid reflectors)
mesh:
  mesh_spacing_mm: float (required if mesh present)     # Mesh hole size in mm, >0
  wire_diameter_mm: float (required if mesh present)    # Wire diameter in mm, >0
```

### 11.2 Validation Rules Summary

| Field | Constraint | Typical Range |
|-------|-----------|---------------|
| `antenna_id` | Non-empty, no spaces | "antenna_1" |
| `diameter_m` | > 0 | 1-70 |
| `focal_length_m` | > 0, f/D ∈ [0.2, 2.0] | 0.3-30 |
| `surface_rms_mm` | ≥ 0 | 0.1-5.0 |
| `q_factor` | > 0 | 1.0-30.0 |
| `frequency_range` | min < max | [100, 50000] MHz |
| `mesh_spacing_mm` | > wire_diameter_mm | 1-10 |
| `wire_diameter_mm` | > 0, < mesh_spacing | 0.1-2.0 |

### 11.3 Multi-Feed Configuration

**Example: Antenna with 3 Feeds**
```yaml
antenna_id: "dss43"
antenna_name: "DSS-43 Deep Space Station"

reflector:
  diameter_m: 70.0
  focal_length_m: 28.0
  surface_rms_mm: 0.5

feeds:
  - feed_id: "x_band_downlink"
    name: "X-Band Downlink (8.4 GHz)"
    position: [0.0, 0.0, 0.0]
    q_factor: 12.0
    frequency_range: [8400.0, 8450.0]

  - feed_id: "ka_band_downlink"
    name: "Ka-Band Downlink (32 GHz)"
    position: [0.15, 0.0, 0.0]  # Offset feed
    q_factor: 15.0
    frequency_range: [31800.0, 32300.0]

  - feed_id: "ka_band_uplink"
    name: "Ka-Band Uplink (34 GHz)"
    position: [-0.15, 0.0, 0.0]  # Offset feed
    q_factor: 15.0
    frequency_range: [34200.0, 34700.0]
```

**Key Points:**
- Each feed can have different position (for offset feeds)
- Each feed can have different q-factor (different illumination patterns)
- Each feed must have unique `feed_id`
- Boresight calibration is done per-feed (must specify `--feed-id`)

### 11.4 Mesh vs Solid Reflectors

**Solid Reflector:**
```yaml
reflector:
  diameter_m: 13.0
  focal_length_m: 5.2
  surface_rms_mm: 0.5

# Omit mesh section entirely
```

**Mesh Reflector:**
```yaml
reflector:
  diameter_m: 7.3
  focal_length_m: 3.65
  surface_rms_mm: 1.0

mesh:
  mesh_spacing_mm: 3.0
  wire_diameter_mm: 0.3
```

**When to Use Mesh:**
- Lower frequency bands (L, S, C-band) where mesh holes are electrically small
- Weight reduction requirements
- Cost reduction (mesh is cheaper than solid)

**Mesh Transparency:**
- Frequency-dependent loss due to mesh holes
- Modeled via physics: `transparency = f(mesh_spacing, wire_diameter, frequency, incidence_angle)`
- Negligible at low frequencies (mesh_spacing « λ)
- Significant at high frequencies (mesh_spacing ~ λ)

**Rule of Thumb:** Mesh spacing should be < λ/10 for <1% transparency loss
- Example: 3mm mesh → use up to 10 GHz (λ = 30mm)
- Example: 5mm mesh → use up to 6 GHz (λ = 50mm)

### 11.5 Example Design Specs

See provided example files:
- `design_specs/small_groundstation.yaml` - 3.7m, X/S-band, mesh
- `design_specs/medium_groundstation.yaml` - 7.3m, X/Ka-band, mesh
- `design_specs/solid_reflector.yaml` - 13m, DSN-class, solid, multi-feed

---

## Summary

This guide provides complete workflows for all three calibration levels:

1. **Uncalibrated (Design Specs)**: Zero test time, ±2 dB loss accuracy - ideal for planning
2. **Boresight Calibration**: 1 hour test time, ±1.5 dB at boresight - ideal for commissioning
3. **Full Grid Calibration**: 8 hour test time, ±1 dB everywhere - ideal for production

**Key Takeaways:**
- Calibration is incremental: uncalibrated → boresight → full grid
- Loss accuracy is better than absolute for uncalibrated/partially calibrated
- API responses include calibration status with accuracy estimates
- Service gracefully degrades accuracy based on calibration level
- All calibration levels produce valid `.bin` artifacts usable in service

**For More Information:**
- API Examples: `examples/README.md`
- Boresight Examples: `examples/README_boresight.md`
- Architecture: `docs/architecture.md`
- Implementation Plan: `docs/implementation-plan.md`
