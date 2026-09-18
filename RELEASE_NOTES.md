# Nanna v0.3.22-beta.31 — Every MCP Server, Both Directions

The last release said it plainly: **MCP servers speaking the newest protocol (2026-07-28) will not
connect**, and servers reached over HTTP were not started at all. That revision dropped the
`initialize` handshake Nanna's client opened with, so a current server simply refused it. Both are
fixed. Nanna now talks to MCP servers of every era, over every transport the spec has had, in both
directions: Nanna connecting to servers, and other MCP clients connecting to Nanna.

This was checked against the **official reference SDKs** (the TypeScript `@modelcontextprotocol`
server and client 2.0, and `server-everything`), and against a second, independent server on the
official Rust SDK (`rmcp` 3.4), not only against our own reading of the spec.
That mattered: the first real modern client rejected one of our answers over a caching field the
spec requires and our reading had missed.

## What's New

**Modern MCP servers connect.** Nanna first asks a server which protocol it speaks
(`server/discover`). If the server is 2026-07-28, it uses that; if not, it falls back to the older
`initialize` handshake. Nothing in your config names a protocol version.

**Remote MCP servers, from config.** An `[[mcp.servers]]` entry can now give a `url` instead of a
`command`. A token goes in the keyring and is named by `bearer_secret`, the same way `secret_env`
works for local servers. Nanna tries these in order: Streamable HTTP (2026-07-28, then the 2025
session style), then the deprecated 2024 HTTP+SSE transport. Older hosted servers that only speak
the old transport still work.

**A server's tools stay current.** When a server says its tool list changed, Nanna re-reads it and
the model sees the new tools on its next turn. Before this, a list read once at startup was never
read again. A server that says a call's routing headers are stale (`-32020`) gets its tools
re-listed and the call retried once.

**MCP servers can ask you questions.** When a server needs input mid-call (a confirmation, a missing
field), the question comes to you through the same `ask_user` prompt Nanna uses, in the app or the
chat app you wrote from. Your answer goes back to the server. Previously such a call was refused.

**The Tools page says how each server was reached**, for example "12 tools · 2026-07-28 over
Streamable HTTP", so a server that fell back to an old transport is visible.

**`nanna mcp serve` serves your running Nanna.** Point Claude Code or any MCP client at it and it
gets the tools of the daemon that is already running, with its config, keys and MCP servers. It
used to start a separate, bare copy. `--standalone` keeps the old behaviour. The server side speaks
2026-07-28 as well, so a modern-only client can connect.

**Telegram shows the answer as it is written.** In a private chat you now see Nanna's reply appear
as a live draft while she writes it. You see "Thinking…" until the first words arrive, instead of
"typing…". The draft has a **stop button**, and pressing it stops the turn exactly like sending
`/stop`.

**Ollama on another machine, with a token.** Settings → Models and the onboarding step both take
an Ollama server address and an optional bearer token. The address can be a local Ollama, one on
another machine, or any Ollama-compatible server behind a proxy. Give the path the server lives
under, for example `https://host/ollama`; Nanna adds `/api/…` itself. The token is kept in your OS
keychain and sent as `Authorization: Bearer …` to that server only: chat, embeddings, model details
and the connection check all carry it. When the server won't talk, you're told why. It wants a
token, or it refused the one you gave, or nothing Ollama-compatible answers at that address (which
usually means the path is missing).

## What's Fixed

**Nanna could fail to start at all.** If the embedding model was busy — the free OpenRouter model
answering "too many requests" was enough — opening Nanna showed *Starting* forever. At startup the
server asked that model one question to learn the shape of its vectors, and waited for an answer
with the same patience it uses for saving a memory in the background: up to eight minutes. The app
gave up after ninety seconds and stopped it, and every relaunch did the same. The startup question
is now asked once, without waiting; a busy model means starting on a provisional setting that
corrects itself as soon as the model answers. Checked against the real server with an embedding
endpoint that refuses everything: the previous release still had its door closed after 30 seconds,
this one opened in a quarter of a second.

**The Ollama setup step now checks.** Onboarding used to assume a local Ollama was running. It now
asks, and tells apart "Ollama isn't running" from "it's running but this model isn't pulled",
with the exact `ollama pull` to run. The model picker in Settings asks the same way. A server that
lists a model as `qwen` rather than `qwen:latest` no longer has it reported missing.

**Recovery acts on the Ollama server you set, and never kills a local one for a remote one.** When
chat against Ollama kept failing, Nanna unloaded the model and, as a last resort, restarted Ollama.
It did both to an Ollama on this computer (the one `OLLAMA_HOST` named, or the default address), no
matter which server chat was using. With a server on another machine, a failing proxy there meant
Nanna unloaded a model from the Ollama on your own computer and then killed it, and "waited for
Ollama to come back" by checking that one. Recovery now uses the address in Settings → Models. It
waits for that server wherever it is, sending its token, and it unloads or restarts only a server on
this computer. `OLLAMA_HOST` no longer changes which server Nanna looks at: a local Ollama on another
port is entered in Settings like any other address.

**A remote Ollama's context window is no longer sized from this computer's graphics card.** Nanna
sizes a local model's context window to the video memory free on this machine, and it did the same
for a server on another machine, whose card it cannot see. A busy card here could shrink a remote
model's window to the minimum. A remote server now starts at 16,384 tokens, the size Nanna uses
whenever the card cannot be read, and still steps down if that server runs out of memory. The size
is kept per server, so changing the server in Settings while Nanna runs no longer carries the old
server's size over, and a model used on two servers (summaries on this computer, chat on another)
keeps one for each.

**Commands Nanna runs from the Linux app picked up the app's own libraries.** The AppImage's
`LD_LIBRARY_PATH` leaked into every command, so tools like `git` loaded the bundled copies of
`libssl` and `libpcre2`. Commands now run with your system's.

**MCP servers are shut down with the daemon.** Before, they were left running. Now each server is
asked to exit, given two seconds, and then stopped. All servers are closed at once rather than one
by one.

**Linux without a keyring.** On a desktop with no Secret Service running, every secret operation
failed: storing an API key, reading one, anything. Secrets now fall back to the encrypted file
(created readable by you only), as they already did when the keyring refused a single entry.

**A killed app no longer leaves its server running (Linux).** If the app was killed rather than
closed, its background daemon kept running with nothing attached to it. It now notices within a
second and shuts down cleanly.

**A malformed IPC request gets an answer it can match.** Before, the error came back under the id
`"unknown"`, so the caller never saw its request fail. It now comes back under the request's own id.

## For Developers

- **The real app is driven over WebDriver on Linux.** `cargo build -p nanna-gui --features
  e2e-webdriver`, then run `gui/scripts/webdriver-smoke.sh <binary> <out dir>`. It runs the app fully
  isolated from your own Nanna: its own HOME, config, data directory and daemon port. Several recent
  GUI changes were verified this way for the first time: the onboarding Ollama check, the chat
  Files panel, the MCP server list, and a cleared session reloading. The feature is never part of a
  release build.
- **The live MCP interop suite runs in CI** (`mcp-interop.yml`), against the TypeScript reference
  SDKs pinned by a committed lockfile and a fixture server on the Rust SDK (`rmcp` 3.4).
- Toolchain pinned to `nightly-2026-09-17`. One dependency bump (`generator` 0.8.10); the `libc`
  and `malachite-bigint` pins still hold.

## What This Release Does Not Do

**The Telegram draft stream and stop button were not tried with a real bot.** There is no bot token
on the build host. They are tested against a scripted Bot API server, with request and update shapes
from the Bot API changelog and a mirror of its reference page.

**Conversation summaries do not use the Ollama server set above.** They still read their own
`[llm].ollama_url` and send no token, so a summarizer pointed at a server that requires one is
refused. A chat through a real remote model was not run either: the token path was driven against a
test server that refuses every request without it, and the real remote server only answered the
connection check.

**Only Linux was driven end to end tonight.** The Windows and macOS builds are covered by the
release workflow, not by a run of the real app.

## Numbers

- **2,224 Rust tests pass, 0 fail**; clippy reports no warnings. The live interop suite passes
  **11/11** against the real SDKs.
- Ways to reach an MCP server: **1 → 5**. Before, only stdio with the old handshake. Now stdio in
  both protocol eras, Streamable HTTP in both eras, and the 2024 HTTP+SSE transport.
