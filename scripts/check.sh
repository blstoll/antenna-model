#!/usr/bin/env bash
# Local CI gate — mirrors .github/workflows/ci.yml. Run before pushing.
# Exits nonzero on the first failing check.
set -euo pipefail

# macOS OpenBLAS linking for the calibrate crate (ndarray-linalg openblas-system).
# No-op on Linux, where libopenblas-dev provides the system paths.
if [[ "$(uname)" == "Darwin" ]]; then
  export LDFLAGS="${LDFLAGS:-} -L/opt/homebrew/opt/openblas/lib"
  export CPPFLAGS="${CPPFLAGS:-} -I/opt/homebrew/opt/openblas/include"
fi

# Match CI: give libtest worker threads a larger stack. The calibrate 3D→4D
# round-trip evaluation overflows the ~2 MiB default on Linux debug builds.
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> cargo audit (non-blocking)"
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit || echo "WARNING: cargo audit reported issues (non-blocking)"
else
  echo "SKIP: cargo-audit not installed (run: cargo install cargo-audit)"
fi

echo "All gate checks passed."
