#!/bin/bash
# Drive ONE GUI-path benchmark leg end to end through the real chat path:
# GPU hygiene -> runtime model bind -> daemon bounce (output captured) ->
# fresh read-only workspace -> mission over IPC -> liveness-gated polling with
# per-poll work denominator + artifact snapshots -> summary with peak/final
# and a validity verdict. One leg per invocation.
#
# The instrument protects the number, in the order the campaign lost them:
#   IMMUTABLE  — first act: copy self + helpers into the run dir, record every
#                hash in the ledger header, exec the copy. Editing the working
#                tree affects the NEXT leg, never a running one (leg 4 died at
#                "line 152: unexpected EOF" when bash re-read an edited file
#                after 4 full hours of work).
#   LIVENESS   — no poll is recorded without a daemon probe first; 3
#                consecutive failures abort the leg as INVALID rather than
#                publish a number from a corpse (ministral: 14 ws errors in
#                the ledger, then an official 0/42 from a daemon 4h dead).
#   DENOMINATOR— every poll records work done (tool calls, side-effecting
#                calls, tokens, stop reasons, uptime), so a 28-tool leg can
#                never again sit beside a 1121-tool leg unmarked.
#   HISTORY    — every poll snapshots the artifact + scored verdict into
#                run/history/<minutes>/; the summary reports peak,
#                time-of-peak AND final (every archived artifact used to be
#                the post-collapse wreck; qwen's 22/42 peak existed nowhere).
#   SUPERVISED — a separate tiny supervisor kills the leg loudly on stale
#                heartbeat / dead worker (leg 4's death went unnoticed 19h35m;
#                gemma's driver hang was caught only by a human at the screen).
#
# Usage: leg.sh <ollama-model-tag> <slug>     e.g. leg.sh ornith:9b ornith-9b
#
# Env knobs (defaults for the reference bench machine):
#   GUI_LEG_BENCH_ROOT    run dirs live here          (D:/Development/nanna-bench)
#   GUI_LEG_DAEMON_EXE    daemon binary               (bench build in Cargo Target)
#   GUI_LEG_PORT          daemon IPC port             (5149)
#   GUI_LEG_LOGDIR        daemon tracing logs         (Roaming/nanna data/logs)
#   GUI_LEG_RUN_SECS      leg window                  (14400 = 4h)
#   GUI_LEG_POLL_SECS     score/snapshot cadence      (900 = 15m)
#   GUI_LEG_TICK_SECS     heartbeat/probe cadence     (60)
#   GUI_LEG_PINNED_CTX    frozen-series num_ctx       (16384)
#   GUI_LEG_DRYRUN        1 = no GPU / no daemon management (CI self-test)
#   GUI_LEG_MANAGE_DAEMON 0 = attach to a running daemon instead of bouncing
# Full list: grep 'GUI_LEG_' below — every knob has a bounded default.
set -u

MODEL="${1:?usage: leg.sh <model-tag> <slug>}"
SLUG="${2:?usage: leg.sh <model-tag> <slug>}"

BENCH_ROOT="${GUI_LEG_BENCH_ROOT:-D:/Development/nanna-bench}"
DAEMON="${GUI_LEG_DAEMON_EXE:-D:/Development/Cargo Target/debug/nanna-daemon-bench.exe}"
export GUI_LEG_PORT="${GUI_LEG_PORT:-5149}"
LOGDIR="${GUI_LEG_LOGDIR:-C:/Users/physi/AppData/Roaming/nanna/nanna/data/logs}"
RUN_SECS="${GUI_LEG_RUN_SECS:-14400}"
POLL_SECS="${GUI_LEG_POLL_SECS:-900}"
TICK_SECS="${GUI_LEG_TICK_SECS:-60}"
STALL_SECS="${GUI_LEG_STALL_SECS:-720}"
PINNED_CTX="${GUI_LEG_PINNED_CTX:-16384}"
CTX_WAIT="${GUI_LEG_CTX_WAIT_SECS:-90}"
PROBE_TIMEOUT_MS="${GUI_LEG_PROBE_TIMEOUT_MS:-15000}"
MAX_STREAK="${GUI_LEG_MAX_UNREACHABLE_STREAK:-3}"
MAX_UNREACHABLE_POLLS="${GUI_LEG_MAX_UNREACHABLE_POLLS:-2}"
MAX_INEFFECTIVE_RESUMES="${GUI_LEG_MAX_INEFFECTIVE_RESUMES:-3}"
MAX_RESUMES="${GUI_LEG_MAX_RESUMES:-24}"
VRAM_FLOOR="${GUI_LEG_VRAM_FLOOR_MIB:-7000}"
SCORE_TIMEOUT="${GUI_LEG_SCORE_TIMEOUT_SECS:-480}"
DRYRUN="${GUI_LEG_DRYRUN:-0}"
MANAGE="${GUI_LEG_MANAGE_DAEMON:-1}"
[ "$DRYRUN" = "1" ] && MANAGE=0
ALLOW_DEGRADED="${GUI_LEG_ALLOW_DEGRADED:-0}"

DRIVER_FILES="leg.sh supervisor.sh score.sh nanna-ipc.mjs run-state.mjs start-run.mjs resume-run.mjs quiesce-sessions.mjs mission.txt"

# =============================================================================
# STAGE 0 — immutability. Copy the driver into the run dir, hash everything,
# exec the copy. From here on nothing outside $RUN/driver is executed.
# =============================================================================
if [ -z "${GUI_LEG_STAGED:-}" ]; then
  SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
  STAMP="$(date +%Y%m%d-%H%M%S)"
  RUN="$BENCH_ROOT/run-$SLUG-$STAMP"
  mkdir -p "$RUN/driver" || { echo "ERROR cannot create $RUN"; exit 1; }
  for f in $DRIVER_FILES; do
    cp "$SRC_DIR/$f" "$RUN/driver/$f" || { echo "ERROR staging $f"; exit 1; }
  done
  cp -r "$SRC_DIR/ladder-42" "$RUN/driver/ladder-42" || { echo "ERROR staging ladder"; exit 1; }
  {
    echo "=== GUI-LEG $SLUG ($MODEL) ==="
    echo "started: $(date '+%Y-%m-%d %H:%M:%S %z')"
    echo "run_dir: $RUN"
    echo "driver_sha256:"
    # shellcheck disable=SC2086  # deliberate word splitting over the file list
    (cd "$RUN/driver" && sha256sum $DRIVER_FILES | sed 's/^/  /')
    echo "ladder_sha256: $(cd "$RUN/driver/ladder-42" && sha256sum test_*.sh | sha256sum | cut -d' ' -f1) (42 files, combined)"
    echo "config: RUN_SECS=$RUN_SECS POLL_SECS=$POLL_SECS TICK_SECS=$TICK_SECS PINNED_CTX=$PINNED_CTX MAX_STREAK=$MAX_STREAK MAX_UNREACHABLE_POLLS=$MAX_UNREACHABLE_POLLS DRYRUN=$DRYRUN MANAGE=$MANAGE"
  } > "$RUN/ledger.txt"
  exec env GUI_LEG_STAGED="$RUN" bash "$RUN/driver/leg.sh" "$MODEL" "$SLUG"
fi

# =============================================================================
# STAGED — everything below runs from the immutable copy.
# =============================================================================
RUN="$GUI_LEG_STAGED"
DRIVER="$RUN/driver"
LEDGER="$RUN/ledger.txt"
WS="$RUN/workspace"
HIST="$RUN/history"
export GUI_LEG_LADDER="$DRIVER/ladder-42"
export GUI_LEG_MISSION_FILE="$DRIVER/mission.txt"
mkdir -p "$HIST"
echo $$ > "$RUN/worker.pid"

# --- leg state (initialized before anything can fail: the summary and the
# --- verdict must be writable from every abort path). -------------------------
SESSION=""
probe_streak=0          # consecutive failed probes (ticks)
unreachable_polls=0     # polls recorded UNREACHABLE (feeds the summary refusal)
polls=0
mins=0
now=$(date +%s)
last_score="n/a"
last_is_running="null"
last_tools=0
last_tool_change=$now
gpu_bad_streak=0
caveats=""
resumes_fired=0; resumes_effective=0; resumes_ineffective_streak=0; resumes_suppressed=0
pending_resume=""       # "poll_idx tools artifact_sha score..." at resume time
prev_uptime=-1
PROBE_UPTIME=-1

say() { echo "[$(date '+%m-%d %H:%M:%S')] $*" | tee -a "$LEDGER"; }

# Terminal verdict: exactly one, first writer wins (worker or supervisor).
write_verdict() {
  ( set -C; printf '%s\n' "$1" > "$RUN/verdict" ) 2>/dev/null || return 0
  say "VERDICT $1"
}

add_caveat() { case " $caveats " in *" $1 "*) ;; *) caveats="$caveats $1";; esac; }

artifact_sha() {
  if [ -f "$WS/minidb" ]; then sha256sum "$WS/minidb" | cut -d' ' -f1; else echo "absent"; fi
}

status_line() { # $1 = phase (tick|poll), $2 = probe result (ok|fail)
  printf '{"t":%s,"minutes":%s,"phase":"%s","probe":"%s","probe_streak":%s,"uptime":%s,"is_running":%s,"tools":%s,"score":"%s"}\n' \
    "$(date +%s)" "$mins" "$1" "$2" "$probe_streak" "$PROBE_UPTIME" "$last_is_running" "$last_tools" "$last_score" \
    >> "$RUN/status.jsonl"
}

# --- IPC helpers (all bounded). ----------------------------------------------
ipc() { # ipc '<json action>' [timeout-ms]
  timeout $(( (${2:-20000} / 1000) + 10 )) node "$DRIVER/nanna-ipc.mjs" req "$1" "${2:-20000}"
}
probe() { # sets PROBE_UPTIME; returns 0 iff daemon answers status=running
  local out
  out=$(timeout $(( (PROBE_TIMEOUT_MS / 1000) + 10 )) node "$DRIVER/nanna-ipc.mjs" probe "$PROBE_TIMEOUT_MS" 2>/dev/null) || return 1
  PROBE_UPTIME="${out##* }"
  return 0
}

free_gpu() {
  # Orphaned llama-server holds VRAM and `ollama ps` will NOT list it.
  local m
  for m in $(curl -s --max-time 5 http://127.0.0.1:11434/api/ps \
      | tr ',' '\n' | grep -oE '"name":"[^"]+"' | cut -d'"' -f4); do
    curl -s --max-time 20 -X POST http://127.0.0.1:11434/api/generate -d "{\"model\":\"$m\",\"keep_alive\":0}" >/dev/null 2>&1
  done
  sleep 4
}

daemon_stop() {
  ipc '{"type":"system","action":"shutdown"}' 15000 >/dev/null 2>&1
  sleep 6
  powershell.exe -NoProfile -Command "Get-Process nanna-daemon,nanna-daemon-bench -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue" >/dev/null 2>&1
  sleep 3
}

daemon_start() { # $1 = stdout/stderr capture basename (daemon1 | daemon)
  # Output IS captured: the ministral crash is permanently unexplainable
  # because -WindowStyle Hidden with no redirection threw away the only
  # evidence a hard death leaves.
  local exe_win out_win err_win i
  exe_win=$(printf '%s' "$DAEMON" | sed 's|/|\\|g')
  out_win=$(printf '%s' "$RUN/$1.out" | sed 's|/|\\|g')
  err_win=$(printf '%s' "$RUN/$1.err" | sed 's|/|\\|g')
  powershell.exe -NoProfile -Command "Start-Process -FilePath '$exe_win' -ArgumentList '--port','$GUI_LEG_PORT','--host','127.0.0.1','run' -WindowStyle Hidden -RedirectStandardOutput '$out_win' -RedirectStandardError '$err_win'" >/dev/null 2>&1
  for i in $(seq 1 60); do
    probe && { sleep 3; return 0; }
    sleep 2
  done
  return 1
}

bind_model() {
  ipc "{\"type\":\"config\",\"action\":\"set\",\"path\":\"llm.model\",\"value\":\"ollama/$MODEL\"}" >/dev/null 2>&1
  ipc "{\"type\":\"config\",\"action\":\"set\",\"path\":\"llm.model_priority\",\"value\":[\"ollama/$MODEL\"]}" >/dev/null 2>&1
  # The SUMMARIZER picks its own model: left unpinned it loaded a second
  # 7.4GB local model mid-leg-start. One model serves everything during a leg.
  ipc "{\"type\":\"config\",\"action\":\"set\",\"path\":\"llm.summarization_priority\",\"value\":[\"ollama/$MODEL\"]}" >/dev/null 2>&1
}

# Anchored log reads: stale pre-leg lines can never satisfy a verification
# (the first campaign leg's context check was fooled exactly that way).
# Spans the UTC daily rotation by concatenating files newer than the anchor.
LOG0=""; LOGMARK=0
anchor_log() {
  LOG0="$LOGDIR/nanna-daemon.$(date -u '+%Y-%m-%d').log"
  LOGMARK=0; [ -f "$LOG0" ] && LOGMARK=$(stat -c%s "$LOG0")
}
log_since() {
  [ -n "$LOG0" ] || return 0
  tail -c "+$((LOGMARK + 1))" "$LOG0" 2>/dev/null
  local f base0 base
  base0=$(basename "$LOG0")
  for f in "$LOGDIR"/nanna-daemon.*.log; do
    [ -f "$f" ] || continue
    base=$(basename "$f")
    [[ "$base" > "$base0" ]] && cat "$f"
  done
}

# History dirs are named by elapsed minutes; with sub-minute poll cadences
# (dry-run) a second poll in the same minute gets a -pN suffix instead of
# silently overwriting the earlier snapshot.
hist_dir() {
  local mm
  mm=$(printf '%04d' "$mins")
  [ -e "$HIST/$mm/verdict.txt" ] || [ -e "$HIST/$mm/probe.txt" ] && mm="$mm-p$polls"
  printf '%s' "$HIST/$mm"
}

# =============================================================================
# SUMMARY — peak, time-of-peak, final; REFUSES a score past the validity gate.
# =============================================================================
summarize() {
  local out="$RUN/summary.txt"
  {
    echo "=== LEG SUMMARY $SLUG ($MODEL) ==="
    echo "verdict: $(cat "$RUN/verdict" 2>/dev/null || echo "MISSING")"
    echo "polls: total=$polls unreachable=$unreachable_polls (max allowed: $MAX_UNREACHABLE_POLLS)"
    echo "resumes: fired=$resumes_fired effective=$resumes_effective suppressed=$resumes_suppressed"
    echo "caveats:${caveats:- none}"
    if [ -f "$RUN/verdict" ] && grep -qE 'INVALID|ABORTED' "$RUN/verdict"; then
      echo "score: REFUSED — leg verdict is $(cat "$RUN/verdict")"
    elif [ "$unreachable_polls" -gt "$MAX_UNREACHABLE_POLLS" ]; then
      echo "score: REFUSED — $unreachable_polls polls were UNREACHABLE (> $MAX_UNREACHABLE_POLLS); a plausible number from a dead system is worse than none"
    else
      local peak=0 peak_at="none" final="none" final_at="none" d v
      for d in "$HIST"/*/; do
        [ -f "$d/verdict.txt" ] || continue
        v=$(grep -oE 'verified  : [0-9]+' "$d/verdict.txt" | grep -oE '[0-9]+$' || true)
        if [ -n "$v" ]; then
          final="$v/42"; final_at="$(basename "$d")m"
          if [ "$v" -gt "$peak" ]; then peak=$v; peak_at="$(basename "$d")m"; fi
        elif grep -q 'ABSENT' "$d/verdict.txt" 2>/dev/null; then
          final="ABSENT"; final_at="$(basename "$d")m"
        fi
      done
      echo "peak: $peak/42 at $peak_at"
      echo "final: $final at $final_at"
      echo "work (last denominator + anchored log):"
      local lastd
      lastd=$(ls -d "$HIST"/*/ 2>/dev/null | tail -1)
      [ -n "$lastd" ] && [ -f "$lastd/denominator.json" ] && sed 's/^/  ipc: /' "$lastd/denominator.json"
      [ -n "$lastd" ] && [ -f "$lastd/log-counters.txt" ] && sed 's/^/  /' "$lastd/log-counters.txt"
    fi
  } > "$out"
  tee -a "$LEDGER" < "$out"
}

fail_leg() { # $1 = INVALID(...)/ABORTED(...) — abort now, refuse to score.
  write_verdict "$1"
  say "LEG FAILED: $1 — no score will be emitted for this leg"
  summarize
  exit 1
}

# =============================================================================
# LOCK + SUPERVISOR
# =============================================================================
LOCK="$BENCH_ROOT/gui-leg.pid"
if [ -f "$LOCK" ] && kill -0 "$(cat "$LOCK")" 2>/dev/null; then
  say "ERROR another leg is running (pid $(cat "$LOCK")) — refusing"
  write_verdict "ABORTED(lock-held)"
  exit 1
fi
echo $$ > "$LOCK"

cleanup() {
  rc=$?
  # A worker that exits without a verdict is itself a process failure — name it.
  [ -f "$RUN/verdict" ] || write_verdict "ABORTED(worker-exit-rc=$rc)"
  [ -f "$RUN/supervisor.pid" ] && kill "$(cat "$RUN/supervisor.pid")" 2>/dev/null
  rm -f "$LOCK"
}
trap cleanup EXIT

say "=== LEG START $SLUG ($MODEL) staged in $RUN ==="

date +%s > "$RUN/heartbeat"
# Stale threshold: the longest LEGAL gap between heartbeats is one full poll
# body, dominated by the scorer's bound — not the tick period. Deriving it
# from the tick (3x60s) would let the supervisor kill a leg mid slow-but-
# honest score.
HB_MAX_AGE="${GUI_LEG_HEARTBEAT_MAX_AGE:-$((SCORE_TIMEOUT + 300))}"
SUP_MAX_SECS=$((RUN_SECS + 3600)) SUP_HEARTBEAT_MAX_AGE="$HB_MAX_AGE" \
SUP_PROBE_STREAK_MAX="$MAX_STREAK" \
  bash "$DRIVER/supervisor.sh" "$RUN" $$ >> "$RUN/supervisor.out" 2>&1 &
SUP_PID=$!
echo "$SUP_PID" > "$RUN/supervisor.pid"
say "  supervisor up (pid $SUP_PID, heartbeat max age ${HB_MAX_AGE}s)"

# =============================================================================
# PREFLIGHT (start) — asserted, not assumed. GPU, daemon, model bind, store.
# =============================================================================
if [ "$MANAGE" = "1" ]; then
  command -v nvidia-smi >/dev/null 2>&1 || { say "ERROR nvidia-smi not found — cannot assert exclusive GPU"; fail_leg "ABORTED(no-gpu-visibility)"; }
  free_gpu
  vram=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null || echo 99999)
  say "  gpu freed: ${vram} MiB used"
  [ "$vram" -lt "$VRAM_FLOOR" ] || { say "ERROR GPU not exclusive (${vram} MiB >= floor $VRAM_FLOOR)"; fail_leg "ABORTED(gpu-not-exclusive)"; }

  # Bounce #1: bind the model at RUNTIME over IPC (PR #139 provider rebuild) —
  # the same mechanism the GUI picker uses. Editing config files clobbered a
  # multi-line array once and the legacy boot path ignores [llm] anyway.
  daemon_stop
  daemon_start daemon1 || { say "ERROR daemon did not come up"; fail_leg "ABORTED(daemon-no-boot)"; }
  say "  daemon up (bench build)"
  bind_model
  bound=$(ipc '{"type":"config","action":"get","path":"llm.model"}' 2>/dev/null | tr -d ' "{}' | tail -c 80)
  say "  runtime model bound: ${bound:-<no response>} (wanted ollama/$MODEL)"

  # Quiesce stale sessions, then settle the card: quiesce-triggered work
  # (summarization) reloads models faster than one unload sweep can clear.
  timeout 180 node "$DRIVER/quiesce-sessions.mjs" 2>&1 | sed 's/^/  /' | tee -a "$LEDGER"
  stable=0; resident=0; vram2=99999
  for _ in $(seq 1 12); do
    free_gpu
    sleep 8
    resident=$(curl -s --max-time 5 http://127.0.0.1:11434/api/ps | grep -c '"name"')
    vram2=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null || echo 99999)
    if [ "$resident" -eq 0 ] && [ "$vram2" -lt "$VRAM_FLOOR" ]; then
      stable=$((stable + 1)); [ "$stable" -ge 2 ] && break
    else
      stable=0
    fi
  done
  say "  gpu settled post-quiesce: ${vram2} MiB used, resident=$resident (stable=$stable)"

  # FRESH DAEMON for the mission: the num_ctx latch is per-process and the
  # quiesce's summarization burst can size-and-latch under transient load
  # (poisoned at 4096 for the daemon lifetime, observed twice). The second
  # boot has no session storm — the card is quiet when the mission sizes.
  daemon_stop
  free_gpu
  daemon_start daemon || { say "ERROR daemon did not come back after quiesce restart"; fail_leg "ABORTED(daemon-no-reboot)"; }
  bind_model
  say "  fresh daemon up for the mission (latch clean)"
else
  probe || { say "ERROR no daemon reachable on port $GUI_LEG_PORT (MANAGE=0)"; fail_leg "ABORTED(daemon-unreachable-at-start)"; }
  say "  attached to running daemon (uptime ${PROBE_UPTIME}s, MANAGE=0)"
fi

anchor_log
say "  log anchor: $LOG0 @ $LOGMARK"

# Embedding-store health, recorded in the header either way: every campaign
# leg booted into a dimension-mismatch re-embed storm and two ran with the
# router fully benched — the legs were not equal conditions and nothing said so.
store_json=$(ipc '{"type":"system","action":"status"}' 2>/dev/null || echo '{}')
mem_degraded=$(printf '%s' "$store_json" | grep -o '"memory_degraded":[a-z]*' | cut -d: -f2)
mem_total=$(printf '%s' "$store_json" | grep -o '"total":[0-9]*' | head -1 | cut -d: -f2)
say "  memory store: total=${mem_total:-?} degraded=${mem_degraded:-?} isolation=shared-production-store"
if [ "${mem_degraded:-false}" = "true" ] && [ "$ALLOW_DEGRADED" != "1" ]; then
  fail_leg "ABORTED(embedding-store-degraded)"
fi

# Fresh workspace with the pristine read-only ladder (from the staged copy).
mkdir -p "$WS" || fail_leg "ABORTED(workspace-create)"
cp -r "$GUI_LEG_LADDER" "$WS/tests" || fail_leg "ABORTED(ladder-copy)"
count=$(find "$WS/tests" -name 'test_*.sh' | wc -l)
[ "$count" -eq 42 ] || { say "ERROR expected 42 tests, copied $count"; fail_leg "ABORTED(ladder-count-$count)"; }
chmod -w "$WS/tests"/*.sh || fail_leg "ABORTED(ladder-chmod)"
if (echo x > "$WS/tests/test_01.sh") 2>/dev/null; then
  say "ERROR tests still writable after chmod -w"; fail_leg "ABORTED(ladder-writable)"
fi
say "  workspace $WS (42 tests, read-only)"

# Send the mission; the session id is the leg's handle for run-state polls.
win_ws=$(printf '%s' "$WS" | sed 's|/|\\|g')
timeout 420 node "$DRIVER/start-run.mjs" "$win_ws" "$SLUG" "$RUN/session.id" 2>&1 | sed 's/^/    /' | tee -a "$LEDGER"
SESSION=$(cat "$RUN/session.id" 2>/dev/null || true)
[ -n "$SESSION" ] || fail_leg "ABORTED(no-session-id)"
say "  session $SESSION — mission sent"

# num_ctx verification: the pin must hold or the leg is invalid. Read only
# PAST the anchor, and confirm the sized model is OURS. HARD GATE: a leg not
# at the pinned value is not the frozen-series condition — kill it now
# instead of burning four hours (this gate converted two would-be 4-hour
# losses into 2.5-minute aborts).
sleep "$CTX_WAIT"
ctx=$(log_since | grep "sized ollama context" | grep -F "$MODEL" | tail -1 | grep -oE "num_ctx=[0-9]+")
say "  CONTEXT: ${ctx:-<none logged for $MODEL yet>}"
if [ "${ctx:-}" != "num_ctx=$PINNED_CTX" ]; then
  say "ERROR context gate failed (${ctx:-none}, want num_ctx=$PINNED_CTX) — cancelling mission"
  timeout 180 node "$DRIVER/quiesce-sessions.mjs" >/dev/null 2>&1
  fail_leg "INVALID(ctx-gate-${ctx:-none})"
fi

# =============================================================================
# POLL — runs only after a SUCCESSFUL probe this tick (liveness-gated).
# =============================================================================
do_poll() {
  polls=$((polls + 1))
  date +%s > "$RUN/heartbeat"
  local hd
  hd=$(hist_dir)
  mkdir -p "$hd"

  # --- preflight re-assert (per poll) ---
  # num_ctx: a mid-leg demotion silently invalidated a 4h bench once.
  local c
  c=$(log_since | grep "sized ollama context" | grep -F "$MODEL" | tail -1 | grep -oE "num_ctx=[0-9]+")
  if [ -n "${c:-}" ] && [ "$c" != "num_ctx=$PINNED_CTX" ]; then
    fail_leg "INVALID(ctx-demoted-mid-leg-$c)"
  fi
  # GPU exclusivity: every resident model must be ours.
  if [ "$MANAGE" = "1" ]; then
    local foreign
    foreign=$(curl -s --max-time 5 http://127.0.0.1:11434/api/ps \
      | tr ',' '\n' | grep -oE '"name":"[^"]+"' | cut -d'"' -f4 | grep -vF "$MODEL" || true)
    if [ -n "$foreign" ]; then
      gpu_bad_streak=$((gpu_bad_streak + 1)); add_caveat "gpu-shared"
      say "  WARN foreign model resident: $(echo "$foreign" | tr '\n' ' ') (streak $gpu_bad_streak)"
      [ "$gpu_bad_streak" -ge 3 ] && fail_leg "INVALID(gpu-not-exclusive)"
    else
      gpu_bad_streak=0
    fi
  fi
  # Embedding health: recorded, never silent (fatal only at leg start).
  local sj deg
  sj=$(ipc '{"type":"system","action":"status"}' 2>/dev/null || echo '{}')
  deg=$(printf '%s' "$sj" | grep -o '"memory_degraded":[a-z]*' | cut -d: -f2)
  [ "${deg:-false}" = "true" ] && { add_caveat "embedding-degraded"; say "  WARN memory store degraded"; }
  log_since | grep -q "embedding providers are benched" && add_caveat "embedding-benched"

  # --- work denominator: IPC first, anchored log greps as the fallback the
  # --- T4b session-liveness verb doesn't cover yet (steps / stop reasons). ---
  local den
  den=$(timeout 60 node "$DRIVER/run-state.mjs" "$SESSION" 2>/dev/null || echo '{"ok":false,"error":"run-state failed"}')
  printf '%s\n' "$den" > "$hd/denominator.json"
  {
    echo "executing_tool_lines: $(log_since | grep -c 'Executing tool')"
    echo "stop_reasons:"
    log_since | grep -oE 'stop=[A-Za-z]+' | sort | uniq -c | sed 's/^/  /'
  } > "$hd/log-counters.txt"
  local tools running
  tools=$(printf '%s' "$den" | grep -o '"tool_calls":[0-9]*' | cut -d: -f2)
  running=$(printf '%s' "$den" | grep -o '"is_running":[a-z]*' | cut -d: -f2)
  last_is_running="${running:-null}"
  if [ -n "${tools:-}" ] && [ "$tools" -gt "$last_tools" ]; then
    last_tools=$tools; last_tool_change=$now
  fi

  # --- artifact snapshot + scored verdict (liveness proven this tick) ---
  if [ -f "$WS/minidb" ]; then
    cp "$WS/minidb" "$hd/minidb" 2>/dev/null
    timeout "$SCORE_TIMEOUT" bash "$DRIVER/score.sh" "$hd" > "$hd/verdict.txt" 2>&1
  else
    timeout "$SCORE_TIMEOUT" bash "$DRIVER/score.sh" "$WS" > "$hd/verdict.txt" 2>&1
  fi
  date +%s > "$RUN/heartbeat"
  last_score=$(grep -E 'verified|ABSENT' "$hd/verdict.txt" | head -1 | tr -s ' ' | sed 's/^ *//')
  say "  ${mins}m  ${last_score:-<no score line>}  tools=${tools:-?} running=${running:-?}"

  # --- resume-effect bookkeeping (contract point 3) ---
  if [ -n "$pending_resume" ]; then
    local p_poll p_tools p_sha p_score effect
    read -r p_poll p_tools p_sha p_score <<< "$pending_resume"
    if [ "${tools:-0}" != "$p_tools" ] || [ "$(artifact_sha)" != "$p_sha" ] || [ "$last_score" != "$p_score" ]; then
      effect="changed"; resumes_effective=$((resumes_effective + 1)); resumes_ineffective_streak=0
    else
      effect="no-change"; resumes_ineffective_streak=$((resumes_ineffective_streak + 1))
    fi
    say "  RESUME_EFFECT resume-at-poll=$p_poll $effect (tools $p_tools->${tools:-?})"
    echo "{\"resume_poll\":$p_poll,\"checked_poll\":$polls,\"effect\":\"$effect\"}" >> "$RUN/resumes.jsonl"
    pending_resume=""
  fi

  # --- resume (contract points 1+2): interjection-only, fires ONLY when the
  # --- probe just succeeded but the run is NOT live. Never cancel+resend. ---
  if [ "${running:-}" = "false" ]; then
    if [ "$resumes_fired" -ge "$MAX_RESUMES" ] || [ "$resumes_ineffective_streak" -ge "$MAX_INEFFECTIVE_RESUMES" ]; then
      resumes_suppressed=$((resumes_suppressed + 1))
      say "  RESUME_SUPPRESSED (fired=$resumes_fired ineffective_streak=$resumes_ineffective_streak) — an ineffective resume must not be repeated blind"
    else
      say "  run not live — interjection resume"
      if timeout 300 node "$DRIVER/resume-run.mjs" "$SESSION" 2>&1 | sed 's/^/      /' | tee -a "$LEDGER" | grep -q "RESUME_SENT"; then
        resumes_fired=$((resumes_fired + 1))
        pending_resume="$polls ${tools:-0} $(artifact_sha) $last_score"
      fi
    fi
  elif [ "${running:-}" = "true" ] && [ $((now - last_tool_change)) -ge "$STALL_SECS" ]; then
    # Live but no new tool executions: record, do NOT cancel (the cancel+resend
    # shape regressed an artifact 13/42 -> 9/42; a live run's schedule is the
    # daemon's decision, not the bench's).
    say "  STALL_SUSPECTED run live but no new tool calls for $((now - last_tool_change))s"
  fi

  # Daemon restart detection: uptime went backwards.
  if [ "$PROBE_UPTIME" -ge 0 ] && [ "$prev_uptime" -ge 0 ] && [ "$PROBE_UPTIME" -lt "$prev_uptime" ]; then
    add_caveat "daemon-restarted"
    say "  WARN daemon restart detected (uptime ${prev_uptime}s -> ${PROBE_UPTIME}s)"
  fi
  prev_uptime=$PROBE_UPTIME
}

# =============================================================================
# MAIN LOOP — heartbeat + probe every tick; score/snapshot every poll.
# =============================================================================
deadline=$(( $(date +%s) + RUN_SECS ))
TICKS_PER_POLL=$(( POLL_SECS / TICK_SECS )); [ "$TICKS_PER_POLL" -ge 1 ] || TICKS_PER_POLL=1
max_ticks=$(( RUN_SECS / TICK_SECS + 2 ))

tick=0
while [ "$tick" -lt "$max_ticks" ] && [ "$(date +%s)" -lt "$deadline" ]; do
  left=$(( deadline - $(date +%s) ))
  if [ "$left" -lt "$TICK_SECS" ]; then sleep "$left"; else sleep "$TICK_SECS"; fi
  tick=$((tick + 1))
  now=$(date +%s)
  mins=$(( (RUN_SECS - (deadline - now)) / 60 ))
  date +%s > "$RUN/heartbeat"

  if probe; then
    probe_streak=0
    if [ $(( tick % TICKS_PER_POLL )) -eq 0 ]; then
      do_poll
      status_line "poll" "ok"
    else
      status_line "tick" "ok"
    fi
  else
    probe_streak=$((probe_streak + 1))
    say "  ${mins}m  probe FAILED (streak $probe_streak/$MAX_STREAK)"
    if [ $(( tick % TICKS_PER_POLL )) -eq 0 ]; then
      # Liveness-gated scoring: an unreachable poll records UNREACHABLE and
      # nothing else — never a score, never a resume, never "impl archived".
      polls=$((polls + 1)); unreachable_polls=$((unreachable_polls + 1))
      hd=$(hist_dir); mkdir -p "$hd"
      echo "UNREACHABLE" > "$hd/probe.txt"
      say "  ${mins}m  POLL UNREACHABLE (total $unreachable_polls)"
    fi
    status_line "tick" "fail"
    [ "$probe_streak" -ge "$MAX_STREAK" ] && fail_leg "INVALID(daemon-unreachable)"
  fi
done

# =============================================================================
# END OF WINDOW — final snapshot, integrity checks, verdict, summary.
# =============================================================================
say "$((RUN_SECS / 60))m MARK $SLUG"
mins=$(( RUN_SECS / 60 ))
final_ok=0
if probe; then
  final_ok=1
  polls=$((polls + 1))
  hd=$(hist_dir); mkdir -p "$hd" "$RUN/final"
  if [ -f "$WS/minidb" ]; then
    cp "$WS/minidb" "$hd/minidb"
    cp "$WS/minidb" "$RUN/final/minidb"
    timeout "$SCORE_TIMEOUT" bash "$DRIVER/score.sh" "$hd" > "$hd/verdict.txt" 2>&1 || true
    sed 's/^/    /' "$hd/verdict.txt" | tee -a "$LEDGER"
  else
    echo "minidb    : ABSENT" > "$hd/verdict.txt"
    say "    minidb : ABSENT at final mark"
  fi
else
  say "  final probe FAILED — no final score can be recorded"
  polls=$((polls + 1)); unreachable_polls=$((unreachable_polls + 1))
fi
diff -rq "$GUI_LEG_LADDER" "$WS/tests" >/dev/null 2>&1 \
  && say "  tests intact" || { say "  WARN tests were modified"; add_caveat "tests-modified"; }
say "  cuda faults (anchored log): $(log_since | grep -c 'illegal memory access')"

if [ "$MANAGE" = "1" ]; then
  ipc '{"type":"system","action":"shutdown"}' 15000 >/dev/null 2>&1
  sleep 5
fi

if [ "$unreachable_polls" -gt "$MAX_UNREACHABLE_POLLS" ]; then
  write_verdict "INVALID(unreachable-polls-$unreachable_polls)"
elif [ "$final_ok" -ne 1 ]; then
  write_verdict "INVALID(daemon-unreachable-at-final)"
elif [ -n "$caveats" ]; then
  write_verdict "VALID_WITH_CAVEATS(${caveats# })"
else
  write_verdict "VALID"
fi
summarize
say "LEG_DONE $SLUG $(cat "$RUN/verdict") dir=$RUN"
exit 0
