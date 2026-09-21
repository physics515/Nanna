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
