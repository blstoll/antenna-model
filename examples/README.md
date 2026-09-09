# API Examples

This directory contains example requests and responses for all Antenna Model Service API endpoints.

## Directory Structure

```
examples/
├── README.md                    # This file
├── curl-examples.sh             # All curl examples in a single executable script
├── requests/                    # JSON request body examples
│   ├── gain_request.json
│   ├── gain_request_geodetic.json
│   ├── batch_request.json
│   └── heatmap_request.json
└── responses/                   # Example responses
    ├── gain_response.json
    ├── batch_response.json
    ├── heatmap_response.json
    ├── antenna_list_response.json
    └── antenna_details_response.json
```

## Quick Start

### Start the Service

```bash
# Run the service locally (default: http://localhost:3000)
cargo run --release --bin antenna-model
```

### Run All Examples

```bash
# Execute all curl examples
bash examples/curl-examples.sh
```

## Available Endpoints

### Health & Status Endpoints

#### 1. GET /health - Liveness Probe
```bash
curl http://localhost:3000/health
```

#### 2. GET /ready - Readiness Probe
```bash
curl http://localhost:3000/ready
```

#### 3. GET /status - Service Status
```bash
curl http://localhost:3000/status
```

### Gain Computation Endpoints

#### 4. POST /api/v1/gain - Single Gain Computation

```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d @examples/requests/gain_request.json
```

DSN 70 m, X-band, emitter on boresight: `gain_db` ≈ **74.3 dBi**, which you can
check against the aperture directly — `10·log₁₀(0.65·(πD/λ)²)` = 74.0 dBi for
D = 70 m at 8450 MHz. For the same endpoint in the geodetic frame use
`gain_request_geodetic.json`, which is a different antenna and geometry
(`gs_3.7m` at 8200 MHz, emitter well off boresight), not this request restated.

#### 5. POST /api/v1/gain/batch - Batch Gain Computation
```bash
curl -X POST http://localhost:3000/api/v1/gain/batch \
  -H "Content-Type: application/json" \
  -d @examples/requests/batch_request.json
```

The three items share one aim point and differ only in antenna/feed/frequency, so
the gain spread between them (≈ 31 / 57 / 23 dBi) is a property of the **feeds**,
not an error: `dsn_34m` `s_band` sits at the focal point and returns its peak,
while `dsn_34m` `x_band` and `gs_3.7m` `x_band_feed` are laterally offset by
design (0.15 m and 0.05 m), which squints each beam off the mechanical boresight
the request aims at. `loss_db` on each item reports exactly that.

`/api/v1/gain/batch` returns **HTTP 200 even when items fail** — read
`metadata.failure_count`, not the status code. (That is not hypothetical: this
file's every item failed under a 200 for two months. See roadmap D30.)

### Heatmap Endpoint

#### 6. POST /api/v1/heatmap - Generate Loss Heatmap
```bash
curl -X POST http://localhost:3000/api/v1/heatmap \
  -H "Content-Type: application/json" \
  -d @examples/requests/heatmap_request.json
```

### Antenna Information Endpoints

#### 7. GET /api/v1/antennas - List All Antennas
```bash
curl http://localhost:3000/api/v1/antennas
```

#### 8. GET /api/v1/antennas/:id - Get Antenna Details
```bash
curl http://localhost:3000/api/v1/antennas/dsn_34m_uncalibrated
```

#### 9. GET /api/v1/antennas/:id/feeds - List Antenna Feeds
```bash
curl http://localhost:3000/api/v1/antennas/dsn_34m_uncalibrated/feeds
```

#### 10. GET /api/v1/antennas/:id/feeds/:feed_id - Get Feed Details
```bash
curl http://localhost:3000/api/v1/antennas/dsn_34m_uncalibrated/feeds/x_band
```

## Request Examples

### ECEF Coordinates Example

The service supports ECEF (Earth-Centered Earth-Fixed) coordinates — tag them `"coordinate_system": "ecef"`:

```json
{
  "antenna_id": "dsn_34m_uncalibrated",
  "feed_id": "x_band",
  "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
  "emitter_position": {"x": 7000000.0, "y": 0.0, "z": 500000.0, "coordinate_system": "ecef"},
  "frequency_mhz": 8450.0
}
```

### Geodetic Coordinates Example

Geodetic coordinates (longitude, latitude in degrees; altitude in meters) are auto-detected:

```json
{
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "x_band_feed",
  "vehicle_position": {"x": -118.0, "y": 34.0, "z": 500.0, "coordinate_system": "geodetic"},
  "emitter_position": {"x": -100.0, "y": 35.0, "z": 500000.0, "coordinate_system": "geodetic"},
  "frequency_mhz": 8200.0
}
```

### Vehicle Attitude (Quaternion)

The `vehicle_attitude` field is an optional normalized quaternion, given as a
JSON array in `[w, x, y, z]` (w-first) order. The example below is the identity
rotation:

```json
"vehicle_attitude": [1.0, 0.0, 0.0, 0.0]
```

Only body **+X** is read from it: projected onto the plane perpendicular to
boresight, it becomes the azimuth-zero reference. So the identity is **not** a
safe default — against a boresight along ECEF +X it leaves body +X parallel to
boresight, the azimuth reference degenerate, and the request a **422**. That is
not hypothetical: it is what `gain_request.json` did for two months (roadmap
D30), and why that file now carries `[0.5, 0.5, 0.5, 0.5]`, which puts body +Z on
ECEF +X — its boresight — and body +X on ECEF +Y. Supply an attitude whose body
+Z matches your boresight, or **omit the field** and let the service derive
azimuth-zero from an Earth-Z/East cross product.

## Response Examples

All responses include:
- Request ID header (`X-Request-Id`)
- JSON content type
- Structured error responses on failure

See `responses/` directory for full example responses.

`gain_response.json`, `batch_response.json` and `heatmap_response.json` are
**captured verbatim from a running service** on the request file of the same
name — regenerate them the same way rather than hand-editing:

```bash
curl -s -X POST http://localhost:3000/api/v1/gain \
  -H 'Content-Type: application/json' \
  -d @examples/requests/gain_request.json | jq . > examples/responses/gain_response.json
```

They were hand-written until 2026-08-16, and it showed: `gain_response.json`
carried a gain of **−3,370,985 dB** and `heatmap_response.json` a smooth analytic
grid for an antenna that does not exist. Both parsed cleanly, which was all the
guard checked (roadmap D30). Two guards now cover them —
`example_responses_deserialize.rs` for shape, and
`example_execution_tests::every_response_example_matches_its_request_and_is_physically_possible`
for whether the response describes its own request and could have come from an
antenna. Neither pins exact values, so these do not need updating on every
physics change — only when they drift far enough to mislead.

## Calibration Status in API Responses

All gain computation endpoints return calibration status information (v2.0+). The `calibration_status` field provides accuracy estimates and indicates which calibration method was used.

### Fully Calibrated Response

For antennas with complete grid measurements and correction surface:

```json
{
  "antenna_id": "dsn_34m_fully_calibrated",
  "feed_id": "x_band",
  "gain_db": 45.3,
  "loss_db": 2.1,
  "reference_gain_db": 47.4,
  "calibration_status": {
    "status": "fully_calibrated",
    "accuracy_estimate_db": 1.0,
    "correction_application": "all",
    "correction_applied": true,
    "parameters_source": "measurement_tuned"
  },
  "warnings": [],
  "metadata": {
    "computation_time_ms": 2.8,
    "extrapolated": false
  }
}
```

**Key Fields:**
- `status`: "fully_calibrated" - highest accuracy level
- `accuracy_estimate_db`: 1.0 - expect ±1.0 dB accuracy
- `correction_application`: "all" - every successful direction used the B-spline correction surface
- `correction_applied`: true - compatibility boolean; true for `partial` or `all`
- `warnings`: Empty - no calibration warnings for fully calibrated antennas

### Partially Calibrated (Boresight) Response

For antennas calibrated with boresight measurements only:

```json
{
  "antenna_id": "gs_7.3m_boresight",
  "feed_id": "x_band",
  "gain_db": 44.8,
  "loss_db": 2.3,
  "reference_gain_db": 47.1,
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 1.5,
    "correction_application": "unavailable",
    "correction_applied": false,
    "parameters_source": "boresight_tuned",
    "coverage": {
      "azimuth_range_deg": [0.0, 360.0],
      "elevation_range_deg": [0.0, 0.01],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 15,
      "is_boresight_only": true
    }
  },
  "warnings": [
    {
      "code": "partially_calibrated",
      "message": "Antenna 'gs_7.3m_boresight' is partially calibrated. Accuracy estimate: ±1.5 dB"
    }
  ],
  "metadata": {
    "computation_time_ms": 1.5,
    "extrapolated": false
  }
}
```

**Key Fields:**
- `status`: "partially_calibrated" - limited coverage
- `accuracy_estimate_db`: 1.5 - ±1.5 dB at boresight
- `correction_application`: "unavailable" - no correction surface exists
- `correction_applied`: false - compatibility boolean
- `coverage.is_boresight_only`: true - measurements on the boresight axis. Coverage is an
  on-axis **cone** (`elevation ≤ 0.01°`, azimuth unconstrained), not the point `(0, 0)`:
  boresight is the pole of the (azimuth, polar-angle) system, where azimuth is degenerate.
- `warnings`: Informs about partial calibration limitation

### Partially Calibrated (Out-of-Coverage) Response

When query is outside the calibrated region:

```json
{
  "antenna_id": "gs_7.3m_boresight",
  "feed_id": "x_band",
  "gain_db": 42.1,
  "loss_db": 3.2,
  "calibration_status": {
    "status": "partially_calibrated",
    "accuracy_estimate_db": 2.5,
    "correction_application": "none",
    "correction_applied": false,
    "parameters_source": "boresight_tuned",
    "coverage": {
      "azimuth_range_deg": [0.0, 360.0],
      "elevation_range_deg": [0.0, 0.01],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 15,
      "is_boresight_only": true
    }
  },
  "warnings": [
    {
      "code": "partially_calibrated",
      "message": "Antenna 'gs_7.3m_boresight' is partially calibrated. Accuracy estimate: ±1.5 dB"
    },
    {
      "code": "out_of_coverage",
      "message": "Query is outside calibrated region - using physics model extrapolation"
    }
  ],
  "metadata": {
    "computation_time_ms": 1.8,
    "extrapolated": false
  }
}
```

**Key Observations:**
- `accuracy_estimate_db`: 2.5 - degraded to ±2-3 dB off-axis (physics extrapolation)
- Additional warning about extrapolation beyond calibrated region
- Physics model still valid, just less accurate than at boresight

### Uncalibrated Response

For antennas using design specifications only (no measurements):

```json
{
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "x_band_feed",
  "gain_db": 43.5,
  "loss_db": 2.5,
  "reference_gain_db": 46.0,
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 4.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_application": "unavailable",
    "correction_applied": false,
    "parameters_source": "design_specifications"
  },
  "warnings": [
    {
      "code": "uncalibrated",
      "message": "Antenna 'gs_3.7m_uncalibrated' is uncalibrated (using design specifications). Absolute gain accuracy: ±4.0 dB, Loss accuracy: ±2.0 dB"
    }
  ],
  "metadata": {
    "computation_time_ms": 1.2,
    "extrapolated": false
  }
}
```

**Key Fields:**
- `status`: "uncalibrated" - design specs only
- `accuracy_estimate_db`: 4.0 - ±3-5 dB absolute gain uncertainty
- `loss_accuracy_estimate_db`: 2.0 - **better accuracy for loss (±2 dB)** due to error cancellation
- `parameters_source`: "design_specifications"

For aggregate heatmap and H3 responses, `correction_application` is computed from
successful points/cells only. It is `partial` when correction was applied to some but not
all successful directions. Failed directions do not change the denominator. The legacy
`correction_applied` field remains `true` for both `partial` and `all`.

**Important:** For uncalibrated antennas, **use loss values** for comparative analysis. Loss accuracy (±2 dB) is significantly better than absolute gain accuracy (±4 dB) because systematic parameter errors cancel when comparing two pointing directions.

### Backward Compatibility (v1.x clients)

Older API clients will receive responses without the `calibration_status` field:

```json
{
  "antenna_id": "antenna_1",
  "gain_db": 45.2,
  "loss_db": 2.1,
  "warnings": []
}
```

**Compatibility Notes:**
- The `calibration_status` field is optional
- Omitted when not available (old calibration files)
- Forward compatible: new fields ignored by old parsers
- No breaking changes

### Using Calibration Status in Client Code

**Python Example:**
```python
import requests

response = requests.post('http://localhost:3000/api/v1/gain', json={
    "antenna_id": "gs_3.7m_uncalibrated",
    "feed_id": "x_band_feed",
    ...
})

data = response.json()

# Check if calibration status is available
if 'calibration_status' in data:
    status = data['calibration_status']['status']
    accuracy = data['calibration_status']['accuracy_estimate_db']

    print(f"Calibration: {status}, Accuracy: ±{accuracy} dB")

    # For uncalibrated antennas, prefer loss values
    if status == 'uncalibrated':
        loss_accuracy = data['calibration_status']['loss_accuracy_estimate_db']
        print(f"Loss accuracy (better): ±{loss_accuracy} dB")
        print(f"Use loss_db ({data['loss_db']}) for comparative analysis")

    # For partially calibrated, check if query is in coverage.
    # Branch on the warning `code`, never on `message` — codes are the stable
    # contract, messages may be reworded in any release.
    if status == 'partially_calibrated':
        codes = {w['code'] for w in data.get('warnings', [])}
        if 'out_of_coverage' in codes:
            print("Warning: Query outside calibrated region (degraded accuracy)")
else:
    print("Calibration status not available (old format or fully calibrated)")
```

### Accuracy Expectations Summary

| Calibration Status | Absolute Gain | Loss (Relative) | Recommended Use |
|-------------------|---------------|-----------------|-----------------|
| **Fully Calibrated** | ±1.0 dB | ±1.0 dB | All applications |
| **Partially (in-coverage)** | ±1.0-1.5 dB | ±1.0-1.5 dB | Boresight queries, parameter validation |
| **Partially (out-of-coverage)** | ±2-3 dB | ±2-3 dB | Physics extrapolation acceptable |
| **Uncalibrated** | ±3-5 dB | **±2 dB** | **Use loss for comparative analysis** |

## Testing with Different Configurations

### Custom Port

```bash
# If service runs on a different port
export API_BASE_URL=http://localhost:8080
bash examples/curl-examples.sh
```

### Pretty-print JSON responses

```bash
curl http://localhost:3000/api/v1/antennas | jq .
```

### Save response to file

```bash
curl http://localhost:3000/status -o status_response.json
```

## Error Handling

The API returns standard HTTP status codes:

- `200 OK` - Success
- `400 Bad Request` - Invalid input
- `404 Not Found` - Antenna/feed not found
- `422 Unprocessable Entity` - Validation error
- `500 Internal Server Error` - Server error
- `503 Service Unavailable` - Service not ready

Error responses follow this format:

```json
{
  "error": "validation_error",
  "message": "Detailed error message"
}
```

## Performance Notes

- Single gain computation: <100ms p95 latency
- Batch processing: 100 evaluations in <500ms
- Heatmap generation (72x46 grid): <2 seconds
- Parallel processing for batches ≥5 requests

## Further Documentation

- **API Design**: See `docs/antenna-model-design-doc.md`
- **Architecture**: See `docs/architecture.md`
- **Implementation Plan**: See `docs/implementation-plan.md`
- **Project Instructions**: See `CLAUDE.md`
