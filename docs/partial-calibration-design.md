# Partial and Uncalibrated Antenna Support - Design Document

## Executive Summary

This document describes enhancements to the Antenna Model Service to support **partially calibrated** and **uncalibrated antennas** while maintaining the existing fully-calibrated antenna workflow. The system will always provide best-effort responses, prioritizing **loss accuracy (1-2 dB)** over absolute gain accuracy.

**Priority:**
1. **Partially calibrated antennas** (highest priority) - Some measurements available, typically boresight-only
2. **Uncalibrated antennas** (lower priority) - Design specifications only

**Key Design Principles:**
- **Never reject queries** - Always return best-effort predictions with quality indicators
- **Parameter tuning prioritized** - Use measurements to optimize physical parameters
- **Correction surface optional** - Secondary priority for partial calibration
- **Graceful upgrades** - Support evolving from uncalibrated → partially → fully calibrated
- **Loss accuracy focus** - Systematic errors cancel in loss computation (reference_gain - actual_gain)

---

## 1. Calibration Status Model

### 1.1 Status Hierarchy

```
Uncalibrated
    ↓ (add boresight measurements)
Partially Calibrated - Boresight Only
    ↓ (add off-axis measurements)
Partially Calibrated - Limited Coverage
    ↓ (add full measurement grid)
Fully Calibrated
```

### 1.2 Status Definitions

#### Uncalibrated
- **Data Available:** Design specifications (diameter, f/D, feed location, surface quality estimate)
- **Physics Model:** Default parameters from design specs
- **Correction Surface:** None
- **Accuracy Estimate:**
  - Absolute gain: ±3-5 dB
  - Loss (relative gain): ±2-3 dB (systematic errors partially cancel)
- **Use Cases:** New antennas, prototype modeling, fallback when data unavailable

#### Partially Calibrated - Boresight Only
- **Data Available:** Boresight measurements (az=0, el=0) across frequency and optionally temperature
- **Physics Model:** Parameters tuned to match boresight measurements (surface RMS, q-factor, mesh properties)
- **Correction Surface:** Optional, typically frequency-only (single spatial point)
- **Accuracy Estimate:**
  - Absolute gain (boresight): ±1 dB (tuned)
  - Absolute gain (off-axis): ±2-3 dB (physics model only)
  - Loss (relative): ±1-2 dB (excellent - systematic errors cancel)
- **Use Cases:** Feed steering analysis, quick calibration validation, operational antennas with limited test data

#### Partially Calibrated - Limited Coverage
- **Data Available:** Measurements at sparse grid (e.g., main lobe + first sidelobe only)
- **Physics Model:** Parameters tuned to measurements
- **Correction Surface:** Optional, sparse B-spline (limited spatial coverage)
- **Accuracy Estimate:**
  - In-coverage: ±1-1.5 dB
  - Out-of-coverage: ±2-3 dB (extrapolated)
  - Loss: ±1-1.5 dB
- **Use Cases:** Operational antennas with partial characterization, targeted measurement campaigns

#### Fully Calibrated
- **Data Available:** Dense measurement grid across azimuth, elevation, frequency
- **Physics Model:** Fully tuned parameters
- **Correction Surface:** Dense B-spline capturing all residuals
- **Accuracy Estimate:** ±1 dB (main lobe and first sidelobe)
- **Use Cases:** Critical science antennas, deep space network, high-accuracy applications

---

## 2. YAML Configuration Schema

### 2.1 Enhanced Schema

```yaml
antennas:
  # ==== Fully Calibrated Antenna (existing workflow) ====
  - id: "antenna_1_calibrated"
    name: "Deep Space Network 34m"
    calibration_file: "antenna_1.bin"  # Full calibration artifact
    enabled: true

  # ==== Partially Calibrated - Boresight Only ====
  - id: "antenna_2_boresight"
    name: "Ground Station 2 - Boresight Calibration"
    calibration_status: "partially_calibrated"

    # Optional: Partial calibration artifact with tuned parameters
    calibration_file: "antenna_2_boresight.bin"

    # Metadata about calibration coverage
    calibration_coverage:
      azimuth_range: [0.0, 0.0]      # Single point
      elevation_range: [0.0, 0.0]    # Single point
      frequency_range: [2000.0, 2300.0]  # MHz
      num_measurements: 25            # Frequency samples at boresight

    enabled: true

  # ==== Partially Calibrated - Limited Coverage ====
  - id: "antenna_3_limited"
    name: "Ground Station 3 - Limited Calibration"
    calibration_status: "partially_calibrated"
    calibration_file: "antenna_3_limited.bin"

    calibration_coverage:
      azimuth_range: [0.0, 360.0]
      elevation_range: [30.0, 60.0]  # Limited elevation range
      frequency_range: [7100.0, 8500.0]
      num_measurements: 450

    enabled: true

  # ==== Uncalibrated Antenna (design specs only) ====
  - id: "antenna_4_uncalibrated"
    name: "Ground Station 4 - Uncalibrated"
    calibration_status: "uncalibrated"
    # No calibration_file - use design specs

    # Design specifications (required for uncalibrated)
    design_specs:
      # Reflector
      diameter_m: 3.7
      focal_length_m: 1.85
      f_over_d_ratio: 0.5
      surface_rms_mm: 1.5  # Estimate from specifications or visual inspection

      # Feed (can have multiple feeds)
      feeds:
        - id: "x_band_feed"
          name: "X-Band Primary Feed"
          position: [0.0, 0.0, 0.0]  # At focal point (x, y, z in meters)
          q_factor: 8.0  # Typical for horn feed
          phase_center_offset_m: 0.0
          frequency_range: [7100.0, 8500.0]  # MHz

        - id: "s_band_feed"
          name: "S-Band Secondary Feed"
          position: [0.05, 0.0, 0.0]  # Offset from focal point
          q_factor: 7.0
          phase_center_offset_m: 0.0
          frequency_range: [2000.0, 2300.0]

      # Mesh parameters (optional, for mesh reflectors)
      mesh:
        mesh_spacing_mm: 5.0
        wire_diameter_mm: 0.5

    # Validity ranges (conservative estimates)
    validity_ranges:
      azimuth_range: [0.0, 360.0]
      elevation_range: [0.0, 90.0]
      frequency_range: [2000.0, 8500.0]  # Union of feed ranges
      temperature_k: 290.0  # Assume room temperature

    enabled: true
```

### 2.2 Schema Fields

| Field | Required | Description |
|-------|----------|-------------|
| `id` | Yes | Unique antenna identifier |
| `name` | Yes | Human-readable name |
| `calibration_status` | No | "fully_calibrated" (default), "partially_calibrated", "uncalibrated" |
| `calibration_file` | Conditional | Required for fully/partially calibrated; absent for uncalibrated |
| `calibration_coverage` | No | Metadata about measurement coverage (for partial calibration) |
| `design_specs` | Conditional | Required for uncalibrated; optional for others (fallback defaults) |
| `validity_ranges` | No | Override default validity ranges |
| `enabled` | Yes | Whether antenna is available for queries |

---

## 3. Calibration Data Types Enhancement

### 3.1 Extended `AntennaCalibration` Structure

```rust
// In src/data/types.rs

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct AntennaCalibration {
    pub antenna_id: String,
    pub feed_id: String,
    pub metadata: CalibrationMetadata,
    pub physical_config: PhysicalAntennaConfig,
    pub correction_surface: Option<BSplineModel4D>,
    pub validity_ranges: ValidityRanges,

    // NEW: Calibration status and quality
    pub calibration_status: CalibrationStatus,
    pub calibration_coverage: Option<CalibrationCoverage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum CalibrationStatus {
    /// Fully calibrated with dense measurement grid
    FullyCalibrated {
        accuracy_estimate_db: f64,  // e.g., 1.0
    },

    /// Partially calibrated with limited measurements
    PartiallyCalibrated {
        accuracy_estimate_db: f64,  // e.g., 1.5
        coverage: CalibrationCoverage,
    },

    /// Uncalibrated - using design specifications
    Uncalibrated {
        accuracy_estimate_db: f64,  // e.g., 3.0
        loss_accuracy_estimate_db: f64,  // e.g., 2.0 (better due to cancellation)
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct CalibrationCoverage {
    /// Azimuth range with measurements (degrees)
    pub azimuth_range: (f64, f64),

    /// Elevation range with measurements (degrees)
    pub elevation_range: (f64, f64),

    /// Frequency range with measurements (MHz)
    pub frequency_range: (f64, f64),

    /// Number of measurement points
    pub num_measurements: usize,

    /// Whether correction surface is available
    pub has_correction_surface: bool,
}
```

### 3.2 Extended `CalibrationMetadata`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct CalibrationMetadata {
    // ... existing fields ...

    // NEW: Source of physical parameters
    pub parameters_source: ParameterSource,

    // NEW: Measurement density indicator
    pub measurement_density: MeasurementDensity,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum ParameterSource {
    /// Parameters from design specifications
    DesignSpecifications,

    /// Parameters tuned from boresight measurements only
    BoresightTuning { num_measurements: usize },

    /// Parameters tuned from partial measurement grid
    PartialGridTuning { num_measurements: usize },

    /// Parameters tuned from full measurement grid
    FullGridTuning { num_measurements: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub enum MeasurementDensity {
    None,                          // Uncalibrated
    BoresightOnly,                 // Single spatial point
    Sparse { points_per_beam: f64 },  // e.g., 2-5 points per beamwidth
    Dense { points_per_beam: f64 },   // e.g., >10 points per beamwidth
}
```

---

## 4. Calibration Tool Enhancements

### 4.1 Partial Calibration Workflow

The `calibrate` CLI tool will support partial calibration:

```bash
# Boresight-only calibration
cargo run --release --bin calibrate -- \
  --input measurements/antenna_2_boresight.csv \
  --output calibration_data/antenna_2_boresight.bin \
  --antenna-id antenna_2 \
  --feed-id x_band_feed \
  --calibration-mode boresight \
  --design-specs design_specs/antenna_2.yaml

# Limited-coverage calibration
cargo run --release --bin calibrate -- \
  --input measurements/antenna_3_partial.csv \
  --output calibration_data/antenna_3_limited.bin \
  --antenna-id antenna_3 \
  --feed-id primary_feed \
  --calibration-mode partial \
  --design-specs design_specs/antenna_3.yaml
```

### 4.2 Calibration Modes

#### Mode 1: Boresight-Only Calibration

**Input:**
- CSV with boresight measurements: `frequency_mhz, temperature_k, g_over_t_db`
- All measurements at `(azimuth, elevation) = (0, 0)`

**Process:**
1. **Load design specs** as initial parameter estimates
2. **Tune physical parameters** using differential evolution:
   - Optimize: `surface_rms_mm`, `q_factor`, `mesh_spacing_mm`, `wire_diameter_mm`
   - Objective: Minimize `|measured_G/T - physics_model_G/T|` at boresight
   - Constraints: Keep parameters within physically reasonable ranges
3. **Optional correction surface:**
   - Fit 1D frequency-only correction: `correction(freq) = measured - physics`
   - Low priority - skip if physics model error < 0.5 dB
4. **Validate:** Check that tuned parameters are physically reasonable
5. **Output:** `.bin` artifact with:
   - Status: `PartiallyCalibrated`
   - Tuned `PhysicalAntennaConfig`
   - Optional frequency-only correction
   - Coverage: azimuth=[0,0], elevation=[0,0]

**Accuracy Expectations:**
- Boresight: <1 dB (tuned)
- Off-axis: 2-3 dB (physics model, untested)
- Loss: 1-2 dB (error cancellation)

#### Mode 2: Limited-Coverage Calibration

**Input:**
- CSV with sparse measurements: `azimuth_deg, elevation_deg, frequency_mhz, temperature_k, g_over_t_db`
- Measurements cover partial angular range (e.g., main lobe only)

**Process:**
1. **Load design specs** as initial estimates
2. **Tune physical parameters** across all measurement points
3. **Optional correction surface:**
   - Fit sparse 3D B-spline (azimuth, elevation, frequency)
   - Use measurements to construct sparse grid
   - Low priority - only if physics model has systematic bias
4. **Validate:** Check in-coverage accuracy
5. **Output:** `.bin` artifact with coverage metadata

**Accuracy Expectations:**
- In-coverage: 1-1.5 dB
- Out-of-coverage: 2-3 dB (extrapolated)
- Loss: 1-1.5 dB

### 4.3 Design Specs File Format

```yaml
# design_specs/antenna_2.yaml
antenna:
  reflector:
    diameter_m: 3.7
    focal_length_m: 1.85
    f_over_d_ratio: 0.5
    surface_rms_mm: 1.5  # Initial estimate

  feeds:
    - id: "x_band_feed"
      position: [0.0, 0.0, 0.0]
      q_factor: 8.0  # Initial estimate
      phase_center_offset_m: 0.0
      frequency_range: [7100.0, 8500.0]

  mesh:  # Optional
    mesh_spacing_mm: 5.0
    wire_diameter_mm: 0.5

# Parameter tuning bounds (for optimizer)
tuning_bounds:
  surface_rms_mm: [0.5, 3.0]
  q_factor: [6.0, 12.0]
  mesh_spacing_mm: [3.0, 10.0]
  wire_diameter_mm: [0.3, 1.0]
```

---

## 5. Service Layer Changes

### 5.1 Repository Enhancement

```rust
// src/data/repository.rs

impl CalibrationRepository {
    /// Load antenna from configuration (supports all calibration statuses)
    pub fn load_antenna(&mut self, config: &AntennaConfig) -> Result<(), DataError> {
        match config.calibration_status.as_deref() {
            Some("uncalibrated") | None if config.calibration_file.is_none() => {
                // Uncalibrated: construct from design specs
                self.load_uncalibrated_antenna(config)?;
            }
            Some("partially_calibrated") | Some("fully_calibrated") | _ => {
                // Load from calibration file
                let cal_file = config.calibration_file.as_ref()
                    .ok_or_else(|| DataError::ConfigurationError(
                        format!("Antenna {} requires calibration_file", config.id)
                    ))?;
                let calibration = loader::load_calibration_artifact(cal_file)?;
                self.add_calibration(calibration)?;
            }
        }
        Ok(())
    }

    /// Construct uncalibrated antenna from design specs
    fn load_uncalibrated_antenna(&mut self, config: &AntennaConfig) -> Result<(), DataError> {
        let design = config.design_specs.as_ref()
            .ok_or_else(|| DataError::ConfigurationError(
                format!("Uncalibrated antenna {} requires design_specs", config.id)
            ))?;

        // Build physical config from design specs
        for feed_spec in &design.feeds {
            let physical_config = PhysicalAntennaConfig {
                reflector: ReflectorGeometry {
                    diameter_m: design.diameter_m,
                    focal_length_m: design.focal_length_m,
                    f_over_d_ratio: design.f_over_d_ratio,
                    surface_rms_mm: design.surface_rms_mm,
                },
                feed: FeedParameters {
                    position: feed_spec.position,
                    q_factor: feed_spec.q_factor,
                    phase_center_offset_m: feed_spec.phase_center_offset_m,
                },
                mesh: design.mesh.clone(),
            };

            let calibration = AntennaCalibration {
                antenna_id: config.id.clone(),
                feed_id: feed_spec.id.clone(),
                metadata: CalibrationMetadata {
                    antenna_name: config.name.clone(),
                    calibration_date: "N/A".to_string(),
                    format_version: "2.0".to_string(),
                    data_source: "design_specifications".to_string(),
                    rmse_db: f64::NAN,
                    r_squared: f64::NAN,
                    num_measurements: 0,
                    notes: Some("Uncalibrated - using design specifications".to_string()),
                    physics_only_rmse_db: None,
                    correction_improvement_db: None,
                    parameters_tuned: false,
                    antenna_class: None,
                },
                physical_config,
                correction_surface: None,
                validity_ranges: ValidityRanges {
                    azimuth_min_max: (0.0, 360.0),
                    elevation_min_max: (0.0, 90.0),
                    frequency_min_max: (feed_spec.frequency_range.0, feed_spec.frequency_range.1),
                    temperature_const: 290.0,
                },
                calibration_status: CalibrationStatus::Uncalibrated {
                    accuracy_estimate_db: 3.0,
                    loss_accuracy_estimate_db: 2.0,
                },
                calibration_coverage: None,
            };

            self.add_calibration(calibration)?;
        }

        Ok(())
    }
}
```

### 5.2 Evaluator Enhancement

```rust
// src/service/evaluator.rs

pub fn compute_gain_from_request(
    request: &GainRequest,
    repository: &CalibrationRepository,
) -> Result<GainResponse, ServiceError> {
    // 1. Load calibration (all statuses supported)
    let calibration = repository.get_calibration(&request.antenna_id, &request.feed_id)?;

    // 2. Transform coordinates to antenna frame
    let geometry = compute_geometry_from_request(request)?;

    // 3. Compute physics-based gain (always executed)
    let gain_physics = compute_physics_model(
        &calibration.physical_config,
        &geometry,
        request.frequency_mhz,
    )?;

    // 4. Apply correction surface (if available and in range)
    let (correction_db, correction_applied) = match &calibration.correction_surface {
        Some(correction) => {
            if is_in_coverage(&calibration.calibration_coverage, &geometry) {
                let corr = evaluate_correction_surface(correction, &geometry, request.frequency_mhz)?;
                (corr, true)
            } else {
                // Out of coverage - skip correction
                (0.0, false)
            }
        }
        None => (0.0, false),
    };

    let final_gain_db = gain_physics + correction_db;

    // 5. Compute reference gain if requested
    let (reference_gain_db, loss_db) = if request.include_reference {
        let ref_geometry = compute_reference_geometry(request)?;
        let ref_gain_physics = compute_physics_model(
            &calibration.physical_config,
            &ref_geometry,
            request.frequency_mhz,
        )?;

        // Note: Reference typically doesn't use correction surface
        // (it's for ideal boresight case)
        let loss = ref_gain_physics - final_gain_db;
        (Some(ref_gain_physics), Some(loss))
    } else {
        (None, None)
    };

    // 6. Generate warnings
    let mut warnings = Vec::new();

    // Calibration status warnings
    match &calibration.calibration_status {
        CalibrationStatus::Uncalibrated { accuracy_estimate_db, loss_accuracy_estimate_db } => {
            warnings.push(format!(
                "Antenna '{}' is uncalibrated (using design specifications). \
                 Absolute gain accuracy: ±{:.1} dB, Loss accuracy: ±{:.1} dB",
                request.antenna_id, accuracy_estimate_db, loss_accuracy_estimate_db
            ));
        }
        CalibrationStatus::PartiallyCalibrated { accuracy_estimate_db, coverage } => {
            warnings.push(format!(
                "Antenna '{}' is partially calibrated. Accuracy estimate: ±{:.1} dB",
                request.antenna_id, accuracy_estimate_db
            ));

            if !is_in_coverage(&Some(coverage.clone()), &geometry) {
                warnings.push(
                    "Query is outside calibrated region - using physics model extrapolation".to_string()
                );
            }
        }
        CalibrationStatus::FullyCalibrated { .. } => {
            // No warnings for fully calibrated
        }
    }

    // Correction surface warnings
    if !correction_applied && calibration.correction_surface.is_some() {
        warnings.push("Correction surface not applied (out of coverage)".to_string());
    }

    // 7. Build response with calibration status
    Ok(GainResponse {
        antenna_id: request.antenna_id.clone(),
        feed_id: request.feed_id.clone(),
        gain_db: final_gain_db,
        reference_gain_db,
        loss_db,
        geometry: geometry.to_geometry_info(),
        warnings,
        metadata: ComputationMetadata {
            computation_time_ms: 0.0,  // Updated by handler
            coordinate_transform_ms: 0.0,
            physics_model_ms: 0.0,
            correction_surface_ms: 0.0,
            extrapolated: !correction_applied,
        },
        calibration_status: CalibrationStatusInfo::from(&calibration.calibration_status),
    })
}

/// Check if query is within calibrated coverage
fn is_in_coverage(
    coverage: &Option<CalibrationCoverage>,
    geometry: &AntennaGeometry,
) -> bool {
    match coverage {
        Some(cov) => {
            geometry.emitter_azimuth_deg >= cov.azimuth_range.0
                && geometry.emitter_azimuth_deg <= cov.azimuth_range.1
                && geometry.emitter_elevation_deg >= cov.elevation_range.0
                && geometry.emitter_elevation_deg <= cov.elevation_range.1
        }
        None => false,  // Uncalibrated - no coverage
    }
}
```

---

## 6. API Response Schema Updates

### 6.1 Extended `GainResponse`

```rust
// src/api/schemas.rs

#[derive(Serialize)]
pub struct GainResponse {
    pub antenna_id: String,
    pub feed_id: String,
    pub gain_db: f64,
    pub reference_gain_db: Option<f64>,
    pub loss_db: Option<f64>,
    pub geometry: GeometryInfo,
    pub warnings: Vec<String>,
    pub metadata: ComputationMetadata,

    // NEW: Calibration status information
    pub calibration_status: CalibrationStatusInfo,
}

#[derive(Serialize)]
pub struct CalibrationStatusInfo {
    /// Status: "fully_calibrated", "partially_calibrated", "uncalibrated"
    pub status: String,

    /// Estimated absolute gain accuracy (dB)
    pub accuracy_estimate_db: f64,

    /// Estimated loss (relative gain) accuracy (dB)
    /// Only present for uncalibrated antennas where loss is more accurate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_accuracy_estimate_db: Option<f64>,

    /// Calibration coverage (if partially calibrated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageInfo>,

    /// Whether correction surface was applied
    pub correction_applied: bool,

    /// Source of physical parameters
    pub parameters_source: String,
}

#[derive(Serialize)]
pub struct CoverageInfo {
    pub azimuth_range: [f64; 2],
    pub elevation_range: [f64; 2],
    pub frequency_range: [f64; 2],
    pub num_measurements: usize,
}

impl From<&CalibrationStatus> for CalibrationStatusInfo {
    fn from(status: &CalibrationStatus) -> Self {
        match status {
            CalibrationStatus::FullyCalibrated { accuracy_estimate_db } => {
                CalibrationStatusInfo {
                    status: "fully_calibrated".to_string(),
                    accuracy_estimate_db: *accuracy_estimate_db,
                    loss_accuracy_estimate_db: None,
                    coverage: None,
                    correction_applied: true,
                    parameters_source: "full_grid_tuning".to_string(),
                }
            }
            CalibrationStatus::PartiallyCalibrated { accuracy_estimate_db, coverage } => {
                CalibrationStatusInfo {
                    status: "partially_calibrated".to_string(),
                    accuracy_estimate_db: *accuracy_estimate_db,
                    loss_accuracy_estimate_db: None,
                    coverage: Some(CoverageInfo {
                        azimuth_range: [coverage.azimuth_range.0, coverage.azimuth_range.1],
                        elevation_range: [coverage.elevation_range.0, coverage.elevation_range.1],
                        frequency_range: [coverage.frequency_range.0, coverage.frequency_range.1],
                        num_measurements: coverage.num_measurements,
                    }),
                    correction_applied: coverage.has_correction_surface,
                    parameters_source: "partial_tuning".to_string(),
                }
            }
            CalibrationStatus::Uncalibrated { accuracy_estimate_db, loss_accuracy_estimate_db } => {
                CalibrationStatusInfo {
                    status: "uncalibrated".to_string(),
                    accuracy_estimate_db: *accuracy_estimate_db,
                    loss_accuracy_estimate_db: Some(*loss_accuracy_estimate_db),
                    coverage: None,
                    correction_applied: false,
                    parameters_source: "design_specifications".to_string(),
                }
            }
        }
    }
}
```

### 6.2 Example API Response

```json
{
  "antenna_id": "antenna_2",
  "feed_id": "x_band_feed",
  "gain_db": 41.2,
  "reference_gain_db": 43.5,
  "loss_db": 2.3,
  "geometry": {
    "feed_offset_meters": {
      "x": 0.05,
      "y": 0.02,
      "z": 0.01
    },
    "emitter_azimuth_deg": 15.5,
    "emitter_elevation_deg": 32.1
  },
  "warnings": [
    "Antenna 'antenna_2' is partially calibrated. Accuracy estimate: ±1.5 dB",
    "Query is outside calibrated region - using physics model extrapolation"
  ],
  "metadata": {
    "computation_time_ms": 2.8,
    "extrapolated": true
  },
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 1.5,
    "coverage": {
      "azimuth_range": [0.0, 0.0],
      "elevation_range": [0.0, 0.0],
      "frequency_range": [7100.0, 8500.0],
      "num_measurements": 25
    },
    "correction_applied": false,
    "parameters_source": "boresight_tuning"
  }
}
```

---

## 7. Antenna Details Endpoint Enhancement

### 7.1 Enhanced `GET /api/v1/antennas/{id}` Response

```json
{
  "antenna_id": "antenna_2",
  "name": "Ground Station 2",
  "calibration_status": "partially_calibrated",
  "accuracy_estimate_db": 1.5,
  "loss_accuracy_estimate_db": 1.2,
  "feeds": [
    {
      "feed_id": "x_band_feed",
      "name": "X-Band Primary Feed",
      "frequency_range": [7100.0, 8500.0],
      "position": [0.0, 0.0, 0.0]
    }
  ],
  "calibration_coverage": {
    "azimuth_range": [0.0, 0.0],
    "elevation_range": [0.0, 0.0],
    "frequency_range": [7100.0, 8500.0],
    "num_measurements": 25
  },
  "physical_parameters": {
    "source": "boresight_tuning",
    "diameter_m": 3.7,
    "f_over_d": 0.5,
    "surface_rms_mm": 1.52,
    "feed": {
      "q_factor": 8.3,
      "phase_center_offset_m": 0.0
    },
    "mesh": {
      "mesh_spacing_mm": 5.1,
      "wire_diameter_mm": 0.48
    }
  },
  "correction_surface": "frequency_only",
  "validity_ranges": {
    "azimuth": [0.0, 360.0],
    "elevation": [0.0, 90.0],
    "frequency": [7100.0, 8500.0]
  },
  "metadata": {
    "calibration_date": "2025-01-10",
    "parameters_tuned": true,
    "num_measurements": 25
  }
}
```

---

## 8. Calibration Upgrade Path

### 8.1 Workflow: Uncalibrated → Partially → Fully Calibrated

```bash
# Step 1: Add uncalibrated antenna to antennas.yaml with design specs
vim calibration_data/antennas.yaml

# Step 2: Service loads and provides responses with design specs
# (Accuracy: absolute ±3 dB, loss ±2 dB)

# Step 3: Collect boresight measurements
# measurements/antenna_2_boresight.csv:
# frequency_mhz,temperature_k,g_over_t_db
# 7100,290,39.5
# 7500,290,40.2
# ...

# Step 4: Calibrate with boresight measurements
cargo run --release --bin calibrate -- \
  --input measurements/antenna_2_boresight.csv \
  --output calibration_data/antenna_2_boresight.bin \
  --antenna-id antenna_2 \
  --feed-id x_band_feed \
  --calibration-mode boresight \
  --design-specs-from-config calibration_data/antennas.yaml

# Step 5: Update antennas.yaml to reference calibration file
# Change:
#   calibration_status: "uncalibrated"
# To:
#   calibration_status: "partially_calibrated"
#   calibration_file: "antenna_2_boresight.bin"

# Step 6: Service now uses tuned parameters
# (Accuracy: boresight ±1 dB, off-axis ±2 dB, loss ±1.5 dB)

# Step 7: Collect full measurement grid
# measurements/antenna_2_full.csv:
# azimuth_deg,elevation_deg,frequency_mhz,temperature_k,g_over_t_db
# 0,0,7100,290,39.5
# 5,0,7100,290,38.2
# ...

# Step 8: Full calibration
cargo run --release --bin calibrate -- \
  --input measurements/antenna_2_full.csv \
  --output calibration_data/antenna_2_full.bin \
  --antenna-id antenna_2 \
  --feed-id x_band_feed \
  --calibration-mode full

# Step 9: Update antennas.yaml
# Change:
#   calibration_status: "partially_calibrated"
#   calibration_file: "antenna_2_boresight.bin"
# To:
#   calibration_status: "fully_calibrated"
#   calibration_file: "antenna_2_full.bin"

# Step 10: Service now fully calibrated
# (Accuracy: ±1 dB everywhere)
```

### 8.2 Hot-Reload Calibration Updates

**Option A:** Restart service (simple, acceptable for MVP)
```bash
kubectl rollout restart deployment/antenna-model-service
```

**Option B:** Hot-reload endpoint (future enhancement)
```bash
curl -X POST http://service/api/v1/admin/reload
```

---

## 9. Implementation Tasks

### Phase 1: Data Model & Configuration (Sprint 6, 1-2 days)

**Task 6.x.1:** Extend data types
- Add `CalibrationStatus` enum to `types.rs`
- Add `CalibrationCoverage` struct
- Add `ParameterSource` and `MeasurementDensity` enums
- Update `AntennaCalibration` structure
- Update serialization/deserialization tests

**Task 6.x.2:** Configuration schema support
- Update `antennas.yaml` parsing to support new fields
- Add `design_specs` parsing
- Add `calibration_coverage` parsing
- Update configuration validation

### Phase 2: Uncalibrated Antenna Support (Sprint 6, 2-3 days)

**Task 6.x.3:** Repository enhancement
- Implement `load_uncalibrated_antenna()` method
- Build `PhysicalAntennaConfig` from design specs
- Add tests for uncalibrated antenna loading

**Task 6.x.4:** Service layer updates
- Update `compute_gain_from_request()` to handle all calibration statuses
- Implement `is_in_coverage()` helper
- Add calibration status warnings
- Update tests with uncalibrated scenarios

**Task 6.x.5:** API schema updates
- Add `CalibrationStatusInfo` to responses
- Update `GainResponse`, `HeatmapResponse`, `BatchGainResponse`
- Update antenna details endpoint
- Update API tests

### Phase 3: Calibration Tool - Boresight Mode (Sprint 7, 3-4 days)

**Task 7.x.1:** Parameter tuning for boresight data
- Implement `--calibration-mode boresight` flag
- Load design specs as initial parameters
- Implement parameter optimization for boresight-only data:
  - Optimize `surface_rms_mm`, `q_factor`, mesh parameters
  - Use differential evolution (existing optimizer)
  - Objective: minimize error at boresight across frequencies
- Generate `PartiallyCalibrated` status with coverage metadata

**Task 7.x.2:** Optional frequency-only correction
- Fit 1D B-spline for `correction(frequency)`
- Skip if physics model error < 0.5 dB
- Store as degenerate 4D B-spline (single spatial point)

**Task 7.x.3:** Design specs file support
- Parse YAML design specs file
- Use as initial parameter guesses
- Validate parameter bounds

### Phase 4: Calibration Tool - Limited Coverage (Sprint 7, 2-3 days)

**Task 7.x.4:** Partial grid calibration
- Implement `--calibration-mode partial` flag
- Tune parameters across all available measurements
- Compute coverage metadata (azimuth/elevation/frequency ranges)
- Generate sparse correction surface (optional)

**Task 7.x.5:** Coverage analysis
- Detect measurement density (points per beamwidth)
- Set `MeasurementDensity` in metadata
- Compute accuracy estimates based on coverage

### Phase 5: Testing & Documentation (Sprint 7-8, 2-3 days)

**Task 8.x.1:** Integration tests
- Test uncalibrated antenna queries
- Test boresight-calibrated antenna queries
- Test partially calibrated antenna queries
- Test upgrade path (uncalibrated → partial → full)
- Verify loss accuracy for uncalibrated cases

**Task 8.x.2:** Documentation
- Update architecture doc with calibration statuses
- Update API documentation with new response fields
- Create calibration workflow guide
- Document upgrade process

---

## 10. Testing Strategy

### 10.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_uncalibrated_antenna_from_design_specs() {
        // Load uncalibrated antenna from YAML config
        // Verify PhysicalAntennaConfig constructed correctly
        // Verify CalibrationStatus is Uncalibrated
    }

    #[test]
    fn test_boresight_calibration_parameter_tuning() {
        // Synthetic boresight measurements
        // Run parameter optimization
        // Verify tuned parameters within expected ranges
        // Verify boresight error < 1 dB
    }

    #[test]
    fn test_partial_coverage_detection() {
        // Sparse measurement grid
        // Verify coverage metadata computed correctly
        // Verify in_coverage() function works
    }

    #[test]
    fn test_correction_surface_skip_out_of_coverage() {
        // Query outside calibrated region
        // Verify correction surface not applied
        // Verify warning generated
    }
}
```

### 10.2 Integration Tests

```rust
#[tokio::test]
async fn test_uncalibrated_antenna_gain_query() {
    // Start service with uncalibrated antenna
    // POST /api/v1/gain with valid request
    // Verify response includes:
    //   - gain_db (physics model only)
    //   - calibration_status = "uncalibrated"
    //   - accuracy_estimate_db = 3.0
    //   - loss_accuracy_estimate_db = 2.0
    //   - warnings about using design specs
}

#[tokio::test]
async fn test_loss_accuracy_uncalibrated() {
    // Query reference gain and actual gain
    // Compute loss = reference - actual
    // Verify loss is more accurate than absolute gain
    // (systematic errors cancel)
}

#[tokio::test]
async fn test_calibration_upgrade_path() {
    // 1. Query uncalibrated antenna (design specs)
    // 2. Calibrate with boresight data
    // 3. Reload service
    // 4. Query partially calibrated antenna (tuned params)
    // 5. Calibrate with full grid
    // 6. Reload service
    // 7. Query fully calibrated antenna
    // Verify accuracy improves at each stage
}
```

### 10.3 Accuracy Validation Tests

```rust
#[test]
fn validate_boresight_calibration_accuracy() {
    // Use synthetic data with known physics model
    // Add noise to simulate measurements
    // Run boresight calibration
    // Verify:
    //   - Recovered parameters close to truth
    //   - Boresight predictions < 1 dB error
    //   - Off-axis predictions < 3 dB error (untested)
}

#[test]
fn validate_loss_computation_error_cancellation() {
    // Inject systematic bias in surface_rms_mm
    // Compute reference gain and actual gain (both biased)
    // Compute loss = reference - actual
    // Verify loss error << absolute gain error
}
```

---

## 11. Expected Outcomes

### 11.1 Accuracy by Calibration Status

| Status | Measurements | Absolute Gain Accuracy | Loss Accuracy | Coverage |
|--------|--------------|----------------------|---------------|----------|
| **Uncalibrated** | 0 | ±3-5 dB | ±2-3 dB | All |
| **Partial - Boresight** | 10-50 (freq only) | ±1 dB (boresight), ±2-3 dB (off-axis) | ±1-2 dB | All (physics extrapolation) |
| **Partial - Limited** | 100-500 | ±1-1.5 dB (in-coverage), ±2-3 dB (out) | ±1-1.5 dB | Limited region |
| **Fully Calibrated** | 1000-5000 | ±1 dB | ±1 dB | Full FOV |

### 11.2 Key Benefits

1. **Operational Flexibility:**
   - Deploy antennas immediately with design specs
   - Gradual accuracy improvement as measurements arrive
   - No service interruption during upgrades

2. **Loss Computation Emphasis:**
   - Loss accuracy better than absolute gain for uncalibrated/partial
   - Systematic errors (surface RMS, q-factor) cancel in difference
   - Meets 1-2 dB loss accuracy target even with partial calibration

3. **Resource Efficiency:**
   - Boresight-only calibration requires minimal test time (~1 hour)
   - Full calibration remains available for critical antennas
   - Physics model provides reasonable extrapolation

4. **Transparency:**
   - API responses clearly indicate calibration quality
   - Accuracy estimates guide user decision-making
   - Warnings for extrapolated regions

---

## 12. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| **Physics model inaccuracy** | Validate against measurements; tune conservatively; provide accuracy estimates |
| **Parameter optimization divergence** | Use bounded optimization; good initial guesses from design specs; multiple optimizer runs |
| **Poor extrapolation for off-axis** | Clear warnings in API; recommend boresight + sparse off-axis measurements |
| **Confusion about calibration quality** | Prominent calibration_status in responses; documentation; examples |
| **Users treating uncalibrated as accurate** | Accuracy estimates in every response; warnings; API docs emphasize limitations |

---

## 13. Future Enhancements (Post-MVP)

### 13.1 Adaptive Measurement Planning
- Analyze current calibration quality
- Recommend optimal measurement points to improve coverage
- Active learning approach to minimize test time

### 13.2 Shared Parameter Libraries
- Build library of typical parameters by antenna class
- Use as priors for new antennas
- Reduce calibration time via transfer learning

### 13.3 Real-Time Calibration Updates
- Hot-reload endpoint: `POST /api/v1/admin/reload`
- Zero-downtime calibration upgrades
- A/B testing between calibration versions

### 13.4 Uncertainty Quantification
- Parameter uncertainty from optimization covariance
- Propagate to gain predictions
- Return confidence intervals in API

### 13.5 Multi-Antenna Bayesian Calibration
- Share information across antenna fleet
- Hierarchical models for antenna classes
- Improved priors for uncalibrated antennas

---

## Appendix A: Configuration Examples

See Section 2.1 for complete YAML examples.

---

## Appendix B: Calibration Tool CLI Reference

```bash
# Uncalibrated antenna (no calibration needed - use design specs in YAML)

# Boresight-only calibration
calibrate \
  --input measurements/boresight.csv \
  --output calibration.bin \
  --antenna-id antenna_2 \
  --feed-id x_band_feed \
  --calibration-mode boresight \
  --design-specs design_specs.yaml \
  --validate

# Limited-coverage calibration
calibrate \
  --input measurements/partial_grid.csv \
  --output calibration.bin \
  --antenna-id antenna_3 \
  --feed-id primary_feed \
  --calibration-mode partial \
  --design-specs design_specs.yaml \
  --validate

# Full calibration (existing workflow)
calibrate \
  --input measurements/full_grid.csv \
  --output calibration.bin \
  --antenna-id antenna_1 \
  --feed-id x_band_feed \
  --calibration-mode full \
  --validate
```

---

**Document Version:** 1.0
**Date:** 2025-01-15
**Author:** System Architect
**Status:** Proposed
