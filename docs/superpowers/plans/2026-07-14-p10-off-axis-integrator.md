# P10 — Off-Axis Aperture-Integral Aliasing Fix (Hankel / Azimuthal-Mode Integrator) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the aliasing 2D aperture quadrature in `integrate_aperture` with a mathematically exact 1D Hankel / azimuthal-mode integrator so that served off-axis gain is physically correct (not 20–35 dB too high) on every endpoint, at O(D/λ) cost.

**Architecture:** The far-field aperture integral's azimuthal dimension has a closed form (Jacobi–Anger). For an azimuthally symmetric aperture the 2D integral collapses to a 1D radial Hankel transform with a `J₀` kernel; for a laterally-displaced feed (coma) it becomes a short sum of azimuthal modes, each a radial transform with a `Jₘ` kernel weighting a θ-independent Fourier coefficient `gₘ(ρ)`. The public signature of `integrate_aperture` is unchanged, so the evaluator, cache, heatmap, and H3 paths are untouched; only the inner quadrature changes. Radial sampling is derived adaptively from `(D/λ, θ)` at ~2× Nyquist with a runtime N-vs-2N convergence self-check.

**Tech Stack:** Rust, `num_complex::Complex64`, existing `model/phase.rs` + `model/illumination.rs`, Simpson's rule. No new dependencies (in-house Bessel functions, matching the repo's no-BLAS pure-Rust rule).

**User decisions (already made):**
- D-1: "Azimuthal-mode expansion" — required, not optional. The served `gs_3.7m` / `dsn_13m` / `dsn_34m` antennas run laterally-offset feeds, so a symmetric-only integrator leaves the production configs aliased.
- D-2: P10 = correct integrator + honesty warning only. The statistical-model substitution/blend is a SEPARATE F7-redesign unit the maintainer decides later.
- D-3: interim honesty fix already shipped (message says "NUMERICALLY INVALID"). P10 reworks it from *numerical* invalidity to *physical* incompleteness once the integrator is correct.
- D-4: single adaptive correct path; `N_rho` from `(D/λ, θ)` at ~2× Nyquist; retire the `fast()`/`high_accuracy()` presets (demote to a safety-factor knob).
- D-5: fix `compute_gain_higher_order` too (same integrand); flag—don't fix—`compute_gain_ray_tracing` (P3 stub).
- D-6: ~2× Nyquist + runtime N-vs-2N convergence self-check that warns/refuses, never silently returns.
- "No silent physics changes": any change to `gain_physics` output bumps `PHYSICS_MODEL_VERSION` (P1b policy).

---

## Background — the math (read before Task 1)

The current integrand (`integration.rs::aperture_integrand`) is `A(ρ,φ')·exp(jΨ)·ρ`, integrated over `(ρ∈[0,R], φ'∈[0,2π])` by 2D Simpson. The total phase (`phase.rs::phase_total`) is:

```text
Ψ(ρ,φ',θ,φ) = Ψ_path + Ψ_feed_displacement + Ψ_surface + Ψ_mesh
Ψ_path = k·[ ρ²/(4f)·(1−cosθ)          (term1: ρ-only "chirp", θ-dependent)
            − ρ·sinθ·cos(φ−φ') ]        (term2: the 2D Fourier kernel)
```

- `term2` is **exactly** the Fourier kernel `−k(u·x+v·y)` with `u=sinθcosφ, v=sinθsinφ, x=ρcosφ', y=ρsinφ'`.
- `Ψ_feed_displacement` (coma + axial defocus, `phase.rs:156`) and the higher-order Seidel terms (`edge_cases.rs::higher_order_aberrations`) take **no θ/φ** — pure aperture-plane functions of `(ρ,φ')`.
- `Ψ_surface` (currently 0) and `Ψ_mesh` are **ρ-only**.

Define the θ-independent aperture-plane function (everything except `term2` and the chirp):

```text
g(ρ,φ') = A(ρ,φ') · exp( j·[ Ψ_feed_displacement(ρ,φ') + Ψ_higher_order(ρ,φ') + Ψ_surface(ρ) + Ψ_mesh(ρ) ] )
```

Fourier-expand in φ':  `g(ρ,φ') = Σ_m g_m(ρ) e^{jmφ'}`, with `g_m(ρ) = (1/2π)∫₀^{2π} g(ρ,φ') e^{−jmφ'} dφ'`.

Apply Jacobi–Anger `exp(−ja·cos(φ−φ')) = Σ_n (−j)^n J_n(a) e^{jn(φ−φ')}` with `a = kρ·sinθ`, and integrate over φ' (orthogonality picks `n=m`):

```text
I(θ,φ) = 2π · Σ_m (−j)^m e^{jmφ} · ∫₀^R exp( j·k·ρ²/(4f)·(1−cosθ) ) · g_m(ρ) · J_m(kρ·sinθ) · ρ dρ
```

**Symmetric case (no lateral feed offset):** `g` is φ'-independent ⇒ only `g_0(ρ)=g(ρ)` survives ⇒

```text
I(θ) = 2π · ∫₀^R exp( j·k·ρ²/(4f)·(1−cosθ) ) · A(ρ) · exp(j·[Ψ_surface+Ψ_mesh]) · J_0(kρ·sinθ) · ρ dρ
```

This is the spike form (`tests/reference_validation.rs::hankel_field`), cross-validated to Δ=0.00 dB vs the converged 2D everywhere the 2D holds, and to the −33.28 dBi brute-force ground truth at θ=90° where the 2D aliases.

**Radial Nyquist:** the chirp + `J_m` oscillate at radial rate `≈ (D/λ)·sinθ` cycles across `[0,R]`, so `N_ρ ≈ 2·(D/λ)·sinθ` is Nyquist. D-6: use **≈2× Nyquist** (`N_ρ ≈ 4·(D/λ)·sinθ`, floored at the current `min_rho_points`), with a runtime N-vs-2N self-check.

**Mode count M:** `g_m(ρ)` decays with `m` as the coma strength `kδ` grows; for the served offsets (`δ/f ≈ 0.004–0.011`) coma is `m≈1`-dominated and `M≈3–5` reaches <0.1 dB. Truncate adaptively (Task 3).

**Method warning (learned in the spike):** a wrong special-function implementation is *confidently* wrong — the spike's first cut used wrong `J₀` small-argument coefficients and was 22 dB off at θ=0 while looking perfect at θ=90° (asymptotic branch). Every Bessel/integrator step MUST be cross-checked at angles with independently-known answers spanning BOTH Bessel branches (`|x|<8` polynomial vs `|x|≥8` asymptotic).

---

## File Structure

- **Create** `antenna-model/src/model/bessel.rs` — in-house `bessel_j0`, `bessel_j1`, `bessel_jn` (real argument), with unit tests pinning both branches against known values. One responsibility: cylindrical Bessel `Jₘ`.
- **Modify** `antenna-model/src/model/integration.rs` — replace `integrate_2d_simpson`/`aperture_integrand` inner quadrature with the Hankel/mode integrator; add adaptive `N_ρ` + convergence self-check. Public `integrate_aperture` signature unchanged.
- **Modify** `antenna-model/src/model/mod.rs` — register `pub mod bessel;`, re-export; bump `PHYSICS_MODEL_VERSION`.
- **Modify** `antenna-model/src/model/pattern.rs` — `select_integration_params` / preset usage: single adaptive path (D-4); ensure `compute_gain_higher_order` routes through the new integrator (D-5).
- **Modify** `antenna-model/src/service/evaluator.rs` + `antenna-model/src/service/h3_link_budget.rs` — swap the `IntegrationParams::fast()` construction for the single adaptive constructor (D-4); rework the off-axis warning from *numerical* to *physical* incompleteness (D-3 completion).
- **Modify** `antenna-model/tests/reference_validation.rs` — promote the `#[ignore]`d spike into a real cross-validation suite over the required angle × D/λ × band grid, both Bessel branches, symmetric + offset-feed antennas.
- **Modify docs** `docs/domain-contract.md`, `docs/api-documentation.md`, `docs/roadmap-2026-07.md`, `docs/roadmap-2026-07-work-units.md` — record P10 as landed; unpark the F7 note as "unblocked, redesign pending (D-2)".

---

## Task 0: In-house Bessel `Jₘ` module

**Goal:** A validated `bessel::bessel_jn(m, x)` for real `x`, correct in both the small-argument (`|x|<8`) and asymptotic (`|x|≥8`) branches, with no external dependency.

**Files:**
- Create: `antenna-model/src/model/bessel.rs`
- Modify: `antenna-model/src/model/mod.rs` (add `pub mod bessel;`)
- Test: unit tests in `antenna-model/src/model/bessel.rs`

**Acceptance Criteria:**
- [ ] `bessel_j0`, `bessel_j1`, `bessel_jn(n, x)` implemented for real `x`, `n ≥ 0`.
- [ ] Each cross-checked against independently-known values in BOTH branches (see test table).
- [ ] `bessel_jn` uses downward (Miller) recurrence for `n ≥ 2` (upward recurrence is numerically unstable for `Jₙ`).
- [ ] No `unwrap`/`expect`/`panic` on the numeric path; `x` may be any finite f64.
- [ ] `cargo clippy -p antenna-model --all-targets -- -D warnings` clean.

**Verify:** `cargo test -p antenna-model --lib bessel -- --nocapture` → all pass.

**Steps:**

- [ ] **Step 1: Write the failing tests** (known reference values; `J₀(0)=1`, `J₁(0)=0`, zeros, both branches).

```rust
// antenna-model/src/model/bessel.rs
#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from Abramowitz & Stegun / standard tables.
    const TOL: f64 = 1e-6;

    #[test]
    fn j0_small_argument_branch() {
        assert!((bessel_j0(0.0) - 1.0).abs() < TOL);
        assert!((bessel_j0(1.0) - 0.765_197_686_5).abs() < TOL);
        assert!((bessel_j0(2.404_825_558) - 0.0).abs() < 1e-6); // first zero of J0
        assert!((bessel_j0(5.0) - (-0.177_596_771_3)).abs() < TOL);
    }

    #[test]
    fn j0_asymptotic_branch() {
        // |x| >= 8 exercises the asymptotic polynomial (the branch the spike got
        // right by luck while J0 small-arg was wrong — pin it explicitly).
        assert!((bessel_j0(10.0) - (-0.245_935_764_5)).abs() < TOL);
        assert!((bessel_j0(20.0) - 0.167_024_664_5).abs() < TOL);
    }

    #[test]
    fn j0_is_even() {
        assert!((bessel_j0(-3.3) - bessel_j0(3.3)).abs() < 1e-12);
    }

    #[test]
    fn j1_both_branches() {
        assert!((bessel_j1(0.0) - 0.0).abs() < TOL);
        assert!((bessel_j1(1.0) - 0.440_050_585_7).abs() < TOL);
        assert!((bessel_j1(5.0) - (-0.327_579_137_9)).abs() < TOL);   // small-arg
        assert!((bessel_j1(10.0) - 0.043_472_746_2).abs() < TOL);     // asymptotic
    }

    #[test]
    fn j1_is_odd() {
        assert!((bessel_j1(-2.5) + bessel_j1(2.5)).abs() < 1e-12);
    }

    #[test]
    fn jn_matches_j0_j1() {
        for &x in &[0.5, 1.0, 5.0, 9.0, 15.0] {
            assert!((bessel_jn(0, x) - bessel_j0(x)).abs() < 1e-9, "n=0 x={x}");
            assert!((bessel_jn(1, x) - bessel_j1(x)).abs() < 1e-9, "n=1 x={x}");
        }
    }

    #[test]
    fn jn_known_values() {
        // J2(5)=0.046565..., J3(10)=0.058379..., J5(10)=-0.234061...
        assert!((bessel_jn(2, 5.0) - 0.046_565_116_3).abs() < TOL);
        assert!((bessel_jn(3, 10.0) - 0.058_379_379_3).abs() < TOL);
        assert!((bessel_jn(5, 10.0) - (-0.234_061_528_2)).abs() < TOL);
    }

    #[test]
    fn jn_high_order_small_x_underflows_to_zero() {
        // J_m(x) ~ (x/2)^m / m! for small x; J10(0.1) is ~1e-26, must be finite & tiny.
        let v = bessel_jn(10, 0.1);
        assert!(v.is_finite() && v.abs() < 1e-20, "got {v}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail** — `cargo test -p antenna-model --lib bessel` → FAIL (functions not defined). (Add `pub mod bessel;` to `mod.rs` first so it compiles.)

- [ ] **Step 3: Implement using the Numerical Recipes rational approximations + Miller downward recurrence.**

```rust
//! In-house cylindrical Bessel functions Jₘ(x) for real argument.
//!
//! Pure Rust (no BLAS / no external crate — matches the repo's dependency rule).
//! `bessel_j0`/`bessel_j1` use the Numerical Recipes (Press et al.) rational
//! approximations: a polynomial ratio for |x| < 8 and an asymptotic amplitude/phase
//! form for |x| >= 8. `bessel_jn` uses Miller's downward recurrence, the stable
//! direction for Jₙ (upward recurrence amplifies round-off catastrophically).
//!
//! Validated in BOTH branches — see the module tests. A special-function routine
//! that is wrong is *confidently* wrong (see docs/findings-2026-07-13...): the
//! coefficients below are pinned by tests at |x|<8 and |x|>=8 independently.

/// Bessel function of the first kind, order 0.
pub fn bessel_j0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let p1 = 57_568_490_574.0
            + y * (-13_362_590_354.0
                + y * (651_619_640.7
                    + y * (-11_214_424.18 + y * (77_392.330_17 + y * (-184.905_245_6)))));
        let p2 = 57_568_490_411.0
            + y * (1_029_532_985.0
                + y * (9_494_680.718 + y * (59_272.648_53 + y * (267.853_271_2 + y))));
        p1 / p2
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 0.785_398_164;
        let p1 = 1.0
            + y * (-0.109_862_862_7e-2
                + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let p2 = -0.156_249_999_5e-1
            + y * (0.143_048_876_5e-3
                + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 + y * (-0.934_935_152e-7))));
        (std::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p1 - z * xx.sin() * p2)
    }
}

/// Bessel function of the first kind, order 1.
pub fn bessel_j1(x: f64) -> f64 {
    let ax = x.abs();
    let ans = if ax < 8.0 {
        let y = x * x;
        let p1 = x
            * (72_362_614_232.0
                + y * (-7_895_059_235.0
                    + y * (242_396_853.1
                        + y * (-2_972_611.439 + y * (15_704.482_60 + y * (-30.160_366_06))))));
        let p2 = 144_725_228_442.0
            + y * (2_300_535_178.0
                + y * (18_583_304.74 + y * (99_447.433_94 + y * (376.999_139_7 + y))));
        p1 / p2
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356_194_491;
        let p1 = 1.0
            + y * (0.183_105e-2
                + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_4e-5 + y * (-0.240_337_019_9e-6))));
        let p2 = 0.046_874_999_95
            + y * (-0.200_269_087_3e-3
                + y * (0.844_919_970_1e-5 + y * (-0.882_898_918_1e-6 + y * 0.105_787_412e-6)));
        let ans1 = (std::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p1 - z * xx.sin() * p2);
        // J1 is odd; restore the sign of x below.
        return if x < 0.0 { -ans1 } else { ans1 };
    };
    ans
}

/// Bessel function of the first kind, integer order `n >= 0`, real argument.
///
/// n=0,1 delegate to the rational approximations; n>=2 use Miller's downward
/// recurrence with renormalization (the numerically stable direction for Jₙ).
pub fn bessel_jn(n: u32, x: f64) -> f64 {
    match n {
        0 => return bessel_j0(x),
        1 => return bessel_j1(x),
        _ => {}
    }
    if x == 0.0 {
        return 0.0;
    }
    let ax = x.abs();
    let n = n as usize;

    // Recurrence: J_{m-1}(x) = (2m/x) J_m(x) - J_{m+1}(x).
    // Start above the requested order to seed the stable downward pass.
    let acc = 40; // NR "IACC": extra iterations for accuracy
    let big = 1.0e10_f64;
    let bigi = 1.0e-10_f64;

    let mut ans = 0.0;
    // Even starting index above n, scaled by argument size (NR heuristic).
    let m_start = 2 * ((n + (acc * ax.sqrt() as usize).max(1)) / 2 + 1);
    let mut bjp = 0.0_f64; // J_{m+1}
    let mut bj = 1.0_f64; // J_m (arbitrary seed; renormalized at the end)
    let mut sum = 0.0_f64;
    let tox = 2.0 / ax;
    for m in (1..=m_start).rev() {
        let bjm = m as f64 * tox * bj - bjp; // J_{m-1}
        bjp = bj;
        bj = bjm;
        if bj.abs() > big {
            // Renormalize to avoid overflow.
            bj *= bigi;
            bjp *= bigi;
            ans *= bigi;
            sum *= bigi;
        }
        if m % 2 == 0 {
            sum += bj; // even orders contribute to the normalization sum
        }
        if m == n {
            ans = bjp; // capture J_n as we pass it
        }
    }
    // Normalization: 1 = J0 + 2*(J2 + J4 + ...), i.e. sum here is J0 + 2ΣJ_even.
    sum = 2.0 * sum - bj; // undo double count of J0
    ans /= sum;
    // Jₙ(−x) = (−1)ⁿ Jₙ(x): correct the sign for negative x, odd n.
    if x < 0.0 && n % 2 == 1 {
        -ans
    } else {
        ans
    }
}
```

- [ ] **Step 4: Run tests to verify they pass** — `cargo test -p antenna-model --lib bessel -- --nocapture` → PASS. If `jn_known_values` fails, the recurrence seed/normalization is off — cross-check `bessel_jn(0,x)==bessel_j0(x)` first (it isolates the normalization from the order-capture).

- [ ] **Step 5: Commit** — `git add antenna-model/src/model/bessel.rs antenna-model/src/model/mod.rs && git commit -m "feat(P10): in-house Bessel Jm module, validated both branches"`

---

## Task 1: Symmetric Hankel radial integrator (m=0 fast path)

**Goal:** Replace the inner 2D quadrature of `integrate_aperture` with the 1D `J₀` Hankel transform for azimuthally symmetric apertures (no lateral feed offset), producing identical results to the old 2D where the 2D is valid and correct results where it aliased. Signature of `integrate_aperture` unchanged.

**Files:**
- Modify: `antenna-model/src/model/integration.rs` (add `hankel_radial_field`; route symmetric case through it)
- Test: unit tests in `integration.rs`; cross-validation in `tests/reference_validation.rs` (Task 4 formalizes)

**Acceptance Criteria:**
- [ ] A new `fn hankel_radial_field(config, theta, phi, k, n_rho, use_higher_order) -> Complex64` computing `I(θ) = 2π ∫ exp(j·chirp)·A(ρ)·exp(j·[Ψ_surface+Ψ_mesh])·J₀(kρsinθ)·ρ dρ` by Simpson over ρ.
- [ ] `integrate_aperture` detects the symmetric case (`config.feed.position.radial_displacement() == 0.0` AND no higher-order aberration active) and uses `hankel_radial_field`; the asymmetric case still uses the existing 2D path until Task 2 replaces it.
- [ ] At θ ∈ {0°,1°,5°,20°} on `dsn_34m_uncalibrated` x-band, the symmetric Hankel result matches the converged 2D (2048×4096) to < 0.05 dB.
- [ ] At θ=90° the symmetric Hankel yields −33.3 ± 0.3 dBi (physical), NOT the 2D's aliased +1.24 dBi.
- [ ] All existing `integration.rs` and `pattern.rs` unit tests still pass unchanged (they use small dishes / on-axis / near-in angles where old and new agree).

**Verify:** `cargo test -p antenna-model --lib integration && cargo test -p antenna-model --lib pattern` → PASS.

**Steps:**

- [ ] **Step 1: Write the failing test** pinning the symmetric-Hankel physical value at θ=90° and agreement near-in. Add to `integration.rs` tests, using a large synthetic dish so the 2D aliasing is unmistakable:

```rust
#[test]
fn hankel_symmetric_is_physical_off_axis() {
    // Large dish (D/λ ~ 280): 2D fast() aliases to a high, flat value off-axis;
    // the Hankel form must fall monotonically and stay well below boresight.
    let config = large_test_antenna(); // 10 m dish, feed at focus (symmetric)
    let f = 8.4e9;
    let g = |deg: f64| {
        let th = deg.to_radians();
        let r = integrate_aperture(th, 0.0, &config, f, &IntegrationParams::default()).unwrap();
        r.field.norm_sqr()
    };
    let g0 = g(0.0);
    // Off-axis power must be far below boresight and must DECREASE with angle
    // (the aliasing signature is a roughly flat high value — this rejects it).
    assert!(g(5.0) < g0 * 1e-2, "5deg not far below boresight");
    assert!(g(20.0) < g(5.0), "pattern must fall from 5deg to 20deg");
    assert!(g(90.0) < g(20.0), "pattern must fall from 20deg to 90deg");
}
```

Add the `large_test_antenna()` helper (10 m diameter, f/D=0.5, feed at focus, q≈2, no mesh):

```rust
fn large_test_antenna() -> AntennaConfiguration {
    let reflector = ReflectorGeometry::new(10.0, 5.0, 0.0).unwrap();
    let feed = FeedParameters::new(FeedPosition::at_focus(5.0), 2.0, 0.0, 1.0).unwrap();
    AntennaConfiguration::new("large".into(), "Large".into(), reflector, feed, None).unwrap()
}
```

- [ ] **Step 2: Run to verify it fails** — with today's aliasing 2D path, `g(5.0) < g0*1e-2` and the monotonic-fall asserts FAIL (off-axis is spuriously high/flat).

- [ ] **Step 3: Implement `hankel_radial_field` and route the symmetric case.** Reuse the ρ-only phase pieces from `phase.rs` (`phase_surface_error` is 0 here; `phase_mesh` if mesh present). Note the illumination `A(ρ)` for a symmetric feed is φ'-independent — evaluate at `phi_prime = 0`.

```rust
use crate::model::bessel::bessel_j0;

/// Symmetric-aperture (no lateral feed offset) Hankel radial field:
///   I(θ) = 2π ∫₀^R exp(j·k·ρ²/(4f)·(1−cosθ)) · A(ρ) · exp(j·Ψ_ρonly) · J₀(kρ sinθ) · ρ dρ
/// where Ψ_ρonly = phase_mesh (+ axial-defocus chirp via the ρ² term if the feed is
/// axially offset — see note). Simpson's rule over ρ with `n_rho` (odd) points.
fn hankel_radial_field(
    config: &AntennaConfiguration,
    theta: f64,
    _phi: f64,
    k: f64,
    n_rho: usize,
) -> Complex64 {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    let mesh_spacing = config.mesh.as_ref().map_or(0.0, |m| m.spacing);
    // Axial defocus (feed z-offset + deliberate axial_defocus) adds a ρ-only quadratic
    // phase that is azimuthally symmetric — fold it into the chirp. Lateral offset is
    // excluded here by the caller (symmetric path only).
    let axial = config.feed.position.z - f + config.feed.axial_defocus;

    let n = if n_rho.is_multiple_of(2) { n_rho + 1 } else { n_rho };
    let h = r_max / (n - 1) as f64;
    let mut sum = Complex64::new(0.0, 0.0);
    for i in 0..n {
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        let amp = illumination_amplitude(rho, 0.0, &config.feed, f);
        // Dish-depth chirp + axial defocus (both ρ-only, azimuthally symmetric):
        let chirp = k * rho * rho / (4.0 * f) * (1.0 - theta.cos());
        // Axial defocus: k·axial·(path curvature). Use phase_feed_displacement with
        // delta_feed=0 to reuse the exact geometric model (φ'-independent when lateral=0).
        let defocus = if axial.abs() > 0.0 {
            crate::model::phase::phase_feed_displacement(rho, 0.0, 0.0, 0.0, axial, f, k)
        } else {
            0.0
        };
        let mesh = if mesh_spacing > 0.0 {
            let theta_inc = rho / (2.0 * f);
            crate::model::phase::phase_mesh(mesh_spacing, theta_inc, k)
        } else {
            0.0
        };
        let j0 = bessel_j0(k * rho * theta.sin());
        let phase = chirp + defocus + mesh;
        sum += Complex64::new(0.0, phase).exp() * amp * j0 * rho * w;
    }
    sum * (h / 3.0) * 2.0 * PI
}
```

Route it inside `integrate_aperture`, replacing the `for iteration ... integrate_2d_simpson` block *for the symmetric case only*. Compute `n_rho` from the adaptive policy stubbed here as `params.max_rho_points` (Task 3 makes it truly adaptive):

```rust
let is_symmetric = config.feed.position.radial_displacement() == 0.0
    && !params.use_higher_order_aberrations;
if is_symmetric {
    // Adaptive N_rho lands in Task 3; use a safe density now.
    let n_rho = radial_points_for(config, theta, wavelength, params); // Task 3 provides; interim: params.max_rho_points
    let field = hankel_radial_field(config, theta, phi, k, n_rho);
    return Ok(IntegrationResult { field, error_estimate: 0.0, num_evaluations: n_rho, converged: true });
}
// else: existing 2D path (asymmetric) until Task 2.
```

For the interim before Task 3, define `fn radial_points_for(...) -> usize { params.max_rho_points.max(2049) }` so the symmetric test converges; Task 3 replaces the body.

- [ ] **Step 4: Run to verify pass** — `cargo test -p antenna-model --lib integration` → the new test PASSES; existing tests still PASS (on-axis: `sinθ=0 ⇒ J₀(0)=1`, chirp=0 ⇒ integral = `2π∫A(ρ)ρdρ`, matching the 2D on-axis value).

- [ ] **Step 5: Commit** — `git add antenna-model/src/model/integration.rs && git commit -m "feat(P10): symmetric Hankel radial integrator (m=0), replaces aliasing 2D for symmetric apertures"`

---

## Task 2: Azimuthal-mode expansion for coma / asymmetric apertures

**Goal:** Generalize the integrator to laterally-displaced feeds (the served `dsn_34m`/`dsn_13m`/`gs_3.7m` offset feeds) via the `Jₘ` azimuthal-mode expansion, so those served configs stop aliasing. Includes the higher-order Seidel terms (D-5) since they are aperture-plane φ'-dependent phase.

**Files:**
- Modify: `antenna-model/src/model/integration.rs` (add `azimuthal_mode_field`; route asymmetric case)
- Test: unit tests in `integration.rs`; small-dish-with-offset cross-validation

**Acceptance Criteria:**
- [ ] A `fn azimuthal_mode_field(config, theta, phi, k, n_rho, n_phi_coeff, m_max, use_higher_order) -> Complex64` implementing `I(θ,φ) = 2π Σ_{m=−M}^{M} (−j)^m e^{jmφ} ∫ exp(j·chirp)·g_m(ρ)·J_m(kρsinθ)·ρ dρ`, where `g_m(ρ)` is a θ-independent φ'-Fourier coefficient computed by quadrature over φ'.
- [ ] `integrate_aperture` routes the asymmetric case (lateral offset > 0 OR higher-order enabled) through it; symmetric case still uses Task 1's fast path.
- [ ] On a **small** dish (3.7 m, where 2D converges) with a lateral feed offset matching the served x-band feed (`[0.05,0,0]`), the mode expansion matches the converged 2D `high_accuracy()` to < 0.1 dB at θ ∈ {0°,1°,5°,20°} and reproduces coma asymmetry (gain at +φ ≠ gain at −φ off-axis).
- [ ] On `dsn_34m_uncalibrated` x-band (offset `[0.15,0,0]`), the result is physical off-axis (no rise with θ; θ=90° well below main−30 dB) and mode-count-converged (M vs M+1 agree < 0.1 dB — pinned by Task 3's self-check).
- [ ] Existing coma/higher-order tests in `pattern.rs`/`edge_cases.rs` still pass (near-in angles).

**Verify:** `cargo test -p antenna-model --lib integration && cargo test -p antenna-model --lib pattern && cargo test -p antenna-model --lib edge_cases` → PASS.

**Steps:**

- [ ] **Step 1: Write the failing test** — small-dish-with-offset agreement vs 2D, and coma asymmetry:

```rust
#[test]
fn azimuthal_modes_match_2d_small_dish_with_offset() {
    // 3.7 m dish, X-band, lateral feed offset 0.05 m (the served gs_3.7m x-band feed).
    // The 2D quadrature is trustworthy here (D/λ ~ 104, small), so it is ground truth
    // near-in. The mode expansion must match it and reproduce coma asymmetry.
    let config = offset_feed_test_antenna(3.7, 1.85, 0.05); // helper below
    let f = 8.4e9;
    let mut hi = IntegrationParams::high_accuracy();
    hi.min_rho_points = 512; hi.max_rho_points = 512;
    hi.min_phi_points = 1024; hi.max_phi_points = 1024; hi.max_iterations = 1;

    for deg in [0.0_f64, 1.0, 5.0, 20.0] {
        let th = deg.to_radians();
        // 2D reference via the (still-present) 2D path forced by a symmetric-breaking config:
        let ref_field = integrate_2d_simpson_public_shim(th, 0.0, &config, f, &hi);
        let mode_field = azimuthal_mode_field(&config, th, 0.0, wavenumber(wavelength_from_frequency(f)), 4097, 64, 8, false);
        let d_db = 20.0 * (mode_field.norm() / ref_field.norm()).log10();
        assert!(d_db.abs() < 0.1, "θ={deg}: mode vs 2D Δ={d_db:.3} dB");
    }
    // Coma asymmetry: off-axis in the +x plane (φ=0) vs −x plane (φ=π) must differ.
    let th = 3.0_f64.to_radians();
    let k = wavenumber(wavelength_from_frequency(f));
    let plus = azimuthal_mode_field(&config, th, 0.0, k, 4097, 64, 8, false).norm();
    let minus = azimuthal_mode_field(&config, th, PI, k, 4097, 64, 8, false).norm();
    assert!((plus - minus).abs() / plus.max(minus) > 1e-3, "coma asymmetry absent");
}
```

Helpers (add to test module):

```rust
fn offset_feed_test_antenna(diameter: f64, focal: f64, lateral: f64) -> AntennaConfiguration {
    let reflector = ReflectorGeometry::new(diameter, focal, 0.0).unwrap();
    let mut pos = FeedPosition::at_focus(focal);
    pos.x = lateral; // lateral offset in +x → breaks azimuthal symmetry (coma)
    let feed = FeedParameters::new(pos, 2.0, 0.0, 1.0).unwrap();
    AntennaConfiguration::new("off".into(), "Off".into(), reflector, feed, None).unwrap()
}
```

If `integrate_2d_simpson` is private, add a `#[cfg(test)]` shim `integrate_2d_simpson_public_shim` in `integration.rs` that calls it (the 2D path is the trusted near-in reference here).

- [ ] **Step 2: Run to verify it fails** — `azimuthal_mode_field` not defined → FAIL.

- [ ] **Step 3: Implement `azimuthal_mode_field`.** Compute `g_m(ρ)` by uniform-grid DFT over φ' (the coma phase is slowly varying in φ', so a modest `n_phi_coeff` resolves it, θ-independent). `g(ρ,φ')` reuses the exact aperture-plane phase from `phase.rs` (feed displacement + optional higher-order) — everything the 2D integrand has except `term2` (the kernel) and the chirp (added in the radial loop).

```rust
use crate::model::bessel::bessel_jn;
use crate::model::edge_cases::higher_order_aberrations;

/// Aperture-plane function g(ρ,φ') = A·exp(j·[Ψ_feed_disp + Ψ_higher_order + Ψ_mesh]),
/// i.e. the full integrand phase MINUS the Fourier kernel term2 and MINUS the chirp
/// (both added in the radial loop). θ/φ do not enter here.
fn aperture_plane_g(
    config: &AntennaConfiguration,
    rho: f64,
    phi_prime: f64,
    k: f64,
    use_higher_order: bool,
) -> Complex64 {
    let f = config.reflector.focal_length;
    let amp = illumination_amplitude(rho, phi_prime, &config.feed, f);
    let delta = config.feed.position.radial_displacement();
    let alpha = config.feed.position.y.atan2(config.feed.position.x);
    let axial = config.feed.position.z - f + config.feed.axial_defocus;
    let mut phase = 0.0;
    if delta > 0.0 || axial.abs() > 0.0 {
        phase += crate::model::phase::phase_feed_displacement(rho, phi_prime, delta, alpha, axial, f, k);
    }
    if use_higher_order && delta > 0.0 {
        phase += higher_order_aberrations(rho, phi_prime, delta, alpha, f, k);
    }
    if let Some(ref mesh) = config.mesh {
        let theta_inc = rho / (2.0 * f);
        phase += crate::model::phase::phase_mesh(mesh.spacing, theta_inc, k);
    }
    Complex64::new(0.0, phase).exp() * amp
}

/// Azimuthal-mode-expansion aperture field for an asymmetric (coma) aperture:
///   I(θ,φ) = 2π Σ_{m=-M}^{M} (-j)^m e^{jmφ} · R_m(θ)
///   R_m(θ) = ∫₀^R exp(j·chirp(ρ,θ)) · g_m(ρ) · J_m(kρ sinθ) · ρ dρ
///   g_m(ρ) = (1/2π) ∫₀^{2π} g(ρ,φ') e^{-jmφ'} dφ'      (θ-independent)
fn azimuthal_mode_field(
    config: &AntennaConfiguration,
    theta: f64,
    phi: f64,
    k: f64,
    n_rho: usize,
    n_phi_coeff: usize,
    m_max: u32,
    use_higher_order: bool,
) -> Complex64 {
    let f = config.reflector.focal_length;
    let r_max = config.reflector.diameter / 2.0;
    let n = if n_rho.is_multiple_of(2) { n_rho + 1 } else { n_rho };
    let h = r_max / (n - 1) as f64;
    let dphi = 2.0 * PI / n_phi_coeff as f64;

    // Accumulate R_m for m = 0..=m_max (negative m handled by symmetry J_{-m}=(-1)^m J_m).
    let mut r_pos = vec![Complex64::new(0.0, 0.0); (m_max + 1) as usize];
    let mut r_neg = vec![Complex64::new(0.0, 0.0); (m_max + 1) as usize];

    for i in 0..n {
        let rho = i as f64 * h;
        let w = simpson_weight(i, n);
        let chirp = k * rho * rho / (4.0 * f) * (1.0 - theta.cos());
        let chirp_factor = Complex64::new(0.0, chirp).exp();
        let a = k * rho * theta.sin();

        // g_m(ρ) via uniform-grid DFT over φ' (θ-independent; trapezoid == rectangle on a
        // periodic grid). Compute all needed m in one φ' sweep.
        // g_m = (1/2π) Σ_j g(ρ,φ'_j) e^{-jmφ'_j} · dφ'
        let mut gm = vec![Complex64::new(0.0, 0.0); (m_max + 1) as usize];
        for j in 0..n_phi_coeff {
            let phip = j as f64 * dphi;
            let g = aperture_plane_g(config, rho, phip, k, use_higher_order);
            for m in 0..=m_max {
                let e = Complex64::new(0.0, -(m as f64) * phip).exp();
                gm[m as usize] += g * e;
            }
        }
        for m in 0..=m_max {
            gm[m as usize] *= dphi / (2.0 * PI);
        }

        // Radial integrand contribution for each m: exp(j·chirp)·g_m·J_m(a)·ρ·w
        for m in 0..=m_max {
            let jm = bessel_jn(m, a);
            let contrib = chirp_factor * gm[m as usize] * jm * rho * w;
            r_pos[m as usize] += contrib;
            if m > 0 {
                // g_{-m}(ρ) = conj-free separate coefficient; recompute via e^{+jmφ'}:
                // but J_{-m} = (-1)^m J_m, and the coefficient for -m uses e^{+jmφ'}.
                // Accumulate g_{-m} in the same sweep would be cleaner; here recompute:
            }
        }
    }
    // Simpson radial scale.
    let scale = h / 3.0;
    // Assemble: I = 2π Σ_m (-j)^m e^{jmφ} R_m, summing m>0 for both +m and -m.
    let mut acc = r_pos[0] * scale; // m=0 term
    for m in 1..=m_max as usize {
        let mf = m as f64;
        let jpow_pos = pow_neg_j(m as i32); // (-j)^m
        let jpow_neg = pow_neg_j(-(m as i32)); // (-j)^{-m} = (j)^m
        let epos = Complex64::new(0.0, mf * phi).exp();
        let eneg = Complex64::new(0.0, -mf * phi).exp();
        // R_{-m} relates to R_{+m} through g_{-m} and J_{-m}=(-1)^m J_m; simplest correct
        // route is to also accumulate g_{-m}. See Step 3b refinement.
        acc += jpow_pos * epos * r_pos[m] * scale;
        acc += jpow_neg * eneg * r_neg[m] * scale;
    }
    acc * 2.0 * PI
}

/// (-j)^m for integer m (m may be negative): (-j)^m = exp(-j·m·π/2).
fn pow_neg_j(m: i32) -> Complex64 {
    Complex64::new(0.0, -(m as f64) * std::f64::consts::FRAC_PI_2).exp()
}
```

- [ ] **Step 3b: Refinement — accumulate both +m and −m coefficients in the φ' sweep.** The `r_neg` array above must be filled: in the same `j` loop, also accumulate `g_{-m}(ρ)` using `e^{+jmφ'}`, then `R_{-m}` uses `J_{-m}=(-1)^m J_m`. Concretely, inside the φ' loop add `gm_neg[m] += g * Complex64::new(0.0, (m as f64)*phip).exp();`, scale identically, and in the radial accumulation set `r_neg[m] += chirp_factor * gm_neg[m] * ((-1f64).powi(m as i32) * jm) * rho * w;`. **Validation guards this**: if the −m branch is wrong, the small-dish-vs-2D test fails (the 2D has no such split), so implement, run, and let the test arbitrate. For a **real-valued** aperture with the feed offset along +x (`alpha=0`), `g_{-m}=conj(g_m)` and the sum is real-symmetric — a useful debugging cross-check, but do not hard-code the assumption (ka-band feed is offset along +y).

- [ ] **Step 4: Run to verify pass** — `cargo test -p antenna-model --lib integration` → the offset test PASSES to <0.1 dB and coma asymmetry is present. Iterate `m_max`/`n_phi_coeff` until agreement; record the minimal `m_max` that hits 0.1 dB (feeds Task 3's adaptive default).

- [ ] **Step 5: Commit** — `git add antenna-model/src/model/integration.rs && git commit -m "feat(P10): azimuthal-mode (Jm) expansion for coma/asymmetric apertures + higher-order Seidel (D-5)"`

---

## Task 3: Adaptive radial density + mode-count + convergence self-check (D-6, D-1 budget)

**Goal:** Derive `N_ρ` from `(D/λ, θ)` at ~2× Nyquist and `M` (mode count) adaptively, with a runtime N-vs-2N (and M-vs-M+1) self-check that flags non-convergence instead of silently returning — replacing the interim fixed densities from Tasks 1–2.

**Files:**
- Modify: `antenna-model/src/model/integration.rs` (`radial_points_for`, mode-count policy, self-check; wire into both integrator paths)
- Test: unit tests in `integration.rs`

**Acceptance Criteria:**
- [ ] `radial_points_for(config, theta, wavelength, params)` returns `N_ρ ≈ 4·(D/λ)·sinθ`, floored at `params.min_rho_points`, capped at a configured safety maximum; documented.
- [ ] A runtime convergence self-check recomputes at `N` and `2N` (symmetric) — or `M` and `M+1` (asymmetric) — and sets `IntegrationResult::converged=false` + a large `error_estimate` when they disagree by more than a relative tolerance, never silently returning an unconverged value.
- [ ] The mode-count default is the minimal `m_max` established in Task 2 (e.g. `m_max` scaling with `k·δ`), with the self-check catching under-resolution.
- [ ] θ=0 uses `N_ρ = min_rho_points` (chirp & kernel vanish; no oversampling) and remains cheap.
- [ ] A test asserts convergence at θ=90° for the largest enabled antenna (`gbt_100m` q-band, D/λ≈16000) completes and returns `converged=true` within the latency budget assumptions (self-check disagreement < tol).

**Verify:** `cargo test -p antenna-model --lib integration -- --nocapture` → PASS; note printed N_ρ for gbt_100m q-band θ=90°.

**Steps:**

- [ ] **Step 1: Write failing tests** — adaptive density scaling + non-convergence signalling:

```rust
#[test]
fn radial_density_scales_with_dlambda_sintheta() {
    let small = test_antenna();        // 1 m
    let large = large_test_antenna();  // 10 m
    let wl = wavelength_from_frequency(8.4e9);
    let p = IntegrationParams::default();
    // θ=0 → floor for both.
    assert_eq!(radial_points_for(&small, 0.0, wl, &p), p.min_rho_points.max(1) | 1);
    // θ=90° → large dish needs ~10× the points of the small dish (∝ D/λ).
    let ns = radial_points_for(&small, PI / 2.0, wl, &p);
    let nl = radial_points_for(&large, PI / 2.0, wl, &p);
    assert!(nl > ns * 5, "large={nl} small={ns}");
}

#[test]
fn unconverged_is_flagged_not_silently_returned() {
    // Force a tiny cap so the self-check cannot converge at a hard off-axis angle,
    // and assert converged=false rather than a plausible-looking number.
    let config = large_test_antenna();
    let mut p = IntegrationParams::default();
    p.max_rho_points = 33; // absurdly low cap for θ=90° on a 10 m dish
    let r = integrate_aperture(90f64.to_radians(), 0.0, &config, 8.4e9, &p).unwrap();
    assert!(!r.converged, "must flag non-convergence at capped density");
}
```

- [ ] **Step 2: Run to verify fail** — `radial_points_for` not yet defined / self-check absent → FAIL.

- [ ] **Step 3: Implement.** Adaptive density + self-check; wire into both integrator branches from Tasks 1–2.

```rust
/// Radial sample count for the Hankel/mode integrator at ~2× Nyquist.
/// Nyquist for the chirp + Jₘ oscillation is N_ρ ≈ 2·(D/λ)·sinθ; we take ~2× that,
/// floored at min_rho_points and capped at max_rho_points (the safety-factor knob, D-4).
fn radial_points_for(
    config: &AntennaConfiguration,
    theta: f64,
    wavelength: f64,
    params: &IntegrationParams,
) -> usize {
    let d_lambda = config.reflector.diameter / wavelength;
    let nyquist = 2.0 * d_lambda * theta.sin().abs();
    let target = (4.0 * d_lambda * theta.sin().abs()).ceil() as usize; // ~2× Nyquist
    let n = target.max(params.min_rho_points).min(params.max_rho_points);
    let _ = nyquist;
    if n.is_multiple_of(2) { n + 1 } else { n }
}
```

In `integrate_aperture`, for the symmetric path compute at `n` and `2n` (capped) and compare; for the asymmetric path compare `m_max` and `m_max+1`. Set `converged`/`error_estimate` accordingly. Keep the `converged=false` warning that `compute_gain_standard` already surfaces (`INTEGRATION_NONCONVERGENCE_WARNING`).

```rust
// Symmetric path with self-check:
let n1 = radial_points_for(config, theta, wavelength, params);
let f1 = hankel_radial_field(config, theta, phi, k, n1);
let n2 = (2 * n1 - 1).min(params.max_rho_points * 2 + 1);
let f2 = hankel_radial_field(config, theta, phi, k, n2);
let diff = (f2 - f1).norm();
let converged = diff <= params.relative_tolerance * f2.norm().max(params.absolute_tolerance);
return Ok(IntegrationResult {
    field: f2,
    error_estimate: diff,
    num_evaluations: n1 + n2,
    converged,
});
```

- [ ] **Step 4: Run to verify pass** — both tests PASS; print `radial_points_for(gbt_100m q-band, 90°)` to confirm it is O(10⁴) (≈32k → ~2 ms per the spike), not O(10⁸).

- [ ] **Step 5: Commit** — `git add antenna-model/src/model/integration.rs && git commit -m "feat(P10): adaptive N_rho (~2x Nyquist) + N-vs-2N / mode convergence self-check (D-6)"`

---

## Task 4: Validation protocol — cross-check grid (REQUIRED, do not shortcut)

**Goal:** Promote the `#[ignore]`d spike into a real, non-ignored cross-validation suite covering the required angle × D/λ × band grid, both Bessel branches, symmetric AND offset-feed antennas — closing the test/production integrator gap at the harness level.

**Files:**
- Modify: `antenna-model/tests/reference_validation.rs` (replace the spike with the protocol suite; keep `bessel_j0`/`hankel_field` only if still needed as an independent oracle)
- Test: the new suite itself

**Acceptance Criteria:**
- [ ] A non-ignored test cross-checks the production `integrate_aperture` (via `compute_gain_db`) against a converged 2D reference at θ ∈ {0°,1°,5°,20°} for the **smallest (3.7 m) and largest (100 m)** enabled antennas, in both Bessel branches (small `a` near-in, large `a` far-off), agreeing < 0.1 dB where the 2D is trustworthy.
- [ ] At θ=90° the served path returns a **physically plausible** value for every enabled antenna: no backlobe above (boresight − 30 dB), and gain does not rise with θ (assert monotone-ish fall across {1°,5°,20°,90°} allowing sidelobe ripple by comparing envelope maxima per decade).
- [ ] Repeated across bands (S → Ka/Q where each antenna defines them), since aliasing onset scales with `(D/λ)·sinθ`.
- [ ] The offset-feed antennas (`dsn_34m` x/ka, `dsn_13m` x/ka, `gs_3.7m` x) are covered for coma; mode-count convergence asserted (M vs M+1 < 0.1 dB).
- [ ] Suite runs in CI time budget (use `#[ignore]` ONLY for the multi-second brute-force ground-truth legs; the fast Hankel-vs-Hankel-convergence checks run by default).

**Verify:** `cargo test -p antenna-model --test reference_validation -- --nocapture` → PASS (non-ignored suite); `... p10 -- --ignored` for the brute-force legs.

**Steps:**

- [ ] **Step 1: Write the protocol test** iterating the enabled repository antennas, asserting the plausibility invariants and (on small dishes) 2D agreement. Use the existing `load_real_repository()` helper and the known values from the findings doc as anchors:

```rust
/// P10 protocol: the served integrator must be physical off-axis for EVERY enabled antenna.
#[test]
fn p10_served_offaxis_is_physical_all_enabled_antennas() {
    let repo = load_real_repository();
    for (antenna_id, feed_id, f_hz) in enabled_antenna_bands() { // helper enumerating configs
        let cal = repo.get_calibration(&antenna_id, &feed_id).expect(&antenna_id);
        let cfg = focused_config(&cal, None);
        let g = |deg: f64| compute_gain_db(deg.to_radians(), 0.0, &cfg, f_hz, &adaptive_params()).unwrap().gain;
        let g0 = g(0.0);
        // No plausible antenna has a backlobe within 30 dB of peak for these idealized dishes.
        for deg in [5.0, 20.0, 90.0, 179.0] {
            assert!(g(deg) < g0 - 30.0, "{antenna_id}/{feed_id} @ {deg}°: {:.1} vs peak {:.1}", g(deg), g0);
        }
        // Must not RISE from 1° to 5° (the aliasing signature).
        assert!(g(5.0) <= g(1.0) + 1.0, "{antenna_id}/{feed_id}: gain rises 1°→5°");
    }
}

/// Anchor values from docs/findings-2026-07-13 (independently reproduced by brute force).
#[test]
fn p10_dsn34m_xband_matches_known_reference_values() {
    let repo = load_real_repository();
    let cal = repo.get_calibration("dsn_34m_uncalibrated", "x_band").unwrap();
    let cfg = focused_config(&cal, None);
    let f = 8.4e9;
    let g = |deg: f64| compute_gain_db(deg.to_radians(), 0.0, &cfg, f, &adaptive_params()).unwrap().gain;
    assert!((g(0.0) - 68.96).abs() < 0.2, "peak {:.2}", g(0.0));
    assert!((g(1.0) - 14.53).abs() < 0.5, "1° {:.2}", g(1.0));
    assert!((g(90.0) - (-33.3)).abs() < 1.5, "90° {:.2} (expect ~-33.3)", g(90.0));
}
```

Add `enabled_antenna_bands()`, `adaptive_params()` helpers. `adaptive_params()` returns the single canonical `IntegrationParams` the service now uses (Task 5 defines the constructor; import it).

- [ ] **Step 2: Run to verify** — before Task 5's wiring these may need `adaptive_params()` to mirror the service; run and confirm the *physics* invariants hold against the new integrator.

- [ ] **Step 3: Keep the brute-force ground-truth leg** (`p10_spike_hankel_vs_2d`) as `#[ignore]`d, updated to compare the new production integrator (not the test-local `hankel_field`) against 2D, so it stays an independent oracle.

- [ ] **Step 4: Run full** — `cargo test -p antenna-model --test reference_validation` → PASS; `--ignored` legs PASS.

- [ ] **Step 5: Commit** — `git add antenna-model/tests/reference_validation.rs && git commit -m "test(P10): off-axis validation protocol — angle×D/λ×band grid, both Bessel branches, coma feeds"`

---

## Task 5: Single adaptive service path; retire presets; route higher-order (D-4, D-5)

**Goal:** Make the service call the one correct adaptive integrator everywhere, retiring the `fast()`/`high_accuracy()` role split that hid this bug; ensure `compute_gain_higher_order` uses the mode integrator; flag (don't fix) ray-tracing.

**Files:**
- Modify: `antenna-model/src/model/integration.rs` (`IntegrationParams`: demote presets to safety-factor knobs; add `IntegrationParams::adaptive()` or repurpose `default()`)
- Modify: `antenna-model/src/model/pattern.rs` (`select_integration_params`, `compute_gain_higher_order` path)
- Modify: `antenna-model/src/service/evaluator.rs:211`, `antenna-model/src/service/h3_link_budget.rs:308` (construct the adaptive params)
- Test: existing service + integration tests; a test asserting evaluator & harness share the path

**Acceptance Criteria:**
- [ ] The service (`evaluator.rs`, `h3_link_budget.rs`) constructs a single canonical `IntegrationParams` (the adaptive one); `IntegrationParams::fast()`/`high_accuracy()` are either removed or clearly demoted to radial safety-factor presets with doc comments saying density is now derived from `(D/λ,θ)`.
- [ ] `compute_gain_higher_order` produces mode-expanded (non-aliased) off-axis gain (routes through `azimuthal_mode_field` with `use_higher_order=true`).
- [ ] `compute_gain_ray_tracing` is unchanged except its existing degraded-accuracy warning is retained (D-5: flag, don't fix).
- [ ] A test asserts the evaluator and `reference_validation` harness evaluate the same gain through the same code path (call `compute_gain_db` with the canonical params, compare to the service result for one geometry).
- [ ] Full workspace green: `cargo test --workspace`.

**Verify:** `cargo test --workspace 2>&1 | grep -c "0 failed"` → equals the binary count (currently 18); `cargo clippy --workspace --all-targets -- -D warnings` clean.

**Steps:**

- [ ] **Step 1: Write/adjust tests** — a service-vs-harness same-path test, and update any test that asserted preset-specific point counts (those assertions about `fast()` having fewer points may now be vacuous — update, don't delete the intent).

```rust
// in tests/integration or evaluator tests:
#[test]
fn service_and_direct_gain_agree_off_axis() {
    // The path that hid the bug: harness used high_accuracy, service used fast.
    // Now both must be the SAME adaptive path and agree exactly.
    let repo = load_real_repository();
    let cal = repo.get_calibration("dsn_34m_uncalibrated", "x_band").unwrap();
    let cfg = focused_config(&cal, None);
    let params = IntegrationParams::adaptive();
    let direct = compute_gain_db(20f64.to_radians(), 0.0, &cfg, 8.4e9, &params).unwrap().gain;
    // Service call for the same geometry (build the request that yields θ=20°, φ=0)...
    // assert (service_gain - direct).abs() < 1e-9;
}
```

- [ ] **Step 2: Run to verify fail** — `IntegrationParams::adaptive()` not defined / service still uses `fast()` → FAIL or divergence.

- [ ] **Step 3: Implement.** Add `IntegrationParams::adaptive()` (sane floors/caps; the density comes from `radial_points_for`). Update `evaluator.rs:211` and `h3_link_budget.rs:308`:

```rust
// evaluator.rs — was: let mut integration_params = IntegrationParams::fast();
let mut integration_params = IntegrationParams::adaptive();
```

Ensure `compute_gain_higher_order` passes `use_higher_order_aberrations: true` into the integrator so `integrate_aperture` selects `azimuthal_mode_field(..., use_higher_order=true)`. Retain the ray-tracing warning block untouched.

- [ ] **Step 4: Run to verify pass** — `cargo test --workspace` → all green; `service_and_direct_gain_agree_off_axis` passes to 1e-9.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "refactor(P10): single adaptive integrator path in service; route higher-order; retire fast/high_accuracy split (D-4,D-5)"`

---

## Task 6: Rework off-axis warning (numerical→physical), bump version, update docs

**Goal:** Now that off-axis gain is numerically valid, change the D-3 interim "NUMERICALLY INVALID" warning into the honest *physical-incompleteness* warning (idealised PO omits blockage/strut/diffraction — the D-2 boundary), bump `PHYSICS_MODEL_VERSION`, and bring all docs to truth (P10 landed; F7 unblocked pending redesign).

**Files:**
- Modify: `antenna-model/src/service/evaluator.rs` (`off_axis_unvalidated_warning` message + the two constants' doc comments)
- Modify: `antenna-model/src/model/mod.rs` (`PHYSICS_MODEL_VERSION` bump)
- Modify: `docs/domain-contract.md` (off-axis section: numerical caveat resolved; F7 unblocked), `docs/api-documentation.md` (off-axis caveat: no longer "do not use"—now "levels not calibrated-grade"), `docs/roadmap-2026-07.md` + `docs/roadmap-2026-07-work-units.md` (P10 → DONE; F7 → unblocked/redesign-pending)
- Test: `evaluator.rs` off-axis warning tests (wording), `PHYSICS_MODEL_VERSION` mismatch test (already exists via P1b)

**Acceptance Criteria:**
- [ ] The off-axis warning no longer says "NUMERICALLY INVALID"/"20–35 dB TOO HIGH"; it states the value is now numerically converged but idealised (levels not calibrated-grade: blockage/strut/edge-diffraction unmodeled), pointing at calibration data / ITU-R S.580 for regulatory use. Message stays constant per (antenna, frequency) for heatmap/H3 dedup.
- [ ] The `evaluator.rs` unit test asserting "NUMERICALLY INVALID" is updated to assert the new physical-incompleteness wording (and that the stale numerical wording is gone).
- [ ] `PHYSICS_MODEL_VERSION` is bumped (off-axis `gain_physics` changed for identical inputs) with a doc-comment note referencing P10; the existing mismatch-warns test still passes.
- [ ] `docs/domain-contract.md` "Off-axis pattern / sidelobe fidelity": the aliasing/numerical-caveat and the P10 parked note are marked resolved; F7 marked unblocked, redesign pending D-2.
- [ ] `docs/api-documentation.md` off-axis caveat rewritten from "off-axis gain is NUMERICALLY INVALID / do not use" to the accurate post-P10 statement.
- [ ] Roadmap register row P10 → **Done**; F7 row → **Unblocked (redesign pending, D-2)**.

**Verify:** `cargo test -p antenna-model --lib off_axis && cargo test -p antenna-model --test '*' off_axis && cargo test --workspace` → PASS.

**Steps:**

- [ ] **Step 1: Update the failing test first** — flip the `evaluator.rs` assertion to the new wording:

```rust
assert!(msg.contains("not calibrated-grade") || msg.contains("idealised physical optics"),
    "post-P10 message must describe physical incompleteness: {msg}");
assert!(!msg.contains("NUMERICALLY INVALID"), "stale numerical-invalidity wording must be gone: {msg}");
assert!(msg.contains("ITU-R S.580"));
```

- [ ] **Step 2: Run to verify fail** — old message still says NUMERICALLY INVALID → FAIL.

- [ ] **Step 3: Rewrite the message + constants doc comments** (state numerically converged; levels idealised; unmodeled blockage/strut/diffraction; point to calibration/S.580). Bump `PHYSICS_MODEL_VERSION` in `model/mod.rs`. Update the four docs.

- [ ] **Step 4: Run to verify pass** — `cargo test --workspace` → green.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat(P10): off-axis warning numerical→physical; bump PHYSICS_MODEL_VERSION; docs truth (P10 done, F7 unblocked)"`

---

## Self-Review notes (author)

- **Spec coverage:** D-1 (Task 2), D-2 (Task 6 warning + roadmap F7 unblock; substitution deferred), D-3 completion (Task 6), D-4 (Task 5), D-5 (Tasks 2+5), D-6 (Task 3), validation protocol (Task 4), version bump (Task 6). Interim D-3 already shipped separately.
- **Bessel risk:** Task 0 is standalone and the highest-leverage correctness gate; its both-branch tests are the guard against the "confidently wrong" failure the spike hit.
- **2D reference availability:** near-in agreement is validated on SMALL dishes (2D trustworthy); far-off correctness is validated against the brute-force ground truth (large dish, θ=90°, `#[ignore]`d) + self-consistency (N-vs-2N, M-vs-M+1). This is deliberate — for large dishes off-axis the 2D cannot converge, so it is not a valid reference there.
- **Type consistency:** `azimuthal_mode_field` / `hankel_radial_field` / `radial_points_for` / `aperture_plane_g` / `IntegrationParams::adaptive()` names are used consistently across Tasks 1–5.
- **Frozen invariant:** the public signature of `integrate_aperture` is unchanged throughout (per the spike), so evaluator/cache/heatmap/H3 need no structural change — only the service param constructor swap in Task 5.
