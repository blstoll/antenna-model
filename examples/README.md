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

### Quaternion vs Euler Angles

The service accepts either quaternion or Euler angles for attitude:

**Quaternion (normalized):**
```json
"vehicle_attitude": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0}
```

**Euler Angles (roll-pitch-yaw):**
```json
"vehicle_attitude": {"roll_deg": 0.0, "pitch_deg": 0.0, "yaw_deg": 0.0}
```

## Response Examples

All responses include:
- Request ID header (`X-Request-Id`)
- JSON content type
- Structured error responses on failure

See `responses/` directory for full example responses.

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
