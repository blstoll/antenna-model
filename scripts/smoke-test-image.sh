#!/usr/bin/env bash
# =============================================================================
# Container Smoke Test
# =============================================================================
#
# Boots a built image and drives it through the real HTTP surface. This exists
# because `docker build` succeeding proves almost nothing about the image: two
# separate defects shipped in the Dockerfile for a long time and neither could
# be caught by a build, only by running the result.
#
#   1. The builder was a Debian rust image (glibc 2.41) while the runtime is
#      UBI9 (glibc 2.34), so the binary died on startup with
#      "GLIBC_2.35 not found". The build was green.
#   2. The image set SERVICE_HOST/SERVICE_PORT, which the config loader does
#      not read. The service bound 127.0.0.1 inside the container, so the
#      published port reached nothing. The container stayed "Up".
#
# Defect 2 is why this checks reachability from OUTSIDE the container rather
# than exec-ing curl inside it, and defect 1 is why it fails loudly on an
# exited container instead of just waiting out the timeout.
#
# Usage:
#   scripts/smoke-test-image.sh <image-ref> [host-port]
#
# Examples:
#   scripts/smoke-test-image.sh antenna-model:local-test
#   scripts/smoke-test-image.sh ghcr.io/blstoll/antenna-model:main 3200
# =============================================================================

set -euo pipefail

IMAGE="${1:?usage: $0 <image-ref> [host-port]}"
PORT="${2:-3100}"
CONTAINER="antenna-model-smoke-$$"
TIMEOUT_SECS="${SMOKE_TIMEOUT_SECS:-60}"
BASE="http://localhost:${PORT}"

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

fail() {
  echo "SMOKE TEST FAILED: $*" >&2
  echo "--- container status ---" >&2
  docker ps -a --filter "name=${CONTAINER}" --format '{{.Status}}' >&2 || true
  echo "--- container logs ---" >&2
  docker logs "$CONTAINER" 2>&1 | tail -50 >&2 || true
  exit 1
}

echo "==> Starting $IMAGE on port $PORT"
docker run -d --name "$CONTAINER" -p "${PORT}:3000" "$IMAGE" >/dev/null

# Wait for readiness, but bail immediately if the process died. A container that
# exits on startup would otherwise burn the whole timeout before reporting.
elapsed=0
until curl -sf "${BASE}/health" >/dev/null 2>&1; do
  running=$(docker inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null || echo false)
  [ "$running" = "true" ] || fail "container exited before serving /health"
  elapsed=$((elapsed + 1))
  [ "$elapsed" -gt "$TIMEOUT_SECS" ] && fail "/health not reachable after ${TIMEOUT_SECS}s"
  sleep 1
done
echo "==> Reachable after ~${elapsed}s"

# Startup budget is a stated performance target (<10 s); worth knowing when the
# image drifts toward it even though this does not fail the build.
[ "$elapsed" -gt 10 ] && echo "WARNING: startup took ${elapsed}s, target is <10s"

check_status() {
  local path="$1" expected="$2" code
  # `|| true` keeps `set -e` from killing the script when curl itself fails
  # (connection reset, DNS): the empty code then reaches the comparison below and
  # fail() gets to dump the container status and logs, which is the whole point.
  code=$(curl -s -o /dev/null -w '%{http_code}' "${BASE}${path}") || true
  [ "$code" = "$expected" ] || fail "GET $path returned HTTP ${code:-<curl failed>}, expected $expected"
  echo "==> GET $path -> $code"
}

check_status /health 200
check_status /ready 200
check_status /status 200

# The service can report healthy with zero antennas loaded, which would make
# every gain request a 404. Assert it actually loaded its calibration data.
# Every capture below ends in `|| true`. Under `set -e` a failing command
# substitution aborts the script AT THE ASSIGNMENT, before the guard on the next
# line runs -- which would skip fail() and lose the container status and logs
# that are this script's entire reason to exist. Capture, then test.
antenna_count=$(curl -s "${BASE}/status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["antenna_count"])' 2>/dev/null) || true
[ -n "$antenna_count" ] || fail "could not read antenna_count from /status"
[ "$antenna_count" -gt 0 ] || fail "/status reports antenna_count=0; no calibration data loaded"
echo "==> /status antenna_count=$antenna_count"

# Drive one real computation. Liveness endpoints do not touch the physics path,
# the config, or the calibration artifacts, so they can pass on an image that
# cannot actually answer a query.
antenna_id=$(
  curl -s "${BASE}/api/v1/antennas" | python3 -c '
import json, sys
a = json.load(sys.stdin)["antennas"]
enabled = [x for x in a if x.get("enabled") and x.get("feed_ids")]
if not enabled:
    sys.exit("no enabled antenna with a feed")
print(enabled[0]["id"])
' 2>/dev/null
) || true
[ -n "$antenna_id" ] || fail "could not pick an enabled antenna from /api/v1/antennas"

# Pick the feed and the frequency together, from what the service itself declares.
# Taking feed_ids[0] and the example's 8400 MHz was a coincidence: the listing is
# sorted alphabetically, so the first feed is whatever sorts first (a Ka-band one
# for the shipped antennas.yaml), and the pair only survives because validation
# checks the global [100, 50000] band rather than the feed's range. Reordering
# antennas.yaml, or adding a per-feed check, would turn the smoke test into a 400
# that says nothing about whether the image works.
#
# Prefer a feed whose declared range covers the example's frequency, so the
# request keeps the example's physics; fall back to the midpoint of the first
# feed's range. Note the service currently reports the ANTENNA-level validity
# range as every feed's frequency_range_mhz (handlers.rs builds FeedInfo from
# cal.validity_ranges), so today every feed looks like it covers 8400 MHz. This
# stays correct if that is narrowed to the real per-feed band.
example_frequency=$(python3 -c '
import json
ex = json.load(open("examples/api_requests.json"))["examples"]
items = ex if isinstance(ex, list) else list(ex.values())
req = next(i["request"] for i in items
           if isinstance(i.get("request"), dict) and "vehicle_position" in i["request"])
print(req["frequency_mhz"])
' 2>/dev/null) || true
[ -n "$example_frequency" ] || fail "could not read frequency_mhz from examples/api_requests.json"
read -r feed_id frequency <<<"$(
  curl -s "${BASE}/api/v1/antennas/${antenna_id}/feeds" | python3 -c '
import json, sys
feeds = json.load(sys.stdin)["feeds"]
if not feeds:
    sys.exit("antenna reports no feed details")
want = float(sys.argv[1])
for f in feeds:
    lo, hi = f["frequency_range_mhz"]
    if lo <= want <= hi:
        print(f["id"], want)
        break
else:
    f = feeds[0]
    lo, hi = f["frequency_range_mhz"]
    print(f["id"], (lo + hi) / 2.0)
' "$example_frequency"
)"
[ -n "$feed_id" ] && [ -n "$frequency" ] ||
  fail "could not pick a feed for ${antenna_id} from /api/v1/antennas/${antenna_id}/feeds"
echo "==> Computing gain for ${antenna_id}/${feed_id} at ${frequency} MHz"

request=$(python3 -c '
import json, sys
ex = json.load(open("examples/api_requests.json"))["examples"]
items = ex if isinstance(ex, list) else list(ex.values())
req = next(i["request"] for i in items
           if isinstance(i.get("request"), dict) and "vehicle_position" in i["request"])
req["antenna_id"], req["feed_id"] = sys.argv[1], sys.argv[2]
# Both frequencies move together: pointing_frequency_mhz drives the focus/steering
# solution, so leaving it on the example value would aim a different band.
req["frequency_mhz"] = req["pointing_frequency_mhz"] = float(sys.argv[3])
print(json.dumps(req))
' "$antenna_id" "$feed_id" "$frequency" 2>/dev/null) || true
[ -n "$request" ] || fail "could not build a gain request from examples/api_requests.json"

response=$(curl -s -w '\n%{http_code}' -X POST "${BASE}/api/v1/gain" \
  -H 'content-type: application/json' --data "$request") || true
[ -n "$response" ] || fail "POST /api/v1/gain returned nothing"
code=$(printf '%s' "$response" | tail -n1)
body=$(printf '%s' "$response" | sed '$d')
[ "$code" = "200" ] || fail "POST /api/v1/gain returned HTTP $code: $body"

# Only assert the value is present and finite. The gain itself depends on the
# example's geometry, which is deliberately not this script's business.
#
# It asserts on gain_db, the always-present answer, not reference_gain_db --
# that one is Option + skip_serializing_if, so it is in the body only because
# the example happens to set include_reference, and a NaN gain_db alongside a
# finite reference would have passed.
printf '%s' "$body" | python3 -c '
import json, math, sys
d = json.load(sys.stdin)
g = d["gain_db"]
if not math.isfinite(g):
    sys.exit("gain_db is not finite: %r" % (g,))
print("==> gain_db=%.2f, warnings=%d" % (g, len(d.get("warnings", []))))
' || fail "gain response failed validation: $body"

echo "==> SMOKE TEST PASSED"
