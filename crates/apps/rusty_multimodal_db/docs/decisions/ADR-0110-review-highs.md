# ADR-0110: The Review's Three High Findings, Fixed

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23 — the review's "do now" list, after "1"). No wire
  change; one client default changes.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `docs/reports/2026-09-23-hardening-line-review.md` (Highs
  1–3), `ADR-0108` (the in-place record this completes), `ADR-0096`/
  `ADR-0105` (the reclaim this race was against), `ADR-0072`
  (`mvcc_begin`), `ADR-0104` (the silent close past the refusal pool),
  `SERVER-001` `FR-026` (the pre-hello fallback this makes opt-in).
- Supersedes/Superseded by: amends `FR-026` (fallback opt-in) and
  `FR-100` (the in-place record is flushed). Additive:
  `MvccState::open_snapshot`, `ConnectOptions::allow_pre_hello_fallback`.

## Context

The independent review of `ADR-0092`–`ADR-0109` found three Highs,
two of them independently by two reviewers:

1. The in-place `UpdateField` recorded into the MVCC index
   (`ADR-0108`) but never flushed `.mvcc`, unlike the non-journaled
   batch arm it claimed to mirror; a restart rebuilt an index without
   the update and `mvcc_get` answered the old value.
2. `mvcc_begin` read `last_committed` under the index lock, released
   it, then registered the snapshot under the open-snapshots lock;
   between the two, `Compact`'s reclaim or `ADR-0105`'s automatic
   trigger could `gc` at a minimum that did not include the snapshot
   and drop the entries it was about to need — a spurious `Conflict`.
3. The Rust client's `FR-026` fallback (default on) treated any close
   under the `Hello` as a pre-hello server and re-dialed at version 1;
   a server at its connection cap closes the same way once its refusal
   pool is full (`ADR-0104`), so a refused connect could become a whole
   session at protocol 1 with no error.

## Decision

Implement all three:

- `RVW-FR-001` — the in-place arm flushes `.mvcc` after its record,
  exactly as the non-journaled batch arm does (with the journal's
  entries-since-checkpoint count when a journal exists), a failure
  withheld as `Storage`. Proven by a restart test: update in place,
  reopen through `open_with_mvcc`, `mvcc_get` at a fresh snapshot
  reads the new value.
- `RVW-FR-002` — `MvccState::open_snapshot` reads `last_committed`
  and registers the snapshot in one critical section under the index
  lock; the three `mvcc_begin`s use it; `reclaim` reads the oldest
  open snapshot under the index lock too (the automatic trigger
  already did). Lock order index → open_snapshots, the trigger's own,
  now everywhere. Proven with the trigger at one append: the write
  right after a begin reclaims at the snapshot, not below it.
- `RVW-FR-003` — `ConnectOptions::require_hello` defaults to `true`;
  `allow_pre_hello_fallback()` restores the old behaviour for the one
  server shape that needs it (`SERVER-001` v0.9.1, which no shipped
  binary is). The pre-hello tests opt in; the default-connect case
  now asserts the error.

## Consequences

- Positive: `MVCC2-FR-008` holds across a restart on every write
  path; the design invariant "after reclaim every open snapshot can
  still read every key at its txn" holds against the automatic
  trigger; a refused connect is an error, never a downgraded session.
- Negative / tradeoffs: an in-place update on an MVCC-active table
  now writes `.mvcc` — the whole index — per update, the cost the
  non-journaled batch arm already paid; a journaled deployment that
  wants cheap durable updates on an MVCC table should turn on
  `SERVER_JOURNAL_UPDATES` (which checkpoints instead). A client
  library caller who relied on the silent version-1 fallback must now
  opt in.
- Named, not hidden: a *journaled* batch's index record between
  checkpoints has the same restart gap (journal replay feeds the
  store, not the index) — the review's Medium 12 neighbourhood,
  pre-existing since `ADR-0072`, not touched here.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.89.0 / `FR-102`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — 975 tests across 44 targets, 0 failed. Builder: Claude; the review's Mediums are the next round.
