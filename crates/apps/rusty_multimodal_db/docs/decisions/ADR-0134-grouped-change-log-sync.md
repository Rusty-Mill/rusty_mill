# ADR-0134: Group the Change Log's `fsync` and Append From Inside the Store's Ordered Section (Proposal, Memory Spiked)

- Status: **Proposed. A `Memory`-only spike is built and measured (below);
  `Entity`, `Relation`, the non-atomic write paths and the server wiring are
  not. Forks for the owner at the end.**
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0131` (the change log and its measured cost), `ADR-0025`/
  `ADR-0026` (the redo journal and its group commit), `ADR-0063` (atomic
  `WriteBatch`), `ADR-0072` (the ordered exclusive section MVCC already records
  into).
- Supersedes/Superseded by: none. If accepted it changes how `ADR-0131`'s log
  is written, not what it holds or the wire.

## Context

`ADR-0131`'s log costs one `fsync` per write and, on a journaled table,
removes group commit: 17.2k against 3.1k update ops/s at 16 writers. The cause
is structural. `ChangeLogged` sits outside the adapter and holds the log's
mutex across the wrapped store's whole apply, to make the log's order the apply
order. The journal's `fsync` happens inside that apply, so writers reach the
journal one at a time and never share a sync.

## The two ideas, and why they are one

1. *Share the log's `fsync` across writers the way the journal does.*
2. *Log from inside the journal's turn, so the two share one `fsync`.*

The second cannot be literal: the journal and the log are two files, and one
`fsync` covers one file. Sharing it would mean putting change-log entries into
the journal file, merging two formats and breaking the journal's
checkpoint-then-truncate discipline. What can be shared is each `fsync` among
many writers, so the cost is per group instead of per write.

The first is not enough alone. Taking only the log's `fsync` out of the lock
still leaves writers arriving one at a time, because the log's mutex is still
held across the journal's. The log's *append* has to move inside the store's
own ordered section, which already applies writes in order (the journal's turn
gate on a journaled table, the exclusive lock on any). So the two ideas are one
mechanism: **append in order from inside the ordered section (2's placement),
sync in a group after it (1's sharing).**

## Decision (proposed)

- `ChangeLog::append_deferred(ops) -> ticket`: append one entry without
  `fsync`, under the log's own short lock, returning the head it made.
  `ChangeLog::sync_through(ticket)`: make every entry through `ticket`
  durable. One caller leads an `fsync` for everything appended so far, the
  others wait on a condition variable and return when a leader's sync covers
  their ticket (the journal's `GRP-FR-002` shape; a plain mutex handed the
  lock back to the leader and starved the rest, which the first spike showed).
- A `ChangeSink` on the adapters, called inside the ordered section after the
  apply with the effective ops (recorded beside `mvcc_record_write_ops`),
  and the group sync after the section is released and before the
  acknowledgement.
- `ChangeLogged` stops wrapping writes for adapters that carry the sink; it
  remains for any adapter that does not.

## The crash contract is unchanged

A write is acknowledged only after its log entry is durable, as now. A crash
after the apply and before the log's sync leaves a write the log lacks; the log
was not shut down cleanly, so the next open is a new epoch and a standby
resyncs (`ADR-0131`). The window is the same shape, not larger: today it is
apply-to-append; here it is apply-to-group-sync.

## The spike: what was built

`Memory`'s atomic `write_batch` only: the sink call in the apply closure, the
sync after the journal or exclusive section, `with_change_log`. The non-atomic
path, `insert_record` and the other single-shot writes, `apply_transaction`,
`Entity` and `Relation` are not wired. One new test:
`a_grouped_change_log_keeps_apply_order_under_concurrent_writers` (8 threads,
800 journaled updates of one record: one entry per batch, the store ends at the
last logged value).

## Measured

`examples/change_log_bench.rs`, release build, the same 4-core ext4 host as
`ADR-0131`, 3 s per cell, ops/s. Absolute numbers move between runs (the
journal-only update row was 17.2k, 14.9k and 12.1k in three runs); the ratios
are the result. `sink` rows are the spike.

**Update**, journaled (group commit can share the journal's `fsync`):

| threads | journal | journal + `ChangeLogged` | journal + sink |
|---:|---:|---:|---:|
| 1 | 5,157 | 2,613 | 2,582 |
| 4 | 7,358 | 2,536 | 4,063 |
| 8 | 11,281 | 2,689 | 7,019 |
| 16 | 14,884 | 2,522 | 10,100 |

The decorator is flat; the sink scales 3.9× from 1 to 16 writers and reaches
68% of journal-only at 16 (it still pays the log's own `fsync`, now per group).
At one writer the sink is no better than the decorator: one `fsync` more per
write is a floor no grouping removes.

**Update**, not journaled (the log's `fsync` is the only one):

| threads | `ChangeLogged` | sink |
|---:|---:|---:|
| 1 | 5,173 | 4,823 |
| 16 | 4,263 | 18,316 |

**Insert**, journaled (the insert log's `fsync` under the store's lock bounds
the baseline at about 2.2k whatever the thread count): journal 2.2k, with
`ChangeLogged` 1.4–1.5k, with the sink 2.3–2.4k at 2 or more writers — the
log's cost drops to within noise of nothing.

## Options

1. **Build it as proposed (recommended)** if a table with many concurrent
   writers and a standby is a real deployment. The change is in the adapters'
   write paths (about nine each), so it wants the crash trials below first.
2. **Leave `ADR-0131` as measured.** Right if a replicated table is
   single-writer or insert-dominated: it then pays only the extra `fsync`.
3. **Log inside the journal file** (the literal second idea). Rejected above.

## Before it ships (if 1)

- Wire `Entity`, `Relation`, and every `Memory` write path (single-shot writes,
  `apply_transaction` and its MVCC form, `delete_record`, `link_records`,
  `update_field`); a path that skips the sink is a write the log never holds.
- Crash trials: a kill between the apply and the group sync, and between the
  sync and the acknowledgement, each leaving a log the next open reads as a new
  epoch or a complete one; log order equals apply order under concurrency on all
  three adapters (the spike's test, extended); the standby test still equals the
  primary.
- A failed group `fsync` fails every writer it was covering, as the journal's
  does (`GRP-FR-005`); the spike returns `Storage` to the caller that led it and
  leaves the others to lead their own attempt.
- Independent inspection before merge: this is the durability path.

## Consequences (if 1)

- Positive: the log's throughput cost on a many-writer table falls from ~5.6×
  to ~1.5× at 16 writers; group commit works again with a log.
- Negative: a hook in each adapter's write path, and a second group-commit
  implementation beside the journal's (the two could share a helper once both
  exist, `KISS` until then).
- Not changed: the wire, the log's format, `FetchSince`, promotion.
