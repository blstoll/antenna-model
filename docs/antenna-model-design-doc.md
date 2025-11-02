# Parabolic Dish Antenna Model Design Document

## Executive Summary

This document outlines the design for a high-performance antenna gain model for parabolic dish antennas with steerable feeds. The system targets satellite communication applications across 100 MHz - 50 GHz with 1 dB accuracy requirements for main lobe and first sidelobe regions.

## 1. System Requirements

### 1.1 Performance Requirements
- **Accuracy**: ±1 dB for main lobe and first sidelobe (down to -30 dB from peak)
- **Frequency Range**: 100 MHz to 50 GHz (500:1 ratio)
- **Computation Speed**: 1-20 evaluations per second
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
2. **Extended**: Generate loss heatmaps across antenna field of view
   - Generate grid of emitter positions (rectangular azimuth/elevation or H3 hexagonal)
   - Compute loss relative to peak gain for each grid point
   - Support field-of-view clipping based on antenna beamwidth
3. **Coordinate System Flexibility**:
   - Auto-detect ECEF (Earth-Centered Earth-Fixed) vs Geodetic (lon, lat, alt) coordinates
   - Transform all positions to antenna frame for gain computation
   - Handle vehicle attitude for proper coordinate frame alignment

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
Ψ_path = k·[ρ²/(4f) - ρ·sin(θ)·cos(φ-φ')]
```

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

E-clock/E-cone to physical feed position:
```
displacement = 2·f·tan(cone_angle/2)
x_feed = displacement·cos(clock_angle)
y_feed = displacement·sin(clock_angle)
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

### 4.1 Input Data Sources

#### G/T Measurements
- **Format**: Tables indexed by frequency, E-clock, E-cone
- **Content**: G/T values in dB/K
- **Coverage**: Main lobe and several sidelobes

#### Reference Patterns
- On-axis gain at multiple frequencies
- Feed pattern cuts (E-plane, H-plane)
- Phase center measurements

### 4.2 Calibration Process

   1. Extract gain from G/T using noise temperature model
   2. Fit Zernike polynomial model to gain surface
   3. Optimize mesh parameters via differential evolution
   4. Generate correction surfaces for systematic errors


### 4.3 Expected Calibration Artifacts

1. **Mesh Parameter Set**
   - Mesh spacing: 1-10 mm range
   - Wire diameter: 0.05-1 mm range
   - Surface RMS: 0.1-2 mm range

2. **Aberration Coefficient Matrix**
   - Zernike coefficients up to 5th order
   - Frequency-dependent scaling factors

3. **Correction Lookup Tables**
   - 3D interpolation grid: (E-clock, E-cone, frequency) → correction_dB
   - Separate tables for main lobe, sidelobes, far field

4. **Feed Model Parameters**
   - Q-factor: 6-10 range
   - Phase center offset: ±λ/4 typical
   - Asymmetry factor for E/H plane differences

5. **Feed Configurations** (Multi-Feed Support)
   - Feed ID to physical position mapping
   - Feed-specific patterns and frequency ranges
   - Example feed configurations:
     - `s_band_feed`: 2.0-2.3 GHz, position offset (0, 0, 0) - at focal point
     - `x_band_feed`: 7.1-8.5 GHz, position offset (0.05, 0, 0) - slightly off-axis
     - `ka_band_feed`: 25.5-27.0 GHz, position offset (0, 0.05, 0) - different offset
   - Per-feed calibration corrections (if applicable)

### 4.4 Validation Metrics

   - Main-lobe max error - target < 1.0dB
   - first side-lobe max error - target < 1.0dB
   - Overall fit quality - root mean squared error
   - Model correlation - R^2
   - Outlier scenarios - cases exceeding tolerance