# Nanna v0.3.29-beta.38 — The Board's Foundations

Nanna is becoming a task board with agents on it rather than a chat with a task store behind it.
This release lays the store that board sits on: members, real deadlines, and a card's thread.
None of it is visible yet — the board client comes later — but everything below is live in the
database and reachable from the task tools.

## What's Changed

**The human and every agent are now one kind of thing.** A new `members` table holds the person
using Nanna, every agent, and each board's Task Management Agent under a single id space, so
"assign this to an agent" and "assign this to a person" stop being different operations. A fresh
install seeds you and a router; every workspace you have already registered gets its own.

**A card's assignee is now a real reference.** It used to be a free-text label that could name
anybody or nobody. It now has to name a member, checked on every write, so a card cannot be
assigned into the void.

**A date and a deadline are different things.** Until now a card had one date doing both jobs,
which meant a card you had deliberately put off looked exactly like a card you were late on.
A *date* now defers a card — it stays out of the way until that day arrives. A *deadline* bounds
it, and being overdue is measured against the deadline alone. You can filter on either
(`deadline before:`, `deadline after:`, `no deadline`), and a deadline that falls before the
date it is deferred to is refused, since that card could never be worked.

**A card has a thread.** Notes on a card were an agent scratchpad signed with a free-text name.
They are now posts: each names the member who wrote it and says what it is — a comment, a
progress line, a question, or a verdict. Nothing can edit a post after the fact; the thread is
the record.

**Dependencies are current.** The usual sweep, plus the markdown renderer moved to 18.0.14.

---

Updating from 0.3.27 or earlier? The
[0.3.28 release notes](https://github.com/physics515/Nanna/releases/tag/v0.3.28-beta.37) cover
the strict-lint pass and exact numeric conversions, and link to 0.3.27's.
