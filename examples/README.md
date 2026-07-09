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

#### 5. POST /api/v1/gain/batch - Batch Gain Computation
```bash
curl -X POST http://localhost:3000/api/v1/gain/batch \
  -H "Content-Type: application/json" \
  -d @examples/requests/batch_request.json
```

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

The service supports ECEF (Earth-Centered Earth-Fixed) coordinates for positions exceeding 6400 km:

```json
{
  "antenna_id": "dsn_34m_uncalibrated",
  "feed_id": "x_band",
  "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
  "emitter_position": {"x": 7000000.0, "y": 0.0, "z": 500000.0},
  "frequency_mhz": 8450.0
}
```

### Geodetic Coordinates Example

Geodetic coordinates (longitude, latitude in degrees; altitude in meters) are auto-detected:

```json
{
  "antenna_id": "gs_3.7m_uncalibrated",
  "feed_id": "x_band_feed",
  "vehicle_position": {"x": -118.0, "y": 34.0, "z": 500.0},
  "emitter_position": {"x": -100.0, "y": 35.0, "z": 500000.0},
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

## Response Examples

All responses include:
- Request ID header (`X-Request-Id`)
- JSON content type
- Structured error responses on failure

See `responses/` directory for full example responses.

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
- `correction_applied`: true - B-spline correction surface was used
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
    "correction_applied": false,
    "parameters_source": "boresight_tuned",
    "coverage": {
      "azimuth_range_deg": [0.0, 0.0],
      "elevation_range_deg": [0.0, 0.0],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 15,
      "is_boresight_only": true
    }
  },
  "warnings": [
    "Antenna 'gs_7.3m_boresight' is partially calibrated. Accuracy estimate: ±1.5 dB"
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
- `correction_applied`: false - physics model only (no correction surface for boresight-only)
- `coverage.is_boresight_only`: true - measurements at single spatial point
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
    "correction_applied": false,
    "parameters_source": "boresight_tuned",
    "coverage": {
      "azimuth_range_deg": [0.0, 0.0],
      "elevation_range_deg": [0.0, 0.0],
      "frequency_range_mhz": [7100.0, 8500.0],
      "num_measurements": 15,
      "is_boresight_only": true
    }
  },
  "warnings": [
    "Antenna 'gs_7.3m_boresight' is partially calibrated. Accuracy estimate: ±1.5 dB",
    "Query is outside calibrated region - using physics model extrapolation"
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
    "correction_applied": false,
    "parameters_source": "design_specifications"
  },
  "warnings": [
    "Antenna 'gs_3.7m_uncalibrated' is uncalibrated (using design specifications). Absolute gain accuracy: ±4.0 dB, Loss accuracy: ±2.0 dB"
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

    # For partially calibrated, check if query is in coverage
    if status == 'partially_calibrated':
        if 'Query is outside calibrated region' in data.get('warnings', []):
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
