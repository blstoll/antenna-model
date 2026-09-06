#!/usr/bin/env bash
# Two invariants behind the numerical certification (GitHub issue #74):
#
#   1. the numerical regressions named in
#      .config/numerical-certification-manifest.txt are SELECTED — by `--profile full`,
#      which is what scripts/check.sh and CI run, and by the bare `default` profile, which
#      is the dev inner loop;
#   2. `antenna-core` is still compiled OPTIMIZED in the debug and test builds, which is
#      the only reason they are affordable in either tier.
#
# Called by both scripts/check.sh and .github/workflows/ci.yml. It asserts; it runs no
# tests. The certification itself runs in the ordinary suite — that is the point of issue
# #74's design: there is one execution of each test, and it is an optimized one, so a
# failing numerical regression fails the normal test step.
#
# Why either half needs asserting at all — neither failure is visible in a test result:
#
#   * a rename, a move between binaries, or an edited default-filter drops a test from a
#     profile silently. The suite passes with less in it, which is the failure mode roadmap
#     P13 records: a guard whose power nothing asserts rots.
#   * deleting the profile override in Cargo.toml costs nothing in correctness and ~92 s of
#     wall (~690 s of CPU) per full run. Every test still passes. Nothing else would say so.
#
# There is deliberately no wall-clock threshold here. Timing assertions in this repo have a
# history: the sustained-load test flaked twice and was deleted with its thread reservation
# (issue #73), and P13 retired a fitted constant nothing asserted the margin of. The
# structural facts — this test is in this profile, this crate is built at this opt-level —
# are the durable statements of the same property.
set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MANIFEST=.config/numerical-certification-manifest.txt

# `set -euo pipefail` does NOT abort on a command that fails to the left of a pipe inside an
# `if`, so cargo output is captured and its exit status checked explicitly. The dep-graph
# script carries the same shape for the same reason: the natural spelling fails OPEN.
#
# stderr goes to a file rather than into the capture: cargo writes "Compiling ..." there, and
# folding it into stdout puts build chatter into the listing, where it reads as a test name.
CARGO_STDERR=$(mktemp)
trap 'rm -f "$CARGO_STDERR"' EXIT

nextest_list() {
  local out rc=0
  out=$(cargo nextest list --color never "$@" 2>"$CARGO_STDERR") || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "ERROR: 'cargo nextest list $*' failed (exit $rc); the numerical-certification" >&2
    echo "       audit did NOT run. Fix the command before trusting this gate." >&2
    cat "$CARGO_STDERR" >&2
    printf '%s\n' "$out" >&2
    return 1
  fi
  printf '%s\n' "$out"
}

# `grep -Fqx "$line" <<< "$listing"`, never `printf … | grep -Fqx`: grep -q exits at the
# first match, the writer then dies of SIGPIPE, and under `pipefail` the pipeline reports 141
# — so the membership test says "absent" exactly when the line is present.
selected_in() {
  local listing="$1" line="$2"
  grep -Fqx "$line" <<< "$listing"
}

# `expected` is every test in the manifest; `both_tiers` is the part above the [full-only]
# marker. The marker line itself is never a test name.
# `|| true` on both greps: a comments-only manifest makes the second one exit 1, and under
# `pipefail` + `set -e` that aborts the assignment — so the "names no tests" diagnostic below
# would never print and the gate would die with no explanation. Verified before fixing.
manifest_body=$(grep -v '^[[:space:]]*#' "$MANIFEST" | grep -v '^[[:space:]]*$' || true)
expected=$(grep -vFx '[full-only]' <<< "$manifest_body" | sort || true)
both_tiers=$(sed '/^\[full-only\]$/,$d' <<< "$manifest_body" | sort)
if [ -z "$expected" ]; then
  echo "ERROR: $MANIFEST names no tests. An empty manifest makes every check below" >&2
  echo "       vacuously true: the gate would certify nothing and report success." >&2
  exit 1
fi
expected_count=$(printf '%s\n' "$expected" | wc -l | tr -d ' ')

# ---------------------------------------------------------------------------
# Invariant 1: the manifest tests are selected by both profiles.
#
# The live control comes first. A membership test that can only ever say "found" would pass
# this file whatever the listings contained, so it is pointed at a name that cannot be in
# any listing. If that reports "found", the checker is broken and its verdicts are worthless.
# ---------------------------------------------------------------------------
echo "--> control: the membership check must reject a name that is not in the listing"
ABSENT_CONTROL='antenna-core this::test::does::not::exist_issue_74_control'
control_listing=$(nextest_list --profile full --workspace)
if selected_in "$control_listing" "$ABSENT_CONTROL"; then
  echo "ERROR: the membership check found '$ABSENT_CONTROL' in the workspace listing." >&2
  echo "       No such test exists, so the checker matches anything it is asked about and" >&2
  echo "       every 'selected' verdict below is meaningless." >&2
  exit 1
fi
if [ "$(printf '%s\n' "$control_listing" | wc -l | tr -d ' ')" -lt "$expected_count" ]; then
  echo "ERROR: the full-profile listing has fewer lines than the manifest has tests." >&2
  echo "       The listing is empty or truncated, so 'not found' below would mean nothing." >&2
  printf '%s\n' "$control_listing" >&2
  exit 1
fi

for profile_args in "--profile full" ""; do
  # shellcheck disable=SC2086 # deliberate word splitting: "" selects the default profile.
  listing=$(nextest_list $profile_args --workspace)
  if [ -n "$profile_args" ]; then
    label="--profile full"
    wanted="$expected"
  else
    label="(default profile — the dev inner loop)"
    wanted="$both_tiers"
  fi
  [ -n "$wanted" ] || continue
  echo "--> asserting the certification is selected by $label"
  missing=""
  while IFS= read -r line; do
    if ! selected_in "$listing" "$line"; then
      missing+="$line"$'\n'
    fi
  done <<< "$wanted"
  if [ -n "$missing" ]; then
    echo "ERROR: these numerical regressions are NOT selected by $label:" >&2
    printf '%s' "$missing" >&2
    echo "       Either they were renamed or moved (update $MANIFEST in the same commit)," >&2
    echo "       or a default-filter in .config/nextest.toml now excludes them. The suite" >&2
    echo "       still passes without them, which is why this check exists: the whole" >&2
    echo "       point of issue #74 is that these run in BOTH tiers, optimized. If a test" >&2
    echo "       genuinely has to leave the dev loop again, move its line below the" >&2
    echo "       [full-only] marker in $MANIFEST instead of deleting it." >&2
    exit 1
  fi
  echo "    ok: all $(printf '%s\n' "$wanted" | wc -l | tr -d ' ') selected"
done

# ---------------------------------------------------------------------------
# Invariant 2: antenna-core is still built optimized for tests.
#
# Read out of Cargo.toml rather than out of a build, because there is no way to ask a stable
# cargo for a unit's effective flags without recompiling it (`cargo rustc --print` is
# nightly-only, and `cargo build -v` prints "Fresh" for anything cached). The verified
# equivalent is `cargo build -p antenna-core --profile test -v`, which shows
# `-C opt-level=3 -C debug-assertions=on` — run it by hand if you want the empirical form.
#
# Both controls run first: the extractor must be able to READ a value that is known to be
# there, and must report NOTHING for an override that is known to be absent. Without the
# second, a broken extractor returning "" for everything would fail closed but for the wrong
# reason; without the first, one returning "" for everything would look like a missing
# override no matter what the file says.
# ---------------------------------------------------------------------------
opt_level_of() {
  # Value of `opt-level` inside one exact [section], empty if the section or key is absent.
  # Matched on the key, not on a field index: TOML accepts `opt-level = 3` and
  # `opt-level=3` alike, and an awk that reads $3 returns nothing for the second spelling —
  # which this script would report as a deleted override.
  # The header comparison strips double quotes, because Cargo accepts
  # [profile.dev.package."antenna-core"] for the same thing and an exact match on the
  # unquoted spelling would report that as a deleted override.
  awk -v section="[$1]" '
    { header = $0; gsub(/"/, "", header) }
    header == section { in_section = 1; next }
    header ~ /^\[/ { in_section = 0 }
    in_section && /^[[:space:]]*opt-level[[:space:]]*=/ {
      sub(/^[^=]*=[[:space:]]*/, "")
      gsub(/[[:space:]"]/, "")
      print
      exit
    }
  ' Cargo.toml
}

echo "--> control: the profile reader must read a known-present and a known-absent key"
release_opt=$(opt_level_of "profile.release")
if [ "$release_opt" != "3" ]; then
  echo "ERROR: the profile reader got '${release_opt:-<nothing>}' for [profile.release]" >&2
  echo "       opt-level, which Cargo.toml sets to 3. The reader is broken (a formatting" >&2
  echo "       change in Cargo.toml, or an awk that no longer parses it), so its verdict on" >&2
  echo "       the antenna-core override below would be worthless." >&2
  exit 1
fi
absent_opt=$(opt_level_of "profile.dev.package.no-such-package-issue-74-control")
if [ -n "$absent_opt" ]; then
  echo "ERROR: the profile reader returned '$absent_opt' for a section that cannot exist" >&2
  echo "       (no such package). It is matching the wrong section, so" >&2
  echo "       it cannot tell a configured override from a missing one." >&2
  exit 1
fi

# `profile.test` inherits `profile.dev` including package overrides, so this one section
# covers the test build too — verified with `cargo build -p antenna-core --profile test -v`,
# which reports `-C opt-level=3` with only this block present.
#
# The assertion is on the exact level, not on "some optimization", for the reason
# assert-dep-graphs.sh gives for CORE_MAX_DEPS: every figure recorded for this change was
# measured at 3, and `opt-level = 1`, `"s"` or `"z"` would quietly invalidate all of them
# while satisfying a looser test. Lowering it is a deliberate act that comes with re-measuring.
SECTION=profile.dev.package.antenna-core
EXPECTED_OPT_LEVEL=3
level=$(opt_level_of "$SECTION")
if [ "$level" != "$EXPECTED_OPT_LEVEL" ]; then
  echo "ERROR: [$SECTION] opt-level is '${level:-<nothing>}', expected $EXPECTED_OPT_LEVEL." >&2
  echo "       antenna-core must be compiled optimized in the debug and test builds: the" >&2
  echo "       numerical certification is aperture-integration bound, and without this the" >&2
  echo "       full suite goes from ~18 s back to ~110 s wall (~81 s to ~771 s CPU) and the" >&2
  echo "       cap test alone from 2.1 s to 36 s. Every test still passes, so nothing but" >&2
  echo "       this check can tell you. See the profile comment in Cargo.toml and" >&2
  echo "       docs/findings-2026-09-06-issue-74-optimized-numerical-certification.md." >&2
  exit 1
fi
echo "    ok: [$SECTION] opt-level = $level"
