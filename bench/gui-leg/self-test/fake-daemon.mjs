// Fake nanna-daemon for CI dry-runs of the leg driver: a minimal WebSocket
// server (raw http + hand-rolled framing — node has no built-in WS server and
// the bench must stay dependency-free) speaking just enough of the IPC
// protocol for leg.sh to run its full control flow with no GPU, no model and
// no Rust build.
//
// Scenarios (env FAKE_SCENARIO):
//   healthy            answers everything for the whole leg; get_run_state
//                      reports a live run with growing tool counters, then
//                      is_running=false after FAKE_RUN_TICKS state polls.
//   die-after-mission  answers normally until chat.send, then FAKE_DIE_MS
//                      later the process stops listening and destroys every
//                      socket — the void the ministral leg scored 14 times.
//   no-tools           run never live, zero tool calls — the lfm shape; the
//                      driver should resume, record no-change, and suppress.
//
// Env: FAKE_PORT (5150), FAKE_SCENARIO (healthy), FAKE_DIE_MS (3000),
//      FAKE_RUN_TICKS (6), FAKE_MAX_SECS (600) hard lifetime bound,
//      FAKE_LOG_FILE (unset) — on chat.send, append the tracing lines the leg
//      driver greps for (sized-context, Executing tool, stop=). Written at
//      send time so they land PAST the driver's log anchor, like the real
//      daemon's do. FAKE_NUM_CTX (16384) lets a test exercise the ctx gate.
import { createServer } from 'node:http';
import { createHash, randomUUID } from 'node:crypto';
import { appendFileSync } from 'node:fs';

const PORT = Number(process.env.FAKE_PORT ?? 5150);
const SCENARIO = process.env.FAKE_SCENARIO ?? 'healthy';
const DIE_MS = Number(process.env.FAKE_DIE_MS ?? 3000);
const RUN_TICKS = Number(process.env.FAKE_RUN_TICKS ?? 6);
const MAX_SECS = Number(process.env.FAKE_MAX_SECS ?? 600);

const started = Date.now();
let missionSent = false;
let dead = false;
let runStatePolls = 0;
const sockets = new Set();

// Bounded lifetime, always: a leaked fake daemon must not outlive its test.
setTimeout(() => process.exit(0), MAX_SECS * 1000);

// --- minimal WS framing ------------------------------------------------------
const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';

function frame(text) {
  const payload = Buffer.from(text, 'utf8');
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.from([0x81, len]);
  } else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x81; header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81; header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  return Buffer.concat([header, payload]);
}

// Parses one client (masked) frame from buf; returns {opcode, text, rest} or null.
function parseFrame(buf) {
  if (buf.length < 2) return null;
  const opcode = buf[0] & 0x0f;
  const masked = (buf[1] & 0x80) !== 0;
  let len = buf[1] & 0x7f;
  let off = 2;
  if (len === 126) {
    if (buf.length < 4) return null;
    len = buf.readUInt16BE(2); off = 4;
  } else if (len === 127) {
    if (buf.length < 10) return null;
    len = Number(buf.readBigUInt64BE(2)); off = 10;
  }
  const maskLen = masked ? 4 : 0;
  if (buf.length < off + maskLen + len) return null;
  let payload = buf.subarray(off + maskLen, off + maskLen + len);
  if (masked) {
    const mask = buf.subarray(off, off + 4);
    const un = Buffer.alloc(len);
    for (let i = 0; i < len; i++) un[i] = payload[i] ^ mask[i % 4];
    payload = un;
  }
  return { opcode, text: payload.toString('utf8'), rest: buf.subarray(off + maskLen + len) };
}

// --- protocol ----------------------------------------------------------------
function respond(action) {
  const type = action?.type;
  const act = action?.action;
  if (type === 'system' && act === 'status') {
    return {
      status: 'running',
      version: 'fake-0.0.0',
      uptime_secs: Math.floor((Date.now() - started) / 1000),
      sessions: 1,
      memory_degraded: false,
      memory_stats: { total: 7, active: 7 },
    };
  }
  if (type === 'system' && act === 'shutdown') return { status: 'shutting_down' };
  if (type === 'config') return { status: 'updated', value: action?.value ?? 'ollama/fake' };
  if (type === 'workspace' && act === 'open') return { id: 'ws-fake-1' };
  if (type === 'workspace' && act === 'set_active') return { ok: true };
  if (type === 'session' && (act === 'create_in_workspace' || act === 'create')) {
    return { id: 'sess-fake-1' };
  }
  if (type === 'session' && act === 'list') {
    return { sessions: [{ id: 'sess-fake-1', workspace_id: 'ws-fake-1' }] };
  }
  if (type === 'session' && act === 'get_run_state') {
    runStatePolls += 1;
    if (SCENARIO === 'no-tools') {
      return {
        is_running: false, completed_tool_calls: [], active_tool_calls: [],
        run_input_tokens: 12, run_output_tokens: 30,
        message_count: 1 + runStatePolls, queued_count: 0, started_at: null,
      };
    }
    const live = runStatePolls <= RUN_TICKS;
    const tools = Array.from({ length: Math.min(runStatePolls * 3, 50) }, (_, i) => ({
      id: `t${i}`, name: i % 3 === 0 ? 'exec' : 'read_file', input: null,
    }));
    return {
      is_running: live, completed_tool_calls: tools, active_tool_calls: [],
      run_input_tokens: runStatePolls * 1000, run_output_tokens: runStatePolls * 400,
      message_count: 2, queued_count: 0,
      started_at: new Date(started).toISOString(),
    };
  }
  if (type === 'chat' && act === 'send') {
    missionSent = true;
    if (process.env.FAKE_LOG_FILE) {
      const ts = new Date().toISOString();
      const ctx = process.env.FAKE_NUM_CTX ?? '16384';
      appendFileSync(process.env.FAKE_LOG_FILE, [
        `${ts} INFO sized ollama context model=fake num_ctx=${ctx}`,
        `${ts} INFO Executing tool exec`,
        `${ts} INFO Executing tool read_file`,
        `${ts} INFO chat run finished stop=EndTurn`,
        '',
      ].join('\n'));
    }
    if (SCENARIO === 'die-after-mission') {
      setTimeout(() => {
        dead = true;
        server.close();
        for (const s of sockets) s.destroy();
        // Keep the process alive but deaf so the port stays visibly dead
        // (an exited process frees the port; a wedged daemon does not).
        setTimeout(() => process.exit(0), MAX_SECS * 1000).unref?.();
      }, DIE_MS);
    }
    return { status: 'started', session_id: action.session_id, message_id: randomUUID() };
  }
  if (type === 'chat' && act === 'cancel') return { status: 'cancelled' };
  return { status: 'ok' };
}

const server = createServer((_req, res) => { res.writeHead(426); res.end(); });
server.on('upgrade', (req, socket) => {
  if (dead) { socket.destroy(); return; }
  const key = req.headers['sec-websocket-key'];
  const accept = createHash('sha1').update(key + GUID).digest('base64');
  socket.write(
    'HTTP/1.1 101 Switching Protocols\r\n'
    + 'Upgrade: websocket\r\nConnection: Upgrade\r\n'
    + `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  sockets.add(socket);
  socket.on('close', () => sockets.delete(socket));
  socket.on('error', () => sockets.delete(socket));
  let buf = Buffer.alloc(0);
  socket.on('data', (chunk) => {
    if (dead) { socket.destroy(); return; }
    buf = Buffer.concat([buf, chunk]);
    let f;
    while ((f = parseFrame(buf)) !== null) {
      buf = f.rest;
      if (f.opcode === 0x8) { socket.end(); return; }   // close
      if (f.opcode === 0x9) continue;                    // ping (ignore; clients here don't ping)
      if (f.opcode !== 0x1) continue;                    // text only
      let msg;
      try { msg = JSON.parse(f.text); } catch { continue; }
      const data = respond(msg.action ?? {});
      if (dead) return;
      socket.write(frame(JSON.stringify({ id: msg.id, result: { status: 'success', data } })));
    }
  });
});
server.listen(PORT, '127.0.0.1', () => {
  console.log(`fake-daemon listening on ${PORT} scenario=${SCENARIO}`);
});
