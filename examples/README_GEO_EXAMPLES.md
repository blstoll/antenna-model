# GEO Satellite Antenna Examples

This directory contains realistic example requests for a GEO satellite antenna scenario, created as part of the antenna model improvement plan (Phase 1).

## Scenario Overview

**Satellite Configuration:**
- **Type**: Geosynchronous (GEO) communications satellite
- **Orbit**: Equatorial orbit at 35,786 km altitude
- **Longitude**: -118° (above California)
- **Antenna**: 34m parabolic dish (DSN-class)
- **Frequency**: X-band (8.4-8.5 GHz, λ ≈ 3.55 cm)
- **Focal Length**: 13.6m (f/D = 0.4)

**Ground Station:**
- **Location**: Southern California (lon=-118°, lat=34°)
- **Altitude**: 100m above sea level
- **Slant Range**: ~37,041 km

**Theoretical Performance:**
- **Maximum Gain**: 67.7 dBi (aperture efficiency 0.65)
- **Ruze Loss**: ~0.5 dB (0.5mm RMS surface)
- **Expected Peak**: 67.2 dBi
- **3dB Beamwidth**: ~0.05° (1.9 km footprint at ground from GEO)

---

## Example Files

### 1. `geo_perfect_alignment.json` - Perfect Alignment

**Purpose:** Verify maximum gain calculation

**Configuration:**
- Feed at ideal focal point (on-axis)
- Emitter (ground station) aligned with boresight
- No feed offset, no pointing error

**Expected Results:**
- **Gain**: 67.2 dBi (near theoretical maximum)
- **Loss**: 0.5 dB (Ruze efficiency penalty only)
- **Reference Gain**: 67.7 dBi
- **Warnings**: Uncalibrated antenna notice

**Validation:**
```
Theoretical: G = 10×log₁₀(0.65 × (πD/λ)²) = 67.7 dBi
Ruze:        η = exp(-(4π×0.0005/0.0355)²) = 0.969 → -0.14 dB
Expected:    67.7 - 0.14 ≈ 67.6 dBi
```

**Use Case:** Baseline test to ensure physics model produces correct maximum gain.

---

### 2. `geo_feed_emitter_colocated_offset.json` - 5° Beam Steering

**Purpose:** Verify sidelobe levels and beam steering capability

**Configuration:**
- Feed steered 5° off-axis (lateral displacement ~1.19m at focal plane)
- Emitter at ground location 5° from boresight (same direction as feed) — the
  emitter sits in the *steered beam*, so the response reports gain near the
  steered-beam peak (heavily reduced by scan loss), not a value 180° away.
- Feed and emitter co-located angularly

**Expected Results:**
- **Gain**: 50-55 dBi (sidelobe region)
- **Loss**: 12-17 dB from peak
- **Pattern**: First/second sidelobe region (5° = 100× beamwidth)

**Physics Check:**
- Feed offset formula: d = 2×f×tan(θ/2) = 2×13.6×tan(2.5°) ≈ 1.19m
- Ground displacement: 37,041 km × tan(5°) ≈ 3,241 km arc length

**Use Case:** Test beam steering and verify sidelobe levels are realistic (not millions of dB loss).

---

### 3. `geo_feed_emitter_separated.json` - Pointing Error

**Purpose:** Demonstrate gain loss when beam is misaligned with emitter

**Configuration:**
- Feed steered 2° from boresight
- Emitter ~3° from feed beam direction (~5° total from boresight)
- Angular mismatch between steered beam and emitter location

**Expected Results:**
- **Gain**: Reduced by angular separation between beam peak and emitter
- **Loss**: Function of beam/emitter mismatch angle
- **Pattern**: Demonstrates that emitter location relative to **steered beam** matters, not just boresight

**Use Case:** Verify that gain correctly accounts for both feed steering and emitter location independently.

---

### 4. `geo_large_feed_offset.json` - Severe Aberrations

**Purpose:** Show impact of large feed displacement on gain (coma, defocusing)

**Configuration:**
- Feed steered 10° from boresight (lateral ~2.38m, severe offset)
- Emitter within 1° of boresight (nearly aligned with dish pointing)
- Large feed/emitter angular separation

**Expected Results:**
- **Gain**: ~47 dBi (20 dB loss from peak)
- **Loss**: Dominated by feed aberrations (coma, defocusing) despite emitter being near boresight
- **Pattern**: Demonstrates that feed position matters independently of emitter location

**Physics:**
- 10° feed offset creates severe aperture phase errors
- Coma aberration penalty: ~(offset/f)² ≈ (2.38/13.6)² ≈ 3% → ~13 dB loss
- Additional defocusing and higher-order aberrations

**Use Case:** Test feed offset aberration modeling. Gain should be poor even though emitter is well-placed.

---

### 5. `geo_beam_squint.json` - Frequency-Dependent Pointing

**Purpose:** Demonstrate beam squint correction when pointing and operating frequencies differ

**Configuration:**
- Same geometry as Example 1 (perfect alignment)
- **Pointing Frequency**: 8400 MHz (feed positioned for this frequency)
- **Operating Frequency**: 8450 MHz (actual transmission frequency)
- **Frequency Offset**: 50 MHz (0.6% shift)

**Expected Results:**
- **Beam Squint**: ~0.03° (0.6% of 0.05° beamwidth)
- **Gain**: 67.1 dBi (very slight reduction from squint)
- **Metadata**: `beam_squint_deg` field populated in response
- **Loss**: Negligible for this small frequency offset

**Physics:**
```
Δθ ≈ (Δf/f) × beamwidth
   = (50/8400) × 0.05°
   ≈ 0.0003° (negligible)

For larger offsets (e.g., 500 MHz):
Δθ ≈ (500/8000) × 0.05° ≈ 0.003° (significant, 60% of beamwidth!)
```

**Use Case:** Verify beam squint correction is applied correctly. For small offsets, effect is negligible. For wideband systems or dual-frequency operation, becomes important.

---

## Coordinate System Notes

All examples use **ECEF (Earth-Centered Earth-Fixed)** coordinates:

**Satellite ECEF** (lon=-118°, lat=0°, alt=35786 km):
```
x = -19,794,863.3 m
y = -37,228,723.3 m
z = 0.0 m
```

**Ground Station ECEF** (lon=-118°, lat=34°, alt=100 m):
```
x = -2,485,073.2 m
y = -4,673,742.9 m
z = 3,546,502.5 m
```

**Boresight Vector** (unit vector from satellite toward ground):
```
[0.467315, 0.878891, 0.095745]
```

**Boresight Position** (satellite + 10m in boresight direction):
```
Defines the pointing direction of the reflector dish
```

**Feed Position** (satellite + [10m + focal_length] in boresight direction + lateral offset):
```
Focal length = 13.6m
Lateral offset depends on steering angle: d = 2×f×tan(θ/2)
```

---

## API Parameter Notes

### Removed Parameter

**`vehicle_attitude`**: This parameter is **NOT** included in these examples as part of the API simplification (Phase 2). The `reflector_boresight` position already establishes the dish orientation relative to the vehicle, making `vehicle_attitude` redundant.

### Optional Parameters

**`pointing_frequency_mhz`**: Only specified in Example 5 (beam squint). When omitted, defaults to `frequency_mhz`.

**`include_reference`**: Set to `true` in all examples to compute ideal reference gain (feed at focus, boresight pointing at emitter). This allows calculation of `loss_db = reference_gain_db - gain_db`.

---

## Testing These Examples

### Quick Test
```bash
# Start the antenna model service
cargo run --release --bin antenna-model

# Test Example 1a (perfect alignment)
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d @examples/requests/geo_perfect_alignment.json | jq

# Expected response:
# {
#   "gain_db": ~67.2,
#   "reference_gain_db": ~67.7,
#   "loss_db": ~0.5
# }
```

### Batch Test All Examples
```bash
for file in examples/requests/geo_*.json; do
  echo "Testing: $file"
  curl -s -X POST http://localhost:3000/api/v1/gain \
    -H "Content-Type: application/json" \
    -d @$file | jq '.gain_db, .loss_db'
  echo ""
done
```

### Expected Gain Summary
| Example | Expected Gain | Expected Loss | Notes |
|---------|--------------|---------------|-------|
| 1a - Perfect | 67.2 dBi | 0.5 dB | Maximum performance |
| 1b - 5° Offset | 50-55 dBi | 12-17 dB | Sidelobe region |
| 1c - Separated | Variable | Varies | Pointing error |
| 1d - Large Offset | ~47 dBi | ~20 dB | Feed aberrations |
| 1e - Beam Squint | 67.1 dBi | 0.6 dB | Minimal squint effect |

---

## Hand Calculation Reference

### Example 1a - Perfect Alignment

**Theoretical Maximum Gain:**
```
G = 10×log₁₀(η_ap × (πD/λ)²)

η_ap = 0.65 (aperture efficiency)
D = 34 m
f = 8450 MHz
λ = c/f = 299,792,458 / 8,450,000,000 = 0.03548 m

G = 10×log₁₀(0.65 × (π × 34 / 0.03548)²)
  = 10×log₁₀(0.65 × 3,018.2²)
  = 10×log₁₀(5,920,428)
  = 67.72 dBi
```

**Ruze Efficiency (Surface Errors):**
```
η_ruze = exp(-(4πσ/λ)²)

σ = 0.5 mm = 0.0005 m (RMS surface error)
λ = 0.03548 m

η_ruze = exp(-(4π × 0.0005 / 0.03548)²)
       = exp(-(0.1774)²)
       = exp(-0.0315)
       = 0.969

Loss = 10×log₁₀(0.969) = -0.14 dB
```

**Expected Gain:**
```
G_final = 67.72 - 0.14 = 67.58 dBi

Acceptance: Model output within ±1 dB → [66.6, 68.6] dBi
```

### Example 1b - 5° Offset

**Beamwidth:**
```
θ_3dB ≈ 70λ/D = 70 × 0.03548 / 34 = 0.073° (HPBW)
θ_null ≈ 1.22λ/D = 1.22 × 0.03548 / 34 = 0.00127 rad = 0.073°
```

**Off-Axis Angle:**
```
θ = 5° / 0.073° = 68.5 beamwidths off-axis
```

**Gaussian Approximation** (invalid for large angles):
```
L = -12 × (θ/θ_3dB)² = -12 × 68.5² = -56,322 dB ❌ WRONG!
```

**Realistic Sidelobe Level:**
```
First sidelobe: ~-17 dB from peak
Far sidelobes: ~-25 to -30 dB from peak

Expected for 5° (far sidelobes): ~50 dBi (67 - 17 dB)
```

---

## Deprecated Examples

The following examples in `examples/requests/` have unrealistic coordinates and should **not** be used:
- `gain_request.json` - Vehicle only 10m from emitter
- `heatmap_request.json` - Same issue
- `batch_request.json` - Same issue

These will be updated or removed in future versions.

---

## References

- **Improvement Plan**: `docs/model_improvement_plan.md` - Full details on Phase 1 example creation
- **Design Doc**: `docs/antenna-model-design-doc.md` - Physics model equations
- **API Docs**: `docs/api-documentation.md` - Request/response schemas

---

**Created**: 2025-11-18
**Phase**: 1 of antenna model improvement plan
**Status**: Complete - Ready for Phase 3 (physics model analysis)
