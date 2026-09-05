# Antenna Model

The Antenna Model context describes parabolic-dish pointing, gain prediction, and calibration. Its language distinguishes commanded geometry from physical feed geometry and measured corrections from physics-only estimates.

## Antenna and feed geometry

**Antenna**:
A parabolic reflector together with one or more feeds whose gain is modeled as a single system.
_Avoid_: Dish, station, terminal when referring to the modeled antenna system

**Reflector**:
The parabolic surface that redirects energy between a feed and the far field.
_Avoid_: Antenna when referring only to the reflecting surface

**Reflector vertex**:
The center and origin of the reflector surface, coincident with the vehicle position in this model.
_Avoid_: Vehicle-to-reflector offset

**Reflector boresight**:
The Earth position toward which the reflector axis points.
_Avoid_: Antenna pointing, look direction

**Feed**:
A receiving or transmitting element associated with an antenna and identified together with that antenna.
_Avoid_: Receiver when referring to the physical feed

**Feed pointing location**:
The Earth position that a feed is aimed at, from which its physical displacement is derived.
_Avoid_: Feed position, physical feed location

**Physical feed offset**:
The feed's displacement from the focal point, including its design offset and any displacement needed to aim at the feed pointing location.
_Avoid_: Feed pointing location

**Design feed offset**:
A feed's fixed physical displacement from the focal point before request-specific steering.
_Avoid_: Feed pointing location, total feed offset

**Focal point**:
The ideal location of a feed's electromagnetic phase center for a focused reflector.
_Avoid_: Reflector vertex

**Vehicle position**:
The antenna's Earth position and the origin of its antenna frame, coincident with the reflector vertex.
_Avoid_: Platform center when implying a separate modeled reflector offset

**Emitter position**:
The Earth position of the source whose gain is being evaluated.
_Avoid_: Feed pointing location, reflector boresight

## Coordinates and pointing

**ECEF**:
The Earth-Centered Earth-Fixed Cartesian frame in meters, with its origin at Earth's center.
_Avoid_: Geodetic coordinates

**Geodetic coordinates**:
Longitude, latitude, and altitude relative to the WGS84 ellipsoid.
_Avoid_: ECEF coordinates, GPS coordinates when the precise frame matters

**ENU**:
The local East-North-Up tangent frame anchored at a geodetic location.
_Avoid_: Antenna frame

**Antenna frame**:
A right-handed frame at the reflector vertex whose positive Z axis is boresight and whose X axis defines zero azimuth.
_Avoid_: ENU frame

**E-cone**:
The polar angle measured away from boresight.
_Avoid_: Elevation above the horizon

**E-clock**:
The azimuthal angle around boresight, measured from the antenna frame's positive X axis toward positive Y.
_Avoid_: Geographic azimuth

**Operating frequency**:
The frequency at which gain is evaluated.
_Avoid_: Pointing frequency

**Pointing frequency**:
The frequency at which the antenna was mechanically pointed; a difference from operating frequency produces beam squint.
_Avoid_: Operating frequency

**Beam squint**:
The frequency-dependent angular displacement between the mechanically pointed beam and the beam at operating frequency.
_Avoid_: Feed steering

## Gain and calibration

**Gain**:
The modeled antenna gain toward the emitter for the requested geometry and operating frequency.
_Avoid_: G/T, loss

**Reference gain**:
The modeled gain for an ideal reference geometry in which the feed is focused and the antenna points at the emitter.
_Avoid_: Peak measured gain

**Loss**:
The reduction from reference gain to requested gain, expressed as reference gain minus gain.
_Avoid_: Negative gain, path loss

**G/T**:
Antenna gain divided by system noise temperature, expressed in decibels per kelvin.
_Avoid_: Gain

**Physical optics model**:
The physics-based estimate of antenna gain from reflector, feed, mesh, frequency, and pointing geometry.
_Avoid_: Calibration model

**Correction surface**:
A fitted residual model representing measured gain minus the physical optics prediction over calibrated conditions.
_Avoid_: Physical optics model, calibration artifact

**Calibration artifact**:
The versioned antenna-and-feed model produced by calibration and consumed for gain evaluation.
_Avoid_: Measurement file, correction surface

**Calibration coverage**:
The angular and frequency region supported by calibration measurements.
_Avoid_: Validity range

**Fully calibrated**:
A calibration status backed by measurements and a correction surface across the intended coverage.
_Avoid_: Calibrated when the distinction from partial calibration matters

**Partially calibrated**:
A calibration status backed by limited measurements, such as a boresight sweep or a restricted angular region.
_Avoid_: Fully calibrated

**Uncalibrated**:
A calibration status based on design specifications without measured correction data.
_Avoid_: Invalid, unsupported

**Extrapolation**:
Evaluation outside the calibration coverage or declared validity range, where the result remains available with reduced confidence.
_Avoid_: Interpolation
