#!/usr/bin/env bash
# Four dependency-graph invariants, asserted in ONE place:
#
#   * the CLI must not compile the web stack (roadmap D4);
#   * antenna-core must stay a physics/artifact crate (roadmap D27 finding 4);
#   * the test HTTP client must not carry `system-proxy` (roadmap D18);
#   * the CLI must not compile the AWS SDK by default (roadmap D6).
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
# Historical note: D4 closed with one survivor from the banned list, `lru`, arriving
# through calibrate's own aws-sdk-s3 rather than through the web stack — hence the
# carve-out that used to sit here. **D6 (2026-08-16) removed it**: aws-sdk-s3 is now
# behind the off-by-default `s3-input` feature, so `lru` is not in calibrate's default
# normal graph at all (verified: 0 by default, 1 with the feature on). No carve-out.
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

# ---------------------------------------------------------------------------
# The test HTTP client must not carry `system-proxy` (roadmap D18).
#
# This is a *latency* invariant, and it is the only one here that no compile can
# observe: enabling the feature keeps the build green, clippy green and CI green,
# and silently returns the suite to ~14 minutes on macOS.
#
# Why it matters that much: reqwest's `system-proxy` makes constructing ANY client
# query the macOS system proxy configuration through SCDynamicStore -> configd.
# Measured 2026-08-15, `reqwest::Client::builder().build()` cost **11.79 s** — against
# 2.7 ms to load the whole calibration repository — and every test that starts a
# `TestServer` pays it once. Dropping it took `antenna-model --profile full` from
# 848 s to 33 s. See docs/findings-2026-08-15-test-suite-execution-time.md.
#
# `default-features = false` in antenna-model/Cargo.toml is therefore the single most
# load-bearing line in the test suite, and prose in a manifest comment is not a gate:
# someone who needs gzip writes features = ["json", "gzip"], drops the key, and nothing
# notices. Per P13, a property worth having is a property something asserts.
#
# The feature edge is the assertion, NOT the `system-configuration` package, because
# that package is macOS-only (`[target.'cfg(target_os = "macos")'.dependencies]`) and so
# a package-name check is **vacuous on the Linux CI runner** — it would pass there
# whatever the feature set says. The feature edge is platform-independent.
#
# Negative control: this detector is known to fire, empirically rather than by
# construction. Before the fix, `cargo tree -p antenna-model -e features` listed
# `reqwest feature "system-proxy"` (alongside "default", "default-tls", "rustls",
# "http2", "charset"); after, it lists exactly `reqwest feature "json"`. The
# positive control below re-checks the live half of that every run: if reqwest leaves
# the graph, is renamed, or cargo changes this output format, the "json" edge stops
# matching and the gate fails loudly instead of reporting a clean graph forever.
# ---------------------------------------------------------------------------
echo "--> asserting the test HTTP client resolves without system-proxy (roadmap D18)"

reqwest_features=$(cargo tree -p antenna-model -e features --prefix none 2>&1) || {
  echo "ERROR: 'cargo tree -p antenna-model -e features' failed; the system-proxy guard" >&2
  echo "       did NOT run. Fix the command before trusting this gate." >&2
  printf '%s\n' "$reqwest_features" >&2
  exit 1
}
reqwest_features=$(printf '%s\n' "$reqwest_features" | grep -E '^reqwest feature "' | sort -u || true)

# Positive control, first: reqwest must be in the graph under that name, with the one
# feature we do want. If this matches nothing, the absence check below proves nothing.
if ! printf '%s\n' "$reqwest_features" | grep -Fqx 'reqwest feature "json"'; then
  echo "ERROR: could not find 'reqwest feature \"json\"' in antenna-model's feature graph." >&2
  echo "       reqwest is a dev-dependency of antenna-model with features = [\"json\"], so" >&2
  echo "       this cannot be true unless the detector is broken (crate renamed, dependency" >&2
  echo "       removed, or cargo tree output format changed). The system-proxy check below" >&2
  echo "       is worthless until this passes. Roadmap D18." >&2
  echo "       Found instead:" >&2
  printf '%s\n' "${reqwest_features:-<nothing>}" >&2
  exit 1
fi

if printf '%s\n' "$reqwest_features" | grep -Fq 'reqwest feature "system-proxy"'; then
  echo "ERROR: reqwest resolves with the 'system-proxy' feature (roadmap D18)." >&2
  echo "       On macOS this makes every reqwest::Client construction query configd," >&2
  echo "       costing ~11.8 s PER TEST that starts a TestServer — it took the" >&2
  echo "       antenna-model suite from 33 s to 848 s. The build stays green, so this" >&2
  echo "       gate is the only thing that can tell you." >&2
  echo "       Fix: keep 'default-features = false' on reqwest in antenna-model/Cargo.toml" >&2
  echo "       and add any feature you need BY NAME. Do not restore default-features." >&2
  printf '%s\n' "$reqwest_features" >&2
  exit 1
fi

echo "    ok: reqwest resolves without system-proxy"

# ---------------------------------------------------------------------------
# The CLI must not compile the AWS SDK by default (roadmap D6).
#
# `s3://` measurement input is the AWS SDK's only user in this workspace, and it is a
# source almost no run uses: every test, script and documented workflow passes a local
# path. Gating it behind the off-by-default `s3-input` feature took calibrate's normal
# graph from **240 unique packages to 80** — 160 fewer, 25 of them `aws-*`.
#
# Those figures are the ones THIS script's own pipeline produces (`--prefix none`, strip
# the ` (*)` elision marker, `sort -u`). Counting raw lines instead gives 119 for `aws-*`,
# because `cargo tree` repeats a shared subtree once per path that reaches it — an earlier
# version of this comment quoted that number, and a gate that exists so figures cannot rot
# should not itself carry a figure its own command contradicts.
#
# This does NOT reduce what `cargo audit` reports: audit reads `Cargo.lock`, and an
# optional dependency stays in the lockfile. The win is what gets compiled.
#
# Why this needs a gate and not just an `optional = true`: nothing about `optional` stops
# a later edit from adding `aws-*` to the default feature set, or from making some other
# dependency pull it in transitively. The build stays green either way — this is the same
# shape as the system-proxy invariant above, a property no compile can observe.
#
# The negative control is a POSITIVE build of the feature, not a second crate: with
# `--features s3-input` the detector MUST fire. If it does not, the pattern has stopped
# matching (crate rename, cargo output change) and the default-graph verdict below is
# worthless.
# ---------------------------------------------------------------------------
echo "--> asserting the CLI carries no AWS SDK by default (roadmap D6)"

AWS_RE='^aws-'

s3_tree=$(cargo tree -p calibrate -e normal --features s3-input --prefix none 2>&1) || {
  echo "ERROR: 'cargo tree -p calibrate --features s3-input' failed; the AWS gate did NOT" >&2
  echo "       run. Fix the command before trusting this gate." >&2
  printf '%s\n' "$s3_tree" >&2
  exit 1
}

echo "--> negative control: the AWS detector must fire with --features s3-input"
if ! printf '%s\n' "$s3_tree" | grep -Eq "$AWS_RE"; then
  echo "ERROR: the AWS detector matched NOTHING in calibrate's graph WITH s3-input on." >&2
  echo "       That feature enables aws-config and aws-sdk-s3, so this cannot be true:" >&2
  echo "       the detector is broken (crate rename, or cargo tree output format), and" >&2
  echo "       its 'clean by default' verdict below would be worthless. Roadmap D6." >&2
  exit 1
fi

if printf '%s\n' "$calibrate_tree" | grep -Eq "$AWS_RE"; then
  echo "ERROR: the AWS SDK is in calibrate's DEFAULT normal graph (roadmap D6)." >&2
  echo "       S3 input must stay behind the off-by-default 's3-input' feature: it is" >&2
  echo "       160 extra packages to compile for a source almost no run uses. Mark the" >&2
  echo "       dependency 'optional = true' and reach it only through the feature." >&2
  printf '%s\n' "$calibrate_tree" | grep -E "$AWS_RE" >&2
  exit 1
fi

default_dep_count=$(printf '%s\n' "$calibrate_tree" | sed -e 's/ (\*)$//' -e '/^$/d' | sort -u | wc -l | tr -d ' ')
s3_dep_count=$(printf '%s\n' "$s3_tree" | sed -e 's/ (\*)$//' -e '/^$/d' | sort -u | wc -l | tr -d ' ')
echo "    ok: no AWS SDK in the default CLI graph ($default_dep_count packages; s3-input: $s3_dep_count)"
