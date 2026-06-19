# Comprehensive Review: Antenna Model Service

**Date:** 2026-06-10
**Scope:** Full-application review for implementation errors, incorrect or incomplete modeling, and anything degrading consistency or accuracy of results. Covers the physics engine (`antenna-model/src/model/`), service layer (`antenna-model/src/service/`), coordinate pipeline, data loading, and the `calibrate` tool.

**Summary:** The numerical machinery (integration, splines, WGS84 round-trips) is mostly well built, but several errors materially break accuracy and consistency: a spurious quadratic phase term that corrupts every off-axis pattern, a physically inverted (and discontinuous) mesh-loss model, a calibration artifact format the service cannot load, an ECEF auto-detection threshold that silently misclassifies satellite positions, and azimuth-convention mismatches that silently disable or misapply correction surfaces.

---

## Critical — wrong results or broken pipeline

### 1. Spurious quadratic phase in `phase_path` corrupts all off-axis patterns

`antenna-model/src/model/phase.rs:88-99` computes:

```text
Ψ = k·[ρ²/(4f) − ρ·sinθ·cos(φ−φ′)]
```

For a parabola fed at focus, the aperture is equiphase at boresight — that is the defining property of a parabola. Deriving the phase properly (feed→surface path `k(f+z)` minus far-field projection `k(ρ sinθ cos(φ−φ′) + z cosθ)`), the quadratic term should be:

```text
k·ρ²/(4f)·(1−cosθ)
```

which vanishes at θ=0. As written, the integrand carries a constant defocus of `k·ρ²/(4f)` at **all** angles — for the 1 m test dish at 8.4 GHz that is ~3.5 wavelengths (22 rad) of spurious defocus; for a 34 m dish at X-band it is hundreds of radians.

Because `compute_gain_standard` (`pattern.rs:302`) normalizes against an *ideal reference computed with the same wrong phase*, the boresight value looks plausible — but the relative pattern (beamwidth, sidelobe structure, coma-lobe shape; everything the <1 dB main-lobe/sidelobe accuracy requirement depends on) is the pattern of a badly defocused aperture. The existing HPBW test passes only because its tolerance is 0.5°–10°.

Note: the design doc apparently states the same formula (Section 2.2), so the error may be upstream of the code. Re-validate the design doc as part of the fix.

### 2. The `full` calibration pipeline produces artifacts the service cannot read

- The calibrate tool's full mode (`calibrate/src/main.rs:648` → `serializer.rs`) writes `"ANTC"` magic + version + CRC32 + length, then a serde-bincode `CalibrationArtifact` whose correction surface is a **3D** spline over (frequency, E-cone, E-clock) (`calibrate/src/correction_surface.rs:148`, `shape: [n_freq, n_cone, n_clock]`).
- The service loader (`antenna-model/src/data/loader.rs:44`) reads the file as **headerless** bincode straight into `AntennaCalibration`, whose `BSplineModel4D` is a **4D** spline over (azimuth, elevation, frequency, temperature).

These are incompatible at every level: header, struct shape, dimensionality, and axis semantics. Only the `boresight` mode (`main.rs:317`, which encodes the service's own types) produces loadable artifacts. The service also never verifies the CRC the tool writes.

### 3. ECEF auto-detection silently mangles geodetic satellite positions

The detection threshold is 1000 km (`antenna-model/src/api/schemas.rs:115-125`), not the 6400 km documented in CLAUDE.md and `coordinates_3d.rs:76`. Meanwhile `validate_geodetic` accepts altitudes up to 400,000 km. A vehicle position given as `(lon, lat, 35_786_000)` — a GEO satellite in geodetic form — has `z > 1e6` and is silently treated as an ECEF point near Earth's center. No error, no warning; the resulting geometry (and gain) is garbage. Any geodetic position above 1000 km altitude hits this.

### 4. Mesh "transparency" model is physically inverted and discontinuous

`pattern.rs:140-150`: for `λ ≤ π·spacing` (high frequency — where a real mesh leaks the most) it returns **1.0 (no loss)**; just above the cutoff it returns 0.5. That is a 3 dB step discontinuity in gain vs. frequency at `λ₀ = π·d`, with the loss curve trending the wrong way on the high-frequency side. The doc-comment contradicts itself ("opaque" and "poor reflector, lets energy through" both map to ~1.0).

A much more complete mesh model exists in `model/mesh.rs` (`mesh_efficiency`, wire-diameter and angle-of-incidence corrections) but is **not** wired into the gain path.

### 5. Azimuth conventions are inconsistent end-to-end — corrections get skipped or misapplied

- `compute_emitter_direction` returns azimuth in (−180°, 180°] (atan2), but coverage ranges and correction-surface knots use 0–360° (see test data and `ValidityRanges`). `is_in_coverage` (`service/evaluator.rs:370-387`) does a plain interval test with no normalization, so a query at az = −90° (≡ 270°) is declared out of coverage and the correction surface is silently dropped for half the azimuth space.
- More fundamentally, the antenna-frame X-axis (azimuth zero) is constructed as `cross(Earth-Z, boresight)` (`coordinates_3d.rs:383-401`) — an arbitrary reference that does not use vehicle attitude at all (CLAUDE.md says attitude should drive this), and which **switches reference vectors discontinuously** when the boresight z-component crosses 0.99. Calibration data is keyed to the measured E-clock zero; the runtime azimuth zero is unrelated to it. Any azimuth-dependent quantity — correction surface lookup, coma-lobe orientation via `feed_displacement_angle` — is therefore inconsistent between calibration and runtime, and can jump discontinuously as the vehicle moves.

### 6. `is_in_coverage` contradicts its own contract and disables corrections for fully-calibrated antennas

The doc-comment (`evaluator.rs:368`) says "for fully calibrated antennas (no coverage specified), always returns true," but the code returns `false` for `None`. Since the correction is only applied when `correction_surface.is_some() && is_in_coverage(...)` (`evaluator.rs:278`), any artifact carrying a correction surface but no `calibration_coverage` — `Option` in `data/types.rs:63`, never written by the full-mode serializer — has its correction surface silently ignored on every request (with only a warning string).

---

## High — accuracy degradation

### 7. Absolute gain is pinned to a hardcoded 0.55 aperture efficiency

`compute_gain_standard` (`pattern.rs:360`) uses the physics integral only as a *relative* pattern and sets the absolute level to `theoretical_max_gain(D, λ, 0.55)`. The feed q-factor, illumination taper, and spillover therefore never affect absolute gain — defeating much of the point of the physical-optics model and of tuning q in calibration. Worse, the `reference_gain_db` in the evaluator (`evaluator.rs:310-316`) uses a *different* efficiency definition (Ruze×mesh, without the 0.55), so the reported `loss_db` carries a built-in ~2.6 dB offset that is not pointing loss.

### 8. H3 link budget never applies the correction surface, but reports that it did

`service/h3_link_budget.rs` calls `compute_gain_db` directly (physics only), yet sets `correction_applied = calibration.correction_surface.is_some()` (line 351). Heatmap gains will disagree with the `/gain` endpoint for the same direction on calibrated antennas, and the metadata actively misreports it.

### 9. Beam squint correction is applied to the wrong axis

`apply_beam_squint_correction` (`coordinates_3d.rs:573-606`) always adds the squint to elevation "for simplicity," regardless of the feed displacement clock angle that actually determines the squint direction. It can also drive the polar-angle elevation negative. The 10× frequency-ratio test case produces 38° of "correction" applied in an essentially arbitrary direction.

### 10. Unit bug: `EClockConeCoordinates::to_degrees` converts the wrong way

`model/coordinates.rs:137-139` calls `.to_radians()` on values already in radians. Any caller gets values ~3283× too small.

### 11. Defocus from feed axial offset is computed, then discarded

Steering yields `z_feed = f − d²/(4f)` (`coordinates.rs:217`), but the integrand uses only the radial displacement (`integration.rs:471`) and `phase_feed_displacement` hard-assumes the feed sits at z = f exactly. Axial defocus never enters the phase model anywhere, despite being second-order in the documented coma model.

### 12. Pole singularity in `ecef_to_geodetic`

Altitude is computed as `p/cos(lat) − N` (`coordinates_3d.rs:237`) — 0/0 at the poles. It happens to survive the unit test by floating-point luck, but it is ill-conditioned for any high-latitude query. Standard fix: `alt = z/sin(lat) − N(1−e²)` near the poles.

### 13. Illumination model omits space attenuation; edge-taper docs are self-contradictory

`illumination_amplitude` uses only `cos^q(ψ)`; the 1/r feed-to-reflector spreading loss (the `(1+cosψ)/2`-type factor) is absent, underestimating edge taper. The module header claims "q ≈ 6–8 for 10 dB edge taper" while its own `edge_taper_db(8.0, 0.5)` returns ≈ −35 dB — a power-vs-amplitude convention confusion that will mislead anyone setting q.

### 14. Duplicate, divergent surface-error implementations

`phase.rs` has its own `ZernikeSurface` that claims Noll ordering but is a 0-indexed, un-normalized variant (its `rms()` ≈ L2-norm claim only holds for normalized polynomials), plus a `GaussianSurface` that is actually uniform-distributed and ignores its correlation length. A correct Noll implementation exists in `model/surface.rs` but is not used by the phase path. In practice the integrand passes `surface_error = 0.0` always (`integration.rs:497`), so surface error is handled purely statistically via Ruze; the trait machinery is dead code that invites accidental misuse.

### 15. `feed_offset_meters` contains degrees

`evaluator.rs:124-128` packs (Δaz°, Δel°, angular magnitude) into a `Vector3D` reported as meters in the API response.

---

## Medium — robustness, consistency, performance

- **Reference field recomputed every request:** `compute_gain_standard` re-runs the full ideal on-axis integration per call (`pattern.rs:341`), doubling the cost of the hottest path against a <100 ms p95 budget. It depends only on (config, frequency) and is trivially cacheable.
- **Silent integration non-convergence:** `integrate_aperture` returns the last iterate without any warning when it exhausts iterations (`integration.rs:317-325`), and the fallback `error_estimate = |result|·tol` is meaningless.
- **Knot-vector panics:** `find_knot_span` (`correction_interpolator.rs:194-199`) indexes `knots[order−1]` and `knots[len−order]` with no length validation — a malformed or short knot vector (like the 2-element ones used in the service's own tests) panics instead of erroring. Artifact loading never validates knots vs. `spline_order`.
- **`compute_beamwidth`** accepts the first point within 0.1 dB of target during binary search and assumes monotonicity — it can lock onto a sidelobe (acknowledged in its docs, but nothing guards against it).
- **Cache staleness:** `GainCacheKey` quantization is fine, but nothing invalidates per-feed caches if calibration data is reloaded; H3 cache hits also drop warnings (documented, but it means warning output is request-order dependent).
- **Ray-tracing and direct-path modes are stubs** (`pattern.rs:282-288` warns for ray tracing; spillover/geometric intersection unimplemented), so feed offsets >0.5f produce results of unknown quality. It warns, which is fine, but worth tracking.
- **Doc/constant mismatch:** `MAX_ALTITUDE_M` comment says "4000000 km" for a 400,000 km constant (`coordinates_3d.rs:62`).
- **`loss_db` in H3 cells** is referenced to the grid's center cell (the feed's ground target), not the actual beam peak — a reasonable approximation, but it is labeled as if it were peak-referenced.

---

## What's solid

The WGS84 geodetic↔ECEF transforms (away from the poles), the Cox-de Boor basis evaluation, the Simpson integration core, the Ruze equation, `theoretical_max_gain`, the LRU cache concurrency design, and the fact that the calibration tuner optimizes against the *same* `compute_g_over_t` the service uses (so model bias is partially absorbed at calibrated points) are all correctly done.

## Recommended starting order

1. Fix `phase_path` (add the `(1−cosθ)` factor) — everything pattern-shaped depends on it, and re-validate the design doc's Section 2.2 formula.
2. Unify the calibration artifact format: pick the service's `AntennaCalibration` + `BSplineModel4D`, make `full` mode emit it, verify the CRC on load, and always write `calibration_coverage`.
3. Normalize azimuth to one convention at the service boundary, and define the antenna-frame X-axis from vehicle attitude rather than Earth-Z cross products.
4. Replace `pattern.rs::mesh_transparency` with the model already in `model/mesh.rs`.
5. Raise the ECEF detection threshold (or better, make the coordinate system explicit in the API) — auto-detection with a 1000 km threshold cannot coexist with geodetic satellite altitudes.
