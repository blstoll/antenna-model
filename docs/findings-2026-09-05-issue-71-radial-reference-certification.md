# Issue #71 — certified radial reference anchors

**Date:** 2026-09-05

**Scope:** `p12_mode_path_radial_convergence_anchors` in
`antenna-model/tests/reference_validation.rs`
**Result:** the four runtime dense integrations were replaced by stored field-level anchors.
The 0.05 dB production acceptance threshold and all four P12 cases are unchanged.

## Quantities and geometries

All frequencies below are in **Hz**, lengths in **metres** unless marked `mm`, angles in
**degrees**, and reference levels in **dB re 1** (`20 log10(|field|)`). The two UHF rows are
the same geometry viewed at different E-clock angles around boresight; retaining both matters
because the illumination is asymmetric.

| Case | Reflector and surface | Feed | Mesh | Frequency | θ | φ |
|---|---|---|---|---:|---:|---:|
| `gs_3.7m/x_band_feed` | D=3.7, f=1.85, RMS=1.5 mm | q=2.04, position=(0.05, 0, 1.85), phase-centre offset=0 | square, spacing=5 mm, wire=0.5 mm | 8,400,000,000 | 5.00 | 0 |
| `dsn_34m/x_band` | D=34, f=13.6, RMS=0.25 mm | q=1.14, position=(0.15, 0, 13.6), recorded phase-centre offset=0.015 | none | 8,450,000,000 | 0.10 | 0 |
| `D12 UHF fixture φ=0` | D=8, f=3.6, RMS=2.0 mm | q=5.0, position=(0, 0, 3.6), asymmetry=1.1, phase-centre offset=0 | square, spacing=10 mm, wire=1 mm | 600,000,000 | 16.00 | 0 |
| `D12 UHF fixture φ=90` | same | same | same | 600,000,000 | 16.00 | 90 |

The GS and DSN geometries match their enabled entries in
`calibration_data/antennas.yaml`; the UHF geometry matches the D12 calibration fixture and
`calibrate/antenna_classes.yaml`. The test continues to construct/load those geometries rather
than treating this table as configuration.

## Numerical method

The certification harness is
`integration::p12_radial_diagnostic::issue_71_certifies_smaller_radial_references` in
`antenna-core/src/model/integration.rs`. It runs inside that module so it can use the same
private azimuthal-mode implementation as production without exposing a test API.

For each geometry it:

1. derives the production azimuthal FFT size `n_phi` and mode truncation once;
2. includes the production `M+1` probe mode (`m_probe`) in every leg;
3. holds `n_phi` and `m_probe` fixed while evaluating the composite-Simpson radial ladder
   `n_rho = 1025, 2049, 4097, 8193, 16385, 32769`;
4. compares the 2,049-point candidate with the 32,769-point endpoint; and
5. conservatively estimates endpoint uncertainty as the absolute 16,385-to-32,769 field-level
   difference.

This isolates radial quadrature error: azimuthal sizing and mode truncation cannot move between
legs. The production `IntegrationResult::converged` flag is deliberately not used as evidence.
The harness independently requires candidate error below **0.005 dB** (10× inside the retained
0.05 dB regression threshold) and dense-tail uncertainty below **0.0005 dB** (100× inside).

## Convergence results and stored anchors

| Case | `n_phi` | `m_probe` | 2,049-point candidate | 32,769-point stored anchor | Candidate error | Dense-tail uncertainty |
|---|---:|---:|---:|---:|---:|---:|
| `gs_3.7m/x_band_feed` | 64 | 21 | -61.980025246607 | **-61.980025126101** | 1.20507e-7 dB | 1.68e-10 dB |
| `dsn_34m/x_band` | 80 | 19 | -10.622071756849 | **-10.622071728263** | 2.8587e-8 dB | 2e-12 dB |
| `D12 UHF fixture φ=0` | 64 | 16 | -51.720297440700 | **-51.720297408702** | 3.1998e-8 dB | 9e-12 dB |
| `D12 UHF fixture φ=90` | 64 | 16 | -57.676029598053 | **-57.676029531433** | 6.6620e-8 dB | 2.2e-11 dB |

The complete ladder was smooth for every case. The largest candidate error is
**1.21e-7 dB**, over 400,000× smaller than the 0.05 dB production threshold. The prior runtime
reference set `min_rho_points=16385`; the production refinement then evaluated both 16,385 and
32,769 points and returned the 32,769-point fine leg. The stored values above therefore preserve
that reference rather than substituting a looser number.

The regression test still records the historical pre-P12 errors—0.82 dB, 1.17 dB, 7.08 dB and
3.85 dB—and still requires each production result to report convergence. The removed-phi-cap
test separately retains its three 0°, 1° and 3° anchors, including the boresight aliasing witness
and off-axis mode margin; the nearby production-geometry phi-sufficiency control is unchanged.

## Regeneration procedure

Run from the workspace root on an otherwise idle machine:

```bash
cargo test --release -p antenna-core --lib \
  issue_71_certifies_smaller_radial_references -- \
  --ignored --nocapture --test-threads=1
```

The harness prints every ladder value and verifies both uncertainty budgets and the stored dense
anchors. A geometry, Fourier-sizing, mode-truncation, Bessel, or quadrature change may legitimately
move an anchor; in that case inspect the entire ladder, explain the numerical change here, and
update both the harness and `reference_validation.rs` together. Do not regenerate from a normal
production integration or from its convergence verdict.

## Comparable timing

Measured warm on the same M-series laptop with Rust's **debug test profile** (unoptimized +
debuginfo), nextest `profile.full`, and exactly one nextest test thread; compilation is excluded
from the nextest summary. The command selected only the radial and removed-phi-cap tests:

```bash
cargo nextest run -p antenna-model --profile full \
  --test reference_validation --test-threads 1 \
  -E 'test(/p12_(mode_path_radial_convergence_anchors|phi_cap_removed_steered_feed_matches_stored_anchors)/)'
```

| Stage | Radial test | Phi-cap test | Two-test summary |
|---|---:|---:|---:|
| Before issue #71 | 14.518 s | 15.154 s | 29.672 s |
| After one-result deduplication only | 14.471 s | 7.571 s | 22.042 s |
| After stored radial references | 0.087 s | 7.581 s | 7.668 s |

The first phase removes seven duplicate production integrations and saves **7.630 s** without
changing reference work. The second phase removes only the four now-certified runtime dense
references; the radial test itself saves a further **14.384 s**. No cases, thresholds,
convergence assertions, or CI coverage moved. The separate certification command is intentionally
release-optimized and
single-threaded; after a warm build its test body took **0.98 s**. Timing claims above do not mix
that release measurement with the debug regression-test measurements.
