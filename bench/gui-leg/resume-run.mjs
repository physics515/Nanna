// Stall recovery with the resume contract ENCODED, not remembered:
//
//   1. A resume is an INTERJECTION — chat.send to the existing session, never
//      cancel+resend. The two semantics were measured against each other by
//      accident: cancel+resend took an artifact 13/42 -> 12 -> 9 (context-free
//      monolith reseeded mid-item), while a plain send took another leg from
//      ABSENT to 18/42 in one shot.
//   2. It fires ONLY when the run is NOT live. This script re-checks
//      get_run_state itself — even if the caller already did — because the
//      ministral leg fired fourteen resumes into a dead socket and a resume
//      that cannot verify "not live" must not fire at all. If the daemon is
//      unreachable the resume FAILS (exit 3); it does not fall through to a
//      blind send.
//   3. The caller records whether the next poll showed change (leg.sh writes
//      RESUME_EFFECT lines); this script prints a machine-readable line for it.
//
// Usage: node resume-run.mjs <session-id>
// Prints: RESUME_SKIPPED(live) | RESUME_SENT
// Exit: 0 skipped-or-sent, 1 usage, 2 send failed, 3 daemon unreachable.
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SESSION = process.argv[2];
if (!SESSION) { console.error('usage: node resume-run.mjs <session-id>'); process.exit(1); }

const MISSION_FILE = process.env.GUI_LEG_MISSION_FILE
  ?? join(dirname(fileURLToPath(import.meta.url)), 'mission.txt');
const MISSION = readFileSync(MISSION_FILE, 'utf8').trim();

const URL_ = process.env.NANNA_IPC_URL
  ?? `ws://127.0.0.1:${process.env.GUI_LEG_PORT ?? '5149'}/ws`;
const REQ_TIMEOUT = 120000;
setTimeout(() => { console.error('resume-run: overall timeout'); process.exit(3); }, REQ_TIMEOUT * 2);

const ws = new WebSocket(URL_);
const pending = new Map();
function req(action, timeoutMs = REQ_TIMEOUT) {
  return new Promise((resolve, reject) => {
    const id = randomUUID();
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, action }));
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`timeout: ${JSON.stringify(action).slice(0, 80)}`));
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
ws.onerror = () => { console.error('resume-run failed: daemon unreachable'); process.exit(3); };
ws.onopen = async () => {
  try {
    const rs = await req(
      { type: 'session', action: 'get_run_state', id: SESSION, light: true },
      20000,
    );
    if (rs?.is_running === true) {
      console.log('RESUME_SKIPPED(live)');
      process.exit(0);
    }
    await req({ type: 'chat', action: 'send', session_id: SESSION, content: MISSION, attachments: [] });
    console.log('RESUME_SENT');
    process.exit(0);
  } catch (e) {
    console.error('resume-run failed:', e.message ?? e);
    process.exit(2);
  }
};
