#!/bin/bash
# CI-safe self-tests for the gui-leg bench instrument. No GPU, no model, no
# Rust build — these protect the INSTRUMENT, not the score: the campaign's two
# most expensive failures (a published 0/42 from a daemon four hours dead, and
# a 4-hour leg containing 28 tool calls) were properties of driver control
# flow, reachable with a fake daemon in minutes.
#
#   1. Syntax:      bash -n on every shell driver (+ shellcheck -S error when
#                   available).
#   2. Scorer:      the known-good reference minidb must score 42/42; the empty
#                   stub must score 0/42 AND fail every individual ladder test;
#                   an empty workspace must report ABSENT.
#   3. Supervisor:  stale heartbeat kills the worker and writes INVALID;
#                   a worker that dies without a verdict is named loudly;
#                   a worker that exits WITH a verdict is a clean end.
#   4. Dry-run:     a full leg against a fake IPC daemon — healthy run ends
#                   VALID with peak/final in the summary and hashes in the
#                   ledger header; daemon-dies-mid-leg ends INVALID(daemon-
#                   unreachable) and the summary REFUSES to emit a score.
#
# Usage: bash run-self-tests.sh          (exit 0 = all green)
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
GL="$(dirname "$HERE")"           # bench/gui-leg
TMPROOT="$(mktemp -d)" || exit 1
FAKE_PIDS=""
cleanup() {
  for p in $FAKE_PIDS; do kill "$p" 2>/dev/null; done
  rm -rf "$TMPROOT"
}
trap cleanup EXIT

fails=0
pass() { echo "ok   - $1"; }
fail() { echo "FAIL - $1"; fails=$((fails + 1)); }

UTC_DATE="$(date -u '+%Y-%m-%d')"

# =============================================================================
# 1. Syntax
# =============================================================================
for f in "$GL/leg.sh" "$GL/supervisor.sh" "$GL/score.sh" "$HERE/run-self-tests.sh"; do
  if bash -n "$f" 2>/dev/null; then pass "bash -n $(basename "$f")"; else fail "bash -n $(basename "$f")"; fi
done
if command -v shellcheck >/dev/null 2>&1; then
  if shellcheck -S error "$GL/leg.sh" "$GL/supervisor.sh" "$GL/score.sh"; then
    pass "shellcheck -S error (drivers)"
  else
    fail "shellcheck -S error (drivers)"
  fi
else
  echo "note - shellcheck not installed; bash -n only"
fi

node_major=$(node -e 'console.log(process.versions.node.split(".")[0])' 2>/dev/null || echo 0)
if [ "$node_major" -ge 21 ]; then
  pass "node >= 21 (global WebSocket client; have $node_major)"
else
  fail "node >= 21 required for the IPC client (have ${node_major})"
fi

# =============================================================================
# 2. Scorer
# =============================================================================
W1="$TMPROOT/ws-oracle"; mkdir -p "$W1"
cp "$HERE/reference-minidb" "$W1/minidb"
out=$(bash "$GL/score.sh" "$W1")
if echo "$out" | grep -q "verified  : 42 / 42"; then
  pass "scorer: reference minidb scores 42/42"
else
  fail "scorer: reference minidb expected 42/42, got: $(echo "$out" | grep verified)"
fi

W2="$TMPROOT/ws-stub"; mkdir -p "$W2"
cp "$HERE/stub-minidb" "$W2/minidb"
out=$(bash "$GL/score.sh" "$W2")
if echo "$out" | grep -q "verified  : 0 / 42"; then
  pass "scorer: empty stub scores 0/42"
else
  fail "scorer: empty stub expected 0/42, got: $(echo "$out" | grep verified)"
fi

# Every ladder test must individually fail against the stub — a test an empty
# program can pass is a broken test.
W3="$TMPROOT/ws-each"; mkdir -p "$W3/tests"
cp "$HERE/stub-minidb" "$W3/minidb"
cp "$GL/ladder-42"/test_*.sh "$W3/tests/"
weak=""
for t in "$W3"/tests/test_*.sh; do
  if (cd "$W3" && timeout 10 sh "tests/$(basename "$t")" >/dev/null 2>&1); then
    weak="$weak $(basename "$t")"
  fi
done
if [ -z "$weak" ]; then
  pass "scorer: all 42 ladder tests fail the empty stub"
else
  fail "scorer: ladder tests passable by an empty stub:$weak"
fi

W4="$TMPROOT/ws-absent"; mkdir -p "$W4"
out=$(bash "$GL/score.sh" "$W4")
if echo "$out" | grep -q "ABSENT"; then
  pass "scorer: missing artifact reports ABSENT (never a number)"
else
  fail "scorer: missing artifact should report ABSENT, got: $out"
fi

# =============================================================================
# 3. Supervisor units
# =============================================================================
R1="$TMPROOT/sup-stale"; mkdir -p "$R1"
echo 1000 > "$R1/heartbeat"      # epoch 1000 = ancient
sleep 300 &
WP=$!
SUP_TICK=1 SUP_HEARTBEAT_MAX_AGE=5 SUP_MAX_SECS=60 bash "$GL/supervisor.sh" "$R1" "$WP" >/dev/null 2>&1
rc=$?
if [ "$rc" -eq 1 ] && grep -q "heartbeat-stale" "$R1/verdict" 2>/dev/null; then
  pass "supervisor: stale heartbeat -> INVALID(heartbeat-stale), exit 1"
else
  fail "supervisor: stale heartbeat (rc=$rc verdict=$(cat "$R1/verdict" 2>/dev/null))"
fi
sleep 1
if kill -0 "$WP" 2>/dev/null; then
  fail "supervisor: worker not killed on stale heartbeat"
  kill "$WP" 2>/dev/null
else
  pass "supervisor: worker killed on stale heartbeat"
fi

R2="$TMPROOT/sup-dead"; mkdir -p "$R2"
date +%s > "$R2/heartbeat"
sleep 1 &
WP2=$!
wait "$WP2" 2>/dev/null            # worker is now dead, no verdict written
SUP_TICK=1 SUP_MAX_SECS=30 bash "$GL/supervisor.sh" "$R2" "$WP2" >/dev/null 2>&1
rc=$?
if [ "$rc" -eq 1 ] && grep -q "worker-died" "$R2/verdict" 2>/dev/null; then
  pass "supervisor: dead worker without verdict -> INVALID(worker-died), exit 1"
else
  fail "supervisor: dead worker (rc=$rc verdict=$(cat "$R2/verdict" 2>/dev/null))"
fi

R3="$TMPROOT/sup-clean"; mkdir -p "$R3"
date +%s > "$R3/heartbeat"
echo "VALID" > "$R3/verdict"
sleep 1 &
WP3=$!
wait "$WP3" 2>/dev/null
SUP_TICK=1 SUP_MAX_SECS=30 bash "$GL/supervisor.sh" "$R3" "$WP3" >/dev/null 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
  pass "supervisor: dead worker WITH verdict -> clean exit 0"
else
  fail "supervisor: clean end treated as failure (rc=$rc)"
fi

# =============================================================================
# 4. Dry-run legs against the fake daemon
# =============================================================================
dry_env() { # $1 bench-root  $2 logdir  $3 port
  export GUI_LEG_DRYRUN=1
  export GUI_LEG_BENCH_ROOT="$1"
  export GUI_LEG_LOGDIR="$2"
  export GUI_LEG_PORT="$3"
  export GUI_LEG_RUN_SECS=40 GUI_LEG_POLL_SECS=10 GUI_LEG_TICK_SECS=2
  export GUI_LEG_CTX_WAIT_SECS=2 GUI_LEG_PROBE_TIMEOUT_MS=3000
  export GUI_LEG_SCORE_TIMEOUT_SECS=300 GUI_LEG_MAX_UNREACHABLE_STREAK=3
}
clear_dry_env() {
  unset GUI_LEG_DRYRUN GUI_LEG_BENCH_ROOT GUI_LEG_LOGDIR GUI_LEG_PORT \
    GUI_LEG_RUN_SECS GUI_LEG_POLL_SECS GUI_LEG_TICK_SECS GUI_LEG_CTX_WAIT_SECS \
    GUI_LEG_PROBE_TIMEOUT_MS GUI_LEG_SCORE_TIMEOUT_SECS GUI_LEG_MAX_UNREACHABLE_STREAK
}

# --- 4a. healthy: full window, VALID, peak/final, hashes ---------------------
B1="$TMPROOT/bench-healthy"; L1="$TMPROOT/logs-healthy"; mkdir -p "$B1" "$L1"
PORT1=5177
FAKE_PORT=$PORT1 FAKE_SCENARIO=healthy FAKE_RUN_TICKS=2 FAKE_MAX_SECS=300 \
  FAKE_LOG_FILE="$L1/nanna-daemon.$UTC_DATE.log" \
  node "$HERE/fake-daemon.mjs" >"$TMPROOT/fake1.out" 2>&1 &
FAKE_PIDS="$FAKE_PIDS $!"
sleep 2

# Simulated agent: plant the known-good artifact once the workspace exists so
# the leg has something real to snapshot and score (bounded retry — the
# workspace is created a few seconds into the leg).
dry_env "$B1" "$L1" "$PORT1"
(
  for _ in $(seq 1 30); do
    d=$(ls -d "$B1"/run-fake-healthy-*/workspace 2>/dev/null | head -1)
    if [ -n "$d" ]; then cp "$HERE/reference-minidb" "$d/minidb"; break; fi
    sleep 1
  done
) &
bash "$GL/leg.sh" fake fake-healthy >"$TMPROOT/leg1.out" 2>&1
rc=$?
clear_dry_env
RUN1=$(ls -d "$B1"/run-fake-healthy-* 2>/dev/null | head -1)
if [ "$rc" -eq 0 ] && grep -q "^VALID" "$RUN1/verdict" 2>/dev/null; then
  pass "dry-run healthy: leg completes with a VALID verdict"
else
  fail "dry-run healthy: rc=$rc verdict=$(cat "$RUN1/verdict" 2>/dev/null) (see leg1.out)"
  tail -30 "$TMPROOT/leg1.out"
fi
if grep -q "final: 42/42" "$RUN1/summary.txt" 2>/dev/null && grep -q "peak: 42/42" "$RUN1/summary.txt" 2>/dev/null; then
  pass "dry-run healthy: summary reports peak AND final"
else
  fail "dry-run healthy: summary missing peak/final: $(grep -E 'peak|final' "$RUN1/summary.txt" 2>/dev/null | tr '\n' ' ')"
fi
if grep -q "driver_sha256:" "$RUN1/ledger.txt" 2>/dev/null \
   && grep -q "leg.sh" "$RUN1/ledger.txt" 2>/dev/null \
   && [ -f "$RUN1/driver/leg.sh" ]; then
  pass "dry-run healthy: immutable staging + hashes in the ledger header"
else
  fail "dry-run healthy: staged driver / hash header missing"
fi
den=$(find "$RUN1/history" -name denominator.json 2>/dev/null | head -1)
if [ -n "$den" ] && grep -q '"tool_calls"' "$den"; then
  pass "dry-run healthy: per-poll work denominator recorded"
else
  fail "dry-run healthy: no denominator.json in history"
fi
if [ -n "$(find "$RUN1/history" -name minidb 2>/dev/null | head -1)" ]; then
  pass "dry-run healthy: per-poll artifact snapshot recorded"
else
  fail "dry-run healthy: no artifact snapshot in history"
fi

# --- 4b. die-after-mission: INVALID, score refused ---------------------------
B2="$TMPROOT/bench-die"; L2="$TMPROOT/logs-die"; mkdir -p "$B2" "$L2"
PORT2=5178
FAKE_PORT=$PORT2 FAKE_SCENARIO=die-after-mission FAKE_DIE_MS=2000 FAKE_MAX_SECS=300 \
  FAKE_LOG_FILE="$L2/nanna-daemon.$UTC_DATE.log" \
  node "$HERE/fake-daemon.mjs" >"$TMPROOT/fake2.out" 2>&1 &
FAKE_PIDS="$FAKE_PIDS $!"
sleep 2

dry_env "$B2" "$L2" "$PORT2"
bash "$GL/leg.sh" fake fake-die >"$TMPROOT/leg2.out" 2>&1
rc=$?
clear_dry_env
RUN2=$(ls -d "$B2"/run-fake-die-* 2>/dev/null | head -1)
if [ "$rc" -ne 0 ] && grep -q "INVALID(daemon-unreachable" "$RUN2/verdict" 2>/dev/null; then
  pass "dry-run die: dead daemon -> INVALID(daemon-unreachable), nonzero exit"
else
  fail "dry-run die: rc=$rc verdict=$(cat "$RUN2/verdict" 2>/dev/null) (see leg2.out)"
  tail -30 "$TMPROOT/leg2.out"
fi
if grep -q "score: REFUSED" "$RUN2/summary.txt" 2>/dev/null \
   && ! grep -qE "^(peak|final):" "$RUN2/summary.txt" 2>/dev/null; then
  pass "dry-run die: summary refuses to emit a score"
else
  fail "dry-run die: summary should refuse the score: $(cat "$RUN2/summary.txt" 2>/dev/null | tr '\n' ' | ')"
fi
if grep -q "UNREACHABLE" "$RUN2/ledger.txt" 2>/dev/null || grep -q "probe FAILED" "$RUN2/ledger.txt" 2>/dev/null; then
  pass "dry-run die: unreachable polls named in the ledger"
else
  fail "dry-run die: no UNREACHABLE marks in the ledger"
fi

# =============================================================================
echo
if [ "$fails" -eq 0 ]; then
  echo "ALL SELF-TESTS PASSED"
  exit 0
else
  echo "$fails SELF-TEST(S) FAILED"
  exit 1
fi
