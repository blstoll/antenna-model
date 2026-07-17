# Antenna Model Service - API Documentation

## Overview

The Antenna Model Service provides a REST API for high-accuracy antenna loss modeling with flexible calibration statuses. The API is fully documented using OpenAPI 3.0 specification.

## OpenAPI Specification

The complete API specification is available in `openapi.yaml` at the repository root.

### Viewing the Documentation

There are several ways to view the interactive API documentation:

#### Option 1: Online Swagger Editor (Recommended)

1. Go to https://editor.swagger.io/
2. Click **File → Import File**
3. Upload `openapi.yaml` from the repository root
4. Explore the interactive documentation

#### Option 2: Redocly CLI (Local)

```bash
# Install Redocly CLI globally (optional)
npm install -g @redocly/cli

# Preview the documentation
npx @redocly/cli preview-docs openapi.yaml

# Open browser to http://localhost:8080
```

#### Option 3: Redoc Static HTML

```bash
# Generate static HTML documentation
npx @redocly/cli build-docs openapi.yaml -o api-docs.html

# Open api-docs.html in your browser
open api-docs.html
```

#### Option 4: SwaggerUI Docker

```bash
# Run SwaggerUI with the OpenAPI spec
docker run -p 8080:8080 \
  -e SWAGGER_JSON=/openapi.yaml \
  -v $(pwd)/openapi.yaml:/openapi.yaml \
  swaggerapi/swagger-ui

# Open browser to http://localhost:8080
```

## API Endpoints

### Health & Status

- `GET /health` - Liveness probe (Kubernetes)
- `GET /ready` - Readiness probe (Kubernetes)
- `GET /status` - Service status with loaded antennas

### Gain Computation

- `POST /api/v1/gain` - Single gain computation from 3D geometric configuration
- `POST /api/v1/gain/batch` - Batch gain computation (up to 1000 evaluations)

### Heatmap Generation

- `POST /api/v1/heatmap` - Generate 2D loss heatmap across antenna field of view

### Antenna Information

- `GET /api/v1/antennas` - List all available antennas with feeds
- `GET /api/v1/antennas/{id}` - Get detailed antenna configuration
- `GET /api/v1/antennas/{id}/feeds` - List feeds for specific antenna
- `GET /api/v1/antennas/{id}/feeds/{feed_id}` - Get detailed feed configuration

## Key Features

### Calibration Status Support

The API supports multiple calibration statuses:

- **Fully Calibrated**: ±1 dB accuracy (main lobe/first sidelobe)
- **Partially Calibrated (Boresight)**: ±1 dB at boresight, ±1-2 dB loss
- **Partially Calibrated (Limited Coverage)**: ±1-1.5 dB in-coverage, ±2-3 dB extrapolated
- **Uncalibrated**: ±3-5 dB absolute gain, ±2-3 dB loss (design specs only)
  - Physical feed-spillover efficiency is now folded into the returned gain on this path
    (reported per-response as `metadata.spillover_loss_db`, dB and negative; applied only for
    small-offset/standard-physical-optics queries). After the 2026-07-10 feed-taper fix
    (q≈1.1–3.1) this correction is material (~0.8 dB for the enabled design-spec antennas —
    see `docs/domain-contract.md`, "Magnitude reality"); the ±3-5 dB accuracy above remains
    limited by design-spec parameter uncertainty (q-factor, surface RMS) and by unmodeled
    blockage/cross-pol — it is not calibrated-grade.

All responses include a `calibration_status` field with accuracy estimates.

**Off-axis (sidelobe) caveat — off-axis gain is now numerically correct, but idealised:**
the accuracy figures above apply to the **main beam and first sidelobe only**. Off-axis gain
is now **numerically converged**: roadmap unit **P10 landed 2026-07-15**, replacing the
aliasing fixed-density quadrature with a Hankel / azimuthal-mode integrator that computes the
physical-optics pattern correctly at all angles (the old code aliased the rapidly-varying
`2π·(D/λ)·sinθ` phase, reporting off-axis gain 20–35 dB too high — e.g. a 34 m dish at +34 dBi
at 90° off-boresight — and gain that *rose* with angle; that P0 defect is fixed). The served
off-axis value is the **raw physical optics** prediction: per maintainer decision D-2 the F7
statistical sidelobe floor is intentionally **OFF** on this path.

The remaining caveat is **physical, not numerical**: idealised physical optics omits
blockage, feed/strut scatter, and aperture-edge diffraction, so far-off-axis sidelobe
*levels* are **optimistic and not calibrated-grade** — the pattern shape is validated, the
absolute levels are not. For sidelobe, interference, adjacent-satellite, or off-axis-EIRP
analysis, use calibration data or a regulatory envelope (e.g. the ITU-R S.580 mask) instead
of the raw off-axis levels.

Queries on **uncalibrated** antennas beyond ~3× the first-null angle (≈ 1.6·λ/D,
beamwidth-relative) return a warning on all four compute endpoints ("… beyond the validated
main-beam region … not calibrated-grade …") stating this physical caveat. The F7
sidelobe-floor redesign (unblocked by P10, redesign pending) would address the absolute
off-axis levels separately — see `docs/domain-contract.md`, "Off-axis pattern / sidelobe
fidelity".

**Rear-hemisphere caveat — no physical validity behind the reflector (θ > 90°):** queries
more than **90° off boresight** return a separate, harder warning on **every** antenna —
**including fully calibrated ones** (a correction surface fitted from forward-hemisphere
measurements says nothing about back lobes). The aperture-integration model is a
forward-radiating formulation with no Huygens obliquity factor; behind the reflector the
returned value is a **numerical extrapolation of an idealised, unshadowed aperture field, not
a prediction**. Real rear-hemisphere levels are set by feed spillover past the rim,
aperture-edge diffraction, and mesh leakage — none of which are modeled. The value is still
returned (grid totality on `/heatmap` and `/h3-heatmap` is preserved) but must be replaced by
measured data or a regulatory rear-lobe envelope for any rear-hemisphere analysis. The
warning message is constant per (antenna, frequency), so heatmap/H3 aggregation deduplicates
it to a single entry.

**Large-feed-offset caveat — ray-tracing stub (> 0.5·f):** when the feed is aimed far enough
from the reflector boresight that the resulting feed displacement exceeds **0.5·f**, gain is
computed by an acknowledged **ray-tracing stub** (`model/ray_trace.rs`) that samples the
aperture but does not model true spillover or geometric ray–reflector intersection. Real ray
tracing is gated as feature **F2** and is not implemented. Such requests are **not rejected**
(warn-don't-refuse; `/heatmap` and `/h3-heatmap` grid totality is preserved) but every result
carries a degraded-accuracy warning (`…ray tracing … not fully implemented; gain accuracy may
be degraded`). The warning appears on **all four compute endpoints** — on `/h3-heatmap` it is
re-emitted at the service layer so it also survives gain-cache hits — and is constant per
antenna config, so heatmap/H3 aggregation deduplicates it to a single entry. See
`docs/domain-contract.md`, "Large feed offsets (> 0.5·f): ray-tracing stub".

### Coordinate System Auto-Detection

3D positions automatically detect coordinate system:

- **ECEF**: If `|x| > 6400e3 OR |y| > 6400e3 OR |z| > 6400e3` (meters)
- **Geodetic**: Otherwise (longitude degrees, latitude degrees, altitude meters)

### Multi-Feed Support

Each antenna can have multiple feeds with independent calibrations. Use composite `(antenna_id, feed_id)` identifiers for all queries.

## Example Usage

### cURL Example: Gain Computation (Geodetic)

```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "feed_id": "x_band_feed",
    "vehicle_position": {"x": -118.1234, "y": 34.5678, "z": 100.0},
    "vehicle_attitude": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
    "reflector_boresight": {"x": -117.0, "y": 35.0, "z": 400000.0},
    "feed_position": {"x": -118.124, "y": 34.568, "z": 105.0},
    "emitter_position": {"x": -117.0, "y": 35.0, "z": 400000.0},
    "frequency_mhz": 8400.0,
    "include_reference": true
  }'
```

### Python Example: List Antennas

```python
import requests

response = requests.get("http://localhost:3000/api/v1/antennas")
antennas = response.json()

for antenna in antennas["antennas"]:
    print(f"{antenna['antenna_id']}: {antenna['name']}")
    print(f"  Feeds: {', '.join(antenna['feeds'])}")
    if "calibration_status" in antenna:
        print(f"  Status: {antenna['calibration_status']}")
```

### JavaScript Example: Heatmap Generation

```javascript
const response = await fetch('http://localhost:3000/api/v1/heatmap', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    antenna_id: "antenna_1",
    feed_id: "x_band_feed",
    vehicle_position: { x: 4510731.123, y: 4510731.456, z: 3488865.789 },
    vehicle_attitude: { w: 1.0, x: 0.0, y: 0.0, z: 0.0 },
    reflector_boresight: { x: 4510732.0, y: 4510732.0, z: 3488950.0 },
    feed_position: { x: 4510731.5, y: 4510731.5, z: 3488870.0 },
    frequency_mhz: 8400.0,
    grid_config: {
      grid_type: "rectangular",
      azimuth_range_deg: { min: 0.0, max: 360.0, step: 5.0 },
      elevation_range_deg: { min: 0.0, max: 90.0, step: 2.0 }
    }
  })
});

const heatmap = await response.json();
console.log(`Peak gain: ${heatmap.metadata.peak_gain_db} dB`);
console.log(`Grid size: ${heatmap.metadata.points_evaluated} points`);
```

## Response Format

All successful gain computation responses include:

- **gain_db**: Computed antenna gain (dB)
- **reference_gain_db**: Optional reference gain for ideal case
- **loss_db**: Optional loss (reference - actual)
- **geometry**: Computed geometric parameters (feed offset, emitter direction)
- **warnings**: Array of warning messages
- **metadata**: Computation metadata (timing, extrapolation flag)
- **calibration_status**: Calibration status with accuracy estimates

Example response:

```json
{
  "antenna_id": "antenna_2",
  "feed_id": "x_band_feed",
  "gain_db": 41.2,
  "reference_gain_db": 43.5,
  "loss_db": 2.3,
  "geometry": {
    "feed_offset_meters": { "x": 0.05, "y": 0.02, "z": 0.01 },
    "emitter_azimuth_deg": 185.5,
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
      "azimuth_range_deg": [0.0, 0.0],
      "elevation_range_deg": [0.0, 0.0],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 25,
      "is_boresight_only": true
    },
    "correction_applied": false,
    "parameters_source": "boresight_tuning"
  }
}
```

## Error Handling

The API uses standard HTTP status codes:

- **200**: Success
- **400**: Invalid request parameters (validation error, invalid coordinates/attitude)
- **404**: Antenna or feed not found
- **500**: Internal server error (computation error, coordinate transform failure)
- **503**: Service unavailable (startup, shutdown)

Error responses follow a consistent format:

```json
{
  "error": "AntennaNotFound",
  "message": "Antenna 'invalid_antenna' not found",
  "details": {
    "antenna_id": "invalid_antenna"
  }
}
```

## Validation Rules

### Request Validation

- **Frequency**: 100-50000 MHz
- **ECEF Coordinates**: |x|, |y|, |z| < 10,000 km
- **Geodetic Coordinates**: lon: -180 to 180°, lat: -90 to 90°, alt < 1,000 km
- **Quaternion**: Must be normalized (|q| ≈ 1.0, tolerance 0.01)
- **Euler Angles**: |angle| < 360 degrees
- **Batch Size**: Maximum 1000 evaluations
- **Heatmap Grid**: Maximum 100,000 points

## Performance

Target performance metrics:

- **Single Evaluation**: 50-100ms p95 latency (includes coordinate transforms)
- **Batch Throughput**: 1-20 requests/second per instance
- **Heatmap Generation**: <2 seconds for 3312-point grid (73×46)
- **Coordinate Transform**: <10ms overhead per request

## Additional Resources

- **Architecture Documentation**: `docs/architecture.md`
- **Design Document**: `docs/antenna-model-design-doc.md`
- **Implementation Plan**: `docs/implementation-plan.md`
- **Partial Calibration Design**: `docs/partial-calibration-design.md`

## Changelog

### Version 1.1.0 (Current)

- Added support for multiple calibration statuses (fully/partially/uncalibrated)
- Added `calibration_status` field to all gain computation responses
- Added support for uncalibrated antennas (design specs only)
- Added multi-feed support with composite identifiers
- Updated all endpoints to include calibration status information
- 100% backward compatibility maintained

### Version 1.0.0

- Initial API release
- Core gain computation endpoints
- Batch processing and heatmap generation
- ECEF and Geodetic coordinate support
- Multi-feed antenna support

## Support

For issues, questions, or feature requests, please contact the Antenna Model Service team or file an issue in the repository.
