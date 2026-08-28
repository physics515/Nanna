// One-shot request over the daemon's WebSocket IPC. Reconstructed for the
// third and final time (the scratchpad original vanished while the script
// that called it nine times survived) — this copy is versioned, and legs run
// it from their staged, hash-recorded snapshot.
//
// Envelope (crates/nanna-daemon/src/protocol.rs): request {id, action} where
// action carries {"type": "...", "action": "...", ...fields}; response
// {id, result: {status: "success", data} | {status: "error", code, message}}.
//
// Usage:
//   node nanna-ipc.mjs req '<json action>' [timeout-ms]   print result data as JSON
//   node nanna-ipc.mjs probe [timeout-ms]                 system status; prints
//                                                         "running <uptime_secs>";
//                                                         exit 0 only if running
//
// Env: NANNA_IPC_URL overrides ws://127.0.0.1:5149/ws (GUI_LEG_PORT sets just
// the port). Exit codes: 0 ok, 1 usage, 2 timeout, 3 connection error,
// 4 daemon returned an error result.
import { randomUUID } from 'node:crypto';

const URL_ = process.env.NANNA_IPC_URL
  ?? `ws://127.0.0.1:${process.env.GUI_LEG_PORT ?? '5149'}/ws`;

const verb = process.argv[2];
let action;
let timeoutMs;
if (verb === 'req') {
  const raw = process.argv[3];
  if (!raw) { console.error('usage: nanna-ipc.mjs req <json-action> [timeout-ms]'); process.exit(1); }
  try { action = JSON.parse(raw); } catch (e) { console.error(`bad action JSON: ${e.message}`); process.exit(1); }
  timeoutMs = Number(process.argv[4] ?? 20000);
} else if (verb === 'probe') {
  action = { type: 'system', action: 'status' };
  timeoutMs = Number(process.argv[3] ?? 10000);
} else {
  console.error('usage: nanna-ipc.mjs req <json-action> [timeout-ms] | probe [timeout-ms]');
  process.exit(1);
}
if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 600000) timeoutMs = 20000;

// Hard ceiling on the whole process, not just the request: a connect that
// neither opens nor errors must still terminate (bounded, always).
setTimeout(() => { console.error('timeout'); process.exit(2); }, timeoutMs + 2000);

const id = randomUUID();
let ws;
try { ws = new WebSocket(URL_); } catch (e) { console.error(`connect failed: ${e.message}`); process.exit(3); }
ws.onerror = () => { console.error('ws error: connection failed'); process.exit(3); };
ws.onclose = () => { console.error('ws closed before response'); process.exit(3); };
ws.onopen = () => {
  ws.send(JSON.stringify({ id, action }));
  setTimeout(() => { console.error('timeout waiting for response'); process.exit(2); }, timeoutMs);
};
ws.onmessage = (m) => {
  let j;
  try { j = JSON.parse(m.data); } catch { return; } // ignore non-JSON frames
  if (j.id !== id) return; // ignore events / other traffic
  const r = j.result ?? {};
  if (r.status === 'error') {
    console.error(`daemon error: ${r.code ?? '?'}: ${r.message ?? ''}`);
    process.exit(4);
  }
  const data = r.data ?? r;
  if (verb === 'probe') {
    const running = data?.status === 'running';
    console.log(`${data?.status ?? 'unknown'} ${data?.uptime_secs ?? -1}`);
    process.exit(running ? 0 : 4);
  }
  console.log(JSON.stringify(data));
  process.exit(0);
};
