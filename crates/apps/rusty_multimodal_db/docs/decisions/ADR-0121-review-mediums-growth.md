# ADR-0121: The Growth-Line Review's Twelve Mediums, Fixed

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "proceed" — the report's recommended order,
  Mediums after the Highs). No wire change: the downgrade rule gains
  one more shape it already promised to cover (`SERVER-002` 0.20.1).
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `docs/reports/2026-09-23-growth-line-review.md` items 4–15;
  `ADR-0115` (the gauge), `ADR-0104`/`ADR-0111` (the refusal thread),
  `ADR-0069`/`ADR-0112` (the scrape listener, `RVL-FR-008`),
  `ADR-0117` (`Null`, rules 3 and 4), `ADR-0114` (the reopen),
  `ADR-0096` (`Compact`'s flush), `ADR-0118` (the refresh CLI).
- Supersedes/Superseded by: corrects the ADRs above in the points
  below. Additive: `BUSY_REFUSAL_TOTAL`,
  `MAX_METRICS_HTTP_CONNECTIONS`, `METRICS_HTTP_TIMEOUT`,
  `SNAPSHOT_PREFIX`, `is_plain_file_name`.

## Decision

- `RGM-FR-001` (M4) — `waiting_writers` counts the followers parked on
  the group's `durable` condvar during a leader's `fsync` as well as
  the writers parked for their turn; a stalled `fsync` now shows.
- `RGM-FR-002` (M5) — a refusal has one deadline end to end,
  `BUSY_REFUSAL_TOTAL` (6 s): a watchdog thread shuts the socket down
  at the deadline whatever step is blocked on it, so a trickling TLS
  handshake can no longer pin a slot beyond it.
- `RGM-FR-003` (M7) — the metrics HTTP listener spawns through
  `thread::Builder`, caps concurrent scrapes at
  `MAX_METRICS_HTTP_CONNECTIONS` (16, past which an accept is closed
  unanswered), and gives each socket `METRICS_HTTP_TIMEOUT` (2 s) read
  and write timeouts.
- `RGM-FR-004` (M9) — `downgrade_for_version` strips a `Null` pair from
  both field lists of every `JoinedRows` row below 31; `Groups` and
  `ScanValues` carry the same debug assertion `StrList` has, since no
  field is nullable yet. `SERVER-002` §7 item 28 names `JoinedRows`.
- `RGM-FR-005` (M8) — both clients refuse, locally and before sending,
  a request carrying `Null` on a connection negotiated below 31
  (`ClientError::Unsupported`, `UnsupportedError`): rule 4, which an
  older server enforces by closing the connection without a reply.
- `RGM-FR-006` (M12) — a snapshot directory is
  `refresh-<secs>-<pid>-<seq>` and `snapshots`/`prune` recognise only
  that prefix with a plausible epoch; an operator's `2026-09-23` is
  left alone. The prefix is new since `ADR-0118` merged; no deployment
  holds the old names.
- `RGM-FR-007` (M13) — a directory that fails verification and cannot
  be renamed aside is removed, and the error says which happened.
- `RGM-FR-008` (M14) — a snapshot file name must be a single normal
  path component or the refresh is refused before any write.
- `RGM-FR-009` (M10) — the reopen flushes the version index before the
  sources it folded are destroyed: the pending insert-log entries are
  folded and flushed *before* the reopen clears the log
  (`persist_pending_history`), the journal is replayed without
  truncation, the replay folded and flushed, and only then truncated
  (`with_journal_inner`); an active index is flushed at every open so
  the reflected count matches the log the reopen reset.
- `RGM-FR-010` (M11) — `Compact` flushes the history again after
  clearing the insert log, with the count at zero. The review asked
  for a test of the `Write` fold arm after a `Compact`; writing it
  found that the arm could not rescue that case at all: the reopen
  folds the log into the store before the replay, so the replayed
  insert is a `Duplicate` that records nothing, and the stale count
  made the pending fold skip it. The second flush is the fix; the test
  fails without it. The `Write` arm stays: every journaled write also
  reaches the log today, so it is belt and braces, not load-bearing.
- `RGM-FR-011` (M15) — `src/lib.rs` says 31; `PROJECT-STATUS.md`'s
  current-milestone line is current; `TRACEABILITY.md`'s PLP row says
  which trial ran; `memory_server`'s docs carry
  `SERVER_METRICS_HTTP_ADDR`.

## Consequences

- Positive: the gauge means what its ADR says; a refusal is bounded;
  the scrape listener cannot take the process down; the wire rule
  holds on both sides; the refresh CLI cannot delete an operator's
  directory or write outside its own; a crash mid-reopen replays
  instead of losing; a compact no longer hides the next journaled
  insert from the index.
- Negative / tradeoffs: one extra sleeping thread per refusal for up
  to 6 s; one extra `.mvcc` write per reopen and per compact; a
  journaled reopen replays twice if it crashes between the flush and
  the truncate (idempotent).
- Named, not hidden: `Groups`/`ScanValues` still only assert; the day
  a field is nullable they need a real rule.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.99.0 / `FR-112`, `SERVER-002` 0.20.1, in one PR with `ADR-0120` and `ADR-0122`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 994 tests across 46 targets, 0 failed; Python 7 tests OK. Builder: Claude.
