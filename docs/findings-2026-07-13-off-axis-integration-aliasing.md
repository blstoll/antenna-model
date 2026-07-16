# Finding (P0, 2026-07-13) — the served off-axis pattern is numerically invalid (aperture-integral aliasing)

**Severity: P0 correctness.** Every off-axis gain this API serves beyond a few degrees is
wrong — typically **20–35 dB too HIGH**. This is **pre-existing** (it predates F7) and it
affects `/gain`, `/gain/batch`, `/heatmap`, and `/h3-heatmap` alike.

Discovered while reworking F7 after the 2026-07-12 code review. It invalidates F7's founding
premise and is the reason unit **F7 is PARKED** (see `roadmap-2026-07.md`).

---

## 1. What is wrong

The service computes every gain with `IntegrationParams::fast()`
(`service/evaluator.rs`, `service/h3_link_budget.rs`). The far-field aperture integral
carries a phase term whose variation across the aperture is

```text
Δφ ≈ 2π · (D/λ) · sin θ      [radians, across the aperture radius]
```

For an electrically large dish at appreciable θ this is enormous — e.g. D/λ = 953
(DSN 34-m at X-band) at θ = 90° gives Δφ ≈ 6000 rad. `fast()`'s grid cannot resolve
anything like that, so the integrand is **under-sampled and aliases**: the quadrature picks
up spurious *coherent* contributions instead of the true cancellation, and |I|² comes out
far too large.

The signature is unmistakable — the aliased pattern is roughly **flat at a high value**,
independent of angle, instead of falling.

## 2. Evidence

### 2.1 Synthetic 10 m dish, X-band (D/λ = 280)

| θ | `fast()` (what the API serves) | `high_accuracy()` | Δ |
|---:|---:|---:|---:|
| 1° | +19.32 | +19.32 | 0.0 |
| 5° | **+23.51** | −7.54 | **31.1** |
| 10° | **+24.03** | −10.68 | **34.7** |
| 20° | +5.27 | −19.30 | 24.6 |
| 40° | **+27.61** | +1.23 | 26.4 |
| 60° | +24.72 | −1.30 | 26.0 |
| 90° | **+16.61** | −5.12 | 21.7 |
| 163° | **+23.28** | +8.43 | 14.9 |

The main beam (θ ≤ ~1–2°) is fine. Everything past it is garbage.

### 2.2 Real served antennas (loaded from `calibration_data/antennas.yaml`)

`gs_3.7m_uncalibrated`, X-band (D/λ = 104):

| θ | `fast()` | `high_accuracy()` |
|---:|---:|---:|
| 0° | 48.04 | 48.04 |
| 1° | 23.91 | 23.91 |
| 5° | −1.02 | −0.92 |
| 20° | **+5.54** | −19.78 |
| 90° | **−3.66** | −27.00 |

`dsn_34m_uncalibrated`, X-band (D/λ = 953) — the worst case:

| θ | `fast()` | `high_accuracy()` |
|---:|---:|---:|
| 0° | 68.96 | 68.96 |
| 1° | **+33.70** | +14.53 |
| 5° | **+40.37** | +18.49 |
| 20° | **+34.89** | +15.61 |
| 90° | **+34.12** | +12.80 |

A 34-metre dish reporting **+34 dBi at 90° off-boresight** — i.e. beside/behind itself —
is physically impossible. Note also that `fast()` reports *more* gain at 5° than at 1°.

### 2.3 `high_accuracy()` does not rescue the large dishes

For D/λ = 953 even `high_accuracy()` yields +12.8 dBi at θ = 90°, which is still far too
high. This was already written down — `docs/domain-contract.md`, "Numerical caveat" — but had
never been connected to the *served* code path.

### 2.4 Convergence + cost sweep — why brute force is NOT the answer

**Read this before attempting to fix P10 by raising the integration density.**

DSN 34-m, X-band (D/λ = 953), θ = 90°. Aperture phase spans **5986 rad** (≈950 cycles), so
Nyquist requires `N_rho ≥ 2·(D/λ)·sinθ ≈ 1905`. Measured (release build, single evaluation):

| N_rho × N_phi | gain (dBi) | ms/call |
|---|---:|---:|
| 64 × 128 | +32.21 | 3.6 |
| 128 × 256 | +19.83 | 14.3 |
| 256 × 512 *(≈ `high_accuracy()`)* | +14.98 | 56.0 |
| 512 × 1024 | +11.62 | 197.8 |
| 1024 × 2048 | +1.24 | 788.6 |
| **2048 × 4096** *(first grid past Nyquist)* | **−33.28** | **3184.5** |

1. **It never converges on the way up — every coarse grid is aliased.** The value falls at
   *every* refinement (+32 → +20 → +15 → +12 → +1 → −33 dBi) and only becomes physical once
   `N_rho` crosses Nyquist. There is no plateau to stop on: any grid below Nyquist returns
   confident nonsense. `fast()` and `high_accuracy()` are both far below it.
2. **The converged answer costs ≈ 3.2 s for ONE gain evaluation** — versus the **< 100 ms p95**
   budget (~32× over). And 2048 is merely *at* Nyquist; Simpson's rule needs several samples
   per cycle for accuracy, so genuine convergence needs 4096–8192 → **12–50 s/call**
   (120–500× over budget).
3. **Cost is quadratic in `(D/λ)·sinθ`** — the clean 4×-per-doubling column confirms
   `cost ∝ N_rho·N_phi ∝ (D/λ·sinθ)²`. This is structural, not a tuning knob. Extrapolating to
   the worst *enabled* antenna, **GBT 100-m at Q-band** (D/λ ≈ 16 000, Nyquist `N_rho` ≈ 32 000):
   roughly **13 minutes per point**. `/heatmap` fans out to 100 000 points.

**Corollary — why this hid for so long:** at θ = 0 the phase term vanishes entirely, so the main
beam is both *cheap* and *correct*. That is the only regime the peak-gain harness ever exercised,
and it is exactly the regime where `fast()` is trustworthy.

## 3. Why it was never caught

A **test/production integrator gap**:

- The reference harness validates off-axis *shape* with `high_accuracy()` on the **small**
  3.7 m dish (`reference_validation::itu_r_s580_sidelobe_envelope_small_dish`). That is the
  one configuration where the integral is still trustworthy, so the test passes.
- Production serves **`fast()`** on dishes up to 100 m.

Nothing exercised the combination that users actually hit.

## 4. Consequences

1. **Off-axis gain is unusable** on every endpoint. Interference, off-axis-EIRP, ACI and
   adjacent-satellite analysis are all invalid — and so is any *desired-signal* margin
   computed off-boresight.
2. **`/h3-heatmap` is hit hardest.** Its cells are, by construction, spread over angle, and
   its `loss_db` / `total_path_loss_db` / `g_over_t_db` are all derived from these gains.
   Its `boresight_gain_db` reference is itself evaluated toward the centre cell (not θ = 0),
   so it is aliased too.
3. **The P8 off-axis warning understates the problem.** It says the value is "not validated."
   The truth is stronger: it is *numerically invalid*.
4. **F7's premise is inverted.** The contract's claim that modelled sidelobes are
   "systematically optimistic (~8–13 dB *below* the ITU-R S.580 mask)" was measured with
   `high_accuracy()` on the small dish. On the **served** path the pattern is 20–35 dB
   **above** reality. A floor that only ever *raises* gain therefore cannot help: applied as
   `max(pattern, floor)` against a spuriously-high pattern it **never fires**. Confirmed
   empirically — the F7 floor engaged in **0 of 6** real service geometries tested.

## 4a. SPIKE RESULT (2026-07-13) — the fix is a contained refactor, not a rewrite

**We are using the wrong algorithm, and the right one is ~3200× faster *and* correct.**

The azimuthal integral has a closed form. `phase_path` (`phase.rs:96`) is

```text
Ψ = k·[ ρ²/(4f)·(1−cosθ)  −  ρ·sinθ·cos(φ−φ′) ]
        └── term1 ──┘        └──── term2 ────┘
```

`term2` is *exactly* the 2D Fourier kernel (`u=sinθcosφ, v=sinθsinφ, x=ρcosφ′, y=ρsinφ′`
⇒ `term2 = −k(ux+vy)`), and **everything else is a pure aperture-plane function** —
`phase_feed_displacement` (coma, axial defocus) takes no θ/φ at all. So by Jacobi–Anger

```text
∫₀²π exp(−j·a·cos(φ−φ′)) dφ′ = 2π·J₀(a),      a = k·ρ·sinθ
```

and for an azimuthally symmetric aperture the 2D integral **collapses to a 1D Hankel
transform**:

```text
I(θ) = 2π ∫₀^R A(ρ)·exp(j·k·ρ²/(4f)·(1−cosθ))·J₀(k·ρ·sinθ)·ρ dρ
```

At Nyquist that is ~4k integrand evaluations instead of 2048×4096 ≈ 8.4 M.

### Cross-validation (dsn_34m, X-band, D/λ = 953)

| θ | 2D (2048×4096) | Hankel | Δ |
|---:|---:|---:|---:|
| 0° | 68.96 | 68.96 | **0.00 dB** |
| 1° | 14.53 | 14.53 | **0.00 dB** |
| 5° | −9.39 | −9.39 | **0.00 dB** |
| 20° | −23.55 | −23.56 | **0.00 dB** |
| 90° | **+1.24** *(2D still aliased)* | **−33.28** | — |

Exact agreement everywhere the 2D is trustworthy. At θ = 90°, where the 2D is aliased even at
8.4 M points, the Hankel converges cleanly and **independently reproduces the −33.28 dBi
brute-force ground truth of §2.4**:

| N_rho | gain (dBi) | ms |
|---:|---:|---:|
| 2049 | −32.61 | 0.13 |
| 4097 | **−33.28** | 0.26 |
| 8193 | −33.30 | 0.51 |
| 16385 | −33.30 | 0.99 |

### Cost

| | gain @ 90° | time |
|---|---:|---:|
| Today's `fast()` | **+34** (garbage) | 1.2 ms |
| Brute force, converged | −33.28 | **3184 ms** |
| **Hankel, converged** | **−33.30** | **~1 ms** |

**~3200× faster than the correct brute-force answer, and ~5× faster than the *wrong* answer we
ship today.** Crucially the complexity class changes: radial Nyquist is `N_rho ≈ 2·(D/λ)·sinθ`,
so cost is **O(D/λ)**, not **O((D/λ)²)**. The GBT 100-m Q-band worst case drops from the
~13 min/point of §2.4 to roughly **2 ms**.

### Consequences for P10

- **Contained refactor.** Swap the inner quadrature in `integrate_aperture`; the signature is
  unchanged, so evaluator / cache / heatmap / h3 are untouched.
- The **<100 ms budget stops being a constraint** — correct off-axis gain is *cheaper* than the
  current incorrect gain.
- The dish-depth chirp (`term1`) needs no special handling; the radial quadrature resolves it at
  the same N.

### Open unknown (do this first in P10)

The spike covers the **azimuthally symmetric** case only (feed at focus, no coma, no mesh). A
laterally displaced feed breaks the symmetry. The generalisation is the standard azimuthal-mode
expansion — expand the aperture phase in `e^{jmφ′}` and each mode yields `2π(−j)^m J_m(a) e^{jmφ}`
— textbook, but **not yet demonstrated here**. Establish how many modes realistic coma needs.

### Reproduce

```bash
cargo test --release -p antenna-model --test reference_validation p10_spike -- --ignored --nocapture
```
(`reference_validation::p10_spike_hankel_vs_2d`, `#[ignore]`d because the 2D reference legs take
~0.4 s each.)

**Method warning, learned the hard way:** the spike initially used wrong Bessel `J₀` small-argument
coefficients and produced a *confidently wrong* 22 dB error at θ = 0 — while still looking perfect
at θ = 90° (which takes the asymptotic branch). It was caught only because θ = 0 has an
independently known answer. Any P10 implementation must be cross-checked at angles where the
answer is already known; an oscillatory integrator that is wrong is not obviously wrong.

## 5. What F7 should become (once this is fixed)

Not a `max()` floor bolted onto a broken pattern, but a **replacement**:

1. Derive the aperture integral's **angular validity limit** θ_valid(D/λ, grid density) —
   the angle beyond which the phase term is under-sampled.
2. **Inside** θ_valid: use the physics pattern (it is accurate there; the S.580 shape test
   validates it).
3. **Beyond** θ_valid: **substitute** the data-calibrated statistical off-axis model — do not
   `max()` with the aliased pattern, which must not be trusted at all out there.
4. Refuse, or warn hard, where neither is defensible.

## 6. Salvage from the parked F7 branch (`feat/f7-sidelobe-floor`)

Still good, and reusable by the redesign:

- **The corrected floor derivation.** Ω = **4π (isotropic)** is the only power-conserving
  choice, because the floor is applied over the whole sphere. The level collapses to
  `floor_linear = p_scatter = 1 − η_ruze` — no free constant. Two consequences: it is
  **bounded by 0 dBi** (so it can never swamp a main beam), and it **tracks the NTIA 84-164
  wide-angle median** to within ±6 dB per bin / ~2.5 dB band-mean. Pinned by
  `reference_validation::sidelobe_floor_tracks_measured_median`, which also asserts power
  conservation and the 0 dBi ceiling. **The shipped-then-reverted Ω = 0.25 sr was wrong**: a
  cone-derived level applied across 4π, implying 136–326% of the antenna's total radiated
  power.
- **Honest scope for the level**: `(1 − η_ruze)` works as a *surface-quality scaling term*
  carrying unmodelled spillover/blockage/diffraction — it is **not** a literal power budget.
  Evidence: Ruze scales as (rms/λ)² while the measured floor is nearly frequency-flat.
- The `apply_sidelobe_floor` flag, the uncalibrated gate, the `PHYSICS_MODEL_VERSION` stamp,
  and the digitised NTIA/NASA reference datasets.

## 7. Also open (from the 2026-07-12 review, independent of the above)

> **TRACKED 2026-07-15** (post-P10 assessment — these two items had been recorded here but
> never promoted to work units): item 1 is now unit **P11** (unified predicate); item 2 is now
> an explicit **precondition in the F7-redesign unit** (bound/document the tuner coupling
> before any floor ships). See `roadmap-2026-07-work-units.md`.

- **Gate/warning predicate mismatch.** The spillover + floor gate on
  `correction_surface.is_none()`, but the P8 warning gates on
  `CalibrationStatus::Uncalibrated`. These are different sets:
  `calibrate/src/boresight_calibration.rs:637,642,687` produces `PartiallyCalibrated` with
  **no** correction surface whenever there is no frequency correction. Such an antenna has
  its physics modified while serving only a "±1–1.5 dB" accuracy claim, with no off-axis
  warning. Should become **one predicate on the calibration**.
- **Boresight tuner couples into off-axis power.** `calibrate/src/boresight_calibration.rs`
  tunes `surface_rms` as a catch-all for boresight gain deficits; any floor keyed on
  `(1 − η_ruze)` then converts that inflated σ into off-axis power. Bounded by the 0 dBi
  ceiling, but still a coupling to document.

## 8. Reproducing

```bash
# Compare the two integrators at increasing theta for a served antenna.
# (Probe code lived in tests/reference_validation.rs; see this doc's tables.)
cargo test -p antenna-model --test reference_validation -- --nocapture
```

The essential check is to call `compute_gain_db(theta, 0.0, &config, f, &params)` for
`IntegrationParams::fast()` vs `::high_accuracy()` at θ ∈ {1°, 5°, 20°, 90°} on any antenna
with D/λ ≳ 100, and observe the divergence.
