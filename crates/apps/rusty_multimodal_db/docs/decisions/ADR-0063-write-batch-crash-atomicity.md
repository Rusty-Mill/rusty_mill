# ADR-0063: Crash-Atomic `WriteBatch` via the Existing Journal, Format Version 2

- Status: **Accepted as designed** (2026-09-14 — the owner picked option
  (a): extend the existing journal to format version 2 with a kind
  byte, reusing `CommitGroup` as-is; (b) a separate journal file and
  (c) decline both declined). Proposed and then implemented in the same
  session.
- Date: 2026-09-14
- Deciders: baileyrd
- Related: `docs/design/SERVER-WRITE-BATCH-CRASH-ATOMICITY-DESIGN.md` (the
  full design), `ADR-0060` (`Request::WriteBatch` — named this exact gap
  as its own declined-for-now follow-on: "needs `JOURNAL_FORMAT_VERSION`
  2 with idempotent redo"), `ADR-0025`/`ADR-0026` (`Request::Transaction`'s
  own crash-atomicity — the journal and group-commit discipline this
  round reuses rather than duplicates), `ADR-0062` (Definition of Done,
  item 4 — the round this ADR answers).
- Supersedes/Superseded by: none. Extends `src/server/journal.rs`'s
  on-disk format (`JOURNAL_FORMAT_VERSION` 1 → 2, one leading kind byte
  per entry; a version-1 journal is refused on open, not silently
  upgraded — matching this journal's own pre-existing strict-version
  behavior, unlike a long-lived blob) and `MemoryConnectionStore`/
  `EntityConnectionStore`/`RelationConnectionStore::write_batch`'s
  atomic path only. No wire or protocol-version change —
  `Request::WriteBatch`'s bytes are unchanged; this is entirely a
  server-side durability improvement a client cannot observe except as
  a stronger guarantee.

## Context

`ADR-0060` shipped `Request::WriteBatch`'s atomic mode as precondition-
and isolation-atomic only: every op is validated and applied under one
`with_exclusive` critical section, so a *concurrent reader* never sees a
partial batch — but each op's own storage write (the insert log, the
edge log, or a tombstone) still `fsync`s individually, so a process
crash between op 2 and op 3 of a 5-op atomic batch leaves ops 1–2
durably applied and 3–5 not, violating "all or nothing" across a crash.
`ADR-0060`'s own Consequences named this precisely and deferred it:
"needs `JOURNAL_FORMAT_VERSION` 2 with idempotent redo... a larger
round."

Two things already exist that make this bounded rather than a
from-scratch build:

1. **The batch journal and group-commit discipline already exist and
   are already wired onto exactly the three domains `WriteBatch`'s
   atomic mode targets.** `MemoryConnectionStore`, `EntityConnectionStore`,
   and `RelationConnectionStore` each already carry a `journal:
   Option<CommitGroup>` field and a `with_journal` constructor
   (`src/server/{memory,entity,relation}.rs`, mirroring
   `DogConnectionStore::with_journal` exactly) — built for
   `Request::Transaction`'s field-update batches (`ADR-0025`/`0026`) and
   already proven: append-then-`fsync`-then-apply, replay-on-open,
   checkpoint-by-size, leader/follower group commit so concurrent
   batches share one `fsync`. **`write_batch`'s atomic path does not use
   it at all today** — it calls `self.store.with_exclusive(...)`
   directly, bypassing `self.journal` completely even when the adapter
   was constructed with journaling on.
2. **The "stale guard on redo" problem this format needs to solve has
   an existing, minimal answer.** `ReplaceIf`'s guard is a `Predicate`
   evaluated against live state — replaying it naively on crash recovery
   would re-evaluate against the *post-crash* state, which may no longer
   match what the original apply saw, breaking idempotency. But
   `write_batch`'s atomic path already has a **prepare** phase
   (`prepare_write`/`PreparedWrite`) that resolves every op's shape
   before entering the exclusive section. Moving guard evaluation into
   that same critical section, *before* the journal write, and writing
   only the now-unconditional, already-decided operations to the
   journal — never a live `ReplaceIf` with its guard — makes every
   journaled entry a plain idempotent overwrite/insert/delete/link,
   exactly the property `BatchJournal`'s own module doc already relies
   on for `TransactionOp`.

`BatchJournal`'s on-disk format is hard-typed to `Vec<TransactionOp>`
today (`journal.rs`'s `open`/`append_unsynced` both name it explicitly),
so it cannot carry a `WriteOp` batch without a format change — the
literal thing `ADR-0060` named.

## Decision

Propose evolving the batch journal to format version 2, mirroring
`src/generic/insert_log.rs`'s own established precedent for exactly this
situation (`LOG_VERSION_1` → `LOG_VERSION_2` added a leading kind byte,
old files still read, rewritten on next append): each journal entry
gains a **kind byte** — `0` a `TransactionOp` batch (today's only
shape, unchanged), `1` a **decided write batch** — a `Vec<DecidedWriteOp>`
where every `ReplaceIf` has already been resolved to a plain `Replace`
(its guard evaluated once, under the same exclusive section the journal
write happens inside, before the `fsync`). A version-1 journal (no kind
byte, every entry a `TransactionOp` batch) still reads exactly as today;
`Vec<TransactionOp>`'s own callers (`Dog`/`Order`/`Employee`) are
unaffected — this round touches only `Memory`/`Entity`/`Relation`'s
already-existing journal wiring.

`write_batch`'s atomic path, when `self.journal.is_some()`: prepare
every op (unchanged); enter the exclusive section; evaluate every
`ReplaceIf` guard against live state, converting it to a concrete
`Replace` or aborting the whole batch with nothing written (unchanged
behavior — this already happens today, just not journaled); append the
now fully-decided op list to the journal and `fsync` via `CommitGroup`
(the *new* step); apply every decided op (unchanged). A crash before
the `fsync` returns leaves nothing durable — the batch never happened.
A crash after leaves the journal holding the decided batch, replayed
idempotently on the next `with_journal` open exactly as `TransactionOp`
batches already are.

The fork, held for the owner:

- **(a) As scoped above — extend the existing journal to format
  version 2 with a kind byte, reuse `CommitGroup` as-is (recommended).**
  Smallest real change: no new journal file, no new fsync-coordination
  primitive, matches `insert_log.rs`'s own established format-evolution
  pattern exactly. `Memory`/`Entity`/`Relation`'s atomic `write_batch`
  gains crash-atomicity only when the adapter is already constructed
  with `with_journal` — pipelined batches, and atomic batches on a
  non-journaled adapter, are unchanged.
- **(b) A wholly separate `WriteBatchJournal` file/type, duplicating
  `BatchJournal`'s discipline for `WriteOp` instead of sharing it.**
  Zero risk to the already-`Verified` `TransactionOp` journal path (no
  format-version bump, no kind-byte parsing added to a proven reader) —
  but a second file per adapter, a second `fsync` discipline to
  maintain, and real duplication of code this crate has otherwise
  consistently shared (`record_blob.rs`/`edge_blob.rs`'s own precedent).
- **(c) Decline.** `WriteBatch`'s atomic mode stays precondition- and
  isolation-atomic only, as documented today and in `ADR-0060`. The hub
  push already tolerates this (per-record last-writer-wins, `ReplaceIf`
  already atomic per record) — a legitimate answer if the owner judges
  the gap not worth closing yet.

## Consequences

- Positive (a): closes `ADR-0060`'s own named gap with the smallest
  real change available — one kind byte, one new entry variant, zero
  new files, zero new fsync machinery. `Memory`/`Entity`/`Relation` are
  the only adapters touched; `Dog`/`Order`/`Employee`'s `Request::
  Transaction` journal path is provably unaffected (version-1 entries,
  their only entry kind, are unchanged bytes).
- Named, not hidden: crash-atomicity here means "a crash between the
  journal `fsync` and full apply replays cleanly" — it does **not**
  cover a storage I/O error *during* apply (a slot write itself
  failing), which still needs the "storage rollback primitive"
  `ADR-0060` separately named as a larger, undecided round. This ADR
  closes the crash half of the gap, not the I/O-failure half.
  Explicitly not claimed as full ACID atomicity.
  - Named, not hidden: the guarantee is conditional on the adapter
  being started with `with_journal` — a server run without it (the
  default in some binaries) gets exactly today's behavior, unchanged.
  Whether every `memory_server`/`entity_server`/`relation`-serving
  binary *should* default to journaled is a separate, smaller decision
  this ADR does not make.

## Acceptance and implementation

- 2026-09-14: proposed, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch: `src/server/journal.rs` (`JOURNAL_FORMAT_VERSION` 2, the kind
  byte, `JournalEntry`/`JournaledBatch`, `CommitGroup::commit_write`),
  `src/server/{memory,entity,relation}.rs` (`write_batch`'s journaled
  atomic branch, `replay_write_batch`, a `schema()` associated function
  extracted from `describe()` so replay has a schema before `Self`
  exists). `Dog`/`Order`/`Employee`/`Reminder`'s replay loops gained a
  defensive `JournalError::Format` arm for a `Write` entry they should
  never see (they never call `commit_write`).
- **A real, simpler mechanism than the Decision's own sketch, found
  during implementation and corrected before writing any code, not
  after**: the Decision above describes resolving every `ReplaceIf`
  guard to a concrete `Replace` *before* the journal write (a new
  `DecidedWriteOp` type). Implementing it ran into a real conflict:
  `CommitGroup`'s own group-commit discipline (`ADR-0026`) requires the
  journal append to happen *before* the exclusive section that would be
  needed to evaluate a guard against live state — the two steps can't
  share one lock scope without giving up the group-commit pipelining
  the whole design exists for. Resolved by journaling the raw,
  already-`prepare_write`-validated `WriteOp` list unchanged (no new
  type) and replaying it by re-running the identical `prepare_write`/
  `apply_prepared` pair, in order, under the replay's own exclusive
  section — safe specifically because replay only ever runs cold-start,
  single-threaded, from the exact pre-crash state: a crash before apply
  means the guard's original live-state input is untouched, so replay
  re-evaluates it against the identical state and reconstructs the
  identical decision, never a stale one. Simpler than the original
  sketch (no new type, no guard-resolution step) and the same guarantee.
- Proven: `journal.rs`'s existing 8 `TransactionOp`-path tests pass
  unmodified after genericizing the entry type (kind `0` bytes
  unchanged); a new kind-`1`/kind-`0` round-trip test in the same
  journal, in order; a real crash-injection test
  (`write_batch_atomic_is_crash_atomic_via_the_journal`,
  `src/server/memory.rs`) — a genuinely committed atomic batch's durable
  journal, replayed onto a fresh copy of the pre-batch store files (the
  state a crash right after the `fsync` but before any apply would
  leave behind), both an `Insert` and a `Replace` landing correctly and
  the journal checkpointed away after. `SERVER-001` v0.52.0 / FR-062.
  `cargo fmt --all -- --check` clean; `cargo clippy --all-features -- -D
  warnings` clean; `cargo test --all-features --no-fail-fast` 523 lib
  tests + every integration target green except the pre-existing,
  unrelated `server_python_client` failure (`python3` missing from this
  session's `PATH`).
