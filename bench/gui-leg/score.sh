#!/bin/bash
# Score a minidb bench workspace: how many of the 42 seeded acceptance checks
# actually pass right now. This is the ONLY number that counts as a result --
# the harness's own "done" counter includes cancelled/closed tasks.
#
# Scores a SNAPSHOT, never the live directory.
# The tests are destructive by design: every one starts with `rm -f ./minidb_data`
# and several write ./out.txt. Running them in a live workspace races the agent.
# It is also how scores get MISread: scoring mid-write once reported 10/42 for a
# run that was really at 31/42.
#
# Worst case is 42 tests x 10 s per-test timeout = 420 s. A caller that wraps
# this in its own timeout must allow >= 480 s, or a slow-but-honest score gets
# silently killed and recorded as a blank (latent bug in the scratchpad-era
# driver: it wrapped the scorer in `timeout 300`).
#
# Usage: score.sh <workspace-dir>
# Env: GUI_LEG_LADDER overrides the pristine ladder dir (default: ladder-42
# next to this script — the committed copy, so a run that edited its own tests
# cannot score itself).
set -u

D="${1:?usage: score.sh <workspace-dir>}"
LADDER="${GUI_LEG_LADDER:-$(cd "$(dirname "$0")" && pwd)/ladder-42}"
PER_TEST_SECS=10
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT

if [ ! -f "$D/minidb" ]; then
  echo "workspace : $D"; echo "minidb    : ABSENT"; exit 0
fi

# Snapshot the artifact under test; take the tests from the pristine master.
cp "$D/minidb" "$TMP/minidb" || exit 1
mkdir -p "$TMP/tests" || exit 1
cp "$LADDER"/*.sh "$TMP/tests/" || exit 1

cd "$TMP" || exit 1
pass=0; fail=0; first_fail=""
# Per-test timeout: a work-in-progress minidb can infinite-loop.
for t in tests/test_*.sh; do
  if timeout "$PER_TEST_SECS" sh "$t" >/dev/null 2>&1; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    [ -z "$first_fail" ] && first_fail="$(basename "$t")"
  fi
done

echo "workspace : $D"
echo "verified  : $pass / $((pass + fail))"
echo "first fail: ${first_fail:-none}"
echo "minidb    : $(wc -c < "$D/minidb" | tr -d ' ') bytes, modified $(date -r "$D/minidb" '+%H:%M:%S')"

shadow=$(find "$D" -mindepth 2 -name minidb -not -path '*/tests/*' 2>/dev/null | head -3)
[ -n "$shadow" ] && echo "SHADOW COPIES FOUND: $shadow"
exit 0
