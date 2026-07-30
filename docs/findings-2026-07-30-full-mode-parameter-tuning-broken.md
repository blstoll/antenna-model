# Finding 2026-07-30 — full-mode `--tune-parameters` crashes, and its bounds cannot reach the target anyway

**Found by:** roadmap unit D12 (calibrate CLI end-to-end test), Task 5, on the first attempt to
drive `calibrate --tune-parameters` end to end.

**Severity:** the documented calibration workflow's step 3 is **non-functional**. Not a
served-path defect — it affects artifact *production*, not query serving — but it means no
full-mode artifact has ever been produced with tuned parameters, and none can be until this is
fixed.

**Status:** filed, not fixed. Outside D12's charter (D12 is a test unit). D12's tuned test is
committed `#[ignore]`d with the reproduction command, ready to enable once this lands.

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

## Why it went unnoticed

- `--tune-parameters` is **off by default** (`main.rs`, `args.tune_parameters` is a bare flag),
  so every prior run of `calibrate` full mode took the untuned path.
- `parameter_tuner.rs` has unit tests, but they exercise the RMSE-evaluation machinery rather
  than driving `tune_parameters` through argmin, so the degenerate simplex is never constructed
  in a test.
- No CLI-level integration test existed for `calibrate` at all until D12 — which is exactly the
  gap D12 was created to close, and exactly the kind of defect it was expected to surface.

## Acceptance for the fix

- `calibrate --tune-parameters` completes without panicking for all three `--tuning-mode` values
  (`surface-only`, `surface-and-mesh`, `all`), i.e. for `N` = 1, 2 and 3 parameters.
- A CLI-level test drives it end to end and asserts the tuner moves the parameter toward a known
  injected perturbation — D12's `cli_tuned_run_recovers_the_surface_rms_perturbation` is written
  and `#[ignore]`d for this purpose; un-ignore it, having first resolved the bounds question
  above and adjusted the fixture's perturbation direction if that is the chosen route.
- Wall-clock cost is measured and the test's CI status decided on that measurement (D12 Task 5's
  original charter, which could not be carried out because the run never completed).
