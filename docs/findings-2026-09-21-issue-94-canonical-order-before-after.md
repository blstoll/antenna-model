# Issue #94 — before/after artifact regression

**Date:** 2026-09-21
**Change under test:** `refactor(#94): fit in the artifact's canonical coefficient order`
**Baseline:** `ff89800` (`fix(#93): preserve correction support in calibration validation`)

Issue #94's acceptance criteria ask for "a deterministic before/after regression [that]
compares decoded artifacts after normalizing timestamp, CRC, and the schema-5.1 stamp". The
comparison is a **migration** check — it needs one build from before the change and one from
after — so it cannot live in the test suite once the change is merged: after the merge there
is no "before" left to fit against. This file is the durable record, with enough detail to
re-run it against any two revisions.

## What was compared

The same measurement grid, calibrated twice, once per revision. The grid itself is generated
deterministically from committed inputs, so nothing but the calibrate build differs between
the two runs.

```bash
S=/tmp/94-regression && mkdir -p "$S"

# Deterministic grid from committed inputs (run once; both calibrations consume it).
cargo build --release --bin cr159703_grid --bin calibrate
./target/release/cr159703_grid \
  --peaks antenna-model/tests/fixtures/reference_datasets/sidelobe_data/nasa_cr159703_pattern_peaks.psv \
  --classes-file calibrate/tests/fixtures/nasa_cr159703_122m_classes.yaml \
  --antenna-class NASA_CR159703_1p22m \
  --output "$S/grid.csv" --summary "$S/grid_summary.json"

# One calibration per revision; run the second after checking out the other revision
# and rebuilding, writing to $S/after.bin.
./target/release/calibrate --calibration-mode full \
  --input "$S/grid.csv" --output "$S/before.bin" \
  --antenna-id nasa_cr159703_122m --feed-id x_band \
  --antenna-class NASA_CR159703_1p22m \
  --classes-file calibrate/tests/fixtures/nasa_cr159703_122m_classes.yaml \
  --report "$S/before_report.json"
```

The two decoded `AntennaCalibration`s were then compared with `calibration_date` and
`format_version` blanked — the timestamp and the schema stamp, the two fields expected to
differ run to run. The container CRC32 is not visible after the decode; it is covered by the
byte count below instead.

## Result

| Comparison | Result |
|---|---|
| `calibrate --report` JSON | byte-identical |
| Raw artifact bytes (`cmp -l`) | **13 bytes differ** — the RFC-3339 timestamp and the CRC32 |
| Decoded structural metadata (timestamp and schema stamp normalized) | equal |
| Correction-surface shape, spline order, all four knot vectors | equal |
| Correction-surface coefficients | **bit-identical** (max `|Δ|` = `0e0`) |
| Evaluated corrections, 1331 probes over interior points **and** exact axis boundaries | max `|Δ|` = **0.0 dB** |
| Probe magnitude (non-vacuity guard) | max `|correction|` = 18.07 dB |

Coefficient agreement is exact rather than within a tolerance because the solve did not move:
the fit already accumulated its normal equations through the shared core layout (issue #99),
and #94 removed the *second copy* of the geometry rather than reordering the system. **Raw
file-byte identity is not claimed** — the artifact is timestamped, so 13 bytes differ by
construction.

## The probe harness

Run as a scratch integration test in `calibrate/tests/` and deliberately not committed, for
the reason in the opening paragraph. To reproduce, place this in
`calibrate/tests/before_after.rs` and run it with `ARTIFACT_DIR` pointing at `$S`:

```rust
let mut before = load_calibration_artifact(format!("{dir}/before.bin")).expect("before");
let mut after = load_calibration_artifact(format!("{dir}/after.bin")).expect("after");
before.metadata.calibration_date = String::new();
after.metadata.calibration_date = String::new();
before.metadata.format_version = String::new();
after.metadata.format_version = String::new();
// … compare the correction surfaces field by field, then set both to `None` and
// assert_eq! the rest of the artifact; then evaluate both through
// FittedCorrectionSurface::from_model4d over an 11 x 11 x 11 grid spanning each axis's
// fitted support inclusive of its exact boundaries.
```

The permanent replacements for the sampled equivalence test this regression retires live in
`calibrate/tests/artifact_export_integration_test.rs`:
`the_served_surface_is_exactly_the_fitted_surface_after_a_service_load` (exact equality
between the fitted surface and the decoded served surface) and
`every_wire_temperature_slab_is_the_canonical_coefficient_vector_verbatim` (the wire order
itself, which a round trip alone cannot see).
