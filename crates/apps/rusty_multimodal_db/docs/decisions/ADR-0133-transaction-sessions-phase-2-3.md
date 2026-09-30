# ADR-0133: Transaction Sessions, Phases 2 and 3 (Proposal)

- Status: **Accepted — options 1A and 2 built (protocol 35), on the owner's "go with recommendations"; multi-table commit deferred as recommended.**
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0130` (phase 1), `ADR-0063` (atomic `WriteBatch`), `ADR-0072`
  (MVCC), `ADR-0025` (the redo journal), `docs/FUTURE-GROWTH.md` ("Transactions").
- If accepted: a wire change, protocol 35 (a `BeginWith` flag bit, no new variant).

## Context

Phase 1 stages record writes and commits one atomic, journaled `write_batch`.
Three things it does not do:

1. **Strict commit.** A soft outcome (`Duplicate`, `NotFound`, `GuardFailed`,
   `AlreadyLinked`) is a per-op result and the other ops still apply.
2. **Whole-record read-your-writes** for staged records.
3. **Multi-table commit.**

## Options

### 1. Strict commit

- **A. A check pass under the write lock (recommended).** Before applying, each
  adapter walks the prepared ops against the locked store plus an in-batch
  overlay (ids inserted or deleted earlier in the same batch) and returns the
  first soft outcome as `TransactionFailed { index, code }`, applying nothing.
  Opt-in through a new `BeginWith` flag, `SESSION_STRICT_COMMIT`, so the
  default `Commit` keeps phase 1's meaning. `ReplaceIf` guards are evaluated
  against the overlay's value. One shared helper with three thin adapter hooks.
- B. Client-side pre-read then commit. Not atomic against another writer; the
  check and the apply are not under one lock. Rejected as not what "all or
  nothing" promises.
- C. Leave as is; document that soft outcomes are results. The status quo.

Cost of A: the overlay must mirror `apply_prepared` exactly for every op kind,
in three adapters, or the check and the apply disagree. That is
correctness-critical and needs a property test (random op sequences: the check
predicts the apply's soft outcomes) plus independent inspection before merge.

### 2. Whole-record read-your-writes

Today a session refuses record writes when read-your-writes, snapshot
isolation or MVCC is on (`Unsupported`), and read-your-writes only covers
`UpdateField`. Lifting it means `GetById` in a session answers the record as
the staged list would leave it. Proposed: only in a strict-commit session, by
replaying the staged list over the stored record (the same overlay as 1A), so
there is one definition of "the record as staged". Without an overlay (1C), do
not build it.

### 3. Multi-table commit

One `Commit` across two tables needs either a shared journal or a two-phase
step; today each table has its own journal and lock. Proposed: **do not build
until a consumer needs a cross-table atomic write.** `rusty_tick`'s writes are
one table each; `Link` across tables is validated but not atomic with the
insert. If needed: one journal file shared by the tables of a server, locks
taken in table-name order, one commit record naming every table. That is a new
journal format (version bump, crash trials) and should be its own ADR.

## Forks for the owner

- Strict commit: A, B, or C. Recommended: A.
- Whole-record RYW: build with 1A's overlay, or not at all. Recommended: with
  1A, in strict sessions only.
- Multi-table: defer (recommended) or design the shared journal now.

## Consequences (if 1A and 2 accepted)

- Positive: an insert-then-update or a guarded replace batch is all or
  nothing; a client can read what it staged.
- Negative: protocol 35; an overlay that must track `apply_prepared` in three
  adapters; a property test and crash trials before it ships. Independent
  inspection before merge.
- Not proposed: holding a lock across round trips (`ADR-0013`).

## As built (protocol 35)

- `src/server/strict.rs`: `Overlay` and `first_soft_failure`, pure over two read
  closures (a record getter and an edge test). Each adapter runs it inside its
  exclusive section (`strict_refusal`), so the answer cannot go stale before
  the apply. `ConnectionStore::write_batch_strict` / `strict_commit_supported`;
  `ChangeLogged` forwards and logs.
- `BeginWith` flag 16, `SESSION_STRICT_COMMIT`. It stands alone; a strict
  session routes every staged op, updates included, into the ordered list, and
  its `GetById` is `strict::overlay_get` over that list (option 2 on option 1A's
  overlay, one definition of "the record as staged").
- **The journal.** A batch is journaled before it is applied, so a refused
  strict batch leaves its entry behind. Replay of a plain entry would apply it
  loosely, so a strict batch is journaled under its own entry kind
  (`KIND_STRICT_WRITE`, format version unchanged); replay re-runs the check on
  the same pre-state, gets the same refusal, and skips it. The cost is that an
  older build refuses a journal holding one.
- **The check earned its property test.** Random op lists over six ids, through
  the check on one store and the real apply on another, found two divergences
  in the first draft (a self-loop link is a *hard* `Malformed` at apply, and
  the missing-endpoint check comes first); both are fixed and pinned.
- Not built: multi-table commit; strict commit for `Dog`/`Order`/`Employee`
  (`Unsupported`).
