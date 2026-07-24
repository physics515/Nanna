# Privacy

Nanna is **local-first**. The agent loop, memory, tools, and scheduler all run
on your machine. Nothing is sent to a remote service unless **you** configure a
cloud provider, a search key, or a messaging channel.

This document describes what is stored locally, what can leave the machine, how
to opt out, and how to delete your data.

## What is stored locally

| Data | Where | Purpose |
|------|-------|---------|
| Config (non-secret) | `config.toml` under the OS config dir (`com.nanna.nanna`) | Provider choice, model names, ports, feature toggles |
| Secrets (API keys, bot tokens) | OS keyring; AES-256-GCM encrypted file fallback (`credentials.enc`) only if no keyring is available | Authenticate to providers and channels |
| Conversations / sessions | Embedded Turso database in the data dir | Resume chat, run journal |
| Long-term memory | Same Turso database (FSRS-6 cognitive memory) | Recall across sessions |
| Task store | Turso | Long-horizon mission plans and notes |
| Tool scripts | Data dir `tools/` (+ any workspace `skills/`) | User-editable skills |
| Logs | Data dir `logs/` (daily rotation, ≤7 files) | Diagnostics |
| Model info cache | OS cache dir | Capability metadata for routing |

On Windows the canonical tree is under `%APPDATA%\nanna\nanna\` (config) and
`%LOCALAPPDATA%\nanna\nanna\` (data). Legacy installs that still use the old
`clawd\Nanna` paths are migrated automatically on first launch of a build
that includes the path unification.

**Memory is opt-in.** Auto-remembering every user/assistant turn is controlled
by `[memory] auto_remember_messages` and defaults to **`false`**. The agent can
still write memories deliberately via the `remember` tool; automatic extraction
of explicit memories during a run is gated by `[memory] enabled`.

## What can leave the machine

Only when the corresponding feature is configured:

| Destination | Trigger | What is sent |
|-------------|---------|--------------|
| **Cloud LLM providers** (Anthropic / OpenAI / OpenRouter) | You set a key and select a cloud model (or the router escalates) | Prompts, tool schemas, tool results, and returned completions. Embeddings go to OpenAI if `OPENAI_API_KEY` is set and the embedding provider is `openai`. |
| **Ollama / local runner** | Default local path | Stays on-machine (or on the host you pointed `ollama_host` at) |
| **Brave Search** | `web_search` tool + `BRAVE_API_KEY` | The search query string |
| **Channel platforms** (Telegram, Discord, Slack, Signal, WhatsApp) | Channel enabled + bot token | Outbound reply text/media; inbound messages are received from the platform |
| **Websites** | `web_fetch` / browser tools | Standard HTTP requests to URLs the agent (or you) chooses |
| **GitHub** | Optional `GITHUB_TOKEN` features | Per those API calls |

Nanna does **not** phone home. There is no telemetry endpoint, no analytics, and
no automatic crash reporter that uploads contents.

### Prompt caching

When talking to Anthropic or OpenAI, Nanna uses each provider’s native prompt
cache. Cache contents live on the provider’s side under their retention policy;
Nanna only stores the token accounting locally.

## How to keep a run fully local

1. Set `[llm] provider = "ollama"` (or, once shipped, `"local"`) and do **not**
   configure any cloud API key.
2. Set `[memory] embedding_provider = "ollama"` (or `"disabled"`).
3. Leave `BRAVE_API_KEY` unset (disables `web_search`).
4. Leave all channel tokens unset.
5. Do not enable the browser tools if you do not want outbound HTTP.

A stock Ollama-only install needs no cloud credentials and performs no
outbound calls beyond whatever you implicitly ask for (e.g. `web_fetch`).

## How to opt out of memory

- Set `[memory] enabled = false` to disable the memory service entirely.
- Set `[memory] auto_remember_messages = false` (the default) to keep
  deliberate `remember` / extraction without hoovering every turn.
- Use the GUI **Settings → Data → Delete All Memories**, or the `clear_memories`
  daemon action, to wipe the store.
- Pause is “turn `enabled` off”; there is no separate pause flag.

## How to delete / export your data

1. **Quit Nanna and stop the daemon** (`nanna daemon stop`, or Exit from the tray).
2. Delete the application directories:
   - Windows: `%APPDATA%\nanna\` and `%LOCALAPPDATA%\nanna\` (and, if present,
     the legacy `%APPDATA%\clawd\Nanna\` / `%LOCALAPPDATA%\clawd\Nanna\`).
   - macOS: `~/Library/Application Support/com.nanna.nanna/` and
     `~/Library/Caches/com.nanna.nanna/`.
   - Linux: `~/.config/nanna/` and `~/.local/share/nanna/`.
3. Clear keyring entries under the service name `nanna`
   (Windows Credential Manager / macOS Keychain / FreeDesktop Secret Service).
4. Optionally remove channel bot registrations on the provider side
   (Telegram BotFather, Discord developer portal, …).

Conversation export (Markdown/JSON) is tracked as an open GUI feature; until it
ships, the Turso database files themselves are the export.

## Channels and third parties

When a channel is connected, that platform’s own privacy policy applies to
anything you send or receive through it. Nanna stores channel messages in the
local session database so the agent has context; it does not relay them to any
other third party.

## Changes

We will update this file when data flows change. The `ROADMAP.md` security and
privacy items are the living checklist behind it.
