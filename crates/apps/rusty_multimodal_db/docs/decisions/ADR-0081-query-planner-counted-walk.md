# ADR-0081: Query Planner Step Six — A Count That Reads No Record

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0080` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the "decode-free count" slice `ADR-0080` named from its own
  measurement; the fork below is held open for the owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-COUNTED-WALK-DESIGN.md`
  (the full design), `ADR-0080` (whose `due-count` row read half the
  table to count it), `ADR-0035` (`Aggregate`, "a full scan then a
  bucket, with no optimizer of any kind"), `ADR-0074` (`Aggregate` on
  the candidate step), `ADR-0075`/`ADR-0079` (`RangeBy`, extended by
  one method; `guarded_range`), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `RangeBy::range_count`
  (generic, `Ordered` the one implementation), a defaulted
  `ConnectionStore::range_count` (`Memory`/`Relation`/`Reminder`),
  `counted_walk_applies`/`counted_walk`, one match in the `Aggregate`
  arm; no wire, protocol, client, or file-format change.

## Context

`ADR-0080` measured the consumer's due count at 22,312.4 µs after
the index (31,831.0 µs on this round's own pre-change run, the same
request under run-to-run variance): the walk names 50,000 of 100,000 ids and `Aggregate` decodes
every one before counting. The sorted index already knows the number —
one `(key, id)` pair per live record — and `evaluate_aggregate` decodes
only because, in general, the candidate step over-approximates. When
the filter is nothing but bounds on the range field (at most one per
side; two would need `ADR-0075`'s declined tightening), the walk is
exact and its length is the count.

## Decision

Implement: `RangeBy::range_count(lower, upper)` on `Ordered`
(`guarded_range(..).count()`), exposed on `GenericProductionStore`;
`ConnectionStore::range_count` with an `Unsupported` default,
implemented by the three range-indexed adapters; `counted_walk_applies`
(no `group_by`, every aggregate `COUNT(*)`, a range field, a filter of
at most one lower and one upper bound on it — empty allowed) and
`counted_walk` (one group, the count once per column, `limit`-truncated
as `evaluate_aggregate` does); the `Aggregate` arm asks it first and
decodes on `None`. `evaluate_aggregate`, `plan_query`, and the wire are
untouched. Every count is the decode path's count, proven on the
fixture, the generic index, and over a socket on all three domains.

The fork, held for the owner:

- **(a) As implemented.** `COUNT(*)` over a pure range.
- **(b) (a) plus `MIN`/`MAX` of the range field** — the walk's first
  and last key, no decode either (`MIN(due_at) WHERE due_at > now`,
  "the next due"). A second round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the due count 31,831.0 → 290.0 µs; `Memory`'s
  1%-range `count(*)` 976.6 → 105.4; `COUNT(*)` with no
  filter on a range-indexed domain is the index's size, no scan; a
  better consistency class than the decode path (one count under one
  lock, no id-then-`get` window).
- Named, not hidden: a count over an equality bucket still decodes
  (`Entity`'s `label` bucket is a superset, so no bucket length is
  trusted); two bounds on one side still decode; every other aggregate
  still decodes.
- Named, not hidden: a generic-layer addition (`range_count`),
  registered under this FR as `ADR-0059`/`ADR-0075`/`ADR-0079` did.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.66.0 /
  `FR-078`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 630 (up from 627), `server_sql_integration`
  56 (up from 55), every other target unchanged and green, 913
  tests across 39 targets, 0 failed, every pre-existing test
  unmodified. Measured (`benches/server.rs`, 100K records over a real
  loopback socket, an idle 4-core Linux container, both binaries back
  to back): `reminder-due` `due-count` 31,831.0 → 290.0 µs;
  `memory-planner` `index-range` `count(*)` 976.6 → 105.4 —
  `RESULTS.md`. The fork above remains the owner's at review; (a) is
  what merges if the PR merges unchanged.
