# Issue #72 — testing the radial-cap control cheaply, without giving up the real-cap certification

**Date:** 2026-09-06

**Scope:** the azimuthal-mode path's radial refinement control in
`antenna-core/src/model/integration.rs`.
**Result:** the refinement loop and the two-axis error combination moved out of
`integrate_aperture` into two internal functions, `refine_radial` and `mode_path_result`,
with the density limit and the sweep operation as parameters. Production numerical output is
**bitwise unchanged** on nine geometries, including the capped one. Eight scripted tests now
cover the control in **0.026 s** in the default developer tier; the expensive real-cap
certification is retained unchanged and stays CI-blocking.

## 1. What the defect was, and why cheap coverage was hard

Until 2026-08-01 the mode path's refinement loop guarded on
`n_rho >= RADIAL_POINTS_SAFETY_MAX`. When `radial_points_for` returned a density that was
already clamped to the cap, the loop `break`ed **before** running any coarse/fine comparison,
so `radial_error` kept its `0.0` initial value. `converged` was still correctly `false` — the
number was never served silently — but `error_estimate` reported the azimuthal axis alone,
understating the total by ~7 orders of magnitude on exactly the geometries whose density was
known to be insufficient. That contradicted P12's combination decision, whose justification is
that summing the two axes never understates.

The behaviour is invisible to the two obvious assertions:

- **`converged == false`** is true *under the defect as well*. Non-convergence is not evidence.
- **`error_estimate > 0`** is satisfied by the azimuthal term alone (~1e-13 on the retained
  fixture), so it passes under the defect too.

The only assertion that discriminates is that a strictly finer leg is **requested** at the cap
and that the reported radial error is the difference between the legs. Before this change the
only way to make that assertion was to reach the production cap with a real geometry —
`mode_path_reports_a_radial_error_even_when_the_density_cap_binds`, ~3 × 65,537 radial samples,
**1.85 s release / ~35 s debug**, which is why it lives in the slow tier.

## 2. The seam

`integrate_aperture`'s mode branch previously inlined: the baseline sweep, the refinement loop,
the mode-truncation self-check and the error combination. It now calls

```rust
let radial = refine_radial(
    n_start,
    RADIAL_POINTS_SAFETY_MAX,   // the density limit, explicit
    params,
    HANKEL_SELF_CHECK_RTOL,
    |n_rho| { /* one full Jm sweep at n_rho; returns (ModeSweep, work units) */ },
)?;
Ok(mode_path_result(&radial, azimuthally_resolved, params))
```

Both parameters are what make the control testable: a test passes a limit of `9` and a closure
that returns scripted `ModeSweep` values, so the same code that runs in production is exercised
without evaluating a single aperture sample. `radial_check_points` gained a
`radial_check_points_within(n1, limit)` form for the same reason and remains a wrapper that
passes the production constant.

Deliberately **not** done, per the acceptance criteria:

- No public API and no configuration knob. Both functions are private to the module; no
  `IntegrationParams` field, `service.yaml` key or CLI flag was added or changed.
- No change to sampling limits, budgets or error policy. `RADIAL_POINTS_SAFETY_MAX`,
  `MAX_RADIAL_REFINEMENTS`, `HANKEL_SELF_CHECK_RTOL`, `MODE_SELF_CHECK_RTOL` and the S3
  wall-clock budget are untouched, and production passes the same values it always passed.
- The convergence verdict inside the loop is now taken by the existing `self_check` helper —
  the same one the symmetric branch uses — rather than by an open-coded copy of the identical
  expression. `self_check`'s `diff <= rtol·max(|fine|, atol) || diff < atol` against
  `magnitude = |fine|` is term-for-term what the loop computed after assigning `sweep = fine`,
  which the bitwise comparison in §4 confirms.

## 3. Coverage added, and what it does not replace

Eight tests in `antenna-core/src/model/integration.rs`, all in the **default** tier:

| Test | What it pins |
|---|---|
| `refinement_compares_a_finer_leg_when_the_limit_binds_from_the_first_sweep` | starting **at** the limit still requests `[9, 17, 19]` and reports the last coarse/fine difference (0.5), not 0.0 |
| `refinement_reports_no_error_only_when_no_finer_leg_exists` | the one honest zero: starting at the `2·limit+1` ceiling asks for one leg and stays non-converged |
| `refinement_exhausts_the_doubling_budget_and_reports_the_last_disagreement` | `MAX_RADIAL_REFINEMENTS + 1` legs, the odd `2N−1` ladder, honest last disagreement, not an error |
| `refinement_stops_at_agreement_and_returns_the_fine_leg` | agreement stops the ladder and the **fine** leg is returned |
| `refinement_propagates_a_failing_leg` | a failing sweep (the S3 budget expiring) aborts rather than being absorbed into a verdict |
| `mode_path_result_sums_the_radial_and_azimuthal_axes` | `error_estimate` = radial + azimuthal, both terms present; field is the finest sweep's total |
| `mode_path_result_gates_on_each_axis_and_aliasing_adds_no_error` | each of the three axes gates `converged`; φ' aliasing contributes no magnitude |
| `radial_check_points_ceilings_just_above_the_production_cap` | on the **real** `RADIAL_POINTS_SAFETY_MAX`: a strictly finer leg exists at the cap, the ceiling is a fixed point, the count stays odd |

Two of the eight are beyond the four cases the issue enumerates: `refinement_propagates_a_failing_leg`
and `mode_path_result_gates_on_each_axis_and_aliasing_adds_no_error`. They cover the seam's two
remaining exits — a failing sweep and the φ' axis — for the same ~3 ms, and are called out here
so the difference from the acceptance list is deliberate and visible rather than silent.
`refinement_reports_no_error_only_when_no_finer_leg_exists` covers a state production cannot
reach (`radial_points_for` clamps `n_start` to the limit); it pins the loop's termination
condition and does **not** discriminate the historical defect, which its doc comment says.

**Discrimination, measured rather than argued.** The historical guard
(`if n_rho >= limit { break; }`) was reintroduced and the block re-run:

- `refinement_compares_a_finer_leg_when_the_limit_binds_from_the_first_sweep` **FAILS**:
  `left: [9] right: [9, 17, 19]`.
- The other seven **PASS**. In particular the non-convergence assertions and
  `mode_path_result_sums_the_radial_and_azimuthal_axes` (whose azimuthal term is nonzero)
  pass under the defect — which is the point of the acceptance criterion: neither
  non-convergence alone nor a nonzero azimuthal-only estimate constitutes replacement
  coverage.
- The retained real-cap certification also FAILS under the reintroduced defect (0.96 s release,
  the fixture aborts at the first assertion) and PASSES after restoring (1.85 s release).

**What the cheap tests cannot do**, and why the expensive one stays: nothing in the scripted
block evaluates an aperture sample, so no test there can show that the production constant is
reachable by a real geometry, that the two capped legs genuinely disagree, or by how much.
`mode_path_reports_a_radial_error_even_when_the_density_cap_binds` is unchanged — same 750 m /
40 GHz fixture, same assertion that `radial_points_for` actually returns
`RADIAL_POINTS_SAFETY_MAX`, and the same independently orchestrated coarse/fine witness
(`azimuthal_mode_field` called directly at `n_rho` and `radial_check_points(n_rho)`) rather
than an oracle derived from the reported error. It stays in the `.config/nextest.toml`
slow-tier exclusion list and therefore stays CI-blocking under `profile.full`. No smaller
injected cap was substituted for it.

Symmetric-branch coverage is untouched: `unconverged_is_flagged_not_silently_returned`,
`radial_points_for_gbt_qband_is_tens_of_thousands` and
`radial_density_scales_with_dlambda_sintheta` are unmodified, and the symmetric branch of
`integrate_aperture` was not edited.

## 4. No production numerical regression

A harness printed `field.re`, `field.im`, `error_estimate` (as raw IEEE-754 bit patterns),
`num_evaluations` and `converged` for nine geometries covering both branches, both convergence
outcomes, and the capped case, using `IntegrationParams { time_budget: None, ..adaptive() }`.
Release build (`cargo test --release -p antenna-core --test tmp_snapshot_72 -- --nocapture`),
run on `main` and again on this branch. It is reproduced in §6 so the comparison can be redone
rather than taken on trust; it is not committed, because it asserts nothing — its only output is
the table below, which a rerun on a later commit would have to be diffed against by hand.

| Case | field.re (bits) | field.im (bits) | error (bits) | evals | conv |
|---|---|---|---|---:|---|
| `sym_1m_x_boresight` | `3fc747f540b5a510` | `0000000000000000` | `3e87b1c582f00000` | 98 | true |
| `sym_1m_x_5deg` | `3f76c308e1f11362` | `bf336892da20ffc6` | `3eaebba9df930346` | 98 | true |
| `sym_10m_x_20deg` | `bf0db64174b91f56` | `3f53c1bf40ff4d09` | `3ed71cc396813448` | 1,172 | true |
| `mode_1m_offset_2deg` | `3f89eb3535558429` | `3f53f351628881d3` | `3ea9079e9d97b6c5` | 7,644 | true |
| `mode_asym_illum_3deg` | `3fae98a51b40d470` | `beafae504fa66b5d` | `3e9eeae820853d02` | 7,938 | true |
| `mode_34m_ka_5deg` | `bf2b4d1fa46df8ce` | `bf37ff3f640736a0` | `3ed6b1edf95a9a1b` | 706,680 | true |
| `mode_12m_uhf_40deg` | `3f9ac22fad3bc672` | `3fa5a699dbb0dd74` | `3f3e1629d2f216f5` | 11,248 | true |
| `sym_gbt_q_90deg` | `3f18116ceb718393` | `bf2549d98e8a4004` | `3ea8e169d33e5a4a` | 190,052 | true |
| `capped_mode_cap_binds` | `bfe2498001582442` | `3fd3ba246a301524` | `3ff34567e9fbaae6` | 24,248,690 | false |

`diff` of the before and after tables is empty: every field, error estimate, evaluation count
and convergence flag is **bitwise identical**. (Re-confirmed after the review pass that dropped
`refine_radial`'s redundant tolerance-floor parameter and renamed `mode_path_verdict` →
`mode_path_result`.)
The suite's own numerical pins (`p12_mode_path_radial_convergence_anchors`,
`p12_phi_cap_removed_steered_feed_matches_stored_anchors`, the `reference_validation` sweeps and
the `calibrate` known-answer scenarios) all pass unchanged.

## 5. Timings

Warmed debug build, `cargo nextest run` on the reference 8-core laptop, default profile unless
stated. Figures are from full concurrent runs, not standalone.

| Measurement | Before | After |
|---|---:|---:|
| `-p antenna-core`, default profile | 349 tests, **6.188 s** | 357 tests, **6.156 s** |
| `--workspace`, default profile | 1,107 tests, **23.820 s** | 1,115 tests, **24.090 s** |
| the eight new tests alone (`-E` filtered, debug) | — | **0.026 s** |
| `mode_path_reports_a_radial_error_even_when_the_density_cap_binds` (release, `profile full`) | 1.85 s | **1.85 s** |

The eight tests add no measurable wall time: the default-tier `antenna-core` figure moved by
−0.03 s, inside run-to-run noise, and the filtered 0.026 s for all eight is dominated by test
process startup. This is what issue #74 needs in place before it can move the actual-cap
certification into an optimized lane: the control flow is now covered in the debug tier at
essentially zero cost, so that lane's job is narrowed to the numerical certification itself.

## 6. The no-regression harness

Dropped into `antenna-core/tests/tmp_snapshot_72.rs` and run with
`cargo test --release -p antenna-core --test tmp_snapshot_72 -- --nocapture`, on `main` and on
the branch; the two outputs are compared with `diff`.

```rust
use antenna_core::model::geometry::{
    AntennaConfiguration, FeedParameters, FeedPosition, ReflectorGeometry,
};
use antenna_core::model::integration::{integrate_aperture, IntegrationParams};

fn cfg(d: f64, f: f64, rms: f64, off: f64, q: f64, asym: f64) -> AntennaConfiguration {
    let reflector = ReflectorGeometry::new(d, f, rms).unwrap();
    let mut pos = FeedPosition::at_focus(f);
    pos.x = off;
    let feed = FeedParameters::new(pos, q, 0.0, asym).unwrap();
    AntennaConfiguration::new("snap".into(), "Snap".into(), reflector, feed, None).unwrap()
}

#[test]
fn snapshot() {
    let pi = std::f64::consts::PI;
    let cases: Vec<(&str, AntennaConfiguration, f64, f64)> = vec![
        ("sym_1m_x_boresight", cfg(1.0, 0.5, 0.0, 0.0, 8.0, 1.0), 8.4e9, 0.0),
        ("sym_1m_x_5deg", cfg(1.0, 0.5, 0.0, 0.0, 8.0, 1.0), 8.4e9, 5f64.to_radians()),
        ("sym_10m_x_20deg", cfg(10.0, 6.0, 0.0005, 0.0, 8.0, 1.0), 8.4e9, 20f64.to_radians()),
        ("mode_1m_offset_2deg", cfg(1.0, 0.5, 0.0, 0.02, 8.0, 1.0), 8.4e9, 2f64.to_radians()),
        ("mode_asym_illum_3deg", cfg(1.0, 0.5, 0.0, 0.0, 8.0, 1.5), 8.4e9, 3f64.to_radians()),
        ("mode_34m_ka_5deg", cfg(34.0, 11.0, 0.0002, 0.05, 3.0, 1.0), 32.0e9, 5f64.to_radians()),
        ("mode_12m_uhf_40deg", cfg(12.0, 4.8, 0.001, 0.1, 3.0, 1.0), 0.45e9, 40f64.to_radians()),
        ("sym_gbt_q_90deg", cfg(100.0, 60.0, 0.000_275, 0.0, 3.15, 1.0), 43.0e9, pi / 2.0),
        // The fixture of the retained slow-tier certification: the density cap binds here.
        ("capped_mode_cap_binds", cfg(750.0, 375.0, 0.0, 0.001, 2.0, 1.0), 40.0e9, pi / 2.0),
    ];
    for (name, config, f_hz, theta) in cases {
        let p = IntegrationParams { time_budget: None, ..IntegrationParams::adaptive() };
        let r = integrate_aperture(theta, 0.0, &config, f_hz, &p).unwrap();
        println!(
            "{name}\tre={:016x}\tim={:016x}\terr={:016x}\tevals={}\tconv={}",
            r.field.re.to_bits(),
            r.field.im.to_bits(),
            r.error_estimate.to_bits(),
            r.num_evaluations,
            r.converged
        );
    }
}
```
