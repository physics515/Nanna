# Nanna v0.3.21-beta.30 — Things That Talk Back

If you wrote to Nanna from Telegram, Discord or Slack, every message was answered with
**"I encountered an error processing your message."** Every one.

Not because the model failed — the model was never asked. Two things were wrong at once. The
conversation a chat message belongs to was never created, and the daemon refuses a message for a
conversation that does not exist. And even past that, the channel code read its reply out of the
daemon's response to "send this message" — which, since the daemon learned to acknowledge a message
the moment it arrives (so a long task never times out the sender), is a receipt, not an answer.

This release makes the chat apps real conversations again, and then builds on that: reminders that
arrive, questions Nanna can ask you, results that land where you asked for them, and an undo for the
files she writes.

## What's Fixed

**Chat apps get answers.** Each chat gets its own conversation, created on first contact with its
reply route remembered — so it survives a restart. When a turn finishes, the answer is sent back to
the chat it came from. If a message cannot be answered at all, the chat is told why ("no model
provider is configured"), not "an error". Webhook conversations are covered by the same path.

**Reminders work — and survive a restart.** `remind`, `list_reminders` and `cancel_reminder` were
the last three tools withheld at boot; **no bundled tool is missing its service any more**. A due
reminder is posted into the conversation that set it — in the app, and in the chat app you wrote
from. A reminder that came due while Nanna was not running is delivered at the next start and says
how late it is. Driven for real: set, delivered 7 s later; set, daemon stopped, restarted 90 s
later, delivered 6 s after boot with "is 2 min late".

**The scheduler no longer runs two copies of one job.** Anything still running when the next
30-second check came round was started again — a slow one-shot fired **8 times** in the test that
now pins it, a slow recurring job ran **11 copies at once**. And a fired one-shot was re-armed by
every restart. Both are gone.

**MCP servers actually start — and their tools would have broken every request.** The MCP client
was complete and nothing ever launched it. Configured servers now start at boot, in the background.
On the way: MCP tools were named `server:tool`, and the colon is invalid for both Anthropic and
OpenAI, which reject the *whole request* over it — the first MCP tool anyone added would have failed
every turn. They are named `mcp__server__tool` now.

**A failed tool call over IPC said nothing.** A direct tool call that failed answered `success:
false` with an empty string; the reason was dropped. It is returned now.

## What's New

**Ask when unsure.** `ask_user` posts a clarifying question into the conversation — app or chat app —
and waits up to half an hour for your reply, which the running task picks up and continues with. No
reply, and it carries on with its best judgement and says what it assumed.

**Undo for file writes.** Before `write_file`, `edit_file` or `file_buffer` changes a file, its
previous content is saved outside your project. Nanna can list and restore her own checkpoints
(`file_history`), and the chat header has a **Files** button that does the same for you. A restore
is itself undoable; restoring a file a tool created removes it. Bounded: 100 recent checkpoints per
conversation plus each file's first version, 256 MiB per conversation, 1 GiB overall.

**Chat commands.** From any chat app: `/status` (up? busy here? able to answer at all?), `/stop`
(cancel what Nanna is doing in that chat), `/model <name>` to pin that conversation's model
(`/model default` undoes it), `/help`.

**Export a chat** to Markdown or JSON from the session menu.

**Scheduled jobs can post their results into a conversation**, and so into a chat app. A quiet
heartbeat posts nothing.

**Hand edits of `config.toml` apply without a restart** (models, providers, scheduler switches). A
half-saved file that does not parse is logged and the running configuration is kept; saving the same
values changes nothing.

**Spend by day, month and conversation.** The per-request log existed with nothing writing it; it is
written now, and `system.cost_rollup` prices it. The Model Stats page shows spend by day for the last
30 days. Models with no list price (local, unknown) are named and marked, never counted as $0.

**Operations.** Prometheus `/metrics` on the health port (tools, models, MCP servers, channel
messages in and out, reminders, live runs). `system.status` and the Tools page show each MCP server
as started, failed (with the error) or not started (with the reason). `nanna doctor` checks MCP
commands are on `PATH`.

**Memory learns from use.** When Nanna recalls a result she stored earlier, the memories that
served it are credited — the first producer of the "used successfully" signal the memory system has
priced since July.

## What This Release Does Not Do

**None of this was driven through a live chat app.** There is no bot token on the build host. The
channel path is tested with a recording channel against the real control plane and session store; a
real Telegram round-trip is the remaining check.

**MCP servers speaking only the newest protocol (2026-07-28) will not connect.** That revision drops
the `initialize` handshake Nanna's client uses. Such a server is reported as failed, with its error,
on the Tools page and in `system.status` — but it does not work yet. Stdio servers only;
HTTP servers are not started from config.

**`ask_user` waiting inside a running task, and a scheduled job's result being posted after a real
model run, were not exercised end to end** — both need a live model turn. The mechanics under them
are tested against the real run registry and session store.

**GUI changes are not WebDriver-verified.** Linux still lacks a WebKitGTK driver matching the app's
WebKitGTK generation. The GUI builds, and its logic is covered by unit tests.

## Numbers

- **2,089 Rust tests and 268 GUI tests pass, 0 fail**; clippy reports 0 errors and no new warnings.
- **3 → 0** bundled tools withheld for want of a daemon service.
- **8 → 1** fires of a slow one-shot job; **11 → 1** concurrent copies of a slow recurring job.
- Dependencies: 19 compatible bumps, `deno_core` 0.411 → 0.412; `@vueuse/core` removed (nothing
  imported it).
