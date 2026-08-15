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
# HERMETIC: every test gets its OWN fresh directory, seeded with a copy of the
# same artifact snapshot and with MINIDB_FILE pointing inside it. The ladder used
# to run all 42 in one shared dir, so one test's leftovers (minidb_data, out.txt,
# alt_db, lock/backup files) were another test's starting state -- a later test
# could pass or fail because of an earlier one. That order-coupling hid a real
# divergence for an entire 5-leg series. A test's verdict must depend on the
# artifact and on nothing else. Directories are removed as soon as their test
# finishes, so peak disk is one test's residue, not 42.
#
# Worst case is 42 tests x 10 s per-test timeout = 420 s. A caller that wraps
# this in its own timeout must allow >= 480 s, or a slow-but-honest score gets
# silently killed and recorded as a blank (latent bug in the scratchpad-era
# driver: it wrapped the scorer in `timeout 300`).
#
# Output contract (drivers grep these): "verified  : N / 42" is the score line
# and stays the FIRST line containing the word "verified". The per-test map is
# emitted after it, never before.
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
# Every per-test directory is seeded from THIS snapshot, never from the live
# workspace, so all 42 tests score identical bytes even if the agent is still
# writing.
cp "$D/minidb" "$TMP/minidb" || exit 1
mkdir -p "$TMP/tests" || exit 1
cp "$LADDER"/*.sh "$TMP/tests/" || exit 1

pass=0; fail=0; first_fail=""; map=""; failed_list=""
# Per-test timeout: a work-in-progress minidb can infinite-loop.
for t in "$TMP"/tests/test_*.sh; do
  name="$(basename "$t")"
  rd="$TMP/run/${name%.sh}"
  mkdir -p "$rd" || exit 1
  cp "$TMP/minidb" "$rd/minidb" || exit 1
  # MINIDB_FILE is exported for a test that does not set its own; the tests
  # that do set it use ./minidb_data, which resolves inside $rd anyway because
  # that is the cwd. Either way the data file cannot escape the sandbox.
  if (cd "$rd" && MINIDB_FILE="$rd/minidb_data" timeout "$PER_TEST_SECS" sh "$t" >/dev/null 2>&1); then
    pass=$((pass + 1))
    map="$map."
  else
    fail=$((fail + 1))
    map="${map}X"
    failed_list="$failed_list $name"
    [ -z "$first_fail" ] && first_fail="$name"
  fi
  rm -rf "$rd" 2>/dev/null
done

echo "workspace : $D"
echo "verified  : $pass / $((pass + fail))"
echo "first fail: ${first_fail:-none}"
echo "minidb    : $(wc -c < "$D/minidb" | tr -d ' ') bytes, modified $(date -r "$D/minidb" '+%H:%M:%S')"
# Per-test map, in ladder order — one character per test, so two polls (or two
# legs) can be diffed to see exactly WHICH tests moved, not just how many.
echo "per-test  : $map (ladder order; . = pass, X = fail)"
echo "failed    :${failed_list:- none}"

shadow=$(find "$D" -mindepth 2 -name minidb -not -path '*/tests/*' 2>/dev/null | head -3)
[ -n "$shadow" ] && echo "SHADOW COPIES FOUND: $shadow"
exit 0
