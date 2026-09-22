# Issue #94 — before/after artifact regression

**Date:** 2026-09-21 (boresight pair added 2026-09-22)
**Change under test:** the `#94` commits on `fix/94-fitter-canonical-order`
**Baselines:** `ff89800` for the full-mode pair (the branch point); `4875e30` for the
boresight pair (the commit before the boresight producer moved onto the shared wire adapter)

Issue #94's acceptance criteria ask for "a deterministic before/after regression [that]
compares decoded artifacts after normalizing timestamp, CRC, and the schema-5.1 stamp". The
comparison is a **migration** check — it needs one build from before the change and one from
after — so it cannot live in the test suite once the change is merged: after the merge there
is no "before" left to fit against. This file is the durable record: the complete harness,
the exact commands, and the measured result, so any reviewer can re-run it against any two
revisions.

## What was compared

Two artifact pairs, one per producer the change touches. Both inputs are committed fixtures
and both producers are deterministic given their input, so the calibrate build is the only
thing that differs between the two runs of a pair.

- **Full mode** — the CR-159703 grid, generated from the committed digitized peaks.
- **Boresight mode** — the NTIA SA 8002A fixture, which is the repo's cover for the
  *corrected* boresight branch: its published Rx/Tx gains are mutually inconsistent, so the
  residual clears the 0.5 dB threshold and a frequency correction is actually fitted. (The
  Andrew 43998 fixture produces no correction surface and would prove nothing here.)

```bash
S=/tmp/94-regression && mkdir -p "$S"

build_and_calibrate() {           # $1 = suffix: "before" or "after"
  cargo build --release --bin cr159703_grid --bin calibrate

  # Full mode. The grid is generated once, from the "before" build, and both
  # calibrations consume that same CSV.
  if [ ! -f "$S/grid.csv" ]; then
    ./target/release/cr159703_grid \
      --peaks antenna-model/tests/fixtures/reference_datasets/sidelobe_data/nasa_cr159703_pattern_peaks.psv \
      --classes-file calibrate/tests/fixtures/nasa_cr159703_122m_classes.yaml \
      --antenna-class NASA_CR159703_1p22m \
      --output "$S/grid.csv" --summary "$S/grid_summary.json"
  fi
  ./target/release/calibrate --calibration-mode full \
    --input "$S/grid.csv" --output "$S/$1.bin" \
    --antenna-id nasa_cr159703_122m --feed-id x_band \
    --antenna-class NASA_CR159703_1p22m \
    --classes-file calibrate/tests/fixtures/nasa_cr159703_122m_classes.yaml \
    --report "$S/${1}_report.json"

  # Boresight mode.
  ./target/release/calibrate --calibration-mode boresight \
    --input calibrate/tests/fixtures/ntia_84_164_sa_8002a_10m_boresight.csv \
    --output "$S/bs_$1.bin" \
    --antenna-id ntia_sa_8002a_10m --feed-id c_band \
    --design-specs calibrate/tests/fixtures/ntia_sa_8002a_10m_design_specs.yaml
}

git checkout <baseline> && build_and_calibrate before
git checkout <branch>   && build_and_calibrate after
```

## How the three normalizations are done

- **Timestamp** — `metadata.calibration_date` is blanked on both decoded artifacts.
- **Schema-5.1 stamp** — `metadata.format_version` is blanked on both.
- **CRC32** — *not* excused, and not compared directly: both normalized artifacts are
  **re-encoded** through the production framing (`encode_calibration_artifact`, the one
  writer per roadmap D27), which recomputes the CRC over the normalized payload. The two
  resulting byte vectors are then compared. A payload difference anywhere — including one
  that happened to leave lengths equal — shows up as a byte mismatch.

## Result

| Comparison | Full mode | Boresight mode |
|---|---|---|
| `calibrate --report` JSON | byte-identical | (no report emitted) |
| Raw artifact bytes (`cmp -l`) | 15 bytes differ | 12 bytes differ |
| Normalized re-encode (timestamp, schema stamp, CRC) | **identical, 39 414 bytes** | **identical, 3 378 bytes** |
| Correction-surface shape, order, all four knot vectors | equal | equal |
| Correction-surface coefficients | bit-identical | bit-identical |
| 1331 probes over interior points **and** exact axis boundaries | max `\|Δ\|` = **0.0 dB** | max `\|Δ\|` = **0.0 dB** |
| Probe magnitude (non-vacuity guard) | max `\|correction\|` = 18.07 dB | max `\|correction\|` = 0.87 dB |

The raw byte differences are the RFC-3339 timestamp and the CRC32 that covers it; they
vanish under the normalization above. Coefficient agreement is exact rather than within a
tolerance because the solve did not move: the fit already accumulated its normal equations
through the shared core layout (issue #99), and #94 removed the *second copy* of the geometry
rather than reordering the system. **Raw file-byte identity is not claimed** — the artifact
is timestamped, so bytes differ by construction.

## The harness, in full

Place this at `calibrate/tests/before_after.rs` and run it with
`ARTIFACT_DIR=$S cargo test -p calibrate --test before_after -- --nocapture --test-threads=1`.
It is reproduced here rather than committed for the reason in the opening paragraph: with
both revisions merged there is no "before" build for it to run against, and a test that
silently passes because its inputs are absent is the failure mode this repo calls the D13
signature.

```rust
//! Issue #94 before/after decoded-artifact regression.
use antenna_core::data::loader::encode_calibration_artifact;
use antenna_core::data::types::AntennaCalibration;
use antenna_core::model::FittedCorrectionSurface;

/// Blank the two fields that differ run to run: the RFC-3339 timestamp and the schema stamp.
/// Re-encoding the normalized artifact recomputes the container CRC32 over identical
/// payloads, which is how the CRC is normalized rather than excused.
fn normalize(calibration: &mut AntennaCalibration) {
    calibration.metadata.calibration_date = String::new();
    calibration.metadata.format_version = String::new();
}

fn compare(stem_before: &str, stem_after: &str) {
    let dir = std::env::var("ARTIFACT_DIR").expect("ARTIFACT_DIR");
    let load = |stem: &str| {
        antenna_core::data::loader::load_calibration_artifact(format!("{dir}/{stem}.bin"))
            .unwrap_or_else(|e| panic!("load {stem}: {e}"))
    };
    let mut before = load(stem_before);
    let mut after = load(stem_after);
    normalize(&mut before);
    normalize(&mut after);

    let before_surface = before.correction_surface.clone();
    let after_surface = after.correction_surface.clone();

    // 1. Whole decoded artifact, including the correction surface, byte for byte after
    //    normalization — timestamp, schema stamp and CRC all accounted for.
    let before_bytes = encode_calibration_artifact(&before).expect("re-encode before");
    let after_bytes = encode_calibration_artifact(&after).expect("re-encode after");
    assert_eq!(
        before_bytes, after_bytes,
        "normalized artifacts must re-encode to identical bytes, CRC included"
    );
    println!(
        "[{stem_before} vs {stem_after}] normalized re-encode: {} bytes, identical",
        before_bytes.len()
    );

    // 2. Evaluated corrections over interior points and exact axis boundaries.
    let (Some(b_model), Some(a_model)) = (before_surface, after_surface) else {
        println!("[{stem_before} vs {stem_after}] no correction surface to probe");
        return;
    };
    let b = FittedCorrectionSurface::from_model4d(&b_model).expect("decode before surface");
    let a = FittedCorrectionSurface::from_model4d(&a_model).expect("decode after surface");
    let order = b_model.spline_order as usize;
    let axis = |knots: &[f64]| -> Vec<f64> {
        let (lo, hi) = (knots[order - 1], knots[knots.len() - order]);
        (0..=10).map(|i| lo + (hi - lo) * i as f64 / 10.0).collect()
    };

    let (mut max_delta, mut max_value, mut probes) = (0.0_f64, 0.0_f64, 0usize);
    for clock in axis(&b_model.knots_azimuth) {
        for cone in axis(&b_model.knots_elevation) {
            for freq in axis(&b_model.knots_frequency) {
                let bv = b.evaluate(clock, cone, freq).correction_db().expect("in support");
                let av = a.evaluate(clock, cone, freq).correction_db().expect("in support");
                assert!(bv.is_finite() && av.is_finite(), "non-finite correction");
                max_delta = max_delta.max((bv - av).abs());
                max_value = max_value.max(bv.abs());
                probes += 1;
            }
        }
    }
    println!(
        "[{stem_before} vs {stem_after}] probes={probes} max |Δ| = {max_delta:e} dB, \
         max |correction| = {max_value:e} dB"
    );
    assert!(max_value > 0.1, "probes must carry a real correction");
    assert!(max_delta < 1e-9, "corrections must agree within 1e-9: {max_delta:e}");
}

#[test]
fn full_mode_artifacts_are_unchanged() {
    compare("before", "after");
}

#[test]
fn boresight_mode_artifacts_are_unchanged() {
    compare("bs_before", "bs_after");
}
```

## What replaces it permanently

The sampled fit-versus-serve equivalence test this regression retires is replaced by two
tests in `calibrate/tests/artifact_export_integration_test.rs`, which need no "before" build
and run on every CI pass:

- `the_served_surface_is_exactly_the_fitted_surface_after_a_service_load` — exact equality
  between the fitted surface and the surface decoded back off disk.
- `every_wire_temperature_slab_is_the_canonical_coefficient_vector_verbatim` — the wire
  order itself, which a round trip alone cannot see: a reindexing loop whose decode inverted
  it would round-trip perfectly.
