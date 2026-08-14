#!/bin/bash
# Tiny supervisor for one gui-leg worker. Deliberately dumb: it reads three
# facts (worker pid alive? heartbeat fresh? worker-reported probe streak) and
# fails the leg LOUDLY when any of them goes wrong. It never scores, never
# probes the model, never touches the workspace — the campaign's two biggest
# losses (a leg death unnoticed for 19h35m, a driver hang caught only by a
# human at the screen) were pure absence-of-a-watcher, and a watcher that does
# anything clever is a second thing that can hang.
#
# Usage: supervisor.sh <run-dir> <worker-pid>
# Env:
#   SUP_TICK               check cadence, seconds            (30)
#   SUP_HEARTBEAT_MAX_AGE  stale threshold, seconds          (210 = 3x60s + 30)
#   SUP_PROBE_STREAK_MAX   worker-reported unreachable limit (3)
#   SUP_MAX_SECS           absolute lifetime bound           (18000)
#
# Exit 0: worker finished and left a terminal verdict.
# Exit 1: the leg was failed by the supervisor (verdict written, worker killed).
set -u

RUN="${1:?usage: supervisor.sh <run-dir> <worker-pid>}"
WPID="${2:?usage: supervisor.sh <run-dir> <worker-pid>}"
TICK="${SUP_TICK:-30}"
MAX_AGE="${SUP_HEARTBEAT_MAX_AGE:-210}"
STREAK_MAX="${SUP_PROBE_STREAK_MAX:-3}"
MAX_SECS="${SUP_MAX_SECS:-18000}"

fail_loud() { # $1 = reason slug — verdict, banner, marker, kill, exit 1
  ( set -C; printf 'INVALID(%s)\n' "$1" > "$RUN/verdict" ) 2>/dev/null
  {
    echo "############################################################"
    echo "# LEG FAILED (supervisor): $1"
    echo "# run: $RUN"
    echo "# $(date '+%Y-%m-%d %H:%M:%S')"
    echo "############################################################"
  } | tee -a "$RUN/ledger.txt" >&2
  touch "$RUN/FAILED"
  kill "$WPID" 2>/dev/null
  sleep 2
  kill -9 "$WPID" 2>/dev/null
  exit 1
}

started=$(date +%s)
streak_seen=0
while :; do
  sleep "$TICK"
  now=$(date +%s)

  # Absolute bound: a supervisor that can outlive its leg indefinitely is a
  # fourth unbounded process, not a watchdog.
  if [ $((now - started)) -gt "$MAX_SECS" ]; then
    fail_loud "supervisor-timeout-${MAX_SECS}s"
  fi

  if ! kill -0 "$WPID" 2>/dev/null; then
    # Worker gone. With a terminal verdict that is a normal end; without one
    # it is the leg-4 shape: a full run's result silently lost.
    [ -f "$RUN/verdict" ] && exit 0
    fail_loud "worker-died"
  fi

  hb=$(cat "$RUN/heartbeat" 2>/dev/null || echo 0)
  case "$hb" in *[!0-9]*|'') hb=0;; esac
  if [ $((now - hb)) -gt "$MAX_AGE" ]; then
    fail_loud "heartbeat-stale-$((now - hb))s"
  fi

  # Backstop for a worker whose own abort logic is wedged: if it keeps
  # reporting an unreachable daemon past its limit for two consecutive checks
  # and still hasn't written a verdict, end the leg for it.
  streak=$(tail -1 "$RUN/status.jsonl" 2>/dev/null | grep -o '"probe_streak":[0-9]*' | cut -d: -f2)
  if [ -n "${streak:-}" ] && [ "$streak" -ge "$STREAK_MAX" ] && [ ! -f "$RUN/verdict" ]; then
    streak_seen=$((streak_seen + 1))
    [ "$streak_seen" -ge 2 ] && fail_loud "daemon-unreachable"
  else
    streak_seen=0
  fi
done
