# Quick Start Guide

Get started with the Antenna Model Service API in 5 minutes.

## Prerequisites

- Rust toolchain installed
- Service built: `cargo build --release`

## 1. Start the Service

```bash
cargo run --release --bin antenna-model
```

The service will start on `http://localhost:3000` by default.

## 2. Verify Service is Running

```bash
curl http://localhost:3000/health
```

Expected response:
```json
{
  "status": "healthy"
}
```

## 3. List Available Antennas

```bash
curl http://localhost:3000/api/v1/antennas | jq
```

## 4. Compute Antenna Gain

### Using curl:

```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d '{
    "antenna_id": "dsn_34m_uncalibrated",
    "feed_id": "x_band",
    "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
    "vehicle_attitude": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
    "reflector_boresight": {"x": 6500010.0, "y": 0.0, "z": 0.0},
    "feed_position": {"x": 6500005.0, "y": 0.0, "z": 0.0},
    "emitter_position": {"x": 7000000.0, "y": 0.0, "z": 500000.0},
    "frequency_mhz": 8450.0,
    "include_reference": true
  }' | jq
```

### Using Python:

```python
import requests

response = requests.post(
    "http://localhost:3000/api/v1/gain",
    json={
        "antenna_id": "dsn_34m_uncalibrated",
        "feed_id": "x_band",
        "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0},
        "vehicle_attitude": {"w": 1.0, "x": 0.0, "y": 0.0, "z": 0.0},
        "reflector_boresight": {"x": 6500010.0, "y": 0.0, "z": 0.0},
        "feed_position": {"x": 6500005.0, "y": 0.0, "z": 0.0},
        "emitter_position": {"x": 7000000.0, "y": 0.0, "z": 500000.0},
        "frequency_mhz": 8450.0,
        "include_reference": True
    }
)

result = response.json()
print(f"Gain: {result['gain_db']} dB")
```

## 5. Run All Examples

### Curl Examples:
```bash
bash examples/curl-examples.sh
```

### Python Examples:
```bash
pip install requests
python examples/python_examples.py
```

## Coordinate Systems

The API auto-detects coordinate systems:

### ECEF (Earth-Centered Earth-Fixed)
Used when any coordinate exceeds 6400 km:
```json
{
  "x": 6500000.0,
  "y": 100000.0,
  "z": 200000.0
}
```

### Geodetic
Used otherwise (longitude, latitude in degrees, altitude in meters):
```json
{
  "x": -118.0,
  "y": 34.0,
  "z": 500.0
}
```

## Attitude Representation

### Quaternion (normalized):
```json
{
  "w": 1.0,
  "x": 0.0,
  "y": 0.0,
  "z": 0.0
}
```

### Euler Angles (roll-pitch-yaw in degrees):
```json
{
  "roll_deg": 0.0,
  "pitch_deg": 10.0,
  "yaw_deg": 45.0
}
```

## Common Tasks

### Get Antenna Details
```bash
curl http://localhost:3000/api/v1/antennas/dsn_34m_uncalibrated | jq
```

### List Feeds for an Antenna
```bash
curl http://localhost:3000/api/v1/antennas/dsn_34m_uncalibrated/feeds | jq
```

### Batch Processing
```bash
curl -X POST http://localhost:3000/api/v1/gain/batch \
  -H "Content-Type: application/json" \
  -d @examples/requests/batch_request.json | jq
```

### Generate Heatmap
```bash
curl -X POST http://localhost:3000/api/v1/heatmap \
  -H "Content-Type: application/json" \
  -d @examples/requests/heatmap_request.json | jq
```

## Performance Expectations

- Single evaluation: <100ms p95 latency
- Batch (100 evaluations): <500ms
- Heatmap (72x46 grid): <2 seconds

## Error Handling

All errors return structured JSON:

```json
{
  "error": "antenna_not_found",
  "message": "Antenna 'nonexistent' not found"
}
```

HTTP status codes:
- `200 OK` - Success
- `400 Bad Request` - Invalid input
- `404 Not Found` - Resource not found
- `422 Unprocessable Entity` - Validation error
- `500 Internal Server Error` - Server error
- `503 Service Unavailable` - Service not ready

## Next Steps

- Read the [full API examples README](README.md)
- Review [request examples](requests/)
- Check [response examples](responses/)
- See [implementation plan](../docs/implementation-plan.md) for architecture details
- Read [CLAUDE.md](../CLAUDE.md) for development guidelines

## Troubleshooting

### Service not starting
```bash
# Check if port 3000 is already in use
lsof -i :3000

# Run on a different port
cargo run --release --bin antenna-model -- --port 8080
```

### No calibration data loaded
The service will start with an empty repository if calibration data is not found. This is normal for testing. See [calibration documentation](../docs/implementation-plan.md) for how to generate calibration data.

### Python requests failing
```bash
# Install dependencies
pip install requests

# Verify service is running
curl http://localhost:3000/health
```

## Support

For issues or questions:
- Check logs for detailed error messages
- Review [design documentation](../docs/antenna-model-design-doc.md)
- See [architecture documentation](../docs/architecture.md)
