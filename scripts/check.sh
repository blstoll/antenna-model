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

echo "==> cargo build -p calibrate (CLI graph: normal deps only)"
# The CLI must not compile the web stack (roadmap D4). Two properties, neither of
# which any workspace-scoped check can see:
#   1. `cargo build -p calibrate` uses ONLY calibrate's normal deps, so it is the
#      one build that fails if calibrate relies on a feature it does not declare.
#      `clippy -p calibrate --all-targets` does NOT substitute: --all-targets pulls
#      the dev-dependency antenna-model back in and re-unifies the features.
#   2. The dep-tree assertion below is what fails if antenna-model (or anything
#      dragging poem/h3o/utoipa/dashmap) returns to calibrate's normal graph.
# The surviving `lru` is calibrate's own, via aws-sdk-s3 — same carve-out as tokio.
cargo build -p calibrate
if cargo tree -p calibrate -e normal | grep -E 'poem|h3o|utoipa|dashmap'; then
  echo "ERROR: web-stack dependency reachable from calibrate's normal graph (roadmap D4)" >&2
  exit 1
fi

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
