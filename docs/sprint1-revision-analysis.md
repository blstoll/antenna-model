# Sprint 1 Revision Analysis: v1.0 → v2.0 Alignment

**Date:** 2025-10-25
**Purpose:** Identify and document Sprint 1 work that doesn't align with the revised v2.0 implementation plan

---

## Executive Summary

Sprint 1 was completed under the **v1.0 interpolation-based approach**, but the implementation plan was subsequently revised to **v2.0 physics-based approach**. This document identifies misalignments and provides concrete recommendations for corrections.

**Key Finding:** The core data structures in `src/data/types.rs` were designed for a pure B-spline interpolation service and need to be restructured to support the physics-based model with correction surfaces.

---

## Background: v1.0 → v2.0 Transition

### Original Approach (v1.0)
- **Primary model:** 4D B-spline interpolation over (azimuth, elevation, frequency, temperature)
- **Calibration:** Fit B-splines directly to measurement data
- **Runtime:** Interpolate G/T values from B-spline coefficients

### Revised Approach (v2.0)
- **Primary model:** Physics-based computation (physical optics, Ruze equation, mesh effects, coma aberration)
- **Calibration:**
  1. Physical parameters (reflector geometry, feed, mesh) - shared or per-antenna
  2. Optional correction surface (B-spline) fitted to residuals (measured - physics model)
- **Runtime:** `G/T_final = PhysicsModel(antenna_config) + CorrectionSurface(freq, cone, clock)`

**Critical difference:** B-splines now serve as **corrections to the physics model**, not the primary model.

---

## Detailed Analysis of Sprint 1 Artifacts

### ✅ What Remains Valid

The following Sprint 1 work aligns with v2.0 and requires NO changes:

1. **Error Handling Framework** (`src/error.rs`)
   - ✅ All error types remain valid (`DataError`, `ApiError`, `ValidationError`, `ComputationError`, `ConfigError`)
   - ✅ Error handling patterns are generic and support both models
   - ✅ No changes needed

2. **Configuration System** (`src/config/settings.rs`)
   - ✅ Service configuration structure remains valid
   - ✅ Logging, server, performance config all apply to v2.0
   - ✅ Antenna config loading mechanism is reusable
   - ✅ No structural changes needed

3. **API Infrastructure** (`src/api/`)
   - ✅ REST API server, middleware, routes remain valid
   - ✅ `/status` endpoint works as-is
   - ✅ Request/response infrastructure is model-agnostic
   - ✅ No changes needed at this layer

4. **Project Structure**
   - ✅ Cargo workspace, build system, test infrastructure all valid
   - ✅ No changes needed

---

## ❌ What Needs Correction

### Issue #1: Core Data Types (`src/data/types.rs`)

**Current State (v1.0):**
```rust
pub struct AntennaCalibration {
    pub antenna_id: String,
    pub metadata: CalibrationMetadata,
    pub model: BSplineModel4D,  // ❌ This is the PRIMARY model
    pub validity_ranges: ValidityRanges,
}
```

**Problem:**
- `BSplineModel4D` is positioned as the primary antenna model
- No structures exist for physical antenna parameters
- Assumes azimuth/elevation coordinates (should use E-clock/E-cone for physics model)
- No support for physics + correction surface hybrid approach

**Required Changes:**

According to Sprint 2 Task 2.1 (lines 297-330 in implementation plan), we need NEW types:

```rust
// NEW: Physics-based antenna configuration (Sprint 2)
pub struct ReflectorGeometry {
    pub diameter_m: f64,
    pub focal_length_m: f64,
    pub f_over_d_ratio: f64,
    pub surface_rms_mm: f64,
}

pub struct FeedParameters {
    pub position: (f64, f64, f64),  // (x, y, z) in meters
    pub q_factor: f64,               // cos^q pattern parameter
    pub phase_center_offset_m: f64,
}

pub struct MeshParameters {
    pub mesh_spacing_mm: f64,
    pub wire_diameter_mm: f64,
}

pub struct PhysicalAntennaConfig {
    pub reflector: ReflectorGeometry,
    pub feed: FeedParameters,
    pub mesh: MeshParameters,
}
```

**Revised `AntennaCalibration` (v2.0):**
```rust
pub struct AntennaCalibration {
    pub antenna_id: String,
    pub metadata: CalibrationMetadata,

    // NEW: Physics-based primary model
    pub physical_config: PhysicalAntennaConfig,

    // REPURPOSED: B-spline now optional, for corrections only
    pub correction_surface: Option<BSplineModel4D>,

    pub validity_ranges: ValidityRanges,
}
```

**Action Items:**
1. ✅ **Keep** existing `BSplineModel4D` structure - it's perfectly valid for correction surfaces
2. ❌ **Add** new physics-based structures (`ReflectorGeometry`, `FeedParameters`, `MeshParameters`, `PhysicalAntennaConfig`)
3. ❌ **Restructure** `AntennaCalibration` to support physics + optional correction
4. ✅ **Keep** `ValidityRanges` and `CalibrationMetadata` - they remain valid

---

### Issue #2: Coordinate System Mismatch

**Current State:**
- `ValidityRanges` uses **azimuth/elevation**
- API schemas likely use azimuth/elevation

**Required for v2.0:**
- Physics model uses **E-clock/E-cone** coordinates
- Need coordinate transformations between systems

**From Sprint 2 Task 2.1 (lines 305-310):**
```rust
// Need coordinate system definitions:
// - Aperture coordinates (ρ, φ') for integration
// - Far-field coordinates (θ, φ) for pattern
// - E-clock/E-cone to Cartesian transformations
```

**Action Items:**
1. ❌ **Add** coordinate transformation module (`src/model/coordinates.rs`) in Sprint 2
2. ❌ **Update** `ValidityRanges` to support BOTH coordinate systems (or convert on the fly)
3. ❌ **Update** API schemas to accept E-clock/E-cone inputs

---

### Issue #3: Calibration Metadata Mismatch

**Current State:**
```rust
pub struct CalibrationMetadata {
    pub rmse_db: f64,        // ✅ Valid for combined model
    pub r_squared: f64,      // ✅ Valid for combined model
    // ... other fields
}
```

**Required for v2.0:**
- Metadata should distinguish between:
  - Physics model parameters (which may be shared across antenna class)
  - Correction surface quality metrics
  - Whether parameter tuning was performed

**Action Items:**
1. ✅ **Keep** existing `CalibrationMetadata` structure - mostly valid
2. ❌ **Add** fields to distinguish physics vs correction metrics:
   ```rust
   pub struct CalibrationMetadata {
       // Existing fields remain
       pub rmse_db: f64,          // Combined model RMSE
       pub r_squared: f64,        // Combined model R²

       // NEW: v2.0-specific fields
       pub physics_only_rmse_db: Option<f64>,  // Physics model RMSE before correction
       pub correction_rmse_db: Option<f64>,    // Residual RMSE after correction
       pub parameters_tuned: bool,              // Was parameter optimization used?
       pub antenna_class: Option<String>,       // Reference to antenna class
   }
   ```

---

### Issue #4: Missing Physics Model Structures

**Current State:**
- `src/model/mod.rs` is empty (0 bytes)
- No physics computation modules exist

**Required for Sprint 2:**

According to Sprint 2 tasks (lines 289-503), need to create:

```
src/model/
├── mod.rs              # Module exports
├── geometry.rs         # ReflectorGeometry, FeedParameters, MeshParameters (Task 2.1)
├── coordinates.rs      # Coordinate transformations (Task 2.1)
├── phase.rs            # Phase function implementations (Task 2.2)
├── illumination.rs     # Feed illumination model (Task 2.3)
├── integration.rs      # Aperture integration engine (Task 2.4)
└── pattern.rs          # Far-field pattern computation (Task 2.5)
```

**Action Items:**
1. ❌ **Create** all physics model modules (Sprint 2 work, not Sprint 1 fix)
2. ✅ **Note:** This is expected - Sprint 2 hasn't started yet

---

## Recommended Correction Plan

### Priority 1: Update Core Data Types (Immediate)

**File: `src/data/types.rs`**

1. **Add** new physics-based structures (after line 114):
   ```rust
   /// Physical reflector geometry parameters
   #[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
   pub struct ReflectorGeometry {
       /// Dish diameter in meters
       pub diameter_m: f64,

       /// Focal length in meters
       pub focal_length_m: f64,

       /// f/D ratio (typically 0.3 - 0.5)
       pub f_over_d_ratio: f64,

       /// Surface RMS error in millimeters
       pub surface_rms_mm: f64,
   }

   // Add FeedParameters, MeshParameters, PhysicalAntennaConfig...
   ```

2. **Restructure** `AntennaCalibration` (around line 15):
   ```rust
   #[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
   pub struct AntennaCalibration {
       pub antenna_id: String,
       pub metadata: CalibrationMetadata,

       // NEW: Physics-based model (primary)
       pub physical_config: PhysicalAntennaConfig,

       // REPURPOSED: B-spline model for corrections (optional)
       pub correction_surface: Option<BSplineModel4D>,

       pub validity_ranges: ValidityRanges,
   }
   ```

3. **Update** builders and validation logic accordingly

4. **Update** tests to reflect new structure

### Priority 2: Update Metadata (Immediate)

**File: `src/data/types.rs`**

Add v2.0-specific fields to `CalibrationMetadata` to track:
- Physics model quality separately from combined model
- Whether parameter tuning was used
- Antenna class reference

### Priority 3: Coordinate System Support (Sprint 2)

**File: `src/model/coordinates.rs` (NEW)**

Create coordinate transformation module as part of Sprint 2 Task 2.1:
- E-clock/E-cone ↔ Cartesian
- Azimuth/Elevation ↔ θ/φ
- Aperture coordinates (ρ, φ')

### Priority 4: Update Configuration (Minor)

**File: `src/config/settings.rs`**

Minor updates if needed:
- Antenna class definitions (if using shared parameters)
- Otherwise, existing structure is fine

---

## Migration Strategy

### Option A: Immediate Full Correction (Recommended)

**Pros:**
- Clean alignment with v2.0 plan before Sprint 2 starts
- Avoids technical debt
- Easier to implement physics model with correct data structures

**Cons:**
- Requires rework of Sprint 1 code
- Tests need updates

**Recommendation:** ✅ **Choose this option**

**Steps:**
1. Update `src/data/types.rs` with new structures
2. Update tests in `src/data/types.rs`
3. Update serialization tests
4. Verify build passes
5. Commit with message: "Align Sprint 1 data types with v2.0 physics-based approach"

### Option B: Incremental Correction During Sprint 2

**Pros:**
- No immediate rework needed
- Can add new structures alongside old ones

**Cons:**
- Technical debt accumulates
- Risk of confusion between old and new structures
- Harder to maintain two parallel approaches

**Recommendation:** ❌ **Avoid this option**

---

## Impact Assessment

### Files Requiring Changes

| File | Change Type | Urgency | Complexity |
|------|-------------|---------|------------|
| `src/data/types.rs` | Major restructuring | High | Medium |
| `src/data/types.rs` tests | Updates for new structure | High | Low |
| `src/config/settings.rs` | Minor additions | Low | Low |
| `src/model/coordinates.rs` | New file (Sprint 2) | Medium | Medium |

### Files Requiring NO Changes

- ✅ `src/error.rs` - Error types remain valid
- ✅ `src/config/settings.rs` - Core config structure valid
- ✅ `src/api/*` - API infrastructure is model-agnostic
- ✅ `src/lib.rs` - Module exports adapt automatically
- ✅ `Cargo.toml` - Dependencies remain valid

---

## Testing Implications

### Tests Requiring Updates

1. **`src/data/types.rs` unit tests:**
   - Update `AntennaCalibration` builder tests
   - Add tests for new physics structures
   - Update serialization tests for new format
   - Keep existing `BSplineModel4D` tests (still valid)

2. **Integration tests:**
   - Currently minimal, so low impact
   - Update when Sprint 2 adds model evaluation logic

### Tests Remaining Valid

- ✅ `BSplineModel4D` validation tests (structure unchanged)
- ✅ `ValidityRanges` tests (may need extension, but core logic valid)
- ✅ Configuration parsing tests
- ✅ Error handling tests

---

## Sprint 2 Readiness

After corrections, Sprint 2 can proceed cleanly:

✅ **Sprint 2 Task 2.1** (Antenna Geometry Data Structures):
- Data types ready to use
- Can create `src/model/geometry.rs` referencing `PhysicalAntennaConfig`

✅ **Sprint 2 Task 2.2-2.5** (Physics computations):
- Clear input structures available
- No impedance from Sprint 1 legacy code

✅ **Sprint 4** (Calibration Tool):
- Calibration artifact format correctly structured for physics + corrections
- `BSplineModel4D` available for correction surface fitting

---

## Recommended Next Steps

1. **Review this analysis** with stakeholders
2. **Approve correction approach** (Option A recommended)
3. **Execute Priority 1 corrections** to `src/data/types.rs`:
   - Add physics structures
   - Restructure `AntennaCalibration`
   - Update tests
4. **Verify build and tests pass**
5. **Commit changes** with clear v2.0 alignment message
6. **Proceed to Sprint 2** with clean foundation

---

## Conclusion

Sprint 1 work is **largely valid** - error handling, configuration, and API infrastructure all align with v2.0. The **primary misalignment** is in the core data types, which assume a pure interpolation approach.

**Recommended action:** Update `src/data/types.rs` immediately (Priority 1) to restructure `AntennaCalibration` for physics + correction surface approach. This is a **medium-complexity change** that will take **2-3 hours** but provides a clean foundation for Sprint 2.

**Impact:** Low risk, high value. The correction aligns Sprint 1 work with the v2.0 plan and prevents technical debt from accumulating.
