# ADR-0113: Journaled Batches Reach the MVCC Index After a Restart, and the Review's Recorded Items

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "1 and 2" — the journaled-batch restart gap named
  in `ADR-0110`, and the items `ADR-0112` recorded but did not
  change). No wire change.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0110` ("Named, not hidden"), `ADR-0112` (the
  remainder), `ADR-0072` (MVCC durability), `ADR-0107` (journaled
  updates), `ADR-0098` (the CI feature sets), `ADR-0056` (the data-dir
  lock), `ADR-0065` (the exposure refusal),
  `docs/reports/2026-09-23-hardening-line-review.md` (Medium 12's
  neighbourhood, Low 21, Low 23).
- Supersedes/Superseded by: closes the gap `ADR-0110` named and the
  items `ADR-0112` recorded, except the two it leaves as they are
  (below). Additive: a `replayed_updates` field on the three adapters,
  filled by `with_journal` and consumed by `with_mvcc`.

## Context

A journaled `Transaction` batch commits to the MVCC index in memory and
to the journal on disk, nothing else, before it is acknowledged; the
`.mvcc` history file is written at the next flush (a checkpoint, an
in-place update, a non-journaled batch). A restart between the two
replays the journal into the *store* (`with_journal`) and reconstructs
the *index* from `.mvcc` (`with_mvcc`), so the store had the batch and
the index did not: a fresh snapshot after the reopen read the
pre-commit value while `GetById` read the committed one. Pre-existing
since `ADR-0072`; `ADR-0110` named it and left it. `memory_server`
refuses the journal-plus-MVCC combination, so only a library caller
could meet it — the reason it was not a High.

`ADR-0112` recorded five more items without changing them: Low 21's
two tests that observed less than they claimed (the allow-insecure
test only a TCP connect, the data-dir-lock test only a connect failure
after the child had exited); Low 23's untrimmed `PATH` in other crates'
Windows tests, the `set_var` race in sessionmgr's harness, and the
check name that embedded the MSRV pin; and the "TEMP" commit in
history.

## Decision

- `JMR-FR-001` — `with_journal` keeps every `Transaction` op it
  replayed (the ones `RVL-FR-004` skipped excluded) in
  `replayed_updates`; `with_mvcc`, on an index that `MvccState::open`
  reconstructed as active, records them all at one fresh transaction
  id through `with_index_quiet` (no reclaim trigger, no count), then
  flushes the history so the next open does not repeat the fold. An
  inactive index needs nothing: its first `Begin` seeds the baseline
  from the live store, which already holds the replay. The fold is
  ordered after the insert-log fold because the journal's entries are
  younger than any pending insert (*not in general — `ADR-0122` note: a
  non-journaled write to the same key after a journaled batch is
  re-applied by the replay and re-folded on top, so store and index
  agree on the older value; `ADR-0025`'s replay semantics, pre-existing*). Proven by journaling a batch on an
  active index, dropping without a checkpoint, and reopening through
  the portable open, `with_journal`, and `with_mvcc`: a fresh snapshot
  reads the batch. Removing the fold fails the test.
- `JMR-FR-002` (Low 21) — the allow-insecure test reads the child's
  stderr line by line up to the listening banner and asserts the
  `WARNING: listening on … (SERVER_ALLOW_INSECURE=1 is set)` line and
  no refusal; the data-dir-lock test asserts the refused second
  server's stderr holds no listening banner, so the refusal is proven
  to precede the bind rather than only to end the process.
- `JMR-FR-003` (Low 23) — `rush`'s Windows job-control harness strips
  every `\target\` entry from the `PATH` it hands the child, as
  sessionmgr's does; sessionmgr's own trim runs once under a
  `std::sync::Once`, so parallel tests no longer race a process-wide
  `set_var`. `platform-windows`'s parity test is unchanged: it shells
  out to `cmd` builtins only (`exit`, `if exist`, `echo`), which no
  `PATH` entry can shadow.
- `JMR-FR-004` (M14) — the root MSRV job is named
  `rusty_multimodal_db msrv (rust-version)`; the pin lives in
  `Cargo.toml` only, so bumping it no longer renames a required check.
- Not changed: the "TEMP … not for merge" commit stays in `main`'s
  history — rewriting a shared branch's history costs more than a
  clean-tree probe commit does.

## Consequences

- Positive: the last restart gap the review named is closed for the
  library's journal-plus-MVCC combination; the two Low-21 tests now
  prove what their names say; the Windows harnesses in three crates
  trim the same way; an MSRV bump is a one-line change.
- Negative / tradeoffs: a reopen with an active index and replayed
  batches pays one `.mvcc` flush; the fold gives every replayed op one
  shared transaction id, which is newer than every id in the history
  but does not reproduce the original commit boundaries — no snapshot
  from before the restart can be open, so nothing observes the
  difference. The root-workflow edit runs the whole workspace's CI once
  (Windows shards included).
- Named, not hidden: `memory_server` still refuses the journal-plus-MVCC
  combination (`SERVER_MVCC_ISOLATION` with `SERVER_TXN_JOURNAL_PATH`);
  this ADR fixes the library path only and does not lift that refusal.
  *Lifted by `ADR-0114`: `open_with_mvcc_journaled` and every replayed
  batch folded, atomic `WriteBatch` outcomes included.*

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.92.0 / `FR-105`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 981 tests across 44 targets, 0 failed. Builder: Claude.
