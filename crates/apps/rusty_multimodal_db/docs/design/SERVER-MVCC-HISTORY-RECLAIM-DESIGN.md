# Server MVCC: `Compact` Reclaims History (Proposed and implemented)

- Status: **Proposed and implemented on one branch; the owner asked
  for it** (2026-09-21, `ADR-0096`). The fifth "do now" item of the
  release-readiness review.
- Date: 2026-09-21
- Related: `ADR-0072`/`docs/design/MVCC-PRODUCTION-DESIGN.md`
  (`MVCC2-FR-010`: the persistent history store, its flush at
  checkpoint/compact, and item 5, `gc`), `ADR-0071` (the spike's
  boundary rule), `ADR-0052` (`Compact`), `ADR-0095` (the round
  before).
- Supersedes/Superseded by: none. Additive: `MvccState::reclaim`,
  `MvccIndex::history_len`, `gc`'s return value, three call sites.

## Purpose and scope

Call the garbage collector that already existed, at the boundary that
already existed. `MvccIndex::gc(min_open_snapshot)` was `ADR-0072`'s
item 5, unit-tested and never wired; `Compact` was the named
reclamation point and never reclaimed history. This round connects
the two.

Scope, exactly: the index reports what it drops and holds
(`HRC-FR-001`); the state reclaims at the oldest open snapshot
(`HRC-FR-002`); `Compact` calls it before the flush (`HRC-FR-003`);
proven through the adapter (`HRC-FR-004`); nothing else changed
(`HRC-FR-005`).

## Non-goals

- **An automatic trigger.** No size threshold, no timer; `Compact`
  is the operator's call, as `ADR-0052` made it.
- **Reporting the count on the wire.** `CompactionReport` is
  unchanged.
- **`Dog`/`Order`/`Employee`.** They answer `Compact` with
  `Unsupported`; unchanged.

## Context and terminology

Read from `main` after PR #295 this pass:

- **`MvccIndex::gc(boundary) -> usize`**: for each chain, keep from the
  newest entry at or below the boundary; drop what precedes it;
  advance `reclaimed_through` to the kept entry's txn id; return the
  total dropped. `None` boundary → everything below the current entry.
- **`MvccIndex::history_len()`**: entries across every chain.
- **`MvccState::reclaim() -> usize`**: `0` unless active; else `gc` at
  `open_snapshots.minimum()` under the index lock.
- **The call**: in `Memory`/`Entity`/`Relation::compact`, inside
  `with_exclusive`, before `mvcc_flush_now` — so the `.mvcc` store
  written at this `Compact` holds the reclaimed chains and their
  `reclaimed_through`, and a reopen reconstructs the same bound.

## Requirements

- `HRC-FR-001` **The index.** `gc` returns the entries dropped;
  `history_len` the entries held; the boundary rule of `ADR-0072`
  unchanged (its existing test still passes).
- `HRC-FR-002` **The state.** `reclaim` is a no-op before activation;
  after, it reclaims at the oldest open snapshot and everything below
  current when none is open.
- `HRC-FR-003` **`Compact`.** Each of the three adapters reclaims
  before its history flush, under the same exclusive section.
- `HRC-FR-004` **Proven through the adapter.** Three commits on one
  field with a snapshot held after the first: `Compact` shrinks the
  history and the held snapshot still reads its value; released, the
  next `Compact` shrinks it further and the current value reads.
- `HRC-FR-005` **Everything else unchanged.** Wire, clients, the
  flush format (`reclaimed_through` was always persisted), the
  research domains.

## Considered options

- **(a) Reclaim at `Compact`, before the flush — implemented.**
- **(b) (a) plus an automatic trigger** at a history-size threshold,
  under the write lock. A policy with a number in it.
- **(c) Reclaim at every checkpoint/flush.** Broader than `ADR-0072`
  named; cheap, but every journal checkpoint would then touch history.
- **(d) Decline.**

The owner's shorthand: **(a)** as implemented; **(b)** an automatic
trigger; **(d)** decline and revert.

## Proposed shape

`src/server/mvcc.rs`: `gc`'s return, `history_len`, `reclaim`.
`src/server/{memory,entity,relation}.rs`: one call each.

## Data/state and invariants

- After `reclaim`, every open snapshot `s` can still read every key
  at `s` (the kept entry is the newest at or below the minimum, and
  the minimum is at most `s`).
- The current entry of every chain is never dropped.
- `history_len` after `reclaim` with nothing open equals the number
  of chains.

## Errors, failure, recovery, and observability

None new: `reclaim` cannot fail; a snapshot older than a reclaimed
interval answers `Conflict` as before. The count is not yet a metric.

## Security, privacy, and compatibility

No wire change; the `.mvcc` store's format is unchanged.

## Acceptance criteria

1. Unit: `HRC-FR-001`/`002` (two tests in `mvcc.rs`).
2. Adapter: `HRC-FR-004` (`memory.rs`).
3. Not measured: `Compact` is not on a request's hot path.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `SERVER-MVCC-HISTORY-RECLAIM`.
- Decision: `ADR-0096`.
- Specification: `SERVER-001` v0.80.0 / `FR-092`.
- Requirements: `HRC-FR-001`–`005`.

## Open questions

- **An automatic trigger** — option (b).
- **A history-size metric** — `dogserver_mvcc_history_entries`, the
  natural next `ServerMetrics` family.

## Change history

- 2026-09-21: proposed and implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` on the owner's word ("5"),
  the fifth "do now" item of the release-readiness review.
- 2026-09-21: implemented as `SERVER-001` v0.80.0 / `FR-092`. Acceptance
  criteria 1–2 are the tests: `mvcc.rs` +2, `memory.rs` +1. `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 657 (up from 654), 961 tests across 43 targets, 0 failed.
  Still no independent review — owed.
