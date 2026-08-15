#!/usr/bin/env bash
# Two dependency-graph invariants, asserted in ONE place:
#
#   * the CLI must not compile the web stack (roadmap D4);
#   * antenna-core must stay a physics/artifact crate (roadmap D27 finding 4).
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
#
# `--prefix none` is deliberate: it emits bare "name version" lines instead of a drawn
# tree, so every pattern below can anchor at `^` and match package identity rather than
# whatever box-drawing characters precede it. An anchored pattern against a drawn tree
# matches nothing — which is a silent pass, the exact failure mode this file exists to
# prevent. Repeated subtrees still repeat, so counting requires `sort -u`.
normal_deps_of() {
  local pkg="$1" out rc=0
  out=$(cargo tree -p "$pkg" -e normal --prefix none 2>&1) || rc=$?
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

# ---------------------------------------------------------------------------
# antenna-core weight (roadmap D27 finding 4).
#
# Core is "the physics engine + artifact data layer, no web stack". The web-stack
# assertions above cannot see a violation of that, by construction: they ask about
# poem, not about weight. D4 moved error.rs wholesale, which brought
# `impl From<serde_yaml::Error>` / `impl From<config::ConfigError>` along, and the
# orphan rule pinned both to core — so core silently acquired the entire `config`
# crate (json5, ron, rust-ini, toml, yaml-rust2, async-trait, serde-untagged, …)
# plus an end-of-life serde_yaml, for two conversions nothing in model/ or data/
# ever performs. 88 of core's then-114 normal packages were that stack.
#
# Both assertions below, same shape as above: a name-based list, and a count
# ceiling that catches a heavy stack arriving under names this list does not know.
# ---------------------------------------------------------------------------
CONFIG_STACK_RE='^(config|serde_yaml|yaml-rust2|unsafe-libyaml|json5|rust-ini|ron) v'

# Max unique packages in antenna-core's normal graph. Measured 21 on 2026-08-14, down
# from 88 before the config stack came out. The headroom is for ordinary growth, not for
# a new stack. If a legitimate dependency pushes past this, raise it deliberately and
# say why — this number is meant to be argued with, not silently bumped.
CORE_MAX_DEPS=28

echo "--> negative control: the config-stack detector must fire on antenna-model's graph"
# antenna-model declares both `config` and `serde_yaml` itself — it is the crate that
# reads config files, which is the whole point of them not being in core. If the
# detector finds neither here, the detector is broken and its verdict on core is worthless.
if ! printf '%s\n' "$control_tree" | grep -Eq "$CONFIG_STACK_RE"; then
  echo "ERROR: the config-stack detector matched NOTHING in antenna-model's normal graph." >&2
  echo "       antenna-model depends on config and serde_yaml directly, so this cannot be" >&2
  echo "       true: the detector itself is broken (cargo tree output format, or a crate" >&2
  echo "       rename). Roadmap D27 finding 4." >&2
  exit 1
fi

echo "--> asserting antenna-core's normal graph"
core_tree=$(normal_deps_of antenna-core)

if printf '%s\n' "$core_tree" | grep -Eq "$CONFIG_STACK_RE"; then
  echo "ERROR: config-file stack reachable from antenna-core's normal graph (roadmap D27)." >&2
  echo "       antenna-core is the physics engine and artifact layer; it does not read" >&2
  echo "       config files. If you need a conversion from a config-crate error, do it in" >&2
  echo "       the crate that owns the config stack (see antenna-model's config/settings.rs)" >&2
  echo "       rather than adding a From impl here — the orphan rule will drag the whole" >&2
  echo "       dependency in with it." >&2
  printf '%s\n' "$core_tree" | grep -E "$CONFIG_STACK_RE" >&2
  exit 1
fi

# Unique packages, not lines: `--prefix none` still repeats a shared subtree once per
# path that reaches it, and a "(*)" suffix marks the elided repeats.
core_dep_count=$(printf '%s\n' "$core_tree" | sed -e 's/ (\*)$//' -e '/^$/d' | sort -u | wc -l | tr -d ' ')
if [ "$core_dep_count" -gt "$CORE_MAX_DEPS" ]; then
  echo "ERROR: antenna-core's normal graph has $core_dep_count packages, over the" >&2
  echo "       ceiling of $CORE_MAX_DEPS (roadmap D27 finding 4). This ceiling exists" >&2
  echo "       because the named list above only knows today's config stack: a heavy" >&2
  echo "       dependency arriving under any other name shows up here instead." >&2
  echo "       Raise it deliberately, with a reason, or move the dependency out." >&2
  exit 1
fi

echo "    ok: antenna-core carries no config stack ($core_dep_count/$CORE_MAX_DEPS packages)"
