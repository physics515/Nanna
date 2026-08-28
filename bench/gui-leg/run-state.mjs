// Per-poll work denominator over IPC: is the session working, and how much
// work has it actually done? Emits ONE JSON line merging session run-state
// with daemon status, so a leg records work done — not just wall-clock — at
// every poll (the campaign compared 1121-tool legs against 28-tool legs in
// identical windows with nothing in the record saying so).
//
// The full T4b session-liveness verb (last step index, current stop state) has
// not landed; this uses session.get_run_state {light:true} — counters without
// the run journal — and the leg driver fills steps/stop-reasons from anchored
// log greps as the documented fallback.
//
// Usage: node run-state.mjs <session-id> [timeout-ms]
// Output on success (exit 0):
//   {"ok":true,"is_running":b,"tool_calls":n,"side_effecting":n,
//    "input_tokens":n,"output_tokens":n,"message_count":n,"queued":n,
//    "started_at":s|null,"uptime_secs":n,"daemon_status":"running"}
// On any failure (exit nonzero): {"ok":false,"error":"..."} — the caller must
// treat that as UNREACHABLE, never as zero work.
import { randomUUID } from 'node:crypto';

const SESSION = process.argv[2];
if (!SESSION) { console.error('usage: run-state.mjs <session-id> [timeout-ms]'); process.exit(1); }
let timeoutMs = Number(process.argv[3] ?? 20000);
if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 600000) timeoutMs = 20000;

const URL_ = process.env.NANNA_IPC_URL
  ?? `ws://127.0.0.1:${process.env.GUI_LEG_PORT ?? '5149'}/ws`;

// Which tool names count as touching the world. Heuristic by name — the
// registry doesn't expose a purity flag — kept in one place and overridable.
const SIDE_EFFECT_RE = new RegExp(
  process.env.GUI_LEG_SIDE_EFFECT_RE
  ?? 'exec|bash|shell|write|edit|create|delete|remove|move|rename|copy|append|patch',
  'i',
);

function bail(msg) { console.log(JSON.stringify({ ok: false, error: msg })); process.exit(2); }
setTimeout(() => bail('timeout'), timeoutMs + 2000);

let ws;
try { ws = new WebSocket(URL_); } catch (e) { bail(`connect failed: ${e.message}`); }
const pending = new Map();
function req(action) {
  return new Promise((resolve, reject) => {
    const id = randomUUID();
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, action }));
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`timeout: ${action.type}.${action.action}`));
    }, timeoutMs);
  });
}
ws.onmessage = (m) => {
  let j; try { j = JSON.parse(m.data); } catch { return; }
  if (!j.id || !pending.has(j.id)) return;
  const { resolve, reject } = pending.get(j.id);
  pending.delete(j.id);
  const r = j.result ?? {};
  if (r.status === 'error') reject(new Error(`${r.code}: ${r.message}`));
  else resolve(r.data ?? r);
};
ws.onerror = () => bail('ws error: connection failed');
ws.onclose = () => { if (pending.size > 0) bail('ws closed mid-request'); };
ws.onopen = async () => {
  try {
    const status = await req({ type: 'system', action: 'status' });
    if (status?.status !== 'running') bail(`daemon status: ${status?.status ?? 'unknown'}`);
    const rs = await req({ type: 'session', action: 'get_run_state', id: SESSION, light: true });
    const completed = Array.isArray(rs?.completed_tool_calls) ? rs.completed_tool_calls : [];
    const active = Array.isArray(rs?.active_tool_calls) ? rs.active_tool_calls : [];
    const sideEffecting = completed.filter((c) => SIDE_EFFECT_RE.test(c?.name ?? '')).length;
    console.log(JSON.stringify({
      ok: true,
      is_running: rs?.is_running === true,
      tool_calls: completed.length + active.length,
      side_effecting: sideEffecting,
      input_tokens: rs?.run_input_tokens ?? 0,
      output_tokens: rs?.run_output_tokens ?? 0,
      message_count: rs?.message_count ?? 0,
      queued: rs?.queued_count ?? 0,
      started_at: rs?.started_at ?? null,
      uptime_secs: status?.uptime_secs ?? -1,
      daemon_status: status.status,
    }));
    process.exit(0);
  } catch (e) {
    bail(e.message ?? String(e));
  }
};
