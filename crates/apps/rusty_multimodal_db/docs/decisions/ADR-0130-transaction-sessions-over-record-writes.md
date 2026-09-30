# ADR-0130: Transaction Sessions Over Record Writes

- Status: **Accepted (the owner: "go with recommendations", 2026-09-30) and phase 1 implemented** as option 1 (unify on `WriteOp`), `SERVER-001` v0.106.0 / `FR-119`, `SERVER-002` 0.22.0. Phases 2 and 3 are not built.
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0024`/`0025` (the session and the journal), `ADR-0060`/
  `0063` (`WriteBatch`, journaled), `ADR-0072` (MVCC), `ADR-0114` (journal
  with MVCC), `docs/FUTURE-GROWTH.md` item 2 ("a general transaction manager
  beyond this — multi-table atomicity and staged (not single-shot)
  transactions over inserts, links, replacements, and deletes").
- Supersedes/Superseded by: none yet. If accepted: wire change, protocol
  32 → 33.

## Context

What exists: a session (`Begin`/`Commit`/`Rollback`) stages **field updates**
(`TransactionOp`) and commits them through `apply_transaction`, crash-atomic
when journaled; `WriteBatch { atomic }` applies **record writes** (`WriteOp`:
insert, replace, replace-if, delete, link) as one isolated batch, crash-atomic
when journaled (journal format 2: a kind byte per entry, `0` for a
`TransactionOp` batch, `1` for a `WriteOp` batch). Inside a session every
record write is refused `SessionOpen`. The two never combine: a client that
must insert a record *and* bump a counter atomically cannot, and neither can
it stage record writes over several round trips.

## Options

1. **Unify on `WriteOp` (recommended).** Append `WriteOp::UpdateField { id,
   field, value }` (protocol 33). A session stages `Vec<WriteOp>` for both
   kinds; `Commit` is one atomic `write_batch`. `Request::Transaction` stays
   and becomes a batch of updates. One journal kind, one MVCC record path
   (`record_writes_into`), one validation path. Costs: every adapter's
   `prepare_write`/`apply_prepared` learn the update op (`Memory`, `Entity`,
   `Relation`, `Reminder` implement writes today); a journal reader of the
   old format still reads (an appended variant), an old server refuses the
   new one.
2. **A third journal kind for a mixed batch.** No new `WriteOp`; the session
   stages an ordered list of either and commit journals kind `2`. Smaller wire
   change (none), larger journal/replay/MVCC surface, two apply paths that
   must stay in step.
3. **Either/or sessions.** A session may stage updates *or* record writes,
   never both. No wire change beyond lifting the `SessionOpen` refusal; does
   not answer the insert-and-bump case.

Independent of the choice, three scope forks:

- **Read-your-writes** (`SESSION_READ_YOUR_WRITES`) over a staged insert,
  replace or delete: the overlay today is `(id, field) → value`. Phase 1
  proposes refusing record writes in a session that asked for read-your-writes
  or snapshot isolation (`Unsupported`), so the flags never lie; a later phase
  overlays whole records.
- **Multi-table atomicity** (one commit across `memory` and `entity`): needs
  one journal for both tables or a two-phase commit. Proposed as phase 3,
  after a single-table session works; `Use` inside a session stays refused.
- **Cascade** (`Delete` detaches edges in other tables, `DEL-FR-007`): in a
  session the cascade must run at commit, after the batch, not at stage time.

## Proposed phases

1. Sessions stage record writes and updates (option 1), single table, no RYW
   for record writes; commit is `write_batch(atomic)`; journaled where the
   table is.
2. Whole-record read-your-writes and snapshot reads for staged records.
3. Multi-table commit.

## Consequences (if accepted)

- Positive: the insert-and-update case, and multi-round-trip record writes,
  become atomic; one commit path instead of two.
- Negative: a protocol bump; every write-capable adapter changes; the journal
  and MVCC recording gain a case that must be crash-tested (a kill between a
  batch's slot writes, `tests/crash_safety.rs`-style) before it ships. This
  is correctness-critical code: independent inspection before merge.
- Not proposed: holding a lock across round trips (`ADR-0013`'s rejection
  stands), or interactive reads inside a commit.

## Phase 1 as built (2026-09-30)

`WriteOp::UpdateField` (5) and `WriteResult::Updated` (9), protocol 33;
`Memory`, `Entity` and `Relation` prepare and apply it in their atomic
pipeline (validated as `UpdateField` is, an absent record is a soft
`NotFound`), record it into MVCC and journal it with the batch (a mixed
batch is one journal entry and replays whole:
`a_mixed_batch_with_an_update_is_crash_atomic_via_the_journal`). A session at
33+ stages record writes and later updates in one ordered list; `Commit` is
one `write_batch(atomic)`. **What phase 1 does not do, stated plainly:** the
commit is `WriteBatch`'s atomic mode, so a soft outcome (`Duplicate`,
`NotFound`, `GuardFailed`) is a per-op result and the ops beside it still
applied; a strictly all-or-nothing commit needs a precondition pass under the
write lock (phase 2). A session with read-your-writes, snapshot isolation or
real MVCC refuses a record write `Unsupported`. A batch does not cascade a
`Delete` into other tables, nor check a cross-table `Link` endpoint
(`WriteBatch`'s own rule). Existing tests that pinned "never staged" now
connect at 32; the 33 behaviour has `tests/server_session_writes_integration.rs`.
