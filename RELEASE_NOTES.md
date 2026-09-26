# Nanna v0.3.30-beta.39 — The Board Starts Talking

The last release gave the board its foundations — members, real deadlines, a thread on every
card. This one makes the store **say what it is doing**. Every meaningful change to a card now
announces itself on the daemon's event bus, which is the wiring the Task Management Agent will
listen to when it starts placing work. Still nothing new to click: the board client comes later.

## What's Changed

**A card now announces every change to itself.** Nine kinds of announcement — created, assigned,
status changed, blocked, unblocked, posted, due, overdue, and a verdict when it is completed.
They are separate from the existing run events, which describe an agent working; these describe
what happened to a *card*.

**The announcements come from the store, not from whatever asked for the change.** That sounds
like an implementation detail and is the whole point. There are five different ways a card can
be changed — the task tools, the GUI, an agent's own task loop, the scheduled sweeps — and two
of them had no way to announce anything at all. Worse, some changes have no requester to speak
for them: finishing the last item under a heading closes the heading too, and cancelling a
branch closes everything under it. Announcing from the store is the only place that sees all of
it, so a parent can no longer quietly become done while only its child is reported.

**Blocked and unblocked are worked out, not guessed.** A card is blocked when something it is
waiting on is still open — that is computed fresh every time, never stored. So the announcement
is a genuine comparison of before and after, which means finishing one of two things a card is
waiting on correctly says *nothing*: partial progress is not an unblock, and being told
otherwise would send someone to work that is still stuck. Cancelling or deleting a dependency
releases a card exactly as finishing it does, and re-opening one blocks it again.

**Due and overdue are announced once, not every five minutes.** The sweep that notices them now
remembers what it has already said. Move a card's date and it can fall due again; extend a
deadline and it can go overdue against the new one; a repeating card that comes back around
starts fresh. A card is late the day *after* its deadline, not one minute into the day it is due.

### Fixes

**A re-assignment that changed nothing no longer looks like a hand-off.** Setting a card's
assignee to whoever already had it was recorded as a change. Harmless while it only made an
entry in the history; not harmless now that it would wake the agent that routes work.

**Editing a memory no longer leaves it findable by the words you deleted.** When a memory's text
was rewritten, the stored search vector describing the *old* text survived on disk, so the
memory kept matching searches for wording it no longer contained — and nothing about it looked
wrong. The vector is now dropped with the text and the memory is re-indexed; it is briefly
unfindable rather than wrongly findable. Part of this was fixed in August, but only in memory:
a restart loaded the stale copy straight back.

**Dependencies are current.** The usual sweep, plus the icon set moved to 1.48.

---

Updating from 0.3.28 or earlier? The
[0.3.29 release notes](https://github.com/basic-automation/Nanna/releases/tag/v0.3.29-beta.38) cover
the board's foundations — members, deadlines and card threads — and link back to 0.3.28's.
