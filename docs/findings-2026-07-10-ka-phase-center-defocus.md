# Finding: Ka-band under-prediction is phase-center axial defocus, not spillover

**Date:** 2026-07-10
**Status:** Decided 2026-07-10 — option 2 (**model auto-refocus**); execution = roadmap
unit **P7** (`docs/roadmap-2026-07-work-units.md`).
**Discovered by:** the DSN reference-validation harness (`antenna-model/tests/reference_validation.rs`)
while grounding the feed-taper fix.

> ⚠️ **Naming note.** This was initially logged as a "spillover frequency-dependence"
> finding. That hypothesis is **disproven** below — spillover is frequency-independent here.
> The real cause is `phase_center_offset_m` acting as a 1/λ-scaling axial defocus.

---

## TL;DR

After correcting the over-tapered feed `q_factor` values, the uncalibrated model matched the
measured DSN 34-m X-band peak gain to −0.73 dB but stayed **−3.52 dB low at Ka-band** (32 GHz).

Decomposition shows the Ka shortfall is **entirely** an axial-defocus loss driven by the
design-spec `phase_center_offset_m`, whose penalty scales with 1/λ:

| band | freq | D/λ | phase-center offset | spillover frac | Ruze η | illumination η | residual vs DSN |
|------|------|-----|--------------------|----------------|--------|----------------|-----------------|
| X    | 8420 MHz  | 955  | 0.015 m = 0.42λ | 0.171 | 0.992 | **0.739** | −0.62 dB |
| Ka   | 32000 MHz | 3629 | 0.008 m = 0.85λ | 0.171 | 0.894 | **0.404** | −3.40 dB |
| Ka   | 32000 MHz | 3629 | **0.000 m** (test) | 0.171 | 0.894 | **0.886** | ≈ 0 dB |

(Spillover frac is 0.171 after the `estimate_spillover` q-truncation fix — see below; it was
reported as 0.192 in the original probe, which does not change any conclusion here.)

Zeroing the phase-center offset recovers the Ka illumination efficiency from 0.404 → 0.886 and
closes the residual to ~0 dB. The same mechanism costs X-band ~0.8 dB (0.886 → 0.739), i.e.
**most of X-band's −0.73 dB residual is also phase-center defocus.** With a focused feed
(`phase_center_offset_m ≈ 0`) *and* the corrected q-factors, the physics model reproduces the
real DSN 34-m to ~0.1 dB at **both** bands.

---

## What it is NOT: spillover (disproven)

The first hypothesis was that the modeled spillover grew with frequency. It does not:

- `estimate_spillover()` (`antenna-model/src/model/edge_cases.rs:170`) depends only on
  `f/D`, feed `q_factor`, and feed offset — **there is no wavelength term.** The returned
  spillover fraction is **0.171 at both X and Ka** for this geometry.
- The apparent "~19% at Ka vs ~4% at X" in the earlier note was a mis-decomposition: the same
  spillover applies at both bands; what actually differed was the aperture-integral
  illumination efficiency (0.739 vs 0.404), which I had folded into "taper" without isolating.

(Secondary bug found and **fixed** in the same function during review: `cos_edge.powi((q as i32) + 1)`
truncated a fractional `q` — e.g. q=1.14 → exponent 2 instead of 2.14 — which mis-stated the
spillover as 0.192 instead of 0.171 for every uncalibrated served gain (~0.1 dB). Now `powf(q + 1.0)`;
regression: `edge_cases.rs::test_spillover_honors_fractional_q`.)

## Root cause: `phase_center_offset_m` → axial defocus

`antenna-model/src/model/integration.rs:527`:

```rust
let feed_axial_offset =
    config.feed.position.z - config.reflector.focal_length + config.feed.phase_center_offset;
```

The phase-center offset is added to the feed's axial displacement from the focal point and fed
into `phase_feed_displacement`, producing a quadratic (defocus) aperture-phase error. The edge
path error is ≈ `offset · (1 − cos ψ_edge)`; at ψ_edge ≈ 64° (f/D 0.4) that is ≈ 0.56·offset,
which in **wavelengths** is 0.56·offset/λ — hence the 1/λ growth: 0.008 m is 0.27λ of edge
error at Ka vs 0.06λ at X.

**This is intentional and tested** — see `test_phase_center_offset_produces_defocus_loss`
(`integration.rs:990`). So it is *not* a code bug in the phase model. The question is one of
**config realism / operating assumption**, not correctness of the defocus math.

## Why it matters — and the decision to make

A feed's phase center is generally *placed at the focal point* in operation; large deep-space
antennas (incl. the DSN 34-m) refocus per band (subreflector/feed axial positioning). So a real,
operated antenna does **not** exhibit this defocus — its residual phase-center offset is ≈ 0.

Our design specs, by contrast, carry `phase_center_offset_m` of 0.005–0.02 m, which the model
(correctly, given the number) penalizes as an uncompensated defocus that blows up at high
frequency. Two defensible resolutions were identified — **the maintainer chose option 2
(auto-refocus) on 2026-07-10**, on correctness/long-term grounds: option 1's convention leaves
a standing trap where entering a datasheet phase-center value silently costs multi-dB at Ka,
while auto-refocus makes the field mean what a feed engineer expects and is correct per-band
by mechanism. Execution: roadmap unit P7. Option 1 is kept below for the record.

1. **Config realism (not chosen).** Treat `phase_center_offset_m` as the *residual,
   uncompensated* offset after focusing and set it to ≈ 0 for these design specs (the antennas
   are assumed focused). Mirrors the q_factor fix: the physics was fine; the input was
   unphysical. Lowest blast radius; makes uncalibrated absolute gain trustworthy.
2. **Model auto-refocus (CHOSEN 2026-07-10).** Interpret `phase_center_offset_m` as a *feed property* and have the
   evaluator axially reposition the feed so the phase center lands at the focal point (net axial
   offset 0) unless a deliberate defocus is requested. More faithful to feed physics, but a
   behavior change touching `integration.rs` / evaluator, with test implications.

Either way, document the chosen semantics of `phase_center_offset_m` in `domain-contract.md`
(it is currently ambiguous: "offset from feed aperture" vs "defocus from the focal point").

## Reproduction

```
cargo test -p antenna-model --test reference_validation -- --nocapture --test-threads=1
```

The `dsn_34m_reference_residuals_within_tolerance` table shows X −0.73 dB / Ka −3.52 dB. The
decomposition above was produced by a temporary probe (removed) that: built the feed at focus,
called `analyze_edge_cases` for the spillover fraction, `ruze_efficiency` for the surface term,
and `compute_gain_db` with `apply_spillover=false` to isolate illumination efficiency, sweeping
`phase_center_offset` ∈ {design, 0}. Re-add a similar probe to re-verify.

## Suggested follow-up steps

1. ~~Decide semantics (option 1 vs 2 above); add a decision-register row.~~ **Done 2026-07-10:
   option 2 (auto-refocus); register row P7 Decided in `docs/roadmap-2026-07.md` §5.**
2. Apply the chosen fix; re-run the harness — expect Ka residual → ~0.1 dB and X → ~0 dB.
3. Tighten the Ka reference tolerance in
   `tests/fixtures/reference_datasets/dsn_34m_bwg.psv` (currently a loose 5.0 dB) to ~1.5 dB.
4. Fix the `(q as i32)` truncation in `estimate_spillover` while in the file.
5. Add a multi-band reference antenna (e.g. DSN 34-m HEF or 70-m) to confirm the fix generalizes
   across D/λ.

## Related

- Feed-taper fix (same harness, same session): q_factor corrected to a −11 dB edge-taper target.
- `docs/domain-contract.md` — needs a `phase_center_offset_m` semantics entry.
- Memory: `dsn-reference-validation`.
