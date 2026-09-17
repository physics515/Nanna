# Nanna v0.3.20-beta.29 — A Quarter of the Toolbox Was Never Handed Over

Nanna ships 44 tools. On a default install you were getting 28.

Not because the missing ones were broken — because the daemon-side service each one calls was
registered nowhere, and a skill whose service is absent is **withheld from the model** rather than
offered and left to fail. That withholding is correct, and it was also silent: one `info` line per
skill, among a few hundred at boot. The README said "44 filesystem tools — file, shell, web, vision,
OCR, PDF, memory, and scheduling", and three of those categories did not exist at runtime.

This release wires 13 of the 16 back, and makes the remaining gap impossible to acquire again.

## What's Fixed

**16 of 44 bundled skills were withheld at every boot. Now 3 are.** The count came out of a new audit
that cross-checks every skill's declared `requires: [...]` against every service the daemon can
register, parsed with the loader's own extractor rather than a bespoke regex — so a declaration the
test cannot see is one the daemon cannot see either. Any remaining gap has to be named with its
reason, and a second test fails when an entry goes **stale in either direction**: the service got
implemented, or no skill asks for it any more. It caught its own author twice during this run.

The daemon also **announces the gap at boot** now, once, naming the withheld skills *and* the
services blocking them — not just a count, because a count tells you something is missing without
telling you what to configure.

**Nanna can write its own tools, and use them in the same breath.** `create_tool` authors a new
JS/TS tool and registers it live — callable immediately, no restart. `edit_tool` changes one,
refusing an edit that matches zero or several places rather than guessing which you meant, and
re-registering on success. `list_user_tools` shows what has been authored. All three were withheld
before this release; none of their services existed.

**It can see, read, listen, browse and look at your screen** — where you have the pieces for it.
`analyze_image`, `describe_image` and `ocr` need a vision-capable model named in
`[memory] ocr_model_priority`. `text_to_speech` and `transcribe` need an OpenAI key. The four
`browser_*` tools need Chromium or Chrome installed. `screenshot` needs a desktop capture tool and a
display. Where a piece is absent the tools stay withheld and the boot line tells you exactly which
setting or program would turn them on.

**Scanned PDFs are readable, and an empty answer now tells you why it is empty.** `read_pdf` falls
back to model OCR for image-only pages. More usefully, it distinguishes four outcomes that all used
to look like an empty string: no OCR model is configured, OCR ran and recovered text, OCR ran and
the images carried none, or the document has no embedded images at all and simply is not a scan.

**Things that made a file now tell you where it is.** Generated speech, page screenshots and desktop
captures used to report a byte count and drop the bytes — an API call or a browser launch spent
producing something nobody could open. All three write a file and return its path.

**Four browser tools were advertised with five contract mismatches.** `browser_evaluate` sent
`expression` where the daemon read `script`, so every call would have answered `Missing script`.
`browser_extract` offered an `attribute` parameter with no implementation behind it. `browser_action`
advertised `scroll` and `navigate`, neither of which existed. All five are closed, and the browser
services are the one group verified end to end here: a page served on loopback, a real Chromium
launched, and the extraction, attribute, expression, scroll and PNG all asserted.

**A tool that forgot its `permissions.json` was handed the whole filesystem.** The default written on
a tool author's behalf was `read: ["*"], write: ["*"]` — which is how `edit_tool`, the tool whose job
is rewriting other tools' source, ran unscoped while its own sibling was confined to home. It is now
home-scoped, derived from a census of the 44 bundled skills rather than picked, and every such grant
is announced naming the tool. Existing installs are untouched: the grant is persisted, so a directory
that already has one keeps it.

**`~` in a permission scope denied everything instead of meaning home.** The doc comments promised
`~` support; the check compared against a literal `~` path component, which matches nothing. Only
reachable by building permissions programmatically, which is exactly why it would have waited for the
next caller. Now implemented.

**One connection could see every conversation.** `Subscribe` was recorded and ignored — every IPC
client was forwarded the whole event stream, so an attached client saw other sessions' message
deltas, tool calls and errors on the wire. Narrowing is now real and opt-in, costing a single atomic
load until someone uses it.

**Anthropic OAuth identified itself as a Claude Code release from a year ago.** Subscription sign-in
presents Nanna as a Claude Code client, and the version it reported was pinned at `2.1.2`. It now
reports `2.1.273`, matching the current CLI.

**`[general] data_dir` now does something.** The setting parsed, validated and round-tripped while
changing nothing — the daemon always used the platform default. It is honoured now, the daemon says
at boot when a configured location is in use, and `--data-dir` still wins over it. Pointing it at a
new folder does **not** move an existing store; the daemon opens whatever is there. Daemon logs also
follow `--data-dir` and the configured location instead of splitting off to the default one.

## What This Release Does Not Do

**Reminders still do not work,** and the three `schedule.*` tools stay withheld. The blocker is not
the missing service bridge it looked like: the scheduler's one-shot timers are fully live, but the
output of a scheduled run reaches the log and nothing else — `target_session`, the field that looks
like the delivery route, is set by no caller and read by no code. Wiring the services on top of that
would have the model promise you a reminder that fires into a log file.

**The vision, speech and screenshot paths are wired but not exercised end to end here.** This host
has no cloud credential, and running a screen capture to prove plumbing would have photographed the
operator's desktop unasked. Everything up to the request is tested; the request itself is not.

**Browsing costs 3.8 MB.** Enabling the browser stack adds 9 crates and grows the daemon binary by
5.7% (66,483,104 → 70,302,752 bytes). That is the one change here that makes the shipped binary
bigger.

## Numbers

- **1,999 tests pass, 0 fail** across 79 binaries; clippy reports 0 errors.
- **16 → 3** bundled skills withheld at boot.
- **6 dependencies** removed that nothing imported; `backoff`, `instant` and `nix` leave the lockfile.
- Release daemon: **70,325,024 bytes**.
