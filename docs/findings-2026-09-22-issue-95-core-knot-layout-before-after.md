# Issue #95 — before/after artifact regression

**Date:** 2026-09-22
**Change under test:** the `#95` commit on `feat/95-core-knot-layout`
**Baseline:** `1d715eb` (`main`, the branch point)

Issue #95 moves knot-vector construction and validation into
`antenna_core::model::correction_surface` and requires that "existing artifacts are
semantically unchanged after normalizing timestamp, CRC, and the schema-5.1 stamp; evaluated
corrections use explicit tolerances". As with #94 this is a **migration** check — it needs a
build from before the change — so it is recorded here rather than committed as a test.

## Procedure

Identical to [the #94 record](findings-2026-09-21-issue-94-canonical-order-before-after.md):
the same two committed inputs (CR-159703 grid for full mode, NTIA SA 8002A for boresight
mode, which does fit a frequency correction), the same `build_and_calibrate` commands with
`<baseline>` = `1d715eb`, and the same harness (`calibrate/tests/before_after.rs`, reproduced
in full in that document), run with
`ARTIFACT_DIR=$S cargo test -p calibrate --test before_after -- --nocapture --test-threads=1`
from the *after* revision.

Running the harness from the after revision matters here: it decodes the **before**
artifacts through the tightened 5.2 loader, so a pass also shows that artifacts written by
the previous producers satisfy the new invariant set.

Normalization: `metadata.calibration_date` and `metadata.format_version` are blanked (the
latter moves 5.1 → 5.2 in this change), and both artifacts are re-encoded through
`encode_calibration_artifact`, which recomputes the CRC32 over the normalized payload.

## Result

| Comparison | Full mode | Boresight mode |
|---|---|---|
| `calibrate --report` JSON | byte-identical | (no report emitted) |
| Raw artifact bytes (`cmp -l`) | 14 bytes differ | 15 bytes differ |
| Normalized re-encode (timestamp, schema stamp, CRC) | **identical, 39 417 bytes** | **identical, 3 378 bytes** |
| Before artifact loads under the 5.2 invariant set | yes | yes |
| 1331 probes over interior points **and** exact axis boundaries | max `\|Δ\|` = **0.0 dB** | max `\|Δ\|` = **0.0 dB** |
| Probe magnitude (non-vacuity guard) | max `\|correction\|` = 18.07 dB | max `\|correction\|` = 0.87 dB |

The raw differences are the RFC-3339 timestamp, the `"5.1"`→`"5.2"` stamp, and the CRC32
covering them. The tolerance the harness asserts is `1e-9` dB; the measured difference is
exactly zero because the knot vectors, shapes and solve are unchanged — only *where* the
knot vectors are assembled and validated moved. Boresight mode stays order 3 (quadratic):
its frequency knot vector and coefficients are bit-identical.
