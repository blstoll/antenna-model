# Domain Contract: Antenna Model

Ground truth for coordinate frames, parameter meanings, and invariants in this
codebase. Mined from code + design docs + session history on 2026-07-03; refreshed
against HEAD on 2026-07-07 (line numbers re-verified after the beam-steering fixes).

If code and this contract disagree, **stop and ask** — do not assume the contract
is right (it may be stale) or the code is right (it may be the next bug). Update
this file in the same change that resolves the disagreement.

Items previously marked "UNVERIFIED — confirm" were cleared by the maintainer on
2026-07-07; where a claim rests on physics judgment the maintainer is not a domain
expert in, the basis for confirmation is recorded inline so a later expert pass can
re-check it.

## Coordinate frames

| Frame | Convention | Origin | Axes / handedness | Used in |
|---|---|---|---|---|
| ECEF | Earth-Centered Earth-Fixed, meters | Earth's center of mass | X: equator ∩ prime meridian; Y: equator at 90°E; Z: North pole; right-handed | `Position3D` when `\|x\|,\|y\|,\|z\| > 6400 km` (or explicit `coordinate_system: ecef`); `geodetic_to_ecef`/`ecef_to_geodetic` (`antenna-model/src/model/coordinates_3d.rs:169,197`) |
| Geodetic (WGS84) | lon °E ∈ [-180,180], lat °N ∈ [-90,90], alt meters above ellipsoid | WGS84 ellipsoid | N/A (angular + altitude) | `Position3D` default when magnitudes are small; auto-detection is **lossy above 6400 km altitude** — GEO satellites (~35,786 km) MUST set `coordinate_system: geodetic` explicitly or they silently misparse as near-Earth-center ECEF (`schemas.rs:62-98,126`) |
| ENU (East-North-Up) | Local tangent-plane frame at a given (lat, lon) | The (lat, lon) point itself, not Earth's center | Rows of `R` are East, North, Up expressed in ECEF (`coordinates_3d.rs:263-266`) | `ecef_to_enu_rotation` (`coordinates_3d.rs:271`); heatmap emitter-position generation (`service/heatmap.rs:318-362`) |
| Antenna frame | Cartesian, origin at the reflector vertex (≡ `vehicle_position`, see invariant below) | Reflector vertex / vehicle position (assumed coincident) | Z = boresight (vehicle → reflector_boresight, normalized); X = azimuth-zero reference (attitude quaternion body +X if supplied, else Earth-Z/East cross-product heuristic); Y completes right-hand system | `coordinates.rs` (feed math, vertex origin, `:28,31`); `compute_emitter_direction[_with_attitude]` (`coordinates_3d.rs:495,565`) |
| Far-field / E-clock-E-cone | θ = **polar angle from boresight** (0°=boresight, 90°=perpendicular), NOT horizon elevation; φ = azimuth, 0°=+X, 90°=+Y | Antenna frame origin | Right-handed spherical | `FarFieldCoordinates`, `EClockConeCoordinates` (`coordinates.rs:80,115`); `antenna_frame_to_spherical` (`coordinates_3d.rs:321`) |

**Invariant — antenna-frame origin is single (`vehicle_position` ≡ reflector vertex).**
Feed-position math is anchored at the **reflector vertex** (`coordinates.rs:28,31`);
emitter-direction math (`compute_emitter_direction_with_attitude`,
`coordinates_3d.rs:495`) measures the emitter offset from **`vehicle_position`**. No
offset between the two is modeled anywhere, and per the maintainer (2026-07-07) this
is intentional: the dish has a single location, so `vehicle_position` and the
reflector vertex are the same point. **Future code must not introduce a spurious
vehicle-to-vertex offset term** — if a real mount offset ever needs modeling, it is a
deliberate contract change, not an incidental one.

**Gotcha — ENU axis contract (the anchor bug).** `ecef_to_enu_rotation` returns `R` such
that `[E;N;U] = R · [ECEF]` (ECEF→ENU direction). To go ENU→ECEF, use `R` **transposed**,
never `R` itself (R is orthogonal, so `R⁻¹ = Rᵀ`). Getting this backwards silently rotates
into the wrong frame with no error (fixed once already in commit `8a48201`). The transpose
requirement is documented at `coordinates_3d.rs:261`; the heatmap path applies it correctly
inline (`heatmap.rs:357-362`, indexes `rot[k][i]` = `Rᵀ`). Note the heatmap comment calls
these "columns" while `coordinates_3d.rs` calls them "rows" — the math is the transpose
either way, but the wording is inconsistent between the two files.

**Gotcha — the ECEF-detection threshold is lossy for GEO+ altitudes.** The auto-detect
boundary is 6400 km on any axis (`Position3D::ECEF_THRESHOLD_M`, `schemas.rs:126`). A
geodetic position with `alt_m` above that (any GEO or high-HEO satellite) will
auto-classify as ECEF unless `coordinate_system: geodetic` is set explicitly. This is
documented and tested (`schemas.rs:62-98`, `test_explicit_coordinate_system_overrides_detection`
at `:1180`), but it is a standing trap for **any new endpoint or example that constructs a
`Position3D` without setting the field explicitly**.

## Transforms

| From → To | Function / location | Invariants |
|---|---|---|
| Geodetic → ECEF | `geodetic_to_ecef` (`coordinates_3d.rs:169`) | Round trip with `ecef_to_geodetic` is identity to ~1e-6 (test `test_ecef_to_geodetic_roundtrip`, `coordinates_3d.rs:951`) |
| ECEF → Geodetic | `ecef_to_geodetic` (`coordinates_3d.rs:197`) | Pole-safe altitude branch (`coordinates_3d.rs:239`, uses `z/sin(lat)` form when `\|cos(lat)\| ≤ 1e-4`); round trip near poles has its own test (`test_ecef_to_geodetic_pole_with_altitude`, `coordinates_3d.rs:1334`) |
| ECEF → ENU (rotation matrix) | `ecef_to_enu_rotation` (`coordinates_3d.rs:271`) | `R` is orthogonal: `R · Rᵀ = I`. Asserted by `test_ecef_to_enu_rotation_is_orthogonal` (added with this contract). |
| ENU offset → ECEF offset | `Rᵀ · [ENU]` (no dedicated function; done inline in `heatmap.rs:357-362`) | Must use the transpose, never `R` directly (see gotcha above). Asserted by `test_enu_ecef_roundtrip_uses_transpose`. |
| Earth pointing positions (feed + boresight) → physical feed displacement | `compute_feed_position_from_pointing` (`coordinates_3d.rs:673`) | Reflector boresight pointing position maps to `(0,0)` angular offset by construction; feed offset = angular gap between feed-aim and boresight-aim, converted to E-cone/E-clock, then to Cartesian. Applies the beam deviation factor (`beam_deviation_factor`, `coordinates_3d.rs:648`) at `:715`. |
| E-clock/E-cone → physical feed position (Cartesian) | `EClockConeCoordinates::to_feed_position` (geometric, BDF=1, `coordinates.rs:264`); `::to_feed_position_with_bdf` (`:250`) | **Beam-deviation sign flip (SETTLED):** the feed physically displaces **opposite** the clock angle of the desired beam direction (`coordinates.rs:221-222`, "NEGATED: beam deviation puts the feed on the side opposite the desired beam direction"). Confirmed by commit `83193a0` (Task 1 of `docs/superpowers/plans/2026-07-02-review-fixes.md`, verified plan-compliant) and pinned by `antenna-model/tests/beam_steering_direction.rs`. The steering *magnitude* is additionally reduced by the beam deviation factor (BDF ≈ 0.871 at f/D=0.5, Task 2). Round trip `to_feed_position`∘`from_feed_position` holds to 1e-6 except clock angle is undefined at cone=0 (test `test_e_clock_cone_roundtrip`, `coordinates.rs:466`). |
| Antenna-frame Cartesian → far-field spherical (az, el) | `antenna_frame_to_spherical` (`coordinates_3d.rs:321`) | `el` is polar angle, **always in [0°, 180°]**, never horizon-based; `az` from raw `atan2` is in (−180°, 180°] before normalization |
| Raw azimuth → normalized azimuth | `normalize_azimuth_deg` (`coordinates_3d.rs:289`) | Output always in [0°, 360°). Coverage ranges and B-spline knots assume this range — **any new az-producing code path that skips this normalization silently breaks coverage lookups** (finding #5 in `docs/review-findings-2026-06-10.md`, fixed in commit `43a74af`). Boundary values asserted by `test_normalize_azimuth_deg_boundaries` (added with this contract). |
| Uncorrected (az, el) → squint-corrected (az, el) | `apply_beam_squint_correction` (`coordinates_3d.rs:766`) / `squint_corrected_direction` (`coordinates_3d.rs:852`) | **Argument-order trap**: `squint_corrected_direction` takes `(operating_freq, pointing_freq)`; it calls `apply_beam_squint_correction` with `(pointing_freq, operating_freq)` — reversed (`coordinates_3d.rs:869-870`, already commented in code). Corrected elevation is `≥ 0` by construction; corrected azimuth renormalized to [0,360). Pinned by `test_squint_corrected_direction_frequency_argument_order` (added with this contract). |

## Parameter glossary

| Name | Meaning | Units | Range / default | Gotchas |
|---|---|---|---|---|
| `Position3D.{x,y,z}` | ECEF meters *or* geodetic (lon°, lat°, alt m) depending on magnitude/explicit tag | mixed (see frame table) | ECEF magnitude ≤ ~406,378 km; geodetic alt ≤ 400,000 km | Auto-detection threshold is 6400 km; see gotcha above |
| `GainRequest.feed_position` / `H3LinkBudgetRequest.feed_position` | **The feed *pointing* location** — an Earth position the feed is aimed at (off the reflector boresight), NOT the feed's physical location in the antenna | Position3D | n/a | THE anchor bug. The feed's *physical* position (rel. to the vertex) is a derived property — the displacement the feed moves to in order to achieve this aim, given the pointing frequency (which may differ from the collected frequency for multi-receiver feeds). Do not confuse with `model::geometry::FeedPosition` (`geometry.rs:249`), which *is* the physical antenna-frame offset. The API field doc comment is still bare ("Feed position (ECEF or Geodetic)", `schemas.rs:239`); field occurs at `schemas.rs:240,417,568`. Consider renaming to `feed_pointing_location` in a future major version — flagged, not fixed (breaking API change). |
| `GainRequest.reflector_boresight` | Earth position the reflector is pointed at; together with `vehicle_position` defines antenna Z-axis | Position3D | n/a | Must not coincide with `vehicle_position` (< 1mm separation raises `InvalidCoordinate`, `coordinates_3d.rs:513-515` and `:600-602`) |
| `vehicle_attitude` | Optional unit quaternion `[w,x,y,z]`, body→ECEF. Body +Z = boresight, body +X = azimuth-zero reference | dimensionless (unit norm) | norm within 1e-3 of 1.0 | When omitted, azimuth-zero uses the Earth-Z/East cross-product heuristic, which is **discontinuous** near boresight ∥ Earth-Z (switches when `\|z_z\| ≥ 0.99`, i.e. within ≈8.1° of Earth Z, `coordinates_3d.rs:402,427`) — this was finding #5b in the 2026-06-10 review |
| `q_factor` (feed illumination) | cos^q feed pattern exponent; higher = more focused beam, less spillover | dimensionless | Typical 6–12; example configs use 8–10 | The current `illumination.rs` module doc (`illumination.rs:23`) states combined edge taper (cos^q × space loss) for q=8, f/D=0.5 is "approximately −37.4 dB", consistent with `edge_taper_db` (`illumination.rs:257`). **Note:** the classic "q≈6–8 for 10 dB edge taper" textbook rule of thumb does NOT apply here — this codebase's edge taper is the *combined* (pattern × space-loss) definition. Anyone porting a q-factor from another source must re-derive against `edge_taper_db`, not assume the rule of thumb. |
| `phase_center_offset` | Distance from physical feed to EM phase center | meters | Typically ±λ/4, frequency-dependent (`geometry.rs:186`) | **OPEN FINDING (still true as of 2026-07-07):** `illumination::phase_center_offset_phase` (`illumination.rs:357`) exists and is unit-tested (`illumination.rs:578+`), but a code search finds **no call site in `integration.rs` or `pattern.rs`** (the live aperture-integration / gain pipeline). The parameter is computed correctly in isolation but appears not consumed by the live gain path. Re-check with `grep -rn phase_center_offset_phase src/model/{integration,pattern}.rs` before assuming this is wired in. |
| `surface_rms` | Reflector surface RMS deviation from ideal parabola, used in Ruze's equation | meters | Should be ≪ shortest operating wavelength; example configs 0.4mm–1.5mm | **Scope (confirmed 2026-07-07):** the Ruze form `η = exp(-(4π·σ/λ)²)` (Ruze 1966) models **surface-error (roughness) efficiency only** — the boresight-gain loss from random deviations of the real dish from an ideal paraboloid. It is one multiplicative factor in the live gain path: `overall_efficiency` computes `eta_ruze * eta_mesh` (`pattern.rs:128-141`) using `pattern::ruze_efficiency` (`pattern.rs:112`). It is *not* the steering / off-boresight physics (that lives in the aperture-integration / phase model). The `4π` constant is `2·(2π/λ)`, the factor of 2 coming from the reflection double-pass — a correctly-handled reflected path error. Not independently re-derived against the primary reference; scope and constant confirmed self-consistent with the code. **OPEN FINDING:** a second, identical Ruze implementation exists in `surface.rs` (`ruze_efficiency` `:38`, `ruze_efficiency_from_frequency` `:54`) with no live-path callers — duplicated formula, confirm which is canonical and remove the other. |
| `mesh_spacing` / `wire_diameter` | Wire-mesh reflector geometry; mesh introduces frequency-dependent reflection loss | meters | spacing ~1-10mm, wire diameter ~0.05-1mm, wire_diameter < spacing (enforced with an error, `geometry.rs:411-419`) | `pattern.rs::overall_efficiency` (`pattern.rs:128,134`) calls `mesh::mesh_reflection_efficiency` (`mesh.rs:389`, inductive-grid model) directly. **OPEN FINDING:** `MeshParameters::transparency_at_wavelength` (`geometry.rs:435`) is a *different*, simpler low-frequency approximation whose only callers are its own unit tests — effectively dead code as of 2026-07-07; confirm removal vs. an intended-but-unwired second path. |
| `f_over_d` (focal length / diameter) | Reflector geometry ratio, affects illumination/aperture efficiency and beam deviation factor | dimensionless | Typical [0.3, 0.5]; validated range [0.2, 1.0] (`geometry.rs:68,100`) | Out-of-range values do **not** warn or error — the check at `geometry.rs:100-106` is a silent no-op ("unusual but not necessarily invalid"). Confirm this is intentional for exotic designs vs. a missed validation. |
| `pointing_frequency_mhz` vs `frequency_mhz` (operating) | Frequency the antenna was mechanically pointed at vs. actual operating frequency; difference drives beam squint correction. May differ (e.g. multiple receivers on one feed) | MHz | n/a | Argument-order trap in the two squint functions — see Transforms table |
| E-cone / E-clock | Polar/azimuthal angle pair around boresight, used for antenna pointing control | degrees or radians (function-dependent — check signature) | E-cone conventionally [0°,180°], E-clock [0°,360°) | `from_degrees`/`to_degrees` convert consistently (`EClockConeCoordinates::to_degrees`, `coordinates.rs:137`; this was finding #10 in the 2026-06-10 review, fixed in commit `72e16ce`) |

## Invariants

| Statement | Testable? |
|---|---|
| `geodetic_to_ecef` ∘ `ecef_to_geodetic` ≈ identity (away from poles, and at poles via the z/sin(lat) branch) | Yes — tests exist (`coordinates_3d.rs:951,1334`) |
| `ecef_to_enu_rotation(lat,lon)` is orthogonal: `R · Rᵀ = I` | Yes — `test_ecef_to_enu_rotation_is_orthogonal` (added with this contract) |
| ENU→ECEF uses `Rᵀ`: a pure "Up" ENU vector maps via `Rᵀ` to local vertical in ECEF | Yes — `test_enu_ecef_roundtrip_uses_transpose` (added with this contract) |
| `EClockConeCoordinates::to_feed_position` ∘ `from_feed_position` ≈ identity, except clock angle undefined at cone=0 | Yes — test exists (`coordinates.rs:466`) |
| A feed physically displaced at clock angle φ produces peak gain at aim clock angle φ (not φ+180°) | Yes — test exists (`antenna-model/tests/beam_steering_direction.rs`) |
| `feed_position` API field is an Earth aim point, not a fixed physical offset: same `feed_position` + different `vehicle_position` ⇒ different physical feed displacement | Yes — `feed_position_is_pointing_target.rs` (added with this contract) |
| `normalize_azimuth_deg` output always in [0°, 360°) | Yes — `test_normalize_azimuth_deg_boundaries` (added with this contract) |
| `squint_corrected_direction(op,pt)` == `apply_beam_squint_correction(pt,op)` (arg-order contract) | Yes — `test_squint_corrected_direction_frequency_argument_order` (added with this contract) |
| Elevation/E-cone (polar angle) is always in [0°, 180°], never negative | Partially — `apply_beam_squint_correction` guards `≥ 0` via `debug_assert!` (`coordinates_3d.rs:775`) **and** a release-mode `.abs()` (`:781`); `antenna_frame_to_spherical` relies on `acos` range (mathematically [0,π]), not asserted at the boundary |
| GEO-altitude geodetic positions round-trip through the API without misclassifying as ECEF, when `coordinate_system` is set explicitly | Yes — `test_explicit_coordinate_system_overrides_detection` (`schemas.rs:1180`) |
| A quaternion passed as `vehicle_attitude` must be unit-norm; `quaternion_rotate` preserves vector length only for unit-norm input | Yes — `quaternion_rotate` doc states the assumption (`coordinates_3d.rs:368`); confirm `validate_gain_request`/`validate_h3_link_budget_request` reject non-unit quaternions |
| Reflector boresight position must not coincide with vehicle position | Yes — enforced with an error (`coordinates_3d.rs:513-515,600-602`) |
| `vehicle_position` ≡ reflector vertex (single antenna-frame origin, no offset modeled) | No executable test — a documented modeling assumption (see frame table); future code must not add a vehicle-to-vertex offset without a contract change |

## Open items surfaced while mining (not fixed here)

- `phase_center_offset_phase` computed/tested but apparently not consumed by the live
  gain path (`integration.rs`/`pattern.rs`) — see parameter glossary.
- `MeshParameters::transparency_at_wavelength` is test-only dead code — see glossary.
- Duplicate Ruze implementation: `surface.rs::ruze_efficiency` / `ruze_efficiency_from_frequency`
  duplicate `pattern.rs::ruze_efficiency` (the one the live gain path uses); the `surface.rs`
  pair has no live-path callers — see `surface_rms` glossary entry.
- `f_over_d` out-of-range is a silent no-op, not a warning — confirm intentional.
- **Design-doc / code formula parity (process item):** `docs/antenna-model-design-doc.md`
  Section 2.5 documents the E-clock/E-cone → feed-position formula *without* the
  beam-deviation sign flip now in `coordinates.rs:221-222`. Update the design doc to
  match before trusting it as a second source for anything coordinate-related
  (`docs/review-findings-2026-06-10.md:30` flagged the same class of drift).
