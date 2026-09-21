# ADR-0097: Synced Updates — `msync` Before an In-Place Update Is Acknowledged

- Status: **Proposed and implemented on one branch; the owner asked for
  it** (2026-09-21, "durability"). The one High finding the release-readiness
  review left after its five "do now" items.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-SYNCED-UPDATES-DESIGN.md` (the full
  design), `ADR-0046` (the insert log, `fsync`ed per entry — why
  `Insert`/`Replace`/`Delete`/`Link` were already durable on
  acknowledgement), `ADR-0025`/`ADR-0063` (the journal — why a
  journaled `Transaction`/`WriteBatch` already was), `STORAGE-017`
  (`SlotFile::write_value`, the in-place copy with no syscall),
  `src/bin/crash_safety_harness.rs` (trial 1, "unflushed loss"),
  `ADR-0092` (the round that named this as the remaining half).
- Supersedes/Superseded by: none. Additive: `with_synced_updates(bool)`
  and a private `sync_update_ack` on `Memory`/`Entity`/`Relation`, one
  check in each non-journaled `Transaction` arm, `SERVER_SYNC_UPDATES`
  on `memory_server`. No wire, library, or format change.

## Context

Every write but one reached disk before its acknowledgement: an
insert-log entry is `sync_data`ed, a journal entry `fsync`ed. The one
was the in-place field update — `UpdateField`, and a `Transaction`
batch on a table with no journal — a bounded copy into a mapped page
with no syscall (`STORAGE-017`), on disk at the next `Flush`,
checkpoint, or the OS's write-back. It survives a process crash (the
page cache is the file's, not the process's — the harness's trial 1)
and not a power loss. The consumer's most frequent write is exactly
this one (`access_count`). The review rated it High.

## Decision

Implement: `with_synced_updates(true)` — after a successful in-place
update, `msync` the table's slot files (the stack's `Flush`, under the
write lock) before answering; an `msync` failure withholds the
acknowledgement as `Storage`. Journaled arms are untouched. Opt-in and
unset by default; `memory_server` reads `SERVER_SYNC_UPDATES=1`. The
cost is measured, not assumed: `update_field` on `Memory`, release build, this container, 2,000 updates each: 0.1 → 105.4 µs per update at 1K rows, 0.2 → 99.7 µs at 100K rows (unsynced → synced); the `msync` costs what the insert log's per-entry `sync_data` costs, and does not scale with the table.

## Consequences

- Positive: with the setting, every acknowledged write on the three
  deployment tables is on disk (as far as `msync`/`fsync` reach) —
  the property a DBMS is expected to have, at the operator's choice.
- Negative / tradeoffs: one `msync` per update, ~105 µs at 1K rows
  on this container's disk — the same order as the insert log's
  per-entry `sync_data`. Opt-in means a deployment that sets nothing
  keeps the loss window; the fork for the owner is defaults on, or the
  journal made to cover `UpdateField` (option (c)).
- Named, not hidden: `msync` on the whole mapping, not one slot — the
  kernel writes only dirty pages, so the cost is per dirty page, not
  per file; a ranged flush is option (b). What `msync` adds is not
  observable in-process; the test pins the contract's shape, the
  guarantee is the syscall's. `Dog`/`Order`/`Employee` unchanged.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.81.0 / `FR-093`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 658 (up from 657), 962 tests across 43 targets, 0 failed. Measured: `update_field` on `Memory`, release build, this container, 2,000 updates each: 0.1 → 105.4 µs per update at 1K rows, 0.2 → 99.7 µs at 100K rows (unsynced → synced); the `msync` costs what the insert log's per-entry `sync_data` costs, and does not scale with the table. Builder: Claude; independent Codex
  inspection owed.
