# ADR-0131: Continuous Replication by Shipping a Change Log

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
defeats journal group commit (measured below: 5.6× at 16 update writers);
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

## Measured: what the log's lock costs (2026-09-30)

`examples/change_log_bench.rs`, release build, one 4-core ext4 host (`fsync`
bound: the numbers are this disk's, the ratios are the point), 3 s per cell,
`Memory` writes through `write_batch(atomic)` from N threads.

**Update** (in place, so with a journal the journal's `fsync` is the only one and
group commit can batch it), ops/s:

| threads | journal | journal + change log |
|---:|---:|---:|
| 1 | 5,593 | 3,130 |
| 2 | 7,252 | 3,082 |
| 4 | 8,736 | 3,042 |
| 8 | 13,056 | 2,534 |
| 16 | 17,207 | 3,074 |

The journal scales 3× from 1 to 16 writers (group commit); with the log it is
flat at about 3k. The log's mutex is held across the wrapped store's whole
apply, so writers reach the journal one at a time and never share an `fsync`:
**5.6× slower at 16 writers**, 1.8× at one (the log's own `fsync`). p50 latency
at 16 writers is lower with the log (297 µs against 893 µs) only because the
queue moved onto the mutex.

**Insert** (the insert log is `fsync`ed under the store's lock, so the baseline
does not scale either): plain 3.9–4.7k ops/s at every thread count, with the
log 2.3–2.7k (one more `fsync` per write, −40%); journaled 2.3–2.9k, with the
log 1.6–1.8k (−36% at 8 writers).

So the cost is one `fsync` per write everywhere, and on top of it the loss of
group commit wherever group commit was doing the work. A workload of single
writer or insert-dominated writes pays the first only; a many-writer,
update-heavy table pays both.

**If it matters** (not built): make the log's append cheap under its lock and
share its `fsync` the way the journal does (`GRP-FR-001`: append under the lock,
one `fsync` for the group, apply in turn order), or log from inside the journal
turn so the two share one `fsync`. Either is a change to the crash contract
(what is durable when a write is acknowledged), so it wants its own ADR.

**Follow-up:** `ADR-0134` proposes appending from inside the store's ordered section and
syncing in a group, and records a `Memory` spike (journaled updates at 16 writers: 10.1k with
it against 2.5k with the decorator, 14.9k with no log).
