// Cancel every session's in-flight chat so no stale mission competes with the
// leg about to start: stale bench sessions auto-resume on daemon boot and load
// THEIR model tag first, starving the new leg's context sizing.
import { randomUUID } from 'node:crypto';

const URL_ = process.env.NANNA_IPC_URL
  ?? `ws://127.0.0.1:${process.env.GUI_LEG_PORT ?? '5149'}/ws`;
setTimeout(() => { console.error('quiesce: overall timeout'); process.exit(3); }, 120000);

const ws = new WebSocket(URL_);
const pending = new Map();
function req(action) {
  return new Promise((resolve, reject) => {
    const id = randomUUID();
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, action }));
    setTimeout(() => { if (pending.delete(id)) reject(new Error('timeout')); }, 20000);
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
ws.onerror = () => { console.error('ws error: connection failed'); process.exit(3); };
ws.onopen = async () => {
  try {
    const ld = await req({ type: 'session', action: 'list' });
    const sessions = ld?.sessions ?? [];
    let n = 0;
    for (const s of sessions) {
      const id = s.id ?? s.session_id;
      if (!id) continue;
      try { await req({ type: 'chat', action: 'cancel', session_id: id }); n++; } catch { /* best effort */ }
    }
    console.log(`quiesced ${n} sessions`);
    process.exit(0);
  } catch (e) {
    console.error('quiesce failed:', e.message ?? e);
    process.exit(2);
  }
};
