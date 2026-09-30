# ADR-0131: Continuous Replication by Shipping a Change Log (Proposal)

- Status: **Proposed — design only, no code. Forks for the owner below.**
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0067` (`FetchSnapshot`, full snapshots), `ADR-0118`/`0123`
  (`replica_refresh`, a manually promoted cold standby), `ADR-0025` (the redo
  journal: checkpoint-then-truncate, no acknowledgment concept), `ADR-0125`
  (a change feed stays app-side for `rusty_tick`), `docs/FUTURE-GROWTH.md`
  ("Replication/high availability").
- Supersedes/Superseded by: none yet. If accepted: wire change, protocol
  32 → 33 (or later).

## Context

A standby today is a snapshot: `FetchSnapshot` copies every file of a table
(at most `MAX_SNAPSHOT_BYTES`, 8 MiB) under the write lock, and
`replica_refresh` repeats it on an interval into a verified directory. The
gap between refreshes is data loss on failover, and the 8 MiB cap bounds the
table. The crash journal cannot be tailed: it checkpoints and truncates, and
its entries only replay against an identical starting snapshot.

## Options

1. **A per-table change log with sequence numbers (recommended).** Every
   committed write is appended, inside the write lock it already holds, to a
   separate append-only log as `(seq, op)`: `WriteOp`s and `TransactionOp`s as
   committed, in commit order. A new request `FetchSince { after: u64, limit }`
   (gated `TokenClass::Replication`, like `FetchSnapshot`) answers `Changes {
   from, ops, head }`. A replica bootstraps from a snapshot that records the
   `seq` it was taken at, then tails, applying through its own `write_batch`.
   Retention is a byte/entry bound; a replica behind it re-bootstraps.
2. **Lower the refresh interval and lift the size cap.** No new protocol: a
   chunked `FetchSnapshot`. Cheap, but every refresh is a full copy, the loss
   window only shrinks to the interval, and it does not scale with the table.
3. **Leave it to the volume** (block-level or filesystem replication under the
   data directory). No engine work, no engine guarantees: a mid-write copy is
   not crash-consistent unless the volume layer is.

## Proposed phases (option 1)

1. The log, written on every write path (`insert`, `replace`, `replace_if`,
   `delete`, `link`, `update_field`, transactions, batches), with `seq` on the
   snapshot manifest and `FetchSince`. Crash-tested: a kill between the write
   and the log append must not lose a write the log claims, or the reverse.
2. `replica_refresh --follow`: bootstrap, then tail and apply; lag and
   `seq` metrics on both ends.
3. Promotion: a documented, manual step that stops tailing and opens the
   directory as a primary. **No automatic failover, no consensus, no write
   forwarding**: those stay named non-goals.

## Forks for the owner

- Option 1, 2 or 3.
- Whether the change log may share the crash journal's file (one `fsync` per
  commit, but a coupled format) or is its own (a second `fsync`, independent
  retention). Proposed: its own, so the journal's checkpoint-and-truncate
  discipline is untouched.
- Whether compaction (`Compact` rewrites slot files) is a logged change or a
  replica-side no-op. Proposed: not logged; a replica compacts on its own.

## Consequences (if accepted)

- Positive: a standby that trails by a bounded lag instead of an interval;
  no size cap on the table.
- Negative: every write path gains a log append inside its lock (a cost to
  measure against `ADR-0107`'s numbers), a second durable file to retain and
  bound, and a correctness surface (log/store agreement across a crash) that
  needs the crash-safety trials before it ships. Independent inspection before
  merge.
