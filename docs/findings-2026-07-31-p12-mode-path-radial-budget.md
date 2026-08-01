# P12 task 1 — why the azimuthal-mode path's radial budget is insufficient

**Date:** 2026-07-31
**Unit:** roadmap **P12** — "The azimuthal-mode path never checks radial convergence"
([`roadmap-2026-07-work-units.md`](roadmap-2026-07-work-units.md), P12 task 1)
**Status:** investigation complete; **no production code changed**. This document is the
mechanism report P12 task 1 requires *before* any constant is touched.

P12 task 1 reads:

> **Establish the mechanism for (b) before changing any constant.** Instrument the per-mode
> radial integrand at the `gs_3.7m` θ=5° point; determine what radial content
> `radial_points_for` is not counting. Report it — a wrong budget formula is a finding in its
> own right.

**Answer: `radial_points_for` is not failing to count anything.** Its cycle count is correct
and in fact ~30% conservative. The budget is short for a different reason, and the framing in
the unit needs revising as a result — including both of its open decisions.

---

## 0. The instrument

Six `#[ignore]`d diagnostics in `antenna-model/src/model/integration.rs`, module
`p12_radial_diagnostic` (they must live inside that file — `radial_points_for`,
`azimuthal_mode_field_inner`, `aperture_plane_g` and `AperturePlaneConst` are private to it):

```
cargo test --release -p antenna-model --lib p12_ -- --ignored --nocapture
```

They print; they assert nothing. The one gating test in the module,
`per_mode_decomposition_reproduces_the_integrator`, is **not** `#[ignore]`d: the diagnostics
need a per-mode decomposition that `azimuthal_mode_field_inner` does not expose, so the module
replicates its radial loop, and that test pins the replica to the real integrator's total
(`rel < 1e-12`). If it ever fails, every number below is void.

Geometries are transcribed from the real configurations, not from the nearby test helpers:
`gs_3.7m_uncalibrated`/`x_band_feed` and `dsn_34m_uncalibrated`/`x_band` from
`calibration_data/antennas.yaml`, and D12's `UHF_Array_Element` from
`calibrate/tests/support/mod.rs` / `calibrate/antenna_classes.yaml`.

Everything below is measured at the **field** level with `n_phi` and `m_max` held fixed, so
every delta is purely radial. Field dB and gain dB differ only by factors common to both legs.
The instrument reproduces the two rows P12 filed with a stated φ: **−0.8157 dB** (`gs_3.7m`)
and **−1.1671 dB** (`dsn_34m`), against 0.82 and 1.17 dB filed.

---

## 1. No radial content is missing (the filed hypothesis is falsified)

P12 flagged a candidate explicitly: *"it may be that the per-mode integrand `gₘ(ρ)·Jₘ(kρ sinθ)`
carries radial content the m=0 budget does not model."*

Measured, at `gs_3.7m` θ=5°, by Hann-windowed DFT of the actual per-mode radial integrand
`Fₘ(ρ) = exp(j·k·ρ²/(4f)·(1−cosθ))·gₘ(ρ)·Jₘ(kρ sinθ)·ρ` sampled at 4096 points:

| m | 99% BW | 99.9% BW | 99.99% BW |
|---|---|---|---|
| 0 | 7 | 7 | 8 |
| 1 | 7 | 7 | 8 |
| 2 | 7 | 7 | 7 |
| 3 | 6 | 7 | 7 |
| 4 | 6 | 7 | 7 |
| 5 | 6 | 7 | 7 |

(cycles across `[0, R]`; the Hann window matters — `Fₘ` is not periodic on `[0,R]`, the
illumination taper leaves ≈ −11 dB at the rim, and an unwindowed DFT would manufacture
bandwidth that is a transform artifact.)

`radial_points_for` predicts **10.486** cycles for this point (kernel 9.036 + coma 1.401 +
defocus 0.000 + chirp 0.049). The true content is **7–8**. The budget **over**-estimates by
~30%; nothing is missing from it.

## 2. 4 samples/cycle is not universally too coarse either — the control

Same dish, same θ, feed moved to the focus so it routes to the **symmetric** branch (one mode
instead of a sum), at the same budget density:

| θ | n_rho | pts/cycle | N leg vs converged | 2N leg vs converged |
|---|---|---|---|---|
| 0.5° | 17 | 18.78 | +0.0001 dB | +0.0000 dB |
| 2° | 17 | 4.69 | −0.0103 dB | −0.0006 dB |
| **5°** | **37** | **4.07** | **+0.0432 dB** | +0.0025 dB |
| 20° | 145 | 4.00 | +0.0066 dB | +0.0004 dB |

At **4.07** samples/cycle the symmetric branch's coarse leg is **0.043 dB** off. The mode path
at **4.10** samples/cycle, same dish, same angle, is **0.816 dB** off — 19× worse at the same
density. The samples-per-cycle constant is not the discriminator.

## 3. The mechanism: the budget sizes for *resolution*, but accuracy is set by *cancellation*

`radial_points_for` answers "how many samples resolve this integrand?" — and answers it
correctly. What it never looks at is how much the integral **cancels**: the returned value is a
small residue of a much larger integrand, so a quadrature error that is tiny relative to the
integrand is large relative to the answer. That amplification factor is where the mode path's
accuracy actually lives, and no term in the formula references it.

It shows up in two distinct forms, and P12's three rows contain both.

### 3a. Inter-mode cancellation (`gs_3.7m`, `dsn_34m`)

At `gs_3.7m` θ=5°, `n_rho = 43`:

```
|I| = 7.962e-4;  Σ|R_m| = 8.861e-2;  cancellation ratio Σ|R_m|/|I| = 111.3×
```

| m | \|Rₘ\|/\|I\| | per-mode error, % of \|Rₘ\| | … % of \|I\| |
|---|---|---|---|
| 0 | 8.20 | 0.9198 % | 7.540 % |
| 1 | 3.42 | 1.3258 % | 4.530 % |
| 2 | 16.28 | 0.0569 % | 0.925 % |
| 3 | 13.96 | 0.1088 % | 1.520 % |
| 4 | 8.25 | 0.1168 % | 0.963 % |
| 5 | 26.75 | 0.1297 % | 3.470 % |
| 6 | 2.97 | 0.1656 % | 0.492 % |
| 7 | 20.06 | 0.1110 % | 2.227 % |

Every mode's own radial integral is accurate to **0.06–1.3%** — the budget did its job. But the
modes cancel to a residue **111× smaller** than the sum of their magnitudes, so those same
errors are **0.5–7.5% of the answer**, totalling 9.202% ⇒ **−0.8157 dB**.

`dsn_34m` θ=0.10° is the same story at ratio **58.9×** (m=0 error 6.25% of its own mode →
12.3% of the total; 16.371% ⇒ **−1.1671 dB**).

Note the per-mode **magnitude**-ratio columns in the convergence-ladder diagnostic (±0.1 dB)
badly understate this: a mode whose magnitude is perfect but whose phase is off by 0.02 rad
contributes 0.02·|Rₘ| of error, which is enormous when |Rₘ| is 27× |I|. The complex-error
column above is the honest one.

### 3b. Intra-mode cancellation (D12's UHF fixture)

The UHF fixture has **no lateral feed offset at all** — it reaches the mode path purely through
`asymmetry_factor = 1.1`, so its coma budget is exactly zero. Its modes do **not** cancel
against each other: |Rₘ|/|I| = 0.721, 0.000, 0.357, 0.000, 0.016 (odd modes vanish, as the
cos 2φ′ illumination modulation implies). Σ|Rₘ|/|I| ≈ 1.1.

Yet at the served `n_rho = 19` the answer is **−7.08 dB** off (φ=0), with m=0's own radial
integral **13.8 dB** wrong. Here the cancellation is *within* a single mode: at D/λ = 16 and
θ = 16° the integrand oscillates and integrates down to a small residue. Same mechanism, one
level down.

### 3c. Why the symmetric branch escapes both

Not because its budget is better — it is the same budget. Because of what it *does with it*:
it computes at N **and** 2N, **returns the fine (2N) leg**, and **checks the difference**
(`integration.rs:519-523`; `self_check` returns `fine`). The mode path computes at N and
returns N (`:545-551`).

Applying that machinery unchanged to the mode path, at the same budget:

| geometry | served (returns N) | what the 2N leg returns | N-vs-2N check |
|---|---|---|---|
| `gs_3.7m` θ=5° | −0.8157 dB | **−0.0445 dB** | \|Δ\|/\|fine\| = 0.0872 ⇒ **converged=false** |
| `dsn_34m` θ=0.10° | −1.1671 dB | **−0.0553 dB** | \|Δ\|/\|fine\| = 0.1568 ⇒ **converged=false** |

Returning the fine leg removes ~95% of the error, and the existing 2% radial tolerance floor
correctly flags what is left. **The symmetric branch is not more accurate by design — it is
more accurate by a factor of two in delivered density, plus honest about the remainder.** The
mode path inherited the budget and neither the doubling nor the check.

---

## 4. This revises both open decisions

### D-B — "is `adaptive()`'s floor of 16 simply wrong?"

P12 records sub-defect (a) as "the floor is too low … it binds when `4·(D/λ)·sinθ < 16` and
costs >1 dB there (rows 2 and 3)", and recommends raising it to 32.

**Measured: the floor of 16 is not binding at any of the three rows.**

| geometry | budget asks | floor 16 | floor 32 | floor 64 |
|---|---|---|---|---|
| `gs_3.7m` θ=5° | 42 pts | n=43, **−0.8157 dB** | n=43, −0.8157 dB | n=65, −0.1356 dB |
| `dsn_34m` θ=0.10° | 28 pts | n=29, **−1.1671 dB** | n=33, −0.6228 dB | n=65, −0.0318 dB |
| UHF φ=0 θ=16° | 18 pts | n=19, **−7.0761 dB** | n=33, −0.5690 dB | n=65, −0.0339 dB |
| UHF φ=90 θ=16° | 18 pts | n=19, **−3.8546 dB** | n=33, −1.2285 dB | n=65, −0.0707 dB |

P12 already noted the floor was not binding at `gs_3.7m`. It is not binding at rows 2 or 3
either — the budget asks for 28 and 18 points, both above 16. So sub-defect (a) as filed is not
the mechanism *anywhere* in the measured set; there is really only sub-defect (b).

Raising the floor to 32 still improves rows 2 and 3, but **not by fixing a floor problem** — it
improves them by *overriding* the budget with a bigger number, and it leaves row 1 (the largest
served antenna case) completely untouched. It is a coincidence, not a fix, and even where it
helps it leaves 0.62 dB and 1.23 dB on the table. Consequences for the decision:

- The recommendation ("raise to 32 — cheap, independent, fixes rows 2 and 3") should be read as
  *partial mitigation at two of four measured points*, not as closing sub-defect (a).
- The side benefit P12 cites is real and unaffected: it closes D17's leftover `calibrate`
  (`default()`, floor 32) vs service (`adaptive()`, floor 16) preset divergence.
- If a floor is meant to carry real weight, **64** is what the measurements support (≤0.14 dB at
  every row) — but a floor is the wrong instrument for a defect that is not floor-shaped.

### D-A — the form of the radial check

Three things bear on the choice:

1. **The cheapest correct-ish option was not on the list.** Simply *returning the fine leg* —
   what the symmetric branch already does — takes `gs_3.7m` from 0.82 to 0.045 dB and
   `dsn_34m` from 1.17 to 0.055 dB. That is exactly the "full N-vs-2N" cost (option i, ~3× on
   this path) if you also keep the comparison, but a plain 2× density with **no** check would
   capture most of the accuracy at 2× cost. Worth pricing explicitly against options (i)–(iii).
2. **A subset check must anchor on m=0, and must not pick its second probe by |gₘ|.** The
   ranking by mode magnitude and the ranking by mode error disagree at the top:

   | geometry | top-5 by \|Rₘ\| | top-5 by error |
   |---|---|---|
   | `gs_3.7m` θ=5° | 5, 7, 2, 3, 4 | **0, 1**, 5, 7, 3 |
   | `dsn_34m` θ=0.10° | 4, 3, 6, 1, 5 | **0, 1**, 3, 4, 5 |

   In both, the two largest error contributors are m=0 and m=1, neither of which is in the top
   three by magnitude. The good news for option (ii) is that **m=0 is the single largest error
   contributor in all three geometries** (and in the UHF case is essentially the whole error),
   so an m=0-anchored check would fire everywhere measured. The bad news is that "plus the
   largest-|gₘ| mode" would add the wrong second probe.
3. **The UHF row argues the check cannot be optional.** −7.08 dB with `converged = true` is the
   largest silent error found in this unit, and it comes from a fixture whose feed is *at the
   focus* — i.e. from the `asymmetry_factor` door into the mode path, not the coma door.

The options are priced in §4a.

---

## 4a. D-A priced

Measured 2026-07-31, `--release`, min-of-N wall clock
(`p12_price_decision_d_a_options`, `p12_grade_decision_d_a_options`,
`p12_price_refine_until_converged`). N = `radial_points_for`, 2N = `radial_check_points(N)`,
subset = modes {0,1}.

### The cost structure D-A actually has

An operation count says a "check 2 of 195 modes" design costs 2/195 of a full sweep. That is
wrong. The φ' sweep evaluates `aperture_plane_g` at `n_rho × n_phi` points **regardless of how
many modes are requested**; only the inner accumulation scales with the mode count. So a subset
check is floored by the g-evaluation, and how binding that floor is depends on `m_max`:

| geometry | n_rho | n_phi | m_max | 2-mode sweep as % of full sweep |
|---|---|---|---|---|
| `gs_3.7m` X θ=5° | 43 | 64 | 20 | **52.0 %** |
| `dsn_34m` X θ=0.10° | 29 | 128 | 12 | **61.5 %** |
| D12 UHF θ=16° | 19 | 64 | 15 | **51.8 %** |
| `dsn_34m` Ka θ=5° | 1355 | 512 | 195 | **18.3 %** |
| `dsn_34m` Ka θ=90° | 16867 | 512 | 195 | **18.5 %** |

A subset check is cheap only where the mode count is large — i.e. **exactly on the Ka geometries
D-A is worried about**. On the small-`m_max` rows it saves almost nothing, but there the absolute
cost is sub-millisecond.

### Priced options

Multiples are of the current served cost; absolute ms are the measured baseline.

| option | `gs_3.7m` 0.42 ms | `dsn_34m` X 0.45 ms | UHF 0.17 ms | Ka θ=5° 299 ms | Ka θ=90° 3706 ms |
|---|---|---|---|---|---|
| baseline (return N, no check) | 1.00× | 1.00× | 1.00× | 1.00× | 1.00× |
| **(A)** fine leg only, return 2N | 2.08× | 1.97× | 1.96× | 2.00× | 2.00× — 7.41 s |
| **(i)** full N-vs-2N, return 2N | 3.08× | 2.97× | 2.96× | 3.00× | 3.00× — **11.12 s** |
| **(ii-a)** subset@2N check, return N | 2.03× | 2.22× | 2.02× | **1.36×** | **1.37× — 5.08 s** |
| **(ii-b)** subset@N check, return 2N | 2.60× | 2.59× | 2.48× | 2.18× | 2.19× — 8.10 s |
| **(iii)** 3N, checked in tests only | 2.94× | 3.01× | 3.13× | 3.00× | 3.00× — 11.13 s |

**Option (iii) is dominated and can be dropped.** A 3N fixed density costs the same 3.00× as the
honest N-vs-2N check and delivers strictly less (no runtime verdict). If the "validated margin"
were 2× instead of 3×, (iii) *is* option (A).

### The result that reframes the decision: cost and need are anti-correlated

D-A's premise is that an honest check is unaffordable because `dsn_34m` Ka already costs ~3.3 s.
Measured, **the Ka geometries do not need the check at all** — the served density is already
converged there:

| geometry | baseline cost | true error of the served N leg | subset check fires? |
|---|---|---|---|
| `gs_3.7m` X θ=5° | 0.42 ms | **−0.8157 dB** | **yes** (0.1249) |
| `dsn_34m` X θ=0.10° | 0.45 ms | **−1.1671 dB** | **yes** (0.1831) |
| D12 UHF θ=16° | 0.17 ms | **−7.0761 dB** | **yes** (1.6116) |
| `dsn_34m` Ka θ=5° | 299 ms | +0.0126 dB | no (0.0004) |
| `dsn_34m` Ka θ=90° | 3706 ms | −0.0226 dB | no (0.0001) |

Across these five, every geometry that is wrong is **sub-millisecond** and every geometry that is
expensive is **already accurate to ±0.02 dB**. (Five points; see the caveats below — this is a
pattern with a mechanism behind it, not a proven law.) The `{0,1}` subset check gets all five
verdicts right, and its estimate is
consistently *larger* than the full N-vs-2N estimate — conservative, which is the safe direction
for a gate.

What is **measured** is where the failures sit: all three are at **low `D/λ` or low θ**, where the
total cycle count is small (4.5 to 10.5 cycles) and — for the two coma rows — the answer is a
heavily cancelled residue (111.3× and 58.9×, §3a). At Ka the budget asks for 1355 and 16867
points against cycle counts dominated by `kernel_cycles` (`D/λ = 3629`), and the served answer is
right to ±0.02 dB.

The **hypothesis** connecting them — that a large absolute cycle count implies both a
well-resolved integrand and a mode sum out of the deep-cancellation regime — is *not* measured
here: the cancellation ratio at Ka was not computed (the per-mode replica is
`O(n_rho·n_phi·m_max)` with an `exp()` per term, which is not tractable at a converged Ka
reference). Establishing it is task-4 work, and it is what would turn the anti-correlation from a
pattern into a rule you can size a check by.

### The option that is not on P12's list: refine until converged

No listed option iterates, and no fixed multiplier is right for all geometries — the 2N leg is
−0.0445 dB at `gs_3.7m`, −0.0553 dB at `dsn_34m`, but **−0.3494 dB** at the UHF row. Doubling
until the N-vs-2N estimate clears the 2% floor (cost is exactly linear in `n_rho`, so `d`
doublings cost `2^(d+1) − 1` baselines):

| geometry | doublings to converge | final n_rho | final true error | cost | absolute |
|---|---|---|---|---|---|
| `gs_3.7m` X θ=5° | 2 | 169 | −0.0027 dB | 7× | **2.9 ms** |
| `dsn_34m` X θ=0.10° | 2 | 113 | −0.0033 dB | 7× | **3.2 ms** |
| D12 UHF θ=16° | 3 | 145 | −0.0013 dB | 15× | **2.6 ms** |
| `dsn_34m` Ka θ=5° | 1 | 2709 | +0.0008 dB | 3× | 0.9 s |

### Recommendation

**(ii-a) as the gate, refinement as the response**: compute at N, run the `{0,1}` subset check,
and refine only when it fires. The measurements price this at

- **+37% on the Ka geometries** (5.08 s at θ=90°, against 11.12 s for option (i)) — and the
  check declines to refine there, so that 37% is the whole cost;
- **~3 ms on every geometry that is actually wrong**, converging all of them to better than
  0.01 dB.

This costs less than option (i) everywhere, is the only priced option that fixes the −7.08 dB UHF
row, and leaves P10-perf's target case paying 1.37× rather than 3×. It also keeps P12's stated
fallback available: bound the refinement by S3's existing wall-clock budget and return an honest
`converged = false` when it runs out, rather than a silent wrong number.

**Two caveats before this ships**, both for P12 task 4:

1. **Five points is a signal, not a validation.** The subset check is 5/5 here, but a
   false-positive sweep (does it fire where the answer is already good?) and a false-negative
   sweep (is there a geometry where m=0 and m=1 are *not* the error carriers?) both need to run
   across θ and D/λ. `radial_points_for` was itself validated on the symmetric branch and did
   not survive contact with this one.
2. **Why m=0 and m=1 carry the error is not yet understood.** They are the error carriers in all
   three failing geometries, and they are *not* the largest modes by magnitude (§4, D-A point 2).
   Choosing the subset by fitting to three data points is the same mistake in kind as the budget
   formula's. Establish the reason, or make the subset adaptive rather than fixed.

---

## 5. Discrepancies with what is currently filed

Recorded rather than resolved; both want a second pair of eyes before P12's tests are written.

1. **The UHF row is much worse than filed.** P12 records D12's `UHF_Array_Element` fixture at
   600 MHz, θ=16° as **1.23 dB** (−50.7668 vs −49.5383 dBi). Measured here at the field level:
   **−7.0761 dB** at φ=0 and **−3.8546 dB** at φ=90 — and **−1.2285 dB** at floor 32, which is
   `default()`'s floor, not `adaptive()`'s. P12's row does not record φ.
2. **D17's preset labels look transposed.** D17 reports `default()` = −50.7668 and `adaptive()`
   = −49.6090 with `high_accuracy()` = −49.5426 "agreeing with `default()` to 0.066 dB" — but
   −49.5426 agrees with −49.6090, not with −50.7668. On the mode path `default()` (floor 32) is
   strictly more accurate than `adaptive()` (floor 16), since `min_rho_points` is the only field
   of either preset that `radial_points_for` reads. The measurement here has floor 32 → −1.2285
   dB and floor 64 → −0.0707 dB (φ=90), matching D17's *numbers* with the labels swapped.
   The direction of the divergence D17 filed is unaffected; which preset is the worse one is not.
3. Both may be explained by D17 having measured through `compute_gain_db` (where the F7
   sidelobe floor and the spillover gate can compress or lift a low PO value) rather than at
   the field level. The field-level measurement isolates the integrator, which is what P12 is
   about.

---

## 6. What this means for the rest of P12

- **Task 1 is answered, and the answer is not "fix the formula".** `radial_points_for`'s cycle
  count is sound. Do not re-derive it; do not add a term to it. The gap is between "resolved"
  and "accurate", and it is a property of the *consumer*, not the budget.
- The **exit criterion** "the mechanism behind (b) is documented" is met by this document.
- Task 4's regression anchors should include a **φ=0 UHF point** (the −7.08 dB case) and a
  **symmetric-branch control** at matched density — the control is what proves a future fix
  did not simply raise density everywhere.
- P12's own warning holds and is now quantified: this **will** move served values on every
  antenna with an offset or asymmetric feed, by up to several dB at some angles.

---

## 6a. Implemented (2026-07-31)

`PHYSICS_MODEL_VERSION` **5 → 6**.

**D-A, as decided:** the mode path now establishes radial convergence the way the symmetric
branch does — compare `N` against `2N`, **return the fine leg** — with two additions the
symmetric branch does not need:

- a **cheap pre-gate** (`RADIAL_PROBE_MODES = {0,1}`, `RADIAL_PRE_GATE_SAFETY = 32`) on
  geometries where a full check leg is expensive (`use_radial_pre_gate`, threshold
  `FULL_RADIAL_CHECK_WORK_LIMIT = 4·10⁶` work units ≈ 10 ms). The pre-gate may only *certify*;
  the moment it says the answer is moving, control falls through to the honest loop.
- **refinement** (`MAX_RADIAL_REFINEMENTS = 4`), because no fixed multiplier is right
  everywhere — the 2N leg lands at −0.045 dB on `gs_3.7m` but −0.349 dB on the UHF fixture.

The safety factor is the price of shipping the pre-gate on five points: the `{0,1}` probe is
conservative where it fires (1.17–2.18× the honest estimate) but **anti-conservative where it
passes**, underestimating by 3.5× at Ka θ=5° and **26×** at θ=90°. 32 covers the measured worst
case with margin and errs toward escalating. Retire or re-derive it once a θ × D/λ sweep bounds
the probe-to-total ratio properly.

The two error estimates are **summed**, not merged — P12 required this be decided explicitly.
They bound errors on different axes of the same returned field, both are absolute field-magnitude
differences, and summing never understates. `converged = mode_converged && radially_converged`.

**D-B, as decided:** `adaptive()`'s `min_rho_points` 16 → 32, matching `default()`, framed in
the docstring as closing D17's calibrate-vs-service preset divergence and explicitly *not* as
the fix for sub-defect (a) — which the measurements show is not a floor problem at any measured
point. Not 64, which would reopen the divergence inverted.

### Measured outcome

| geometry | pre-P12 | post-P12 | `converged` |
|---|---|---|---|
| `gs_3.7m`/`x_band_feed` 8.4 GHz θ=5° | −0.8157 dB | **−0.0027 dB** | true |
| `dsn_34m`/`x_band` 8.45 GHz θ=0.10° | −1.1671 dB | **−0.0033 dB** | true |
| D12 UHF 600 MHz θ=16° φ=0 | −7.0761 dB | **−0.0013 dB** | true |
| D12 UHF 600 MHz θ=16° φ=90 | −3.8546 dB | **−0.0027 dB** | true |
| `dsn_34m`/`ka_band` 32 GHz θ=5° | +0.0126 dB | +0.0126 dB (pre-gate certified, 2 legs) | true |

Symmetric-branch control unmoved and still cheap (50–434 evaluations; Δ ≤ 0.0025 dB), and the
`reference_validation` boresight anchors did not move — the change did not leak onto the J₀ path.

### Pre-gate yield: does it earn its complexity?

A pre-gate that always declines costs an extra leg and buys nothing, so this was measured after
the fact (`p12_pre_gate_yield_across_geometries`). `legs` counts full sweeps: 2 = the pre-gate
certified (or one refinement sufficed), 3+ = refinement ran.

| geometry | work | pre-gated? | N | legs |
|---|---|---|---|---|
| `gs_3.7m` X θ=5° | 6.1·10⁴ | no | 43 | 4 |
| `dsn_34m` X θ=0.1° | 5.9·10⁴ | no | 33 | 4 |
| `dsn_34m` X θ=5° | 2.7·10⁶ | no | 359 | 2 |
| `dsn_34m` X θ=10° | 5.2·10⁶ | yes | 697 | **3** — declined |
| `dsn_34m` X θ=45° | 2.2·10⁷ | yes | 2909 | **3** — declined |
| D12 UHF θ=16° | 3.6·10⁴ | no | 33 | 4 |
| `dsn_34m` Ka θ=1° | 3.4·10⁷ | yes | 335 | **2** ✅ |
| `dsn_34m` Ka θ=5° | 1.4·10⁸ | yes | 1355 | **2** ✅ |
| `dsn_34m` Ka θ=45° | 1.1·10⁹ | yes | 11011 | **2** ✅ |
| `dsn_34m` Ka θ=90° | 1.7·10⁹ | yes | 16867 | **2** ✅ |

It earns its keep where it was supposed to: **all four Ka points certify in two legs**, which is
the 300 ms–3.7 s regime P10-perf cares about. On the two mid-cost X-band points the safety factor
makes it decline and refinement runs — those pay `5N−2` radial units instead of the `3N−1` a plain
full check would have cost, i.e. the pre-gate is a net *loss* of roughly 1.7× there. That is the
safety factor working as designed (fail toward the honest check) and it is a real, bounded cost.
Tightening `FULL_RADIAL_CHECK_WORK_LIMIT` upward — so the pre-gate only runs where it reliably
certifies — is the obvious tuning knob if that 1.7× ever matters; it is left alone for now because
the measured X-band points are 5–100 ms, not seconds.

Pinned by three tests in `antenna-model/tests/reference_validation.rs`:
`p12_mode_path_radial_convergence_anchors`, `p12_symmetric_branch_control_still_accurate_and_cheap`
(asserts accuracy **and** that the work did not grow, so a future "fix" cannot pass by raising
density globally), and `p12_pre_gate_keeps_expensive_ka_at_two_legs` (cost guard).

### Two existing tests moved

- `evaluator::test_sidelobe_floor_does_not_perturb_boresight_reference` reconstructed the
  evaluator's reference with `fast()` while the evaluator uses `adaptive()`. That passed only
  because the two presets were field-for-field identical; D-B separated them and the test failed
  on a 0.0003 dB preset difference unrelated to the sidelobe floor it isolates. Now uses
  `adaptive()`. At θ=0 the radial budget is zero cycles, so `min_rho_points` is the only thing
  setting density — this test is unusually sensitive to that field.
- `pattern::p2_moderate_offset_exact_only_gain_pinned_and_routes_standard_po` moved
  16.05 → 13.72 dBi. See §7 — **neither value is physically right**, and the reason is not P12.

---

## 7. Filed, not fixed: the azimuthal cap silently costs +28.7 dB on strongly-steered feeds

Found while arbitrating the `p2_moderate_offset` pin, which moved 2.3 dB. That geometry (3 m
dish, 8.4 GHz, lateral feed offset 0.6 m ⇒ `δ/f = 0.4`) trips **both** deliberate
strongly-steered performance caps, and the one P12 did *not* charter is far the larger error.

At `D/λ = 84` the retired 2D Simpson quadrature is a trustworthy oracle, so it can arbitrate.
Mode path vs that oracle at boresight, sweeping both axes independently:

| `n_rho` \ `n_phi` | 64 (the served cap) | 256 | 512 |
|---|---|---|---|
| 33 (served) | +30.9954 dB | +23.4929 | +23.4929 |
| 65 | +28.6587 | −0.3864 | −0.3864 |
| 129 | +28.6690 | **−0.0173** | −0.0173 |
| 257 | +28.6695 | −0.0010 | −0.0010 |
| 2049 | +28.6695 | +0.0000 | +0.0000 |

**Read the first column.** With `n_phi = 64` the mode path converges radially to **+28.67 dB
above the oracle** and stays there at any radial density. P12's fix works — the radial axis is
now converged (−0.0005 dB) — but it converges to a number that is ~29 dB wrong for an
independent reason. With `n_phi ≥ 256` the same integrator reproduces the oracle.

**Mechanism.** `MODE_PHI_STEERED_MAX` clamps `n_phi` to 64 whenever `δ/f > MODE_STEERING_RATIO`
(0.05). Here the true azimuthal bandwidth is `k·δ·(R/f) ≈ 106` modes, so 64 φ' samples alias
high modes straight into `g₀`. At θ=0 only `m = 0` survives the `Jₘ(0)` kernel, so the aliased
`g₀` *is* the answer. This is the same failure `mode_count_for`'s own docstring warns about
("under-sizing `n_phi` aliases high input modes into `g_0` … the exact defect that made a
heavily-steered feed read far too high off-axis") — the cap reintroduces it deliberately, for
cost, and does not tell anyone. `converged` comes back **true**.

**Same class as P12, different axis**: a performance cap that silently returns a wrong number.
It is P10-class in magnitude (+28.7 dB against P10's 20–35 dB).

**Exposure.** No enabled *design* feed trips it — `gs_3.7m` is `δ/f = 0.027`, `dsn_34m` 0.011,
both below 0.05. But the served feed position is design offset **plus steering**
(`compute_feed_position_from_pointing`), so a sufficiently steered request can cross the
threshold at runtime. Establishing whether real steering reaches `δ/f > 0.05` is the first
question for whoever picks this up.

**Not fixed here** because it is the azimuthal axis, P12 charters the radial one, and the fix
is a genuine cost/correctness decision of its own (the cap exists for a reason; removing it
makes `n_phi` up to 512 on steered feeds). Also note the interaction: on such a geometry P12's
refinement now spends up to 15× the radial work converging to a value dominated by azimuthal
aliasing — correct behaviour per-axis, wasteful in aggregate, and an argument for fixing the two
together.

---

## 7a. Fixed the same day — and it was worse than §7 measured

`PHYSICS_MODEL_VERSION` **6 → 7**. Three caps came out, in the order they were found; each was
exposed by removing the one before it.

### The φ' cap (`MODE_PHI_STEERED_MAX`)

Removed. `n_phi` is now sized from the aperture function's azimuthal bandwidth
`B = k·δ·(R/f)` — capped physically at `k·R` — and no longer rounded up to a power of two (the
φ' transform is a naive `O(n_phi·M)` DFT, not an FFT, so nothing required it, and the rounding
was costing up to 2× for nothing). `MODE_PHI_MAX` 512 → 2048. When the ceiling still binds,
`ModeSizing::azimuthally_resolved` is false and `converged` follows.

**§7 measured +28.7 dB on the `p2_moderate_offset` geometry. On the `coma_aberration_test`
geometry — 34 m dish, δ = 1.19 m, i.e. `δ/f = 0.0875`, a routine ~5° beam steer — the same cap
was wrong by up to +82 dB:**

| θ | n_phi=64 (old cap) | 128 | 256 | 512 | 1024 |
|---|---|---|---|---|---|
| 0° | **+77.4** | +67.4 | −0.000 | −0.000 | +0.000 |
| 1° | **+82.1** | +75.0 | +0.466 | +0.000 | +0.000 |
| 3° | **+80.6** | +81.8 | +65.2 | −0.000 | −0.000 |
| 5° | **+72.1** | +65.1 | +21.0 | −0.000 | −0.000 |

(dB against `n_phi = 4096`, radial density pinned high, `m_max` tracking `n_phi`.)

This also corrected my own first attempt: I initially gated `azimuthally_resolved` on a hard
`n_phi ≥ 2B + 2` Nyquist line, which called the converged `n_phi = 512` row **unresolved**.
`gₘ(ρ) = jᵐ Jₘ(k·δ·ρ/f)` decays super-exponentially past `m = a`, and `a` reaches `B` only at
the rim, so real content dies out below `B`. The line is now `n_phi ≥ 2B`, still deliberately
conservative: a spurious `converged = false` costs a warning; a spurious `true` costs 82 dB.

### The radial cap (`MODE_RADIAL_CYCLE_CAP`), removed as a consequence

Its sibling, keyed off the same `MODE_STEERING_RATIO`, clamping the coma radial term to 8
cycles. After P12 gave this path refinement it was **strictly harmful** — less correct *and*
more expensive. At the same geometry, θ=0, true coma content 41.9 cycles:

- capped: budget asks 8 ⇒ `n_rho` starts at 33; four doublings (33→65→129→257→513, **997
  radial units**) still ended **0.34 dB short**, honestly reporting `converged = false`;
- uncapped: starts at 169, converges on the first check, **506 units**.

Starting a refinement loop below the physics saves nothing — every wasted leg is discarded.
`MODE_STEERING_RATIO` had no remaining users and went with it.

### The mode-truncation margin, exposed by both

With `n_phi` no longer clamping `m_max` far below `m_theta`, the `M`-vs-`M+1` check began
firing on the steered geometry — 14% top-mode contribution at θ=3°. It was right.
`m_theta = k·R·sinθ + 6` cuts into live spectrum: `Jₘ(x)` has an Airy turning point at `m = x`
and decays over a width `~x^(1/3)`, so a flat `+6` is far inside the transition. Measured
against an `m = 382` reference: **+0.103 dB at θ=1°** and **+0.490 dB at θ=3°**, both recovered
by ~16 more modes. Now `m_theta = x + 4·x^(1/3) + 6`, which scales the way the transition does.

**Worth noting how this one was nearly missed:** the value-level anchors could not see it,
because they derive `m_max` from θ internally and so truncated their references identically.
Only the self-check, which measures the tail directly, disagreed with them — and it was the
self-check that was right.

### Outcome

| geometry | old (capped) | now | `converged` |
|---|---|---|---|
| steered coma 34 m, θ=0° | +77.4 dB wrong | **−0.0068 dB** | true |
| θ=1° | +82.1 dB wrong | **+0.0030 dB** | true |
| θ=3° | +80.6 dB wrong | **+0.0007 dB** | true |
| θ=5° | +72.1 dB wrong | **+0.0009 dB** | true |

`p2_moderate_offset`'s pin moved again, 13.72 → **−14.95 dBi**, which is the oracle-consistent
value (13.72 − 28.67) — it is now a validated number rather than a known-wrong one.

Two enabled geometries got **cheaper**: dropping the power-of-two rounding took `dsn_34m` X-band
from `n_phi` 128 → 76 and Ka from 512 → 260, both still comfortably above `2B`. That "still
above 2B" is an argument, not a measurement, so it is now pinned by
`served_n_phi_sizing_is_sufficient_on_every_asymmetric_geometry` — a non-ignored test comparing
every served geometry's sizing against a 4× denser φ' grid. It is the **only** automatic guard
on this axis, because the radial and truncation checks both operate on `gₘ` that φ' aliasing has
already corrupted.

### The effort ceiling moved to the model's own scope boundary

Removing the cap outright over-corrected, and the integration suite said so: seven latency and
budget tests failed, because the shared fixture steers the feed to **3.06f** and the integrator
was now dutifully resolving a `B = k·R ≈ 310` spectrum for it. That is waste — past
`SEVERE_OFFSET_THRESHOLD` (0.5f) the feed is outside physical-optics scope, the caller is already
told so (`SevereFeedOffset` + `RayTraceDegraded`), gain routes to the ray-tracing stub, and this
integral survives only as the stub's normalization anchor. Converging a number the model has
already disclaimed buys nothing.

So an effort ceiling stays — deliberately the same *shape* as the deleted
`MODE_PHI_STEERED_MAX`, differing on exactly the two things that made that constant a defect:

| | old `MODE_PHI_STEERED_MAX` | now |
|---|---|---|
| triggers at | `δ/f > 0.05` — catches ordinary beam-steering (a 5° steer is 0.0875) | `δ/f > 0.5` — the documented PO scope boundary |
| when it binds | silent, `converged = true`, up to +82 dB wrong | `azimuthally_resolved = false` ⇒ `converged = false` |

That restored the integration suite to 5.5 s (from 66 s) with every correctness gain intact:
δ/f = 0.0875 and δ/f = 0.4 are both fully resolved, and only geometries the model has already
declared out of scope get the cheap treatment — and are told.

**The radial axis needed the same treatment, and one more test caught it.** Having removed
`MODE_RADIAL_CYCLE_CAP` outright, the 3.06f fixture still paid ~6× radially (uncapped coma
content 49.3 cycles ⇒ `n_rho` starts at 198 instead of 33, then refines), and
`concurrent_tests::test_sustained_load` failed in debug: every request succeeded, but only 12
landed in the 2 s window against a floor of `num_workers × 5`. So the cap returns on that axis
too, as `BEYOND_SCOPE_COMA_CYCLE_CAP`, keyed to the same `SEVERE_OFFSET_THRESHOLD` predicate —
the two axes now agree on where scope ends — and likewise not silent, since P12's radial
N-vs-2N check reports `converged = false` if the clamp costs accuracy.

The shape of the whole fix, then: **size from the physics inside the model's scope; cap effort
outside it; never be silent about which.** The old constants got the first part wrong (they
capped inside scope) and the third part wrong (silently). The threshold, not the mechanism, was
the defect.

### Cost, stated plainly

A steered geometry now costs ~69× more per evaluation than the (wrong) capped version: `n_phi`
64 → 536 and `m_max` 30 → 254 at wide angles. In a debug build a single such integration exceeds
S3's 30 s budget. **The production implication is real and intended**: a steered wide-angle
evaluation can now hit the wall-clock budget and return 504 rather than a silently-aliased
number. P10-perf's FFT for the `gₘ` φ'-DFT (`O(n_phi·log n_phi)` instead of `O(n_phi·M)`,
~28× here) is what buys the headroom back — it is now a latency *and* a coverage item.

`coma_aberration_test`'s peak search was restructured coarse-then-fine (201 → 52 points, same
0.1° resolution), and its two **off-axis** tests moved to an electrically-scaled twin — same
`f/D`, same `δ/f = 0.0875`, a 10×-smaller dish — because everything they assert (steering angle
`≈ δ/f`, steering direction, coma-lobe asymmetry) depends on `δ/f`, not `D/λ`. That took the
binary from 50 s to **0.29 s** in release and from unbounded to trivial in debug, and it fixed a
pre-existing weakness: on the 34 m dish HPBW is ≈0.06°, so a 0.1° peak search was under-sampling
the main lobe; at 3.4 m the beam is ≈0.73° and the search actually resolves it. The boresight
tests keep the 34 m geometry (θ=0 collapses `m_max` to 6, so it is cheap). The expensive regime
keeps dedicated coverage at bounded angles, in the two guards named above.

Three tests now opt out of the S3 wall-clock budget, each with the reason recorded at the site:
a test deliberately exercising an expensive geometry should not be gated by the production
budget. Debug-build cost of the two new guards: 10.6 s and 96 s.
