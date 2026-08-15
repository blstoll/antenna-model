#!/usr/bin/env bash
# The CLI must not compile the web stack (roadmap D4), asserted in ONE place.
#
# Called by both `scripts/check.sh` and `.github/workflows/ci.yml`. It used to be
# copy-pasted into both, so the banned-crate list lived in four places with nothing
# detecting drift between them.
#
# Two properties, neither of which any workspace-scoped check can see:
#
#   1. `cargo build -p calibrate` uses ONLY calibrate's normal deps, so it is the one
#      build that fails if calibrate relies on a feature it does not declare.
#      `clippy -p calibrate --all-targets` does NOT substitute: --all-targets pulls the
#      dev-dependency antenna-model back in and re-unifies the features.
#   2. The dep-graph assertion below is what fails if antenna-model (or anything dragging
#      the web stack) returns to calibrate's normal graph.
#
# The surviving `lru` is calibrate's own, via aws-sdk-s3 — same carve-out as tokio.
set -euo pipefail

# Crates that must never be reachable from calibrate's normal graph. This is a
# *symptom* list, and it is the weaker of the two assertions below precisely because
# it enumerates today's web stack rather than the invariant.
BANNED_RE='poem|h3o|utoipa|dashmap'

# `cargo tree` output is captured, never piped straight into an `if`.
#
# The bug this shape exists to prevent: `if cargo tree … | grep -E …; then fail; fi`
# **fails open**. Under `set -euo pipefail` a nonzero `cargo tree` does not abort — the
# failing pipeline simply selects the `if`'s false branch — so a renamed package, a
# manifest error, or a cargo behaviour change made the gate print success without
# having checked anything. Verified: `if false | grep -E poem; then echo FIRED; fi`
# prints nothing and returns 0.
normal_deps_of() {
  local pkg="$1" out rc=0
  out=$(cargo tree -p "$pkg" -e normal 2>&1) || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "ERROR: 'cargo tree -p $pkg -e normal' failed (exit $rc); the web-stack guard" >&2
    echo "       did NOT run. Fix the command before trusting this gate." >&2
    printf '%s\n' "$out" >&2
    return 1
  fi
  printf '%s\n' "$out"
}

echo "--> cargo build -p calibrate (CLI graph: normal deps only)"
cargo build -p calibrate

# ---------------------------------------------------------------------------
# Negative control, run BEFORE the assertion it protects.
#
# Roadmap P13's rule: a guard whose margin nothing asserts rots silently — P13 deleted
# a fitted constant that had drifted 43.5× past its own safety factor with nothing in
# the build able to notice. The grep-based version of this check had the same shape:
# had `cargo tree`'s output format changed, or the crate names moved, it would have
# matched nothing and reported a clean graph forever.
#
# So the detector is first pointed at a graph that MUST trip it. antenna-model is the
# service crate; poem/utoipa/dashmap are its direct dependencies and h3o backs
# /h3-heatmap. If the detector comes up clean here, the detector is broken, not the
# graph.
# ---------------------------------------------------------------------------
echo "--> negative control: the detector must fire on antenna-model's own graph"
control_tree=$(normal_deps_of antenna-model)
if ! printf '%s\n' "$control_tree" | grep -Eq "$BANNED_RE"; then
  echo "ERROR: the web-stack detector matched NOTHING in antenna-model's normal graph." >&2
  echo "       antenna-model depends on poem, utoipa, dashmap and h3o, so this cannot" >&2
  echo "       be true: the detector itself is broken (cargo tree output format, or a" >&2
  echo "       crate rename). Every 'calibrate is clean' verdict it gives is worthless" >&2
  echo "       until this passes. Roadmap D4/D27." >&2
  exit 1
fi

echo "--> asserting calibrate's normal graph"
calibrate_tree=$(normal_deps_of calibrate)

# The INVARIANT, checked first: antenna-model is not a normal dependency of calibrate.
# This is the property D4 actually established — the web stack is reachable only
# *through* antenna-model, and antenna-model is deliberately kept a dev-dependency so
# the CLI e2e tests can serve artifacts through the real service path (roadmap C13).
# Stating it this way survives the web stack being re-spelled: swap poem for axum and
# the symptom list below goes quiet while this does not.
if printf '%s\n' "$calibrate_tree" | grep -Eq '(^|[^A-Za-z0-9_-])antenna-model v'; then
  echo "ERROR: antenna-model is a NORMAL dependency of calibrate (roadmap D4)." >&2
  echo "       It must stay a dev-dependency: the CLI e2e tests serve artifacts through" >&2
  echo "       the real service path, but the shipped CLI must compile no web stack." >&2
  printf '%s\n' "$calibrate_tree" | grep -E '(^|[^A-Za-z0-9_-])antenna-model v' >&2
  exit 1
fi

# The symptom list, as defence in depth: it catches a web-stack crate arriving by some
# route other than antenna-model, which the invariant above would not see.
if printf '%s\n' "$calibrate_tree" | grep -Eq "$BANNED_RE"; then
  echo "ERROR: web-stack dependency reachable from calibrate's normal graph (roadmap D4)" >&2
  printf '%s\n' "$calibrate_tree" | grep -E "$BANNED_RE" >&2
  exit 1
fi

echo "    ok: calibrate's normal graph carries no web stack"
