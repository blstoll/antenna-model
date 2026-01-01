# Integration Tests - Test Data Setup Guide

This document describes how to set up and run the integration tests for the Antenna Model Service.

## Overview

The integration tests verify the complete API workflow from HTTP requests through the physics engine to the final gain computation. Tests cover:

- All 10 API endpoints (health, status, gain, batch, heatmap, antennas, feeds)
- All three calibration statuses (uncalibrated, boresight-calibrated, fully-calibrated)
- Concurrent request handling and thread safety
- Error recovery and validation

## Test Data Structure

```
tests/
├── integration.rs              # Main test entry point
├── integration/
│   ├── mod.rs                  # Module organization
│   ├── helpers.rs              # Test utilities (server, HTTP client, validators)
│   ├── api_tests.rs            # API endpoint tests (30 tests)
│   ├── partial_calibration_tests.rs  # Calibration status tests (17 tests)
│   └── concurrent_tests.rs     # Concurrent access tests (9 tests)
└── fixtures/
    ├── test_antennas.yaml      # Test antenna configuration
    ├── test_service.yaml       # Test service configuration
    ├── calibration_data/       # Generated calibration artifacts
    │   ├── test_uncalibrated_xband_boresight.bin  # Boresight X-band
    │   └── test_uncalibrated_sband_boresight.bin  # Boresight S-band
    ├── measurements/           # Synthetic measurement data
    │   ├── test_boresight_xband.csv  # X-band frequency sweep
    │   ├── test_boresight_sband.csv  # S-band frequency sweep
    │   └── test_full_grid_primary_dense.csv  # Dense grid (for future full calibration)
    └── design_specs_test_uncalibrated.yaml  # Design specs for test antenna
```

## Test Antennas

The test suite uses 4 test antennas representing all calibration statuses:

### 1. Boresight-Calibrated Antenna (X-Band) - `test_boresight_xband`
- **Status:** `PartiallyCalibrated` (boresight only)
- **Calibration file:** `test_uncalibrated_xband_boresight.bin`
- **Frequency range:** 7.1 - 8.5 GHz
- **Measurements:** 15 frequency points at boresight (az=0°, el=0°)
- **Expected accuracy:** ±1 dB at boresight, ±2-3 dB off-axis

### 2. Boresight-Calibrated Antenna (S-Band) - `test_boresight_sband`
- **Status:** `PartiallyCalibrated` (boresight only)
- **Calibration file:** `test_uncalibrated_sband_boresight.bin`
- **Frequency range:** 2.0 - 2.3 GHz
- **Measurements:** 7 frequency points at boresight
- **Expected accuracy:** ±1 dB at boresight, ±2-3 dB off-axis

### 3. Uncalibrated Antenna - `test_uncalibrated`
- **Status:** `Uncalibrated`
- **Configuration:** Design specs only (3.7m, f/D=0.5, mesh reflector)
- **Feeds:** X-band and S-band
- **Expected accuracy:** ±3-5 dB absolute, ±2 dB loss (relative)

### 4. Simple Uncalibrated Antenna - `test_simple`
- **Status:** `Uncalibrated`
- **Configuration:** 5.0m solid reflector, single feed
- **Frequency range:** 8.0 - 8.5 GHz
- **Purpose:** Basic testing with simple parameters

## How Test Data Was Generated

### Step 1: Synthetic Measurement Data

Synthetic measurement CSV files were created with realistic G/T values based on typical antenna performance:

- **Boresight measurements:** Frequency sweeps at az=0°, el=0° (15 points for X-band, 7 for S-band)
- **Full grid measurements:** Dense angular grid (136 points) for future full calibration testing

Files created:
- `fixtures/measurements/test_boresight_xband.csv`
- `fixtures/measurements/test_boresight_sband.csv`
- `fixtures/measurements/test_full_grid_primary_dense.csv`

### Step 2: Design Specifications

Design specs files created based on test antenna configurations:

```yaml
# fixtures/design_specs_test_uncalibrated.yaml
antenna_id: "test_uncalibrated"
reflector:
  diameter_m: 3.7
  focal_length_m: 1.85
  surface_rms_mm: 1.5
feeds:
  - feed_id: "x_band"
    q_factor: 8.0
    frequency_range: [7100.0, 8500.0]
  # ... (see file for complete specs)
```

### Step 3: Generate Calibration Artifacts

Boresight calibration artifacts were generated using the `calibrate` tool:

```bash
# X-band boresight calibration
cargo run --release --bin calibrate -- \
  --calibration-mode boresight \
  --input antenna-model/tests/fixtures/measurements/test_boresight_xband.csv \
  --design-specs antenna-model/tests/fixtures/design_specs_test_uncalibrated.yaml \
  --output antenna-model/tests/fixtures/calibration_data/test_uncalibrated_xband_boresight.bin \
  --antenna-id test_uncalibrated \
  --feed-id x_band \
  --verbose

# S-band boresight calibration
cargo run --release --bin calibrate -- \
  --calibration-mode boresight \
  --input antenna-model/tests/fixtures/measurements/test_boresight_sband.csv \
  --design-specs antenna-model/tests/fixtures/design_specs_test_uncalibrated.yaml \
  --output antenna-model/tests/fixtures/calibration_data/test_uncalibrated_sband_boresight.bin \
  --antenna-id test_uncalibrated \
  --feed-id s_band \
  --verbose
```

The calibration process:
1. Loads design specs as initial parameter estimates
2. Optimizes physical parameters (surface_rms, q_factor, mesh parameters) using Nelder-Mead
3. Generates `.bin` artifact with `PartiallyCalibrated` status
4. Includes calibration coverage metadata (boresight-only)

## Running the Tests

### Run all integration tests:
```bash
cargo test -p antenna-model --test integration
```

### Run specific test suite:
```bash
# API endpoint tests
cargo test -p antenna-model --test integration api_tests

# Partial calibration tests
cargo test -p antenna-model --test integration partial_calibration_tests

# Concurrent access tests
cargo test -p antenna-model --test integration concurrent_tests
```

### Run specific test:
```bash
cargo test -p antenna-model --test integration test_boresight_calibrated_status
```

### Run with output:
```bash
cargo test -p antenna-model --test integration -- --nocapture
```

## Test Results

Current status: **✅ All 42 tests passing** (as of 2025-11-27)

```
test result: ok. 42 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Test Coverage by Category:

- **API Tests:** 20 tests
  - Health/status endpoints (3)
  - Single/batch gain computation (7)
  - Heatmap generation (2)
  - Antenna/feed listing and details (8)

- **Partial Calibration Tests:** 13 tests
  - Calibration status verification (3)
  - Uncalibrated antenna workflows (7)
  - Mixed calibration scenarios (3)

- **Concurrent Tests:** 9 tests
  - Parallel request handling (5)
  - Thread safety (2)
  - Error handling under load (2)

## Regenerating Test Data

If you need to regenerate the test calibration artifacts:

1. **Clean existing artifacts:**
   ```bash
   rm antenna-model/tests/fixtures/calibration_data/*.bin
   ```

2. **Regenerate boresight calibrations:**
   ```bash
   # Run the commands from Step 3 above
   ```

3. **Verify tests pass:**
   ```bash
   cargo test -p antenna-model --test integration
   ```

## Adding New Test Antennas

To add a new test antenna configuration:

1. **Create measurement data:** Add CSV file to `fixtures/measurements/`
2. **Create design specs (if boresight):** Add YAML to `fixtures/`
3. **Generate calibration artifact:** Run `calibrate` tool
4. **Update test_antennas.yaml:** Add antenna configuration
5. **Run tests:** Verify all tests pass

Example antenna configuration:
```yaml
- id: "my_test_antenna"
  name: "My Test Antenna"
  calibration_status: "partially_calibrated"
  calibration_file: "tests/fixtures/calibration_data/my_antenna.bin"
  enabled: true
  calibration_coverage:
    azimuth_range: [0.0, 0.0]
    elevation_range: [0.0, 0.0]
    frequency_range: [8000.0, 8500.0]
    num_measurements: 10
  description: "Custom test antenna"
```

## Troubleshooting

### Tests fail with "Failed to load calibration data"

**Cause:** Calibration file paths are incorrect or files don't exist.

**Solution:**
1. Check that `.bin` files exist in `fixtures/calibration_data/`
2. Verify paths in `test_antennas.yaml` are relative to workspace root
3. Regenerate calibration artifacts if needed

### Tests fail with "Configuration error"

**Cause:** Invalid antenna configuration in `test_antennas.yaml`.

**Solution:**
1. Validate YAML syntax
2. Ensure `partially_calibrated` antennas have `calibration_file` field
3. Ensure `uncalibrated` antennas have `design_specs` field

### Boresight calibration tool crashes

**Cause:** Nelder-Mead simplex initialization bug (fixed in Task 7.4b).

**Solution:**
- Ensure you're using the latest version with the simplex fix in `calibrate/src/boresight_calibration.rs`

## References

- **Implementation Plan:** `docs/implementation-plan.md` (Task 7.4b)
- **Boresight Calibration Guide:** `examples/README_boresight.md`
- **Test Infrastructure:** `antenna-model/tests/integration/helpers.rs`

## Notes

- Test server runs on port 3001 (different from production 3000) to avoid conflicts
- All tests use realistic physical antenna models with proper geometry
- Concurrent tests verify thread safety with up to 50 parallel requests
- Test data is deterministic and reproducible
- Expected test time: ~3-5 seconds for full suite
