# Nanna v0.3.15-beta.24 — The Log Underneath the Facts

Memory has had one layer for a long time: facts, FSRS-weighted, never expiring. This release adds the
layer underneath it — the raw episodic stream of what actually happened, on a wall-clock axis — and
retires a dependency pin that has been in the tree since July.

## What's New

### An append-only episodic timeline

`memories` answers *what is true*. It cannot answer *what happened, and when*, because a fact is the
residue of many episodes and has no single timestamp. `MIGRATION_014` adds `memory_events`: messages,
tool calls, recalls and outcomes as they occur, each stamped in Unix milliseconds, with a nullable
embedding and a normalized salience. `nanna-storage` gains an append-only `MemoryEventRepository`, and
a new **`nanna-timeline`** crate owns the policy — the closed `EventKind` set, the caps, and the
`Timeline` facade.

Four properties are enforced rather than assumed, and each exists for a phase that comes later:

- **Integer timestamps.** Every consumer of this table is arithmetic — resample into a series,
  decimate a window, detect a peak. Text timestamps would mean parsing on every sample.
- **Half-open windows `[start, end)`.** Adjacent windows tile the axis exactly once, so a resampler
  stepping bucket to bucket cannot double-count an event that lands on a boundary.
- **Truncation is recorded, not just performed.** `content_len_chars` stores the length *before* the
  8192-character cap, so a shortened episode is detectable by comparison rather than by a flag nobody
  sets. The cut is character-wise, never byte-wise — there is a test that caps 8202 em dashes,
  because byte-slicing arbitrary text at a fixed offset is the exact mistake that has panicked this
  codebase twice.
- **Append is idempotent on `event_id`.** A channel retrying a message cannot inflate the timeline.

Nothing writes to this log yet; wiring the producers is a separate change.

### A migration bug caught before it shipped, and closed for good

`Storage::migrate` executes a migration with `sql.split(';')` — a splitter that does not know what a
comment is. A semicolon inside a `--` comment therefore cuts the statement *around* it in half and
hands the database two fragments. Migration 014 hit this while being written:

```sql
embedding BLOB,               -- f32 little-endian; NULL until embedded
```

That comment split the `CREATE TABLE` at the preceding comma. The failure would have been invisible
until someone opened a **fresh** database, because every existing install already has the table —
so it would have shipped as "works for me" and broken only new installs.

Three unit tests now assert the property across all 14 migrations: no semicolon inside a comment, no
comment-only chunk reaching `conn.execute`, and unique names in applied order. The 13 pre-existing
migrations were audited and are clean, so this is a trap closed before it was ever sprung.

### The boa git pin is gone

`boa_engine`/`boa_runtime` have been pinned to a git revision of boa `main` since 2026-07-10, because
the then-current release (0.21.1) held `icu ~2.0` against a tree on icu 2.2. **`boa_engine 0.22.0`
shipped on 2026-08-28** — newer than the pinned revision, so returning to crates.io moves forward, not
back. It requires `icu ~2.3` and pulls the whole tree there: all 15 `icu*` crates resolve to a single
2.3.x, with no split anywhere.

`boa_runtime` was dropped at the same time. It was an optional dependency that **no source file has
ever referenced** — the same dead-weight class as the `swc_core` removal before it.

### Dependency freshness

`lopdf 0.44 → 0.45`, plus the routine sweep (`bon 3.10.1`, `serde_with 3.23.0`) and on the frontend
`marked 18.0.12` and `@lucide/vue 1.43.0`. Both documented lockfile landmines fired exactly as
recorded and were re-pinned: `libc` back to 0.2.186 (RustPython 0.5.0 still needs the ceiling) and
`malachite-bigint` back to 0.9.2. The guard tests reported each in 0.00s rather than twenty minutes
into a release build, which is what they were written for.

One trap worth naming: `cargo upgrade` reported `lopdf → 0.42.0` for one crate and `→ 0.45.0` for
another **in the same table** — a stale registry-index read, not two different requirements. Never
take a `cargo-upgrade` row without checking the crate's real version list.

## Verified

`cargo check --workspace --exclude nanna-gui --all-targets` clean ·
`cargo test --workspace --exclude nanna-gui` **1722 passed / 0 failed / 12 ignored** across 69 test
binaries, doctests included · `cargo clippy` **0 errors**, and the new `nanna-timeline` crate is
warning-clean under `pedantic` + `nursery` · rustfmt clean on every new file · frontend
`vue-tsc --noEmit` clean, **238/238** vitest, `pnpm build` green with 4 routes prerendered.

The 16 new timeline tests include `migration_014_creates_a_usable_event_log`, which opens a **fresh**
database — the exact case the comment-splitting bug above would have broken, and the one an existing
install can never exercise.

## Known limits

- **The Tauri GUI is unverified, and on this host it cannot be verified.** `webkit2gtk-4.1` is
  installed but Arch's package ships no `WebKitWebDriver` binary at all, and Arch has no
  `webkit2gtk-driver` package — so `tauri-driver` cannot start. The fix is a WebKitGTK source build,
  not a package install. Filed with the evidence.
- **Nothing produces timeline events yet.** The store, its bounds and its tests are real; the
  producers are not wired.
- The toolchain pin was not re-tested against the current nightly this cycle.
