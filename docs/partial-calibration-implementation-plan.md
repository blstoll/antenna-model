# Partial Calibration Support - Implementation Plan

**Status:** Sprint 6 complete (Phase 1 100% COMPLETE: all data model, service layer, and API endpoints support calibration statuses)
**Target:** Complete Phase 2 by end of Sprint 7
**Total Effort:** 7-9 days additional work (Phase 1: 3 days completed)

---

## Overview

This plan integrates partial/uncalibrated antenna support into the existing Sprint 6/7 roadmap. We prioritize boresight-calibrated antennas (primary use case) while maintaining backward compatibility with fully-calibrated antennas.

**Key Priorities:**
1. Uncalibrated antenna support (design specs) - **Sprint 6**
2. Service-side handling of all calibration statuses - **Sprint 6**
3. Boresight parameter tuning (calibration tool) - **Sprint 7**
4. Limited-coverage calibration - **Sprint 7** (optional)

---

## Phase 1: Data Model & Configuration (Sprint 6 Remaining)

**Target:** Complete before Sprint 6 ends
**Effort:** 2-3 days
**Dependencies:** None - can start immediately

### Task 6.4: Extend Data Types (4-6 hours) ✅ COMPLETE

**Status:** ✅ **COMPLETE** (2025-01-15)
**File:** `antenna-model/src/data/types.rs`

**Changes:**
1. Add `CalibrationStatus` enum:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
   pub enum CalibrationStatus {
       FullyCalibrated { accuracy_estimate_db: f64 },
       PartiallyCalibrated {
           accuracy_estimate_db: f64,
           coverage: CalibrationCoverage,
       },
       Uncalibrated {
           accuracy_estimate_db: f64,
           loss_accuracy_estimate_db: f64,
       },
   }
   ```

2. Add `CalibrationCoverage` struct:
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
   pub struct CalibrationCoverage {
       pub azimuth_range: (f64, f64),
       pub elevation_range: (f64, f64),
       pub frequency_range: (f64, f64),
       pub num_measurements: usize,
       pub has_correction_surface: bool,
   }
   ```

3. Add fields to `AntennaCalibration`:
   ```rust
   pub calibration_status: CalibrationStatus,
   pub calibration_coverage: Option<CalibrationCoverage>,
   ```

4. Add `ParameterSource` and `MeasurementDensity` enums to `CalibrationMetadata`

**Tests:**
- Serialization/deserialization for new types
- Backward compatibility with existing `.bin` files (Option defaults)
- Validation logic for calibration status

**Deliverables:**
- Extended `types.rs` with new enums/structs
- Unit tests for new types (10-15 tests)
- Documentation comments

**Acceptance:**
- All existing tests pass (no breaking changes)
- New types serialize/deserialize correctly
- Bincode format remains compatible

**Completion Summary:**
- ✅ Added `CalibrationStatus` enum with 3 variants (FullyCalibrated, PartiallyCalibrated, Uncalibrated)
- ✅ Added `CalibrationCoverage` struct with validation and helper methods
- ✅ Added `ParameterSource` enum with 4 variants
- ✅ Added `MeasurementDensity` enum with 4 variants
- ✅ Extended `AntennaCalibration` with optional `calibration_status` and `calibration_coverage` fields
- ✅ Extended `CalibrationMetadata` with optional `parameters_source` and `measurement_density` fields
- ✅ Created `CalibrationCoverageBuilder` for ergonomic construction
- ✅ Added 18 comprehensive unit tests covering all new types
- ✅ All 414 existing tests pass (100% backward compatibility)
- ✅ No clippy warnings
- ✅ Bincode serialization/deserialization tested and working
- ✅ JSON serialization tested for API compatibility

**Files Modified:**
- `antenna-model/src/data/types.rs` (+328 lines: types, builders, tests)

---

### Task 6.5: Configuration Parsing (4-6 hours) ✅ COMPLETE

**Status:** ✅ **COMPLETE** (2025-01-15)
**File:** `antenna-model/src/config/settings.rs`

**Changes:**
1. Extend `AntennaConfig` struct to parse `antennas.yaml`:
   ```rust
   #[derive(Debug, Clone, Deserialize)]
   pub struct AntennaConfig {
       pub id: String,
       pub name: String,
       pub calibration_status: Option<String>,  // "fully_calibrated", "partially_calibrated", "uncalibrated"
       pub calibration_file: Option<PathBuf>,
       pub calibration_coverage: Option<CalibrationCoverageConfig>,
       pub design_specs: Option<DesignSpecsConfig>,
       pub validity_ranges: Option<ValidityRangesConfig>,
       pub enabled: bool,
   }
   ```

2. Add `DesignSpecsConfig` struct:
   ```rust
   #[derive(Debug, Clone, Deserialize)]
   pub struct DesignSpecsConfig {
       pub diameter_m: f64,
       pub focal_length_m: f64,
       pub f_over_d_ratio: f64,
       pub surface_rms_mm: f64,
       pub feeds: Vec<FeedSpecConfig>,
       pub mesh: Option<MeshConfig>,
   }

   #[derive(Debug, Clone, Deserialize)]
   pub struct FeedSpecConfig {
       pub id: String,
       pub name: String,
       pub position: [f64; 3],
       pub q_factor: f64,
       pub phase_center_offset_m: f64,
       pub frequency_range: [f64; 2],
   }
   ```

3. Update config loading to validate design_specs for uncalibrated antennas

**Tests:**
- Parse test `antennas.yaml` with all calibration statuses
- Validation: uncalibrated requires design_specs
- Validation: calibrated requires calibration_file
- Error handling for malformed configs

**Deliverables:**
- Extended config structures
- Config validation logic
- Unit tests (8-12 tests)

**Acceptance:**
- Successfully parses `calibration_data/antennas.yaml`
- Clear error messages for invalid configurations
- All existing config tests pass

**Completion Summary:**
- ✅ Added `DesignSpecsConfig` struct with reflector geometry, feeds, and optional mesh
- ✅ Added `FeedSpecConfig` struct with position, q-factor, phase center offset, and frequency range
- ✅ Added `MeshConfig` struct for mesh reflector parameters
- ✅ Added `CalibrationCoverageConfig` struct for partial calibration metadata
- ✅ Added `ValidityRangesConfig` struct for antenna validity ranges
- ✅ Extended `AntennaConfigEntry` with optional fields:
  - `calibration_status: Option<String>` (defaults to "fully_calibrated")
  - `calibration_file: Option<String>` (now optional, was required)
  - `calibration_coverage: Option<CalibrationCoverageConfig>`
  - `design_specs: Option<DesignSpecsConfig>`
  - `validity_ranges: Option<ValidityRangesConfig>`
  - `description: Option<String>` and `location: Option<String>`
- ✅ Implemented comprehensive validation logic:
  - `AntennaConfigEntry::validate()` - validates based on calibration status
  - `DesignSpecsConfig::validate()` - validates reflector geometry and feeds
  - `FeedSpecConfig::validate()` - validates feed parameters
  - `MeshConfig::validate()` - validates mesh parameters
  - `CalibrationCoverageConfig::validate()` - validates coverage ranges
  - `ValidityRangesConfig::validate()` - validates validity ranges
- ✅ Added `get_calibration_status()` helper method for backward compatibility
- ✅ Added 14 comprehensive unit tests:
  - Uncalibrated antenna parsing (simple, with mesh, multi-feed)
  - Partially calibrated antenna with coverage
  - Antenna with validity ranges
  - Validation error cases (missing design_specs, missing calibration_file, invalid status)
  - Design specs validation (invalid diameter, no feeds)
  - Feed validation (invalid q-factor)
  - Mesh validation (wire too large)
  - Backward compatibility test
  - Real antennas.yaml parsing test
- ✅ Updated repository to skip uncalibrated antennas (to be loaded in Task 6.6)
- ✅ All 428 existing tests pass (100% backward compatibility)
- ✅ No clippy warnings
- ✅ Successfully parses real `calibration_data/antennas.yaml` with uncalibrated antennas

**Files Modified:**
- `antenna-model/src/config/settings.rs` (+704 lines: 5 new structs, validation methods, 14 tests)
- `antenna-model/src/data/repository.rs` (+8 lines: skip uncalibrated antennas)

---

### Task 6.6: Repository - Uncalibrated Antenna Loading (6-8 hours) ✅ COMPLETE

**Status:** ✅ **COMPLETE** (2025-01-15)
**File:** `antenna-model/src/data/repository.rs`

**Changes:**
1. Update `load_antenna()` method to handle all statuses:
   ```rust
   pub fn load_antenna(&mut self, config: &AntennaConfig) -> Result<(), DataError> {
       match (config.calibration_status.as_deref(), &config.calibration_file) {
           (Some("uncalibrated"), None) | (None, None) => {
               self.load_uncalibrated_antenna(config)?;
           }
           (_, Some(cal_file)) => {
               let calibration = loader::load_calibration_artifact(cal_file)?;
               self.add_calibration(calibration)?;
           }
           _ => {
               return Err(DataError::ConfigurationError(
                   format!("Invalid configuration for antenna {}", config.id)
               ));
           }
       }
       Ok(())
   }
   ```

2. Implement `load_uncalibrated_antenna()`:
   ```rust
   fn load_uncalibrated_antenna(&mut self, config: &AntennaConfig) -> Result<(), DataError> {
       let design = config.design_specs.as_ref()
           .ok_or_else(|| DataError::ConfigurationError(
               format!("Uncalibrated antenna {} requires design_specs", config.id)
           ))?;

       // For each feed in design specs
       for feed_spec in &design.feeds {
           // Build PhysicalAntennaConfig from design specs
           let physical_config = PhysicalAntennaConfig {
               reflector: ReflectorGeometry {
                   diameter_m: design.diameter_m,
                   focal_length_m: design.focal_length_m,
                   f_over_d_ratio: design.f_over_d_ratio,
                   surface_rms_mm: design.surface_rms_mm,
               },
               feed: FeedParameters {
                   position: (feed_spec.position[0], feed_spec.position[1], feed_spec.position[2]),
                   q_factor: feed_spec.q_factor,
                   phase_center_offset_m: feed_spec.phase_center_offset_m,
               },
               mesh: design.mesh.as_ref().map(|m| MeshParameters {
                   mesh_spacing_mm: m.mesh_spacing_mm,
                   wire_diameter_mm: m.wire_diameter_mm,
               }),
           };

           // Build AntennaCalibration with Uncalibrated status
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
               validity_ranges: build_validity_ranges(config, feed_spec),
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
   ```

**Tests:**
- Load uncalibrated antenna from config
- Load partially calibrated antenna (with .bin file)
- Load fully calibrated antenna (backward compatibility)
- Multi-feed uncalibrated antenna
- Error handling: missing design_specs, invalid parameters

**Deliverables:**
- Updated repository with uncalibrated antenna support
- Unit tests (12-15 tests)
- Integration test with real `antennas.yaml`

**Acceptance:**
- Repository successfully loads all antennas from `calibration_data/antennas.yaml`
- Uncalibrated antennas have valid `PhysicalAntennaConfig`
- All feeds loaded correctly for multi-feed antennas
- Existing calibrated antenna loading unchanged

**Completion Summary:**
- ✅ Implemented `load_antenna()` method that dispatches based on presence of `calibration_file`
- ✅ Implemented `load_uncalibrated_antenna()` method for constructing calibrations from design specs
- ✅ Implemented `build_validity_ranges()` helper function for validity range construction
- ✅ Added 12 comprehensive unit tests covering:
  - Single-feed uncalibrated antenna loading
  - Multi-feed uncalibrated antenna loading (3 feeds)
  - Uncalibrated antenna with mesh parameters
  - Uncalibrated antenna with custom validity ranges
  - Error handling: missing design specs
  - Mixed calibrated and uncalibrated antenna loading
  - Invalid design specs (fail-fast behavior)
  - List operations on uncalibrated antennas
  - Backward compatibility requirements
- ✅ All 437 tests pass (100% backward compatibility)
- ✅ No clippy warnings
- ✅ Repository successfully loads uncalibrated antennas from design specs
- ✅ Multi-feed support working correctly
- ✅ Validity ranges built correctly from config or defaults

**Files Modified:**
- `antenna-model/src/data/repository.rs` (+169 lines: new methods, 12 tests)
- `antenna-model/src/config/mod.rs` (+1 line: export FeedSpecConfig)

---

### Task 6.7: API Schema Updates (4-5 hours) ✅ COMPLETE

**Status:** ✅ **COMPLETE** (2025-01-15)
**File:** `antenna-model/src/api/schemas.rs`

**Changes:**
1. Add `CalibrationStatusInfo` to responses:
   ```rust
   #[derive(Serialize)]
   pub struct CalibrationStatusInfo {
       pub status: String,  // "fully_calibrated", "partially_calibrated", "uncalibrated"
       pub accuracy_estimate_db: f64,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub loss_accuracy_estimate_db: Option<f64>,
       #[serde(skip_serializing_if = "Option::is_none")]
       pub coverage: Option<CoverageInfo>,
       pub correction_applied: bool,
       pub parameters_source: String,
   }

   impl From<&CalibrationStatus> for CalibrationStatusInfo { ... }
   ```

2. Add `calibration_status` field to:
   - `GainResponse`
   - `BatchGainResponse` (per-result)
   - `HeatmapResponse`
   - `AntennaDetailsResponse`

3. Add `CoverageInfo` struct for API responses

**Tests:**
- Serialization of CalibrationStatusInfo for each status type
- Response schema validation
- Backward compatibility (existing clients should still work)

**Deliverables:**
- Updated schemas with calibration status
- Unit tests (8-10 tests)
- Updated `examples/api_requests.json` with expected responses

**Acceptance:**
- All responses include `calibration_status` field
- JSON serialization matches API spec
- Existing tests pass with schema extensions

**Completion Summary:**
- ✅ Added `CalibrationStatusInfo` struct with all required fields (status, accuracy_estimate_db, loss_accuracy_estimate_db, coverage, correction_applied, parameters_source)
- ✅ Implemented `From<&CalibrationStatus>` trait for CalibrationStatusInfo conversion
- ✅ Added `CoverageInfo` struct with azimuth_range_deg, elevation_range_deg, frequency_range_mhz, num_measurements, is_boresight_only
- ✅ Implemented `From<&CalibrationCoverage>` trait for CoverageInfo conversion
- ✅ Extended `GainResponse` with optional `calibration_status: Option<CalibrationStatusInfo>` field
- ✅ Extended `HeatmapResponse` with optional `calibration_status: Option<CalibrationStatusInfo>` field
- ✅ Extended `AntennaDetailsResponse` with optional `calibration_status: Option<CalibrationStatusInfo>` field
- ✅ BatchGainResponse automatically includes calibration_status per-result via GainResponse
- ✅ Added 11 comprehensive unit tests:
  - CalibrationStatusInfo From trait for FullyCalibrated
  - CalibrationStatusInfo From trait for PartiallyCalibrated
  - CalibrationStatusInfo From trait for Uncalibrated
  - CalibrationStatusInfo serialization for FullyCalibrated (with skip_serializing validation)
  - CalibrationStatusInfo serialization for PartiallyCalibrated
  - CalibrationStatusInfo serialization for Uncalibrated
  - CoverageInfo From trait for CalibrationCoverage
  - CoverageInfo boresight-only detection (boresight vs sparse grid)
  - CoverageInfo serialization
  - GainResponse with calibration_status
  - GainResponse backward compatibility (without calibration_status)
- ✅ All 448 existing tests pass (100% backward compatibility)
- ✅ No clippy warnings
- ✅ Made calibration_status fields optional (Option<CalibrationStatusInfo>) with skip_serializing_if for backward compatibility
- ✅ Updated service layer (evaluator, batch, heatmap) and API handlers to populate calibration_status: None (placeholder for Task 6.8)

**Files Modified:**
- `antenna-model/src/api/schemas.rs` (+250 lines: 2 new structs with From traits, 4 response updates, 11 tests)
- `antenna-model/src/service/evaluator.rs` (+1 line: placeholder for Task 6.8)
- `antenna-model/src/service/batch.rs` (+1 line: placeholder for Task 6.8)
- `antenna-model/src/service/heatmap.rs` (+1 line: placeholder for Task 6.8)
- `antenna-model/src/api/handlers.rs` (+1 line: placeholder for Task 6.8)

**Notes:**
- Fields are optional (Option<>) to maintain backward compatibility with existing code
- Service layer will populate calibration_status in Task 6.8
- JSON serialization properly skips None values for cleaner API responses

---

### Task 6.8: Service Layer - Handle All Calibration Statuses (6-8 hours)

**Files:**
- `antenna-model/src/service/evaluator.rs`
- `antenna-model/src/service/batch.rs`
- `antenna-model/src/service/heatmap.rs`

**Changes:**

1. **Evaluator - Core Gain Computation:**
   ```rust
   pub fn compute_gain_from_request(
       request: &GainRequest,
       repository: &CalibrationRepository,
   ) -> Result<GainResponse, ServiceError> {
       // Load calibration (all statuses supported)
       let calibration = repository.get_calibration(&request.antenna_id, &request.feed_id)?;

       // Transform coordinates and compute geometry
       let geometry = compute_geometry_from_request(request)?;

       // ALWAYS compute physics model (works for any calibration status)
       let gain_physics = compute_physics_model(
           &calibration.physical_config,
           &geometry,
           request.frequency_mhz,
       )?;

       // Apply correction surface (if available and in coverage)
       let (correction_db, correction_applied) = match &calibration.correction_surface {
           Some(correction) if is_in_coverage(&calibration.calibration_coverage, &geometry) => {
               let corr = evaluate_correction_surface(correction, &geometry, request.frequency_mhz)?;
               (corr, true)
           }
           _ => (0.0, false),
       };

       let final_gain_db = gain_physics + correction_db;

       // Compute reference gain if requested
       let (reference_gain_db, loss_db) = if request.include_reference {
           let ref_geometry = compute_reference_geometry(request)?;
           let ref_gain_physics = compute_physics_model(
               &calibration.physical_config,
               &ref_geometry,
               request.frequency_mhz,
           )?;
           // Note: reference doesn't use correction surface (ideal case)
           let loss = ref_gain_physics - final_gain_db;
           (Some(ref_gain_physics), Some(loss))
       } else {
           (None, None)
       };

       // Generate warnings based on calibration status
       let warnings = generate_calibration_warnings(&calibration, &geometry, correction_applied);

       Ok(GainResponse {
           antenna_id: request.antenna_id.clone(),
           feed_id: request.feed_id.clone(),
           gain_db: final_gain_db,
           reference_gain_db,
           loss_db,
           geometry: geometry.to_geometry_info(),
           warnings,
           metadata: ComputationMetadata { ... },
           calibration_status: CalibrationStatusInfo::from(&calibration.calibration_status),
       })
   }

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
           None => false,
       }
   }

   fn generate_calibration_warnings(
       calibration: &AntennaCalibration,
       geometry: &AntennaGeometry,
       correction_applied: bool,
   ) -> Vec<String> {
       let mut warnings = Vec::new();

       match &calibration.calibration_status {
           CalibrationStatus::Uncalibrated { accuracy_estimate_db, loss_accuracy_estimate_db } => {
               warnings.push(format!(
                   "Antenna '{}' is uncalibrated (using design specifications). \
                    Absolute gain accuracy: ±{:.1} dB, Loss accuracy: ±{:.1} dB",
                   calibration.antenna_id, accuracy_estimate_db, loss_accuracy_estimate_db
               ));
           }
           CalibrationStatus::PartiallyCalibrated { accuracy_estimate_db, coverage } => {
               warnings.push(format!(
                   "Antenna '{}' is partially calibrated. Accuracy estimate: ±{:.1} dB",
                   calibration.antenna_id, accuracy_estimate_db
               ));

               if !is_in_coverage(&Some(coverage.clone()), geometry) {
                   warnings.push(
                       "Query is outside calibrated region - using physics model extrapolation".to_string()
                   );
               }
           }
           CalibrationStatus::FullyCalibrated { .. } => {
               // No calibration warnings for fully calibrated
           }
       }

       if !correction_applied && calibration.correction_surface.is_some() {
           warnings.push("Correction surface not applied (out of coverage)".to_string());
       }

       warnings
   }
   ```

2. **Batch and Heatmap:** Update to use new evaluator (no changes needed if they call `compute_gain_from_request`)

**Tests:**
- Uncalibrated antenna gain query (physics only)
- Partially calibrated in-coverage query (physics + correction)
- Partially calibrated out-of-coverage query (physics only)
- Fully calibrated query (backward compatibility)
- Reference gain and loss computation for uncalibrated
- Warning generation for each status type

**Deliverables:**
- Updated evaluator with calibration status handling
- Helper functions (`is_in_coverage`, `generate_calibration_warnings`)
- Unit tests (15-20 tests)
- Integration tests with uncalibrated antennas

**Acceptance:**
- All calibration statuses work end-to-end
- Warnings generated appropriately
- Loss computation works for uncalibrated antennas
- Existing fully-calibrated workflow unchanged
- All 500+ tests still pass

**Completion Summary:**
- ✅ Updated `compute_gain_from_request()` to handle all calibration statuses
  - Physics model is always computed (works for any calibration status)
  - Correction surface is conditionally applied based on coverage check
  - Calibration warnings are generated based on status
  - `calibration_status` field populated properly in response
- ✅ Implemented `is_in_coverage()` helper function
  - Checks if query point (azimuth, elevation, frequency) is within calibrated coverage
  - Returns false for uncalibrated antennas (no coverage metadata)
  - Returns true for queries within PartiallyCalibrated coverage ranges
- ✅ Implemented `generate_calibration_warnings()` helper function
  - Generates uncalibrated warning with accuracy estimates
  - Generates partially calibrated warning with coverage info
  - Warns when query is outside calibrated spatial region
  - Warns when correction surface exists but wasn't applied
  - No warnings for fully calibrated antennas
- ✅ Updated `heatmap.rs` to populate `calibration_status` field
  - Retrieves calibration from repository
  - Sets `correction_applied` based on presence of correction surface
- ✅ Batch service already correctly handles calibration_status (via evaluator)
- ✅ Added 17 comprehensive unit tests:
  - 5 tests for `is_in_coverage()` (full coverage, outside azimuth/elevation/frequency, none)
  - 5 tests for `generate_calibration_warnings()` (uncalibrated, partially calibrated in/out of coverage, fully calibrated, correction not applied)
  - 7 end-to-end tests (uncalibrated antenna, uncalibrated with reference, fully calibrated, partially calibrated, antenna not found, backward compatibility)
- ✅ All 464 existing tests pass (100% backward compatibility)
- ✅ No clippy warnings
- ✅ Service now handles all calibration statuses end-to-end

**Files Modified:**
- `antenna-model/src/service/evaluator.rs` (+437 lines: updated compute_gain_from_request, 2 helper functions, 17 tests)
- `antenna-model/src/service/heatmap.rs` (+13 lines: populate calibration_status)
- `antenna-model/src/service/batch.rs` (no changes needed - uses evaluator)

**Notes:**
- Backward compatibility maintained: calibrations without `calibration_status` field are handled correctly (treated as fully calibrated)
- Correction surface application is now coverage-aware
- Warning system provides clear user feedback about calibration quality and extrapolation

---

### Task 6.9: Antenna Details Endpoint Enhancement (2-3 hours)

**File:** `antenna-model/src/api/handlers.rs`

**Changes:**
1. Update `get_antenna_details` handler to include calibration status
2. Update `AntennaDetailsResponse` with calibration info
3. Show design_specs source for uncalibrated antennas

**Tests:**
- GET antenna details for uncalibrated antenna
- GET antenna details for partially calibrated antenna
- Verify calibration_status, coverage, and parameters_source

**Deliverables:**
- Enhanced antenna details endpoint
- Unit tests (5-7 tests)
- API example responses

**Acceptance:**
- Antenna details show calibration status
- Coverage information displayed for partial calibration
- Parameters source indicated clearly

**Completion Summary:**
- ✅ Updated `get_antenna_details` handler to populate `calibration_status` field
  - Retrieves calibration status from calibration object
  - Converts to `CalibrationStatusInfo` using From trait
  - Sets `correction_applied` based on presence of correction surface
- ✅ Added 4 comprehensive integration tests to `routes.rs`:
  - Test antenna details with uncalibrated status (validates accuracy estimates, parameters_source)
  - Test antenna details with partially calibrated status (validates coverage info, is_boresight_only flag)
  - Test antenna details with fully calibrated status (validates status and accuracy)
  - Test backward compatibility without calibration_status (ensures old format still works)
- ✅ All 468 tests pass (100% backward compatibility maintained)
- ✅ No clippy warnings
- ✅ Antenna details endpoint now shows calibration status for all antennas

**Files Modified:**
- `antenna-model/src/api/handlers.rs` (+7 lines: import CalibrationStatusInfo, populate calibration_status)
- `antenna-model/src/api/routes.rs` (+365 lines: 4 comprehensive integration tests)

**Notes:**
- For backward compatibility, antennas without `calibration_status` field return `None` (which is skipped in JSON serialization)
- The `correction_applied` flag indicates whether the antenna has a correction surface available
- Tests verify all three calibration status types (Uncalibrated, PartiallyCalibrated, FullyCalibrated)
- Coverage information is properly serialized for partially calibrated antennas

---

### Phase 1 Deliverables Summary

**By End of Sprint 6:**
- ✅ Data types support all calibration statuses
- ✅ Configuration parsing for `antennas.yaml`
- ✅ Repository loads uncalibrated antennas from design specs
- ✅ API schemas include calibration status and accuracy estimates
- ✅ Service layer handles all statuses (physics + optional correction) - **Complete (Task 6.8)**
- ✅ All endpoints work with uncalibrated antennas - **Complete (Task 6.8)**
- ✅ Antenna details endpoint enhancement - **Complete (Task 6.9)**
- ✅ Comprehensive test coverage (81+ new tests completed)

**Status:** Phase 1 COMPLETE! Service fully supports uncalibrated antennas for loss analysis with all endpoints!

---

## Phase 2: Calibration Tool - Boresight Mode (Sprint 7)

**Target:** Complete in first half of Sprint 7
**Effort:** 3-4 days
**Dependencies:** Phase 1 complete

### Task 7.1: Boresight Calibration Mode (8-10 hours)

**File:** `calibrate/src/main.rs` and new modules

**Changes:**
1. Add `--calibration-mode` CLI flag:
   ```rust
   #[derive(Parser)]
   pub struct Cli {
       // ... existing fields ...

       #[arg(long, default_value = "full")]
       pub calibration_mode: CalibrationMode,

       #[arg(long)]
       pub design_specs: Option<PathBuf>,
   }

   #[derive(Debug, Clone, ValueEnum)]
   pub enum CalibrationMode {
       Full,       // Existing workflow
       Boresight,  // New: boresight-only calibration
       Partial,    // New: limited coverage calibration
   }
   ```

2. Implement boresight calibration workflow:
   ```rust
   // calibrate/src/boresight_calibration.rs

   pub fn calibrate_boresight(
       measurements: &[BoresightMeasurement],
       design_specs: &DesignSpecs,
       antenna_id: &str,
       feed_id: &str,
   ) -> Result<AntennaCalibration, CalibrationError> {
       // 1. Load design specs as initial parameter guesses
       let initial_params = PhysicalAntennaConfig::from_design_specs(design_specs);

       // 2. Define parameter tuning problem
       let tuning_problem = BoresightTuningProblem {
           measurements,
           initial_params: &initial_params,
           bounds: &design_specs.tuning_bounds,
       };

       // 3. Run differential evolution optimizer (reuse existing)
       let tuned_params = optimize_parameters(&tuning_problem)?;

       // 4. Compute residuals with tuned parameters
       let residuals = compute_residuals(measurements, &tuned_params);

       // 5. Optional: fit frequency-only correction surface
       let correction_surface = if should_fit_correction(&residuals) {
           Some(fit_frequency_correction(measurements, &residuals)?)
       } else {
           None
       };

       // 6. Build calibration with PartiallyCalibrated status
       Ok(AntennaCalibration {
           antenna_id: antenna_id.to_string(),
           feed_id: feed_id.to_string(),
           metadata: CalibrationMetadata {
               // ... metadata fields ...
               parameters_tuned: true,
               num_measurements: measurements.len(),
               // ...
           },
           physical_config: tuned_params,
           correction_surface,
           validity_ranges: ValidityRanges {
               azimuth_min_max: (0.0, 360.0),  // Assume valid everywhere
               elevation_min_max: (0.0, 90.0),
               frequency_min_max: extract_frequency_range(measurements),
               temperature_const: 290.0,
           },
           calibration_status: CalibrationStatus::PartiallyCalibrated {
               accuracy_estimate_db: estimate_accuracy(&residuals),
               coverage: CalibrationCoverage {
                   azimuth_range: (0.0, 0.0),
                   elevation_range: (0.0, 0.0),
                   frequency_range: extract_frequency_range(measurements),
                   num_measurements: measurements.len(),
                   has_correction_surface: correction_surface.is_some(),
               },
           },
           calibration_coverage: Some(...),
       })
   }

   struct BoresightTuningProblem<'a> {
       measurements: &'a [BoresightMeasurement],
       initial_params: &'a PhysicalAntennaConfig,
       bounds: &'a TuningBounds,
   }

   impl OptimizationProblem for BoresightTuningProblem<'_> {
       fn evaluate(&self, params: &[f64]) -> f64 {
           // params = [surface_rms_mm, q_factor, mesh_spacing_mm, wire_diameter_mm]

           // Build physical config from parameters
           let physical_config = self.params_to_config(params);

           // Compute physics model predictions at boresight for all frequencies
           let mut error_sum = 0.0;
           for meas in self.measurements {
               let predicted_gain = compute_boresight_gain(&physical_config, meas.frequency_mhz);
               let error = (predicted_gain - meas.g_over_t_db).powi(2);
               error_sum += error;
           }

           // Return RMSE
           (error_sum / self.measurements.len() as f64).sqrt()
       }

       fn bounds(&self) -> Vec<(f64, f64)> {
           vec![
               (self.bounds.surface_rms_mm.0, self.bounds.surface_rms_mm.1),
               (self.bounds.q_factor.0, self.bounds.q_factor.1),
               (self.bounds.mesh_spacing_mm.0, self.bounds.mesh_spacing_mm.1),
               (self.bounds.wire_diameter_mm.0, self.bounds.wire_diameter_mm.1),
           ]
       }
   }
   ```

**Tests:**
- Parse boresight CSV (frequency, temperature, g_over_t)
- Load design specs from YAML
- Parameter optimization converges
- Tuned parameters within bounds
- Boresight predictions < 1 dB error
- Calibration artifact generation

**Deliverables:**
- `calibrate/src/boresight_calibration.rs` module
- `calibrate/src/design_specs_loader.rs` module
- Updated CLI with new flags
- Unit tests (15-20 tests)
- Example boresight measurement CSV
- Documentation

**Acceptance:**
- `--calibration-mode boresight` works end-to-end
- Tuned parameters improve fit over design specs
- Generated `.bin` file loads in service
- Boresight predictions accurate

---

### Task 7.2: Design Specs Loader (4-5 hours)

**File:** `calibrate/src/design_specs_loader.rs`

**Changes:**
1. Parse design specs YAML files (e.g., `small_groundstation.yaml`)
2. Extract from `antennas.yaml` if `--design-specs-from-config` flag used
3. Validate design specs (physically reasonable parameters)

**Tests:**
- Load design specs from standalone YAML
- Extract design specs from `antennas.yaml`
- Validation errors for invalid specs
- Round-trip serialization

**Deliverables:**
- Design specs loader module
- Unit tests (8-10 tests)
- Example design specs files (already created)

**Acceptance:**
- Design specs successfully loaded from both sources
- Clear error messages for malformed files

---

### Task 7.3: Optional Frequency Correction Surface (4-5 hours)

**File:** `calibrate/src/frequency_correction.rs`

**Changes:**
1. Fit 1D B-spline to frequency-only residuals:
   ```rust
   pub fn fit_frequency_correction(
       measurements: &[BoresightMeasurement],
       residuals: &[f64],
   ) -> Result<BSplineModel4D, CalibrationError> {
       // Extract unique frequencies
       let frequencies: Vec<f64> = measurements.iter()
           .map(|m| m.frequency_mhz)
           .collect();

       // Fit 1D cubic B-spline: correction(freq)
       let spline_1d = fit_cubic_spline(&frequencies, residuals)?;

       // Convert to degenerate 4D B-spline (single spatial point)
       let bspline_4d = BSplineModel4D {
           coefficients: spline_1d.coefficients.clone(),
           shape: [1, 1, frequencies.len(), 1],  // Degenerate in az, el, temp
           knots_azimuth: vec![0.0, 0.0],
           knots_elevation: vec![0.0, 0.0],
           knots_frequency: spline_1d.knots.clone(),
           knots_temperature: vec![290.0, 290.0],
           spline_order: 3,
       };

       Ok(bspline_4d)
   }

   pub fn should_fit_correction(residuals: &[f64]) -> bool {
       // Only fit if residuals show systematic bias > 0.5 dB
       let rmse = (residuals.iter().map(|r| r.powi(2)).sum::<f64>() / residuals.len() as f64).sqrt();
       rmse > 0.5
   }
   ```

**Tests:**
- Fit correction to synthetic residuals
- Skip correction if residuals small
- Degenerate 4D B-spline format
- Correction surface evaluation

**Deliverables:**
- Frequency correction module
- Unit tests (8-10 tests)

**Acceptance:**
- Correction fitted when appropriate
- Service can evaluate degenerate 4D correction

---

### Phase 2 Deliverables Summary

**By End of Sprint 7 (First Half):**
- ✅ Boresight calibration mode in `calibrate` tool
- ✅ Parameter tuning from boresight measurements
- ✅ Design specs loading
- ✅ Optional frequency-only correction surface
- ✅ Generated `.bin` artifacts work in service
- ✅ Comprehensive test coverage

**Status:** Engineers can upgrade uncalibrated antennas to boresight-calibrated!

---

## Phase 3: Limited Coverage Calibration (Sprint 7 - Optional)

**Target:** Complete in second half of Sprint 7 if time permits
**Effort:** 2-3 days
**Priority:** Lower (boresight is primary use case)

### Task 7.4: Partial Grid Calibration (6-8 hours)

**File:** `calibrate/src/partial_grid_calibration.rs`

**Changes:**
1. Extend boresight calibration to sparse grids
2. Detect measurement coverage (azimuth/elevation/frequency ranges)
3. Fit sparse 3D correction surface (optional)
4. Generate coverage metadata

**Tests:**
- Parse partial grid CSV
- Coverage detection
- Parameter tuning with partial grid
- Sparse correction surface

**Deliverables:**
- Partial grid calibration module
- Unit tests (12-15 tests)
- Example partial grid CSV

**Acceptance:**
- `--calibration-mode partial` works end-to-end
- Coverage metadata accurate
- In-coverage accuracy improved

---

## Phase 4: Testing & Documentation (Sprint 7)

**Target:** Complete by end of Sprint 7
**Effort:** 2-3 days
**Priority:** High

### Task 7.5: Integration Tests (6-8 hours)

**Files:** `antenna-model/tests/integration/`

**Tests:**
1. **Uncalibrated antenna workflow:**
   - Start service with uncalibrated antenna
   - Query gain, loss, heatmap
   - Verify responses include calibration status
   - Verify loss accuracy better than absolute gain (error cancellation)

2. **Boresight calibration workflow:**
   - Generate boresight measurements (synthetic data)
   - Run `calibrate --calibration-mode boresight`
   - Update `antennas.yaml`
   - Restart service
   - Query antenna - verify improved accuracy at boresight

3. **Calibration upgrade path:**
   - Start with uncalibrated
   - Upgrade to boresight-calibrated
   - Upgrade to fully-calibrated
   - Verify accuracy improves at each stage

4. **Multi-feed scenarios:**
   - Uncalibrated antenna with multiple feeds
   - Verify each feed has independent parameters
   - Query different feeds

**Deliverables:**
- Integration test suite (10-15 tests)
- Synthetic measurement data generators
- Test fixtures

**Acceptance:**
- All integration tests pass
- End-to-end workflows validated
- Multi-feed support confirmed

---

### Task 7.6: Documentation (4-6 hours)

**Files:**
- Update `docs/architecture.md`
- Update `docs/implementation-plan.md`
- Create `docs/calibration-workflow-guide.md`
- Update API documentation
- Update `README.md`

**Content:**
1. Calibration status types and use cases
2. Upgrade workflow (uncalibrated → partial → full)
3. API response schema changes
4. Calibration tool usage examples
5. Design specs file format
6. Accuracy expectations by calibration status

**Deliverables:**
- Updated architecture documentation
- Calibration workflow guide
- API documentation
- User-facing README updates

**Acceptance:**
- Documentation covers all calibration statuses
- Examples provided for each workflow
- Clear accuracy expectations documented

---

## Testing Strategy Summary

### Unit Tests (Sprint 6 & 7)
- Data types: 25 tests
- Configuration parsing: 12 tests
- Repository: 18 tests
- Service layer: 25 tests
- Calibration tool: 40 tests
- **Total: ~120 new unit tests**

### Integration Tests (Sprint 7)
- Uncalibrated antenna: 5 tests
- Boresight calibration: 5 tests
- Upgrade workflow: 5 tests
- Multi-feed: 3 tests
- **Total: ~18 new integration tests**

### Manual Testing
- Load real `antennas.yaml` with uncalibrated antennas
- Query each calibration status type
- Run boresight calibration on synthetic data
- Verify API responses match specification

---

## Risk Management

### High Risks

| Risk | Mitigation |
|------|------------|
| **Parameter optimization doesn't converge** | Use bounded optimization; good initial guesses from design specs; multiple optimizer runs |
| **Physics model inaccuracy for uncalibrated** | Provide clear accuracy estimates; emphasize loss computation advantage |
| **Backward compatibility breaking** | Extensive testing with existing workflows; Option<> for new fields |
| **Configuration parsing complexity** | Clear validation; helpful error messages; examples |

### Medium Risks

| Risk | Mitigation |
|------|------------|
| **Time overrun in Sprint 6/7** | Phase 3 (limited coverage) is optional; can defer to Sprint 8 |
| **Frequency-only correction ineffective** | Make correction optional (low priority); skip if residuals < 0.5 dB |
| **Design specs unavailable** | Request from antenna vendor; estimate conservatively; document assumptions |

---

## Dependencies & Prerequisites

### Before Starting Phase 1 (Sprint 6):
- [x] Sprint 6 tasks 6.1-6.3 complete (batch, heatmap, antenna endpoints) - DONE
- [x] Design specs YAML files created - DONE
- [x] `antennas.yaml` populated with test cases - DONE

### Before Starting Phase 2 (Sprint 7):
- [ ] Phase 1 complete and tested
- [ ] Service successfully loads uncalibrated antennas
- [ ] All Sprint 6 tests passing

### Before Integration Testing (Sprint 7):
- [ ] Phase 1 and Phase 2 complete
- [ ] Boresight calibration tool functional
- [ ] Synthetic measurement data generators ready

---

## Acceptance Criteria

### Phase 1 Complete When:
- ✅ Service loads all antennas from `calibration_data/antennas.yaml`
- ✅ Uncalibrated antennas work for gain/loss/heatmap queries
- ✅ API responses include calibration status
- ✅ All 500+ existing tests pass
- ✅ 60-80 new tests pass

### Phase 2 Complete When:
- ✅ `calibrate --calibration-mode boresight` generates valid `.bin` files
- ✅ Service loads boresight-calibrated antennas
- ✅ Boresight accuracy < 1 dB after tuning
- ✅ Design specs loading works from YAML
- ✅ 40+ calibration tool tests pass

### Final Acceptance (End of Sprint 7):
- ✅ All phases complete
- ✅ Integration tests pass
- ✅ Documentation complete
- ✅ Can demonstrate full upgrade workflow: uncalibrated → boresight → full
- ✅ Loss accuracy ±1-2 dB for boresight-calibrated antennas
- ✅ All 650+ tests pass

---

## Timeline Summary

| Phase | Sprint | Duration | Status |
|-------|--------|----------|--------|
| **Phase 1:** Data Model & Service Support | Sprint 6 | 2-3 days | ⏳ Ready to start |
| **Phase 2:** Boresight Calibration Tool | Sprint 7 | 3-4 days | 📋 Pending |
| **Phase 3:** Limited Coverage (Optional) | Sprint 7 | 2-3 days | 📋 Optional |
| **Phase 4:** Testing & Documentation | Sprint 7 | 2-3 days | 📋 Pending |
| **Total Effort:** | | **9-13 days** | |

**Target Completion:** End of Sprint 7 (2 weeks from now)

---

## Next Immediate Steps

### This Week (Sprint 6 Remaining):

1. **Start Task 6.4** - Extend data types (`CalibrationStatus`, `CalibrationCoverage`) - 4-6 hours
2. **Complete Task 6.5** - Configuration parsing for `antennas.yaml` - 4-6 hours
3. **Complete Task 6.6** - Repository uncalibrated antenna loading - 6-8 hours

**Goal:** Service can load and query uncalibrated antennas by end of Sprint 6

### Next Week (Sprint 7 Start):

4. **Complete Task 6.7-6.9** - Service layer and API updates - 12-16 hours
5. **Begin Task 7.1** - Boresight calibration tool - 8-10 hours

**Goal:** Full end-to-end workflow working by mid-Sprint 7

---

## Questions for Review

Before starting implementation:

1. **Priority Confirmation:** Is boresight-calibrated (Phase 2) still the highest priority use case?
2. **Timeline:** Is end of Sprint 7 acceptable, or should we target earlier?
3. **Optional Features:** Should we defer Phase 3 (limited coverage) to Sprint 8 to ensure quality?
4. **Testing Depth:** Is ~140 new tests sufficient, or do we need more coverage?
5. **Documentation:** Any specific documentation requirements beyond what's listed?

---

**Ready to proceed with implementation!** Let me know if you'd like to adjust priorities or start with Task 6.4.
