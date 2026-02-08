# Phase 8: Clawdbot Feature Parity

**Status:** 🔶 Mostly Complete — Core daemon, channels, webhooks, scheduler done. Browser relay, paired devices, and sub-agent sessions remain.

## Overview

The goal: Nanna can do everything Clawdbot can — always-on, multi-channel, fully autonomous. The key architectural insight is that the GUI is not a privileged controller; it's just another channel. ALL channels have full access to the control plane, with rendering adapted to their capabilities.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Control Plane                        │
│  ┌──────────┬──────────┬──────────┬──────────┬──────────┐  │
│  │ Sessions │ Memory   │ Config   │ Tools    │ Scheduler│  │
│  │ Manager  │ Browser  │ Manager  │ Registry │ /Cron    │  │
│  └──────────┴──────────┴──────────┴──────────┴──────────┘  │
│                            ▲                                │
│                            │ Full Access (all channels)     │
│  ┌─────────────────────────┴───────────────────────────┐   │
│  │                  Channel Router                      │   │
│  └──┬────────┬────────┬────────┬────────┬────────┬─────┘   │
└─────┼────────┼────────┼────────┼────────┼────────┼─────────┘
      ▼        ▼        ▼        ▼        ▼        ▼
  Telegram  Discord   GUI    CLI     API    Slack
```

## Current Implementation

### Daemon Mode ✅ Complete

**Location:** `crates/nanna-daemon/` (~8,000 lines, 18 files)

The daemon is well-architected with a clean builder pattern:

| File | Lines | Purpose |
|------|-------|---------|
| `control.rs` | 1,523 | Control plane — scheduler, workspace, system status |
| `server.rs` | 990 | `DaemonServer` + `DaemonBuilder` (fluent API, 15+ `with_*` methods) |
| `webhook.rs` | 891 | HTTP webhook server — routes per platform + generic `/:id` |
| `agent_service.rs` | 654 | Agent lifecycle management |
| `protocol.rs` | 482 | Wire types: `Action`, `Response`, `Event`, `SchedulerAction` |
| `session.rs` | 436 | `SessionManager` — CRUD + history |
| `main.rs` | 411 | CLI entry (clap): run/start/stop/restart/status/install/uninstall |
| `service.rs` | 386 | `ServiceStatus`, service management |
| `health.rs` | 364 | `HealthServer`, `HealthState`, `PidFile` |
| `user_tools.rs` | 365 | User-defined tool support |
| `ipc.rs` | 286 | `IpcServer` + config |
| `windows_service.rs` | 282 | Windows service integration (conditional) |
| `channels.rs` | 261 | Daemon-side channel orchestration |
| `persistence.rs` | 260 | `PersistenceManager` — auto-save |
| `llm_router.rs` | 202 | Model/provider routing |
| `memory_adapter.rs` | 75 | Glue to `nanna_memory` |
| `lib.rs` | 67 | 13 public modules + `DaemonError` |

**`DaemonBuilder`** creates a `DaemonServer` that spawns `ControlPlane`, `HealthServer`, `IpcServer`, `WebhookServer`, `SessionManager`, `PersistenceManager`, `AgentService`, `LlmRouter`. Clean separation of concerns.

**CLI:** `nanna-daemon run|start|stop|restart|status|install|uninstall`

**Issues:**
- **`control.rs` at 1,523 lines** is getting dense. Houses both the scheduler and general control plane logic. Should be split if more control operations are added.
- **No end-to-end testing** — The daemon mode + embedded fallback + reconnection path hasn't been integration tested.
- **No log rotation** — File-based logs accumulate without rotation.
- **Health-check based request timeout** — The GUI client uses a hard timeout for daemon requests. Long-running agent loops (30+ tool iterations) can be killed by the client before the daemon finishes. Should poll `get_session_run_state` instead.

**Suggestions:**
- Split `control.rs` into `control/scheduler.rs`, `control/workspace.rs`, `control/config.rs`, `control/system.rs`
- Add integration test suite that starts daemon, connects client, runs a conversation, and verifies persistence
- Implement log rotation with the `tracing-appender` crate (daily rotation, max 7 files)
- Replace hard timeout with health-check polling in `nanna-client`

---

### Channel Listeners ✅ Complete

**Location:** `crates/nanna-channels/` (~7,258 lines, 15 files)

**Core abstractions** (`lib.rs`, 414 lines):
- **`Channel` trait** (async): `send`, `react`, `edit`, `delete`, `pin`, `create_thread`, `reply_thread`, `send_typing`, `upload_file`, `send_poll` — most default to "unsupported"
- **`ChannelFeatures` bitflags**: 12 capability flags (reactions, replies, edits, threads, images, audio, documents, polls, pins, typing, uploads, stickers)
- **`MessageContent` enum**: Text, Image, Audio, Document, Location, Sticker, Poll
- **`MessageRouter`**: registers channels by name, `flume`-based incoming queue
- **`ListenerManager`**: coordinates multiple listeners with shared `mpsc` channel

**Listeners:**

| Provider | Lines | Transport | Notes |
|----------|-------|-----------|-------|
| Telegram | 423 | HTTP long-poll | Rich media (photo/doc/audio/voice/video/location/sticker) |
| Discord | 411 | WebSocket (Gateway v10) | Intents, guild filtering, resume/reconnect |
| Slack | 381 | WebSocket (Socket Mode) | App token + bot token, channel filtering |
| Signal | 416 | Poll or SSE | Via signal-cli REST API, `ReceiveMode` enum |
| WhatsApp | 500 | WS / SSE / Poll | Via bridge API, `BridgeStatus` check |

**Send modules** (outbound, per-provider): Discord (699), Slack (734), Telegram (703), Signal (555), WhatsApp (664)

**Infrastructure:**
- `queue.rs` (596 lines) — `MessagePriority` (Critical→Bulk), `RateLimiter` (token bucket), retry with exponential backoff
- `status.rs` (581 lines) — `ConnectionState` (7 states), `HealthMetrics`, `HealthChecker`

**Issues:**
- **Per-channel sessions not implemented** — All messages from a channel share context. Each chat/channel/DM should get its own session.
- **No message deduplication** — If a webhook and listener both receive the same message, it could be processed twice.
- **Telegram webhook vs long-polling** — Both exist but there's no automatic switching based on deployment mode.

**Suggestions:**
- Implement per-channel session mapping: `channel_id:chat_id → session_id`
- Add message ID deduplication with a short TTL cache (5 minutes)
- Auto-select Telegram mode: webhook when webhook server is enabled, long-polling otherwise
- Add channel-specific message formatting (Telegram MarkdownV2, Discord markdown, Slack mrkdwn)

---

### Client Library ✅ Complete

**Location:** `crates/nanna-client/` (567 lines, 3 files)

Thin WebSocket client with typed API surfaces:

| API | Methods |
|-----|---------|
| `SessionsApi` | list, get, create, delete, rename, clear, history |
| `ChatApi` | send, cancel, regenerate |
| `MemoryApi` | search, get, create, delete, stats |
| `ConfigApi` | get, set, reload, export |
| `ToolsApi` | list, get, execute |
| `SystemApi` | status, version, health, restart, shutdown |

**Issues:**
- **No SchedulerApi** — Scheduler operations exist in the daemon protocol but aren't exposed in the client library.
- **No WorkspaceApi** — Same gap for workspace operations.
- **No ChannelApi** — Channel management not exposed.

**Suggestions:**
- Add `SchedulerApi`, `WorkspaceApi`, `ChannelApi` to match daemon protocol coverage
- Add typed event subscription (currently raw JSON events)
- Consider gRPC or HTTP as alternative transport (WebSocket is fragile for some networks)

---

### Webhook Server ✅ Complete

**Location:** `crates/nanna-daemon/src/webhook.rs` (891 lines)

Axum-based HTTP server with per-platform routes:
- `/webhook/telegram` — Secret verification
- `/webhook/discord` — PING handling + Ed25519 signature verification
- `/webhook/slack` — URL verification challenge + signing secret
- `/webhook/whatsapp` — Verify token + Cloud API parsing
- `/webhook/:id` — Generic endpoint with Bearer/secret authentication

**Issues:**
- **No ngrok/tunnel integration** — Dev mode requires manual tunnel setup for webhook testing.
- **No request logging** — Webhook requests aren't logged for debugging.

**Suggestions:**
- Add optional `ngrok` integration for dev mode (auto-tunnel on startup)
- Add structured request logging with `tower-http` tracing layer
- Add webhook delivery retry tracking (store last N deliveries per endpoint)
- Add webhook signature verification metrics (track invalid signatures)

---

### Cron & Scheduled Jobs ✅ Complete

**Location:** `crates/nanna-daemon/src/control.rs` + `crates/nanna-core/src/cron.rs`

Scheduler actions: List, Get, Add, Update, Remove, RunNow, History.

**Issues:**
- **No timezone support** — All cron jobs run in UTC. Users need per-job timezone.
- **No missed job handling** — If the daemon was down when a job should have fired, it's simply missed.
- **No job dependencies** — Can't express "run B after A completes."

**Suggestions:**
- Add `chrono-tz` for per-job timezone configuration
- On startup, check for missed jobs and optionally run them (configurable per job: `run_if_missed: bool`)
- Implement simple job chaining: `after_job: Option<String>` field that triggers on completion
- Add job execution timeout (kill runaway jobs)

---

### Heartbeats 🔶 Minimal

**Location:** Referenced in `control.rs` as a scheduler task type and workspace context field.

The heartbeat concept exists as a scheduler task type but lacks the full Clawdbot semantics:
- No `HEARTBEAT.md` execution
- No inbox checking
- No proactive alerts
- No quiet hours

**Suggestions:**
- Implement `HEARTBEAT.md` parser: a workspace file that defines periodic tasks
  ```markdown
  # Heartbeat Tasks
  - Check email inbox and summarize unread
  - Review calendar for upcoming events
  - Check monitoring dashboards for alerts
  ```
- Add quiet hours configuration: `quiet_hours: { start: "22:00", end: "07:00", timezone: "America/Chicago" }`
- Implement proactive outreach: heartbeat can send messages to channels if something important is found
- Track heartbeat history: what was checked, what was found, what was sent

---

### Sub-Agent Sessions 🔜 Minimal

**Location:** `crates/nanna-agent/src/registry.rs` — `AgentRole::SubAgent`, `spawn_sub_agent()`

The `task` tool spawns sub-agents with isolated context, but the full sub-agent session system isn't built:
- No named session labels
- No inter-session messaging
- No session lifecycle management
- No GUI session monitor

**Suggestions:**
- Implement `SessionManager::spawn_child_session()` that creates a linked child session
- Add session labels: `spawn_session("research-competitor-analysis", config)`
- Implement inter-session messaging via the existing `AgentCoordinator` mailbox system
- Add session timeout with configurable limits
- Add result callbacks: parent session gets notified when child completes
- Build GUI view showing active sub-agent sessions with status, duration, token usage

---

### TTS (Text-to-Speech) 🔶 Partial

**Location:** `crates/nanna-tools/src/builtin/audio.rs`

Exists as `TextToSpeechTool` with `OpenAiTts` client. Default voice "nova". Callback-based `TtsFn` architecture supports multiple providers.

**Issues:**
- **Only OpenAI provider wired** — ElevenLabs, local TTS (Piper/Coqui) not implemented.
- **No per-channel TTS** — Can't enable TTS for Telegram but disable for Discord.
- **No voice message sending** — TTS generates audio but doesn't send it as a voice note on messaging platforms.
- **No caching** — Common phrases regenerated every time.

**Suggestions:**
- Add ElevenLabs provider (higher quality, more voices)
- Add local TTS via Piper (offline, zero-cost, runs on CPU)
- Implement per-channel TTS config:
  ```toml
  [tts.channels]
  telegram = { enabled = true, voice = "nova" }
  discord = { enabled = false }
  ```
- Wire TTS output to channel send: auto-convert text responses to voice notes when TTS is enabled for that channel
- Add audio caching with content hash keys (cache common greetings, etc.)

---

### Browser Relay ❌ Not Started

**Status:** Zero implementation. No structs, modules, or references exist anywhere in the codebase.

**What's needed:**
A Chrome extension ("Nanna Browser Relay") that lets Nanna control browser tabs. The extension communicates with the daemon via WebSocket.

**Proposed Architecture:**
```
Chrome Extension                     Nanna Daemon
┌─────────────────┐                 ┌──────────────┐
│ background.js   │◄───WebSocket───►│ relay server │
│ content.js      │                 │ (port 5150)  │
│ popup.html      │                 └──────────────┘
└─────────────────┘
```

**Implementation Plan:**
1. **Daemon relay server** — New WebSocket endpoint in `nanna-daemon` (port 5150 or path on existing health server)
2. **Chrome extension** — Manifest V3, `activeTab` + `scripting` permissions
   - `background.js` — Service worker, WebSocket connection to daemon
   - `content.js` — DOM access, element interaction, screenshot capture
   - `popup.html` — Simple UI to attach/detach tabs
3. **Relay protocol** — JSON messages:
   - `tab_list` — List attached tabs
   - `tab_snapshot` — Get DOM/accessibility tree
   - `tab_action` — Click, type, scroll, navigate
   - `tab_screenshot` — Capture visible viewport
   - `tab_evaluate` — Execute JavaScript
   - `tab_console` — Read console logs
4. **Nanna tools** — New tools: `browser_relay_snapshot`, `browser_relay_action`, `browser_relay_screenshot`

**Suggestions:**
- Start with a minimal MVP: attach tab → snapshot DOM → click element
- Use the accessibility tree (not raw DOM) for LLM consumption — much smaller and more meaningful
- Add tab filtering: only attached tabs are accessible (user must explicitly grant access)
- Consider Firefox extension support (WebExtension API is mostly compatible)

---

### Paired Devices (Nodes) ❌ Not Started

**Status:** Zero implementation. No concept of device pairing, node discovery, or multi-node coordination exists in the codebase.

This is architecturally complex and overlaps significantly with Phase 9 (Tor P2P). Consider deferring to Phase 9 where the full peer-to-peer infrastructure is planned.

**If implementing independently of Tor:**
- Use mDNS for local network discovery (`mdns-sd` crate)
- WebSocket relay for remote devices
- QR code pairing flow
- Per-device tool exposure (phone: camera, GPS; desktop: filesystem, GPU)

**Suggestions:**
- Defer to Phase 9 — the Tor-based approach is more robust and doesn't require network configuration
- If local-only is needed sooner, implement a simple WebSocket relay with shared secret pairing
- Start with the mobile app (Tauri Android) as the first "paired device"

---

### Gateway Control 🔶 Partial

**Location:** Spread across daemon

What exists:
- Config reload via `ConfigAction::Reload` in daemon protocol
- Status via `SystemAction::Status`
- Restart via `SystemAction::Restart`
- Shutdown via `SystemAction::Shutdown`

What's missing:
- **Self-update** — No auto-update mechanism
- **Backup/restore** — No full state backup (sessions + memory + config)
- **Live config reload from file** — Config file changes aren't watched

**Suggestions:**
- Add `notify` crate file watcher for `config.toml` with debounce
- Implement full state backup: export sessions, memories, config, scheduled jobs as a single archive
- Self-update can use GitHub releases API + `self_update` crate
- Add `/restart` and `/status` as channel commands (not just IPC)

---

## Channel Capabilities Matrix

| Channel | Markdown | Tables | Embeds | Buttons | Modals | Streaming |
|---------|----------|--------|--------|---------|--------|-----------|
| GUI     | ✓        | ✓      | ✓      | ✓       | ✓      | ✓         |
| Telegram| ✓        | -      | -      | ✓       | -      | -         |
| Discord | ✓        | -      | ✓      | ✓       | ✓      | -         |
| Slack   | ✓        | -      | ✓      | ✓       | ✓      | -         |
| Signal  | -        | -      | -      | -       | -      | -         |
| WhatsApp| ✓        | -      | -      | ✓       | -      | -         |
| CLI     | ✓        | ✓      | -      | -       | -      | ✓         |
| API     | -        | ✓      | -      | -       | -      | ✓         |

**Note:** The rendering adaptation based on channel capabilities isn't fully implemented. Currently, all channels receive the same raw text response. The `ChannelFeatures` bitflags exist but aren't used to adapt response formatting.

**Suggestion:** Implement a `ResponseFormatter` that takes `ChannelFeatures` and adapts the response:
- Strip markdown for Signal
- Convert tables to text for Telegram
- Add embeds for Discord
- Use Block Kit for Slack

---

## Pending Items Summary

| Item | Priority | Effort | Status |
|------|----------|--------|--------|
| Per-channel sessions | High | Medium | Not started |
| Sub-agent session management | Medium | Medium | Minimal |
| Response formatting per channel | Medium | Medium | Not started |
| Browser relay extension | Low | High | Not started |
| Paired devices / nodes | Low | High | Defer to Phase 9 |
| TTS multi-provider | Low | Medium | OpenAI only |
| End-to-end daemon testing | High | Medium | Not started |
| Client API completeness | Medium | Low | Missing scheduler/workspace/channel APIs |
| Log rotation | Medium | Low | Not started |
| Heartbeat full implementation | Medium | Medium | Minimal |
| Gateway self-update | Low | Medium | Not started |

## Implementation Order

1. **End-to-end testing** — Verify what we have actually works
2. **Per-channel sessions** — Essential for multi-channel deployment
3. **Client API completeness** — Wire remaining daemon features to client
4. **Response formatting** — Use ChannelFeatures to adapt output
5. **Heartbeat implementation** — HEARTBEAT.md execution
6. **Sub-agent sessions** — Named sessions with lifecycle
7. **TTS multi-provider** — ElevenLabs + local Piper
8. **Log rotation** — Operational necessity
9. **Browser relay** — Chrome extension MVP
10. **Paired devices** — Defer to Phase 9
