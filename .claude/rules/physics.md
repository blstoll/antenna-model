---
description: Invariants and validation protocol for the physical-optics engine — integrator, Bessel, FFT, phase/coma.
paths:
  - "antenna-core/src/model/**"
  - "antenna-model/benches/**"
  - "antenna-model/tests/reference_validation.rs"
---

# Physics engine rules

## A wrong oscillatory integrator is not obviously wrong

It returns a plausible number. Any change to `integration.rs` or `bessel.rs` must be
cross-checked at angles whose answers are independently known, spanning the full θ range
**and both Bessel branches** (small-argument and asymptotic): a P10-era spike was
confidently wrong by 22 dB at θ=0 while looking flawless at θ=90°, because special-function
bugs fail branch-locally.

The validation protocol lives in `antenna-model/tests/reference_validation.rs` (anchors,
independent Hankel oracle, physicality sweeps, and since P12 the mode-path
radial-convergence anchors + symmetric control) — **run it, and never validate at a single
angle**. **Cross-check against a method that is not the one you are changing**: P12's
`p2_moderate_offset` pin moved 2.3 dB and only the 2D Simpson oracle could show that *both*
the old and new values were ~29 dB wrong for an unrelated reason.

**Never buy speed by reducing sample density.** P10-perf got 2.4–7.4× without touching a
single sample count, by making each sample cheaper. The remaining cost is ~85% aperture-plane
function evaluation, so that is where the next win is, not in the quadrature.
Counter-intuitively, cost and convergence are **anti-correlated** here: every geometry
measured with a radial error was sub-millisecond, while the 300 ms–3.7 s Ka cases were
already accurate to ±0.02 dB.

## Two more standing rules

- **Phase wrapping:** phase functions must handle 2π wrapping correctly (`model/phase.rs`).
- **Feed offset sign conventions:** coma lobe direction depends on feed displacement sign;
  follow the right-hand rule.

## Key modules

- **`coordinates.rs`** — ECEF ↔ Geodetic ↔ Antenna Frame ↔ Spherical transforms.
- **`coordinates_3d.rs`** — 3D position → antenna-frame direction transforms.
- **`correction_interpolator.rs`** — 4D B-spline evaluation of the residual correction surface.
- **`illumination.rs`** — feed pattern: cos^q with q-factor.
- **`mesh.rs`** — mesh transparency (wire-mesh reflection efficiency). Surface RMS / Ruze
  efficiency lives in `pattern.rs`, not here.
- **`pattern.rs`** — far-field pattern with Ruze efficiency and the Huygens obliquity factor
  `(1+cosθ)/2` (F7, `absolute_gain_from_integral`).
- **`phase.rs`** — path length, coma (full path-length model), surface error (statistical Ruze;
  per-point Zernike maps are **not** implemented — the aperture integrand uses
  `surface_error = 0.0` and the calibration correction surface absorbs systematic surface
  deviations), mesh.
- **`edge_cases.rs`, `ray_trace.rs`** — special cases / large feed offsets (>0.5f).

### `integration.rs` — the Hankel / azimuthal-mode (Jₘ) integrator

Landed as roadmap P10 (2026-07-15): the φ' integral is collapsed analytically (Jacobi–Anger),
radial density is derived adaptively from `(D/λ, θ)` at ~2× Nyquist, and runtime self-checks
flag non-convergence (surfaced as a response warning).

**Both branches verify BOTH axes** (P12, `PHYSICS_MODEL_VERSION` 6). Until P12 the asymmetric
branch sized `n_rho` once and self-checked only mode truncation, so on a laterally-offset or
`asymmetry_factor != 1.0` feed — **five of the enabled feeds** — `converged = true` asserted
nothing about the radial quadrature. Measured silent errors up to **7.08 dB**; all now within
0.013 dB. The mechanism was that the mode path returned the coarse leg and never checked,
while the answer is a residue of mode integrals that cancel 59–111×, so per-mode errors of
~1% become ~10% of the result. The mode path now returns the **fine (2N) leg** and refines
until converged (`MAX_RADIAL_REFINEMENTS`).

**`PHYSICS_MODEL_VERSION` 8 (P13) deleted the `{0,1}`-mode radial pre-gate**, along with
`RADIAL_PROBE_MODES`, `RADIAL_PRE_GATE_SAFETY`, `FULL_RADIAL_CHECK_WORK_LIMIT` and
`radial_probe_field`. There is now exactly one radial shape for every geometry. The reason
to remember: the safety factor **stopped bounding its quantity because of a change with no
physics content** — P10-perf's `next_fast_len` φ' resizing (512 → 270) moved the worst
*passing* probe-to-total ratio from 26× to **43.5×** against a constant of 32, with nothing
in the build able to notice. **Do not reintroduce a fitted numeric guard on this path without
a test that asserts its margin** — that absence is what let this one rot silently.

**The φ' cap is sized from physics, not fitted** (`PHYSICS_MODEL_VERSION` 7).
`MODE_PHI_STEERED_MAX` used to clamp `n_phi` to 64 on steered feeds, aliasing high modes into
`g₀` — measured **+82 dB** wrong on a routine ~5° beam steer, with `converged = true`, because
neither existing check can see φ' aliasing. `n_phi` is now sized from the azimuthal bandwidth
`B = k·δ·(R/f)`, rounded up to the next even 5-smooth FFT length, `MODE_PHI_MAX` = 2048, and
`ModeSizing::azimuthally_resolved` gates `converged` when the ceiling binds. An effort ceiling
remains but is keyed to `SEVERE_OFFSET_THRESHOLD` (0.5f) — the model's own PO scope boundary —
instead of an arbitrary 0.05. Its sibling `MODE_RADIAL_CYCLE_CAP` was re-keyed the same way
(now `BEYOND_SCOPE_COMA_CYCLE_CAP`).

**The rule both caps follow: size from the physics inside the model's scope, cap effort
outside it, never be silent about which.** The old constants capped *inside* scope and did it
silently — the threshold, not the mechanism, was the defect. That exposed
`m_theta = k·R·sinθ + 6`: `Jₘ` has an Airy turning point at `m = x` with transition width
`~x^(1/3)`, so a flat `+6` truncated live spectrum (+0.49 dB at θ=3°); it is now
`x + 4·x^(1/3) + 6`.

The φ' axis has exactly one automatic guard,
`served_n_phi_sizing_is_sufficient_on_every_asymmetric_geometry` — **do not weaken it.**
The legacy 2D Simpson quadrature survives only as a `#[cfg(test)]` reference oracle. The
`IntegrationParams` presets (`fast()`, `high_accuracy()`) no longer gate served correctness —
the served path uses `adaptive()` and most preset fields are inert.

See `docs/findings-2026-08-01-p13-pre-gate-retirement.md` and
`docs/findings-2026-07-31-p12-mode-path-radial-budget.md`.

### `bessel.rs` — in-house Bessel Jₘ

Pinned by tests in every branch, across the **turning point** `m ≈ x`, and — since P14
(`PHYSICS_MODEL_VERSION` 9) — against an **independent oracle**: a compensated trapezoidal
quadrature of `Jₘ(x) = (1/2π)∫₀^{2π} cos(mτ − x sinτ)dτ`, which shares no machinery with the
recurrences. **Add that oracle to any Bessel change**: the module's other graders are
recurrence identities, and *an identity is scale-invariant* — a uniformly mis-normalized
Miller result satisfies it exactly, which is the one way Miller's algorithm actually fails.

`bessel_jn_array` returns every order `J_0…J_{m_max}` from a single sweep and is what the mode
integrator uses — **do not mix it with per-order `bessel_jn` calls on the same path**, since
the two select their recurrence direction from different orders.

P14 closed the accuracy cliff at `m ≈ x` (was 2e-8 at x=255, 9e-3 at x=10⁴, growing without
bound in x; now ~3e-16 flat) by making the Miller start offset scale with the turning-point
width, `12·x^(1/3)`, where the 12 is **derived** from an Airy decay requirement rather than
fitted — and `miller_start_offset_has_real_margin` asserts that constant's margin *directly*.

**Two accuracy floors remain, both deliberate and pinned:** `bessel_j0`/`bessel_j1` above
|x| = 8 are the Numerical Recipes rational fit at ~3e-9 absolute (below |x| = 8, the convergent
series, ~1e-14), and that ceiling propagates to every order the *upward* branch produces; and
a renormalized downward sweep is accurate to ~ε·(largest Jₘ in the sweep) in **absolute**
terms. Chasing either one relatively is asking a normalized recurrence for something it cannot
give.

### `fft.rs`

Mixed-radix (2/3/5) forward FFT backing the integrator's φ' transform (P10-perf).
Crate-internal, forward-only, deliberately not a general FFT crate. `next_fast_len` rounds up
to the next even 5-smooth number — **not** a power of two, because the padding is paid in
aperture-plane evaluations (536 → 540 costs 0.7%; 536 → 1024 would cost 91%). Validated
against a literal DFT transcription at every fast length the integrator can ask for, not
spot-checked.

## Coma aberration model

Feed displacement uses a **full path-length model**: the exact geometric path difference
between the path from the ideal focal point to each aperture point and the path from the
displaced feed position. This naturally includes beam steering (θ ≈ δ/f), defocus/astigmatism,
true coma with asymmetric sidelobes, and higher orders — more accurate than linear
approximations, especially for offsets >0.1f.

**No separate higher-order aberration mode (roadmap P2).** Because `phase_feed_displacement`
is the *exact* geometric path difference, it already carries the complete low-order aberration
content as an exact function of the displacement. The former `HigherOrderAberrations` mode
(0.3f–0.5f) added heuristic Seidel terms *on top of* that exact phase — a double-count, with
wrong-sign/wrong-scale coefficients (it coded ρ³ distortion where the exact model and classical
theory give leading ρ¹). It was removed; 0.3f–0.5f offsets route through
`StandardPhysicalOptics`. The pin
`edge_cases::exact_feed_displacement_phase_contains_all_low_order_aberrations` proves the exact
phase's content against an independent closed form. Offsets >0.5f route to the ray-tracing stub
(P3). This is why `PHYSICS_MODEL_VERSION` is 4.

## Physics references

- **Antenna Theory**: Balanis — reflector antenna chapters
- **Ruze Equation**: J. Ruze, "Antenna Tolerance Theory" (1966) — surface error effects
- **Zernike Polynomials**: Noll — standard ordering
- **Mesh Reflectors**: wire mesh EM scattering literature
- **Numerical**: Jacobi–Anger / Hankel transforms for the azimuthal collapse; composite
  Simpson for the radial quadrature; mixed-radix Cooley–Tukey FFT for the φ' coefficients;
  Bessel Jₘ rational approximations and recurrences, including Miller's downward recurrence
  (Press et al., "Numerical Recipes"; Abramowitz & Stegun for reference values)
