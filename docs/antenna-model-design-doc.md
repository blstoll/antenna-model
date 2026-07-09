# Parabolic Dish Antenna Model Design Document

## Executive Summary

This document outlines the design for a high-performance antenna gain model for parabolic dish antennas with steerable feeds. The system targets satellite communication applications across 100 MHz - 50 GHz with flexible accuracy requirements based on calibration status:

- **Fully Calibrated**: ±1 dB accuracy (main lobe and first sidelobe)
- **Partially Calibrated (Boresight)**: ±1 dB at boresight, ±2-3 dB off-axis, ±1-2 dB loss
- **Partially Calibrated (Limited Coverage)**: ±1-1.5 dB in-coverage, ±2-3 dB extrapolated
- **Uncalibrated (Design Specs)**: ±3-5 dB absolute gain, ±2-3 dB loss

The system supports graceful degradation from fully calibrated to uncalibrated antennas, prioritizing **loss accuracy** (reference_gain - actual_gain) where systematic errors cancel.

## 1. System Requirements

### 1.1 Performance Requirements
- **Accuracy** (varies by calibration status):
  - **Fully Calibrated**: ±1 dB for main lobe and first sidelobe (down to -30 dB from peak)
  - **Partially Calibrated (Boresight)**: ±1 dB at boresight, ±2-3 dB off-axis, ±1-2 dB loss
  - **Partially Calibrated (Limited Coverage)**: ±1-1.5 dB in-coverage, ±2-3 dB out-of-coverage
  - **Uncalibrated**: ±3-5 dB absolute gain, ±2-3 dB loss (error cancellation)
- **Frequency Range**: 100 MHz to 50 GHz (500:1 ratio)
- **Computation Speed**: 1-20 evaluations per second (all calibration statuses)
- **Platform**: Rust implementation with GPU acceleration support
- **Memory**: Unconstrained (lookup tables permitted)

### 1.2 Antenna Configuration
- **Reflector Type**: Mesh parabolic reflector
- **f/D Ratio**: ~0.5 (focal length ≈ half the dish diameter)
- **Feed System**: Moveable feeds (not focal plane array)
- **Feed Displacement Range**: Up to ±D/2 from focal point
- **Quality**: High-accuracy, high-quality components assumed

### 1.3 Use Cases
1. **Primary**: Compute antenna gain from 3D geometric configuration
   - Given: Vehicle position (ECEF or Geodetic), vehicle attitude (quaternion/Euler)
   - Given: Reflector boresight position, feed position, emitter position (all 3D coordinates)
   - Given: Operating frequency and optional pointing frequency
   - Compute: Absolute gain at emitter position
   - Optionally compute: Reference gain (ideal: feed at focus, pointing at emitter)
   - Optionally compute: Loss = reference gain - actual gain
   - Support: Multiple feeds per antenna (composite antenna_id + feed_id identifier)
   - Support: Beam squint correction for pointing frequency ≠ operating frequency
   - **Support: All calibration statuses** (fully calibrated, partially calibrated, uncalibrated)
   - **Return: Calibration status and accuracy estimates** in API responses

2. **Extended**: Generate loss heatmaps across antenna field of view
   - Generate grid of emitter positions (rectangular azimuth/elevation or H3 hexagonal)
   - Compute loss relative to peak gain for each grid point
   - Support field-of-view clipping based on antenna beamwidth
   - **Include calibration status information** in heatmap responses

3. **Coordinate System Flexibility**:
   - Auto-detect ECEF (Earth-Centered Earth-Fixed) vs Geodetic (lon, lat, alt) coordinates
   - Transform all positions to antenna frame for gain computation
   - Handle vehicle attitude for proper coordinate frame alignment

4. **Calibration Upgrade Path** (NEW):
   - **Deploy uncalibrated antennas** with design specs only (±3-5 dB absolute, ±2-3 dB loss)
   - **Quick boresight calibration** (~1 hour test time) for ±1-2 dB loss accuracy
   - **Incremental upgrades** to partial or full calibration as measurements become available
   - **No service downtime** for calibration updates

## 2. Mathematical Formulation

### 2.1 Core Physical Optics Model

The far-field electric field pattern:
```
E(θ,φ) = (jk·exp(-jkr))/(2λr) ∬_Aperture A(ρ,φ') · exp[jΨ(ρ,φ')] · ρ dρ dφ'
```

### 2.2 Phase Components

Total phase function:
```
Ψ_total = Ψ_path + Ψ_feed_displacement + Ψ_surface + Ψ_mesh
```

#### Path Phase (Standard):
```
Ψ_path = k·[ρ²/(4f)·(1−cosθ) − ρ·sinθ·cos(φ−φ')]
```

**Derivation:** The feed→surface optical path is k(f + z) with z = ρ²/(4f). The
far-field projection removes k(ρ sinθ cos(φ−φ') + z cosθ). Dropping the constant
kf gives the formula above. The (1−cosθ) factor is essential: it ensures the aperture
is equiphase at θ = 0, which is the defining optical property of a parabola.

*Note (2026-06-11): The earlier formula omitted (1−cosθ), which injected a
spurious defocus across the aperture, corrupting off-axis pattern shape.*

#### Feed Displacement Phase (Coma Aberration):
```
Ψ_feed_displacement = k·δ_feed·[ρ/(2f)]·[2·cos(α) - (ρ/(2f))·cos(2α-φ')]
```

#### Surface Error Phase:
```
Ψ_surface = (4π/λ)·ε(ρ,φ')·cos(θ_incident)
```

#### Mesh-Specific Phase:
```
Ψ_mesh = arctan[(2π·d_mesh/λ)·sin(θ_incident)]
```

### 2.3 Illumination Function

Feed pattern model using cos^q approximation:
```
F_feed(ψ) = cos(ψ)^q  for ψ < π/2, else 0
```
where q ≈ 6-10 for typical 10 dB edge taper.

### 2.4 Mesh Reflector Efficiency

Ruze's equation for surface errors:
```
η = exp(-(4π·σ/λ)²)
```

Frequency-dependent mesh transparency:
```
T = 1/(1 + (λ₀/λ)²)  for λ > 10·mesh_spacing
```

### 2.5 Coordinate Transformations

E-clock/E-cone to physical feed position. A lateral feed displacement steers
the beam to the OPPOSITE side, so to aim the beam at clock angle `clock_angle`
the feed is displaced at `clock_angle + 180°` (hence the negative x/y). The
displacement is divided by the beam deviation factor (BDF, Lo 1960) so the PO
beam peak lands at the requested angle:
```
displacement = 2·f·tan(cone_angle/2) / BDF
x_feed = -displacement·cos(clock_angle)
y_feed = -displacement·sin(clock_angle)
z_feed = -displacement²/(4f)  for large displacements
```

## 3. Algorithmic Considerations

### 3.1 Edge Cases

#### Coordinate Transformation Edge Cases
- **Coordinate Auto-Detection Boundary**:
  - Threshold: |x| or |y| or |z| > 6400 km → ECEF
  - Near-threshold coordinates may be ambiguous (unlikely in practice)
  - Validation: Detect obviously invalid coordinates (NaN, Inf, unreasonable magnitudes)
- **Geodetic Singularities**:
  - Poles (latitude = ±90°): Handle azimuth ambiguity
  - Earth center (altitude → -6371 km): Invalid for antenna locations
- **Attitude Singularities**:
  - Gimbal lock in Euler angles (pitch = ±90°)
  - Quaternion normalization: Warn if |q| deviates from 1.0 by >0.01
- **Vehicle at High Altitude**:
  - Low Earth Orbit (LEO): 200-2000 km altitude
  - Medium Earth Orbit (MEO): 2000-35786 km altitude
  - Ensure coordinate transforms remain accurate

#### Large Feed Offset (> 0.3f)
- Switch from parabolic approximations to ray tracing
- Include higher-order Seidel aberrations
- Account for increased spillover

#### Near-Boresight/Far-Feed Scenario
- Compute direct feed reception
- Calculate reflected path with severe phase errors
- Model interference between paths

#### Frequency-Dependent Effects
- **Low frequency (< 1 GHz)**: Mesh transparency model
- **Transition region**: Full Floquet mode analysis
- **High frequency (> 10 GHz)**: Surface roughness dominance

#### Multi-Feed Antenna Scenarios
- **Feed Selection**: Validate feed_id exists for antenna_id
- **Feed Offset**: Feed positions typically at or near focal point
- **Frequency Bands**: Different feeds for different frequency ranges (e.g., S-band, X-band, Ka-band)
- **Beam Squint**: Frequency-dependent beam pointing differs from mechanical pointing

### 3.2 Numerical Stability

- Adaptive integration near pattern nulls
- Minimum noise floor enforcement (-60 dB typical)
- Kaiser windowing for sidelobe continuity

## 4. Calibration Methodology

### 4.1 Calibration Status Types

The system supports multiple calibration statuses, enabling graceful degradation from fully calibrated to uncalibrated antennas:

#### Fully Calibrated (Target Status)
- **Data Available**: Dense measurement grid across azimuth, elevation, frequency
- **Physics Model**: Fully tuned parameters
- **Correction Surface**: Dense B-spline capturing all residuals
- **Accuracy Estimate**: ±1 dB (main lobe and first sidelobe)
- **Use Cases**: Critical science antennas, deep space network, high-accuracy applications

#### Partially Calibrated - Boresight Only
- **Data Available**: Boresight measurements (az=0, el=0) across frequency and optionally temperature
- **Physics Model**: Parameters tuned to match boresight measurements (surface RMS, q-factor, mesh properties)
- **Correction Surface**: Optional, typically frequency-only (single spatial point)
- **Accuracy Estimate**:
  - Absolute gain (boresight): ±1 dB (tuned)
  - Absolute gain (off-axis): ±2-3 dB (physics model only)
  - Loss (relative): ±1-2 dB (systematic errors cancel)
- **Use Cases**: Feed steering analysis, quick calibration validation, operational antennas with limited test data

#### Partially Calibrated - Limited Coverage
- **Data Available**: Measurements at sparse grid (e.g., main lobe + first sidelobe only)
- **Physics Model**: Parameters tuned to measurements
- **Correction Surface**: Optional, sparse B-spline (limited spatial coverage)
- **Accuracy Estimate**:
  - In-coverage: ±1-1.5 dB
  - Out-of-coverage: ±2-3 dB (extrapolated)
  - Loss: ±1-1.5 dB
- **Use Cases**: Operational antennas with partial characterization, targeted measurement campaigns

#### Uncalibrated
- **Data Available**: Design specifications (diameter, f/D, feed location, surface quality estimate)
- **Physics Model**: Default parameters from design specs
- **Correction Surface**: None
- **Accuracy Estimate**:
  - Absolute gain: ±3-5 dB
  - Loss (relative gain): ±2-3 dB (systematic errors partially cancel)
- **Use Cases**: New antennas, prototype modeling, fallback when data unavailable

**Key Design Principle**: Loss accuracy (reference_gain - actual_gain) is prioritized over absolute gain accuracy, as systematic parameter errors cancel in the difference computation.

### 4.2 Input Data Sources

#### Design Specifications (Required for Uncalibrated Antennas)
- **Reflector Geometry**: Diameter, focal length, f/D ratio, surface RMS estimate
- **Feed Configuration**: Position, q-factor, phase center offset, frequency range
- **Mesh Parameters** (if applicable): Mesh spacing, wire diameter
- **Source**: Manufacturer specifications, engineering drawings, visual inspection
- **Use**: Initial parameter estimates for uncalibrated antennas; starting point for parameter tuning

#### G/T Measurements (Full Calibration)
- **Format**: Tables indexed by frequency, E-clock, E-cone
- **Content**: G/T values in dB/K
- **Coverage**: Main lobe and several sidelobes
- **Density**: Dense grid (>10 points per beamwidth)

#### Boresight Measurements (Partial Calibration)
- **Format**: Frequency sweep at (az=0, el=0)
- **Content**: G/T values across frequency range
- **Coverage**: Single spatial point, multiple frequencies
- **Use**: Parameter tuning (surface RMS, q-factor, mesh properties)

#### Sparse Grid Measurements (Limited Coverage Calibration)
- **Format**: Partial angular grid (e.g., ±15° azimuth/elevation)
- **Content**: G/T values at sparse spatial sampling
- **Coverage**: Limited angular region (2-5 points per beamwidth)
- **Use**: Parameter tuning + sparse correction surface

#### Reference Patterns
- On-axis gain at multiple frequencies
- Feed pattern cuts (E-plane, H-plane)
- Phase center measurements

### 4.3 Calibration Process

#### Full Calibration Workflow (Existing)
   1. Extract gain from G/T using noise temperature model
   2. Fit Zernike polynomial model to gain surface
   3. Optimize mesh parameters via differential evolution
   4. Generate correction surfaces for systematic errors
   5. Validate against measurements (target: <1 dB error in main lobe/first sidelobe)

#### Boresight Calibration Workflow (NEW - Sprint 7)
   1. **Load design specs** as initial parameter estimates
   2. **Tune physical parameters** using differential evolution:
      - Optimize: `surface_rms_mm`, `q_factor`, `mesh_spacing_mm`, `wire_diameter_mm`
      - Objective: Minimize `|measured_G/T - physics_model_G/T|` at boresight across frequencies
      - Constraints: Keep parameters within physically reasonable ranges
   3. **Optional correction surface**:
      - Fit 1D frequency-only correction: `correction(freq) = measured - physics`
      - Skip if physics model error < 0.5 dB (low priority)
   4. **Validate**: Check that tuned parameters are physically reasonable
   5. **Output**: Calibration artifact with `PartiallyCalibrated` status
      - Coverage: azimuth=[0,0], elevation=[0,0], frequency range from measurements

#### Limited Coverage Calibration Workflow (Future)
   1. **Load design specs** as initial estimates
   2. **Tune physical parameters** across all measurement points
   3. **Optional correction surface**:
      - Fit sparse 3D B-spline (azimuth, elevation, frequency)
      - Use measurements to construct sparse grid
   4. **Validate**: Check in-coverage accuracy
   5. **Output**: Calibration artifact with coverage metadata

#### Uncalibrated Workflow (NO CALIBRATION REQUIRED)
   1. **Load design specs** from configuration file
   2. **Construct `PhysicalAntennaConfig`** from design specifications
   3. **No measurements** - physics model only
   4. **Output**: In-memory calibration with `Uncalibrated` status
      - No `.bin` file required
      - Loaded directly from `antennas.yaml` at service startup


### 4.4 Expected Calibration Artifacts

The calibration artifacts vary based on calibration status:

#### Fully Calibrated Artifacts
1. **Mesh Parameter Set** (Tuned)
   - Mesh spacing: 1-10 mm range
   - Wire diameter: 0.05-1 mm range
   - Surface RMS: 0.1-2 mm range
   - **Source**: Optimized from full measurement grid

2. **Aberration Coefficient Matrix**
   - Zernike coefficients up to 5th order
   - Frequency-dependent scaling factors

3. **Correction Lookup Tables** (Dense)
   - 3D interpolation grid: (E-clock, E-cone, frequency) → correction_dB
   - Separate tables for main lobe, sidelobes, far field
   - **Coverage**: Full field of view

4. **Feed Model Parameters** (Tuned)
   - Q-factor: 6-10 range (optimized)
   - Phase center offset: ±λ/4 typical (measured)
   - Asymmetry factor for E/H plane differences

#### Partially Calibrated Artifacts (Boresight Only)
1. **Mesh Parameter Set** (Tuned from Boresight)
   - Optimized from boresight measurements across frequencies
   - **Accuracy**: Tuned to <1 dB at boresight
   - **Limitation**: Off-axis accuracy ±2-3 dB (physics extrapolation)

2. **Optional Frequency Correction** (1D)
   - Frequency-only correction: `correction(freq)`
   - Stored as degenerate 4D B-spline (single spatial point)
   - **Applied only**: When query is at or near boresight

3. **Calibration Coverage Metadata**
   - Azimuth range: [0.0, 0.0] (single point)
   - Elevation range: [0.0, 0.0] (single point)
   - Frequency range: from measurements
   - Num measurements: typically 10-50 frequency samples

#### Partially Calibrated Artifacts (Limited Coverage)
1. **Mesh Parameter Set** (Tuned from Sparse Grid)
   - Optimized from limited angular measurements

2. **Sparse Correction Surface** (Optional)
   - 3D B-spline with limited spatial coverage
   - **In-coverage**: ±1-1.5 dB accuracy
   - **Out-of-coverage**: ±2-3 dB (physics extrapolation)

3. **Calibration Coverage Metadata**
   - Azimuth range: limited (e.g., [0, 360] or [-30, 30])
   - Elevation range: limited (e.g., [30, 60])
   - Frequency range: from measurements
   - Num measurements: typically 100-500 points

#### Uncalibrated Artifacts (Design Specs Only)
1. **Mesh Parameter Set** (From Design Specs)
   - **Source**: Manufacturer specifications or estimates
   - **Accuracy**: Untested, ±3-5 dB absolute gain
   - **Loss Accuracy**: ±2-3 dB (systematic errors partially cancel)

2. **No Correction Surface**
   - Physics model only

3. **Feed Model Parameters** (Estimated)
   - Q-factor: Typical value (e.g., 8.0 for horn feed)
   - Phase center offset: 0.0 (assumed)

#### Multi-Feed Support (All Calibration Statuses)
5. **Feed Configurations**
   - Feed ID to physical position mapping
   - Feed-specific patterns and frequency ranges
   - Example feed configurations:
     - `s_band_feed`: 2.0-2.3 GHz, position offset (0, 0, 0) - at focal point
     - `x_band_feed`: 7.1-8.5 GHz, position offset (0.05, 0, 0) - slightly off-axis
     - `ka_band_feed`: 25.5-27.0 GHz, position offset (0, 0.05, 0) - different offset
   - Per-feed calibration corrections (if applicable)
   - Each feed can have different calibration status

### 4.5 Validation Metrics

Validation criteria vary by calibration status:

#### Fully Calibrated
   - **Main-lobe max error**: <1.0 dB
   - **First side-lobe max error**: <1.0 dB
   - **Overall RMSE**: <0.5 dB
   - **Model correlation**: R² > 0.95
   - **Coverage**: Full field of view
   - **Outlier scenarios**: <5% of points exceed tolerance

#### Partially Calibrated - Boresight Only
   - **Boresight error**: <1.0 dB across frequencies
   - **Parameter consistency**: Tuned parameters within physically reasonable bounds
   - **Off-axis predictions**: Not validated (physics extrapolation, expect ±2-3 dB)
   - **Loss accuracy**: ±1-2 dB (verified via reference gain computation)
   - **Coverage**: Single spatial point

#### Partially Calibrated - Limited Coverage
   - **In-coverage error**: <1.5 dB
   - **Out-of-coverage**: Not validated (physics extrapolation)
   - **Coverage metadata**: Accurately reflects measurement extent
   - **Transition quality**: Smooth extrapolation at coverage boundaries

#### Uncalibrated
   - **No validation**: Design specs used as-is
   - **Expected accuracy**: ±3-5 dB absolute gain
   - **Loss accuracy**: ±2-3 dB (systematic error cancellation)
   - **Parameter reasonableness**: Check that design specs are physically plausible

#### General Validation (All Statuses)
   - **API response format**: Includes `calibration_status` field with accuracy estimates
   - **Warning generation**: Appropriate warnings for extrapolation or low confidence
   - **Multi-feed support**: Each feed validated independently

---

## 5. Calibration Upgrade Path

The system supports graceful evolution from uncalibrated to fully calibrated antennas:

### 5.1 Upgrade Sequence

```
Uncalibrated (Design Specs Only)
    ↓ Collect boresight measurements (10-50 frequency samples)
Partially Calibrated - Boresight Only
    ↓ Collect sparse off-axis measurements (100-500 points)
Partially Calibrated - Limited Coverage
    ↓ Collect full measurement grid (1000-5000 points)
Fully Calibrated
```

### 5.2 Operational Benefits

1. **Immediate Deployment**: New antennas can be added with design specs only
2. **Incremental Improvement**: Accuracy improves as measurements become available
3. **No Service Interruption**: Calibration upgrades don't require downtime
4. **Loss-First Strategy**: Focus on loss accuracy (1-2 dB) over absolute gain accuracy
5. **Cost-Effective**: Boresight calibration requires ~1 hour test time vs. ~8 hours for full calibration

### 5.3 Accuracy Evolution

| Stage | Test Time | Absolute Gain | Loss Accuracy | Primary Use Case |
|-------|-----------|---------------|---------------|------------------|
| Uncalibrated | 0 hours | ±3-5 dB | ±2-3 dB | Feed steering analysis, prototype modeling |
| Boresight Only | ~1 hour | ±1 dB (boresight), ±2-3 dB (off-axis) | ±1-2 dB | Operational antennas, quick validation |
| Limited Coverage | ~3-4 hours | ±1-1.5 dB (in-coverage) | ±1-1.5 dB | Targeted applications, cost-constrained scenarios |
| Full Calibration | ~8 hours | ±1 dB (full FOV) | ±1 dB | Critical science, deep space network |

### 5.4 Implementation Status

- **Phase 1 (Sprint 6)**: ✅ COMPLETE
  - Data model extensions for calibration statuses
  - Service layer support for all statuses
  - API schemas with calibration status information
  - Uncalibrated antenna loading from design specs

- **Phase 2 (Sprint 7)**: 📋 PLANNED
  - Boresight calibration mode in `calibrate` tool
  - Parameter tuning from boresight measurements
  - Design specs loading and validation
  - Optional frequency-only correction surface

- **Phase 3 (Future)**: 📋 DEFERRED
  - Limited coverage calibration mode
  - Sparse correction surface fitting
  - Coverage analysis and metadata generation

### 5.5 Reference Documents

For detailed design specifications, implementation tasks, and API schemas, see:
- **Detailed Design**: `docs/partial-calibration-design.md`
- **Implementation Plan**: `docs/partial-calibration-implementation-plan.md`
- **Main Implementation Plan**: `docs/implementation-plan.md` (Sprint 6 & 7 sections)