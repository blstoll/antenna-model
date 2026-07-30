High-accuracy antenna loss modeling system providing REST API access to antenna models
with flexible calibration statuses. Supports real-time queries for G/T (Gain-to-Temperature)
predictions based on 3D geometric configuration.

## Calibration Statuses

The system supports multiple calibration statuses:
- **Fully Calibrated**: ±1 dB accuracy (main lobe/first sidelobe)
- **Partially Calibrated (Boresight)**: ±1 dB at boresight, ±1-2 dB loss
- **Partially Calibrated (Limited Coverage)**: ±1-1.5 dB in-coverage, ±2-3 dB extrapolated
- **Uncalibrated**: ±3-5 dB absolute gain, ±2-3 dB loss (design specs only)

## Coordinate Systems

Every 3D position must declare its frame in the required `coordinate_system` field:
- **`ecef`**: x, y, z in meters from Earth's centre
- **`geodetic`**: x = longitude degrees, y = latitude degrees, z = altitude meters

There is no auto-detection. A position that omits `coordinate_system` is rejected
with **400 `invalid_request_body`** naming the field.
