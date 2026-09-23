# ADR-0114: The Journal and MVCC Compose — the Journaled Reopen

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "1" — the first item `ADR-0113` left open:
  `memory_server`'s refusal of `SERVER_TXN_JOURNAL_PATH` with
  `SERVER_MVCC_ISOLATION`). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0113` (replayed transactions folded at open),
  `ADR-0072` (MVCC production wiring; its "journal remainder"),
  `docs/design/MVCC-OPEN-HOOK-PROPOSAL.md` (the insert-log reopen,
  `open_with_mvcc`; its open question on the journal), `ADR-0025`
  (the redo journal), `ADR-0063` (journaled `WriteBatch`), `ADR-0107`
  (journaled updates), `ADR-0112` (`RVL-FR-002`, quiet folds).
- Supersedes/Superseded by: closes `ADR-0072`'s journal remainder and
  the open-hook proposal's open question; lifts the startup refusal
  `ADR-0072`'s production wiring added. Additive:
  `ReplayedBatch`, `open_with_mvcc_journaled`, `attach_mvcc` (private),
  the quiet record helpers.

## Context

Since `ADR-0072`, `memory_server` refused to start with both the
crash-atomic journal and real MVCC configured, because a reopened
table could not compose them: the normal reopen
(`open_or_create_*_production_stack`) folds and clears the insert log
before `with_journal` or `with_mvcc` can read it, so an ordinary write
pending in that log at the crash never reached the version index. The
open-hook proposal fixed the non-journaled reopen with
`open_with_mvcc` (read the log first, then reopen) and named the
journaled case as its open question. `ADR-0113` then folded the
`Transaction` batches `with_journal` replayed into the index — but
only those: a replayed atomic `WriteBatch` (insert, replace, delete)
still reached the store and not the index, and no constructor read the
pending log ahead of a journaled reopen. The refusal stayed.

## Decision

- `JMC-FR-001` — `with_journal` keeps everything its replay applied,
  in journal order, as `ReplayedBatch`: a `Transaction` holds the ops
  that applied (`RVL-FR-004` skips the moot ones); a `Write` pairs
  every op with its outcome, exactly as the live record does. The
  index folds each batch at one fresh transaction id through the same
  record logic the live paths use (`record_transaction_into`,
  `record_writes_into`; the `_quiet` variants hold the reclaim trigger
  off, `RVL-FR-002`). Journal order is kept: a replace then a
  transaction on one record leaves the transaction's value on top.
- `JMC-FR-002` — `open_with_mvcc_journaled(path, journal_path)` on
  `Memory`, `Entity`, `Relation`: read the pending insert log before
  the reopen clears it, reopen, `with_journal`, then attach the index
  with the pending entries folded first (the older) and the replayed
  batches after. `with_mvcc`, `open_with_mvcc` and the journaled reopen
  share one tail, `attach_mvcc`, which now flushes the history whenever
  it folded anything — pending log or replay — so the next open depends
  on neither a log the reopen cleared nor a journal the replay
  truncated. `open_with_mvcc` gains that flush; before, a second
  restart before any other flush could lose the folded entries from the
  index again.
- `JMC-FR-003` — `memory_server` accepts `SERVER_TXN_JOURNAL_PATH` and
  `SERVER_MVCC_ISOLATION` together: an existing `memory` table takes
  the journaled reopen; a fresh one takes `with_journal` then
  `with_mvcc` as before. The other two tables never had the journal.
  With it, `open_stores` no longer opens a table an MVCC constructor
  will reopen (`TableOpen::ReopenWithMvcc`): through `ADR-0113` the
  binary opened every table once there and a second time in the
  reopen — the accepted "doubled open" of `ADR-0072` — and that first,
  unused mapping had already folded and cleared the insert log, so
  `open_with_mvcc`'s pre-open read found nothing. An ordinary write
  survived only because it flushes the history itself; a journaled
  batch pending in the log did not, and the binary test below failed
  until the first open was removed. Proven through the real binary: a
  first process activates the index, commits an MVCC session's
  `UpdateField` (a journaled transaction) and an atomic `WriteBatch`
  insert; a second process on the same directory starts and a fresh
  snapshot on it sees both.
- Tests: `Memory`'s unit test journals an insert, a replace and a
  delete in one atomic batch and then a transaction on the replaced
  record, drops without a checkpoint, reopens with
  `open_with_mvcc_journaled`, and reads all of it from a fresh snapshot
  in journal order; removing the fold fails it. `Entity` and `Relation`
  carry the replace arm.

## Consequences

- Positive: the last named gap between the journal and MVCC is closed;
  an operator can have crash-atomic batches and snapshot isolation on
  the same table. The live and open-time records share one body, so
  they cannot drift. `open_with_mvcc`'s fold is durable.
- Negative / tradeoffs: a reopen with an active index pays one `.mvcc`
  flush when it folded anything; a replayed batch gets one fresh
  transaction id, newer than every id in the history — no snapshot from
  before the restart can be open, so nothing observes the difference. A
  replayed batch that the pending insert log already carried (a
  journaled insert also appends to the log live) is folded twice, at
  two ids, to the same value: idempotent. `memory_server`'s doubled open
  of a reopened MVCC table (`ADR-0072`'s accepted tradeoff) is gone,
  and with it the wasted mapping and the cleared log it caused.
- Named, not hidden: `apply_transaction`'s and `write_batch`'s
  journaled arms still flush the history only at a checkpoint; between
  checkpoints the journal is the batch's only durable record and the
  reopen above is what recovers it — the design of `ADR-0072`, now
  complete rather than changed.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.93.0 / `FR-106`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 985 tests across 44 targets, 0 failed. Builder: Claude.
