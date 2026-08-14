// Start one benchmark run bound to a FRESH workspace. Registering + activating
// a workspace is NOT enough: chat.rs re-points the tool working directory from
// the SESSION's workspace on every turn, so the session is created here, in the
// workspace, and the mission is sent to that session id. The GUI is a viewer.
//
// The session id is written to a file so the leg driver can poll
// session.get_run_state and resume by id instead of by newest-session
// heuristics. On a chat.send timeout the run state is checked before declaring
// failure: twice in the campaign a send "timed out" while the daemon had in
// fact accepted the mission and was sizing a context — the driver then started
// a competing leg against a live run.
//
// Usage: node start-run.mjs <workspace-path> <slug> <session-id-out-file>
// Env: NANNA_IPC_URL / GUI_LEG_PORT, GUI_LEG_MISSION_FILE (default: mission.txt
// next to this script).
import { randomUUID } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const WS_PATH = process.argv[2];
const SLUG = process.argv[3] ?? 'bench';
const SESSION_OUT = process.argv[4];
if (!WS_PATH || !SESSION_OUT) {
  console.error('usage: node start-run.mjs <workspace-path> <slug> <session-id-out-file>');
  process.exit(1);
}

const MISSION_FILE = process.env.GUI_LEG_MISSION_FILE
  ?? join(dirname(fileURLToPath(import.meta.url)), 'mission.txt');
const MISSION = readFileSync(MISSION_FILE, 'utf8').trim();

const URL_ = process.env.NANNA_IPC_URL
  ?? `ws://127.0.0.1:${process.env.GUI_LEG_PORT ?? '5149'}/ws`;
const REQ_TIMEOUT = 120000;

// The whole process is bounded regardless of which await wedges.
setTimeout(() => { console.error('start-run: overall timeout'); process.exit(2); }, REQ_TIMEOUT * 3);

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
ws.onerror = (e) => { console.error('ws error:', e.message ?? 'connection failed'); process.exit(3); };
ws.onopen = async () => {
  try {
    const od = await req({ type: 'workspace', action: 'open', path: WS_PATH });
    const wsId = od?.id ?? od?.workspace?.id ?? od?.workspace_id;
    if (!wsId) throw new Error(`no workspace id in open response: ${JSON.stringify(od).slice(0, 300)}`);
    console.log(`workspace open -> ${wsId}`);
    await req({ type: 'workspace', action: 'set_active', id: wsId });
    console.log('workspace set_active -> ok');
    let sd;
    try {
      sd = await req({
        type: 'session', action: 'create_in_workspace',
        name: `bench ${SLUG} ${new Date().toISOString().slice(0, 16)}`,
        workspace_id: wsId,
      });
    } catch {
      sd = await req({ type: 'session', action: 'create', name: `bench ${SLUG}`, workspace_id: wsId });
    }
    const sessionId = sd?.id ?? sd?.session?.id ?? sd?.session_id;
    if (!sessionId) throw new Error(`no session id: ${JSON.stringify(sd).slice(0, 300)}`);
    // Persist BEFORE sending: if the send below times out ambiguously, the leg
    // still knows which session to interrogate.
    writeFileSync(SESSION_OUT, sessionId);
    console.log(`session -> ${sessionId}`);
    try {
      await req({ type: 'chat', action: 'send', session_id: sessionId, content: MISSION, attachments: [] });
      console.log(`mission sent to ${sessionId} — run is live`);
      process.exit(0);
    } catch (e) {
      // Disambiguate "not delivered" from "accepted, slow first token".
      const rs = await req(
        { type: 'session', action: 'get_run_state', id: sessionId, light: true },
        20000,
      ).catch(() => null);
      if (rs?.is_running === true) {
        console.log(`chat.send response timed out but the run IS live (${sessionId}) — treating as started`);
        process.exit(0);
      }
      throw e;
    }
  } catch (e) {
    console.error('start-run failed:', e.message ?? e);
    process.exit(2);
  }
};
