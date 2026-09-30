# ADR-0131: Continuous Replication by Shipping a Change Log (Proposal)

- Status: **Accepted — phase 1 built (protocol 34), on the owner's "go with recommendations": option 1, its own log file, compaction not logged. Phase 2 (`replica_refresh --follow`) and phase 3 (documented manual promotion) are built too.**
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0067` (`FetchSnapshot`, full snapshots), `ADR-0118`/`0123`
  (`replica_refresh`, a manually promoted cold standby), `ADR-0025` (the redo
  journal: checkpoint-then-truncate, no acknowledgment concept), `ADR-0125`
  (a change feed stays app-side for `rusty_tick`), `docs/FUTURE-GROWTH.md`
  ("Replication/high availability").
- Supersedes/Superseded by: none yet. If accepted: wire change, protocol
  32 → 34 (built; 33 went to `ADR-0130`).

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

## Phase 1 as built (protocol 34)

- `src/server/changelog.rs`: `ChangeLog`, an append-only file per table
  (`<dir>/<table>.changes`, `SERVER_CHANGE_LOG_DIR`, needs `SERVER_DATA_DIR`).
  Each entry is one write batch of *effective* `WriteOp`s, fsynced under the
  log's lock, so log order is apply order. Header: magic, version, `epoch`,
  base `seq`, a clean-shutdown flag. An unclean open (kill, crash) bumps the
  epoch; a clean drain (`ADR-0127`) keeps it.
- `src/server/changelogged.rs`: `ChangeLogged`, a `ConnectionStore` decorator
  over any adapter; logs only writes that succeeded and changed something
  (`Duplicate`, `GuardFailed`, `AlreadyLinked`, `NotFound`, `Failed` are not
  logged). A `ReplaceIf` that applied is logged as a `Replace`.
- Wire (protocol 34): `Request::FetchSince { epoch, after, limit }` (39),
  `Response::Changes { epoch, first, head, entries }` (26),
  `Response::SnapshotAt { files, epoch, seq }` (27), `ErrorCode::Gone` (16).
  `FetchSnapshot` answers `SnapshotAt` when the table has a log (`Snapshot`
  below 34). `FetchSince` needs the Replication token.
- Retention: `SERVER_CHANGE_LOG_RETAIN_MB` drops the oldest half; a standby
  behind it, or on another epoch, is told `Gone` and re-bootstraps.
- Crash contract: a write applied but not logged leaves the log unclean, so
  the next open is a new epoch and the standby resyncs from a snapshot. The
  log never claims a write the store lacks.
- Verified: unit tests (append, reopen, epoch, retention, torn tail), an
  integration test where a standby restores a snapshot at a position, tails
  the log and equals the primary, and a real-binary test that a clean drain
  keeps the epoch while a kill starts a new one.

**Known limits, named:** the log mutex serialises a table's writers and
defeats journal group commit (measure before enabling under load);
`MAX_SNAPSHOT_BYTES` still caps the bootstrap snapshot; `detach_record`
cascades and `Compact` are not logged; the standby-side tail tool
(`replica_refresh --follow`) and promotion are phases 2 and 3, below.

## Phases 2 and 3 as built

- `replica_refresh --follow` (`examples/support/replica_refresh_lib.rs`):
  `refresh_at` installs a snapshot and writes its log position beside the
  files (`replica.position`, crash-safe); `follow` opens the directory as the
  domain's table and, per fetched batch, applies it and rewrites the
  position. Replay is non-atomic and at-least-once: every logged op is
  idempotent, so a crash between apply and position write converges. A
  `Gone` answer makes the tool take a fresh snapshot and follow that.
- Promotion is manual and documented in the tool: stop it, start a server with
  `SERVER_DATA_DIR` at the directory (it is a complete table; the position
  file is ignored). No failover, consensus or write forwarding, as before.
- Test: `follow_applies_the_primarys_later_writes_to_a_refreshed_directory`
  (a replace and a delete after the snapshot reach the standby; a stale epoch
  is `Resync`).
