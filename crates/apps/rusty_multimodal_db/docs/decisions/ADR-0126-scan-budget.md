# ADR-0126: A Scan Budget for `Aggregate` and `Join`

- Status: **Proposed and implemented on one branch; the owner chose it**
  (2026-09-30, "and the multi-year items, scan budget + drain, nullable
  column, MVCC + journal together").
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0093` (the row cap; named this a non-goal: "their cost is
  the scan, a scan budget is a planner question"), `ADR-0086` (`plan_of`),
  `ADR-0079` (`INTERSECT_WALK_BUDGET`, the crate's only cost constant),
  `ADR-0102` (the row cap's clamp).
- Supersedes/Superseded by: none. Additive: `ConnectionStore::record_count`,
  `ServeOptions::with_max_scan_rows`, `request_exceeds_scan_budget`, one
  guard arm, `SERVER_MAX_SCAN_ROWS`. No wire change.

## Context

`ADR-0093` bounded what a peer can *receive*. An `Aggregate` or a `Join`
answers few rows and reads many: with no usable index the candidate step is
`scan_all`, which decodes every record. One request can hold a table's
worth of work regardless of the row cap.

## Decision

Under an opt-in budget, refuse before any read an `Aggregate` or `Join`
whose candidate step ([`plan_of`] = `FullScan`) would scan a table holding
more than `budget` records. The refusal is the row cap's: `TooLarge` at
protocol 25 or above, `Malformed` below (rule 3). An indexed or walked plan
reads less than the table and is never refused; a table at or under the
budget refuses nothing.

The table's size comes from a new `ConnectionStore::record_count`, the
length of the id list with no record decoded. Its default is `None`
("unknown"), which never trips the budget, so a third-party adapter is
unchanged; every shipped adapter answers `Some`.

`memory_server`: `SERVER_MAX_SCAN_ROWS=<n>`, unset by default. Turning it
on changes what a running deployment's `Aggregate` and `Join` answer, which
is the owner's call, as the defaults were for `ADR-0093`.

## Consequences

- Positive: an unindexed aggregate over a large table is refused for the
  price of an id-list length, not a table decode.
- Negative / tradeoffs: the budget is on table size, not on what the scan
  would keep, so a selective filter with no index is refused too; the
  remedy is a declared index or a larger budget. `Join`'s later relation
  lookups are bounded by the left candidates, so the left side's plan is the
  one checked. No estimate is made: like `ADR-0079`'s constant, the rule is
  a comparison, not a cost model.
- Not done: a per-request budget on the wire (a protocol round), and a
  budget on `Compact`.

## Acceptance and implementation

- 2026-09-30: implemented as `SERVER-001` v0.102.0 / `FR-115`.
  `tests/server_limits_integration.rs` proves the refusal, the protocol-24
  downgrade, a sorted-index `COUNT` untouched, and a budget equal to the
  table refusing nothing; a unit test covers `Join` and the non-scan
  requests. fmt and clippy (`--features server,research -D warnings`)
  clean; the crate's tests pass with 0 failures. Builder: Claude;
  independent inspection owed.
