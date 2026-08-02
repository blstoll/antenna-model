# P13 — retiring P12's radial pre-gate, and correcting D17's record

**Date:** 2026-08-01
**Unit:** roadmap **P13** — "Validate or retire P12's empirical guards
(`RADIAL_PRE_GATE_SAFETY`, probe-mode set)"
([`roadmap-2026-07-work-units.md`](roadmap-2026-07-work-units.md))
**Status:** implemented. `PHYSICS_MODEL_VERSION` **7 → 8**.
**Predecessor:** [`findings-2026-07-31-p12-mode-path-radial-budget.md`](findings-2026-07-31-p12-mode-path-radial-budget.md)

P13 asked three questions. The answers, up front:

1. **Does `RADIAL_PRE_GATE_SAFETY = 32` bound the probe-to-total ratio?** **No.** A θ × D/λ
   sweep measures **43.5×** where the probe passes — on `dsn_34m` Ka at θ=90°, a served antenna
   at a served angle, and the very geometry the constant was fitted on. The pre-gate is
   **retired**, not re-tuned.
2. **Why did modes {0,1} carry the radial error?** Because per-mode relative quadrature error is
   set by **intra-mode cancellation**, not by mode magnitude — and the cancellation ratio is
   systematically largest at low `m`. The probe set was not arbitrary after all; it was
   implicitly selecting the most self-cancelling modes. Mechanism established, subset deleted
   anyway (§2).
3. **Is D17's `default()`-vs-`adaptive()` table right?** Partly. Its `default()` and
   `high_accuracy()` rows reproduce exactly and identify the φ it never recorded; its
   `adaptive()` row is **unreproducible**. Both of P12's §5 corrections to it are themselves
   wrong, in different ways (§3).

---

## 1. The pre-gate no longer pays, and its safety factor never bounded anything

### 1a. What the pre-gate was

P12 gave the azimuthal-mode path a radial N-vs-2N convergence check. On geometries where a full
check leg was judged expensive (`use_radial_pre_gate`, threshold `FULL_RADIAL_CHECK_WORK_LIMIT`),
it first ran a cheap leg over only `RADIAL_PROBE_MODES = {0,1}`; if that partial sum had barely
moved — after multiplication by `RADIAL_PRE_GATE_SAFETY = 32` — the result was certified
converged and the **coarse `N` leg** was returned. Everywhere else the honest check ran and the
**fine `2N` leg** was returned.

P12 shipped it knowing both constants were fitted to five geometries, and said so.

### 1b. The economic premise expired when the transform became an FFT

The pre-gate's entire justification is that a 2-mode leg is much cheaper than a full one. That
was true when the φ' transform was an `O(n_phi · M)` DFT: the probe skipped the `×M` term.
P10-perf (2026-08-01) replaced it with an `O(n_phi log n_phi)` FFT and made the mode work `O(M)`,
so per radial sample a probe leg costs `n_phi + 2` against a full leg's `n_phi + M + 1`.

Measured (`p12_price_decision_d_a_options`, release, min-of-N):

| geometry | a 2-mode sweep, as % of a full sweep | P12's figure (pre-FFT) |
|---|---|---|
| `gs_3.7m` X θ=5° | 61.6 % | 52.0 % |
| `dsn_34m` X θ=0.1° | 69.5 % | 61.5 % |
| D12 UHF θ=16° | 59.3 % | 51.8 % |
| `dsn_34m` Ka θ=5° | **66.5 %** | **18.3 %** |
| `dsn_34m` Ka θ=90° | **66.3 %** | **18.5 %** |

The Ka rows are the ones that mattered — they are the regime the pre-gate exists for — and the
probe went from saving ~82 % of a leg to saving ~34 %.

Priced end to end at `dsn_34m` Ka θ=90° (multiples of one baseline sweep):

| option | cost | what it returns |
|---|---|---|
| (ii-a) subset check, return `N` — **the pre-gate** | 2.33× — 583 ms | the **coarse** leg |
| (A) fine leg only, no check | 2.00× — 498 ms | the fine leg, unchecked |
| (i) full N-vs-2N, return `2N` — **the honest check** | 3.00× — 748 ms | the fine leg |

Note the middle row. Post-FFT the pre-gate is **dominated**: it costs *more* than simply
computing at `2N` and returning it, and delivers a *less accurate* answer than that cheaper
option. Its only remaining claim over the honest check is a 0.67× saving — bought by returning
the worse of the two legs it has already paid for.

### 1c. The safety premise never held

`RADIAL_PRE_GATE_SAFETY` exists because the probe's estimate is not a bound. The quantity it
must cover is the **probe-to-total ratio**: the honest N-vs-2N estimate divided by the `{0,1}`
probe's estimate of it, measured *where the probe passes* (where it fires, control falls through
to the honest loop and the estimate's accuracy is irrelevant).

P12 measured that ratio at 3.5× and 26× on two points and chose 32. `p13_probe_to_total_ratio_sweep`
sweeps the plane instead — `D/λ` from 400 to 3600, θ from 0.1° to 90°, restricted to the
pre-gated regime, 46 points:

| geometry | D/λ | θ | probe est. | honest est. | ratio | probe verdict |
|---|---|---|---|---|---|---|
| `dsn_34m` X | 958 | 90° | 6.18e-4 | 1.91e-3 | 3.1 | pass |
| `dsn_34m` Ka | 3629 | 5° | 4.07e-4 | 1.38e-3 | 3.4 | pass |
| `dsn_34m` Ka | 3629 | 20° | 2.22e-4 | 1.75e-3 | 7.9 | pass |
| `dsn_34m` Ka | 3629 | 45° | 2.63e-4 | 3.20e-3 | 12.2 | pass |
| **`dsn_34m` Ka** | **3629** | **90°** | **5.95e-5** | **2.59e-3** | **43.5** | **pass** |
| swept 30 m δ/f=.10 | 1201 | 90° | 2.20e-4 | 3.60e-3 | 16.3 | pass |
| swept 70 m δ/f=.02 | 1961 | 90° | 1.20e-4 | 2.04e-3 | 17.1 | pass |

**Worst passing ratio: 43.5×, against a constant of 32 — margin 0.74×.** The constant does not
bound the quantity it exists to bound, and it fails on `dsn_34m` Ka θ=90°: an enabled antenna, a
served angle, and *the same geometry P12 fitted the constant on*, where P12 measured 26×.

The ratio moved because P10-perf resized `n_phi` (512 → 270 via `next_fast_len`) — a change with
no physics content whatsoever, which nonetheless invalidated a fitted correctness constant with
nothing in the build to notice. **That is the durable lesson here**, and it generalizes past this
unit: a constant fitted to measurements is coupled to every input of those measurements,
including ones nobody thinks of as inputs.

No *counterexample* was found — no point where the probe passes while the honest check fires —
but that is not reassurance. At the 43.5× point the honest estimate (2.59e-3) sits ~8× below the
2 % tolerance floor for reasons unrelated to the pre-gate; the margin that was supposed to
protect the gap had already been consumed.

### 1d. Decision: delete

The pre-gate is removed, along with `RADIAL_PROBE_MODES`, `RADIAL_PRE_GATE_SAFETY`,
`FULL_RADIAL_CHECK_WORK_LIMIT`, `use_radial_pre_gate`, `radial_probe_field`, and
`ModeSweep::radial_probe`. Every geometry now takes one shape: full sweep at `N`, full sweeps at
`2N−1`, `4N−3`, … until the difference clears tolerance or `MAX_RADIAL_REFINEMENTS` runs out;
the fine leg is returned and the estimate is honest.

**Accuracy** (radial error against a converged reference, `p12_post_fix_served_behaviour`):

| geometry | pre-gated before? | P12 | P13 |
|---|---|---|---|
| `dsn_34m`/`ka_band` θ=5° | yes | +0.0126 dB | **+0.0008 dB** |
| `gs_3.7m`/`x_band_feed` θ=5° | no | −0.0027 dB | −0.0027 dB |
| `dsn_34m`/`x_band` θ=0.1° | no | −0.0033 dB | −0.0019 dB |
| D12 UHF θ=16° φ=0 | no | −0.0013 dB | −0.0021 dB |
| D12 UHF θ=16° φ=90 | no | −0.0027 dB | −0.0044 dB |

The pre-gated row is **16× more accurate**. The un-pre-gated rows are unchanged by P13; their
small movements are P10-perf's `n_phi` resizing, and all stay inside 0.005 dB.

**Cost** (work units, `p13_radial_leg_count_across_geometries`):

| geometry | P12 | P13 | change |
|---|---|---|---|
| `dsn_34m` Ka θ=1° | 317 643 | 406 620 | +28 % |
| `dsn_34m` Ka θ=5° | 1 285 623 | 1 645 920 | +28 % |
| `dsn_34m` Ka θ=90° | 16 006 511 | 20 493 000 | +28 % |
| **`dsn_34m` X θ=45°** | **1 524 114** | **1 047 120** | **−31 %** |
| everything below the old work threshold | — | — | unchanged |

The X-band row is the pre-gate's other cost, now recovered: there it *declined*, so its probe leg
was pure waste on top of the honest check that followed. P12 recorded that as a known ~1.7× loss
and left it; deleting the pre-gate collects it back.

Net on the served path: `dsn_34m` Ka θ=90° goes ~583 → ~748 ms. Still far inside S3's 30 s
wall-clock budget, still far outside the <100 ms p95 target — which was already true at 583 ms
and is a separate problem (~85 % aperture-plane evaluation; see the P10-perf risk entry).

**What this buys beyond the numbers:** on those geometries `converged = true` now means two full
sweeps agreed, rather than two of 135 modes having moved little enough after multiplication by an
unvalidated constant.

---

## 2. Why modes {0,1} carried the error — mechanism, not fit

P12 chose the probe set by fitting to three failures and flagged it as a defect in its own
reasoning: *"picking the subset by fit is the same kind of mistake the budget formula made."* The
puzzle was that `m = 0, 1` are the top error contributors everywhere measured while being
conspicuously **not** the largest modes by `|Rₘ|`.

The mechanism is the findings doc's own §3b applied one level down. §3a explains the *total*
error by **inter**-mode cancellation: the modes sum to a residue far smaller than `Σ|Rₘ|`, so
small per-mode errors become large total ones. The same thing happens *within* a mode. Define the
intra-mode cancellation ratio

```
Cₘ = ∫|Fₘ(ρ)|dρ / |∫Fₘ(ρ)dρ|
```

— how much mode `m`'s own radial integral cancels. Measured (`p13_intra_mode_cancellation_explains_the_probe_set`):

| geometry | ranking by `Cₘ` | ranking by relative error | ranking by `\|Rₘ\|` |
|---|---|---|---|
| `gs_3.7m` θ=5° | **1**, 6, 4, **0**, 2, 3 | **1**, **0**, 6, 5, 9, 4 | 5, 7, 0, 2, 3, 4 |
| `dsn_34m` θ=0.1° | **0**, 2, **1**, 3, 8, 5 | **0**, **1**, 8, 3, 5, 6 | 4, 3, 6, 1, 5, 0 |
| D12 UHF θ=16° | **0**, 2, 4, 8, 6 | **0**, 2, 8, 4, 6 | 0, 2, 4, 6, 8 |

The error ranking tracks `Cₘ`, not `|Rₘ|`. The clearest single case is `dsn_34m`: its **largest**
mode by magnitude (`m=4`, `|Rₘ|` = 5.4e-1) has `Cₘ` = 3.02 and is the **most accurate** mode in
the table (0.0079 % error), while `m=0` has `Cₘ` = 47.3 and 3.44 % error. At the UHF fixture
`m=0` has `Cₘ` = **1765** and 9.76 % error against every other mode's `Cₘ` ≤ 46 and ≤ 0.09 %.

Why `Cₘ` is largest at low `m`: `Jₘ(kρ sinθ) ~ ρᵐ` near the axis, so a high-`m` integrand is
suppressed over the inner aperture and concentrated in a narrow annulus near the rim, across
which the chirp and coma phases sweep comparatively little — a short support with little phase
variation integrates up without much cancellation. A low-`m` integrand spans the full `[0, R]`
and integrates an oscillating phase across the whole aperture, which is exactly the deep-residue
situation. Hence the probe set was selecting the most self-cancelling modes without knowing it.

`Cₘ` is a *necessary-not-sufficient* indicator: it bounds how much amplification is available,
not how much is realized (which also depends on where the `L¹` mass sits in radial frequency).
`gs_3.7m`'s `m=6` shows the gap — `Cₘ` = 78, second-highest, but only 0.166 % error.

**Consequence for the code:** none, since the subset is deleted. **Consequence for anyone
reintroducing a subset check:** rank by `Cₘ` computed from the geometry, not by `|gₘ|`, and treat
it as a screen rather than a bound.

---

## 3. Correcting D17's record

P12's findings §5 recorded two discrepancies with D17 "rather than resolved", and P13's exit
criteria require fixing them in place. Re-measured with `p13_recheck_d17_preset_divergence_table`.
The record is load-bearing: D17's table is what justified P12's decision D-B.

D17 filed, on D12's UHF fixture at 600 MHz / θ=16°:
`default()` = −50.7668, `adaptive()` = −49.6090, `high_accuracy()` = −49.5426 dBi, "1.16 dB
apart, `converged = true`, no warning", with `high_accuracy()` described as "agreeing with
`default()` to 0.066 dB".

### 3a. What is true now — the divergence is closed

| φ | preset | `min_rho_points` | `n_rho` | served gain | field vs converged |
|---|---|---|---|---|---|
| 0° | `default()` | 32 | 33 | −43.5846 | −0.0021 dB |
| 0° | `adaptive()` | 32 | 33 | −43.5846 | −0.0021 dB |
| 0° | `high_accuracy()` | 64 | 65 | −43.5826 | −0.0021 dB |
| 90° | `default()` | 32 | 33 | −49.5426 | −0.0044 dB |
| 90° | `adaptive()` | 32 | 33 | −49.5426 | −0.0044 dB |
| 90° | `high_accuracy()` | 64 | 65 | −49.5385 | −0.0044 dB |

`default()` and `adaptive()` are now **identical to four decimals** — D-B raised `adaptive()`'s
floor to 32 to match, and `min_rho_points` is the only preset field `radial_points_for` reads on
this path. `high_accuracy()` agrees to 0.004 dB. **D17's preset divergence is closed**, and
closing it is what P12's D-B was for.

### 3b. Discrepancy 1 — the UHF magnitude was understated: confirmed, and φ identified

P12's roadmap row records this geometry as **1.23 dB**; the field-level measurement is **−7.0761
dB at φ=0** and **−3.8546 dB at φ=90**, and D17's row records no φ.

D17's φ is now identified as **90°**: its `high_accuracy()` figure of −49.5426 is *exactly* what
all three presets return today at φ=90 (and nothing near the φ=0 value of −43.58). So the honest
restatement of D17's row is: at φ=90, pre-P12, the served value was 1.23 dB from converged at
floor 32 and 3.85 dB at floor 16 — and at φ=0 the same fixture was **7.08 dB** off.

### 3c. Discrepancy 2 — "the labels are transposed" is itself wrong

P12's §5 inferred that D17's preset labels were swapped, reasoning that pre-P12 `default()`
(floor 32) was strictly more accurate than `adaptive()` (floor 16), so the more-accurate-looking
number could not be `adaptive()`'s.

The reasoning about which preset is denser is right. The inference is not. Reconstructing the
pre-P12 single-leg behaviour at both floors, at φ=90, against today's converged −49.5426:

| `min_rho_points` | `n_rho` | Δ vs converged | reconstructed raw PO |
|---|---|---|---|
| 16 | 19 | −3.8546 dB | **−53.3972 dBi** |
| 32 | 33 | −1.2285 dB | **−50.7711 dBi** |

D17's `default()` = −50.7668 matches the floor-32 reconstruction to **0.004 dB**. That label is
**correct**. But D17's `adaptive()` = −49.6090 matches nothing: floor 16 gives −53.3972, and no
(floor, φ) combination in either principal plane produces −49.6090. The difference between the
two floors is 2.63 dB at φ=90 and 6.51 dB at φ=0 — never D17's stated 1.16 dB.

**Correct conclusion: D17's `adaptive()` figure is unreproducible from what the row records, and
the reason is that the row records neither φ nor the gate configuration it was measured under.**
Not a transposition. What *is* wrong in D17's prose is its attribution — it says
`high_accuracy()` agrees with `default()` to 0.066 dB, when −49.5426 agrees with −49.6090
(0.066 dB) and differs from −50.7668 by 1.22 dB.

### 3d. Discrepancy 3 — P12's proposed explanation is falsified

P12's §5 item 3 suggested both discrepancies "may be explained by D17 having measured through
`compute_gain_db`, where the F7 sidelobe floor and the spillover gate can compress or lift a low
PO value."

Measured: the F7 statistical sidelobe floor for this fixture is **−25.9834 dBi** — about **24 dB
above** every value in D17's table. Had it been active, all three of D17's rows would read
≈ −25.98 dBi and none of them do. Every number in that table is raw, floor-off PO, so the floor
cannot explain anything about it. (The floor is `1 − η_ruze` spread isotropically; at 2.0 mm RMS
and λ = 0.5 m, `4πσ/λ = 0.05` gives ≈ 0.0025 ⇒ −26 dBi, so the value is exactly as expected.)

This does have a live consequence worth stating separately: **for this fixture, served with
uncorrected physics and therefore with the floor ON, the served gain at θ=16° is ≈ −25.98 dBi
regardless of preset** — the PO values are 24 dB below the floor and entirely masked by it. The
whole preset-divergence question is invisible on the served path for this geometry. It was
visible to D17 only because `calibrate` evaluates with the floor off. That does not make D17's
finding wrong — the calibrate-side divergence was real and P12 fixed its cause — but it does mean
the *served* stakes on this particular fixture were lower than the table suggests.

---

## 4. What P13 changed

Production (`antenna-model/src/model/integration.rs`):
- deleted `RADIAL_PROBE_MODES`, `RADIAL_PRE_GATE_SAFETY`, `FULL_RADIAL_CHECK_WORK_LIMIT`,
  `use_radial_pre_gate`, `radial_probe_field`, and `ModeSweep::radial_probe`;
- the mode path's radial block is now one unconditional refinement loop.
- `PHYSICS_MODEL_VERSION` 7 → 8 (served gain moves on the formerly pre-gated geometries).

Tests:
- `reference_validation::p12_pre_gate_certifies_an_already_converged_geometry_in_two_legs` →
  `mode_path_settles_an_already_converged_geometry_in_two_legs`. Same geometry, same two-leg
  assertion, different reason: it now pins that the honest check *agrees on its first comparison*
  rather than that a pre-gate certified. Verified at 406 620 work units against a derived
  two-leg figure of 406 620.
- `radial_probe_field_matches_the_full_sweeps_probe_accumulation` deleted with the code it
  guarded.
- `p12_pre_gate_yield_across_geometries` → `p13_radial_leg_count_across_geometries`.
- new: `p13_probe_to_total_ratio_sweep` (§1c — kept runnable, with the retired constants restated
  locally and the probe legs taken from the test module's own `mode_subset_field`, so the
  decision can be re-checked rather than taken on trust),
  `p13_intra_mode_cancellation_explains_the_probe_set` (§2),
  `p13_recheck_d17_preset_divergence_table` (§3).

All 995 tests pass across both nextest tiers, including every P12 anchor
(`p12_mode_path_radial_convergence_anchors`, `p12_symmetric_branch_control_still_accurate_and_cheap`,
`p12_phi_cap_removed_steered_feed_matches_converged_reference`).

## 5. Filed, not fixed

- **`p13_probe_to_total_ratio_sweep` found no counterexample**, only an exceeded margin. If a
  subset check is ever reintroduced, the sweep should be extended to look for the stronger
  failure (probe passes while the honest check fires) at low `D/λ`, which this sweep could not
  reach — that regime never crossed the old work threshold, so it has no pre-gated points.
- **The `<100 ms` p95 target is still missed at wide-angle Ka** (~748 ms at θ=90°, up from
  ~583 ms). Unchanged in kind by this unit and already recorded under the P10-perf risk entry;
  the remaining cost is ~85 % aperture-plane function evaluation, not quadrature.
