# Nanna v0.3.24-beta.33 — Plain Chat, Told Straight

Ask Nanna a question with a small local model and you could get the answer **seven times in a row,
followed by "could not finish: every planned task was abandoned."** Press Regenerate and the new
answer came with a warning that "nothing was written… if you expected something to exist by now, it
does not." Attach a picture and it never reached the model. Write in anything but plain ASCII and a
long reply could die halfway through with a UTF-8 error.

None of that was the model's fault. This release comes from driving the real conversation path —
daemon, IPC, client — against a scripted model, turn by turn, and fixing what it found. Every fix
below is now a test that runs on each pull request.

## What's Fixed

**An answer is given once.** Small models often answer and forget to say they are done. Nanna used
to ask again and again, streaming the same answer each time until it gave up. Two identical answers
with nothing done in between now count as finished — and the saved reply keeps one copy. (You may
briefly see the repeat while it streams; the seven copies and the "could not finish" are gone.)

**Replies read as prose.** When a task took several steps, each step's text ran straight into the
next — `…the file.The file says…`. Each step now starts a new paragraph, live and in history.

**No more false alarms on Regenerate.** The "repeat completion" warning exists for long jobs that
keep claiming success while nothing changes on disk. It no longer fires on a plain answer you asked
for again.

**Stop means stop.** A question you stopped used to be answered anyway on your next, unrelated
message. A turn now works what it planned. Unfinished work from earlier is shown to Nanna so she can
pick it back up if your message asks for it — and if she does, it is resumed, not duplicated. A
stopped reply is saved as "[Stopped by user]", as the app showed it, instead of an empty message.
Deleting a conversation now stops the work still running in it.

**Pictures reach the model.** Attached images were dropped, and underneath that, images had never
been sent to Ollama or OpenAI-compatible models at all. Both are fixed. A file Nanna cannot read
(a PDF, say) is named to her so she can tell you, instead of silently vanishing.

**Any language survives streaming.** A multi-byte character — an emoji, an accent, any CJK — split
across two network packets killed the whole reply with a UTF-8 error, on every provider. A long
non-English reply could fail after more than a minute of retries. Now it arrives intact. The same
bug corrupted Signal and WhatsApp messages into `��` and could silently drop MCP responses; fixed
there too.

**Memory works without an embedding provider.** With no embedder configured, `recall` told the model
"no memories found" about something saved seconds earlier. It now falls back to matching words and
says that is what it did.

**Honest reports.** A follow-up question is no longer told that an unchecked answer "passed its
check". A task abandoned after 6 steps no longer says 8. A reply that is just "TASK COMPLETE" says it
finished without a reply instead of showing nothing. Internal notes meant for the model no longer
appear in the report you read.

**Big tool calls stay in context.** A tool call with a huge argument pushed Nanna's own call out of
her memory of the conversation. Large arguments are now summarized the same way large results are.

**Reasoning stays out of replies.** Models that write `<think>…</think>` inline had their whole
chain of thought delivered as the answer. It is now kept as reasoning.

**Ctrl+Enter right after typing always sends.** The composer judged whether it was empty from a
copy of your text that lagged a frame or two behind, so a message sent the instant it was typed
could stay in the box, unsent and unexplained. It now reads the text itself. Sending with
Ctrl+Enter also no longer leaves an invisible line break behind that kept the Send button lit.

**Each provider's API key stays with that provider.** A key that `nanna init` saved for OpenRouter
or OpenAI was also read as the Anthropic key, so a `claude-*` chat or an Anthropic summary could
send it to Anthropic. Every provider now has its own field and keyring entry. The first time Nanna
loads a config saved the old way, it moves the key to its provider's entry, once. A key with
Anthropic's `sk-ant-` prefix is never moved. Neither is a key that disagrees with one already in the
provider's own place: both are kept, and a warning names the conflict.

**Small things.** Tool errors no longer read `Error: Error: …`; tool calls from Ollama get unique ids;
a blank message is refused instead of being answered as if you had asked something.
A message too long for the model's context window now says exactly that, instead of blaming GPU
memory.

## What's New

**The Logs page can follow one conversation.** Every log line now carries the conversation, step
and tool call it came from, and search matches it — paste a session id to see just that
conversation.

**Structured tracing.** The daemon's log names each turn, step, model call and tool call, with how
long it took, how many tokens it used and whether it succeeded.

**Two new CI gates.** The end-to-end conversation suite runs on every pull request, and a dependency
audit (RustSec and npm) runs on every lockfile change and weekly.

## Still Open

- The converging answer still streams twice before the saved reply drops the repeat — the second
  copy is the signal, and it cannot be recognized until it has streamed.
- Work you never return to stays open (and visible to Nanna) indefinitely.
- Images sent while a task is already running join it as text only.
- Not verified against a real vision model or a live Signal/WhatsApp bridge — none on the build host.

---

# Also in this update

Version 0.3.23 was prepared but never published on its own, so its notes below are new to you as
well. Version 0.3.22 was published; its notes are here for anyone updating from 0.3.21.

## Nanna v0.3.23-beta.32 — Measuring Before Building

This release is mostly about the memory system, and mostly about deciding things with numbers
instead of with arguments. Three long-standing questions in the roadmap were each waiting on a
measurement that nobody had taken. All three are now answered, and two of them turned out to have
an answer nobody expected.

Recall got faster. Dreaming got faster. Neither is a rewrite — they are the cheap wins, taken
deliberately so that the expensive one can be judged on evidence rather than intuition.

### What's Faster

**Recall stopped ranking your whole memory store to return ten results.** Every time Nanna recalls
something — and she recalls on every ingest, not just when you ask — she scored every memory and
then **sorted all of them** before keeping the top handful. At fifty thousand memories that is
sorting fifty thousand things to keep ten. It now selects rather than sorts, which is **5-18% off
the entire recall path**, cosine included.

That change came with a quieter correctness fix. The ranking relied on sort stability to break
ties, and ties are ordinary — every memory that cannot be compared scores identically. It now
orders ties explicitly, so repeated recalls return the same memories in the same order. And a
memory with a degenerate embedding used to produce a score that was *not a number*, which the old
comparator treated as "equal to everything" and could seat anywhere in your results, including
first. It now ranks last, where it belongs.

**Dreaming got about a third cheaper on a long-lived store.** Two passes, both of which leave the
result bit-for-bit identical:

- The similarity kernel was recomputing each memory's magnitude for every pair it looked at, inside
  a loop that looks at every pair. Those are now computed once. Worth 11-18%.
- Pairs that **cannot possibly** be close enough to merge are now rejected before Nanna reads
  their embeddings at all. On a store spread over months this skips **21.9% of all comparisons**,
  for another 16-22%.

The second one is exact rather than clever: it computes the best score a pair could achieve with a
perfect match, and skips only when even that is not good enough. Nothing that would have merged is
missed, and the test suite proves it by dreaming the same store twice, with and without the
shortcut, and requiring identical results.

### What We Learned, Including Where We Were Wrong

**Keeping memories in RAM beats querying them from the database, by 9-14x.** Nanna can search
memories with an exact database query that uses almost no memory, and it was an open question
whether that should become the normal path. It should not: the in-memory scan is between nine and
fourteen times faster, and the cost of loading memories in the first place pays for itself after
**two or three searches**. The database path stays for stores too large to hold in memory, and we
now know the size where that starts to matter — around 350,000 memories, roughly seven times
today's practical ceiling.

**Making the arithmetic cheaper does not fix a quadratic, and we measured how little it helps.**
Removing two thirds of the similarity kernel's arithmetic was expected to be worth two to three
times. It was worth thirteen percent, because the work is limited by memory bandwidth, not by
arithmetic. Recorded plainly, because it closes off a whole direction: for a store of half a
million memories, every cheap optimization available — a third of the arithmetic, a fifth of the
comparisons — moves a dream cycle from about 73 minutes to about 47. The next step has to be
comparing fundamentally fewer pairs, not comparing them faster.

**Thirteen of Nanna's twenty-one memory-decay parameters do not do anything.** They are public,
they are saved to your config, several are non-zero, and they look exactly like tuning knobs.
Eight of them are wired to a formula; the rest are a table inherited from a published algorithm
whose update rules Nanna does not use. This is now proven by a test that perturbs each one and
checks whether anything moves, so the documentation cannot quietly go stale — and the module no
longer claims to be an implementation of an algorithm it only borrows one curve from.

### Under the Hood

- **Toolchain moved to nightly-2026-09-20**, with the full gate re-run under it.
- **Clippy warnings halved** (21 → 11) by taking a newer lint's rewrites across 22 sites. Two of
  its suggestions were rejected for being less readable than the code they replaced.
- **Dependencies refreshed**: 8 Rust crates, 2 GUI packages. TypeScript 7 remains blocked upstream.
- **2,178 tests pass**, up from 2,167.
- The dreaming quality baseline — 0.90 compression at 1.000 recall — is **unchanged**, which is how
  we know the speedups above cost nothing in what Nanna actually remembers.

## Nanna v0.3.22-beta.31 — Every MCP Server, Both Directions

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

### What's New

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
and the connection check all carry it. (A token set in the `OLLAMA_API_KEY` environment variable
instead goes to whatever address is configured, and Settings says so.) When the server won't talk,
you're told why. It wants a token, or it refused the one you gave, or nothing Ollama-compatible
answers at that address (which usually means the path is missing). A token saved while Nanna runs
reaches chat and embeddings at once. A new address reaches chat at once; embeddings keep the address
and model they started with until Nanna restarts, and the log says so.

**Nanna says what it is doing while it starts.** Until the daemon answers, the window shows a
start-up screen instead of an app that cannot do anything yet: "Starting the daemon…", then "Still
starting · 1m 05s" with why a start can take that long. If the daemon stops while starting, the
screen says so in the daemon's own words (for example "Error: IPC port 127.0.0.1:5149 unavailable:
Address already in use") with its exit code, and a Restart button. There is always a way out:
**Open Nanna anyway** (or Esc; settings and logs work without the daemon, and Nanna keeps trying to
start it), **Show log** (the daemon's own start-up output, live), **Quit**, and **Update** when an
update is waiting, the way out of a daemon from another version. The screen appears only at launch;
losing the daemon later still shows in the status bar, as before.

### What's Fixed

**Nanna could fail to start at all.** If the embedding model was busy — the free OpenRouter model
answering "too many requests" was enough — opening Nanna showed *Starting* forever. At startup the
server asked that model one question to learn the shape of its vectors, and waited for an answer
with the same patience it uses for saving a memory in the background: up to eight minutes. The app
gave up after ninety seconds and stopped it, and every relaunch did the same. The startup question
is now asked once, without waiting; a busy model means starting on a provisional setting that
corrects itself as soon as the model answers. Checked against the real server with an embedding
endpoint that refuses everything: the previous release still had its door closed after 30 seconds,
this one opened in a quarter of a second.

**A slow start no longer becomes "never starts".** The app stopped a daemon that had not finished
starting after 90 seconds, and when its first attempt to connect failed it never tried again. So a
daemon that needed two minutes, or one started by hand afterwards, was never used. While the
daemon's process is alive the app now waits for it, with no deadline, and it keeps trying to
connect until one answers.

**Restart works on a start that hangs, and on a daemon that died while starting.** Nothing could
stop a daemon that was alive but never finished starting, short of quitting the app, and a daemon
that exited before its first connection was never started again. Restart now stops the stuck
process and starts a fresh one. Checked on this machine with a stand-in daemon that never opens its
port: after 34 s, Restart replaced its process and the count started over.

**A daemon that crashed while an MCP server it started was still running looked alive.** The app
counted the daemon as gone only once every process holding its output had closed it, and an MCP
server outlives the daemon that started it. The daemon's own exit decides now.

**Quitting stops a daemon that has stopped answering.** When the daemon did not answer the request
to shut down, the app sent the kill after it had already begun exiting, so the kill could be lost
and the daemon left running.

**The onboarding "Ready check" said the backend was ready when it was not.** It read a field the
status never had, so any answer passed, even with the daemon down or still starting, and the
version it named was the app's own. It now reads whether the daemon is connected, names the
daemon's version, and says what state the daemon is in when it is not ready.

**A stuck local Ollama could freeze part of the daemon.** Sizing a chat request's context window
asks the local Ollama for its models and waited with no time limit, on a thread the daemon needs
for other work. It now gives up after 3 seconds and waits on a thread of its own. Found as a test
that hung one run in four; it passed 20 of 20 after the fix.

**The Ollama setup step now checks.** Onboarding used to assume a local Ollama was running. It now
asks, and tells apart "Ollama isn't running" from "it's running but this model isn't pulled",
with the exact `ollama pull` to run. The model picker in Settings asks the same way. A server that
lists a model as `qwen` rather than `qwen:latest` no longer has it reported missing.

**Summaries use the Summarization models you chose in Settings, in order.** Shortening a long
conversation, condensing a large tool result, the notes a long task keeps as it goes, and picking
out memories to keep all use the Summarization Model Priority list (Settings → Models, Context
Summarization), first to last. Each model is reached the same way chat reaches it, with the same
keys, so an `ollama/` entry goes to the Ollama server set above, with its token. When a model
cannot be reached, or its answer is empty or unusable, the next one is tried; for condensing a
tool result, an answer that does not rate every sentence counts as unusable. The conversation is
cut to fit only when no model in the list answers, or when the list is empty, as the hint in
Settings says. Picking out memories falls back to the chat model instead, now also when every
listed model fails, so with a local summarizer down, memories are picked out by your chat model,
which may cost more.

Before this, summaries used their own `[llm].ollama_url`, which pointed at this computer unless you
changed it, and sent no token, so with chat on a remote server they were refused. Several of them
also tried only the first model in the list. In the app, a new server, token or key reaches
summaries at once, even in a chat that is already running (`nanna chat` in a terminal reads them
when it starts); a change to the list applies from the next message, and a background task keeps
the list it started with. Picking out memories still holds a reply up no longer than its one call
used to. Condensing one large tool result, and the pass that condenses older ones, each stop after
that same time in all (two minutes), so a server that takes a request and never answers holds the
reply up once, not once for every model or every result. A summary on your Ollama server waits for
a chat reply being generated there to finish, rather than cutting it off.

`[llm].ollama_url` is no longer read. An old config that has it still loads, and setting it through
the daemon is refused with a note saying what replaced it. If yours pointed at a different server
from the one in Settings → Models, the log says so when the config loads: set that server in
Settings if your summarization models are on it. `nanna doctor` now checks one Ollama server, for
chat, embedding and summary models together, and expects a model there exactly when chat, the
embedders or the summarizers would send it there. An entry typed without a provider now goes where
chat would send it: `qwen3` and `meta-llama/llama-3` to Anthropic, `gpt-oss:20b` to OpenAI. `nanna
doctor` warns about each and shows how to write it (`ollama/qwen3`).

**Memory consolidation skipped `anthropic/` and `openai/` summarization models.** Settings writes
these entries as `anthropic/<model>` and `openai/<model>`. Both were sent to Anthropic with the
prefix still on the name, so dreaming, consolidating on request, and the `day_dream` tool failed on
them every time and moved on. They now reach their own provider under the model's real name. With
the Summarization Model Priority list empty, all three now use your chat models in order; before,
two of them used only the first chat model. A model that answers with no text is now passed over
like one that fails. Before, it ended the search as if it had answered, so dreaming could replace a
group of memories with an empty one and `day_dream` reported an empty result without asking the
next model. The same routing applies to chat models. `nanna init` offers OpenRouter users
`anthropic/claude-sonnet-4` and `openai/gpt-4o`; in the app these now go to Anthropic or OpenAI
directly, where before they went to Anthropic with the prefix on and failed. To use them through
OpenRouter, write `openrouter/openai/gpt-4o`.

**Recovery acts on the Ollama server you set, and never kills a local one for a remote one.** When
chat against Ollama kept failing, Nanna unloaded the model and, as a last resort, restarted Ollama.
Both went to the server `OLLAMA_HOST` named, or to the default address on this computer, no matter
which server chat was using; only the restart refused a server elsewhere. So with chat on another
machine and `OLLAMA_HOST` unset, a failing proxy there meant Nanna unloaded a model from the Ollama
on your own computer and then killed it, and "waited for Ollama to come back" by checking that one.
Recovery now uses the address in Settings → Models. It waits for that server wherever it is,
sending its token, and stops waiting as soon as the server answers at all, or when you press Stop;
it unloads or restarts only a server on this computer. `OLLAMA_HOST` no longer changes which server
Nanna looks at: a local Ollama on another port is entered in Settings like any other address. Enter
a server on this computer as `localhost` or `127.0.0.1`. Nanna treats any other address as another
machine, this computer's own network name or LAN address included, so it will not restart that
server or size its context window to this computer's card.

**A remote Ollama's context window is no longer sized from this computer's graphics card.** Nanna
sizes a local model's context window to the video memory free on this machine, and it did the same
for a server on another machine, whose card it cannot see. A busy card here could shrink a remote
model's window to the minimum. A remote server now starts at 16,384 tokens, the size Nanna uses
whenever the card cannot be read, and still steps down if that server runs out of memory. The size
is kept per server, so changing the server in Settings while Nanna runs no longer carries the old
server's size over, and a model used on two servers keeps one for each. Each prompt is sized for
the window of the server it is sent to, and running out of memory on one server shrinks only that
server's window.

**A failed model lookup is no longer remembered for a week.** When Nanna could not get a model's
details from its provider (a refused token, a server that was down, a network error), it cached the
fallback as if it were the answer, for seven days, and sized every prompt for that model to a
32,000-token guess. Only real answers are cached now, each under the server that gave it, and
entries written by earlier versions are dropped. So an Ollama behind a proxy that refused the lookup
before this release gets its real window on the first request.

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

### For Developers

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

### What This Release Does Not Do

**The Telegram draft stream and stop button were not tried with a real bot.** There is no bot token
on the build host. They are tested against a scripted Bot API server, with request and update shapes
from the Bot API changelog and a mirror of its reference page.

**A chat through a real remote Ollama model was not run.** The token path was driven against a test
server that refuses every request without it, and the real remote server only answered the
connection check. Summaries on a remote server were checked the same way.

**Only Linux was driven end to end tonight.** The Windows and macOS builds are covered by the
release workflow, not by a run of the real app.

### Numbers

- **2,436 Rust tests pass, 0 fail**; clippy reports no warnings, with and without every feature.
  The live interop suite passes **11/11** against the real SDKs.
- **390 GUI unit tests pass.** In the browser suite **37 of 38** pass. The one failure, sending a
  chat and pressing Stop, also fails on the previous release and is tracked separately.
- The start-up screen was driven in the real app on Linux: a 45-second start, a start that fails,
  a start that hangs (then Restart), and the real daemon.
- Ways to reach an MCP server: **1 → 5**. Before, only stdio with the old handshake. Now stdio in
  both protocol eras, Streamable HTTP in both eras, and the 2024 HTTP+SSE transport.
