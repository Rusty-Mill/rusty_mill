# ADR-0096: `Compact` Reclaims MVCC History

- Status: **Proposed and implemented on one branch; the owner asked for
  the hardening** (2026-09-21). The fifth and last "do now" item of the
  release-readiness review, after `ADR-0092`–`ADR-0095`.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-MVCC-HISTORY-RECLAIM-DESIGN.md` (the
  full design), `ADR-0072` (`MVCC2-FR-010` item 5 — "GC proper",
  `MvccIndex::gc`, written and unit-tested but never called by
  production code; "reclaimed only at the next `Compact`" was the
  accepted cost, and no `Compact` reclaimed), `ADR-0071` (the spike
  that named the boundary rule), `ADR-0052` (`Compact`).
- Supersedes/Superseded by: none. Additive: `MvccState::reclaim`,
  `MvccIndex::history_len`, `MvccIndex::gc` returning its count, one
  call in each of `Memory`/`Entity`/`Relation`'s `compact`. No wire
  change (`CompactionReport` unchanged).

## Context

`MvccIndex::gc` — drop every chain entry older than the newest one at
or below the oldest open snapshot — existed since `ADR-0072`, with a
unit test proving its boundary rule, and had no caller outside the
research spike. Once a table went MVCC-active, every commit added an
entry that nothing ever removed; `ADR-0072` accepted "unbounded
between explicit `Compact` runs" and `Compact` did not run it. The
release-readiness review rated this High.

## Decision

Implement: `MvccState::reclaim()` — a no-op until MVCC is activated,
else `gc` at `open_snapshots.minimum()` (everything below the current
state when nothing is open) — called by `Memory`, `Entity`, and
`Relation`'s `compact` inside the exclusive section, before the
history flush, so the persisted `.mvcc` store is the reclaimed index.
An open snapshot keeps exactly what it can still read; a snapshot older
than a reclaimed interval already answers `HistoryReclaimed` →
`Conflict`, `ADR-0072`'s existing rule. Proven at the index, at the
state, and through the adapter: with a snapshot open `Compact` drops
only what is below it and that snapshot still reads its value;
released, the next `Compact` drops the rest and the current value
stands.

## Consequences

- Positive: history is bounded by the interval between `Compact`s
  and the oldest open snapshot — the shape `ADR-0072` promised.
- Negative / tradeoffs: reclamation is still tied to `Compact`; a
  table that is never compacted still grows. An automatic trigger (a
  history-size threshold, a timer) is the fork for the owner.
  `CompactionReport` does not say how much history was reclaimed (a
  wire change); the count is returned in code only.
- Named, not hidden: `Dog`/`Order`/`Employee` do not implement
  `Compact` (`Unsupported`) and so still never reclaim; they are the
  research domains.

## Acceptance and implementation

- 2026-09-21: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.80.0 / `FR-092`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — lib 657 (up from 654), 961 tests across 43 targets, 0 failed. Builder: Claude; independent Codex inspection owed.
