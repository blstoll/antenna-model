#!/usr/bin/env bash
# Local CI gate — mirrors .github/workflows/ci.yml. Run before pushing.
# Exits nonzero on the first failing check.
set -euo pipefail

# No RUST_MIN_STACK here, and none in ci.yml either — roadmap D3 retired the
# 16 MiB workaround on 2026-08-20. See the comment in .github/workflows/ci.yml for
# why it was never about the B-spline evaluation, and
# `the_round_trip_fits_in_a_small_thread_stack`
# (calibrate/tests/artifact_export_integration_test.rs) for the guard that replaced
# it: it sets its own 512 KiB stack, so it holds regardless of what the harness
# hands the other tests.

if ! cargo nextest --version >/dev/null 2>&1; then
  echo "ERROR: cargo-nextest is not installed. Install it with:" >&2
  echo "  cargo install cargo-nextest --locked" >&2
  exit 1
fi

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy -p antenna-core --all-targets -- -D warnings (openapi OFF)"
# The workspace build always unifies antenna-core's `openapi` feature ON, because
# antenna-model enables it. This package-scoped check is the only thing that
# compiles the feature-OFF configuration — the one `cargo build -p calibrate`
# produces (roadmap D4). Without it, a bare `#[schema(...)]` attribute on a
# feature-gated type passes every workspace check and breaks only the CLI build.
cargo clippy -p antenna-core --all-targets -- -D warnings

echo "==> scripts/assert-dep-graphs.sh (CLI graph + antenna-core weight)"
# The build + dep-graph assertions live in one script that CI runs too, so the
# banned-crate list has a single home (roadmap D27). It was inlined here and
# copy-pasted into ci.yml, and in both copies the assertion **failed open**.
./scripts/assert-dep-graphs.sh

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo clippy -p calibrate --features s3-input -- -D warnings (S3 path ON)"
# The `s3-input` feature is OFF by default (roadmap D6), so every check above compiles
# only the stub side of `parser::fetch_from_s3`. This is the one check that compiles the
# real S3 client — without it, the feature could stop building and nothing would notice
# until someone needed it. Kept package-scoped and without --all-targets deliberately:
# --all-targets would pull the dev-dependency antenna-model back in and re-unify
# features, which is the same trap D4 documents for the dep-graph assertions.
cargo clippy -p calibrate --features s3-input -- -D warnings

echo "==> scripts/assert-numerical-certification.sh"
# Asserts that the numerical certification is still selected by the profiles that run it,
# and that antenna-core is still compiled optimized (GitHub issue #74). Neither failure
# shows up as a failing test — see the script's header for what each invariant is, and
# the profile comment in Cargo.toml for the measurements. The certification itself runs in
# the test step below: one execution of each of those tests, and it is optimized.
./scripts/assert-numerical-certification.sh

echo "==> cargo nextest run --workspace --profile full"
# full profile = both test tiers (see .config/nextest.toml, roadmap unit D18).
# The bare default profile is the dev inner loop and excludes the slow tier;
# the gate must run everything.
cargo nextest run --workspace --profile full

# nextest doesn't run doctests (https://github.com/nextest-rs/nextest/issues/16);
# cargo test --workspace used to cover these, so run them separately.
echo "==> cargo test --doc --workspace"
cargo test --doc --workspace

echo "==> cargo audit (non-blocking)"
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit || echo "WARNING: cargo audit reported issues (non-blocking)"
else
  echo "SKIP: cargo-audit not installed (run: cargo install cargo-audit)"
fi

echo "All gate checks passed."
