# Parabolic Dish Antenna Model Design Document

> **Document status (audited 2026-08-19, roadmap unit D5).** This is the *original
> design document*. It was written ahead of the implementation and parts of it were
> never built, or were built differently after the physics was re-derived. Every
> section has been checked against the code as of 2026-08-19 and is now marked:
>
> - **Corrected** — the text has been rewritten to describe what the code does, with
>   the superseded design noted and the reason it changed.
> - **Historical — not implemented** — the design was never built (or was built and
>   removed). The text is kept for provenance; **do not implement from it** without
>   filing a decision item first.
>
> Unmarked text was verified to match the code. When this document and the code
> disagree in the future, the code and
> [`docs/domain-contract.md`](domain-contract.md) win — file the disagreement as a
> decision item in [`roadmap-2026-07.md`](roadmap-2026-07.md) rather than editing
> either to match the other.

## Executive Summary

This document outlines the design for a high-performance antenna gain model for parabolic dish antennas with steerable feeds. The system targets satellite communication applications across 100 MHz - 50 GHz with flexible accuracy requirements based on calibration status:

- **Fully Calibrated**: ±1 dB accuracy (main lobe and first sidelobe)
- **Partially Calibrated (Boresight)**: ±1 dB at boresight, ±2-3 dB off-axis, ±1-2 dB loss
- **Partially Calibrated (Limited Coverage)**: ±1-1.5 dB in-coverage, ±2-3 dB extrapolated
- **Uncalibrated (Design Specs)**: ±3-5 dB absolute gain, ±2-3 dB loss

The system supports graceful degradation from fully calibrated to uncalibrated antennas, prioritizing **loss accuracy** (reference_gain - actual_gain) where systematic errors cancel.

## 1. System Requirements

### 1.1 Performance Requirements
- **Accuracy** (varies by calibration status):
  - **Fully Calibrated**: ±1 dB for main lobe and first sidelobe (down to -30 dB from peak)
  - **Partially Calibrated (Boresight)**: ±1 dB at boresight, ±2-3 dB off-axis, ±1-2 dB loss
  - **Partially Calibrated (Limited Coverage)**: ±1-1.5 dB in-coverage, ±2-3 dB out-of-coverage
  - **Uncalibrated**: ±3-5 dB absolute gain, ±2-3 dB loss (error cancellation)
- **Frequency Range**: 100 MHz to 50 GHz (500:1 ratio)
- **Computation Speed**: 1-20 evaluations per second (all calibration statuses)
- **Platform**: Rust implementation
  - *Historical — not implemented:* the original requirement read "with GPU
    acceleration support". There is no GPU path and none is planned; the tree contains
    no GPU dependency. The off-axis integrator's cost was instead brought down 2.4–7.4×
    on the CPU by roadmap **P10-perf** (FFT φ' transform, one-sweep Jₘ ladder).
- **Memory**: Unconstrained (lookup tables permitted)
  - *Corrected:* the service targets a **<512 MB** footprint (see `docs/architecture.md`
    and CLAUDE.md); "unconstrained" describes the modelling maths, not the deployed
    service.

### 1.2 Antenna Configuration
- **Reflector Type**: Mesh parabolic reflector
- **f/D Ratio**: ~0.5 (focal length ≈ half the dish diameter)
- **Feed System**: Moveable feeds (not focal plane array)
- **Feed Displacement Range**: Up to ±D/2 from focal point
- **Quality**: High-accuracy, high-quality components assumed

### 1.3 Use Cases
1. **Primary**: Compute antenna gain from 3D geometric configuration
   - Given: Vehicle position (ECEF or Geodetic), vehicle attitude (quaternion/Euler)
   - Given: Reflector boresight position, feed position, emitter position (all 3D coordinates)
   - Given: Operating frequency and optional pointing frequency
   - Compute: Absolute gain at emitter position
   - Optionally compute: Reference gain (ideal: feed at focus, pointing at emitter)
   - Optionally compute: Loss = reference gain - actual gain
   - Support: Multiple feeds per antenna (composite antenna_id + feed_id identifier)
   - Support: Beam squint correction for pointing frequency ≠ operating frequency
   - **Support: All calibration statuses** (fully calibrated, partially calibrated, uncalibrated)
   - **Return: Calibration status and accuracy estimates** in API responses

2. **Extended**: Generate loss heatmaps across antenna field of view
   - Generate grid of emitter positions (rectangular azimuth/elevation or H3 hexagonal)
   - *Corrected:* these are **two endpoints**, not one grid-type parameter. `/heatmap`
     serves rectangular grids only — its `h3` grid type was a `not_implemented` stub and
     was removed 2026-07-28 (roadmap C8 stage 4) — and `/h3-heatmap` serves the H3
     hexagonal link budget. Merging them is filed as feature **F5** and is undecided.
   - Compute loss relative to peak gain for each grid point
   - Support field-of-view clipping based on antenna beamwidth
   - **Include calibration status information** in heatmap responses

3. **Coordinate System Flexibility**:
   - ~~Auto-detect ECEF (Earth-Centered Earth-Fixed) vs Geodetic (lon, lat, alt) coordinates~~
     *Corrected 2026-07-27 (roadmap C8 stage 2):* the frame is **declared, never
     inferred**. Every `Position3D` carries a required `coordinate_system` tag
     (`"ecef"` or `"geodetic"`); omitting it is a 400 naming the field. The magnitude
     heuristic was deleted because no threshold can separate a geodetic GEO satellite
     from an ECEF point — it silently returned a wrong gain when it guessed. See
     `docs/domain-contract.md`.
   - Transform all positions to antenna frame for gain computation
   - Handle vehicle attitude for proper coordinate frame alignment

4. **Calibration Upgrade Path** (NEW):
   - **Deploy uncalibrated antennas** with design specs only (±3-5 dB absolute, ±2-3 dB loss)
   - **Quick boresight calibration** (~1 hour test time) for ±1-2 dB loss accuracy
   - **Incremental upgrades** to partial or full calibration as measurements become available
   - **No service downtime** for calibration updates

## 2. Mathematical Formulation

### 2.1 Core Physical Optics Model

The far-field electric field pattern:
```
E(θ,φ) = (jk·exp(-jkr))/(2λr) ∬_Aperture A(ρ,φ') · exp[jΨ(ρ,φ')] · ρ dρ dφ'
```

### 2.2 Phase Components

Total phase function:
```
Ψ_total = Ψ_path + Ψ_feed_displacement + Ψ_surface + Ψ_mesh
```

#### Path Phase (Standard):
```
Ψ_path = k·[ρ²/(4f)·(1−cosθ) − ρ·sinθ·cos(φ−φ')]
```

**Derivation:** The feed→surface optical path is k(f + z) with z = ρ²/(4f). The
far-field projection removes k(ρ sinθ cos(φ−φ') + z cosθ). Dropping the constant
kf gives the formula above. The (1−cosθ) factor is essential: it ensures the aperture
is equiphase at θ = 0, which is the defining optical property of a parabola.

*Note (2026-06-11): The earlier formula omitted (1−cosθ), which injected a
spurious defocus across the aperture, corrupting off-axis pattern shape.*

#### Feed Displacement Phase (Coma Aberration):

**Corrected — the shipped model is the exact path-length difference, not the series
below.** `model/phase.rs::phase_feed_displacement` computes, for each aperture point
`(x, y, z) = (ρcosφ', ρsinφ', ρ²/4f)`:

```
Ψ_feed_displacement = k·(|P − F_displaced| − |P − F_ideal|)
  F_ideal     = (0, 0, f)
  F_displaced = (δ·cosα, δ·sinα, f + δz)
```

This is exact in the displacement, so it carries **all** orders of aberration —
beam steering (first), defocus/astigmatism (second, and the only route by which the
axial offset `δz` enters), coma (third), and higher — as an exact function of `(δ, α, δz)`.
The completeness pin `edge_cases::exact_feed_displacement_phase_contains_all_low_order_aberrations`
proves that content against an independent closed form.

*Historical — not implemented:* the original design gave the truncated series

```
Ψ_feed_displacement = k·δ_feed·[ρ/(2f)]·[2·cos(α) - (ρ/(2f))·cos(2α-φ')]
```

— a small-displacement expansion, and **not one to trust or re-derive from**: as written
its leading term depends on `cos α` alone, with no `φ'` dependence at all, where the exact
phase (and standard coma theory) varies as `cos(φ' − α)` around the aperture. It was
superseded because the exact form costs no more to evaluate and is valid out to the
model's PO scope boundary (0.5f). That exactness is also *why* roadmap **P2** deleted the
former `HigherOrderAberrations` mode: layering heuristic Seidel terms on an already-exact
phase double-counts (see §3.1).

#### Surface Error Phase:

*Historical — not implemented as a per-point aperture phase.* `phase_surface_error`
exists and implements the formula below, but the served aperture integrand passes
`surface_error = 0.0` at every point (`model/integration.rs`). Surface error is applied
**statistically**, as the Ruze efficiency `η = exp(−(4πσ/λ)²)` in `model/pattern.rs`
(§2.4), and systematic surface deviations are absorbed by the calibration correction
surface. There is no Zernike or measured-surface map anywhere in the tree; see §4.4.

```
Ψ_surface = (4π/λ)·ε(ρ,φ')·cos(θ_incident)
```

#### Mesh-Specific Phase:

Implemented and live in the aperture integrand (`phase_mesh`, applied whenever the
antenna declares mesh parameters), with `θ_incident ≈ ρ/(2f)`:

```
Ψ_mesh = arctan[(2π·d_mesh/λ)·sin(θ_incident)]
```

### 2.3 Illumination Function

Feed pattern model using cos^q approximation:
```
F_feed(ψ) = cos(ψ)^q  for ψ < π/2, else 0
```

**Corrected — the amplitude also carries a space-attenuation factor, and the edge-taper
figure was a power-vs-amplitude confusion.** `illumination_amplitude` returns

```
A(ψ) = cos(ψ)^q · (1 + cos ψ)/2
```

where `(1+cosψ)/2` accounts for the feed-to-surface distance growing toward the rim
(`r = 2f/(1+cosψ)` for a parabola), normalized to unity at boresight. With that factor
and the `20·log₁₀` amplitude convention `edge_taper_db` uses, **q = 8 at f/D = 0.5 gives
≈ −37.4 dB** at the rim, not −10 dB.

**A taper band cannot be quoted without naming the f/D** — the rim half-angle, and so
`cos ψ_edge`, is set by it. Computed from `edge_taper_db`:

| f/D | rim `cos ψ` | q = 6 | q = 8 | q = 12 |
|---|---|---|---|---|
| 0.4 | 0.438 | −45.9 dB | −60.2 dB | −88.9 dB |
| 0.5 | 0.600 | −28.6 dB | −37.4 dB | −55.2 dB |
| 0.6 | 0.704 | −19.7 dB | −25.8 dB | −38.0 dB |

*(An earlier draft of this correction quoted "roughly −25 to −45 dB for q = 6–12" with no
f/D. That band is about right at f/D ≈ 0.55–0.6 and wrong by 17 dB at the low end for the
f/D = 0.5 anchored two sentences above. It was taken from `edge_taper_db`'s own doc
comment — which carries the same unqualified band, and which a docs-only unit did not
edit; flagged in D5's closeout. Recompute, do not quote.)*

The original "q ≈ 6-10 for typical 10 dB edge taper" was inherited from a cos^q-only,
power-convention reading and is wrong in both halves (it was finding 13 of the
2026-06-10 review). **The q-factors this tree actually ships are 1.14–3.15**, derived per
antenna from `q_factor_from_taper` for a ~−11 dB edge taper at that antenna's own f/D
(1.14 at f/D 0.4, 1.39 at 0.43, 2.04 at 0.5, 3.15 at 0.6 — see the comments beside each
`q_factor` in `calibration_data/antennas.yaml`). They were 8–11 before that correction,
which is where the "6-10" came from. Use `q_factor_from_taper`, never the rule of thumb.

### 2.4 Mesh Reflector Efficiency

Ruze's equation for surface errors:
```
η = exp(-(4π·σ/λ)²)
```

**Corrected — the mesh model is an inductive-grid reflectivity with no cutoff.** The
original design gave a "transparency"

```
T = 1/(1 + (λ₀/λ)²)  for λ > 10·mesh_spacing
```

whose implementation was physically inverted (it returned *no* loss at high frequency,
where a real mesh leaks most) and carried a 3 dB step discontinuity at the cutoff —
finding 4 of the 2026-06-10 review. It was replaced (`ad25817`) by the Wait/Marcuvitz
inductive-grid shunt model in `model/mesh.rs`, which is what
`pattern::overall_efficiency` consumes:

```
X    = (g/λ)·ln(g/(π·d))          g = wire spacing, d = wire diameter
|R|² = 1/(1 + 4·X²)
```

It is continuous everywhere, monotonic in λ, and tends to the solid-reflector limit
(|R|² → 1) as λ → ∞. There is **no** `λ > 10·mesh_spacing` validity cutoff — the
piecewise definition was the source of the discontinuity. Full Floquet-mode analysis
of the transition region is *historical — not implemented* (see §3.1).

### 2.5 Coordinate Transformations

E-clock/E-cone to physical feed position. A lateral feed displacement steers
the beam to the OPPOSITE side, so to aim the beam at clock angle `clock_angle`
the feed is displaced at `clock_angle + 180°` (hence the negative x/y). The
displacement is divided by the beam deviation factor (BDF, Lo 1960) so the PO
beam peak lands at the requested angle:
```
displacement = 2·f·tan(cone_angle/2) / BDF
x_feed = -displacement·cos(clock_angle)
y_feed = -displacement·sin(clock_angle)
z_feed = -displacement²/(4f)  for large displacements
```

> **Verified against the code 2026-08-19 (roadmap D5 exit criterion 3) — this section
> agrees, sign for sign.** Checked against
> `antenna-core/src/model/coordinates.rs::EClockConeCoordinates::to_feed_displacement_with_bdf`:
> the displacement magnitude is `2·f·tan(e_cone/2)/bdf`, `x_feed`/`y_feed` are the
> **negated** `cos`/`sin` of `e_clock` (the clock+180° flip described above), and
> `z_feed = −displacement²/(4f)`. The inverse `from_feed_position` is consistent — it
> recovers the clock angle as `atan2(−y, −x)`, i.e. the direction from the feed *back*
> through the axis. The 180° flip is not cosmetic: before `83193a0` the feed was placed
> at the *same* clock angle as the requested aim point, which put the beam peak 180° away
> (measured: a beam aimed at az=0°/el=2° peaked at az=180°, el≈1.75°, leaving −17.5 dBi
> in the requested direction). The BDF division landed with `661c6cb` and closes the
> remaining ~12% magnitude error at f/D = 0.5.
>
> Two clarifications, neither a disagreement:
> - The `z_feed` term is applied **unconditionally**, not only "for large displacements".
>   It is `O(displacement²)` and therefore negligible for small steers, which is what the
>   original hedge meant.
> - `bdf` is a parameter, not a constant: `to_feed_displacement` / `to_feed_position` pass
>   `1.0` to reproduce the uncorrected geometric mapping, and the served path passes the
>   real beam deviation factor from `coordinates_3d::beam_deviation_factor`.
>
> **Do not edit this section's math without re-verifying it against that function**
> (standing rule 2 of the work-unit list: never touch the feed-steering sign convention
> in a non-physics unit).

## 3. Algorithmic Considerations

### 3.1 Edge Cases

#### Coordinate Transformation Edge Cases
- **Coordinate Frame Declaration** (auto-detection removed 2026-07-27, roadmap C8 stage 2):
  - Each `Position3D` carries a required `coordinate_system` tag; there is no threshold and
    no ambiguous band. The removed heuristic classified components above 6400 km as ECEF,
    which could not separate a geodetic GEO satellite from an ECEF point.
  - Validation: reject NaN/Inf, then apply the range rules of the **declared** frame
- **Geodetic Singularities**:
  - Poles (latitude = ±90°): Handle azimuth ambiguity
  - Earth center (altitude → -6371 km): Invalid for antenna locations
- **Attitude Singularities**:
  - Gimbal lock in Euler angles (pitch = ±90°)
  - Quaternion normalization: Warn if |q| deviates from 1.0 by >0.01
- **Vehicle at High Altitude**:
  - Low Earth Orbit (LEO): 200-2000 km altitude
  - Medium Earth Orbit (MEO): 2000-35786 km altitude
  - Ensure coordinate transforms remain accurate

#### Large Feed Offset

**Corrected — the threshold is 0.5f, and there is no Seidel-aberration mode.**
`model/edge_cases.rs` selects the computation mode from the feed-offset ratio alone:

| offset / f | mode | notes |
|---|---|---|
| ≤ 0.5 | `StandardPhysicalOptics` | the exact coma phase of §2.2 covers the whole band |
| > 0.5 | `RayTracing` | `SEVERE_OFFSET_THRESHOLD`; the ray tracer is a **stub** and every endpoint warns (roadmap P3) |

- *Historical — not implemented (removed):* "Include higher-order Seidel aberrations".
  A `HigherOrderAberrations` mode for the 0.3f–0.5f band existed and was deleted by
  roadmap **P2** (`ef24214`, `PHYSICS_MODEL_VERSION` 4). It added heuristic Seidel terms
  *on top of* the already-exact geometric phase — a double-count, and with wrong-sign /
  wrong-pupil-power coefficients (it coded ρ³ distortion where both the exact model and
  classical theory give leading ρ¹). That band now routes through `StandardPhysicalOptics`.
- "Account for increased spillover" is **partly** implemented: spillover is modelled on
  the uncalibrated path (roadmap P1) and gated to `StandardPhysicalOptics`, but
  `SPILLOVER_MAX_OFFSET_RATIO = 0.3` marks where the offset extrapolation stops being
  validated — beyond it the response carries a degraded-accuracy warning rather than a
  better number.

#### Near-Boresight/Far-Feed Scenario

*Historical — not implemented (removed).* A `DirectPathInterference` computation mode
existed and was deleted as physically unsound in `c850165`. **Do not reimplement from
this description** without filing a decision item: the removed mode summed a "direct
feed reception" term with the reflected path incoherently and with no defensible
amplitude for either, so it produced a plausible number with no physics behind it.
Near-boresight queries on a far-displaced feed now follow the ordinary mode selection
above — `StandardPhysicalOptics` up to 0.5f, the flagged ray-tracing stub beyond it.

- ~~Compute direct feed reception~~
- ~~Calculate reflected path with severe phase errors~~
- ~~Model interference between paths~~

#### Frequency-Dependent Effects
- **Low frequency (< 1 GHz)**: Mesh reflectivity model — implemented, but as the
  continuous inductive-grid model of §2.4, which applies at **all** frequencies rather
  than only below a cutoff.
- **Transition region**: ~~Full Floquet mode analysis~~ — *historical — not implemented.*
  The inductive-grid model is a quasi-static approximation to it; nothing in the tree
  decomposes Floquet modes.
- **High frequency (> 10 GHz)**: Surface roughness dominance — implemented statistically
  via the Ruze efficiency (§2.4), not as a per-point surface map (§2.2).

#### Multi-Feed Antenna Scenarios
- **Feed Selection**: Validate feed_id exists for antenna_id
- **Feed Offset**: Feed positions typically at or near focal point
- **Frequency Bands**: Different feeds for different frequency ranges (e.g., S-band, X-band, Ka-band)
- **Beam Squint**: Frequency-dependent beam pointing differs from mechanical pointing

### 3.2 Numerical Stability

- **Adaptive integration near pattern nulls** — implemented:
  `edge_cases::needs_adaptive_integration` feeds `pattern::select_integration_params`.
  Note this is *not* what makes the served off-axis pattern converge; that is the
  adaptive radial/azimuthal-mode sizing inside the Hankel integrator (roadmap P10/P12),
  which self-checks on both axes and reports non-convergence as a response warning.
- **Minimum noise floor enforcement (-60 dB typical)** — implemented:
  `MIN_GAIN_FLOOR_DB = -60.0`, applied by `apply_gain_floor` / `apply_gain_floor_db` on
  the served gain path.
- ~~Kaiser windowing for sidelobe continuity~~ — *historical — not implemented.* No
  window function of any kind is applied to the aperture integrand; the word "kaiser"
  appears nowhere in the tree. Sidelobe-floor behaviour instead comes from the F7
  statistical Ruze floor (`pattern::sidelobe_floor_gain`, power sum forward and
  floor-only in the rear hemisphere). It is opt-in per `IntegrationParams`
  (`apply_sidelobe_floor`, default `false`); the service enables it exactly for antennas
  whose physics is uncorrected (`AntennaCalibration::physics_is_uncorrected`), so a
  calibrated antenna's measured residuals are never stacked on top of a modelled floor.

## 4. Calibration Methodology

### 4.1 Calibration Status Types

The system supports multiple calibration statuses, enabling graceful degradation from fully calibrated to uncalibrated antennas:

#### Fully Calibrated (Target Status)
- **Data Available**: Dense measurement grid across azimuth, elevation, frequency
- **Physics Model**: Fully tuned parameters
- **Correction Surface**: Dense B-spline capturing all residuals
- **Accuracy Estimate**: ±1 dB (main lobe and first sidelobe)
- **Use Cases**: Critical science antennas, deep space network, high-accuracy applications

#### Partially Calibrated - Boresight Only
- **Data Available**: Boresight measurements (az=0, el=0) across frequency and optionally temperature
- **Physics Model**: Parameters tuned to match boresight measurements (surface RMS, mesh
  spacing, wire diameter — *corrected: not q-factor, see §4.2*)
- **Correction Surface**: Optional, typically frequency-only (single spatial point)
- **Accuracy Estimate**:
  - Absolute gain (boresight): ±1 dB (tuned)
  - Absolute gain (off-axis): ±2-3 dB (physics model only)
  - Loss (relative): ±1-2 dB (systematic errors cancel)
- **Use Cases**: Feed steering analysis, quick calibration validation, operational antennas with limited test data

#### Partially Calibrated - Limited Coverage
- **Data Available**: Measurements at sparse grid (e.g., main lobe + first sidelobe only)
- **Physics Model**: Parameters tuned to measurements
- **Correction Surface**: Optional, sparse B-spline (limited spatial coverage)
- **Accuracy Estimate**:
  - In-coverage: ±1-1.5 dB
  - Out-of-coverage: ±2-3 dB (extrapolated)
  - Loss: ±1-1.5 dB
- **Use Cases**: Operational antennas with partial characterization, targeted measurement campaigns

#### Uncalibrated
- **Data Available**: Design specifications (diameter, f/D, feed location, surface quality estimate)
- **Physics Model**: Default parameters from design specs
- **Correction Surface**: None
- **Accuracy Estimate**:
  - Absolute gain: ±3-5 dB
  - Loss (relative gain): ±2-3 dB (systematic errors partially cancel)
- **Use Cases**: New antennas, prototype modeling, fallback when data unavailable

**Key Design Principle**: Loss accuracy (reference_gain - actual_gain) is prioritized over absolute gain accuracy, as systematic parameter errors cancel in the difference computation.

### 4.2 Input Data Sources

#### Design Specifications (Required for Uncalibrated Antennas)
- **Reflector Geometry**: Diameter, focal length, f/D ratio, surface RMS estimate
- **Feed Configuration**: Position, q-factor, phase center offset, frequency range
- **Mesh Parameters** (if applicable): Mesh spacing, wire diameter
- **Source**: Manufacturer specifications, engineering drawings, visual inspection
- **Use**: Initial parameter estimates for uncalibrated antennas; starting point for parameter tuning

#### G/T Measurements (Full Calibration)
- **Format**: Tables indexed by frequency, E-clock, E-cone
- **Content**: G/T values in dB/K
- **Coverage**: Main lobe and several sidelobes
- **Density**: Dense grid (>10 points per beamwidth)

#### Boresight Measurements (Partial Calibration)
- **Format**: Frequency sweep at (az=0, el=0)
- **Content**: G/T values across frequency range
- **Coverage**: Single spatial point, multiple frequencies
- **Use**: Parameter tuning (surface RMS, mesh spacing, wire diameter)
  - *Corrected:* **the feed q-factor is not tuned, here or anywhere.**
    `calibrate/src/parameter_tuner.rs` tunes at most three parameters — surface RMS,
    mesh spacing, wire diameter (`TuningMode::{SurfaceRmsOnly, SurfaceAndMeshSpacing, All}`)
    — and takes `q_factor`, `phase_center_offset` and `asymmetry_factor` from the antenna
    class as fixed *declared* design properties. This is deliberate: they are horn
    geometry, and the tuner's module doc states it. The asymmetry factor in particular is
    declared rather than fitted because boresight data carries no information about it
    (roadmap **D23**).

#### Sparse Grid Measurements (Limited Coverage Calibration)
- **Format**: Partial angular grid (e.g., ±15° azimuth/elevation)
- **Content**: G/T values at sparse spatial sampling
- **Coverage**: Limited angular region (2-5 points per beamwidth)
- **Use**: Parameter tuning + sparse correction surface

#### Reference Patterns
- On-axis gain at multiple frequencies
- Feed pattern cuts (E-plane, H-plane)
- Phase center measurements

### 4.3 Calibration Process

#### Full Calibration Workflow

**Corrected — this is the shipped `calibrate --calibration-mode full` pipeline**
(`calibrate/src/main.rs`); the original five steps below it are kept for provenance.

   1. **Parse the measurement CSV** (`parser.rs`): `e_clock_deg, e_cone_deg,
      frequency_mhz, g_over_t_db, temperature_k`. Every row is normalised into the
      **polar convention** on the way in — a negative E-cone names the same direction as
      `(clock + 180°, |cone|)` and only the second form is reachable on the served path
      (roadmap **D26**).
   2. **Tune physical parameters** with a **Nelder-Mead simplex** (`parameter_tuner.rs`,
      `argmin::solver::neldermead`), optional and off unless `--tune-parameters` is
      passed. Tunes surface RMS, mesh spacing, wire diameter; bounds are a multiplicative
      bracket around the antenna class's own nominal (`ParameterBounds::from_class`,
      roadmap D16).
   3. **Compute model predictions** at every measurement point, in **G/T space** —
      `compute_g_over_t`, the same function the service uses — so model bias is absorbed
      at calibrated points rather than laundered through a separate gain conversion.
   4. **Fit the correction surface** to the residuals `measured − model`
      (`correction_surface.rs`): a **4D B-spline** over (E-clock, E-cone, frequency,
      temperature), normal equations accumulated from the basis's local support and
      solved by an in-house Cholesky (no BLAS). An underdetermined fit is a hard error
      (roadmap D20), and the delivered knot spacing is compared against the antenna's own
      `λ/D` lobe period and reported (roadmap D21).
   5. **Validate** (`validator.rs`, `--validate`): strided k-fold cross-validation
      (point `i` held out by fold `i % K`, roadmap D22) plus main-lobe / first-sidelobe
      error checks.
   6. **Serialize** (`artifact_export.rs`): an `AntennaCalibration` in **postcard**,
      wrapped in the ANTC header (magic + version + CRC32 + length).

*Historical — the original design, not how the tool works:*

   1. ~~Extract gain from G/T using noise temperature model~~ — the pipeline never leaves
      G/T space; residuals are G/T dB throughout.
   2. ~~Fit Zernike polynomial model to gain surface~~ — **never implemented.** No
      Zernike machinery exists in the tree (the last of it was deleted in `a6dac0c`), and
      the correction surface is a B-spline over measurement axes, not a modal fit to a
      surface. See §4.4.
   3. ~~Optimize mesh parameters via differential evolution~~ — the optimizer is
      **Nelder-Mead**. No differential-evolution optimizer was ever implemented under any
      name (checked 2026-07-31: `git log --all -S` finds no `DifferentialEvolution` or
      `differential_evolution` anywhere in `calibrate/`).
   4. ~~Generate correction surfaces for systematic errors~~ — this one survived, as
      step 4 above.
   5. ~~Validate against measurements~~ — survived as step 5; the <1 dB main-lobe /
      first-sidelobe target is unchanged.

#### Boresight Calibration Workflow (Sprint 7 — shipped)
   1. **Load design specs** as initial parameter estimates
   2. **Tune physical parameters** using a **Nelder-Mead simplex** *(corrected: not
      differential evolution — see the note under the full workflow above)*:
      - Optimize: `surface_rms_mm`, `mesh_spacing_mm`, `wire_diameter_mm`
        *(corrected: `q_factor` is declared, not tuned — see §4.2)*
      - Objective: Minimize `|measured_G/T - physics_model_G/T|` at boresight across frequencies
      - Constraints: Keep parameters within physically reasonable ranges
   3. **Optional correction surface**:
      - Fit 1D frequency-only correction: `correction(freq) = measured - physics`
      - Skip if physics model error < 0.5 dB (low priority)
   4. **Validate**: Check that tuned parameters are physically reasonable
   5. **Output**: Calibration artifact with `PartiallyCalibrated` status
      - Coverage: azimuth=[0,0], elevation=[0,0], frequency range from measurements

#### Limited Coverage Calibration Workflow (Future)

*Still future — `calibrate --calibration-mode` accepts `full` and `boresight` only.*

   1. **Load design specs** as initial estimates
   2. **Tune physical parameters** across all measurement points
   3. **Optional correction surface**:
      - Fit sparse ~~3D~~ **4D** B-spline (azimuth, elevation, frequency, **temperature**)
        *(corrected: `BSplineModel4D` is the only surface shape any artifact can carry, and
        postcard's encoding is positional — a 3D surface is not a smaller valid artifact,
        it is an unloadable one)*
      - Use measurements to construct sparse grid
   4. **Validate**: Check in-coverage accuracy
   5. **Output**: Calibration artifact with coverage metadata

#### Uncalibrated Workflow (NO CALIBRATION REQUIRED)
   1. **Load design specs** from configuration file
   2. **Construct `PhysicalAntennaConfig`** from design specifications
   3. **No measurements** - physics model only
   4. **Output**: In-memory calibration with `Uncalibrated` status
      - No `.bin` file required
      - Loaded directly from `antennas.yaml` at service startup


### 4.4 Expected Calibration Artifacts

The calibration artifacts vary based on calibration status:

#### Fully Calibrated Artifacts
1. **Mesh Parameter Set** (Tuned)
   - Mesh spacing: 1-10 mm range
   - Wire diameter: 0.05-1 mm range
   - Surface RMS: 0.1-2 mm range
   - **Source**: optimized from the full measurement grid — but *only* when `calibrate` is
     run with `--tune-parameters`, which is **off by default**. Without it the artifact
     carries the antenna class's nominal values and the correction surface absorbs the
     difference. Either way the artifact stamps what the residuals were fitted against.

2. **Aberration Coefficient Matrix**
   - *Historical — not implemented.* ~~Zernike coefficients up to 5th order;
     frequency-dependent scaling factors.~~ No artifact carries aberration coefficients
     and no Zernike code exists (§4.3). Aberration from feed displacement is computed
     exactly from the geometry at evaluation time (§2.2), and residual surface error is
     absorbed by item 3.

3. **Correction Surface** (Dense)
   - *Corrected:* a **4D B-spline**, not a 3D lookup table:
     (azimuth = E-clock, elevation = E-cone, frequency, temperature) → correction_dB,
     evaluated by `model/correction_interpolator.rs` and applied in
     `service/evaluator.rs`. `BSplineModel4D` carries the coefficients, per-axis knot
     vectors and the spline order; the loader validates knot count and monotonicity
     against the declared shape.
   - ~~Separate tables for main lobe, sidelobes, far field~~ — *not implemented.* One
     surface covers the whole domain; there is no lobe-region partition.
   - **Coverage**: recorded per artifact in `calibration_coverage`; a query outside it
     still returns a value, with an extrapolation warning.
   - **Known limitation (roadmap D21/D24):** the angular knots are absolute while the
     pattern scale is `λ/D`, so on a narrow-beam antenna the surface carries the
     residual's *envelope trend*, not its lobe structure. Every full-mode fit now reports
     this (`assess_angular_resolution`, `CalibrationMetadata.angular_resolution`).

4. **Feed Model Parameters**
   - *Corrected: none of these are tuned.* Q-factor, phase-center offset and asymmetry
     factor are **declared** design properties, read from the antenna class and stamped
     into the artifact unchanged (§4.2). The artifact must carry every one of them, or
     the service evaluates a different antenna than the residuals describe — that is
     roadmap **C13** (feed position, 27.3 dB) and **D23** (asymmetry factor, up to
     1.20 dB off-axis but 0.0003 dB at boresight).
   - Q-factor: **1.14–3.15** across the shipped antennas, each derived from
     `q_factor_from_taper` for a ~−11 dB edge taper at that antenna's f/D (declared).
     *Corrected: the original "6-10 range (optimized)" is wrong twice over — see §2.3 for
     why the range moved, and §4.2 for why it is not optimized.*
   - Phase center offset: ±λ/4 typical (declared; compensated by per-band feed
     positioning — the P7 auto-refocus — so it does not also enter as defocus)
   - Asymmetry factor for E/H plane differences (declared; `1.0` means symmetric, and a
     value ≠ 1.0 moves the evaluation onto the azimuthal-mode integrator branch)

#### Partially Calibrated Artifacts (Boresight Only)
1. **Mesh Parameter Set** (Tuned from Boresight)
   - Optimized from boresight measurements across frequencies
   - **Accuracy**: Tuned to <1 dB at boresight
   - **Limitation**: Off-axis accuracy ±2-3 dB (physics extrapolation)

2. **Optional Frequency Correction** (1D)
   - Frequency-only correction: `correction(freq)`
   - Stored as degenerate 4D B-spline (single spatial point)
   - **Applied only**: When query is at or near boresight

3. **Calibration Coverage Metadata**
   - Azimuth range: [0.0, 0.0] (single point)
   - Elevation range: [0.0, 0.0] (single point)
   - Frequency range: from measurements
   - Num measurements: typically 10-50 frequency samples

#### Partially Calibrated Artifacts (Limited Coverage)
1. **Mesh Parameter Set** (Tuned from Sparse Grid)
   - Optimized from limited angular measurements

2. **Sparse Correction Surface** (Optional)
   - 3D B-spline with limited spatial coverage
   - **In-coverage**: ±1-1.5 dB accuracy
   - **Out-of-coverage**: ±2-3 dB (physics extrapolation)

3. **Calibration Coverage Metadata**
   - Azimuth range: limited (e.g., [0, 360] or [-30, 30])
   - Elevation range: limited (e.g., [30, 60])
   - Frequency range: from measurements
   - Num measurements: typically 100-500 points

#### Uncalibrated Artifacts (Design Specs Only)
1. **Mesh Parameter Set** (From Design Specs)
   - **Source**: Manufacturer specifications or estimates
   - **Accuracy**: Untested, ±3-5 dB absolute gain
   - **Loss Accuracy**: ±2-3 dB (systematic errors partially cancel)

2. **No Correction Surface**
   - Physics model only

3. **Feed Model Parameters** (Estimated)
   - Q-factor: derived from the design f/D for a ~−11 dB edge taper — of order 1–3, not
     the "8.0 for horn feed" this originally read (§2.3)
   - Phase center offset: 0.0 (assumed)

#### Multi-Feed Support (All Calibration Statuses)
1. **Feed Configurations**
   - Feed ID to physical position mapping
   - Feed-specific patterns and frequency ranges
   - Example feed configurations:
     - `s_band_feed`: 2.0-2.3 GHz, position offset (0, 0, 0) - at focal point
     - `x_band_feed`: 7.1-8.5 GHz, position offset (0.05, 0, 0) - slightly off-axis
     - `ka_band_feed`: 25.5-27.0 GHz, position offset (0, 0.05, 0) - different offset
   - Per-feed calibration corrections (if applicable)
   - Each feed can have different calibration status

### 4.5 Validation Metrics

Validation criteria vary by calibration status:

#### Fully Calibrated
   - **Main-lobe max error**: <1.0 dB
   - **First side-lobe max error**: <1.0 dB
   - **Overall RMSE**: <0.5 dB
   - **Model correlation**: R² > 0.95
   - **Coverage**: Full field of view
   - **Outlier scenarios**: <5% of points exceed tolerance

#### Partially Calibrated - Boresight Only
   - **Boresight error**: <1.0 dB across frequencies
   - **Parameter consistency**: Tuned parameters within physically reasonable bounds
   - **Off-axis predictions**: Not validated (physics extrapolation, expect ±2-3 dB)
   - **Loss accuracy**: ±1-2 dB (verified via reference gain computation)
   - **Coverage**: Single spatial point

#### Partially Calibrated - Limited Coverage
   - **In-coverage error**: <1.5 dB
   - **Out-of-coverage**: Not validated (physics extrapolation)
   - **Coverage metadata**: Accurately reflects measurement extent
   - **Transition quality**: Smooth extrapolation at coverage boundaries

#### Uncalibrated
   - **No validation**: Design specs used as-is
   - **Expected accuracy**: ±3-5 dB absolute gain
   - **Loss accuracy**: ±2-3 dB (systematic error cancellation)
   - **Parameter reasonableness**: Check that design specs are physically plausible

#### General Validation (All Statuses)
   - **API response format**: Includes `calibration_status` field with accuracy estimates
   - **Warning generation**: Appropriate warnings for extrapolation or low confidence
   - **Multi-feed support**: Each feed validated independently

---

## 5. Calibration Upgrade Path

The system supports graceful evolution from uncalibrated to fully calibrated antennas:

### 5.1 Upgrade Sequence

```
Uncalibrated (Design Specs Only)
    ↓ Collect boresight measurements (10-50 frequency samples)
Partially Calibrated - Boresight Only
    ↓ Collect sparse off-axis measurements (100-500 points)
Partially Calibrated - Limited Coverage
    ↓ Collect full measurement grid (1000-5000 points)
Fully Calibrated
```

### 5.2 Operational Benefits

1. **Immediate Deployment**: New antennas can be added with design specs only
2. **Incremental Improvement**: Accuracy improves as measurements become available
3. **No Service Interruption**: Calibration upgrades don't require downtime
4. **Loss-First Strategy**: Focus on loss accuracy (1-2 dB) over absolute gain accuracy
5. **Cost-Effective**: Boresight calibration requires ~1 hour test time vs. ~8 hours for full calibration

### 5.3 Accuracy Evolution

| Stage | Test Time | Absolute Gain | Loss Accuracy | Primary Use Case |
|-------|-----------|---------------|---------------|------------------|
| Uncalibrated | 0 hours | ±3-5 dB | ±2-3 dB | Feed steering analysis, prototype modeling |
| Boresight Only | ~1 hour | ±1 dB (boresight), ±2-3 dB (off-axis) | ±1-2 dB | Operational antennas, quick validation |
| Limited Coverage | ~3-4 hours | ±1-1.5 dB (in-coverage) | ±1-1.5 dB | Targeted applications, cost-constrained scenarios |
| Full Calibration | ~8 hours | ±1 dB (full FOV) | ±1 dB | Critical science, deep space network |

### 5.4 Implementation Status

- **Phase 1 (Sprint 6)**: ✅ COMPLETE
  - Data model extensions for calibration statuses
  - Service layer support for all statuses
  - API schemas with calibration status information
  - Uncalibrated antenna loading from design specs

- **Phase 2 (Sprint 7)**: ✅ COMPLETE *(corrected 2026-08-19 — this read "📋 PLANNED";
  Sprint 7 shipped, see `docs/implementation-plan.md`)*
  - Boresight calibration mode in `calibrate` tool
  - Parameter tuning from boresight measurements
  - Design specs loading and validation
  - Optional frequency-only correction surface *(still optional and off by default)*

- **Phase 3 (Future)**: 📋 DEFERRED
  - Limited coverage calibration mode
  - Sparse correction surface fitting
  - Coverage analysis and metadata generation

### 5.5 Reference Documents

For detailed design specifications, implementation tasks, and API schemas, see:
- **Detailed Design**: `docs/partial-calibration-design.md`
- **Implementation Plan**: `docs/partial-calibration-implementation-plan.md`
- **Main Implementation Plan**: `docs/implementation-plan.md` (Sprint 6 & 7 sections)