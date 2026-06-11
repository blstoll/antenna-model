# Review Findings Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers-extended-cc:subagent-driven-development (recommended) or superpowers-extended-cc:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the correctness and consistency defects documented in `docs/review-findings-2026-06-10.md`, critical findings first, so the service produces physically correct patterns, loads calibration artifacts from both calibrate modes, and applies correction surfaces consistently.

**Architecture:** The fixes are localized: physics formula corrections in `antenna-model/src/model/` (phase, mesh, illumination, gain normalization), convention/validation fixes at the service boundary (`api/schemas.rs`, `model/coordinates_3d.rs`, `service/evaluator.rs`, `service/h3_link_budget.rs`), and artifact-format unification between `calibrate` and the service loader (`data/loader.rs`, `calibrate/src/main.rs`). Tasks 1–6 are the critical fixes; 7–16 are high/medium. Several fixes change computed gain values — **any existing calibration artifacts must be regenerated after Tasks 1, 2, 9, 11, and 14** (the physics changes invalidate fitted residuals).

**Tech Stack:** Rust workspace (`antenna-model` service + `calibrate` CLI), poem, bincode 2, ndarray, rayon, h3o. Tests via `cargo test`. macOS note: `calibrate` tests may need `LDFLAGS="-L/opt/homebrew/opt/openblas/lib" CPPFLAGS="-I/opt/homebrew/opt/openblas/include"`.

**Reference:** Finding numbers (F1–F15 + medium items) refer to `docs/review-findings-2026-06-10.md`.

---

## Task 1: Fix the parabolic path phase (F1 — critical)

**Goal:** Add the missing `(1−cosθ)` factor to `phase_path` so the aperture is equiphase at boresight, restoring correct pattern shape.

**Files:**
- Modify: `antenna-model/src/model/phase.rs:88-99` (function), `phase.rs:64-86` (doc comment), tests `phase.rs:502-528`
- Modify: `antenna-model/src/model/pattern.rs` (tighten `test_compute_beamwidth`, pattern.rs:932-944)
- Check: `docs/antenna-model-design-doc.md` Section 2.2 — if it states the unfactored formula, correct it there too

**Acceptance Criteria:**
- [ ] `phase_path(rho, _, 0.0, _, f, k) == 0.0` for any `rho` (boresight equiphase)
- [ ] Off-axis formula is `k·[ρ²/(4f)·(1−cosθ) − ρ·sinθ·cos(φ−φ′)]`
- [ ] Half-power beamwidth for the 1 m / f-D 0.5 dish at 8.4 GHz lands in 0.9°–2.0° (boresight-to-−3 dB), down from the old 0.5°–10° tolerance
- [ ] All existing `cargo test -p antenna-model` tests pass (after updating the two phase_path unit tests' expected values)

**Verify:** `cargo test -p antenna-model phase_path && cargo test -p antenna-model test_compute_beamwidth` → all PASS

**Steps:**

- [ ] **Step 1: Write the failing tests** (replace the two existing `phase_path` tests and add a boresight-equiphase test in `phase.rs`):

```rust
#[test]
fn test_phase_path_boresight_equiphase() {
    // For a parabola fed at focus, the aperture is equiphase at theta=0.
    let focal_length = 17.0;
    let k = wavenumber(0.03);
    for rho in [0.0, 1.0, 5.0, 10.0, 17.0] {
        let phase = phase_path(rho, 0.7, 0.0, 0.0, focal_length, k);
        assert!(phase.abs() < EPSILON, "rho={rho}: phase={phase}");
    }
}

#[test]
fn test_phase_path_off_axis() {
    let focal_length = 17.0;
    let k = wavenumber(0.03);
    let (rho, phi_prime, theta, phi) = (5.0, PI / 4.0, 0.1, PI / 3.0);
    let phase = phase_path(rho, phi_prime, theta, phi, focal_length, k);
    let term1 = rho * rho / (4.0 * focal_length) * (1.0 - theta.cos());
    let term2 = rho * theta.sin() * (phi - phi_prime).cos();
    assert!((phase - k * (term1 - term2)).abs() < EPSILON);
}
```

Also update `test_phase_path_on_axis` (phase.rs:502-510): expected value becomes `0.0`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p antenna-model phase_path`
Expected: FAIL (`test_phase_path_boresight_equiphase` asserts nonzero phase with current code)

- [ ] **Step 3: Fix the implementation** (phase.rs:88-99):

```rust
pub fn phase_path(
    rho: f64,
    phi_prime: f64,
    theta: f64,
    phi: f64,
    focal_length: f64,
    k: f64,
) -> f64 {
    // Feed→surface path is k(f+z); far-field projection removes
    // k(ρ·sinθ·cos(φ−φ′) + z·cosθ). Dropping the constant kf:
    //   Ψ = k·[z·(1−cosθ) − ρ·sinθ·cos(φ−φ′)],  z = ρ²/(4f)
    let term1 = rho * rho / (4.0 * focal_length) * (1.0 - theta.cos());
    let term2 = rho * theta.sin() * (phi - phi_prime).cos();
    k * (term1 - term2)
}
```

Update the formula in the doc comment (phase.rs:69-75) to match, noting the derivation.

- [ ] **Step 4: Tighten the beamwidth test** (pattern.rs:932-944):

```rust
#[test]
fn test_compute_beamwidth() {
    let config = test_antenna();
    let params = IntegrationParams::fast();
    let hpbw = compute_beamwidth(&config, 8.4e9, 3.0, 0.0, &params).unwrap();
    // 1 m dish at 8.4 GHz: full HPBW ≈ 1.05–1.2·λ/D ≈ 2.1°–2.5°;
    // this function returns boresight→−3dB (half of that), widened by the
    // q=8 taper. Anything outside 0.9°–2.0° indicates a phase model bug.
    let half_deg = hpbw.to_degrees();
    assert!(half_deg > 0.9 && half_deg < 2.0, "got {half_deg}°");
}
```

- [ ] **Step 5: Run the full model test suite, fix any tests asserting old defocused-pattern values**

Run: `cargo test -p antenna-model`
Expected: PASS (update any pattern/integration tests whose expected magnitudes baked in the old phase)

- [ ] **Step 6: Check the design doc** — open `docs/antenna-model-design-doc.md` Section 2.2; if it states `Ψ_path = k·[ρ²/(4f) − ρ·sinθ·cos(φ−φ′)]`, correct it to include `(1−cosθ)` with a changelog note.

- [ ] **Step 7: Commit**

```bash
git add antenna-model/src/model/phase.rs antenna-model/src/model/pattern.rs docs/antenna-model-design-doc.md
git commit -m "fix: add missing (1-cos theta) factor to parabolic path phase"
```

---

## Task 2: Replace the inverted mesh-loss model (F4 — critical)

**Goal:** Replace `pattern.rs::mesh_transparency` with a physically sensible, continuous mesh reflection-efficiency model (inductive-grid formula) wired into `overall_efficiency`.

**Files:**
- Modify: `antenna-model/src/model/mesh.rs` (add `mesh_reflection_efficiency`)
- Modify: `antenna-model/src/model/pattern.rs:109-175` (delete `mesh_transparency`, update `overall_efficiency` and its doctests/tests at pattern.rs:779-849)
- Modify: `antenna-model/src/model/mod.rs` (re-export if needed)

**Acceptance Criteria:**
- [ ] Efficiency is continuous in wavelength (no step at λ = π·spacing)
- [ ] Efficiency → 1 as λ → ∞ (mesh reflects like solid at low frequency)
- [ ] Efficiency decreases monotonically as λ decreases (leaks at high frequency)
- [ ] `overall_efficiency` uses the new function; old `mesh_transparency` removed

**Verify:** `cargo test -p antenna-model mesh` → PASS

**Steps:**

- [ ] **Step 1: Write failing tests** in `mesh.rs`:

```rust
#[test]
fn test_reflection_efficiency_low_frequency_is_high() {
    // 5mm mesh, 0.5mm wire at 100 MHz (λ=3m): excellent reflector
    let eff = mesh_reflection_efficiency(0.005, 0.0005, 3.0);
    assert!(eff > 0.99, "got {eff}");
}

#[test]
fn test_reflection_efficiency_monotonic_in_wavelength() {
    let (g, d) = (0.005, 0.0005);
    let mut prev = 0.0;
    for lambda in [0.005, 0.01, 0.0357, 0.1, 1.0, 3.0] {
        let eff = mesh_reflection_efficiency(g, d, lambda);
        assert!(eff >= prev, "non-monotonic at λ={lambda}: {eff} < {prev}");
        prev = eff;
    }
}

#[test]
fn test_reflection_efficiency_continuous_at_old_cutoff() {
    let (g, d) = (0.005, 0.0005);
    let cutoff = std::f64::consts::PI * g;
    let below = mesh_reflection_efficiency(g, d, cutoff * 0.999);
    let above = mesh_reflection_efficiency(g, d, cutoff * 1.001);
    assert!((below - above).abs() < 0.01, "step at cutoff: {below} vs {above}");
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p antenna-model mesh_reflection` → FAIL (function not defined)

- [ ] **Step 3: Implement** in `mesh.rs` (Wait/Marcuvitz inductive-grid shunt model — square mesh, spacing g, wire diameter d, wire radius a=d/2; normalized reactance `X = (g/λ)·ln(g/(2πa))`; power reflectivity `|R|² = 1/(1+4X²)`):

```rust
/// Power reflection efficiency of a square wire mesh (inductive grid model).
///
/// X = (g/λ)·ln(g/(π·d)) is the normalized shunt reactance (a = d/2),
/// |R|² = 1/(1 + 4X²).  λ→∞ ⇒ |R|²→1; small λ ⇒ mesh leaks.
/// Falls back to solid-reflector behaviour when the log term is non-positive
/// (wire so thick the surface is effectively solid).
pub fn mesh_reflection_efficiency(mesh_spacing: f64, wire_diameter: f64, wavelength: f64) -> f64 {
    if mesh_spacing <= 0.0 || wavelength <= 0.0 {
        return 1.0;
    }
    let log_term = (mesh_spacing / (std::f64::consts::PI * wire_diameter)).ln();
    if log_term <= 0.0 {
        return 1.0; // effectively solid
    }
    let x = (mesh_spacing / wavelength) * log_term;
    1.0 / (1.0 + 4.0 * x * x)
}
```

- [ ] **Step 4: Wire into `overall_efficiency`** (pattern.rs:162-175) and delete `mesh_transparency` plus its tests/doctests:

```rust
pub fn overall_efficiency(config: &AntennaConfiguration, wavelength: f64) -> f64 {
    let eta_ruze = ruze_efficiency(config.reflector.surface_rms, wavelength);
    let eta_mesh = if let Some(ref mesh) = config.mesh {
        crate::model::mesh::mesh_reflection_efficiency(mesh.spacing, mesh.wire_diameter, wavelength)
    } else {
        1.0
    };
    eta_ruze * eta_mesh
}
```

Update `test_overall_efficiency_with_mesh` (pattern.rs:837-849) to compare against the new function. Grep for remaining `mesh_transparency` callers: `grep -rn mesh_transparency antenna-model/` and update them all.

- [ ] **Step 5: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/model/mesh.rs antenna-model/src/model/pattern.rs antenna-model/src/model/mod.rs
git commit -m "fix: replace inverted/discontinuous mesh transparency with inductive-grid reflectivity"
```

---

## Task 3: Make coordinate-system detection safe (F3 — critical)

**Goal:** Raise the ECEF auto-detect threshold to the documented 6400 km, allow explicit per-position override, and warn on ambiguous geodetic altitudes.

**Files:**
- Modify: `antenna-model/src/api/schemas.rs:110-136` (Position3D), tests at schemas.rs:1050-1090
- Modify: `antenna-model/src/model/coordinates_3d.rs` tests at 618-639 (boundary values)
- Modify: `antenna-model/src/service/validator.rs` (ambiguity warning)

**Acceptance Criteria:**
- [ ] Threshold constant is `6_400_000.0` m; doc comments updated (CLAUDE.md already says 6400 km)
- [ ] `Position3D` gains optional `coordinate_system: Option<CoordinateSystem>` (serde default `None`, ignored on the wire when absent — backward compatible)
- [ ] Explicit value overrides auto-detection
- [ ] Service validation emits a warning when a position auto-detects as geodetic with `|z| > 100_000` m (high-altitude geodetic is legal but ambiguous) and when it auto-detects as ECEF without an explicit tag

**Verify:** `cargo test -p antenna-model coordinate` → PASS

**Steps:**

- [ ] **Step 1: Failing tests** in schemas.rs:

```rust
#[test]
fn test_explicit_coordinate_system_overrides_detection() {
    let mut pos = Position3D::new(0.0, 0.0, 35_786_000.0); // GEO altitude, geodetic form
    pos.coordinate_system = Some(CoordinateSystem::Geodetic);
    assert!(pos.is_geodetic());

    let mut pos2 = Position3D::new(100.0, 100.0, 100.0);
    pos2.coordinate_system = Some(CoordinateSystem::ECEF);
    assert!(pos2.is_ecef());
}

#[test]
fn test_detection_threshold_is_6400km() {
    assert!(!Position3D::new(6_399_000.0, 0.0, 0.0).is_ecef());
    assert!(Position3D::new(6_401_000.0, 0.0, 0.0).is_ecef());
}
```

- [ ] **Step 2: Run** — `cargo test -p antenna-model test_explicit_coordinate_system` → FAIL (no such field)

- [ ] **Step 3: Implement** in schemas.rs:

```rust
pub struct Position3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Optional explicit coordinate system. When `None`, auto-detected by magnitude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_system: Option<CoordinateSystem>,
}

impl Position3D {
    pub const ECEF_THRESHOLD_M: f64 = 6_400_000.0;

    pub fn coordinate_system(&self) -> CoordinateSystem {
        if let Some(cs) = self.coordinate_system {
            return cs;
        }
        if self.x.abs() > Self::ECEF_THRESHOLD_M
            || self.y.abs() > Self::ECEF_THRESHOLD_M
            || self.z.abs() > Self::ECEF_THRESHOLD_M
        {
            CoordinateSystem::ECEF
        } else {
            CoordinateSystem::Geodetic
        }
    }
}
```

`CoordinateSystem` needs `Serialize, Deserialize, Clone, Copy` with `#[serde(rename_all = "lowercase")]`. Update `Position3D::new` to set `coordinate_system: None` and fix every struct-literal construction site (`grep -rn "Position3D {" antenna-model/`).

- [ ] **Step 4: Ambiguity warning** in `service/validator.rs` — for each request position, add:

```rust
fn warn_if_ambiguous(pos: &Position3D, name: &str, warnings: &mut Vec<String>) {
    if pos.coordinate_system.is_none() {
        match pos.coordinate_system() {
            CoordinateSystem::Geodetic if pos.z.abs() > 100_000.0 => warnings.push(format!(
                "{name}: auto-detected as geodetic with altitude {:.0} km; \
                 set coordinate_system explicitly to avoid ECEF misclassification",
                pos.z / 1000.0
            )),
            _ => {}
        }
    }
}
```

Call it for `vehicle_position`, `reflector_boresight`, `feed_position`, `emitter_position` wherever the validator builds its warning list.

- [ ] **Step 5: Update boundary tests** — schemas.rs:1076-1089 and coordinates_3d.rs:631-639 boundary values change from 999_999/1_000_001 to 6_399_000/6_401_000. Update the doc comment at coordinates_3d.rs:76 (it already says 6400 km — now true). Update `openapi.yaml` Position3D schema with the optional `coordinate_system` enum field.

- [ ] **Step 6: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 7: Commit**

```bash
git add antenna-model/src/api/schemas.rs antenna-model/src/model/coordinates_3d.rs antenna-model/src/service/validator.rs openapi.yaml
git commit -m "fix: raise ECEF auto-detect threshold to 6400km and allow explicit coordinate_system"
```

---

## Task 4: Normalize azimuth and fix coverage gating (F5a, F6 — critical)

**Goal:** One azimuth convention ([0, 360)) at the service boundary, and `is_in_coverage(None) == true` so correction surfaces are not silently dropped.

**Files:**
- Modify: `antenna-model/src/model/coordinates_3d.rs` (`compute_emitter_direction` return, add `normalize_azimuth_deg`)
- Modify: `antenna-model/src/service/evaluator.rs:370-387` (`is_in_coverage`) and its tests (evaluator.rs:587-590)

**Acceptance Criteria:**
- [ ] `compute_emitter_direction` returns azimuth in [0, 360)
- [ ] `is_in_coverage(&None, ..) == true` (doc and code agree); correction application still requires `correction_surface.is_some()`
- [ ] Existing evaluator tests updated; coverage check passes for az = 270° on a [0, 360] range that previously failed at −90°

**Verify:** `cargo test -p antenna-model is_in_coverage && cargo test -p antenna-model emitter_direction` → PASS

**Steps:**

- [ ] **Step 1: Failing tests**

In coordinates_3d.rs:

```rust
#[test]
fn test_emitter_azimuth_normalized_to_0_360() {
    // Geometry that previously produced a negative atan2 azimuth
    let vehicle = Position3D::new(0.0, 0.0, 42_000_000.0);
    let boresight = Position3D::new(10.0, 15.0, 0.0);
    let emitter = Position3D::new(9.0, 14.0, 100.0); // off-axis, negative quadrant
    let (az, _el) = compute_emitter_direction(&emitter, &vehicle, &boresight).unwrap();
    assert!((0.0..360.0).contains(&az), "az={az}");
}
```

In evaluator.rs, replace `test_is_in_coverage_none`:

```rust
#[test]
fn test_is_in_coverage_none_means_unrestricted() {
    // No coverage restriction (fully calibrated artifact) → always in coverage.
    assert!(is_in_coverage(&None, 180.0, 45.0, 8400.0));
}
```

- [ ] **Step 2: Run** — both new tests FAIL with current code.

- [ ] **Step 3: Implement** in coordinates_3d.rs:

```rust
/// Normalize an azimuth in degrees to [0, 360).
pub fn normalize_azimuth_deg(az_deg: f64) -> f64 {
    let a = az_deg % 360.0;
    if a < 0.0 { a + 360.0 } else { a }
}
```

At the end of `compute_emitter_direction` (coordinates_3d.rs:414-417):

```rust
    let (azimuth_deg, elevation_deg, _range) =
        antenna_frame_to_spherical(antenna_x, antenna_y, antenna_z)?;
    Ok((normalize_azimuth_deg(azimuth_deg), elevation_deg))
```

In evaluator.rs `is_in_coverage` change the `None` arm and the doc comment:

```rust
        // No coverage restriction recorded (fully calibrated artifact):
        // the correction surface applies everywhere it has data.
        None => true,
```

- [ ] **Step 4: Audit knock-on call sites** — `compute_feed_position_from_pointing` subtracts azimuths (`delta_az = feed_az - refl_az`, coordinates_3d.rs:524); with refl_az = 0 by construction this is unaffected, but add a comment noting azimuths are now [0, 360). The H3 path and evaluator consume the normalized value automatically. Check evaluator.rs:119 (`feed_offset_az = feed_az - refl_az`) — same reasoning, leave as-is.

- [ ] **Step 5: Run** — `cargo test -p antenna-model` → PASS (update `test_generate_warnings_*` expectations if a previously-skipped correction now applies)

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/model/coordinates_3d.rs antenna-model/src/service/evaluator.rs
git commit -m "fix: normalize azimuth to [0,360) and treat absent coverage as unrestricted"
```

---

## Task 5: Harden the artifact loader — header, CRC, spline validation (F2 service side + knot-panic medium)

**Goal:** The service loader accepts both legacy headerless artifacts and `"ANTC"`-headered artifacts with CRC verification, and validates B-spline knot vectors at load so malformed data errors instead of panicking.

**Files:**
- Modify: `antenna-model/src/data/loader.rs:31-103`
- Modify: `antenna-model/src/data/types.rs` (add `BSplineModel4D::validate`, call from `AntennaCalibration::validate`)
- Modify: `antenna-model/Cargo.toml` (add `crc32fast = "1"`)

**Acceptance Criteria:**
- [ ] Loader detects `b"ANTC"` magic, parses `[version u32 LE][crc u32 LE][len u64 LE][payload]`, verifies CRC32 of payload, then decodes payload; falls back to whole-file decode otherwise
- [ ] Corrupted payload (bad CRC) → `DataError::LoadError`, not a successful load
- [ ] `BSplineModel4D::validate` rejects: `spline_order < 1`, any knot vector with `len < 2 * spline_order`, non-monotonic knots, `coefficients.len() != shape.iter().product()`
- [ ] `find_knot_span` can no longer be reached with a knot vector shorter than `2*order` via loaded data

**Verify:** `cargo test -p antenna-model loader && cargo test -p antenna-model bspline` → PASS

**Steps:**

- [ ] **Step 1: Failing tests** in loader.rs test module:

```rust
#[test]
fn test_load_antc_headered_artifact() {
    let calibration = make_test_calibration(); // reuse existing test helper in this module
    let payload = bincode::encode_to_vec(&calibration, bincode::config::standard()).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"ANTC");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&payload);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("antc.bin");
    std::fs::write(&path, &bytes).unwrap();
    let loaded = load_calibration_artifact(&path).unwrap();
    assert_eq!(loaded.antenna_id, calibration.antenna_id);
}

#[test]
fn test_load_antc_bad_crc_rejected() {
    // same as above but flip one payload byte after computing the CRC
    // → expect Err(DataError::LoadError)
}
```

And in types.rs:

```rust
#[test]
fn test_bspline_validate_rejects_short_knots() {
    let model = BSplineModel4D {
        coefficients: vec![0.0; 8],
        shape: [2, 2, 2, 1],
        knots_azimuth: vec![0.0, 360.0], // too short for order 3
        knots_elevation: vec![0.0, 0.0, 0.0, 90.0, 90.0, 90.0],
        knots_frequency: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        knots_temperature: vec![290.0, 290.0, 290.0, 290.0, 290.0, 290.0],
        spline_order: 3,
    };
    assert!(model.validate().is_err());
}
```

- [ ] **Step 2: Run** — FAIL (no header support / no `validate`).

- [ ] **Step 3: Implement loader header handling** (loader.rs:36-48):

```rust
    let bytes = std::fs::read(path).map_err(|e| DataError::LoadError { /* unchanged */ })?;

    let payload: &[u8] = if bytes.len() >= 20 && &bytes[0..4] == b"ANTC" {
        let crc = u32::from_le_bytes(bytes[8..12].try_into().expect("sliced"));
        let len = u64::from_le_bytes(bytes[12..20].try_into().expect("sliced")) as usize;
        let payload = bytes.get(20..20 + len).ok_or_else(|| DataError::LoadError {
            path: path.display().to_string(),
            reason: "ANTC header length exceeds file size".to_string(),
        })?;
        if crc32fast::hash(payload) != crc {
            return Err(DataError::LoadError {
                path: path.display().to_string(),
                reason: "CRC32 mismatch — artifact corrupted".to_string(),
            });
        }
        payload
    } else {
        &bytes
    };

    let (calibration, _): (AntennaCalibration, usize) =
        bincode::decode_from_slice(payload, config).map_err(|e| DataError::LoadError { /* unchanged */ })?;
```

(Note: `expect` on `try_into` of a checked 4/8-byte slice is statically safe; if project policy forbids it entirely, use `map_err` into `DataError::LoadError`.)

- [ ] **Step 4: Implement `BSplineModel4D::validate`** in types.rs and call it from `AntennaCalibration::validate` when `correction_surface.is_some()`:

```rust
impl BSplineModel4D {
    pub fn validate(&self) -> Result<(), String> {
        let order = self.spline_order as usize;
        if order == 0 {
            return Err("spline_order must be >= 1".into());
        }
        let expected: usize = self.shape.iter().product();
        if self.coefficients.len() != expected {
            return Err(format!(
                "coefficient count {} != shape product {}",
                self.coefficients.len(), expected
            ));
        }
        for (name, knots) in [
            ("azimuth", &self.knots_azimuth),
            ("elevation", &self.knots_elevation),
            ("frequency", &self.knots_frequency),
            ("temperature", &self.knots_temperature),
        ] {
            if knots.len() < 2 * order {
                return Err(format!(
                    "{name} knot vector has {} entries; need >= {} for order {}",
                    knots.len(), 2 * order, order
                ));
            }
            if knots.windows(2).any(|w| w[1] < w[0]) {
                return Err(format!("{name} knot vector is not non-decreasing"));
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Run** — `cargo test -p antenna-model` → PASS. Fix any existing tests that construct dummy 2-knot `BSplineModel4D` values *and* run them through `validate` (the evaluator tests at evaluator.rs:647-655 only attach them without validating — leave those, but confirm).

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/data/loader.rs antenna-model/src/data/types.rs antenna-model/Cargo.toml Cargo.lock
git commit -m "fix: accept ANTC-headered artifacts with CRC verification; validate B-spline knots at load"
```

---

## Task 6: Make `full` calibration mode emit the service artifact format (F2 tool side — critical)

**Goal:** `calibrate --mode full` writes an `AntennaCalibration` (with 4D `BSplineModel4D` and `calibration_coverage`) that the service loads, instead of the incompatible serde `CalibrationArtifact`.

**Files:**
- Create: `calibrate/src/artifact_export.rs` (3D→4D surface conversion + AntennaCalibration assembly)
- Modify: `calibrate/src/main.rs:600-660` (full-mode output path), `calibrate/src/lib.rs` / `mod.rs` (export module)
- Keep: `calibrate/src/serializer.rs` (legacy format still used by old tooling — mark deprecated in module doc)

**Acceptance Criteria:**
- [ ] Full-mode output decodes via the service's `load_calibration_artifact`
- [ ] Correction values agree: evaluating the exported 4D surface at (az = e_clock, el = e_cone, freq, T₀) matches the calibrate tool's own 3D surface evaluation within 1e-9 at 20 sample points
- [ ] `calibration_coverage` is populated from measurement extents (az/el/freq min–max, num_measurements, has_correction_surface = true)
- [ ] `calibration_status` set to `FullyCalibrated { accuracy_estimate_db: <validation RMSE> }`

**Verify:** `cargo test -p calibrate artifact_export` (and the round-trip integration test below) → PASS

**Steps:**

- [ ] **Step 1: Confirm the calibrate tool's flat coefficient layout.** Open `calibrate/src/correction_surface.rs` and read the evaluation function around line 713 (`let [n_freq, n_cone, _n_clock] = self.shape;`). Record the exact flat-index formula it uses (the code below assumes `idx = clock + n_clock*(cone + n_cone*freq)`; if it differs, adjust `src_idx` accordingly — the round-trip test in Step 3 will catch any mismatch).

- [ ] **Step 2: Write the conversion + failing round-trip test** in `calibrate/src/artifact_export.rs`:

```rust
//! Export full-mode calibration results in the service's AntennaCalibration format.

use antenna_model::data::types::{
    AntennaCalibration, BSplineModel4D, CalibrationCoverage, CalibrationStatus,
};
use crate::correction_surface::CorrectionSurface;

/// Convert the calibrate tool's 3D (freq, e_cone, e_clock) surface into the
/// service's 4D (azimuth, elevation, frequency, temperature) model.
/// Mapping: azimuth := e_clock, elevation := e_cone (both polar-angle degree
/// conventions already match the service); temperature is a degenerate axis
/// pinned at `temperature_k`.
pub fn to_bspline_4d(surface: &CorrectionSurface, temperature_k: f64) -> BSplineModel4D {
    let [n_freq, n_cone, n_clock] = surface.shape;
    let order = surface.spline_order as usize;

    // Service layout (correction_interpolator.rs): az fastest →
    //   dst = az + n_az*(el + n_el*(freq + n_freq*temp)),  n_temp = 1
    let mut coefficients = vec![0.0; n_freq * n_cone * n_clock];
    for f in 0..n_freq {
        for c in 0..n_cone {
            for k in 0..n_clock {
                let src = k + n_clock * (c + n_cone * f); // verify in Step 1
                let dst = k + n_clock * (c + n_cone * f); // az=k, el=c, freq=f, temp=0
                coefficients[dst] = surface.coefficients[src];
            }
        }
    }

    // Degenerate temperature axis: a valid clamped knot vector for the order,
    // single coefficient slice (n_temp = 1).
    let temp_knots = vec![temperature_k; 2 * order];

    BSplineModel4D {
        coefficients,
        shape: [n_clock, n_cone, n_freq, 1],
        knots_azimuth: surface.knots_eclock.clone(),
        knots_elevation: surface.knots_econe.clone(),
        knots_frequency: surface.knots_frequency.clone(),
        knots_temperature: temp_knots,
        spline_order: surface.spline_order,
    }
}
```

Round-trip test (same file, `#[cfg(test)]`):

```rust
#[test]
fn test_3d_to_4d_round_trip_evaluation() {
    let surface = make_test_surface(); // fit a small surface from synthetic residuals
    let model4d = to_bspline_4d(&surface, 290.0);
    model4d.validate().expect("exported surface must validate");
    for &(freq, cone, clock) in SAMPLE_POINTS {
        let expected = surface.evaluate(freq, cone, clock).unwrap();
        let got = antenna_model::model::evaluate_correction(&model4d, clock, cone, freq, 290.0)
            .unwrap()
            .correction_db;
        assert!((got - expected).abs() < 1e-9, "mismatch at ({freq},{cone},{clock}): {got} vs {expected}");
    }
}
```

`SAMPLE_POINTS`: 20 (freq, cone, clock) tuples spanning the fitted ranges including boundaries. `make_test_surface` calls the existing `fit_correction_surface` with ~50 synthetic residual points (e.g. `residual = 0.1·sin(clock°/57.3) + 0.01·freq_offset`).

- [ ] **Step 3: Run** — `cargo test -p calibrate artifact_export` → likely FAIL on the round-trip until the `src` index formula from Step 1 is correct. Iterate until it passes; the equality test is the proof of correct reordering.

- [ ] **Step 4: Assemble and write the full `AntennaCalibration`** — add `pub fn export_full_calibration(...) -> AntennaCalibration` in `artifact_export.rs` that fills `antenna_id`, `feed_id`, metadata (date, format_version "2.0", rmse/r² from the validation report), `physical_config` from the tuned parameters, `correction_surface: Some(to_bspline_4d(..))`, `validity_ranges` and `calibration_coverage` from measurement min/max:

```rust
let coverage = CalibrationCoverage::builder()
    .azimuth_range(az_min, az_max)
    .elevation_range(el_min, el_max)
    .frequency_range(freq_min, freq_max)
    .num_measurements(measurements.points.len())
    .has_correction_surface(true)
    .build()?;
```

Then in `main.rs` full mode (replacing the `save_artifact` call at main.rs:648), serialize exactly like boresight mode does at main.rs:317 (`bincode::encode_to_vec(&calibration, bincode::config::standard())`), optionally prepending the ANTC header (the loader from Task 5 accepts both — prepend it, with `crc32fast::hash`, for integrity).

- [ ] **Step 5: Integration test** — `calibrate/tests/` (or extend an existing integration test): run full calibration on a small synthetic CSV, write the artifact to a temp dir, load it with `antenna_model::data::loader::load_calibration_artifact`, assert antenna_id/feed_id/correction-surface shape round-trip.

- [ ] **Step 6: Run everything** — `cargo test -p calibrate && cargo test -p antenna-model` → PASS

- [ ] **Step 7: Commit**

```bash
git add calibrate/src/artifact_export.rs calibrate/src/main.rs calibrate/src/lib.rs calibrate/src/serializer.rs calibrate/tests/
git commit -m "fix: full calibration mode emits service-compatible AntennaCalibration artifacts"
```

---

## Task 7: Small coordinate fixes — to_degrees, pole altitude, stale comment (F10, F12 + doc)

**Goal:** Fix the radians/degrees unit bug, the pole singularity in `ecef_to_geodetic`, and the wrong `MAX_ALTITUDE_M` comment.

**Files:**
- Modify: `antenna-model/src/model/coordinates.rs:136-139`
- Modify: `antenna-model/src/model/coordinates_3d.rs:62-63, 232-239`

**Acceptance Criteria:**
- [ ] `EClockConeCoordinates::to_degrees()` returns degrees
- [ ] `ecef_to_geodetic` round-trips (lat ±90°, alt 1000 m) to <1 mm altitude error
- [ ] `MAX_ALTITUDE_M` comment says 400,000 km

**Verify:** `cargo test -p antenna-model to_degrees && cargo test -p antenna-model geodetic` → PASS

**Steps:**

- [ ] **Step 1: Failing tests**

```rust
// coordinates.rs
#[test]
fn test_to_degrees_returns_degrees() {
    let ecc = EClockConeCoordinates::new(PI / 2.0, PI);
    let (cone_deg, clock_deg) = ecc.to_degrees();
    assert!((cone_deg - 90.0).abs() < EPSILON);
    assert!((clock_deg - 180.0).abs() < EPSILON);
}

// coordinates_3d.rs
#[test]
fn test_ecef_to_geodetic_pole_with_altitude() {
    for lat in [90.0, -90.0] {
        let (x, y, z) = geodetic_to_ecef(0.0, lat, 1000.0).unwrap();
        let (_lon, lat2, alt2) = ecef_to_geodetic(x, y, z).unwrap();
        assert!((lat2 - lat).abs() < 1e-9, "lat {lat}: got {lat2}");
        assert!((alt2 - 1000.0).abs() < 1e-3, "lat {lat}: alt {alt2}");
    }
}
```

- [ ] **Step 2: Run** — `to_degrees` test FAILS; pole test may pass marginally but is ill-conditioned — proceed regardless.

- [ ] **Step 3: Fix** coordinates.rs:137-139:

```rust
    pub fn to_degrees(&self) -> (f64, f64) {
        (self.e_cone.to_degrees(), self.e_clock.to_degrees())
    }
```

Fix coordinates_3d.rs:234-239 (altitude near poles):

```rust
    let sin_lat = lat_rad.sin();
    let cos_lat = lat_rad.cos();
    let n = WGS84_A / (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
    // alt = p/cos(lat) − N is 0/0 at the poles; use the z-based form there.
    let alt_m = if cos_lat.abs() > 1e-4 {
        p / cos_lat - n
    } else {
        z / sin_lat - n * (1.0 - WGS84_E2)
    };
```

Fix the comment at coordinates_3d.rs:62: `/// Maximum reasonable altitude for coordinate validation (400,000 km, allows HEO satellites)`.

- [ ] **Step 4: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/model/coordinates.rs antenna-model/src/model/coordinates_3d.rs
git commit -m "fix: to_degrees unit bug, pole-safe geodetic altitude, MAX_ALTITUDE comment"
```

---

## Task 8: Apply correction surface in H3 link budget; truthful correction_applied (F8 + H3 loss labeling)

**Goal:** H3 cell gains include the calibration correction (matching the `/gain` endpoint), and `correction_applied` reflects reality.

**Files:**
- Modify: `antenna-model/src/service/h3_link_budget.rs:128-187` (compute_cell_gain), `:341-353` (status), `:373-445` (compute_cell_result)
- Modify: `antenna-model/src/api/schemas.rs` (document `loss_db` semantics on `H3CellResult`)

**Acceptance Criteria:**
- [ ] Each cell's gain = physics gain + `evaluate_correction(...)` when a correction surface exists and the cell is in coverage (same gating as `evaluator.rs:277-299`)
- [ ] `correction_applied` is true only if the correction was actually applied to at least one cell
- [ ] Extrapolation warnings from the correction surface surface in the response warnings
- [ ] `H3CellResult::loss_db` doc states it is relative to the grid-center cell, not the true beam peak

**Verify:** `cargo test -p antenna-model h3` → PASS

**Steps:**

- [ ] **Step 1: Failing test** in h3_link_budget.rs — build a calibration with a constant-offset correction surface (all coefficients = 2.0 dB, valid clamped knots covering az 0–360, el 0–90, the request frequency, T=290) and assert every returned cell gain is 2.0 dB above the same request run with `correction_surface = None`:

```rust
#[test]
fn test_h3_applies_correction_surface() {
    let (req, cal_no_corr, cache) = make_h3_fixture();          // existing-style fixture
    let mut cal_corr = cal_no_corr.clone();
    cal_corr.correction_surface = Some(constant_surface_db(2.0)); // helper: valid 4D surface, all coeffs 2.0
    cal_corr.calibration_coverage = None;                          // unrestricted (Task 4 semantics)

    let base = compute_h3_link_budget(&req, &cal_no_corr, &cache, std::time::Instant::now()).unwrap();
    let cache2 = GainCache::new(false, 1); // disable cache so gains recompute
    let corr = compute_h3_link_budget(&req, &cal_corr, &cache2, std::time::Instant::now()).unwrap();

    for (a, b) in base.cells.iter().zip(corr.cells.iter()) {
        assert!((b.gain_db - a.gain_db - 2.0).abs() < 1e-6, "cell {}: {} vs {}", a.cell_id, a.gain_db, b.gain_db);
    }
    assert!(corr.calibration_status.unwrap().correction_applied);
}
```

`constant_surface_db(v)`: order-1 spline (`spline_order: 1`), `shape: [1,1,1,1]`, `coefficients: vec![v]`, each knot vector `vec![min, max]` spanning the domain — or order-3 with repeated clamped knots; whichever passes `BSplineModel4D::validate` from Task 5.

- [ ] **Step 2: Run** — FAIL (gains identical, correction never applied).

- [ ] **Step 3: Implement** — in `compute_cell_gain`, after the cached physics gain, mirror the evaluator's gating:

```rust
    let mut correction_applied = false;
    let mut gain_db = gain_db; // physics value from cache
    if let Some(ref surface) = calibration.correction_surface {
        if crate::service::evaluator::is_in_coverage(
            &calibration.calibration_coverage, az_deg, el_deg, request.frequency_mhz,
        ) {
            let corr = crate::model::evaluate_correction(surface, az_deg, el_deg, request.frequency_mhz, 290.0)?;
            gain_db += corr.correction_db;
            captured_warnings.extend(corr.warnings);
            correction_applied = true;
        }
    }
```

Thread `calibration: &AntennaCalibration` into `compute_cell_gain`/`compute_cell_result` (it is already available in `compute_h3_link_budget`), return `correction_applied` up, aggregate with a fold (`any_correction_applied`), and replace h3_link_budget.rs:351 with `info.correction_applied = any_correction_applied;`. **Important:** the gain cache stores physics-only values (key has no correction dimension) — apply the correction *after* the cache lookup, as above, never inside the cached closure.

- [ ] **Step 4: Document loss semantics** — on `H3CellResult.loss_db` in schemas.rs: `/// Gain relative to the grid-center cell (feed ground target), in dB. Not referenced to the true beam peak.` Mirror in `openapi.yaml`.

- [ ] **Step 5: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/service/h3_link_budget.rs antenna-model/src/api/schemas.rs openapi.yaml
git commit -m "fix: apply correction surface in H3 link budget and report correction_applied truthfully"
```

---

## Task 9: Absolute gain from the aperture integral; consistent reference gain (F7)

**Goal:** Derive absolute gain from the physical-optics integral (directivity formula with taper efficiency built in) instead of `theoretical_max_gain × 0.55 × relative`, and make `reference_gain_db` use the identical formula so `loss_db` has no built-in offset.

**Files:**
- Modify: `antenna-model/src/model/integration.rs` (add `integrate_amplitude_squared`)
- Modify: `antenna-model/src/model/pattern.rs:301-531` (all four `compute_gain_*` normalizations)
- Modify: `antenna-model/src/service/evaluator.rs:309-322` (reference gain)

**Acceptance Criteria:**
- [ ] `gain(θ,φ) = η_ruze · η_mesh · (4π/λ²) · |∬ A e^{jΨ} dA|² / ∬ |A|² dA` (standard aperture directivity with amplitude-taper efficiency; spillover still unmodeled — document that)
- [ ] No hardcoded 0.55 anywhere in the gain path (`grep -rn "0.55" antenna-model/src/model/pattern.rs` → only comments/tests)
- [ ] Boresight gain for the 1 m / 8.4 GHz / q=8 test antenna lands in 34–38.5 dBi (taper efficiency 0.5–0.9 of the 39.0 dBi uniform-aperture maximum)
- [ ] `reference_gain_db` = same formula evaluated for the ideal config at boresight → `loss_db ≈ 0` for an on-boresight, focused-feed request without correction
- [ ] The per-call ideal-reference integration in `compute_gain_standard` (pattern.rs:319-353) is gone (only computed when `include_reference` is requested)

**Verify:** `cargo test -p antenna-model compute_gain && cargo test -p antenna-model reference` → PASS

**Steps:**

- [ ] **Step 1: Failing tests**

```rust
// pattern.rs
#[test]
fn test_boresight_gain_reflects_taper_efficiency() {
    let config = test_antenna_no_mesh(); // like test_antenna() but mesh: None, surface_rms 0.0
    let params = IntegrationParams::default();
    let result = compute_gain_db(0.0, 0.0, &config, 8.4e9, &params).unwrap();
    let uniform_db = 10.0 * (4.0 * PI * PI * 0.25 / (0.0357_f64 * 0.0357)).log10(); // ≈ 39.0 dBi
    assert!(result.gain < uniform_db, "taper must cost gain: {} vs {uniform_db}", result.gain);
    assert!(result.gain > uniform_db - 5.0, "taper loss implausibly large: {}", result.gain);
}

// evaluator.rs
#[test]
fn test_loss_near_zero_for_boresight_focused_feed() {
    // create_test_request() aims feed at the boresight target (focused feed);
    // emitter moved to lie along the boresight direction.
    let mut request = create_test_request();
    request.emitter_position = request.reflector_boresight.clone();
    request.include_reference = true;
    let response = compute_gain_from_request(&request, &repo_with_fully_calibrated()).unwrap();
    let loss = response.loss_db.unwrap();
    assert!(loss.abs() < 0.5, "boresight loss should be ~0 dB, got {loss}");
}
```

- [ ] **Step 2: Run** — FAIL (current loss carries the ~2.6 dB 0.55 offset; gain pinned to 0.55).

- [ ] **Step 3: Add the amplitude-squared integral** in integration.rs (same Simpson grid machinery, real-valued):

```rust
/// ∬ |A(ρ,φ′)|² ρ dρ dφ′ over the aperture — denominator of the
/// aperture-directivity formula. Uses the same illumination model as
/// `aperture_integrand` but no phase factor.
pub fn integrate_amplitude_squared(
    config: &AntennaConfiguration,
    n_rho: usize,
    n_phi: usize,
) -> f64 {
    let rho_max = config.reflector.diameter / 2.0;
    let n_rho = if n_rho % 2 == 0 { n_rho + 1 } else { n_rho };
    let n_phi = if n_phi % 2 == 0 { n_phi + 1 } else { n_phi };
    let h_rho = rho_max / (n_rho - 1) as f64;
    let h_phi = 2.0 * PI / (n_phi - 1) as f64;
    let mut sum = 0.0;
    for j in 0..n_phi {
        let phi_prime = j as f64 * h_phi;
        let wj = simpson_weight(j, n_phi);
        let mut inner = 0.0;
        for i in 0..n_rho {
            let rho = i as f64 * h_rho;
            let a = illumination_amplitude(rho, phi_prime, &config.feed, config.reflector.focal_length);
            inner += a * a * rho * simpson_weight(i, n_rho);
        }
        sum += inner * wj;
    }
    sum * h_rho * h_phi / 9.0
}
```

- [ ] **Step 4: Rewrite the gain normalization.** Add one shared helper in pattern.rs and use it from all four `compute_gain_*` functions:

```rust
/// Absolute gain from the raw aperture integral (directivity formula).
/// gain = η · (4π/λ²) · |I|² / ∬|A|²dA, where I = ∬ A e^{jΨ} dA.
fn absolute_gain_from_integral(
    raw_field: num_complex::Complex64, // integrate_aperture(...).field — NOT compute_far_field
    config: &AntennaConfiguration,
    wavelength: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    let amp_sq = integrate_amplitude_squared(config, params.min_rho_points, params.min_phi_points);
    if amp_sq <= 1e-20 {
        return Err(ComputationError::NumericalInstability {
            operation: "absolute_gain_from_integral".to_string(),
            reason: "amplitude integral is zero".to_string(),
        });
    }
    let directivity = 4.0 * PI / (wavelength * wavelength) * raw_field.norm_sqr() / amp_sq;
    Ok(directivity * overall_efficiency(config, wavelength))
}
```

In `compute_gain_standard` (and `_higher_order`, `_direct_path`): call `integrate_aperture` directly (not `compute_far_field` — the constant `jπ/λ²` normalization must not be squared into the directivity), pass `result.field` to the helper, and **delete** the ideal-config construction + ideal on-axis integration + `theoretical_max_gain(…, 0.55)` blocks (pattern.rs:319-364, 395-437, 493-530). For `compute_gain_ray_tracing`, keep its existing relative normalization but scale by `absolute_gain_from_integral` of the boresight PO field — and leave its stub warning untouched. Add a module-level doc note: *spillover efficiency is not modeled; calibration absorbs it.*

- [ ] **Step 5: Fix the reference gain** in evaluator.rs:310-316 — compute the ideal-config boresight gain through the same pipeline:

```rust
    let (reference_gain_db, loss_db) = if request.include_reference {
        let ideal_config = build_ideal_config(&antenna_config)?; // feed at focus, surface_rms 0, same mesh — extract the helper from old pattern.rs code
        let reference = compute_gain_db(0.0, 0.0, &ideal_config, frequency_hz, &integration_params)?;
        (Some(reference.gain), Some(reference.gain - final_gain_db))
    } else {
        (None, None)
    };
```

- [ ] **Step 6: Update value-asserting tests** — `test_compute_gain_positive`, `test_compute_gain_db_reasonable`, `test_compute_g_over_t` ranges still hold (boresight ~34–38 dBi); update if needed with a comment deriving the expected range. Run `cargo test -p antenna-model && cargo test -p calibrate` (the tuner consumes `compute_g_over_t` — its synthetic tests may need re-baselining).

- [ ] **Step 7: Commit**

```bash
git add antenna-model/src/model/integration.rs antenna-model/src/model/pattern.rs antenna-model/src/service/evaluator.rs
git commit -m "fix: derive absolute gain from aperture integral; remove hardcoded 0.55 and reference-gain offset"
```

---

## Task 10: Beam squint along the feed displacement direction (F9)

**Goal:** Apply beam squint along the feed-displacement clock angle in direction-cosine space, instead of always shifting elevation.

**Files:**
- Modify: `antenna-model/src/model/coordinates_3d.rs:573-606` (signature + math), tests `:778-857`
- Modify: `antenna-model/src/service/evaluator.rs:188-200` (pass clock angle)

**Acceptance Criteria:**
- [ ] `apply_beam_squint_correction` takes the feed clock angle `feed_clock_rad` and shifts the pointing in that direction
- [ ] Result elevation (polar angle) is always ≥ 0; azimuth normalized to [0, 360)
- [ ] Feed displaced along +X (clock 0) with a frequency offset shifts the beam in the φ=0/180 plane, not in whatever plane the emitter happens to lie

**Verify:** `cargo test -p antenna-model beam_squint` → PASS

**Steps:**

- [ ] **Step 1: Failing test**

```rust
#[test]
fn test_beam_squint_applied_along_feed_clock_angle() {
    // Feed displaced along +Y (clock = 90°). Squint must move the beam in the
    // v (sinθ·sinφ) direction, leaving u (sinθ·cosφ) unchanged.
    let (az, el, squint) = apply_beam_squint_correction(
        0.0, 2.0,            // pointing: az=0°, el=2° polar
        8400.0, 8800.0,      // freq offset
        1.0, 13.6,           // displacement, focal length
        std::f64::consts::FRAC_PI_2, // feed clock angle = +Y
    );
    assert!(squint > 0.0);
    let theta = el.to_radians();
    let phi = az.to_radians();
    let u = theta.sin() * phi.cos();
    // original u = sin(2°)·cos(0) ≈ 0.0349 — must be unchanged by a +Y squint
    assert!((u - 2.0_f64.to_radians().sin()).abs() < 1e-6, "u changed: {u}");
    assert!(el >= 0.0);
}
```

- [ ] **Step 2: Run** — FAIL (signature has no clock angle).

- [ ] **Step 3: Implement** — work in direction cosines (u = sinθ·cosφ, v = sinθ·sinφ):

```rust
pub fn apply_beam_squint_correction(
    azimuth_deg: f64,
    elevation_deg: f64, // polar angle from boresight
    pointing_freq_mhz: f64,
    operating_freq_mhz: f64,
    feed_displacement_m: f64,
    focal_length_m: f64,
    feed_clock_rad: f64, // atan2(feed_y, feed_x) in the antenna frame
) -> (f64, f64, f64) {
    if (pointing_freq_mhz - operating_freq_mhz).abs() / pointing_freq_mhz < 0.001
        || feed_displacement_m < 1e-6
    {
        return (azimuth_deg, elevation_deg, 0.0);
    }
    let freq_shift_ratio = (operating_freq_mhz - pointing_freq_mhz) / pointing_freq_mhz;
    let squint_rad = freq_shift_ratio * (feed_displacement_m / focal_length_m);

    let theta = elevation_deg.to_radians();
    let phi = azimuth_deg.to_radians();
    // Squint shifts the beam along the feed-displacement direction in (u, v).
    let u = theta.sin() * phi.cos() + squint_rad * feed_clock_rad.cos();
    let v = theta.sin() * phi.sin() + squint_rad * feed_clock_rad.sin();
    let s = (u * u + v * v).sqrt().min(1.0);
    let new_theta = s.asin();
    let new_phi = v.atan2(u);

    (
        normalize_azimuth_deg(new_phi.to_degrees()),
        new_theta.to_degrees(),
        squint_rad.to_degrees().abs(),
    )
}
```

- [ ] **Step 4: Update the call site** (evaluator.rs:188-200): pass `feed_y.atan2(feed_x)` as `feed_clock_rad`. Update the three existing beam-squint tests for the new signature (they pass clock 0.0; the "elevation shifted" assertion in `test_beam_squint_correction_applied` becomes a u-shift assertion).

- [ ] **Step 5: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/model/coordinates_3d.rs antenna-model/src/service/evaluator.rs
git commit -m "fix: apply beam squint along feed displacement clock angle in direction-cosine space"
```

---

## Task 11: Model axial feed defocus in the phase (F11)

**Goal:** `phase_feed_displacement` accounts for the feed's axial offset from the focal point instead of assuming z = f.

**Files:**
- Modify: `antenna-model/src/model/phase.rs:141-173, 249-293` (add `delta_z` parameter to `phase_feed_displacement` and `phase_total`), tests `:531-698`
- Modify: `antenna-model/src/model/integration.rs:460-524` (pass `config.feed.position.z - focal_length`)

**Acceptance Criteria:**
- [ ] Feed at `(dx, dy, f + δz)`; displaced path uses `dz_feed = z_surface − (f + δz)`
- [ ] Pure axial displacement (δ_lateral = 0, δz ≠ 0) produces a ρ-dependent (defocus) phase — currently it produces exactly zero
- [ ] `δz = 0` reproduces today's values bit-for-bit (regression guard)

**Verify:** `cargo test -p antenna-model phase_feed_displacement` → PASS

**Steps:**

- [ ] **Step 1: Failing test**

```rust
#[test]
fn test_axial_displacement_produces_defocus() {
    let (f, k) = (17.0, wavenumber(0.03));
    // Pure axial offset: 10 cm behind focus.
    let p_center = phase_feed_displacement(0.0, 0.0, 0.0, 0.0, 0.10, f, k);
    let p_edge = phase_feed_displacement(8.0, 0.0, 0.0, 0.0, 0.10, f, k);
    // Defocus = ρ-dependent phase; edge and center must differ.
    assert!((p_edge - p_center).abs() > 1.0, "no defocus: {p_center} vs {p_edge}");
}

#[test]
fn test_zero_axial_matches_previous_model() {
    let (f, k) = (17.0, wavenumber(0.03));
    let with_z0 = phase_feed_displacement(5.0, PI / 4.0, 1.0, 0.0, 0.0, f, k);
    // Expected value = old 6-arg model output (compute the literal before changing the code)
    let expected = {
        let (x, y) = (5.0 * (PI / 4.0).cos(), 5.0 * (PI / 4.0).sin());
        let z = 25.0 / (4.0 * f);
        let dz = z - f;
        let ideal = (x * x + y * y + dz * dz).sqrt();
        let displaced = ((x - 1.0).powi(2) + y * y + dz * dz).sqrt();
        k * (displaced - ideal)
    };
    assert!((with_z0 - expected).abs() < 1e-10);
}
```

- [ ] **Step 2: Run** — FAIL (arity mismatch).

- [ ] **Step 3: Implement** — new signature `phase_feed_displacement(rho, phi_prime, delta_feed, alpha, delta_z, focal_length, k)`:

```rust
    let dz_ideal = z - focal_length;
    let path_ideal = (x * x + y * y + dz_ideal * dz_ideal).sqrt();
    let dz_displaced = z - (focal_length + delta_z);
    let path_displaced = ((x - dx).powi(2) + (y - dy).powi(2) + dz_displaced * dz_displaced).sqrt();
    k * (path_displaced - path_ideal)
```

Update `phase_total` to take and forward `feed_axial_offset: f64`, and change its guard from `feed_displacement > 0.0` to `feed_displacement > 0.0 || feed_axial_offset.abs() > 0.0`. In `aperture_integrand` (integration.rs:469-472):

```rust
    let feed_displacement = config.feed.position.radial_displacement();
    let feed_displacement_angle = config.feed.position.y.atan2(config.feed.position.x);
    let feed_axial_offset = config.feed.position.z - config.reflector.focal_length;
```

Update every `phase_total`/`phase_feed_displacement` call and test for the new arity (the existing tests pass `0.0` for `delta_z`).

- [ ] **Step 4: Run** — `cargo test -p antenna-model` → PASS. Note in the commit message that steered-feed gains change slightly (the z = f − d²/4f offset from `to_feed_position` now participates), so calibration artifacts need regeneration.

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/model/phase.rs antenna-model/src/model/integration.rs
git commit -m "fix: include axial feed defocus in the feed-displacement phase model"
```

---

## Task 12: Report integration non-convergence; honest error estimate (medium)

**Goal:** Non-converged aperture integrations carry a flag/warning to the caller instead of silently returning the last iterate with a fabricated error estimate.

**Files:**
- Modify: `antenna-model/src/model/integration.rs:39-53, 254-325` (add `converged: bool` to `IntegrationResult`; real error estimate)
- Modify: `antenna-model/src/model/pattern.rs` (propagate a warning from `compute_gain_*` when not converged)

**Acceptance Criteria:**
- [ ] `IntegrationResult.converged` is false when the refinement loop exhausts iterations without meeting tolerance
- [ ] On non-convergence, `error_estimate` = the last inter-iteration difference (not `|result|·tol`)
- [ ] `compute_gain` warnings include `"aperture integration did not converge"` when applicable (exercise with `max_iterations: 1`, `relative_tolerance: 1e-15`)

**Verify:** `cargo test -p antenna-model convergence` → PASS

**Steps:**

- [ ] **Step 1: Failing test** in integration.rs:

```rust
#[test]
fn test_non_convergence_is_reported() {
    let config = test_antenna();
    let params = IntegrationParams {
        max_iterations: 1,            // cannot converge: convergence needs iteration > 0
        relative_tolerance: 1e-15,
        ..IntegrationParams::fast()
    };
    let result = integrate_aperture(0.3, 0.0, &config, 8.4e9, &params).unwrap();
    assert!(!result.converged);
}
```

- [ ] **Step 2: Run** — FAIL (no `converged` field).

- [ ] **Step 3: Implement** — add `pub converged: bool` to `IntegrationResult` (set `true` in the in-loop success return). Track `last_difference: f64` in the loop (the `difference` computed each iteration); after the loop:

```rust
    tracing::warn!(
        theta, phi, last_difference,
        "aperture integration did not converge within {} iterations", params.max_iterations
    );
    Ok(IntegrationResult {
        field: previous_result,
        error_estimate: last_difference,
        num_evaluations,
        converged: false,
    })
```

`compute_far_field` currently returns bare `Complex64`; change it to return `(Complex64, bool)` **or** add `pub fn compute_far_field_full(..) -> ComputationResult<IntegrationResult-with-normalized-field>` and keep the old signature delegating — take the second option to limit churn. In `compute_gain_standard`/`_higher_order`/`_direct_path`, use the full variant and push `"aperture integration did not converge; gain accuracy may be degraded".to_string()` into the returned warnings when `!converged`. (After Task 9 these functions already call `integrate_aperture` directly, so just read `result.converged`.)

- [ ] **Step 4: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/model/integration.rs antenna-model/src/model/pattern.rs
git commit -m "fix: surface aperture-integration non-convergence as a warning with honest error estimate"
```

---

## Task 13: Report the physical feed offset, in meters (F15)

**Goal:** `GeometryInfo.feed_offset_meters` contains the actual feed offset vector in meters, not (Δaz°, Δel°, angle).

**Files:**
- Modify: `antenna-model/src/service/evaluator.rs:104-128, 336-351`
- Modify: `antenna-model/src/api/schemas.rs` (doc comment on `feed_offset_meters`)

**Acceptance Criteria:**
- [ ] `feed_offset_meters = (feed_x, feed_y, feed_z − focal_length)` — physical offset from the focal point in the antenna frame, meters
- [ ] Boresight-aimed feed with zero design offset reports ≈ (0, 0, 0)

**Verify:** `cargo test -p antenna-model feed_offset` → PASS

**Steps:**

- [ ] **Step 1: Failing test** in evaluator.rs:

```rust
#[test]
fn test_feed_offset_reported_in_meters() {
    let mut repo = CalibrationRepository::new();
    repo.add_calibration(create_test_calibration(CalibrationStatus::FullyCalibrated {
        accuracy_estimate_db: 1.0,
    }));
    let mut request = create_test_request();
    request.feed_position = request.reflector_boresight.clone(); // feed aimed at boresight: focused
    let response = compute_gain_from_request(&request, &repo).unwrap();
    let off = &response.geometry.feed_offset_meters;
    assert!(off.x.abs() < 0.01 && off.y.abs() < 0.01 && off.z.abs() < 0.01,
        "expected ~zero offset, got ({}, {}, {})", off.x, off.y, off.z);
}
```

- [ ] **Step 2: Run** — FAIL (current value packs degrees).

- [ ] **Step 3: Implement** — delete the angular `feed_offset` block (evaluator.rs:104-128: the two `compute_emitter_direction` calls and the `Vector3D::new(az, el, mag)`), and after `feed_x/feed_y/feed_z` are computed (evaluator.rs:174-177):

```rust
    let feed_offset = crate::api::schemas::Vector3D::new(
        feed_x,
        feed_y,
        feed_z - focal_length_m,
    );
```

Update the `feed_offset_meters` doc in schemas.rs: `/// Physical feed offset from the focal point in the antenna frame (meters).` Mirror in `openapi.yaml`.

- [ ] **Step 4: Run** — `cargo test -p antenna-model` → PASS

- [ ] **Step 5: Commit**

```bash
git add antenna-model/src/service/evaluator.rs antenna-model/src/api/schemas.rs openapi.yaml
git commit -m "fix: report physical feed offset in meters instead of angular degrees"
```

---

## Task 14: Deduplicate surface-error models; add illumination space attenuation (F13, F14)

**Goal:** One Zernike implementation (the correct Noll one in `surface.rs`), no fake-Gaussian placeholder, and the standard `(1+cosψ)/2` space-attenuation factor in the aperture illumination.

**Files:**
- Modify: `antenna-model/src/model/phase.rs:295-451` (delete `SurfaceErrorModel` trait copy, `IdealSurface`, `GaussianSurface`, `ZernikeSurface` + their tests; `surface.rs` already has the canonical versions)
- Modify: `antenna-model/src/model/illumination.rs:194-219` (space attenuation), `:1-25` (fix edge-taper doc claim)
- Modify: `antenna-model/src/model/mod.rs` (re-exports)

**Acceptance Criteria:**
- [ ] `grep -n "struct ZernikeSurface" antenna-model/src/model/` → only `surface.rs`
- [ ] `illumination_amplitude(ρ_edge, ..)` < the pure `cos^q` value (extra edge taper from space loss); boresight value still 1.0
- [ ] Module header no longer claims "q ≈ 6–8 for 10 dB edge taper" (state the actual ≈ −35 dB amplitude taper for q=8, f/D=0.5, or re-derive)
- [ ] All existing tests pass with re-baselined gain values

**Verify:** `cargo test -p antenna-model illumination && cargo test -p antenna-model surface` → PASS

**Steps:**

- [ ] **Step 1: Failing test** in illumination.rs:

```rust
#[test]
fn test_space_attenuation_adds_edge_taper() {
    let feed = FeedParameters::new(FeedPosition::at_focus(0.5), 8.0, 0.0, 1.0).unwrap();
    let psi_edge = feed_angle(0.5, 0.0, &feed.position, 0.5);
    let pure_cos_q = cos_q_pattern(psi_edge, 8.0);
    let amp = illumination_amplitude(0.5, 0.0, &feed, 0.5);
    assert!(amp < pure_cos_q, "space loss must add taper: {amp} vs {pure_cos_q}");
    // Boresight (vertex) unchanged at 1.0
    let amp0 = illumination_amplitude(0.0, 0.0, &feed, 0.5);
    assert!((amp0 - 1.0).abs() < 1e-9);
}
```

- [ ] **Step 2: Run** — FAIL.

- [ ] **Step 3: Implement** — at the end of `illumination_amplitude`:

```rust
    // Space attenuation: feed→reflector distance r = 2f/(1+cosψ) for a parabola,
    // so the aperture amplitude carries an extra (1+cosψ)/2 factor (normalized
    // to 1 at ψ=0).
    let space_loss = (1.0 + psi.cos()) / 2.0;
    cos_q_pattern(psi, q_effective) * space_loss
```

Fix the module header (illumination.rs:18-21): replace the "q ≈ 6-8 for 10 dB edge taper" lines with the measured behaviour of this model (`edge_taper_db(8.0, 0.5) ≈ −37 dB` amplitude taper including space loss — compute the exact value from the updated function and state it).

- [ ] **Step 4: Delete duplicates from phase.rs** — remove `SurfaceErrorModel`, `IdealSurface`, `GaussianSurface`, `ZernikeSurface` and their tests (phase.rs:295-451, 789-839). `grep -rn "phase::ZernikeSurface\|phase::GaussianSurface\|phase::IdealSurface\|phase::SurfaceErrorModel" antenna-model/ calibrate/` and repoint any callers to `crate::model::surface::*`. Update `mod.rs` re-exports.

- [ ] **Step 5: Run** — `cargo test -p antenna-model` → PASS (re-baseline any gain tests shifted by the added taper; expect boresight gain to drop ~0.3–1 dB). Note in commit: calibration artifacts need regeneration.

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/model/phase.rs antenna-model/src/model/illumination.rs antenna-model/src/model/mod.rs
git commit -m "fix: single Zernike implementation; add (1+cos psi)/2 space attenuation to illumination"
```

---

## Task 15: Robust beamwidth search (medium)

**Goal:** `compute_beamwidth` brackets the first −N dB crossing by outward march before bisecting, so it cannot lock onto a sidelobe.

**Files:**
- Modify: `antenna-model/src/model/pattern.rs:669-712`

**Acceptance Criteria:**
- [ ] Marches outward in steps of `0.1·λ/D` rad from boresight until gain first drops below target, then bisects within that bracket
- [ ] Returns `ComputationError` (not a silent number) if no crossing is found within `π/4`
- [ ] Existing beamwidth test (tightened in Task 1) still passes

**Verify:** `cargo test -p antenna-model beamwidth` → PASS

**Steps:**

- [ ] **Step 1: Implement** (the Task-1 test already constrains the result; this is a robustness refactor):

```rust
pub fn compute_beamwidth(
    config: &AntennaConfiguration,
    frequency_hz: f64,
    gain_drop_db: f64,
    phi: f64,
    params: &IntegrationParams,
) -> ComputationResult<f64> {
    let result_peak = compute_gain_db(0.0, phi, config, frequency_hz, params)?;
    let target_gain = result_peak.gain - gain_drop_db;

    // March outward to bracket the FIRST crossing (avoids sidelobe lock-on).
    let wavelength = wavelength_from_frequency(frequency_hz);
    let step = 0.1 * wavelength / config.reflector.diameter;
    let mut theta_lo = 0.0;
    let mut theta_hi = step;
    loop {
        if theta_hi > PI / 4.0 {
            return Err(ComputationError::NumericalInstability {
                operation: "compute_beamwidth".to_string(),
                reason: format!("no -{gain_drop_db} dB crossing within 45 deg"),
            });
        }
        let g = compute_gain_db(theta_hi, phi, config, frequency_hz, params)?.gain;
        if g < target_gain {
            break;
        }
        theta_lo = theta_hi;
        theta_hi += step;
    }

    // Bisect within [theta_lo, theta_hi].
    for _ in 0..30 {
        let mid = 0.5 * (theta_lo + theta_hi);
        let g = compute_gain_db(mid, phi, config, frequency_hz, params)?.gain;
        if g > target_gain { theta_lo = mid; } else { theta_hi = mid; }
        if theta_hi - theta_lo < 1e-5 {
            break;
        }
    }
    Ok(0.5 * (theta_lo + theta_hi))
}
```

- [ ] **Step 2: Add a guard test** — a config with a deliberately coarse first sidelobe (the standard `test_antenna()` at high frequency, 32 GHz) still returns a beamwidth smaller than the first-null angle `1.22·λ/D`:

```rust
#[test]
fn test_beamwidth_does_not_lock_on_sidelobe() {
    let config = test_antenna();
    let params = IntegrationParams::fast();
    let hpbw = compute_beamwidth(&config, 32e9, 3.0, 0.0, &params).unwrap();
    let first_null = 1.22 * wavelength_from_frequency(32e9) / 1.0;
    assert!(hpbw < first_null, "beamwidth {hpbw} beyond first null {first_null}");
}
```

- [ ] **Step 3: Run** — `cargo test -p antenna-model beamwidth` → PASS

- [ ] **Step 4: Commit**

```bash
git add antenna-model/src/model/pattern.rs
git commit -m "fix: bracket first crossing in beamwidth search to avoid sidelobe lock-on"
```

---

## Task 16: Optional vehicle attitude for a stable antenna frame (F5b — critical, API addition)

**Goal:** Requests can supply a vehicle attitude quaternion; when present, the antenna-frame X-axis (azimuth zero) is derived from it — deterministic and matching the calibration's E-clock reference — instead of the discontinuous Earth-Z cross-product heuristic.

**Design decision (assumed, flag for review):** attitude is a unit quaternion `[w, x, y, z]` (scalar-first) rotating **body-frame vectors into ECEF**; body axes: +Z = antenna boresight, +X = E-clock zero reference. When supplied, body X (rotated to ECEF, projected ⊥ boresight) defines azimuth zero. When absent, current behaviour is kept (documented as approximate for azimuth-dependent quantities).

**Files:**
- Modify: `antenna-model/src/api/schemas.rs` (optional `vehicle_attitude: Option<[f64; 4]>` on `GainRequest` and `H3LinkBudgetRequest`)
- Modify: `antenna-model/src/model/coordinates_3d.rs` (quaternion rotate + attitude-aware frame construction)
- Modify: `antenna-model/src/service/evaluator.rs`, `antenna-model/src/service/h3_link_budget.rs` (thread attitude through)
- Modify: `openapi.yaml`

**Acceptance Criteria:**
- [ ] `vehicle_attitude` omitted → byte-identical behaviour to today (serde default `None`)
- [ ] With attitude supplied, azimuth zero tracks the body X-axis: rotating the attitude 90° about boresight rotates reported emitter azimuth by 90°
- [ ] Attitude validated: quaternion norm within 1e-3 of 1.0, else `ValidationError`
- [ ] Doc comment on the fallback path warns that auto-derived azimuth is approximate and discontinuous near boresight ‖ Earth-Z

**Verify:** `cargo test -p antenna-model attitude` → PASS

**Steps:**

- [ ] **Step 1: Failing test** in coordinates_3d.rs:

```rust
#[test]
fn test_attitude_defines_azimuth_zero() {
    // Boresight along ECEF +X; emitter offset toward ECEF +Y.
    let vehicle = Position3D::new(7_000_000.0, 0.0, 0.0);
    let boresight = Position3D::new(8_000_000.0, 0.0, 0.0);
    let emitter = Position3D::new(8_000_000.0, 50_000.0, 0.0);

    // Attitude A: body X = ECEF +Y (identity-ish for this geometry):
    // body Z (boresight) → ECEF +X, body X → ECEF +Y: rotation of +90° about ECEF Z
    // applied to a frame where body Z starts at ECEF Z... construct directly:
    let q_a = quaternion_from_axes((0.0, 1.0, 0.0), (1.0, 0.0, 0.0)); // (body_x_in_ecef, body_z_in_ecef)
    let (az_a, _el) =
        compute_emitter_direction_with_attitude(&emitter, &vehicle, &boresight, Some(q_a)).unwrap();
    assert!(az_a.abs() < 1e-6 || (az_a - 360.0).abs() < 1e-6, "emitter on body-X: az {az_a}");

    // Attitude B: body X = ECEF +Z → same emitter now at azimuth 270°.
    let q_b = quaternion_from_axes((0.0, 0.0, 1.0), (1.0, 0.0, 0.0));
    let (az_b, _el) =
        compute_emitter_direction_with_attitude(&emitter, &vehicle, &boresight, Some(q_b)).unwrap();
    assert!((az_b - 270.0).abs() < 1e-6, "rotated frame: az {az_b}");
}
```

- [ ] **Step 2: Run** — FAIL (functions absent).

- [ ] **Step 3: Implement** in coordinates_3d.rs:

```rust
/// Rotate vector v by unit quaternion q = [w, x, y, z] (body → ECEF).
pub fn quaternion_rotate(q: [f64; 4], v: (f64, f64, f64)) -> (f64, f64, f64) {
    let [w, x, y, z] = q;
    // v' = v + 2*qv × (qv × v + w*v), qv = (x, y, z)
    let (cx, cy, cz) = (
        y * v.2 - z * v.1 + w * v.0,
        z * v.0 - x * v.2 + w * v.1,
        x * v.1 - y * v.0 + w * v.2,
    );
    (
        v.0 + 2.0 * (y * cz - z * cy),
        v.1 + 2.0 * (z * cx - x * cz),
        v.2 + 2.0 * (x * cy - y * cx),
    )
}

/// Test helper: quaternion mapping body X→`x_ecef`, body Z→`z_ecef` (orthonormal inputs).
#[cfg(test)]
fn quaternion_from_axes(x_ecef: (f64, f64, f64), z_ecef: (f64, f64, f64)) -> [f64; 4] {
    let (x, z) = (x_ecef, z_ecef);
    let y = (
        z.1 * x.2 - z.2 * x.1, // y = z × x completes the right-handed frame
        z.2 * x.0 - z.0 * x.2,
        z.0 * x.1 - z.1 * x.0,
    );
    // Rotation matrix with columns [x y z]; convert via Shepperd's method.
    let (m00, m01, m02) = (x.0, y.0, z.0);
    let (m10, m11, m12) = (x.1, y.1, z.1);
    let (m20, m21, m22) = (x.2, y.2, z.2);
    let trace = m00 + m11 + m22;
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [s / 4.0, (m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [(m21 - m12) / s, s / 4.0, (m01 + m10) / s, (m02 + m20) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(m02 - m20) / s, (m01 + m10) / s, s / 4.0, (m12 + m21) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(m10 - m01) / s, (m02 + m20) / s, (m12 + m21) / s, s / 4.0]
    }
}
```

`compute_emitter_direction_with_attitude(emitter, vehicle, boresight, attitude: Option<[f64;4]>)`: identical to `compute_emitter_direction` except the X-axis branch — when `Some(q)`:

```rust
    // Body X rotated into ECEF, projected onto the plane ⊥ boresight (Z-axis):
    let (bx, by, bz) = quaternion_rotate(q, (1.0, 0.0, 0.0));
    let dot = bx * z_x + by * z_y + bz * z_z;
    let (x_x_raw, x_y_raw, x_z_raw) = (bx - dot * z_x, by - dot * z_y, bz - dot * z_z);
    // if projection degenerate (body X ‖ boresight) → ValidationError
```

Refactor `compute_emitter_direction` to delegate with `None`. Apply the same optional attitude to `compute_feed_offset_v2` and `compute_feed_position_from_pointing` (they share the frame-construction code — extract a shared `fn antenna_frame_axes(boresight_unit, attitude) -> ([f64;3],[f64;3],[f64;3])` helper to avoid the current triplicated cross-product block).

- [ ] **Step 4: Thread through the API** — add to `GainRequest` and `H3LinkBudgetRequest`:

```rust
    /// Optional vehicle attitude as a unit quaternion [w, x, y, z] (body→ECEF).
    /// Body +Z = boresight, body +X = azimuth-zero (E-clock zero) reference.
    /// When omitted, azimuth zero is derived from Earth-Z (approximate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vehicle_attitude: Option<[f64; 4]>,
```

Validate norm in `service/validator.rs` (`(w²+x²+y²+z²).sqrt()` within `1.0 ± 1e-3`). Pass `request.vehicle_attitude` to every `compute_emitter_direction_with_attitude`/`compute_feed_position_from_pointing` call in evaluator.rs and h3_link_budget.rs. Update `openapi.yaml`. Add the fallback warning doc to `compute_emitter_direction`.

- [ ] **Step 5: Run** — `cargo test -p antenna-model && cargo test --all` → PASS

- [ ] **Step 6: Commit**

```bash
git add antenna-model/src/api/schemas.rs antenna-model/src/model/coordinates_3d.rs antenna-model/src/service/evaluator.rs antenna-model/src/service/h3_link_budget.rs antenna-model/src/service/validator.rs openapi.yaml
git commit -m "feat: optional vehicle attitude quaternion defines a stable antenna-frame azimuth reference"
```

---

## Explicitly deferred (not tasks)

- **Cache invalidation on calibration reload:** no hot-reload path exists in `CalibrationRepository` today; `GainCache::invalidate` already exists for whoever adds one. Re-check when reload is built.
- **Ray-tracing / direct-path completion (>0.5f offsets):** both already warn; completing them is new feature work, not a defect fix — needs its own spec.
- **Spillover efficiency:** documented as unmodeled in Task 9; calibration absorbs it. Modeling it is feature work.
- **Coverage ranges that wrap 0°/360° azimuth:** Task 4 normalizes the query side; wrap-around *ranges* (min > max) are a calibrate-tool feature to spec separately.

## Post-plan operational note

After Tasks 1, 2, 9, 11, 14 land, **regenerate all calibration artifacts** in `calibration_data/` (the physics changes invalidate fitted residual surfaces) and re-run `cargo bench` to confirm the <100 ms p95 target still holds (Task 9 removes the doubled integration, so headroom should improve).

## Suggested execution order & dependencies

1 → 2 → 3 → 4 → 5 → 6 (critical path; 6 requires 5's loader)
7, 8 (8 requires 4), 10, 13, 15, 16 — independent after the critical batch
9 requires 1 and 2; 11 requires 1; 12 requires 9; 14 requires 9 (gain re-baselining)
