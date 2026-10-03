# ADR-0010: Persistence on the rusty_multimodal_db engine

- **Status:** Accepted
- **Date:** 2026-10-03

## Context

A run that stopped blocked on a question could not continue in a later
process: the plan, board, and ledger lived only in memory (ADR-0009,
consequences). ADR-0003 had pencilled in `rusty_sqlite` as the future
persistence edge, and ARCHITECTURE listed a board store as planned. The
owner then chose `rusty_multimodal_db` instead. Its product crate is an
apps-layer member of another family, which the workspace layer check
forbids as a dependency; its storage engine was extracted to the libs
layer as `rusty_multimodal_db_engine` for exactly this kind of embedder
(its ADR-0124, `rusty_remind_me`'s ADR-0021).

What has to survive: the plan with every card's lifecycle state, the
append-only board, and the ledger's call counts. The goal contract and the
routing come from the goal file, which the next process reads again.

## Decision

- **One snapshot record per goal.** `orch-store` saves the plan, board, and
  ledger of a goal as a single `GenericMmapStore` record, keyed by the goal
  id, replaced on every save. The engine makes a `replace` durable before
  it returns, so a save is atomic by construction and no journal is
  needed. Per-aggregate records with a journal batch are the upgrade when a
  second reader of the board exists; none does yet.
- **Rebuilt, not trusted.** `load` replays the snapshot through the
  domain's own constructors: `Plan::add` in id order and then each card's
  lifecycle transitions, `Board::append` in id order, and a new additive
  `Ledger::from_counts`. Cards depend only on earlier ids, so prerequisites
  hold their final state before a card starts. A snapshot the domain
  refuses is `StoreError::Corrupt`. `orch-core`'s API does not change.
- **Ids.** Orch ids are 1-based `u64` counters; the engine keys on `i64`.
  The cast is lossless. One index (the goal id) and one mmap slot (the
  save count, `revision`) satisfy the engine's record shape.
- **Fingerprint.** The FNV-1a hash of the goal file's text is stored with
  the snapshot. A run whose goal file hashes differently is refused
  (`RunError::GoalChanged`) rather than resumed against a plan built from
  another file.
- **Save points.** After every dispatcher run and after every answer
  round, in `rusty_orch::run::execute`. A crash mid-run re-runs at most the
  calls since the last block; the ledger is saved with the plan, so the
  ceilings still bound the total. Per-step saving would need a seam in
  `Dispatcher::run` and is not done.
- **Wall clock is per process.** `Budget::wall_clock` bounds one
  invocation. A goal waiting days for a human answer does not spend it.
- **Surface.** `rusty_orch run <goal.json> --state <dir>` (env
  `RUSTY_ORCH_STATE`). Without it the run is in memory as before. The
  directory is locked for the process (`StoreError::Locked`).
- **Deviation, accepted.** The engine's record bound needs `serde` derives
  on the snapshot rows, so `orch-store` depends on `serde` directly and
  carries the engine's pinned registry crates (`uuid`, `thiserror`,
  `serde`, `bincode`, `memmap2`) transitively: the first registry
  dependencies in the family, confined to this one crate. Its
  `rust-version` is the engine's 1.89; the other crates keep 1.75. The
  snapshot rows are bincode-encoded, so their field and variant order is
  the on-disk format: append, never reorder. The schema tag
  `rusty_orch::GoalRecord` is checked before any byte is decoded.

## Consequences

- A blocked run resumes: `rusty_orch run goal.json --state .orch` exits 3,
  the same command with `--interactive` later loads the state, asks, and
  continues with the ledger intact.
- ADR-0003's "Plan/Board persistence" row is superseded by this decision;
  `rusty_sqlite` is not used.
- Adding `orch-store` to the root manifest puts CI on the full sweep once
  more, as `orch-codex` did.
- The store keys several goals in one directory, but the binary always
  uses goal id 1 and one directory per goal. Multi-goal directories, a
  separate "answer" command, and resuming across a changed goal file are
  not in scope.
