# Partial Calibration Support - Setup Summary

**Date:** 2025-01-15
**Status:** Ready for implementation

---

## What Was Created

### 1. Design Specifications (3 files)

**Location:** `calibration_data/design_specs/`

These YAML files define physical antenna parameters for uncalibrated antennas:

- **`small_groundstation.yaml`**
  - 3.7m mesh reflector
  - Dual-feed (S-band, X-band)
  - Commercial ground station specification
  - Surface RMS: 1.5mm (estimated)

- **`medium_dsn.yaml`**
  - 13m solid reflector
  - Triple-feed (X-band uplink/downlink, Ka-band)
  - Deep space network specification
  - Surface RMS: 0.4mm (high precision)

- **`large_dsn.yaml`**
  - 34m solid reflector
  - Triple-feed (S-band, X-band, Ka-band)
  - Large DSN antenna specification
  - Surface RMS: 0.25mm (ultra-high precision)

**Purpose:**
- Initial parameter estimates for uncalibrated antennas
- Tuning bounds for parameter optimization
- Documentation of design specifications

---

### 2. Antenna Configuration File (Updated)

**Location:** `calibration_data/antennas.yaml`

Comprehensive antenna registry with **4 enabled uncalibrated antennas** ready for immediate testing:

#### Test Antennas (Currently Enabled):

1. **`gs_3.7m_uncalibrated`**
   - Small ground station (3.7m mesh)
   - Dual-feed: S-band, X-band
   - **Status:** Uncalibrated - design specs only
   - **Use:** Loss analysis, feed steering simulation
   - **Accuracy:** Absolute ±3 dB, Loss ±2 dB

2. **`dsn_13m_uncalibrated`**
   - Medium DSN antenna (13m solid)
   - Triple-feed: X-band up/down, Ka-band
   - **Status:** Uncalibrated
   - **Use:** Multi-band loss analysis
   - **Accuracy:** Absolute ±3 dB, Loss ±2 dB

3. **`dsn_34m_uncalibrated`**
   - Large DSN antenna (34m solid)
   - Triple-feed: S-band, X-band, Ka-band
   - **Status:** Uncalibrated
   - **Use:** High-precision loss modeling
   - **Accuracy:** Absolute ±3 dB, Loss ±2 dB

4. **`test_simple`**
   - Simple 5m test antenna
   - Single-feed: X-band
   - **Status:** Uncalibrated
   - **Use:** Development and testing

#### Placeholder Antennas (Currently Disabled):

- **Fully calibrated:** `dsn_34m_full` (requires `.bin` file)
- **Partially calibrated (boresight):** `gs_3.7m_boresight`, `dsn_13m_boresight`
- **Partially calibrated (limited):** `gs_3.7m_limited`

**Total Feeds Available:** 10 feeds across 4 enabled antennas

---

### 3. Design Documentation (3 files)

#### `docs/partial-calibration-design.md` (26 KB)
Comprehensive design document covering:
- Calibration status hierarchy (Uncalibrated → Partial → Full)
- YAML configuration schema
- Data type extensions
- API response schema updates
- Calibration tool enhancements
- Service layer changes
- Upgrade workflow

#### `docs/partial-calibration-implementation-plan.md` (32 KB)
Detailed implementation plan with:
- **Phase 1:** Data model & service support (Sprint 6, 2-3 days)
- **Phase 2:** Boresight calibration tool (Sprint 7, 3-4 days)
- **Phase 3:** Limited coverage (Sprint 7, optional, 2-3 days)
- **Phase 4:** Testing & documentation (Sprint 7, 2-3 days)
- Task breakdowns with acceptance criteria
- Risk management
- Testing strategy (140+ new tests)

#### `docs/partial-calibration-setup-summary.md` (this file)
Quick reference and next steps

---

## Key Design Decisions

### 1. **Loss Accuracy Priority**
- **Uncalibrated antennas provide better loss accuracy** (±2 dB) than absolute gain (±3 dB)
- Systematic errors (surface RMS, q-factor) **cancel** in loss computation: `Loss = Reference_Gain - Actual_Gain`
- Both gains computed with same (potentially incorrect) parameters → errors subtract out

### 2. **Physics Model Unchanged**
- Existing physics engine (`model/pattern.rs`, `model/phase.rs`, etc.) requires **no modifications**
- Already operates on `PhysicalAntennaConfig` parameters regardless of source
- Parameters come from design specs (uncalibrated) OR tuning (calibrated)

### 3. **Never Reject Queries**
- All calibration statuses return best-effort responses
- Warnings indicate quality, but queries always succeed
- Supports immediate operational use of uncalibrated antennas

### 4. **Graceful Upgrade Path**
- Uncalibrated → Boresight Partial → Limited Partial → Fully Calibrated
- Simple YAML edits to upgrade (change `calibration_file`, add `calibration_coverage`)
- No code changes required for upgrades

### 5. **Boresight Calibration Priority**
- **Primary use case:** Boresight measurements across frequency
- Fast to collect (~1 hour vs days for full grid)
- Enables parameter tuning (surface RMS, q-factor, mesh properties)
- Improves loss accuracy from ±2 dB to ±1-2 dB

---

## What Happens Next

### Immediate Testing (Before Implementation)

You can **test the configuration** right now:

```bash
# Parse antennas.yaml (will fail with config parsing error - expected)
cargo run --release --bin antenna-model

# Expected error: "Unknown field `design_specs`" or similar
# This is expected - config parsing not yet implemented
```

**Why test now?**
- Validates YAML syntax
- Confirms file locations
- Identifies any structural issues before coding

---

### Implementation Phases

#### **Phase 1: Service Support (Sprint 6, 2-3 days)**

**Tasks:**
1. Extend data types: `CalibrationStatus`, `CalibrationCoverage`, etc.
2. Update config parsing to read `design_specs` from YAML
3. Repository: Load uncalibrated antennas from design specs
4. Service: Handle all calibration statuses (physics + optional correction)
5. API: Add `calibration_status` to all responses
6. Tests: 60-80 new unit/integration tests

**Outcome:** Service can query uncalibrated antennas for loss analysis

**Example Request (after Phase 1):**
```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "gs_3.7m_uncalibrated",
    "feed_id": "x_band_feed",
    "vehicle_position": {"x": -118.0, "y": 34.0, "z": 100.0},
    "vehicle_attitude": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
    "reflector_boresight": {"x": -117.0, "y": 35.0, "z": 400000.0},
    "feed_position": {"x": -118.0, "y": 34.0, "z": 105.0},
    "emitter_position": {"x": -117.0, "y": 35.0, "z": 400000.0},
    "frequency_mhz": 8400.0,
    "include_reference": true
  }'

# Expected response:
{
  "gain_db": 41.2,
  "reference_gain_db": 43.5,
  "loss_db": 2.3,
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 3.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_applied": false,
    "parameters_source": "design_specifications"
  },
  "warnings": [
    "Antenna 'gs_3.7m_uncalibrated' is uncalibrated (using design specifications). Absolute gain accuracy: ±3.0 dB, Loss accuracy: ±2.0 dB"
  ]
}
```

---

#### **Phase 2: Boresight Calibration (Sprint 7, 3-4 days)**

**Tasks:**
1. Calibration tool: Add `--calibration-mode boresight` flag
2. Parameter optimization: Tune surface RMS, q-factor, mesh from boresight data
3. Design specs loading: Parse YAML design files
4. Optional frequency correction: Fit 1D B-spline if residuals > 0.5 dB
5. Tests: 40+ calibration tool tests

**Outcome:** Engineers can upgrade uncalibrated → boresight-calibrated

**Example Workflow:**
```bash
# 1. Collect boresight measurements (CSV)
cat > measurements/gs_3.7m_boresight.csv << EOF
frequency_mhz,temperature_k,g_over_t_db
7100,290,39.5
7500,290,40.2
8000,290,40.8
8400,290,41.2
EOF

# 2. Run calibration tool
cargo run --release --bin calibrate -- \
  --input measurements/gs_3.7m_boresight.csv \
  --output calibration_data/gs_3.7m_boresight.bin \
  --antenna-id gs_3.7m_boresight \
  --feed-id x_band_feed \
  --calibration-mode boresight \
  --design-specs calibration_data/design_specs/small_groundstation.yaml \
  --validate

# 3. Update antennas.yaml (change enabled: false → true for gs_3.7m_boresight)

# 4. Restart service
cargo run --release --bin antenna-model

# 5. Query now uses tuned parameters (improved accuracy)
```

---

#### **Phase 3: Limited Coverage (Sprint 7, optional)**

**Tasks:**
1. Partial grid calibration mode
2. Sparse correction surface fitting
3. Coverage detection and metadata

**Outcome:** Support sparse measurement grids (e.g., main lobe only)

---

#### **Phase 4: Testing & Docs (Sprint 7, 2-3 days)**

**Tasks:**
1. Integration tests: End-to-end workflows
2. Documentation: Architecture, workflow guide, API docs
3. Validation: Accuracy checks, upgrade path testing

---

## File Structure Summary

```
antenna_model/
├── calibration_data/
│   ├── antennas.yaml                    # ✅ UPDATED - 4 uncalibrated antennas enabled
│   └── design_specs/                    # ✅ NEW DIRECTORY
│       ├── small_groundstation.yaml     # ✅ NEW - 3.7m mesh antenna
│       ├── medium_dsn.yaml              # ✅ NEW - 13m solid antenna
│       └── large_dsn.yaml               # ✅ NEW - 34m solid antenna
│
├── docs/
│   ├── partial-calibration-design.md                  # ✅ NEW - 26 KB design doc
│   ├── partial-calibration-implementation-plan.md     # ✅ NEW - 32 KB implementation plan
│   └── partial-calibration-setup-summary.md           # ✅ NEW - This file
│
├── antenna-model/src/
│   ├── data/
│   │   ├── types.rs            # 📋 TO UPDATE - Add CalibrationStatus enum
│   │   ├── repository.rs       # 📋 TO UPDATE - Add load_uncalibrated_antenna()
│   │   └── loader.rs           # ⚠️ No changes needed
│   ├── service/
│   │   ├── evaluator.rs        # 📋 TO UPDATE - Handle all statuses
│   │   └── validator.rs        # ⚠️ No changes needed
│   ├── api/
│   │   ├── schemas.rs          # 📋 TO UPDATE - Add CalibrationStatusInfo
│   │   └── handlers.rs         # 📋 TO UPDATE - Enhanced responses
│   └── config/
│       └── settings.rs         # 📋 TO UPDATE - Parse design_specs
│
└── calibrate/src/
    ├── main.rs                 # 📋 TO UPDATE - Add --calibration-mode flag
    ├── boresight_calibration.rs   # 📋 NEW MODULE - Boresight optimization
    ├── design_specs_loader.rs     # 📋 NEW MODULE - Load design specs
    └── frequency_correction.rs    # 📋 NEW MODULE - 1D correction surface
```

**Legend:**
- ✅ **NEW/UPDATED** - Already created/modified
- 📋 **TO UPDATE** - Requires implementation (Phases 1-3)
- ⚠️ **No changes** - Works as-is

---

## Testing the Configuration

### Quick Validation

Check YAML syntax:
```bash
# Install yq (YAML processor) if needed
brew install yq  # macOS
# or: apt-get install yq  # Linux

# Validate antennas.yaml syntax
yq eval '.' calibration_data/antennas.yaml > /dev/null && echo "✅ Valid YAML" || echo "❌ Syntax error"

# Count enabled antennas
yq eval '.antennas[] | select(.enabled == true) | .id' calibration_data/antennas.yaml

# Expected output:
# gs_3.7m_uncalibrated
# dsn_13m_uncalibrated
# dsn_34m_uncalibrated
# test_simple

# Count total feeds
yq eval '.antennas[] | select(.enabled == true) | .design_specs.feeds[].id' calibration_data/antennas.yaml | wc -l
# Expected: 10 feeds
```

### Review Antenna Details

```bash
# View uncalibrated antenna configuration
yq eval '.antennas[] | select(.id == "gs_3.7m_uncalibrated")' calibration_data/antennas.yaml

# View design specs for small ground station
cat calibration_data/design_specs/small_groundstation.yaml
```

---

## Expected Behavior After Implementation

### Uncalibrated Antenna Query

**Request:**
```json
POST /api/v1/gain
{
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "x_band_feed",
  "frequency_mhz": 8400.0,
  ...
}
```

**Response:**
```json
{
  "gain_db": 41.2,
  "loss_db": 2.3,
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 3.0,
    "loss_accuracy_estimate_db": 2.0,
    "parameters_source": "design_specifications",
    "correction_applied": false
  },
  "warnings": [
    "Antenna 'gs_3.7m_uncalibrated' is uncalibrated (using design specifications). Absolute gain accuracy: ±3.0 dB, Loss accuracy: ±2.0 dB"
  ]
}
```

### Antenna List Query

**Request:**
```
GET /api/v1/antennas
```

**Response:**
```json
{
  "antennas": [
    {
      "antenna_id": "gs_3.7m_uncalibrated",
      "name": "Ground Station 3.7m - Uncalibrated",
      "calibration_status": "uncalibrated",
      "feeds": ["s_band_feed", "x_band_feed"],
      "feed_count": 2
    },
    {
      "antenna_id": "dsn_13m_uncalibrated",
      "name": "DSN 13m - Uncalibrated",
      "calibration_status": "uncalibrated",
      "feeds": ["x_band_downlink", "x_band_uplink", "ka_band_downlink"],
      "feed_count": 3
    },
    ...
  ],
  "total_count": 4
}
```

---

## Accuracy Expectations

### By Calibration Status

| Status | Absolute Gain | Loss (Relative Gain) | Coverage | Time to Calibrate |
|--------|---------------|---------------------|----------|-------------------|
| **Uncalibrated** | ±3-5 dB | ±2-3 dB | All | 0 (immediate) |
| **Boresight Partial** | ±1 dB (boresight)<br>±2-3 dB (off-axis) | ±1-2 dB | All (physics extrapolation) | ~1 hour |
| **Limited Partial** | ±1-1.5 dB (in-coverage)<br>±2-3 dB (out) | ±1-1.5 dB | Limited region | ~4-8 hours |
| **Fully Calibrated** | ±1 dB | ±1 dB | Full FOV | 1-3 days |

### Why Loss is More Accurate

**Uncalibrated antenna example:**
- Surface RMS incorrect (1.5mm design vs 2.0mm actual)
- Both reference gain and actual gain biased **low** by ~1.5 dB
- Loss = (Reference - 1.5 dB) - (Actual - 1.5 dB) = Reference - Actual ✅
- **Error cancels** in the subtraction

---

## Next Steps

### For Development (Start with Phase 1):

1. **Review design documents:**
   - Read `docs/partial-calibration-design.md`
   - Read `docs/partial-calibration-implementation-plan.md`

2. **Start Task 6.4:** Extend data types (4-6 hours)
   - Add `CalibrationStatus` enum
   - Add `CalibrationCoverage` struct
   - Update `AntennaCalibration` structure

3. **Continue with Task 6.5:** Config parsing (4-6 hours)
   - Parse `design_specs` from YAML
   - Parse `calibration_coverage`

4. **Complete Task 6.6:** Repository updates (6-8 hours)
   - Implement `load_uncalibrated_antenna()`

### For Testing (After Phase 1 Complete):

```bash
# 1. Start service with uncalibrated antennas
cargo run --release --bin antenna-model

# 2. Query antenna list
curl http://localhost:3000/api/v1/antennas

# 3. Query specific antenna details
curl http://localhost:3000/api/v1/antennas/gs_3.7m_uncalibrated

# 4. Compute gain and loss
curl -X POST http://localhost:3000/api/v1/gain -d @test_request.json

# 5. Generate heatmap
curl -X POST http://localhost:3000/api/v1/heatmap -d @heatmap_request.json
```

---

## Questions?

Before starting implementation, clarify:

1. **Timeline:** Is end of Sprint 7 (2 weeks) acceptable?
2. **Priority:** Confirm boresight calibration is highest priority?
3. **Deferment:** OK to defer Phase 3 (limited coverage) to Sprint 8 if needed?
4. **Testing:** Is ~140 new tests sufficient coverage?

---

**Status:** Ready to begin Phase 1 implementation! 🚀

All configuration files are in place, design is complete, and implementation plan is detailed. The system is ready to support uncalibrated and partially calibrated antennas while maintaining full backward compatibility with existing fully-calibrated workflows.
