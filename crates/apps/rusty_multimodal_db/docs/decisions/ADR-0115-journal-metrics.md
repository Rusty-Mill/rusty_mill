# ADR-0115: `dogserver_journal_*`, Three Gauges Per Journaled Table

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "1-5", item 1 — the two metrics
  `docs/FUTURE-GROWTH.md` still named absent: journal size and queue
  depth). No wire change: the `Metrics` text gains three families,
  which is content, not shape.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0064` (`ServerMetrics`), `ADR-0069` (the HTTP scrape),
  `ADR-0109` (`dogserver_mvcc_history_entries`, the per-table gauge
  this round copies), `ADR-0026` (the commit group whose turn gate is
  the queue), `ADR-0025` (the journal and its checkpoint).
- Supersedes/Superseded by: none. Additive: `JournalStats`,
  `CommitGroup::stats`, `ConnectionStore::journal_stats` (defaulted
  `None`).

## Context

`ADR-0109` gave the version index a per-table gauge. The journal had
none: an operator could not see how far the file had grown toward its
checkpoint, how many entries a crash would replay, or whether writers
were stacking up behind the group's turn gate. The growth document
named "journal size" and "queue depth" as the two metrics still absent.

## Decision

- `JSM-FR-001` — `CommitGroup::stats` reads, under the group's own
  lock in one go, `JournalStats { bytes, entries_since_checkpoint,
  waiting_writers }`: the file's size header included, the entries the
  next checkpoint will drop, and the writers parked for their apply
  turn (`GRP-FR-003`), which is the group's queue depth. A poisoned
  lock reads as zeros: a scrape never fails on a journal that has.
  `ConnectionStore::journal_stats(&self) -> Option<JournalStats>` is
  defaulted `None`; every adapter that can hold a journal (`Dog`,
  `Employee`, `Order`, `Reminder`, `Memory`, `Entity`, `Relation`)
  answers `Some` when it does.
- `JSM-FR-002` — `render_metrics` renders, after the MVCC gauge, one
  family each: `dogserver_journal_bytes`,
  `dogserver_journal_entries_since_checkpoint`,
  `dogserver_journal_waiting_writers`, `{table="…"}` per journaled
  table, each read once per table so the three describe one instant;
  no header and no sample when no table has a journal. The three
  families and the MVCC one share `push_gauge_family`. The `Metrics`
  request and the HTTP scrape both carry them.
- Proven over the wire with two served tables, one journaled and one
  not: the journaled table reads its header length, `0` and `0`, the
  plain table has no sample; a `Transaction` grows the bytes and
  counts one entry since the checkpoint.

## Consequences

- Positive: the checkpoint cadence, the replay a crash would cost, and
  the write queue are visible; an alert on `waiting_writers` catches a
  stalled `fsync` before clients time out.
- Negative / tradeoffs: a render takes each journaled table's group
  lock briefly, the same lock every commit takes; `BatchJournal::len_bytes`
  is no longer test-only (`CommitGroup::len_bytes` stays a test hook;
  `stats` is the path).
- Named, not hidden: `waiting_writers` counts writers parked for their
  turn after their append, not writers blocked on the group's mutex
  before it — the latter has no counter and would need one on the
  mutex's own entry.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.94.0 / `FR-107`, in one PR with `ADR-0116`–`ADR-0119`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 990 tests across 46 targets, 0 failed. Builder: Claude.
