#!/usr/bin/env bash
# Generate the NASA CR-159703 1.22 m calibration artifact end to end.
#
# Roadmap unit D14 built this; roadmap unit D9 is why it is a script.
#
# ============================================================================
# What this is (D9's worked example)
# ============================================================================
#
# D9 asked whether `.bin` calibration artifacts should be committed, and the answer was
# "no binaries; document + script the generation path". This is that path, made concrete:
# every input is in the repository, the output is not, and anyone can re-derive the
# artifact byte-for-byte from a clean checkout. Point `--output` wherever your service
# reads calibration data from and add the antenna to `calibration_data/antennas.yaml`.
#
# The artifact is deliberately written OUTSIDE the repository tree (default: a directory
# under the system temp dir). Do not commit it, and do not commit the intermediate CSV:
# the CSV is not measured data (see below) and a checked-in copy invites exactly the
# misreading this unit spent its provenance blocks trying to prevent.
#
# ============================================================================
# What the data is, and what it is NOT
# ============================================================================
#
# The measurement grid is SYNTHESIZED: it is this repository's own physical-optics model
# plus a residual derived from digitized measurements published in NASA CR-159703
# (Collin & Gabel, 1979). Only the residual comes from measurements, and only at the 19
# digitized peak angles. **Never present this dataset, or a gain computed from it, as
# measured data.** `cr159703_grid` prints the full list of fabrications on every run and
# writes it into the summary JSON; read it.
#
# Inputs, all committed:
#   antenna-model/tests/fixtures/reference_datasets/sidelobe_data/
#       nasa_cr159703_pattern_peaks.psv    the digitized measurements + their provenance
#   calibrate/tests/fixtures/
#       nasa_cr159703_122m_classes.yaml    the antenna class + what in it is assumed
#
# Outputs (in $OUT_DIR):
#   cr159703_grid.csv           the synthesized measurement grid
#   cr159703_grid_summary.json  fabrications, anchor table, injected residual RMS
#   cr159703_122m.bin           the calibration artifact the service loads
#   cr159703_report.json        validation report (RMSE, cross-validation, outliers)
#   cr159703_metadata.json      artifact metadata sidecar
#
# Usage:
#   scripts/generate-cr159703-artifact.sh [OUT_DIR]
#
# The end-to-end behaviour of this pipeline is pinned by
# calibrate/tests/cli_full_mode_real_data_e2e.rs, which runs the same two binaries and then
# serves the artifact through the service. Its main pipeline deliberately runs WITHOUT
# --validate (cross-validation costs five extra refits and none of its assertions read the
# result); the flags below that it does not otherwise use — --validate and --metadata — are
# covered by `the_scripts_validated_run_produces_an_artifact` in that file, so no argument
# here is unexercised.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="${1:-${TMPDIR:-/tmp}/cr159703-calibration}"

ANTENNA_ID="nasa_cr159703_122m"
FEED_ID="kumar_12ghz"
ANTENNA_CLASS="NASA_CR159703_1p22m"
CLASSES_FILE="$REPO_ROOT/calibrate/tests/fixtures/nasa_cr159703_122m_classes.yaml"
PEAKS_FILE="$REPO_ROOT/antenna-model/tests/fixtures/reference_datasets/sidelobe_data/nasa_cr159703_pattern_peaks.psv"

mkdir -p "$OUT_DIR"

echo "==> cargo build --release --bin cr159703_grid --bin calibrate"
cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" \
  --bin cr159703_grid --bin calibrate

BIN_DIR="$REPO_ROOT/target/release"

echo
echo "==> step 1/2: synthesize the real-anchored measurement grid"
"$BIN_DIR/cr159703_grid" \
  --peaks "$PEAKS_FILE" \
  --classes-file "$CLASSES_FILE" \
  --antenna-class "$ANTENNA_CLASS" \
  --output "$OUT_DIR/cr159703_grid.csv" \
  --summary "$OUT_DIR/cr159703_grid_summary.json"

echo
echo "==> step 2/2: fit the correction surface and write the artifact"
# --validate runs the 5-fold cross-validation. The grid is sized for it: 3240 points
# against the 960 coefficients the shipped 4/6/8 knot counts declare, so each training
# split still sees 2592 (roadmap D20 — an underdetermined fit is now a hard error, and a
# fold is what has to clear the count, not just the whole grid).
#
# READ THE CV NUMBERS WITH CARE. The validator assigns folds as CONTIGUOUS slices of the
# input file, and this file is written frequency-major, so the first and last folds hold
# out an entire frequency slab and the fit has to extrapolate the frequency axis to reach
# it. Measured here: fold RMSEs 10.07 / 0.56 / 0.12 / 0.64 / 10.86 dB — the interior folds
# report the surface's actual interpolation error and the two edge folds report a
# frequency extrapolation nobody asked for. The mean (4.45 dB) is therefore not this
# artifact's accuracy; `corrected_rmse` (0.027 dB) and the served comparisons in
# calibrate/tests/cli_full_mode_real_data_e2e.rs are. Filed as roadmap unit **D22**, with
# the measurement, by D14.
"$BIN_DIR/calibrate" \
  --calibration-mode full \
  --input "$OUT_DIR/cr159703_grid.csv" \
  --output "$OUT_DIR/cr159703_122m.bin" \
  --antenna-id "$ANTENNA_ID" \
  --feed-id "$FEED_ID" \
  --antenna-class "$ANTENNA_CLASS" \
  --classes-file "$CLASSES_FILE" \
  --antenna-name "NASA CR-159703 1.22 m (real-anchored hybrid fill)" \
  --validate \
  --report "$OUT_DIR/cr159703_report.json" \
  --metadata "$OUT_DIR/cr159703_metadata.json"

echo
echo "Artifact written to $OUT_DIR/cr159703_122m.bin"
echo
echo "REMINDER: the measurement grid is model-filled, not measured (see"
echo "  $OUT_DIR/cr159703_grid_summary.json, field \"fabrications\")."
echo "Do not commit the artifact or the grid — roadmap D9."
