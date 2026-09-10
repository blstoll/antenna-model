# Testing the API Examples

All examples have been updated to use the actual antennas from `calibration_data/antennas.yaml`.

## Available Antennas

A default `cargo run --release --bin antenna-model` loads these five (verified
2026-08-16 against `/api/v1/antennas`):

- **gs_3.7m_uncalibrated** - Ground station (3.7m) with 2 feeds: s_band_feed, x_band_feed
- **dsn_13m_uncalibrated** - Medium DSN antenna (13m) with 3 feeds: x_band_downlink, x_band_uplink, ka_band_downlink
- **dsn_34m_uncalibrated** - Large DSN antenna (34m) with 3 feeds: s_band, x_band, ka_band
- **dsn_70m_uncalibrated** - DSN 70m reference antenna with 1 feed: x_band
- **gbt_100m_uncalibrated** - GBT 100m reference antenna with 2 feeds: l_band, q_band

All five are uncalibrated (design specifications only), which is why the service
starts `healthy` with no `.bin` artifact present — see roadmap D9 and the header
of `calibration_data/antennas.yaml`. The other four entries in that file are
disabled templates that name a `.bin` a clean checkout does not contain.

`test_simple`, a 5 m development fixture, was **disabled in the shipped config on
2026-08-16** (roadmap D9) so it no longer appears in an operator's antenna list.
The integration tests are unaffected — they load their own
`antenna-model/tests/fixtures/test_antennas.yaml`, which carries its own copy.

## Quick Tests

### 1. Health Check
```bash
curl http://localhost:3000/health
```

Expected: `{"status":"healthy"}`

### 2. List Antennas
```bash
curl http://localhost:3000/api/v1/antennas | jq '.antennas[] | {id, name, feed_count}'
```

### 3. Get Antenna Details
```bash
curl http://localhost:3000/api/v1/antennas/dsn_34m_uncalibrated | jq
```

### 4. Single Gain Computation

```bash
curl -X POST http://localhost:3000/api/v1/gain \
  -H "Content-Type: application/json" \
  -d @examples/requests/gain_request.json | jq
```

Expected: `gain_db` ≈ 74.3 dBi (DSN 70 m, X-band, emitter on boresight), with
`uncalibrated` and `spillover_significant` warnings. `gain_request_geodetic.json`
is the separate geodetic-frame example — a different antenna and geometry
(`gs_3.7m` at 8200 MHz, satellite well off boresight), not this request restated.

### 5. Batch Computation
```bash
curl -X POST http://localhost:3000/api/v1/gain/batch \
  -H "Content-Type: application/json" \
  -d @examples/requests/batch_request.json | jq '.metadata'
```

Expected: `"count": 3, "failure_count": 0`. Check `failure_count`, not the status
code — batch answers **200 even when every item fails**.

### 6. Heatmap Generation
```bash
curl -X POST http://localhost:3000/api/v1/heatmap \
  -H "Content-Type: application/json" \
  -d @examples/requests/heatmap_request.json | jq '.metadata'
```

## Run All Examples

Execute all examples at once:
```bash
bash examples/curl-examples.sh
```

## Verified Test Results

All examples have been tested against the running service:

✓ Health endpoint works
✓ Antenna listing returns 4 antennas
✓ Antenna details work for all loaded antennas
✓ Gain computation works with ECEF coordinates
✓ Gain computation works with Geodetic coordinates
✓ Batch processing works with 3 different antenna/feed combinations
✓ Heatmap generation works

## Example Response Snippets

### Antenna List
```json
{
  "antennas": [
    {
      "id": "dsn_34m_uncalibrated",
      "name": "DSN 34m - Uncalibrated - Ka-Band Feed",
      "enabled": true,
      "feed_count": 3,
      "feed_ids": ["ka_band", "s_band", "x_band"]
    }
  ]
}
```

### Gain Response
```json
{
  "antenna_id": "dsn_34m_uncalibrated",
  "feed_id": "x_band",
  "gain_db": 67.70242948159164,
  "geometry": {
    "physical_feed_offset_m": {"x": -5.0, "y": 0.0, "z": 0.0},
    "emitter_azimuth_deg": 0.0,
    "emitter_elevation_deg": 45.0
  },
  "warnings": [
    {
      "code": "uncalibrated",
      "message": "Antenna 'dsn_34m_uncalibrated' is uncalibrated (using design specifications). Absolute gain accuracy: ±3.0 dB, Loss accuracy: ±2.0 dB"
    }
  ],
  "calibration_status": {
    "status": "uncalibrated",
    "accuracy_estimate_db": 3.0,
    "loss_accuracy_estimate_db": 2.0,
    "correction_application": "unavailable",
    "correction_applied": false,
    "parameters_source": "design_specifications"
  }
}
```

### Batch Response Metadata
```json
{
  "total_computation_time_ms": 0.014834,
  "count": 3
}
```

## Notes

- All antennas are currently uncalibrated, using design specifications
- Uncalibrated antennas have ±3 dB absolute gain accuracy and ±2 dB loss accuracy
- Queries return calibration status information showing accuracy estimates
- ECEF coordinates must be within 400,000 km of Earth's centre to pass validation
- Every position must carry `"coordinate_system": "ecef"` or `"geodetic"`; the service
  does not infer the frame, and an untagged position is a 400
