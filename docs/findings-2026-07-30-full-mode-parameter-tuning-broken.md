# Finding 2026-07-30 — full-mode `--tune-parameters` crashes, and its bounds cannot reach the target anyway

**Found by:** roadmap unit D12 (calibrate CLI end-to-end test), Task 5, on the first attempt to
drive `calibrate --tune-parameters` end to end.

**Severity:** the documented calibration workflow's step 3 is **non-functional**. Not a
served-path defect — it affects artifact *production*, not query serving — but it means no
full-mode artifact has ever been produced with tuned parameters, and none can be until this is
fixed.

**Status: ✅ RESOLVED 2026-07-31.** All defects below are fixed, plus **two more found while
fixing them** (§Defect 3, §Defect 4 — added 2026-07-31). D12's tuned test is un-ignored and
green: the tuner now recovers the injected 2.0 → 2.6 mm surface-RMS perturbation **exactly**
(2.6000 mm, four iterations, ~20 s in a debug build).

The original two defects were necessary but **not sufficient** — fixing only them left the
tuner reporting 0.1 mm against a 2.6 mm truth. Anyone reading this doc for the "port the
sibling simplex fix" summary should read defects 3 and 4 as well; they are where the actual
recovery came from.

---

## Defect 1 — Nelder-Mead is constructed with a degenerate simplex, and panics

`calibrate/src/parameter_tuner.rs:389`:

```rust
let solver = NelderMead::new(vec![initial_params]).with_sd_tolerance(1e-6)?;
```

Nelder-Mead over `N` parameters requires a simplex of **`N + 1`** vertices. This passes exactly
**one**, whatever `N` is. On the first iteration argmin indexes `self.params[num_param_vecs - 2]`
with `num_param_vecs == 1`, and the `usize` subtraction underflows:

```
thread 'main' panicked at argmin-0.11.0/src/solver/neldermead/mod.rs:357:43:
attempt to subtract with overflow
```

It fails for **every** `--tuning-mode` and **every** `--max-tuning-iterations` — this is not a
slowness or convergence problem, it is a hard crash roughly 0.7 s in, before the optimizer
completes a single iteration and before any artifact or sidecar is written.

**The fix already exists in this repo, in the sibling code path.** `boresight_calibration.rs`
hit the identical bug and fixed it (`docs/implementation-plan.md` records it as "Bug Fixed",
2025-11-27). `boresight_calibration.rs:447-459` builds a real simplex:

```rust
for i in 0..n_params {
    let mut perturbed = initial_guess.clone();
    let perturbation = if perturbed[i].abs() > 1.0 { perturbed[i] * 0.1 } else { 0.1 };
    perturbed[i] += perturbation;
    simplex.push(perturbed);
}
let solver = NelderMead::new(simplex).with_sd_tolerance(1e-4)?;
```

The fix was never ported to `parameter_tuner.rs`. Porting it is the obvious first step, but see
defect 2 before assuming that alone makes tuning work.

**✅ Fixed 2026-07-31** — `parameter_tuner::build_initial_simplex`. Note that a *straight*
port would not have worked: the boresight heuristic perturbs every parameter **upward**, and
under the then-current bounds all three `UHF_Array_Element` tunables sat exactly on their
upper cap (see defect 2), so all `N` non-origin vertices would have seeded in the
out-of-bounds 1e10 penalty region — a simplex with no usable gradient. The ported version
steps away from whichever bound is nearer. Pinned by
`parameter_tuner::tests::tune_parameters_completes_for_every_tuning_mode` (drives argmin at
N = 1, 2, 3 — the coverage gap that let this hide) and
`cli_full_mode_e2e::cli_tuned_run_completes_for_every_tuning_mode`.

## Defect 2 — the search bounds cannot reach a realistic target

`ParameterBounds::default()` (`calibrate/src/antenna_config.rs:155`):

```rust
surface_rms_mm: (0.1, 2.0),
```

The upper bound is **2.0 mm**, which is exactly the nominal `surface_rms_mm` of the
`UHF_Array_Element` class in `calibrate/antenna_classes.yaml`. Two consequences:

1. The tuner cannot search *upward* from that class's nominal value at all — the starting point
   sits on the boundary.
2. `DSN_70m` (1.0 mm), `GroundStation_13m` (1.2 mm) and `UHF_Array_Element` (2.0 mm) are all at
   or near the cap, and any real antenna with a surface RMS worse than 2.0 mm is outside the
   search space entirely. 2.0 mm is a tight tolerance for a large dish; this bound looks like it
   was chosen for a small high-precision reflector and never revisited.

D12's fixture perturbs surface RMS **2.0 → 2.6 mm**, which is unreachable under these bounds.
So even with defect 1 fixed, D12's tuned test would still fail — for a second, independent
reason.

**This needs a decision, not just a code fix.** Either the bounds are widened to cover realistic
surfaces (and the relationship between class nominal values and the bound is made explicit), or
D12's fixture is changed to perturb *downward* into the valid range (e.g. 2.0 → 1.5 mm, with the
test's assertion direction flipped accordingly). The first is probably right — the bound is the
thing that looks wrong — but it is a domain call, not a test-fixture call.

**✅ Decided + fixed 2026-07-31 (maintainer): bounds derived per-class from the nominal.**
`ParameterBounds::default()` is gone; `ParameterBounds::from_class` brackets each tunable
log-symmetrically around that class's own nominal (`nominal / 5` to `nominal × 5`). This makes
the starting point interior **by construction**, for every class including ones added later,
rather than by picking better absolute numbers that a future class could again sit on. It also
tracks the physics better than one global range: what governs gain loss is `σ/λ`, not `σ` in
absolute millimetres, and a 0.3 mm precision reflector and a 2.0 mm UHF array element do not
share a plausible surface-quality range. Pinned by
`every_shipped_class_nominal_is_interior_to_its_own_bounds`, which checks the real
`antenna_classes.yaml` entries rather than a fixture.

**Correction to this section's framing:** the diagnosis "the tuner cannot search upward past
2.0 mm" was right but was never the *binding* constraint. Once defect 1 was fixed, the tuner
ran to the **lower** bound (0.1 mm), not the upper one — because of defects 3 and 4 below.
Widening the upper cap alone would have changed nothing observable.

## Defect 3 — the fixture's injected bias is confounded with surface RMS

*Added 2026-07-31, found on the first run after defect 1 was fixed.*

D12's fixture injects a systematic bias averaging **+1.22 dB** so the correction surface has a
known answer to recover. But Ruze loss is `exp(-(4πσ/λ)²)`, and this fixture is UHF
(400–700 MHz, λ = 43–75 cm), so the entire 2.0 → 2.6 mm perturbation is worth **0.0034 dB at
400 MHz and 0.0103 dB at 700 MHz**. The bias is 120–360× larger, and — being near-constant in
shape — is *confounded* with surface RMS rather than merely noisy against it. Minimising RMSE
against biased data is best served by raising predicted gain, i.e. driving surface RMS to its
**lower** bound. Measured: 0.1 mm reported against a 2.6 mm truth.

The tuner and the correction surface were being handed two known answers on one fixture, and
those answers fight. **Fixed** by giving the tuner its own bias-free fixture
(`support::generate_rows_without_bias`); the correction-surface assertions keep the biased one.
Two known answers, two fixtures.

## Defect 4 — the tuner optimised against a different model than the pipeline used

*Added 2026-07-31. This is the one that actually blocked recovery, and it is a pipeline
inconsistency rather than a test-fixture issue.*

`parameter_tuner::tune_parameters` evaluated its objective under `IntegrationParams::fast()`
("speed over accuracy"), while `main.rs::compute_model_predictions` — which computes the
residuals the correction surface is then fitted to — uses `IntegrationParams::default()`. The
two presets are not interchangeable at the angles this fixture covers:

| cone | 400 MHz | 700 MHz |
|---|---|---|
| 0° | −0.00010 dB | −0.00010 dB |
| 6° | +0.00001 dB | +0.00040 dB |
| 12° | +0.00113 dB | −0.01765 dB |
| **24°** | **−0.08816 dB** | **+0.04855 dB** |

Against a surface-RMS signal of 0.0034–0.0103 dB, the 24° mismatch is **26×** the quantity
being fitted. The tuner was therefore minimising integrator discretisation error, then handing
the resulting parameters to a pipeline that recomputed everything under a different integrator.
The deep-sidelobe rows dominate the mismatch — and those are precisely the rows D11 stopped
discarding, so this got worse when D11 landed.

**Fixed** by evaluating the objective under `IntegrationParams::default()`, matching the
prediction path. Cost: the tuned e2e run goes from ~10 s to ~20 s in a debug build, which is
the price of the tuner optimising the model that actually ships. With this in place the tuner
recovers 2.6000 mm from a 2.0 mm start in four iterations.

**Not audited here:** whether any *other* pair of call sites in the calibrate pipeline
disagrees about `IntegrationParams`. Only the tuner-vs-predictions pair was checked.

## Why it went unnoticed

- `--tune-parameters` is **off by default** (`main.rs`, `args.tune_parameters` is a bare flag),
  so every prior run of `calibrate` full mode took the untuned path.
- `parameter_tuner.rs` has unit tests, but they exercise the RMSE-evaluation machinery rather
  than driving `tune_parameters` through argmin, so the degenerate simplex is never constructed
  in a test.
- No CLI-level integration test existed for `calibrate` at all until D12 — which is exactly the
  gap D12 was created to close, and exactly the kind of defect it was expected to surface.

## Acceptance for the fix — ✅ all met 2026-07-31

- ✅ `calibrate --tune-parameters` completes without panicking for all three `--tuning-mode`
  values (`surface-only`, `surface-and-mesh`, `all`), i.e. for `N` = 1, 2 and 3 parameters.
  Pinned at CLI level by `cli_tuned_run_completes_for_every_tuning_mode` and at library level
  by `tune_parameters_completes_for_every_tuning_mode`.
- ✅ A CLI-level test drives it end to end and asserts recovery of the known injected
  perturbation. `cli_tuned_run_recovers_the_surface_rms_perturbation` is **un-ignored** and
  green, and its assertion was *strengthened* from the original directional check
  (`tuned > nominal`) to a known-answer check (`|tuned − 2.6| < 0.15 mm`), because recovery
  turned out to be exact rather than merely directional.
- ✅ Wall-clock measured, CI status decided: **runs in CI, unconditionally.** The tuned
  recovery run is ~20 s and the three-mode completion run ~18 s in a debug build; the whole
  `cli_full_mode_e2e` suite is 34.6 s. Judged worth it — this is the only end-to-end coverage
  of the tuner, and it is what caught defects 3 and 4.

## Wall-clock summary (debug build, macOS/aarch64, 2026-07-31)

| run | before | after |
|---|---|---|
| tuned recovery (4 iters) | crashed at ~0.7 s | 19.7 s |
| three-mode completion (1 iter each) | crashed | 18.3 s |
| whole `cli_full_mode_e2e` suite | — | 34.6 s |

The increase over the crash-only fix (~10 s → ~20 s for the recovery run) is defect 4's doing:
the denser `default()` integrator costs roughly 2× per objective evaluation.

## Follow-ups filed, not fixed here

- **Is `IntegrationParams` consistent elsewhere in the pipeline?** Defect 4 was found by
  inspection of one pair of call sites. Nothing systematically checks that every stage of
  calibrate evaluates the same model, and nothing would catch a future divergence.
- **`BRACKET_FACTOR = 5.0` is a judgement call, not a derived quantity.** It is wide enough
  that a design-spec nominal off by a factor of a few is recoverable and narrow enough to keep
  the search physical, but no data backs the specific value.
