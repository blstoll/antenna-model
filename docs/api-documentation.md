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
- `POST /api/v1/h3-heatmap` - Per-cell link budget over an H3 hexagonal grid on the
  Earth's surface (gain, path loss, optional G/T)

### Antenna Information

- `GET /api/v1/antennas` - List all available antennas with feeds
- `GET /api/v1/antennas/{id}` - Get detailed antenna configuration
- `GET /api/v1/antennas/{id}/feeds` - List feeds for specific antenna
- `GET /api/v1/antennas/{id}/feeds/{feed_id}` - Get detailed feed configuration

## Service Lifecycle (roadmap S5)

### Startup

1. Configuration loads, logging initializes.
2. The calibration repository loads from `calibration.antenna_config_file`.
3. On success: `/status` is populated with the loaded antenna IDs and readiness flips to
   **true**.
4. On failure, behavior depends on `calibration.fail_fast`:
   - `true` (the shipped default): the process logs the error and **exits nonzero**. It does
     not start a useless server.
   - `false`: the server starts **degraded** — readiness stays false, `/health` reports
     `"degraded"`, `/status` reports `antenna_count: 0` with an empty `antenna_ids`, and gain
     requests return 404.

Readiness is false from process start until step 3 completes, so a readiness probe never
routes traffic to an instance that cannot serve it.

### Shutdown

On `SIGTERM` or `SIGINT`:

1. Readiness flips to **false** immediately — `/ready` returns 503.
2. The service keeps serving for `server.shutdown_readiness_delay_secs`, giving load
   balancers a window to observe the flip and stop sending new requests.
3. New connections stop being accepted; in-flight requests drain for at most
   `server.shutdown_timeout_secs`.
4. Cleanup runs (final status log, resource release).

`/health` stays 200 throughout the drain — the instance is alive, just no longer accepting
new work.

### Configuration

| Knob | Default | Notes |
|---|---|---|
| `calibration.fail_fast` | `true` | Abort startup if the calibration load fails. |
| `server.shutdown_readiness_delay_secs` | `0` | Grace window after the readiness flip. Recommended `5` in Kubernetes. |
| `server.shutdown_timeout_secs` | `25` | Bounded drain. Keep `delay + timeout` **strictly** under the pod's `terminationGracePeriodSeconds` (30 in the shipped chart) or the drain is SIGKILLed and cleanup is skipped. |

`delay + timeout` is the worst-case time from `SIGTERM` to the start of cleanup, so it must
leave the grace period room for cleanup to finish. The shipped defaults (`0` + `25`) leave
5 s. The recommended Kubernetes pairing is **`shutdown_readiness_delay_secs: 5` with
`shutdown_timeout_secs: 20`** (5 + 20 = 25 < 30); pairing the recommended delay with the
default timeout gives 5 + 25 = 30, which lands exactly on the grace period and leaves
cleanup no headroom before `SIGKILL`.

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
at 90° off-boresight — and gain that *rose* with angle; that P0 defect is fixed). On
**uncalibrated** antennas (no correction surface — `physics_is_uncorrected()`), the served
off-axis value is now the **incoherent power sum** of that idealised physical-optics term and
the statistical Ruze sidelobe floor (F7 redesign, landed 2026-07-16): `10·log₁₀(10^(PO/10) +
10^(floor/10))`, where the floor is a best-estimate **median** wide-angle level tracking
measured earth-station statistics (NTIA 84-164), not a precise per-antenna prediction.
Calibrated antennas (with a correction surface) do not get the floor — the double-counting
gate keeps their served behavior unchanged.

The remaining caveat is **physical, not numerical**: idealised physical optics omits
blockage, feed/strut scatter, and aperture-edge diffraction, and the statistical floor is a
population median, not a per-antenna measurement — so far-off-axis sidelobe *levels* are
**optimistic/approximate and not calibrated-grade** — the pattern shape is validated, the
absolute levels are not. For sidelobe, interference, adjacent-satellite, or off-axis-EIRP
analysis, use calibration data or a regulatory envelope (e.g. the ITU-R S.580 mask) instead
of the off-axis levels returned here.

Queries on **uncalibrated** antennas beyond ~3× the first-null angle (≈ 1.6·λ/D,
beamwidth-relative) return a warning on all four compute endpoints ("… beyond the validated
main-beam region … not calibrated-grade …") stating this physical caveat and describing the
power-sum combination — see `docs/domain-contract.md`, "Off-axis pattern / sidelobe
fidelity".

**Rear-hemisphere caveat — no physical validity behind the reflector (θ > 90°):** queries
more than **90° off boresight** return a separate, harder warning on **every** antenna —
**including fully calibrated ones** (a correction surface fitted from forward-hemisphere
measurements says nothing about back lobes). The far-field conversion now carries the Huygens
obliquity factor `(1+cosθ)/2` (F7, 2026-07-16), which suppresses the old fictitious rear
backlobe by ~33 dB at 163° — but the aperture-integration model still has no physical
validity behind the reflector. On **uncalibrated** antennas the rear aperture integration is
skipped entirely and the returned value **is the statistical sidelobe floor only** (the
physical-optics term is excluded there). On antennas **with a correction surface**, the
returned value remains a **numerical extrapolation of an idealised, unshadowed aperture
field, not a prediction**. Real rear-hemisphere levels — for either case — are set by feed
spillover past the rim, aperture-edge diffraction, and mesh leakage — none of which are
modeled. The value is still returned (grid totality on `/heatmap` and `/h3-heatmap` is
preserved) but must be replaced by measured data or a regulatory rear-lobe envelope for any
rear-hemisphere analysis. The warning message is constant per (antenna, frequency), so
heatmap/H3 aggregation deduplicates it to a single entry.

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

### Coordinate Systems

Every 3D position carries a **required** `coordinate_system` field naming its frame:

- **`"ecef"`**: `x`, `y`, `z` in meters from Earth's centre
- **`"geodetic"`**: `x` = longitude degrees, `y` = latitude degrees, `z` = altitude meters
  above the WGS84 ellipsoid

```json
{"x": -118.1234, "y": 34.5678, "z": 100.0, "coordinate_system": "geodetic"}
```

The frame is never inferred. A position that omits the field is a request-body parse
failure — **400 `invalid_request_body`**, with the message naming `coordinate_system`.

Range validation follows the frame you declare, so the same three numbers can be a valid
ECEF point and an invalid geodetic one (`x: 6500000` is fine as a metre offset and out of
range as a longitude).

**Breaking change, 2026-07-27.** The field was previously optional, and the frame was
inferred from coordinate magnitude — ECEF when any component exceeded 6400 km. That could
not distinguish a geodetic GEO satellite (altitude ~35,786 km) from an ECEF point, so an
untagged GEO position was read as a near-Earth-centre ECEF one and answered with a
confidently wrong gain under HTTP 200. There is no compatibility shim: tag your positions.

### Multi-Feed Support

Each antenna can have multiple feeds with independent calibrations. Use composite `(antenna_id, feed_id)` identifiers for all queries.

### H3 Link Budget Grid

`POST /api/v1/h3-heatmap` returns a per-cell link budget over an
[H3](https://h3geo.org) hexagonal grid laid on the Earth's surface — gain, free-space
path loss, total path loss, and optionally G/T for every cell.

**Grid placement and size.** The grid is centred on the H3 cell containing
`feed_pointing_location` — the Earth location the beam is *aimed at*, not the feed's physical
location on the antenna (see `docs/domain-contract.md`) — and extends `n_rings` rings
outward, giving `1 + 3·n_rings·(n_rings + 1)` cells:

| `n_rings` | 0 | 1 | 2 | 3 | 5 | 10 (max) |
|---|---|---|---|---|---|---|
| cells | 1 | 7 | 19 | 37 | 91 | 331 |

**Resolution.** Supply `h3_resolution` (0-15) to fix the cell size, or omit it and the
service derives one from `frequency_mhz`:

| Frequency | Resolution |
|---|---|
| < 2000 MHz | 6 |
| ≥ 2000 and < 8000 MHz | 7 |
| ≥ 8000 and < 20000 MHz | 8 |
| ≥ 20000 MHz | 9 |

Either way, the resolution actually used is echoed back as `h3_resolution` in the
response.

**Reading the per-cell numbers.** Two fields have references worth reading before use:

- **`loss_db` is relative to the grid peak**, not to the grid centre cell. It is
  `metadata.peak_gain_db − gain_db`, where `peak_gain_db` is the highest gain over the
  cells actually evaluated — the same rule `/api/v1/heatmap` applies, so the field means
  one thing on both heatmap endpoints. It is therefore never negative, is exactly `0.0` at
  the peak cell, and can be re-derived from the response's own numbers.

  The peak of the *grid* is not necessarily the peak of the *beam*: the grid is centred on
  `feed_pointing_location`, and if the beam peak falls outside the rings you requested, every cell's
  loss is understated by the difference. Widen `n_rings` if you need the true peak in view.
  (`/api/v1/heatmap` carries the same caveat.)

  The reference is one of the cells, so both sides of the subtraction share a basis by
  construction. A grid can still straddle two bases where it leaves calibration coverage —
  in-coverage cells corrected, out-of-coverage cells physics-only — as on `/api/v1/heatmap`.
- **`total_path_loss_db` is `free_space_path_loss_db + loss_db`**. Because `loss_db` is
  peak-referenced and non-negative, this never falls below the free-space value.

If *no* cell evaluates successfully, `cells` is empty, `metadata.failed_points` equals
`metadata.points_evaluated`, and `metadata.peak_gain_db` reports the sentinel `-999999.0` —
there is no peak to reference, and the field is never `null`.

`g_over_t_db` is `gain_db − 10·log₁₀(temperature_k)` and appears only when the request
supplies `temperature_k`. That temperature is a pure passthrough — the service models no
antenna noise temperature of its own.

**Warnings.** Cell warnings are deduplicated into one response-level `warnings` array, so
a warning that fires for many cells appears once. The set does not depend on how much of
the request the internal gain cache could serve: repeating an identical request returns an
identical `warnings` array. One difference from `/api/v1/gain` remains — the
calibration-status warning is not emitted here, because the response carries the
structured `calibration_status` object instead.

**Cost.** This is a compute-heavy endpoint: it runs one aperture integration per cell, so
it shares the admission-control limit and the per-integration compute budget with
`/api/v1/gain/batch` and `/api/v1/heatmap`.

## Example Usage

### cURL Example: Gain Computation (Geodetic)

<!-- api-example: GainRequest -->
```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "antenna_1",
    "feed_id": "x_band_feed",
    "vehicle_position": {"x": -118.1234, "y": 34.5678, "z": 100.0, "coordinate_system": "geodetic"},
    "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
    "reflector_boresight": {"x": -117.0, "y": 35.0, "z": 400000.0, "coordinate_system": "geodetic"},
    "feed_pointing_location": {"x": -118.124, "y": 34.568, "z": 105.0, "coordinate_system": "geodetic"},
    "emitter_position": {"x": -117.0, "y": 35.0, "z": 400000.0, "coordinate_system": "geodetic"},
    "frequency_mhz": 8400.0,
    "include_reference": true
  }'
```

### cURL Example: H3 Link Budget

A 3.7 m ground station at Goldstone aimed at a point ~40 km east, with two rings of
resolution-7 cells (19 cells) around the aim point and a 150 K system temperature. The
antenna and feed are the ones shipped in `calibration_data/antennas.yaml`, so this runs
against a default local service; the body is also checked in as
`examples/requests/h3_link_budget_request.json`.

<!-- api-example: H3LinkBudgetRequest -->
```bash
curl -X POST http://localhost:3000/api/v1/h3-heatmap \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "gs_3.7m_uncalibrated",
    "feed_id": "s_band_feed",
    "vehicle_position": {"x": -116.889, "y": 35.4267, "z": 1036.0, "coordinate_system": "geodetic"},
    "reflector_boresight": {"x": -116.45, "y": 35.4267, "z": 800.0, "coordinate_system": "geodetic"},
    "feed_pointing_location": {"x": -116.45, "y": 35.4267, "z": 800.0, "coordinate_system": "geodetic"},
    "frequency_mhz": 2200.0,
    "n_rings": 2,
    "h3_resolution": 7,
    "temperature_k": 150.0
  }'
```

Response (19 cells; the first two are shown. Note that the grid **centre** cell is not the
peak — the second cell shown is, at `loss_db: 0.0`, and the centre cell is 5.43 dB down
from it. `loss_db` is referenced to that peak, so it is non-negative everywhere):

<!-- api-example: H3LinkBudgetResponse -->
```json
{
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "s_band_feed",
  "frequency_mhz": 2200.0,
  "center_cell_id": "87298484bffffff",
  "h3_resolution": 7,
  "cells": [
    {
      "cell_id": "87298484bffffff",
      "center_lon": -116.44486581522226,
      "center_lat": 35.43632856760593,
      "azimuth_deg": 17.813005656019303,
      "elevation_deg": 1.8951310668765187,
      "distance_km": 40.36081496141387,
      "gain_db": 29.176592765956944,
      "loss_db": 5.431935877144003,
      "free_space_path_loss_db": 131.41543537598358,
      "total_path_loss_db": 136.8473712531276,
      "g_over_t_db": 7.41568017540013
    },
    {
      "cell_id": "87298484affffff",
      "center_lon": -116.42008010656208,
      "center_lat": 35.425316200276974,
      "azimuth_deg": 314.0655073262443,
      "elevation_deg": 1.084785862275493,
      "distance_km": 42.60000836115257,
      "gain_db": 34.60852864310095,
      "loss_db": 0.0,
      "free_space_path_loss_db": 131.88443052517158,
      "total_path_loss_db": 131.88443052517158,
      "g_over_t_db": 12.847616052544133
    }
  ],
  "warnings": ["Estimated spillover 21.2% may reduce aperture efficiency."],
  "metadata": {
    "points_evaluated": 19,
    "computation_time_ms": 1.214708,
    "peak_gain_db": 34.60852864310095,
    "failed_points": 0
  },
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 3.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_applied": false,
    "parameters_source": "design_specifications"
  }
}
```

Gain values from an uncalibrated antenna carry the ±3 dB accuracy stated in
`calibration_status`; the numbers above are illustrative of the shape, not a
specification.

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

<!-- api-example: HeatmapRequest -->
```javascript
const response = await fetch('http://localhost:3000/api/v1/heatmap', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    "antenna_id": "antenna_1",
    "feed_id": "x_band_feed",
    "vehicle_position": { "x": 4510731.123, "y": 4510731.456, "z": 3488865.789, "coordinate_system": "ecef" },
    "vehicle_attitude": [1.0, 0.0, 0.0, 0.0],
    "reflector_boresight": { "x": 4510732.0, "y": 4510732.0, "z": 3488950.0, "coordinate_system": "ecef" },
    "feed_pointing_location": { "x": 4510731.5, "y": 4510731.5, "z": 3488870.0, "coordinate_system": "ecef" },
    "frequency_mhz": 8400.0,
    "grid_config": {
      "grid_type": "rectangular",
      "azimuth_range_deg": { "min": 0.0, "max": 360.0, "step": 5.0 },
      "elevation_range_deg": { "min": 0.0, "max": 90.0, "step": 2.0 }
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

<!-- api-example: GainResponse -->
```json
{
  "antenna_id": "antenna_2",
  "feed_id": "x_band_feed",
  "gain_db": 41.2,
  "reference_gain_db": 43.5,
  "loss_db": 2.3,
  "geometry": {
    "physical_feed_offset_m": { "x": 0.05, "y": 0.02, "z": 0.01 },
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
- **400**: The request body could not be read or parsed. Nothing else returns 400
- **404**: The request names an antenna or feed that does not exist
- **422**: The body parsed but is semantically invalid — a value out of range, a
  degenerate geometry, a batch that is empty or oversized
- **504**: A server-side wall-clock budget was exceeded — either the whole request
  (`request_timeout`, `server.request_timeout_secs`) or a single aperture integration
  (`computation_budget_exceeded`, `performance.integration_budget_ms`)
- **413**: Payload too large (request body exceeds the configured maximum size)
- **500**: Internal server error (computation error, coordinate transform failure)
- **503**: Service unavailable — either lifecycle (startup, shutdown) or admission
  control (`service_overloaded`: too many concurrent heavy requests, carries a
  `Retry-After` header — see below)

Error responses follow a consistent format:

<!-- api-example: ErrorResponse -->
```json
{
  "error": "antenna_not_found",
  "message": "Antenna 'invalid_antenna' not found",
  "field": "antenna_id"
}
```

Every error the service itself produces — from handlers and from middleware alike — is
served with `Content-Type: application/json` and the body shape above. A request body that
fails to parse is included: the framework's bare rejection is normalized to
`invalid_request_body`, preserving the parse location in `message`.

**One exception:** rejections the web framework raises before routing reaches the service
— `404` for a path that matches no route, `405` for a wrong method, `415` for an
unsupported `Content-Type` — are still served as framework-shaped `text/plain` with no
error code. Giving them JSON bodies requires error codes that do not exist yet, which is a
contract decision (roadmap unit C8) rather than a formatting one. Note this means a
`404` can arrive in either shape: `antenna_not_found` as JSON from the service, or bare
text from the router.

`error` is a stable machine-readable code — always `snake_case`, always drawn from the
table below. `message` is human-readable and **not** stable; do not parse it. `field` and
`details` are optional strings, omitted when absent.

### Error codes

This is the complete vocabulary. The service emits no other value in `error`. The set is
defined once in code (`api/schemas.rs`, `mod error_codes`) and referenced by every
emission site, so a code cannot be introduced by a typo.

| Code | Status | Meaning |
|---|---|---|
| `antenna_not_found` | 404 | The named antenna does not exist in the calibration repository. |
| `feed_not_found` | 404 | The antenna exists but the named feed does not. |
| `validation_error` | 422 | The request parsed but is semantically invalid. |
| `invalid_coordinate` | 422 | A position or coordinate value is out of range, or the positions are geometrically degenerate. |
| `invalid_request_body` | 400 | The request body could not be read or parsed. |
| `not_implemented` | 422 | A recognized but unimplemented option — currently only `/heatmap`'s H3 grid type. |
| `payload_too_large` | 413 | The body exceeds `server.max_body_size_bytes`. |
| `request_timeout` | 504 | The request exceeded `server.request_timeout_secs`. |
| `computation_budget_exceeded` | 504 | One aperture integration exceeded `performance.integration_budget_ms`. |
| `service_overloaded` | 503 | Admission control rejected the request; a `Retry-After` header accompanies it. |
| `internal_error` | 500 | An unexpected server-side failure. |

### Status policy

One rule, applied identically on `/api/v1/gain`, `/api/v1/gain/batch`,
`/api/v1/heatmap`, and `/api/v1/h3-heatmap`:

| The request… | Status |
|---|---|
| could not be parsed | **400** |
| parsed, but a value is unusable | **422** |
| names an antenna or feed that does not exist | **404** |

It does not matter which layer notices. A geometry that only the coordinate transform
can reject — a `reflector_boresight` coincident with `vehicle_position`, say — is still
a **422**, not a 400: the body was perfectly readable.

Two consequences worth stating outright:

- **A non-finite number in a request is a 400, not a 422.** JSON cannot encode `NaN`
  or `Infinity`; most serializers emit `null` instead, which does not deserialize into
  the numeric fields the schema declares. The request is genuinely unparseable.
- **`/api/v1/gain/batch` rejects the whole batch.** Every item is validated before any
  physics runs, and the first failure rejects the request with the offending index in
  `field` (for example `"evaluations[3]"`). Per-item degradation survives only for
  *compute*-class failures — an integration over budget, or one that does not converge —
  which cannot be predicted in advance.

**Changed in this release (roadmap unit C2).** Previously the same input could get
different answers from different endpoints: an unknown antenna was **422** on `/gain`
and `/heatmap` but **404** on `/h3-heatmap`; a service-layer `invalid_coordinate` was
**400**; a batch-level violation was **400**; and `/gain/batch` rejected nothing at all,
returning **200** with `"gain_db": null` per bad item and the reason in that item's
`warnings` — so a client checking the status code saw success. An empty `evaluations`
array likewise returned **200** with zero results and is now **422**. The `error` codes
themselves did not change.

### Request Body Size Limit

Every request body is capped by the configured `server.max_body_size_bytes`
(default **10 MB**), and the cap holds regardless of how the client frames the
body:

- **With `content-length`** (the common case): a request whose declared size
  exceeds the limit is rejected up front with **413 Payload Too Large** and the
  standard JSON error body, before the body is read at all.
- **Without `content-length`** (`Transfer-Encoding: chunked`): the size is not
  knowable in advance, so the body is read under a hard cap and the request is
  rejected with the same **413** as soon as the accumulated bytes exceed the
  limit. The message omits the byte count, which is unknown on this path.

Bodyless `GET` requests are unaffected — the service deliberately does *not*
answer `411 Length Required` for a missing `content-length`.

The default comfortably accommodates a maximum-size (1000-item) batch request
(~0.6 MB). Operators can raise or lower the cap via configuration.

<!-- api-example: ErrorResponse -->
```json
{
  "error": "payload_too_large",
  "message": "Request body of 12000000 bytes exceeds the maximum of 10485760 bytes"
}
```

### Request Timeout

The configured `server.request_timeout_secs` (default **30 s**) bounds **every
compute endpoint** — `/api/v1/gain`, `/api/v1/gain/batch`, `/api/v1/heatmap`, and
`/api/v1/h3-heatmap`. Their synchronous rayon work is offloaded to a blocking
thread pool so the async task yields and the timeout can fire promptly instead
of blocking a server worker thread; if the deadline is exceeded the request is
abandoned and the client receives **504 Gateway Timeout** with the standard JSON
error body.

`/api/v1/gain` targets the <100 ms path, so in practice it never approaches the
deadline — but it is offloaded on the same terms as the others (roadmap unit
S2b), because a CPU-bound future that never yields cannot be preempted by the
timeout middleware at all. Before S2b this endpoint ran its physics inline and
a slow single evaluation returned a *late 200* rather than a 504, leaving
`request_timeout_secs` unenforceable on the service's primary endpoint. Two
bounds now apply to it, and they fire independently: `request_timeout` when the
whole request exceeds the deadline, and **`computation_budget_exceeded`** when
one aperture integration exceeds `performance.integration_budget_ms` (roadmap
unit S3, documented below).

The status is **504 (a 5xx)**, not 408: the deadline is a *server-side* budget
(the client sent a valid request; the server exceeded its own processing limit),
so the fault belongs on the server side. It is deliberately not **503 +
Retry-After** — the failure is deterministic in the request payload (the same
heavy grid re-costs the same), so no honest retry delay exists; the remedy is a
smaller request. The machine `error` code stays `request_timeout`. (Admission-
control/overload rejection, which *is* transient, uses 503 + Retry-After — see
**Admission Control** below.)

<!-- api-example: ErrorResponse -->
```json
{
  "error": "request_timeout",
  "message": "Request processing exceeded the configured timeout of 30000 ms"
}
```

**Honest limitation — the response is bounded, the compute is not.** When the
request timeout fires, the server stops waiting and returns 504, but the
background rayon computation already running on the blocking pool is **not
cancelled**: it continues to completion, consuming CPU. Dropping the future does
not stop the pool. Cooperative, wall-clock-bounded compute cancellation at the
level of a single integration is roadmap unit S3, documented next.

### Per-Integration Compute Budget

The configured `performance.integration_budget_ms` (default **30 000 ms**) bounds
a **single aperture integration** — the innermost hot loop of the physics model.
When one integral's radial loop runs past the budget it aborts *cooperatively*
(the deadline is polled at radial chunk boundaries, never per sample, so results
are byte-identical when the budget is not hit) with **504 Gateway Timeout** and
the machine `error` code **`computation_budget_exceeded`** — distinct from the
request timeout's `request_timeout` so operators can tell "the middleware gave up
waiting" from "a single integral was aborted."

<!-- api-example: ErrorResponse -->
```json
{
  "error": "computation_budget_exceeded",
  "message": "computation exceeded time budget in azimuthal_mode_field: 31000 ms > 30000 ms budget"
}
```

Like the request timeout, the overrun is deterministic in the request payload, so
504 (not 503 + Retry-After) is used and the remedy is a smaller request.

**Honest limitation — per integration, not per request.** The budget caps *each*
integral, not the whole request. A single `/api/v1/gain` evaluation runs two
integrations (the off-axis pattern plus its boresight normalization anchor), each
getting a fresh budget. A `/api/v1/heatmap` or `/api/v1/gain/batch` fans out to
many points, each with its own budgeted integrations — an over-budget point fails
just that point (heatmap) or item (batch) rather than the whole request. So the
three limits compose but do not subsume one another: `integration_budget_ms`
caps one integral, `request_timeout_secs` caps the request wall-clock, and
admission control (below) caps how many heavy requests run at once. A huge
heatmap can still spend `budget × points` of background CPU after a
request-timeout 504.

### Admission Control (concurrency limit)

The configured `performance.max_concurrent_heavy_requests` caps how many
**compute-heavy** requests — `/api/v1/gain/batch`, `/api/v1/heatmap`, and
`/api/v1/h3-heatmap` — may execute concurrently, sharing **one** budget across all
three endpoints. When the cap is reached, a further heavy request is **rejected
immediately** (never queued) with **503 Service Unavailable**, the standard JSON
error body (`service_overloaded`), and a **`Retry-After`** header
(`performance.admission_retry_after_secs`, default **5 s**).

<!-- api-example: ErrorResponse -->
```json
{
  "error": "service_overloaded",
  "message": "Server is at its concurrent heavy-request limit (8); retry after 5 s"
}
```

Unlike the request timeout and per-integration budget (both **504**, both
deterministic in the payload, both without `Retry-After`), overload is genuinely
**transient** — a slot frees the instant an in-flight heavy request finishes — so
**503 + `Retry-After`** is the honest response, and retrying the *same* request
after the delay can succeed.

The cheap endpoints — single `/api/v1/gain`, the `/health` / `/ready` / `/status`
probes, and the antenna/feed listings — are **never** admission-limited.

**Default: disabled.** `max_concurrent_heavy_requests` defaults to **0
(unlimited)**, so admission control is off unless an operator sets a positive limit
(a small multiple of `worker_threads` / CPU count is a sane starting point). The
related `performance.worker_threads` knob (default **0 = auto-detect**) sizes the
shared rayon pool that all heavy compute runs on; setting it applies the global
pool once at startup.

## Validation Rules

### Request Validation

- **Frequency**: 100-50000 MHz
- **ECEF Coordinates**: |x|, |y|, |z| < 10,000 km
- **Geodetic Coordinates**: lon: -180 to 180°, lat: -90 to 90°, alt < 1,000 km
- **Quaternion**: Must be normalized (|q| ≈ 1.0, tolerance 0.01)
- **Euler Angles**: |angle| < 360 degrees
- **Batch Size**: Maximum 1000 evaluations
- **Heatmap Grid**: Maximum 100,000 points

### H3 Link Budget Request

The H3 link-budget request (`H3LinkBudgetRequest` schema) validates the same fields as the
gain/heatmap endpoints, plus its H3-specific fields:

- **`frequency_mhz`**: 100-50000 MHz (required).
- **`pointing_frequency_mhz`** (optional): when supplied, 100-50000 MHz — same range as
  `frequency_mhz`.
- **`temperature_k`** (optional): when supplied, must be finite, strictly greater than 0, and at
  most 10000 K (it feeds a `log10` in the G/T computation, so non-positive values are rejected up
  front rather than producing a NaN `g_over_t_db`).
- **`h3_resolution`** (optional): when supplied, 0-15 (validated in the request layer, not deep in
  the H3 grid builder).
- **`n_rings`**: 0-10.

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
