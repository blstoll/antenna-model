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
| `q_factor` (feed illumination) | cos^q **field** (voltage) pattern exponent; higher = more focused beam, less spillover, deeper edge taper | dimensionless | ~1–3 for a ~−11 dB edge taper at f/D 0.4–0.6; design configs corrected 2026-07-10 (were 8–11) | Because `cos_q_pattern` is the *field* pattern, a "textbook horn" q of 8–12 here gives an absurd edge taper (q=9.5 @ f/D 0.4 → **−71 dB** — a dark aperture rim). The old 8–11 design values under-predicted DSN 34-m peak gain by ~5 dB; corrected via `q_factor_from_taper(−11 dB, f/D)` → q≈1.1–2 (`docs/findings-2026-07-10-ka-phase-center-defocus.md` sibling finding; `tests/reference_validation.rs::feed_taper_q_sweep_dsn_34m_xband`). The `illumination.rs:23` module-doc example (q=8, f/D=0.5 → −37.4 dB) is *consistent with `edge_taper_db`* but is itself an over-tapered case — do not read it as a recommended value. The classic "q≈6–8 for 10 dB edge taper" rule of thumb does NOT apply; always re-derive against `edge_taper_db`, never assume the rule of thumb. |
| `phase_center_offset` | Axial distance from the physical feed aperture to the EM phase center | meters | Typically ±λ/4, frequency-dependent (`geometry.rs:186`) | **COMPENSATED — no gain effect (implemented 2026-07-10, roadmap P7, auto-refocus).** `config.feed.phase_center_offset` is now a *recorded feed property only*: the model assumes the feed is positioned so its phase center sits at the focal point (real operated antennas, incl. the DSN 34-m, refocus per band), so this field does **not** enter `feed_axial_offset` and produces **zero** defocus loss. Pinned by `test_phase_center_offset_alone_produces_no_defocus_loss` (`integration.rs:996`, `\|Δ\| < 1e-9 dB`) and, end-to-end, by `test_phase_center_offset_m_is_inert_at_service_level` (`service/evaluator.rs`, bit-identical gain regardless of value). Historically (pre-P7) this field *was* folded directly into `feed_axial_offset` and its 1/λ-scaling defocus penalty was the root cause of a Ka-band under-prediction — see `docs/findings-2026-07-10-ka-phase-center-defocus.md` for the full decomposition; that analysis remains the reference for *why* auto-refocus was chosen. Deliberate defocus now goes through the separate `axial_defocus` field (next row). |
| `axial_defocus` (config field `axial_defocus_m`) | Deliberate axial defocus of the feed's phase center from the focal point — the explicit knob for intentionally defocused-feed studies, distinct from (and not compensated like) `phase_center_offset` | meters | Positive = away from the reflector vertex; default 0 (focused, no defocus loss) | **Added 2026-07-10 (roadmap P7).** Consumed at `integration.rs:529`: `feed_axial_offset = position.z − focal_length + axial_defocus`, driving the same quadratic (defocus) aperture-phase term that `phase_center_offset` drove pre-P7. Pinned by `test_axial_defocus_produces_defocus_loss` (`integration.rs:1031`, 5 cm costs >1 dB at 8.4 GHz) and `test_axial_defocus_m_reduces_gain_at_service_level` (`service/evaluator.rs`, `axial_defocus_m: 0.05` costs >0.5 dB end-to-end). Service-config-only knob — calibrate writers always stamp `axial_defocus_m: 0.0` into artifacts (not a fitted/calibrated quantity). Not to be confused with the separate, pre-existing `phase_center_offset_wavelengths` unit quirk in `calibrate/antenna_config.rs:63` (unrelated, out of scope here). |
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

## Modeled vs unmodeled efficiency terms

The live gain path multiplies these efficiency factors into directivity:

| Term | Where | Applies to |
|---|---|---|
| Ruze (surface roughness) | `pattern.rs::overall_efficiency` (`ruze_efficiency`) | all antennas |
| Mesh reflection | `pattern.rs::overall_efficiency` (`mesh::mesh_reflection_efficiency`) | mesh reflectors |
| **Feed spillover** | `pattern.rs::compute_gain` behind `IntegrationParams::apply_spillover` (roadmap **P1**) | **uncalibrated antennas, StandardPhysicalOptics mode only** |

**Double-counting gate:** spillover is applied *only* when the antenna has no correction
surface at all (whole-antenna gate, decided in the service layer — `compute_gain_from_request`,
and mirrored on the h3 path). For calibrated antennas the fitted correction surface already
absorbs spillover empirically, so applying it again would double-count.

**Offset gate:** spillover is additionally applied only for feed offsets
≤ `SPILLOVER_MAX_OFFSET_RATIO`·f (0.3·f) — the validity limit of `estimate_spillover`'s
small-offset approximation. Beyond it (the 0.3·f–0.5·f band and the >0.5·f ray-tracing
regime) the linear offset extrapolation is unvalidated and saturates to ~100%, which would
clamp gain to the degenerate −60 dB floor; those cases already carry degraded-accuracy
warnings and retain their exact pre-P1 gain. So `spillover_loss_db` is `null` for
large-offset queries even on uncalibrated antennas. (Roadmap **P2** removed the former
`HigherOrderAberrations` mode that used to cover the 0.3·f–0.5·f band; that band now routes
through `StandardPhysicalOptics` — its exact coma phase already carries the full low-order
aberration content. The spillover gate keys off the offset ratio rather than the mode enum
precisely so that removing the mode did **not** widen the spillover regime into the band
where `estimate_spillover` is invalid; behavior is unchanged from P1.)

**Signal:** the applied loss is reported as `ComputationMetadata.spillover_loss_db`
(dB, negative; `null` when not applied — calibrated antenna, or feed offset > 0.3·f).

**Magnitude reality (finding 2026-07-09, revised 2026-07-10):** this note originally observed
that the modeled spillover was only ~0.001–0.05 dB — but that was **because** the enabled feeds
were grossly over-tapered (q=8–11). After the 2026-07-10 feed-taper fix (q≈1.1–3.1 for a ~−11 dB
edge taper), the feeds are broad and spillover is now **material: ~0.8 dB** (fraction ≈0.17–0.25),
matching the textbook range. It is applied on the uncalibrated path and cancels out of `loss_db`
(reference tracks it). Note also that `estimate_spillover` previously truncated a fractional `q`
to an integer exponent (`powi(q as i32 + 1)`); fixed to `powf(q + 1.0)` on 2026-07-10, which
moved the 34-m X-band figure from 0.192 → 0.171.

**Unmodeled (by decision):**
- **Blockage** (feed/strut aperture blockage, ~0.1–0.5 dB) — deferred to feature **F3**;
  data-gated on antenna-config geometry parameters that do not exist yet.
- **Cross-polarization** — out of scope (<0.1 dB on-axis for symmetric prime-focus dishes).

**Honest caveat:** modeling spillover removes a known systematic bias on the uncalibrated
path, but does **not** make uncalibrated predictions calibrated-grade — guessed q-factor and
assumed surface RMS still dominate the uncertainty there.

## Off-axis pattern / sidelobe fidelity

**✅ P10 LANDED 2026-07-15 — off-axis gain is now NUMERICALLY CORRECT (read this first).** The
served path now uses the Hankel / azimuthal-mode aperture integrator (roadmap unit P10): the
off-axis pattern is computed to convergence at all angles, so the aliasing defect described
below (served gain 20–35 dB too high beyond a few degrees, gain rising with angle) is
**RESOLVED**. **✅ F7 LANDED 2026-07-16/17** (`PHYSICS_MODEL_VERSION` 5, branch
`feat/f7-redesign-power-sum-obliquity`, superseding decision D-2 below): on the served
uncalibrated path (`physics_is_uncorrected()`) the returned value is now the **incoherent power
sum** of the idealised physical-optics value and the **F7 statistical sidelobe floor (ON)** in
the forward hemisphere — `10·log₁₀(10^(PO/10) + 10^(floor/10))` — and the **floor ONLY** (PO
excluded, rear integration skipped) behind the dish. See "Three-tier off-axis policy" below for
the full angular breakdown. The remaining caveat is therefore **physical, not numerical**:
idealised PO omits blockage, feed/strut scatter, and edge diffraction, and the floor is a
population median, not a per-antenna measurement, so far-off-axis sidelobe *levels* are
optimistic/approximate and not calibrated-grade (shape validated, absolute levels not). The
history below is preserved as-was and annotated with its resolution.

**✅ P11 LANDED 2026-07-15 — one predicate gates both "uncorrected-physics" behaviors.** The
spillover fold-in and the off-axis honesty warning are now gated by a single named predicate,
`AntennaCalibration::physics_is_uncorrected()` (`data/types.rs`), which is true iff there is no
correction surface (`correction_surface.is_none()`). Before P11 these two gates disagreed: the
spillover fold-in keyed on surface presence while the off-axis warning keyed on
`CalibrationStatus::Uncalibrated`. Those are *different sets* — `calibrate/boresight_calibration.rs`
emits `PartiallyCalibrated` with **no** correction surface whenever there is no frequency
correction, so such an antenna had its physics modified (spillover applied) yet served only a
partial-calibration accuracy claim with **no** off-axis honesty warning (a silent honesty gap).
Under the unified predicate a **partially-calibrated-but-surfaceless** antenna is treated as
uncorrected physics for **both** behaviors: spillover is folded in **and** the off-axis warning
fires beyond threshold. The P8 "don't stack a second warning where a correction surface already
provides an extrapolation warning" constraint is preserved exactly — keying on surface presence
means any surface-bearing antenna (any calibration status) stays silent.

**The model is a main-beam / peak-gain instrument.** Until 2026-07-12 its off-axis (sidelobe)
gain on the uncalibrated path was systematically *optimistic* (too low). **As of F7
(2026-07-12), uncalibrated-path off-axis gain instead includes a Ruze scattered-power sidelobe
floor (`model/pattern.rs::sidelobe_floor_gain`, applied as `max(pattern, floor)` at the
spillover seam in `compute_gain`, gated on `correction_surface.is_none()`) that lifts deep
off-axis nulls/sidelobes to a statistically calibrated best estimate — it tracks the measured
median sidelobe level, not a one-sided conservative bound (register decision revised
2026-07-12: link budget / G/T consumers need accuracy, and a one-sided upper bound is
anti-conservative for desired-signal margin).** It must still NOT be used as a precise
per-antenna prediction for interference, adjacent-satellite, off-axis-EIRP, or ACI analysis —
see the "Best estimate, not a per-antenna prediction" caveat below. (Original finding
2026-07-10; `antenna-model/tests/reference_validation.rs`,
`itu_r_s580_sidelobe_envelope_small_dish` and the `itu_probe_fine_envelope` diagnostic; F7
floor implemented 2026-07-12, branch `feat/f7-sidelobe-floor`.)

**⛔→✅ F7 PARKED 2026-07-13, RESOLVED-BY-P10 2026-07-15, REDESIGN LANDED 2026-07-16/17:** the
note below is the parked-status history. It was true while the served path still aliased.
**P10 landed 2026-07-15** and removed that blocker: the served integrator no longer aliases, so
the premise ("a `max()` floor cannot fire against an already-too-high pattern") no longer
holds. Per decision **D-2** (2026-07-15) P10 deliberately served the raw converged PO with the
**floor OFF** — *superseded 2026-07-16/17 by the F7 redesign: the floor is now ON for
uncorrected-physics antennas as an incoherent power sum* (see the banner at the top of this
section and the three-tier policy below). *Parked-status history follows:* the floor above is real code on the
`feat/f7-sidelobe-floor` branch, but it could not fire on the *served* path in 2026-07. Every
served gain used `IntegrationParams::fast()`, whose aperture integral aliased 20–35 dB too HIGH
beyond a few degrees off-boresight for electrically large dishes — a floor applied via `max()`
against an already-too-high pattern never engaged (confirmed: 0 of 6 real service geometries).
See `docs/findings-2026-07-13-off-axis-integration-aliasing.md` for the finding, the ~3200×-faster
Hankel-transform fix spike (shipped as P10), and what F7 must become now that P10 has landed (a
replacement for the idealised tail beyond a validity angle, not a `max()` floor over it).

**Calibrated and partially-calibrated antennas, and boresight/main-beam queries on any
antenna, are unaffected** — the floor is inert at boresight by construction (it only ever
raises deep nulls/sidelobes) and only applies where `correction_surface.is_none()`;
calibrated antennas' correction surfaces already absorb real sidelobe behavior within
coverage.

Why the underlying pattern is still optimistic without the floor: the live aperture-integral
pattern is an idealized, unblocked, perfect-surface, strut-free symmetric paraboloid. It
contains **none of the sidelobe-*raising* mechanisms** that dominate real wide-angle
sidelobes:

- central/feed **blockage** and quadripod **strut scatter** (unmodeled — see feature F3);
- aperture-**edge diffraction**;
- **surface-error scatter** — surface RMS is still applied as a *scalar Ruze gain-loss* in the
  aperture integrand (`surface_error = 0` in `integration.rs`); the physical mechanism (per-point
  phase scatter) is not modeled. **F7 closes the resulting gap at the population-statistics
  level**: the power the scalar Ruze efficiency removes from boresight is redistributed
  isotropically (`OMEGA_SCATTER = 4π`, the only power-conserving choice for a floor applied via
  `max()` at every angle — see `pattern.rs::OMEGA_SCATTER` doc comment), calibrated against
  measured wide-angle sidelobe statistics from NTIA Report 84-164 (22 C-band earth stations) and
  cross-checked against measured pattern peaks in NASA CR-159703 — see "F7 sidelobe floor"
  below. This bounds a *best estimate*, not any single antenna's exact pattern;
- cross-polarization.

**Evidence:** for the 3.7 m ground station at X-band (D/λ ≈ 99), the modeled sidelobe-*peak*
envelope falls at ~25·log₁₀(θ) — the **same slope** as the ITU-R S.580 mask (29 − 25·log₁₀θ) —
sitting roughly **8–13 dB below** it across 1°–20° (first sidelobe ≈ −31 dB rel. to peak at
~1.6°, reaching ≈ −62 dB by 19°). Matching *slope* confirms the aperture-taper diffraction
physics is right; the offset is the mask's regulatory headroom **plus** the missing real-world
sidelobe sources above.

**So the S.580 check validates pattern *shape*, not sidelobe *levels*.** It is a good regression
guard against gross pattern bugs (illumination errors, beam-steering sign flips, broken off-axis
integration — any of which break the slope or violate the mask). It runs with
`apply_sidelobe_floor: false` (the `IntegrationParams` default), so it exercises the raw
aperture-integral tail described above, unaffected by F7's floor. For off-axis fidelity, use
the calibration correction surface — or the ITU mask itself — as the sidelobe model, not the
physics engine's tail; the F7 floor (below) raises the *served* uncalibrated-path value to a
statistically calibrated best estimate but is not a substitute for either. **As of the F7
redesign (2026-07-16/17) the floor DOES engage on the served path** — it is combined with PO
as an incoherent power sum in the forward hemisphere and served alone (floor-only) behind the
dish, per the banner at the top of this section — a change from the pre-redesign state (D-2,
2026-07-15) where the floor was code-present but OFF.

**Numerical caveat — ✅ RESOLVED BY P10 (2026-07-15):** physical-optics far-sidelobe computation
needs the aperture-phase variation (∝ D·sinθ/λ) resolved — a naive fixed-density 2D grid is
infeasible for electrically huge dishes, which is what aliased the served path. **P10's Hankel /
azimuthal-mode integrator collapses the azimuthal integral in closed form (Jacobi–Anger),
dropping the cost class O((D/λ)²) → O(D/λ) and computing converged off-axis gain for dishes up
to 100 m in ~1 ms.** Far-sidelobe gain is now numerically computable at all angles on the served
path; the S.580 check still runs on the small 3.7 m dish (D/λ ≈ 99) as a shape regression guard,
but the "infeasible for huge dishes" limitation no longer applies to production serving.

**Roadmap (decided 2026-07-10, both now implemented — see below):** an off-axis honesty warning
on uncalibrated far-off-boresight queries = unit **P8**; a statistical envelope/floor model
(Ruze scatter floor) = feature **F7**, gated on its register row *and* on reference sidelobe
data (data gate met 2026-07-12 by unit F8, register row decided 2026-07-12); physical
mechanisms (edge diffraction, strut scatter) remain out of scope regardless (roadmap §6). An
optional ITU-mask envelope output mode was considered but not built as part of F7.

**P8 implemented (2026-07-12).** Queries on antennas with
`CalibrationStatus::Uncalibrated` that fall beyond the validated main-beam/near-in
region now carry an explicit warning on all four compute endpoints (gain, batch,
heatmap, h3-heatmap). Threshold: **3× the first-null angle**, with the first null
taken as **θ_null ≈ 1.6·λ/D radians** (tapered circular-aperture illumination;
uniform would be 1.22·λ/D) — beamwidth-relative, never a fixed angle (a 34-m Ka
beam is ~0.017° wide; a 3.7-m X-band beam ~0.9°). Implementation:
`service/evaluator.rs::off_axis_unvalidated_warning` (constants
`FIRST_NULL_COEFFICIENT = 1.6`, `OFF_AXIS_FIRST_NULL_MULTIPLE = 3.0`), called from
the gain pipeline (batch/heatmap inherit) and from the H3 per-cell path. Design
constraints honored: uncalibrated-only (calibrated/partially-calibrated
out-of-coverage queries already get the extrapolation warning — no stacking), and
the message is constant per (antenna, frequency) so heatmap/H3 aggregation
deduplicates it. C8 stage 3 converts the string to typed code
`off_axis_unvalidated`. **As of the F7 redesign (2026-07-16/17) the warning's wording is the
current truth:** the off-axis value is numerically converged/correct (the P10 integrator
replaced the aliasing quadrature) and is served as the incoherent power sum of that idealised
physical-optics value and the F7 statistical sidelobe floor (ON); the warning states the
remaining caveat — idealised PO omits blockage/strut/edge diffraction and the floor is a
population median, not a per-antenna measurement, so far-off-axis levels are
optimistic/approximate and "not calibrated-grade" — and points consumers at calibration data or
the ITU-R S.580 mask. (Earlier revisions: F7 2026-07-12 reworded it for the (then code-present
but not-served) scattered-power floor; the D-3 interim 2026-07-14 stated "numerically invalid"
while P10 was built; the P10/D-2 revision (2026-07-15) described raw PO with the floor OFF. All
three are superseded by the F7-redesign wording above.)

**F7 sidelobe floor implemented (2026-07-12, branch `feat/f7-sidelobe-floor`; parked
2026-07-13, see the note near the top of this section).** A Ruze scattered-power floor applies
**on the uncalibrated path only**
(`apply_sidelobe_floor = correction_surface.is_none()`, threaded through the gain, batch,
heatmap, and H3 pipelines identically) as `max(pattern, floor)` at the spillover seam in
`model/pattern.rs::compute_gain`. The floor itself
(`model/pattern.rs::sidelobe_floor_gain`):

- is derived from `surface_rms` via the same scalar Ruze efficiency (`1 − η_ruze`) already
  computed for the boresight gain-loss term — the scattered power has to go somewhere, and F7
  redistributes it **isotropically** over the whole sphere:
  `floor_linear = 1 − η_ruze` (i.e. `Ω_SCATTER = 4π`, `OMEGA_SCATTER` in `pattern.rs` — the
  only power-conserving choice for a floor applied via `max()` at every angle; an earlier
  cut used `Ω_SCATTER = 0.25 sr`, which implied 136–326% of the antenna's total radiated
  power and was reverted before this branch was parked);
- is **inert at boresight** and only ever lifts deep off-axis nulls/sidelobes (it is a lower
  bound via `max`, never reduces gain), and is **bounded by 0 dBi** (`p_scatter ≤ 1`), so it
  can never swamp a main beam or near-in sidelobe;
- uses a **single global** `Ω_SCATTER`, not a per-antenna surface correlation length — this is
  a deliberate simplification (planner default adopted 2026-07-12); a per-antenna width is
  deferred roadmap unit **F9**;
- is a **best estimate**, not a conservative envelope (register decision revised
  2026-07-12), calibrated against **NTIA Report 84-164** measured sidelobe statistics: for a
  representative ~1 mm-surface C-band earth station the floor tracks the wide-angle **median**
  sidelobe level to within a band-mean ≈2.5 dB (−2.0 dB at 3950 MHz, +2.9 dB at 6175 MHz;
  ±6 dB per angular bin) — pinned by
  `reference_validation::sidelobe_floor_tracks_measured_median`, which also asserts power
  conservation and the 0 dBi ceiling; the surface-RMS scaling direction is cross-checked
  against **NASA CR-159703** measured pattern peaks by
  `reference_validation::sidelobe_floor_surface_scaling_matches_nasa`.

**Best estimate, not a per-antenna prediction:** the floor tracks a *population* median with
±2.5–6 dB scatter — it is not tuned to any single antenna's actual sidelobe pattern, does not
model the physical mechanisms that raise real sidelobes (edge diffraction, strut scatter —
still out of scope, roadmap §6), and does not improve the near-in first-sidelobe or the
detailed shape of the pattern (those remain governed by the raw aperture integral / calibration
correction surface as before). For a one-sided conservative bound (regulatory/interference
screening), use the ITU mask or calibration data instead — the floor is deliberately not that.
`PHYSICS_MODEL_VERSION` bumped 2 → 3 for this change. No request/response schema field was
added (only warning-description text in `openapi.yaml` was refreshed) — the floor changes
served gain values only, gated silently on `calibration_status`, and — per the P10 note above —
did not, AT THAT TIME (2026-07-12, before P10 fixed the aliasing), change anything actually
served, because the served pattern it was `max`'d against was already aliased higher than the
floor at every angle tested. **Superseded 2026-07-16/17 by the F7 redesign:** P10 (2026-07-15)
fixed the aliasing and the F7 redesign replaced the `max()` combination with an incoherent power
sum, gated on `physics_is_uncorrected()` — the floor now materially changes served values (see
the banner at the top of this section); `PHYSICS_MODEL_VERSION` is now 5.

### Three-tier off-axis policy (P10-tail, 2026-07-15)

The served angular range is now governed by three explicit tiers:

1. **θ ≤ ~3 first-nulls (≈ 3·1.6·λ/D) — VALIDATED.** Main beam / near-in region, <1 dB
   against measurements. No off-axis warning.
2. **3 first-nulls < θ ≤ 90° — IDEALISED-PO, WARNED.** Numerically converged (post-P10) but
   idealised physical optics (no blockage/strut/edge diffraction); the existing off-axis
   honesty warning fires on **uncalibrated** antennas. Pattern shape validated, absolute
   levels optimistic / not calibrated-grade.
3. **θ > 90° — REAR HEMISPHERE, HARD WARNING.** The value is still served (grid totality on
   `/heatmap` and `/h3-heatmap` must be preserved) but is **categorically outside physical
   validity**: PO from an unshadowed aperture is physically meaningless behind a reflector
   *regardless of numerical convergence* — there is no rim diffraction and no dish shadowing
   of the aperture field. **As of F7 (2026-07-16), what is actually served here has changed:**
   on antennas served with **uncorrected physics** (`physics_is_uncorrected()` — currently all
   4 enabled antennas), the rear aperture integration is **skipped entirely** and the served
   value **is the statistical sidelobe floor only** — the PO term is excluded, not merely
   suppressed. On antennas **with a correction surface**, PO is still computed and returned as
   a numerical extrapolation (unchanged from P10-tail). A **rear-hemisphere hard warning**
   (`service/evaluator.rs::rear_hemisphere_warning`, wired into the gain pipeline and the H3
   per-cell path exactly like the off-axis warning) fires for **ANY** antenna — **including
   fully calibrated ones**, because a forward-hemisphere correction surface says nothing about
   back lobes — and its wording now branches on which of the two cases above applies. It is
   gated purely geometrically at `|θ| > 90°` (not on calibration status) and its message is
   constant per (antenna, frequency) so heatmap/H3 aggregation deduplicates it. C8 will later
   convert it to typed code `rear_hemisphere_invalid`.

   **P10-tail radial-budget fix that makes this honest:** `integration.rs::radial_points_for`
   counts the **dish-depth chirp** `k·ρ²/(4f)·(1−cosθ)` (= `(R²/(4fλ))·(1−cosθ)` radial
   cycles) in the sample budget. Forward it is subdominant (why every P10 test passed without
   it); behind the dish it inverts — as θ→180° the `sinθ` kernel budget collapses toward the
   floor while this chirp peaks at ~`R²/(2fλ)` cycles — so without it the N-vs-2N self-check
   would silently under-sample the rear integral. This remains load-bearing for **corrected**
   antennas, which still run the rear PO integral; it is now moot for uncorrected antennas,
   whose rear integration is skipped.

   **HISTORY — diagnostic finding (2026-07-15), RESOLVED by the obliquity factor (F7,
   2026-07-16):** before the Huygens obliquity factor `(1+cosθ)/2` was added, the rear PO value
   was GENUINELY CONVERGED at θ≈120°/163° (verified stable to <0.1 dB against a 20 001-point
   forced density) and yet read a physically-meaningless **+7…+13 dBi** backlobe on every
   enabled antenna at θ≈163° — only ~28 dB below peak for the small dishes. Convergence
   therefore **could not** flag rear invalidity by itself; the *warning* was the safety net,
   not a numerical check or a level bound. The obliquity factor (see handoff 2 below, now DONE)
   suppresses that fictitious backlobe by ~33 dB at 163°, and the uncorrected-physics path no
   longer serves the PO term there at all. (At θ=180° the base-density self-check still reports
   `converged=false` for the corrected-antenna PO path.) Test:
   `reference_validation::p10_served_rear_hemisphere_is_physical_or_flagged`.

**Three handoffs to the F7 redesign (do NOT touch in P10-tail):**

1. **Rear becomes floor-dominated once F7 lands.** The digitized NTIA 84-164 dataset spans
   1°–180°, so the salvaged statistical floor's calibration already covers the back hemisphere.
   F7 should consider **EXCLUDING the PO term from its power-sum for θ > 90°** rather than
   letting a meaningless PO backlobe compete with the statistical floor.
   **✅ DONE 2026-07-16 (F7):** decided and implemented as described in tier 3 above — on
   uncorrected-physics antennas the rear PO term is excluded (integration skipped), not merely
   out-competed by a `max`/power-sum.
2. **The integrand has no Huygens obliquity factor `(1+cosθ)/2`.** Physically that factor
   suppresses an aperture field's backward radiation; without it, rear-hemisphere PO is
   extra-wrong (it is the direct cause of the converged +7…+13 dBi backlobes above), and
   forward wide-angle levels may be up to ~6 dB hot near 90°. Adding it would **shift the θ=90°
   internal-consistency anchors**, so it is a physics decision for F7's "what to serve far
   off-axis" scope — flagged here, deliberately **not** changed in P10-tail (which is a
   sampling-density change only, never an integrand/physics-math change).
   **✅ DONE 2026-07-16 (F7):** the obliquity factor `(1+cosθ)/2` is now applied as a field
   factor in `pattern.rs::absolute_gain_from_integral` (θ=0 unchanged, −6.02 dB at 90°,
   suppresses the old fictitious rear backlobe by ~33 dB at 163°). The θ=90° anchors were
   re-derived under the new factor as part of this same change.
3. **Boresight-tuner → floor coupling (F7 ship precondition 2, BOUNDED 2026-07-16).**
   `calibrate/src/boresight_calibration.rs` tunes `surface_rms` within [0.3x, 3x] of the
   design value as a catch-all for boresight gain deficits, and the F7 floor
   `(1 − η_ruze)·η_mesh` converts that σ directly into off-axis power. The coupling is
   analytically bounded: σ→3σ multiplies the floor by at most 9 (**+9.54 dB**, small-σ
   limit; measured worst case across the enabled antennas: +9.54 dB — see
   `reference_validation::sidelobe_floor_tuner_coupling_bounded`), and the floor is always
   capped at 0 dBi (Ω = 4π power conservation). Judged acceptable to ship because the
   floor is a best-estimate median (±6 dB/bin NTIA tracking band) and the tuner bound is
   within that band's width; if a boresight-tuned antenna without a correction surface
   ever ships with anomalous off-axis levels, constrain the tuner's σ range first.

### Large feed offsets (> 0.5·f): ray-tracing stub (P3, decided 2026-07-16)

This is a **separate axis** from the off-boresight *emitter* angle θ above: it concerns how far
the **feed** is displaced from the focal point, i.e. the angular gap between the request's
`feed_position` (a pointing target) and the `reflector_boresight`, mapped to a physical
displacement (`compute_feed_position_from_pointing` → `≈ 2·f·tan(cone/2)/bdf`). The offset ratio
is geometry-driven and effectively antenna-independent.

| Feed offset | Mode | Fidelity |
|---|---|---|
| ≤ 0.3·f | `StandardPhysicalOptics` | exact coma phase + spillover modeled |
| 0.3·f – 0.5·f | `StandardPhysicalOptics` | exact coma phase; spillover **not** modeled (unvalidated there) — carries its own degraded-accuracy warning |
| **> 0.5·f** | **`RayTracing` stub** (`model/ray_trace.rs`) | **acknowledged stub — gain accuracy degraded** |

Beyond 0.5·f the model routes to `model/ray_trace.rs`, which samples the aperture but does **not**
implement true spillover or geometric ray–reflector intersection (`ray_trace.rs` TODO). Real ray
tracing is gated as feature **F2** and is not implemented. Per maintainer decision **P3
(2026-07-16): document + flag** — the request is **not rejected** (warn-don't-refuse; `/heatmap`
and `/h3-heatmap` grid totality must be preserved), but every such result carries the honest
degraded-accuracy warning `model::pattern::RAY_TRACING_STUB_WARNING` ("…not fully implemented;
gain accuracy may be degraded"). `spillover_loss_db` is `null` in this regime (see the offset gate
above).

**The warning reaches all four compute endpoints.** On `/gain`, `/gain/batch`, and `/heatmap` the
model pushes it directly (those paths run the integrator per query/point). `/h3-heatmap` caches
physics-only gain and runs the model only on a cache **miss**, so it re-emits the identical warning
at the service layer (`service/evaluator.rs::ray_trace_stub_warning`) **outside** the gain-cache
closure — exactly as the P8 off-axis and P10-tail rear-hemisphere warnings are re-emitted — so the
warning survives cache **hits** too. The message is constant per antenna config, so heatmap/H3
aggregation deduplicates it to a single entry. Pinned by
`tests/integration/ray_trace_stub_warning_tests.rs` (one test per endpoint + a warm-cache H3 test +
a small-offset negative control). `ray_trace.rs` math is untouched by P3.

## Open items surfaced while mining (not fixed here)

- **Resolved 2026-07-10 (roadmap P7, implemented).** `phase_center_offset` is now a
  compensated (no gain effect) recorded feed property; deliberate defocus is expressed
  via the new `axial_defocus` field. See both glossary rows above and
  `docs/findings-2026-07-10-ka-phase-center-defocus.md` for the diagnosis that motivated
  the change. The standalone `illumination::phase_center_offset_phase` function mentioned
  in earlier drafts of this contract was a separate, unused implementation (dead code) —
  it has since been **removed** (grep-verified 2026-07-10: zero hits for
  `phase_center_offset_phase` in any `.rs` file). Recorded here as history, not as
  an open item.
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
