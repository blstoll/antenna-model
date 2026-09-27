# 1. Calibration artifacts are validated at construction

- **Status:** Accepted, 2026-09-27
- **Governs:** #105 (follow-up to #97; sequence #104 → #105 → #106)

## Context

A calibration artifact (`antenna_core::types::AntennaCalibration`) carries invariants that
span several fields: non-empty ids, physical parameters in domain, a correction surface whose
knot layout satisfies the core layout rules, calibration coverage contained by that surface's
fitted support, and a single coverage claim when a `PartiallyCalibrated` status duplicates it.

Before this decision the invariants lived on a public `AntennaCalibration::validate()` (plus a
`validate()` on each part), and every producer had to remember to call it. The builder did
not validate, the service's design-spec antennas were built as a struct literal and never
validated, and consumers re-derived validity where they needed it.

An artifact is created in exactly four places:

1. **Decoding bytes from disk** — the ANTC loader.
2. **The full-mode `calibrate` export** — `export_full_calibration`.
3. **The boresight `calibrate` export** — `build_calibration_artifact`.
4. **The service's design-spec (uncalibrated) antennas** built from `antennas.yaml`.

No production code mutates an artifact after construction.

## Decision

All four construction points run one validation, owned privately by `antenna_core::artifact`:

- `AntennaCalibrationBuilder::build()` validates and returns
  `Result<AntennaCalibration, ValidationError>`; an unset required field is
  `ValidationError::MissingField`. Points 2–4 go through it.
- The loader (point 1) runs the same validation on the decoded value.
- Consumers — the service repository included — take any `AntennaCalibration` as valid and do
  not re-check.
- The data types in `antenna_core::types` carry no validation and no dependency on
  `antenna_core::model`; the checks that need the model (knot layout, f/D range,
  coverage ⊆ support) live in `artifact`, which may depend on both.

## Alternatives rejected

- **A "validated artifact" wrapper type** (`ValidatedCalibration(AntennaCalibration)`). It
  would make validity a type-level fact, but with four construction sites and no
  post-construction mutation in production, validating at construction already gives that
  guarantee in practice. The wrapper would take the good name off the primary flow: every
  consumer would handle `ValidatedCalibration` while `AntennaCalibration` became the
  exceptional, unchecked form.
- **Private fields.** They would forbid the direct construction and mutation that tests rely
  on — including tests that deliberately build an invalid artifact to prove a rejection — and
  would force accessor boilerplate onto a plain data record that is also a wire format.
- **A public standalone validate function.** It is what the codebase had. It makes validity a
  convention every caller must remember, and a forgotten call (the design-spec antennas never
  made one) is silent.

## Consequences

- An invalid artifact cannot reach the service through any production path; the repository's
  prepared-correction error arm is reachable only by a hand-mutated value.
- Tests may still build or mutate artifacts directly. Tests that need a valid artifact start
  from one builder per crate: `antenna_core::types::fixtures` in core and
  `service::test_support::calibration_builder` in `antenna-model`'s unit tests. An
  integration-test crate cannot reach either without exposing fixtures in the public API, so
  `antenna-model/tests/feed_steering_test.rs` keeps its own builder-based helper.
- Rejection tests assert on the builder's or the loader's error, since there is no validate
  function to call.
- A future construction point (e.g. an S3 or HTTP loader, #106) must go through the builder or
  the artifact module's decoding — which #106 makes the single public entry point for bytes.
