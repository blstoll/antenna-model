# Antenna Model Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-extended-cc:subagent-driven-development (recommended) or superpowers-extended-cc:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the physics, semantics, and hygiene defects found in the 2026-07-02 comprehensive review — most critically the 180° feed-steering direction flip that makes every steered-feed gain prediction evaluate on the wrong side of boresight.

**Architecture:** All physics fixes land in the model layer (`antenna-model/src/model/`), with the API/service layers picking them up through existing call paths. Each task is independently committable and keeps `cargo test --all` green. The steering sign fix (Task 1) and beam deviation factor (Task 2) change the numeric contract of `EClockConeCoordinates::to_feed_position` / `compute_feed_position_from_pointing`; everything downstream (evaluator, H3 link budget, heatmap) consumes those helpers, so no service-layer physics changes are needed.

**Tech Stack:** Rust workspace (`antenna-model` service crate + `calibrate` CLI crate), `cargo test`, `num-complex`, `poem` (untouched), B-spline correction surfaces.

**Review findings → tasks map:**

| Finding | Task |
|---|---|
| 180° clock-angle flip in feed steering (confirmed, ~70 dB error) | 1 |
| No beam deviation factor (~12% steering-angle error at f/D=0.5) | 2 |
| `direct_path.rs` physically indefensible (0.1 magic scale, wrong feed orientation, mixed normalization) | 3 |
| `phase_center_offset` calibration parameter silently ignored | 4 |
| Correction surface evaluated at hardcoded 290 K instead of `temperature_const` | 5 |
| `find_knot_span` panics on malformed knot vectors | 6 |
| `feed_position` aim-point semantics undocumented; `feed_offset_meters` z-sign doc flipped; loss comment contradicts code; stale example expectations; CLAUDE.md Zernike claim false | 7 |
| Dead code: `numerical_stability.rs`, `compute_feed_offset_v2`, `AzElCoordinates`, Zernike/Gaussian surface models, unused mesh exports; `f_over_d_ratio` never cross-validated | 8 |
| Final verification sweep | 9 |

**Explicitly deferred (YAGNI for this round):** periodic-azimuth B-spline support (0°/360° seam extrapolates today; a real fix requires re-fitting artifacts with periodic knots), true spillover modeling for uncalibrated antennas, ray-tracing mode replacement (it still emits an accuracy warning and only activates at offset > 0.5f).

**Domain background the implementer needs:** For a parabolic dish, displacing the feed laterally from the focal point steers the beam to the *opposite* side (this falls out of the path-length phase model in `phase.rs` — feed at +x makes paths on the +x side of the aperture shorter, tilting the phase front toward −x). The steering magnitude is reduced by the beam deviation factor (BDF ≈ 0.87 at f/D = 0.5). "Elevation" throughout this codebase is **polar angle from boresight** (0° = boresight), not horizon elevation.

---

### Task 1: Fix the 180° feed-steering clock flip

**Goal:** A feed "aimed" at clock angle φ is physically displaced at φ+180° so the beam actually lands at φ; add a regression test that fails against the current code.

**Files:**
- Modify: `antenna-model/src/model/coordinates.rs` (`to_feed_displacement` ~line 207, `from_feed_position` ~line 247, plus 3 tests)
- Modify: `antenna-model/src/model/direct_path.rs:308` (test assertion — file still exists until Task 3)
- Modify: `antenna-model/src/model/ray_trace.rs:379` (test assertion)
- Create: `antenna-model/tests/beam_steering_direction.rs`

**Acceptance Criteria:**
- [ ] With the feed steered toward (az=0°, el=2°), the PO model's gain at (az=0°, el=2°) exceeds gain at (az=180°, el=2°) by > 30 dB (currently it is ~70 dB the *other* way)
- [ ] Same property holds for az=90° steering (catches y-sign errors independently of x)
- [ ] `to_feed_position` → `from_feed_position` round-trip still exact (existing `test_e_clock_cone_roundtrip` passes unmodified)
- [ ] `cargo test -p antenna-model` fully green

**Verify:** `cargo test -p antenna-model` → all pass, including new `beam_steering_direction` tests

**Steps:**

- [ ] **Step 1: Write the failing regression test**

Create `antenna-model/tests/beam_steering_direction.rs`:

```rust
//! Regression tests for the feed-steering direction convention.
//!
//! A lateral feed offset steers a paraboloid's beam to the OPPOSITE side of
//! boresight (beam deviation). `to_feed_position` must therefore place the
//! feed at clock angle φ+180° when the caller asks for a beam at clock φ.
//! Before the 2026-07 fix the feed was placed at φ, putting the beam peak
//! 180° away from every steered-feed aim point (~70 dB error at the target).

use antenna_model::model::{
    compute_gain_db, AntennaConfiguration, EClockConeCoordinates, FeedParameters, FeedPosition,
    IntegrationParams, ReflectorGeometry,
};
use std::f64::consts::PI;

const FREQ_HZ: f64 = 8.4e9;

fn dish_with_feed(x: f64, y: f64, z: f64) -> AntennaConfiguration {
    let reflector = ReflectorGeometry::new(10.0, 5.0, 0.0).unwrap(); // D=10 m, f=5 m, ideal surface
    let feed = FeedParameters::new(FeedPosition::new(x, y, z), 8.0, 0.0, 1.0).unwrap();
    AntennaConfiguration::new("probe".into(), "Probe".into(), reflector, feed, None).unwrap()
}

fn gain_db(config: &AntennaConfiguration, el_deg: f64, az_rad: f64) -> f64 {
    compute_gain_db(
        el_deg.to_radians(),
        az_rad,
        config,
        FREQ_HZ,
        &IntegrationParams::default(),
    )
    .unwrap()
    .gain
}

/// Steer toward az=0°, el=2°: gain on the requested side must dominate.
#[test]
fn steered_beam_lands_on_requested_azimuth_x() {
    let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 2.0);
    let (fx, fy, fz) = ecc.to_feed_position(5.0);
    let config = dish_with_feed(fx, fy, fz);

    let g_target = gain_db(&config, 2.0, 0.0);
    let g_opposite = gain_db(&config, 2.0, PI);
    assert!(
        g_target > g_opposite + 30.0,
        "beam must land on the requested side: target={g_target:.1} dBi, opposite={g_opposite:.1} dBi"
    );
}

/// Steer toward az=90°, el=2°: independently catches y-axis sign errors.
#[test]
fn steered_beam_lands_on_requested_azimuth_y() {
    let ecc = EClockConeCoordinates::from_azimuth_elevation(90.0, 2.0);
    let (fx, fy, fz) = ecc.to_feed_position(5.0);
    let config = dish_with_feed(fx, fy, fz);

    let g_target = gain_db(&config, 2.0, PI / 2.0);
    let g_opposite = gain_db(&config, 2.0, 3.0 * PI / 2.0);
    assert!(
        g_target > g_opposite + 30.0,
        "beam must land on the requested side: target={g_target:.1} dBi, opposite={g_opposite:.1} dBi"
    );
}

/// The feed itself must sit OPPOSITE the aim direction.
#[test]
fn feed_is_displaced_opposite_the_aim_direction() {
    let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 5.0);
    let (fx, fy, _fz) = ecc.to_feed_position(5.0);
    assert!(fx < -0.1, "aim at +x must displace the feed toward -x, got fx={fx}");
    assert!(fy.abs() < 1e-9, "no y component expected, got fy={fy}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p antenna-model --test beam_steering_direction`
Expected: FAIL — `steered_beam_lands_on_requested_azimuth_x` panics (target ≈ −17 dBi, opposite ≈ +52 dBi), `feed_is_displaced_opposite_the_aim_direction` panics (fx > 0).

- [ ] **Step 3: Fix `to_feed_displacement` in `antenna-model/src/model/coordinates.rs` (~line 207)**

Replace the body (negate the lateral components) and update the doc comment:

```rust
    /// Calculate feed displacement from focal point
    ///
    /// This is the key transformation from pointing angles to physical feed position.
    /// A lateral feed offset steers the beam to the OPPOSITE side of boresight
    /// (beam deviation): to point the beam at clock angle φ, the feed must be
    /// displaced at clock angle φ+π. Hence the negated x/y components below.
    /// ```text
    /// displacement = 2·f·tan(cone_angle/2)
    /// x_feed = -displacement·cos(clock_angle)
    /// y_feed = -displacement·sin(clock_angle)
    /// z_feed = -displacement²/(4f)   (defocus term, keeps feed near focal surface)
    /// ```
    ///
    /// # Arguments
    /// - `focal_length`: Focal length of the parabolic reflector (meters)
    ///
    /// # Returns
    /// Feed position (x, y, z) in Cartesian coordinates relative to focal point
    pub fn to_feed_displacement(&self, focal_length: f64) -> (f64, f64, f64) {
        // Radial displacement in xy-plane
        let displacement = 2.0 * focal_length * (self.e_cone / 2.0).tan();

        // Cartesian components — NEGATED: beam deviation puts the feed on the
        // side opposite the desired beam direction.
        let x_feed = -displacement * self.e_clock.cos();
        let y_feed = -displacement * self.e_clock.sin();

        // For large displacements, include z-component (defocus)
        let z_feed = -displacement * displacement / (4.0 * focal_length);

        (x_feed, y_feed, z_feed)
    }
```

- [ ] **Step 4: Fix the inverse `from_feed_position` (~line 247) to preserve the round-trip**

Change the clock-angle line:

```rust
        // Clock angle: the feed sits opposite the beam direction, so the beam's
        // clock angle is the direction from the feed BACK through the axis.
        let e_clock = (-y).atan2(-x);
```

- [ ] **Step 5: Update the three `coordinates.rs` tests that encode the old convention**

In `test_e_clock_cone_feed_displacement_small_angle` (~line 397): the x-assertion flips sign:

```rust
        // Feed is displaced OPPOSITE the aim direction (beam deviation)
        let expected_displacement = 2.0 * focal_length * (e_cone / 2.0).tan();
        assert!((x + expected_displacement).abs() < 0.01);
        assert!(y.abs() < EPSILON);
```

In `test_e_clock_cone_feed_position` (~line 415): the clock-angle check inverts:

```rust
        // x, y should reflect the clock angle, on the OPPOSITE side of the axis
        let _radial = (x * x + y * y).sqrt();
        let angle = (-y).atan2(-x);
        assert!((angle - e_clock).abs() < EPSILON);
```

In `test_from_azimuth_elevation_azimuth_direction` (~line 702): all three direction checks flip:

```rust
        // Azimuth 0° aims the beam along +X, so the feed goes to -X
        let ecc_0 = EClockConeCoordinates::from_azimuth_elevation(0.0, el_deg);
        let (x0, y0, _) = ecc_0.to_feed_position(focal_length);
        assert!(x0 < -1.0, "Azimuth 0° should have large negative x");
        assert!(y0.abs() < 0.01, "Azimuth 0° should have y≈0");

        // Azimuth 90° aims along +Y, so the feed goes to -Y
        let ecc_90 = EClockConeCoordinates::from_azimuth_elevation(90.0, el_deg);
        let (x90, y90, _) = ecc_90.to_feed_position(focal_length);
        assert!(x90.abs() < 0.01, "Azimuth 90° should have x≈0");
        assert!(y90 < -1.0, "Azimuth 90° should have large negative y");

        // Azimuth 180° aims along -X, so the feed goes to +X
        let ecc_180 = EClockConeCoordinates::from_azimuth_elevation(180.0, el_deg);
        let (x180, y180, _) = ecc_180.to_feed_position(focal_length);
        assert!(x180 > 1.0, "Azimuth 180° should have large positive x");
        assert!(y180.abs() < 0.01, "Azimuth 180° should have y≈0");
```

- [ ] **Step 6: Update the two sign-sensitive test-fixture assertions in other modules**

`antenna-model/src/model/direct_path.rs` `test_feed_position_extraction` (~line 308):

```rust
        // E-clock = 0 aims the beam at +x, so the feed sits at -x
        assert!(pos.0 < 0.0);
        assert!(pos.1.abs() < 1e-6); // E-clock = 0 → y = 0
```

`antenna-model/src/model/ray_trace.rs` `test_feed_position_offset` (~line 379):

```rust
        // For E-clock=0 (beam toward +x), feed is displaced toward -x
        assert!((pos.1.abs()) < 1e-6);
        assert!(pos.0 < 0.0);
```

- [ ] **Step 7: Run the full crate test suite**

Run: `cargo test -p antenna-model`
Expected: PASS (all tests, including the new regression file, the untouched round-trip test, and the evaluator's `test_loss_near_zero_for_boresight_focused_feed` which is on-axis and sign-invariant).

- [ ] **Step 8: Commit**

```bash
git add antenna-model/src/model/coordinates.rs antenna-model/src/model/direct_path.rs antenna-model/src/model/ray_trace.rs antenna-model/tests/beam_steering_direction.rs
git commit -m "fix: steer feed opposite the aim direction (180° clock flip)

A lateral feed offset steers a paraboloid's beam to the opposite side of
boresight. to_feed_position placed the feed on the SAME side as the aim
point, so every steered-feed prediction evaluated ~2x the steering angle
away from the actual beam peak (~70 dB error at the aim point).

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Apply the beam deviation factor to steering displacement

**Goal:** Scale the steering displacement by 1/BDF so the physical-optics beam peak lands at the requested angle (currently ~1.75° for a 2° request at f/D = 0.5).

**Files:**
- Modify: `antenna-model/src/model/coordinates.rs` (add `_with_bdf` variants to `EClockConeCoordinates`)
- Modify: `antenna-model/src/model/coordinates_3d.rs` (add `beam_deviation_factor`, change `compute_feed_position_from_pointing` signature)
- Modify: `antenna-model/src/model/mod.rs:43` (export `beam_deviation_factor`)
- Modify: `antenna-model/src/service/evaluator.rs:141` (caller: pass diameter)
- Modify: `antenna-model/src/service/h3_link_budget.rs:87` (caller: pass diameter)
- Modify: `antenna-model/src/service/evaluator.rs` test `test_feed_offset_is_meters_not_degrees` (~line 992, pass diameter)
- Test: `antenna-model/tests/beam_steering_direction.rs` (add peak-location test)

**Acceptance Criteria:**
- [ ] `beam_deviation_factor(0.5)` ∈ [0.86, 0.88]; `beam_deviation_factor(10.0)` > 0.99
- [ ] On a D=10 m, f=5 m dish, a 2° steering request puts the PO beam peak within 0.15° of 2.0°
- [ ] `compute_feed_position_from_pointing` takes diameter; both service callers updated
- [ ] `cargo test -p antenna-model` green

**Verify:** `cargo test -p antenna-model` → all pass, including `steered_beam_peaks_at_requested_angle`

**Steps:**

- [ ] **Step 1: Write the failing peak-location test**

Append to `antenna-model/tests/beam_steering_direction.rs`:

```rust
/// With the beam deviation factor applied, the beam peak must land at the
/// requested steering angle (within a tenth of the ~0.25° beamwidth grid).
/// Without BDF the peak sits at ~2°·0.87 ≈ 1.75° and this test fails.
#[test]
fn steered_beam_peaks_at_requested_angle() {
    use antenna_model::model::{beam_deviation_factor, compute_feed_position_from_pointing};
    use antenna_model::api::schemas::{CoordinateSystem, Position3D};

    // BDF unit checks (Lo 1960, K = 0.36)
    let bdf_half = beam_deviation_factor(0.5);
    assert!(
        (0.86..=0.88).contains(&bdf_half),
        "BDF(f/D=0.5) = {bdf_half}, expected ~0.871"
    );
    assert!(beam_deviation_factor(10.0) > 0.99);

    // Steer 2° at az=0 on the D=10 m, f=5 m dish via the E-cone path directly
    // (unit-level; the service path is exercised by evaluator tests).
    let ecc = EClockConeCoordinates::from_azimuth_elevation(0.0, 2.0);
    let (fx, fy, fz) = ecc.to_feed_position_with_bdf(5.0, bdf_half);
    let config = dish_with_feed(fx, fy, fz);

    // Scan elevation on the az=0 cut for the peak.
    let mut peak = (0.0_f64, f64::NEG_INFINITY);
    let mut el = 1.0_f64;
    while el <= 3.0 {
        let g = gain_db(&config, el, 0.0);
        if g > peak.1 {
            peak = (el, g);
        }
        el += 0.05;
    }
    assert!(
        (peak.0 - 2.0).abs() <= 0.15,
        "beam peak at {:.2}°, expected 2.00° ± 0.15°",
        peak.0
    );

    // Silence unused-import warnings until the service-path assertions below compile.
    let _ = (
        compute_feed_position_from_pointing
            as fn(&Position3D, &Position3D, &Position3D, f64, f64, Option<[f64; 4]>) -> _,
        CoordinateSystem::ECEF,
    );
}
```

- [ ] **Step 2: Run to verify it fails to compile (missing functions), then keep it red**

Run: `cargo test -p antenna-model --test beam_steering_direction`
Expected: compile error — `beam_deviation_factor` and `to_feed_position_with_bdf` don't exist yet.

- [ ] **Step 3: Add `beam_deviation_factor` to `antenna-model/src/model/coordinates_3d.rs`** (place above `compute_feed_position_from_pointing`)

```rust
/// Beam deviation factor for a paraboloid with a laterally displaced feed.
///
/// A feed tilted by angle ψ off-axis steers the beam by only BDF·ψ. Using the
/// classical approximation (Y.T. Lo, "On the beam deviation factor of a
/// parabolic reflector", 1960):
/// ```text
/// BDF = (1 + K·(D/4f)²) / (1 + (D/4f)²),  K ≈ 0.36 for typical tapers
/// ```
/// For f/D = 0.5 this gives ≈ 0.871; BDF → 1 as f/D → ∞ (flat reflector limit).
///
/// Steering code divides the required lateral displacement by BDF so the beam
/// lands at the requested angle.
pub fn beam_deviation_factor(f_over_d: f64) -> f64 {
    const K: f64 = 0.36;
    let x = 1.0 / (4.0 * f_over_d);
    (1.0 + K * x * x) / (1.0 + x * x)
}
```

- [ ] **Step 4: Add BDF-aware variants to `EClockConeCoordinates` in `coordinates.rs`**

Refactor `to_feed_displacement`/`to_feed_position` to delegate:

```rust
    /// `to_feed_displacement` with an explicit beam deviation factor.
    ///
    /// Divides the lateral displacement by `bdf` so the physical-optics beam
    /// peak lands at `e_cone` rather than `BDF·e_cone`. Pass `1.0` to reproduce
    /// the geometric (no-BDF) mapping.
    pub fn to_feed_displacement_with_bdf(&self, focal_length: f64, bdf: f64) -> (f64, f64, f64) {
        // Radial displacement in xy-plane, corrected for beam deviation
        let displacement = 2.0 * focal_length * (self.e_cone / 2.0).tan() / bdf;

        // Cartesian components — NEGATED: beam deviation puts the feed on the
        // side opposite the desired beam direction.
        let x_feed = -displacement * self.e_clock.cos();
        let y_feed = -displacement * self.e_clock.sin();

        // For large displacements, include z-component (defocus)
        let z_feed = -displacement * displacement / (4.0 * focal_length);

        (x_feed, y_feed, z_feed)
    }

    pub fn to_feed_displacement(&self, focal_length: f64) -> (f64, f64, f64) {
        self.to_feed_displacement_with_bdf(focal_length, 1.0)
    }

    /// `to_feed_position` with an explicit beam deviation factor (see
    /// `to_feed_displacement_with_bdf`).
    pub fn to_feed_position_with_bdf(&self, focal_length: f64, bdf: f64) -> (f64, f64, f64) {
        let (dx, dy, dz) = self.to_feed_displacement_with_bdf(focal_length, bdf);
        (dx, dy, focal_length + dz)
    }

    pub fn to_feed_position(&self, focal_length: f64) -> (f64, f64, f64) {
        self.to_feed_position_with_bdf(focal_length, 1.0)
    }
```

Keep the doc comment written in Task 1 on `to_feed_displacement_with_bdf` (move it); the thin wrappers get one-line docs.

Note: `from_feed_position` remains the BDF=1 inverse. That is intentional — it is only used to recover approximate pointing from a physical position, and its round-trip test uses the BDF=1 forward mapping.

- [ ] **Step 5: Thread diameter through `compute_feed_position_from_pointing` (`coordinates_3d.rs` ~line 650)**

New signature and body change (insert `reflector_diameter` after `focal_length`; update the doc comment's Arguments list accordingly):

```rust
pub fn compute_feed_position_from_pointing(
    feed_pointing_pos: &Position3D,
    reflector_pointing_pos: &Position3D,
    vehicle_pos: &Position3D,
    focal_length: f64,
    reflector_diameter: f64,
    attitude: Option<[f64; 4]>,
) -> Result<(f64, f64, f64)> {
```

and replace the final conversion:

```rust
    use crate::model::coordinates::EClockConeCoordinates;
    let ecc = EClockConeCoordinates::from_azimuth_elevation(delta_az, delta_el);

    // Convert angular offset to physical feed position, correcting for the
    // beam deviation factor so the steered beam lands at the requested angle.
    let bdf = beam_deviation_factor(focal_length / reflector_diameter);
    let (x, y, z) = ecc.to_feed_position_with_bdf(focal_length, bdf);

    Ok((x, y, z))
```

- [ ] **Step 6: Update both service callers and the evaluator test**

`antenna-model/src/service/evaluator.rs` (~line 141):

```rust
    let (steer_x, steer_y, steer_z) = compute_feed_position_from_pointing(
        &request.feed_position,
        &request.reflector_boresight,
        &request.vehicle_position,
        focal_length_m,
        diameter_m,
        request.vehicle_attitude,
    )?;
```

`antenna-model/src/service/h3_link_budget.rs` (~line 87): same change (both `focal_length_m` and `diameter_m` are already in scope in `build_antenna_config`).

`evaluator.rs` test `test_feed_offset_is_meters_not_degrees` (~line 992): add `10.0` (the test calibration's `diameter_m`) after `focal_length_m` in the direct helper call.

- [ ] **Step 7: Export `beam_deviation_factor` from `antenna-model/src/model/mod.rs`**

In the `pub use coordinates_3d::{...}` block (line 40), add `beam_deviation_factor,` (keep alphabetical order: after `apply_beam_squint_correction`).

- [ ] **Step 8: Run tests**

Run: `cargo test -p antenna-model`
Expected: PASS. If `steered_beam_peaks_at_requested_angle` finds the peak slightly outside ±0.15°, widen only after confirming BDF is applied (print the scan) — do not silently loosen to hide a wiring bug.

- [ ] **Step 9: Commit**

```bash
git add antenna-model/src/model/coordinates.rs antenna-model/src/model/coordinates_3d.rs antenna-model/src/model/mod.rs antenna-model/src/service/evaluator.rs antenna-model/src/service/h3_link_budget.rs antenna-model/tests/beam_steering_direction.rs
git commit -m "feat: apply beam deviation factor to feed steering displacement

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Remove the near-boresight direct-path interference mode

**Goal:** Delete the physically indefensible direct-path model (hardcoded 0.1 amplitude, feed pattern evaluated as if the feed faced the far field, coherent sum across inconsistent normalizations); near-boresight queries with offset feeds fall through to standard PO.

**Files:**
- Delete: `antenna-model/src/model/direct_path.rs`
- Modify: `antenna-model/src/model/pattern.rs` (lines 36–49 imports, 259–261 dispatch arm, 402–445 `compute_gain_direct_path`)
- Modify: `antenna-model/src/model/edge_cases.rs` (variant, fields, selection logic, warning, 2 tests)
- Modify: `antenna-model/src/model/mod.rs:16,22,97` (module decl, doc bullet, re-export)

**Acceptance Criteria:**
- [ ] `ComputationMode` has three variants; θ < 1° with 0.1f–0.3f offset selects `StandardPhysicalOptics`
- [ ] No references to `direct_path`, `NearBoresightDirectPath`, `has_direct_path`, or `is_near_boresight` remain: `grep -rn "direct_path\|NearBoresightDirectPath\|has_direct_path\|is_near_boresight" antenna-model/src calibrate/src` returns nothing
- [ ] `cargo test --all` green

**Verify:** `cargo test --all` and the grep above → empty

**Steps:**

- [ ] **Step 1: Update `edge_cases.rs`**

Remove the `NearBoresightDirectPath` variant (line 48) and its doc comment. Remove the `is_near_boresight` (lines 63–65) and `has_direct_path` (lines 66–68) fields from `EdgeCaseAnalysis`. In `analyze_edge_cases`, remove lines 102–103 (`is_near_boresight` computation), 108–109 (`has_direct_path`), the direct-path warning block (lines 136–139), and the two struct-literal fields (lines 145–146); change the mode-selection call to `select_computation_mode(offset_ratio)`. Simplify:

```rust
/// Select appropriate computation mode based on edge case analysis
fn select_computation_mode(offset_ratio: f64) -> ComputationMode {
    // Priority: ray tracing > higher-order > standard
    if offset_ratio > SEVERE_OFFSET_THRESHOLD {
        info!(
            "Severe offset threshold detected ({:.3} > {:.1}), switching to ray-tracing",
            offset_ratio, SEVERE_OFFSET_THRESHOLD
        );
        ComputationMode::RayTracing
    } else if offset_ratio > LARGE_OFFSET_THRESHOLD {
        info!(
            "Large offset threshold detected ({:.3} > {:.1}), computing higher-order aberrations",
            offset_ratio, LARGE_OFFSET_THRESHOLD
        );
        ComputationMode::HigherOrderAberrations
    } else {
        info!(
            "Using standard physical optics (offset_ratio={:.3} <= threshold={:.1})",
            offset_ratio, LARGE_OFFSET_THRESHOLD
        );
        ComputationMode::StandardPhysicalOptics
    }
}
```

Check whether `NEAR_BORESIGHT_THRESHOLD` (line 33) still has references (`grep -rn NEAR_BORESIGHT_THRESHOLD antenna-model/src`); if only `mod.rs`'s re-export remains, delete the const and the re-export.

Replace the two tests that assert the removed mode. `test_near_boresight_direct_path` (~line 445) becomes:

```rust
    #[test]
    fn test_near_boresight_moderate_offset_uses_standard_po() {
        // θ < 1° with a 0.1f–0.3f offset previously selected the (removed)
        // direct-path mode; it must now fall through to standard PO.
        let config = offset_antenna(0.15); // helper already in this test module
        let analysis = analyze_edge_cases(&config, 0.005, 0.0);
        assert_eq!(analysis.mode, ComputationMode::StandardPhysicalOptics);
    }
```

(Use the existing test-antenna helper in that module — read the surrounding tests for its exact name/signature and construct a 0.15·f offset with it.) Update the mode-priority test at ~line 539 to drop its direct-path case.

- [ ] **Step 2: Update `pattern.rs`**

Remove `direct_path::compute_with_direct_path` and `far_field_normalization` from the `use crate::model::{...}` block (keep `integrate_amplitude_squared`, `integrate_aperture`, `IntegrationParams`). Remove the dispatch arm:

```rust
        ComputationMode::NearBoresightDirectPath => {
            compute_gain_direct_path(theta, phi, config, frequency_hz, wavelength, params, &mut warnings)?
        }
```

Delete the whole `compute_gain_direct_path` function (lines ~402–445).

- [ ] **Step 3: Delete the module**

```bash
git rm antenna-model/src/model/direct_path.rs
```

In `mod.rs`: remove line 22 (`pub mod direct_path;`), line 97 (`pub use direct_path::{...};`), and the "Direct Path" bullet from the module doc (line 16).

- [ ] **Step 4: Verify and test**

Run: `grep -rn "direct_path\|NearBoresightDirectPath\|has_direct_path\|is_near_boresight" antenna-model/src calibrate/src` → expected: no output.
Run: `cargo test --all` → expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: remove physically unsound direct-path interference mode

The mode coherently summed a hardcoded 0.1-amplitude direct field (with the
feed pattern evaluated as if the feed faced the far field, not the reflector)
against a reflected field on a different normalization scale. Near-boresight
offset-feed queries now use standard physical optics.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: Wire `phase_center_offset` into the axial defocus term

**Goal:** The calibration parameter `phase_center_offset_m` currently has zero effect; make it contribute to the feed's axial offset (defocus), and delete the never-called `phase_center_offset_phase` helper.

**Files:**
- Modify: `antenna-model/src/model/integration.rs:513-514` (`aperture_integrand`) + new test
- Modify: `antenna-model/src/model/illumination.rs` (delete `phase_center_offset_phase` ~line 357 and its tests ~lines 578–607)
- Modify: `antenna-model/src/model/mod.rs:57` (drop the export)
- Modify: `antenna-model/src/model/geometry.rs:184-186` (doc: state the sign convention)

**Acceptance Criteria:**
- [ ] Boresight gain with `phase_center_offset = 0.05` m is at least 1 dB below the `phase_center_offset = 0` gain on the 1 m test dish at 8.4 GHz (defocus loss)
- [ ] `phase_center_offset_phase` no longer exists anywhere
- [ ] `cargo test --all` green

**Verify:** `cargo test -p antenna-model integration` → new test passes

**Steps:**

- [ ] **Step 1: Write the failing test** (append to the `tests` module in `integration.rs`)

```rust
    /// phase_center_offset must act as an axial defocus. Before the fix it was
    /// parsed and validated but never entered the physics, so both gains were
    /// identical and this test failed.
    #[test]
    fn test_phase_center_offset_produces_defocus_loss() {
        let reflector = ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap();
        let feed_focused =
            FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
        let feed_pco = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.05, 1.0).unwrap();

        let mk = |feed| {
            AntennaConfiguration::new("t".into(), "T".into(),
                ReflectorGeometry::new(1.0, 0.5, 0.0).unwrap(), feed, None).unwrap()
        };
        let _ = reflector;

        let params = crate::model::integration::IntegrationParams::default();
        let g_focused = crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_focused), 8.4e9, &params)
            .unwrap().gain;
        let g_pco = crate::model::pattern::compute_gain_db(0.0, 0.0, &mk(feed_pco), 8.4e9, &params)
            .unwrap().gain;

        assert!(
            g_focused - g_pco > 1.0,
            "5 cm phase-center offset must cost >1 dB defocus at 8.4 GHz: focused={g_focused:.2}, pco={g_pco:.2}"
        );
    }
```

Run: `cargo test -p antenna-model test_phase_center_offset_produces_defocus_loss` → expected: FAIL (gains identical).

- [ ] **Step 2: Wire it in `aperture_integrand` (`integration.rs` ~line 513)**

```rust
    // Axial offset of the feed's PHASE CENTER from the focal point:
    // physical z-offset plus the phase-center offset along the feed axis
    // (positive = away from the vertex, matching phase_feed_displacement's delta_z).
    let feed_axial_offset = config.feed.position.z - config.reflector.focal_length
        + config.feed.phase_center_offset;
```

- [ ] **Step 3: Document the sign convention on the field (`geometry.rs:184`)**

```rust
    /// Phase center offset in meters (distance from physical feed to phase center,
    /// along the feed axis; positive = away from the reflector vertex).
    /// Typically ±λ/4, frequency-dependent. Enters the physics as an additional
    /// axial defocus term in the aperture phase.
    pub phase_center_offset: f64,
```

- [ ] **Step 4: Delete `phase_center_offset_phase`**

Remove the function (~line 357 in `illumination.rs`, including its doc/doctest block starting ~line 340) and its four unit tests (~lines 578–607). Remove `phase_center_offset_phase,` from `mod.rs:57`.

- [ ] **Step 5: Test and commit**

Run: `cargo test --all` → PASS.

```bash
git add antenna-model/src/model/integration.rs antenna-model/src/model/illumination.rs antenna-model/src/model/mod.rs antenna-model/src/model/geometry.rs
git commit -m "fix: phase_center_offset now contributes axial defocus (was silently ignored)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: Use the calibration's temperature constant for correction lookups

**Goal:** Replace the three hardcoded `290.0` K arguments with `calibration.validity_ranges.temperature_const` so artifacts calibrated at other temperatures don't silently extrapolate.

**Files:**
- Modify: `antenna-model/src/service/evaluator.rs:271` + new test
- Modify: `antenna-model/src/service/h3_link_budget.rs:235` and `:383`

**Acceptance Criteria:**
- [ ] A calibration with `temperature_const = 300.0` and a correction surface whose temperature knots are all 300.0 produces **no** temperature-extrapolation warning and `metadata.extrapolated == false`
- [ ] No literal `290.0` remains in `evaluator.rs`/`h3_link_budget.rs` correction calls: `grep -n "290.0" antenna-model/src/service/evaluator.rs antenna-model/src/service/h3_link_budget.rs` shows only comments/tests
- [ ] `cargo test --all` green

**Verify:** `cargo test -p antenna-model test_correction_uses_calibration_temperature` → PASS

**Steps:**

- [ ] **Step 1: Write the failing test** (append to `evaluator.rs` tests; the knot layout copies the valid fixture from `correction_interpolator.rs::test_evaluate_correction_simple`, widened to cover the test request and re-based at 300 K)

```rust
    /// The correction surface must be evaluated at the calibration's
    /// temperature_const, not a hardcoded 290 K. This artifact is calibrated
    /// at 300 K; with the old hardcoded 290 K the temperature dimension
    /// extrapolated and emitted a warning.
    #[test]
    fn test_correction_uses_calibration_temperature() {
        let mut repo = CalibrationRepository::new();
        let mut calibration = create_test_calibration(CalibrationStatus::FullyCalibrated {
            accuracy_estimate_db: 1.0,
        });
        calibration.validity_ranges.temperature_const = 300.0;
        calibration.correction_surface = Some(crate::data::types::BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2 * 1],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 360.0, 360.0, 360.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![300.0, 300.0, 300.0, 300.0, 300.0, 300.0],
            spline_order: 3,
        });
        repo.add_calibration(calibration);

        let request = create_test_request();
        let response = compute_gain_from_request(&request, &repo).unwrap();

        assert!(
            !response.warnings.iter().any(|w| w.contains("temperature")),
            "no temperature extrapolation warning expected, got: {:?}",
            response.warnings
        );
        assert!(!response.metadata.extrapolated);
    }
```

Run: `cargo test -p antenna-model test_correction_uses_calibration_temperature` → expected: FAIL (warning "Correction surface extrapolated for: temperature (290.0 K)" and `extrapolated == true`).

- [ ] **Step 2: Fix the three call sites**

`evaluator.rs:271` — replace the literal:

```rust
            let result = evaluate_correction(
                correction,
                corrected_az,
                corrected_el,
                request.frequency_mhz,
                calibration.validity_ranges.temperature_const,
            )?;
```

`h3_link_budget.rs:235` (in `compute_cell_gain`):

```rust
            let corr = evaluate_correction(
                surface,
                az_deg,
                el_deg,
                request.frequency_mhz,
                calibration.validity_ranges.temperature_const,
            )?;
```

`h3_link_budget.rs:383` (boresight block): same substitution.

Also update the comment at `h3_link_budget.rs:130-143` (the "290.0 K temperature constant" sentence) to say "the calibration's `temperature_const`".

- [ ] **Step 3: Test and commit**

Run: `cargo test --all` → PASS.

```bash
git add antenna-model/src/service/evaluator.rs antenna-model/src/service/h3_link_budget.rs
git commit -m "fix: evaluate correction surface at calibration temperature_const, not 290 K

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: Guard against malformed correction-surface knot vectors

**Goal:** `find_knot_span` indexes `knots[order-1]` and `knots[len-order]`; a knot vector shorter than `order+1` panics. Return a proper error instead (CLAUDE.md forbids panics in production paths).

**Files:**
- Modify: `antenna-model/src/model/correction_interpolator.rs` (`evaluate_correction`, after the finite-input check ~line 105) + new test

**Acceptance Criteria:**
- [ ] `evaluate_correction` on a model with `knots_temperature: vec![290.0]` and `spline_order: 3` returns `Err` (not a panic)
- [ ] Well-formed models behave identically (existing interpolator tests untouched and green)

**Verify:** `cargo test -p antenna-model correction_interpolator` → PASS including new test

**Steps:**

- [ ] **Step 1: Write the failing test** (append to `correction_interpolator.rs` tests; base the fixture on `test_evaluate_correction_simple`, replacing only the temperature knots)

```rust
    /// A knot vector shorter than spline_order+1 must produce an error, not an
    /// out-of-bounds panic in find_knot_span.
    #[test]
    fn test_short_knot_vector_returns_error_not_panic() {
        let model = BSplineModel4D {
            coefficients: vec![1.0; 2 * 2 * 2 * 1],
            shape: [2, 2, 2, 1],
            knots_azimuth: vec![0.0, 0.0, 0.0, 10.0, 10.0, 10.0],
            knots_elevation: vec![0.0, 0.0, 0.0, 20.0, 20.0, 20.0],
            knots_frequency: vec![8000.0, 8000.0, 8000.0, 9000.0, 9000.0, 9000.0],
            knots_temperature: vec![290.0], // malformed: needs >= order+1 = 4
            spline_order: 3,
        };
        let result = evaluate_correction(&model, 5.0, 10.0, 8500.0, 290.0);
        assert!(result.is_err(), "expected Err for short knot vector");
    }
```

Run: `cargo test -p antenna-model test_short_knot_vector_returns_error_not_panic` → expected: FAIL with a panic (index out of bounds), which the harness reports as test failure.

- [ ] **Step 2: Add the guard in `evaluate_correction`** (immediately after the non-finite input check, before `find_knot_span` calls)

```rust
    // Guard against malformed knot vectors: find_knot_span indexes
    // knots[order-1] and knots[len-order], which panics if len < order+1.
    let order = model.spline_order as usize;
    for (name, knots) in [
        ("azimuth", &model.knots_azimuth),
        ("elevation", &model.knots_elevation),
        ("frequency", &model.knots_frequency),
        ("temperature", &model.knots_temperature),
    ] {
        if knots.len() < order + 1 {
            return Err(ComputationError::InterpolationFailed {
                azimuth: azimuth_deg,
                elevation: elevation_deg,
                frequency: frequency_mhz,
                temperature: temperature_k,
                reason: format!(
                    "{} knot vector has {} knots; a spline of order {} needs at least {}",
                    name,
                    knots.len(),
                    order,
                    order + 1
                ),
            }
            .into());
        }
    }
```

- [ ] **Step 3: Test and commit**

Run: `cargo test -p antenna-model` → PASS.

```bash
git add antenna-model/src/model/correction_interpolator.rs
git commit -m "fix: reject malformed correction-surface knot vectors instead of panicking

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: Correct API semantics documentation and stale claims

**Goal:** Make the aim-point semantics of `feed_position` explicit everywhere a client or maintainer would look, fix the flipped z-sign doc, the loss comment, the example expectations, and CLAUDE.md's false Zernike claim.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:239-240` and `:316-321`
- Modify: `openapi.yaml` (~line 560, `feed_position` in the gain request schema; apply the same pattern at the other `feed_position` request sites found at lines 133/157/349 if they are request schemas)
- Modify: `antenna-model/src/service/evaluator.rs:295,316` (comment only)
- Modify: `examples/requests/geo_feed_emitter_colocated_offset.json` (`_description`), `examples/README_GEO_EXAMPLES.md` (§2)
- Modify: `CLAUDE.md` (two Zernike mentions)

**Acceptance Criteria:**
- [ ] `schemas.rs` `feed_position` doc states it is an Earth aim point, NOT the feed's physical location
- [ ] `feed_offset_meters` z-sign doc matches `phase.rs` ("positive = away from the reflector vertex")
- [ ] `cargo build` clean; no code behavior change (docs/comments/JSON only)

**Verify:** `cargo build --all` → clean; `grep -n "aimed" antenna-model/src/api/schemas.rs` → shows the new doc

**Steps:**

- [ ] **Step 1: `schemas.rs` — feed_position doc (line 239)**

```rust
    /// Feed pointing target (ECEF or Geodetic).
    ///
    /// **This is the Earth location the feed's beam is aimed at — NOT the
    /// feed's physical location on the antenna.** The service converts the
    /// angular offset between this aim point and `reflector_boresight` into a
    /// physical feed displacement in the antenna frame (including the beam
    /// deviation factor). To model an unsteered (focused) feed, set this equal
    /// to `reflector_boresight`.
    pub feed_position: Position3D,
```

- [ ] **Step 2: `schemas.rs` — GeometryInfo z-sign doc (lines 316–321)**

```rust
    /// Physical feed offset from the focal point in the antenna frame (meters).
    ///
    /// `x` and `y` are the lateral displacement of the feed from the optical axis;
    /// `z` is the axial displacement from the focal point (**positive = away from
    /// the reflector vertex**, matching the phase model's `delta_z` convention).
    /// For an on-axis (boresight-aimed) feed all three components are ~zero.
    pub feed_offset_meters: Vector3D,
```

- [ ] **Step 3: `openapi.yaml` — feed_position description (~line 560)**

A sibling `description` next to `$ref` is ignored in OpenAPI 3.0, so wrap in `allOf`:

```yaml
        feed_position:
          allOf:
            - $ref: '#/components/schemas/Position3D'
          description: >-
            Earth location the feed's beam is aimed at (NOT the feed's physical
            location on the antenna). Set equal to reflector_boresight for an
            unsteered/focused feed.
```

Apply the same `allOf` + description to the `feed_position` properties at the other request-schema sites (lines ~133, ~157, ~349 — verify each is a request schema, not an example, before editing).

- [ ] **Step 4: `evaluator.rs` — fix the loss comment (lines ~295 and ~316)**

The code computes `reference.gain - final_gain_db` where `final_gain_db` includes the correction surface. Change the parenthetical at line ~316:

```rust
        // Loss is reference minus actual gain (final gain, including the
        // correction surface when it was applied).
```

- [ ] **Step 5: Update the example expectation**

`examples/requests/geo_feed_emitter_colocated_offset.json` — replace `_description` with:

```
"_description": "Feed steered 5° off-axis (lateral ~1.37m at focal plane incl. BDF). Emitter at ground location 5° from boresight in same direction, i.e. aligned with the steered beam. Expected: gain near the steered-beam peak; the peak itself is well below boresight gain because a 5° steer is many beamwidths for a 34 m dish (severe coma/scan loss). Re-derive the numeric expectation from the service after the steering-sign fix.",
```

`examples/README_GEO_EXAMPLES.md` §2 (~lines 56–70): update the bullet "Emitter at ground location 5° from boresight (same direction as feed)" to add: "— the emitter sits in the *steered beam*, so the response reports gain near the steered-beam peak (heavily reduced by scan loss), not a value 180° away."

- [ ] **Step 6: CLAUDE.md — correct the Zernike claim (two sites)**

In "Key Architecture" item 1, change "surface error via Zernike polynomials" → "surface error via the statistical Ruze efficiency". In "Key Physics Modules" → `phase.rs` bullet, change "surface error (Zernike)" → "surface error (statistical Ruze model; per-point Zernike maps are not implemented — the calibration correction surface absorbs systematic surface deviations)".

- [ ] **Step 7: Build and commit**

Run: `cargo build --all` → clean.

```bash
git add antenna-model/src/api/schemas.rs openapi.yaml antenna-model/src/service/evaluator.rs examples/ CLAUDE.md
git commit -m "docs: clarify feed_position aim-point semantics; fix z-sign and loss comments

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: Remove dead code and add f/D consistency validation

**Goal:** Delete production-dead modules/functions confirmed unused (checked against both crates), and validate `f_over_d_ratio` against `focal_length_m / diameter_m` at artifact load.

**Files:**
- Delete: `antenna-model/src/model/numerical_stability.rs`
- Modify: `antenna-model/src/model/mod.rs` (lines 17, 28, 99–102 + affected export lists)
- Modify: `antenna-model/src/model/coordinates_3d.rs` (delete `compute_feed_offset_v2`, lines ~573–627, plus any of its tests in the same file)
- Modify: `antenna-model/src/model/coordinates.rs` (delete `AzElCoordinates` struct/impl ~lines 267–314 and `test_azel_to_far_field`)
- Modify: `antenna-model/src/model/surface.rs` (delete unused surface-error models — see decision rule)
- Modify: `antenna-model/src/model/mesh.rs` (delete unused exports — see decision rule)
- Modify: `antenna-model/src/data/types.rs` (`ReflectorGeometry::validate` ~line 358) + new test

**Pre-verified:** `calibrate` imports only `compute_g_over_t`, `AntennaConfiguration(Builder)`, `FeedParametersBuilder`, `IntegrationParams`, `MeshParametersBuilder`, `ReflectorGeometryBuilder`, `EClockConeCoordinates` (only `.to_far_field()`), `evaluate_correction`, and `BSplineModel4D` — none of the items deleted here.

**Acceptance Criteria:**
- [ ] `numerical_stability.rs`, `compute_feed_offset_v2`, `AzElCoordinates` gone; `grep -rn "numerical_stability\|compute_feed_offset_v2\|AzElCoordinates" antenna-model/src calibrate/src` → empty
- [ ] Every remaining `pub` item in `surface.rs`/`mesh.rs` has at least one non-test caller in either crate (or is in the internal call chain of one that does)
- [ ] `ReflectorGeometry::validate` rejects `f_over_d_ratio` inconsistent with `focal_length_m/diameter_m` by more than 1%
- [ ] `cargo test --all` green and `cargo clippy --all -- -D warnings` clean

**Verify:** `cargo test --all && cargo clippy --all -- -D warnings` → clean

**Steps:**

- [ ] **Step 1: Delete the fully dead items**

```bash
git rm antenna-model/src/model/numerical_stability.rs
```

In `mod.rs`: remove line 28 (`pub mod numerical_stability;`), lines 99–102 (its `pub use` block), and the "Numerical Stability" doc bullet (line 17). In `coordinates_3d.rs`: delete `compute_feed_offset_v2` (~lines 573–627) and remove it from `mod.rs:42`; search the file's test module for tests calling it and delete those too (`grep -n compute_feed_offset_v2 antenna-model/src/model/coordinates_3d.rs`). In `coordinates.rs`: delete `AzElCoordinates` (struct + impl, ~lines 267–314), its test `test_azel_to_far_field`, and remove `AzElCoordinates,` from `mod.rs:36`.

- [ ] **Step 2: Prune `surface.rs` by decision rule**

For each of `SurfaceErrorModel`, `IdealSurface`, `GaussianSurface`, `ZernikeSurface`, `ZernikeIndex`, `zernike_polynomial`, `compute_surface_rms`, `ruze_efficiency` (surface.rs's copy), `ruze_efficiency_from_frequency`, run:

```bash
grep -rn "<name>" antenna-model/src calibrate/src --include="*.rs" | grep -v "surface.rs\|mod.rs"
```

Delete every item with no hits (plus its tests and its `mod.rs:76-80` export). Expected outcome based on review: the trait, all three model structs, `ZernikeIndex`, and `zernike_polynomial` are deletable; keep whichever of the RMS/Ruze helpers show real callers (the pipeline's Ruze lives in `pattern.rs`, so surface.rs may empty out entirely — if so, delete the module and its `mod.rs` references, and remove the "Zernike polynomials" bullet from the module doc).

- [ ] **Step 3: Prune `mesh.rs` exports by the same rule**

For each export in `mod.rs:82-87` (`angle_correction_factor`, `basic_transparency`, `cutoff_wavelength`, `effective_cutoff_wavelength`, `mesh_efficiency`, `mesh_efficiency_simple`, `mesh_reflection_coefficient`, `mesh_reflection_efficiency`, `mesh_transparency_polarized`, `mesh_transparency_with_angle`, `transparency_with_diameter`, `Polarization`), grep both crates excluding `mesh.rs` itself and `mod.rs`. Keep `mesh_reflection_efficiency` (used by `pattern.rs`) and anything in its internal call chain; demote internal-chain helpers from the `mod.rs` re-export list to plain `pub` (or `pub(crate)`) in `mesh.rs`; delete exports and functions with no callers at all (with their tests).

- [ ] **Step 4: Add f/D cross-validation with test**

Test first (append to `types.rs` tests):

```rust
    #[test]
    fn test_reflector_geometry_rejects_inconsistent_f_over_d() {
        let geom = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 5.0,
            f_over_d_ratio: 0.6, // truth is 0.5
            surface_rms_mm: 0.5,
        };
        assert!(geom.validate().is_err());

        let consistent = ReflectorGeometry {
            diameter_m: 10.0,
            focal_length_m: 5.0,
            f_over_d_ratio: 0.5,
            surface_rms_mm: 0.5,
        };
        assert!(consistent.validate().is_ok());
    }
```

Run: `cargo test -p antenna-model test_reflector_geometry_rejects_inconsistent_f_over_d` → FAIL. Then add to `ReflectorGeometry::validate` (after the existing `f_over_d_ratio` range check, ~line 366):

```rust
        let implied_f_over_d = self.focal_length_m / self.diameter_m;
        if (self.f_over_d_ratio - implied_f_over_d).abs() > 0.01 * implied_f_over_d {
            return Err(ValidationError::InvalidPhysicalParameter {
                parameter: "f_over_d_ratio".to_string(),
                value: self.f_over_d_ratio,
                reason: format!(
                    "inconsistent with focal_length_m/diameter_m = {:.4}",
                    implied_f_over_d
                ),
            });
        }
```

Then check the shipped artifacts still load: `grep -rn "f_over_d" calibration_data/*.toml design_specs/ 2>/dev/null` and run the full suite — if a checked-in artifact carries an inconsistent ratio, fix the artifact/config value (it is redundant data), not the validation.

- [ ] **Step 5: Full check and commit**

Run: `cargo test --all && cargo clippy --all -- -D warnings` → clean. (macOS: if `calibrate` tests fail on BLAS linking, prefix with `LDFLAGS="-L/opt/homebrew/opt/openblas/lib" CPPFLAGS="-I/opt/homebrew/opt/openblas/include"`.)

```bash
git add -A
git commit -m "refactor: remove dead physics helpers; validate f_over_d_ratio consistency

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 9: Final verification sweep

**Goal:** Prove the whole workspace is healthy after all changes and that the headline bug is demonstrably fixed end-to-end.

**Files:**
- No new files; runs commands only (fix anything that surfaces)

**Acceptance Criteria:**
- [ ] `cargo fmt --all -- --check` clean
- [ ] `cargo clippy --all -- -D warnings` clean
- [ ] `cargo test --all` green
- [ ] `cargo doc --no-deps` builds without warnings about broken intra-doc links to deleted items
- [ ] Steering regression tests pass: `cargo test -p antenna-model --test beam_steering_direction`

**Verify:** all five commands above → clean/pass

**Steps:**

- [ ] **Step 1: Run the sweep**

```bash
cargo fmt --all
cargo clippy --all -- -D warnings
cargo test --all
cargo doc --no-deps 2>&1 | grep -i "warning" || echo "doc clean"
cargo test -p antenna-model --test beam_steering_direction -- --nocapture
```

Expected: everything passes; the `--nocapture` run prints target-side vs opposite-side gains showing the beam on the requested side.

- [ ] **Step 2: Commit any formatting fallout**

```bash
git add -A
git commit -m "chore: post-review-fixes verification sweep (fmt/clippy)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>" || echo "nothing to commit"
```

---

## Self-Review Notes

- **Coverage:** every finding from the review report maps to a task (table at top); azimuth periodicity, spillover, and ray-trace replacement are explicitly deferred with rationale.
- **Type consistency:** `to_feed_position_with_bdf(focal_length, bdf)` defined in Task 2 Step 4 matches its use in Task 2 Steps 1/5; `compute_feed_position_from_pointing` gains `reflector_diameter` as the 5th positional parameter and both callers plus the evaluator test pass `diameter_m`/`10.0` accordingly; `beam_deviation_factor(f_over_d)` takes f/D (callers pass `focal_length / diameter`).
- **Ordering constraint:** Task 1 must land before Task 3 only because Task 1 edits a test inside `direct_path.rs` that Task 3 deletes; executing 3 before 1 would make Task 1's Step 6 partially inapplicable (skip the deleted file's edit in that case). The `.tasks.json` dependency graph encodes 1→2 and (3,4)→8; run 1 first regardless.
