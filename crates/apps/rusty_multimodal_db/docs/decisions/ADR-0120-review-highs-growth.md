# ADR-0120: The Growth-Line Review's Three Highs, Fixed

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "proceed" — the recommended order of
  `docs/reports/2026-09-23-growth-line-review.md`, Highs first). No
  wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: the report; `ADR-0114` (`attach_mvcc`), `ADR-0072`
  (`MvccState::open`'s own rule that an inactive index folds nothing),
  `ADR-0119` (the `dm-log-writes` runbook), `ADR-0116` (the
  old-layout hint), `ADR-0066` (the migration tool's real interface).
- Supersedes/Superseded by: corrects `ADR-0114`, `ADR-0116` and
  `ADR-0119` in the three points below.

## Decision

- `RGF-FR-001` (High 1) — `attach_mvcc` folds the pending insert log
  only into an *active* index, exactly as the replay fold already did.
  An inactive index folds nothing: its first `Begin` seeds the baseline
  from the live store, which already holds the log. Before, the fold
  landed at txn 1 and the later baseline at txn 0 beneath it, so
  enabling MVCC on an existing table read a stale insert while
  `GetById` read the live update. `Memory`, `Entity`, `Relation`.
  Proven by a test that inserts, updates in place, reopens with
  `open_with_mvcc` and reads the update from the first snapshot; it
  fails without the gate.
- `RGF-FR-002` (High 2) — `scripts/power_loss_trial.sh` iterates the
  marks it set itself (`replay-log` has no `--list`), zeroes the data
  device before every replay so blocks written after a mark cannot
  survive into an earlier state, mounts read-write so ext4 replays its
  journal, seeds the torn-write mode with three flushed records, and
  refuses instead of hanging when the writer dies before its awaited
  line (both scripts). Still unrun here: no device-mapper.
- `RGF-FR-003` (High 3) — the old-layout hint names the two slot-file
  paths the migration tool takes, says `<new_dir>/memories.mmap` must
  not exist, says the tool migrates the memory table only and that
  `entities.mmap*`/`relations.mmap*` are copied over unchanged, and
  sets `SERVER_DATA_DIR=<new_dir>`. The test pins those four facts.
  The MVCC reopen arms of `open_stores`' consumers now go through
  `open_error` too (Low 24).

## Consequences

- Positive: the path the binary takes when MVCC is first enabled on an
  existing table is correct; the runbook can test a mark; the hint can
  be followed.
- Negative / tradeoffs: none new.
- Named, not hidden: the `dm-log-writes` runbook is corrected by
  reading `replay-log`'s source, not by running it.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.98.0 / `FR-111`, in one PR with `ADR-0121` and `ADR-0122`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 994 tests across 46 targets, 0 failed. Builder: Claude.
