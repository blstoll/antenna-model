#!/usr/bin/env bash
# Local CI gate — mirrors .github/workflows/ci.yml. Run before pushing.
# Exits nonzero on the first failing check.
set -euo pipefail

# Match CI: give libtest worker threads a larger stack. The calibrate 3D→4D
# round-trip evaluation overflows the ~2 MiB default on Linux debug builds.
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

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

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

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
