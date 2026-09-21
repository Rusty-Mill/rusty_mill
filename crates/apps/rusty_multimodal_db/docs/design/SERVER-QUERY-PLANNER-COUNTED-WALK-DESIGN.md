# Server Query Planner, Step Six: A Count That Reads No Record (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0081`; the `ADR-0059`/`ADR-0076`–`ADR-0080` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the "decode-free
  count" open question `ADR-0080` named from its own measurement; the
  fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0080`/`docs/design/SERVER-REMINDER-DUE-INDEX-DESIGN.md`
  (whose `due-count` row read half the table to count it and named
  this slice), `ADR-0035` (`Request::Aggregate`, `AGG-FR`, "a full scan
  then a bucket, with no optimizer of any kind"), `ADR-0074`
  (`Aggregate` on the candidate step), `ADR-0075` (`RangeBy`, extended
  by one method), `docs/FUTURE-GROWTH.md` (SQL-parity: "a query
  optimizer for aggregation … a bounded `GROUP BY`/`COUNT`/… exists as
  a full scan then a bucket").
- Supersedes/Superseded by: none. Additive: `RangeBy::range_count`
  (generic, `Ordered` the one implementation, exposed on
  `GenericProductionStore`), a defaulted `ConnectionStore::range_count`
  (`Memory`/`Relation`/`Reminder`), two free functions and one match in
  the `Aggregate` dispatch arm. No wire, protocol-version,
  `FieldCapabilities`, client, or file-format change; `evaluate_aggregate`
  untouched. Every count is the decode path's count (`QCW-FR-005`).

## Purpose and scope

`ADR-0080` put `Reminder` under `Ordered` and measured the consumer's
three due shapes: the page fell ~440×, the window ~120×, and the count
only ~2.4× — because `Request::Aggregate` still decodes every candidate
the walk names before counting them, and `due_at <= now` names half the
table. The sorted index already knows the answer: one `(key, id)` pair
per live record, so the number of pairs between two bounds *is* the
count, with no record read.

Read against the code: `evaluate_aggregate` re-checks every predicate
over decoded rows because, in general, the candidate step over-approximates
(an equality bucket may be a superset — `Entity`'s normalized `label`;
a range walk with other predicates beside it). When the filter is
nothing but bounds on the range field — one per side at most, since two
on a side would need `ADR-0075`'s declined bound tightening — the walk
is exact and its length is the count. That is this round: a
`COUNT(*)` with no `GROUP BY` whose filter is a pure range (or empty)
answers with `range_count` and reads nothing.

Scope, exactly: the generic count (`QCW-FR-001`); the adapter method
(`QCW-FR-002`); eligibility (`QCW-FR-003`); the answer's shape
(`QCW-FR-004`); identity, proven (`QCW-FR-005`); no other surface
changed (`QCW-FR-006`).

## Non-goals

- **Any other aggregate** (`SUM`/`AVG`/`MIN`/`MAX`, `GROUP BY`): each
  needs a value per row, hence a decode. `MIN`/`MAX` of the range field
  itself is the walk's first/last key — a real, unbuilt slice, named.
- **Counting an equality bucket** (`COUNT(*) WHERE category = 'x'`):
  `filter_eq` may be a superset (`Entity`'s `label`), so its length is
  not the count in general; a `FieldCapabilities`-level "exact" flag
  would be a protocol round.
- **Bound tightening** (`a >= 1 AND a >= 3`): two bounds on one side
  take the decode path, as under `ADR-0075`. *Taken: `ADR-0083`
  (2026-09-21) — the count walks between the tightest two.*
- **Any change to `evaluate_aggregate`, `plan_query`, or the wire.**

## Context and terminology

Read from `main` after PR #280 (`SERVER-001` v0.65.0) this pass:

- **The `Aggregate` arm**: `validate_aggregate`, then `evaluate_aggregate`
  over `indexed_candidates` — decoded rows, every predicate re-checked,
  bucketed, `limit`-truncated. It now asks `counted_walk` first and
  takes the decode path on `None`.
- **`counted_walk_applies(range_field, group_by, filter, aggregates)`**:
  pure; a range field, no `group_by`, at least one aggregate and every
  one `COUNT(*)` (`Count` with no field), every predicate a `Gt`/`Ge`/
  `Lt`/`Le`/`Eq` on the range field with at most one lower and one
  upper (an `Eq` is both). An empty filter qualifies: the whole index.
- **`counted_walk(store, …)`**: `range_bounds` from the first lower and
  first upper (each at most one), `range_count`, one group with an
  empty key and the count once per `COUNT(*)` column, `limit`-truncated
  exactly as `evaluate_aggregate` truncates (`Some(0)` → no group).
  `None` when ineligible or the adapter refuses.
- **`RangeBy::range_count`**: `guarded_range(..).count()`, the shared
  panic-guarded iterator from `ADR-0079`.

## Requirements

- `QCW-FR-001` **The generic count.** `RangeBy::range_count(lower,
  upper) -> usize` on `Ordered`: the number of pairs within the bounds,
  one visit per pair, no `Vec`, no record read; an inverted or empty
  range is `0`. Exposed on `GenericProductionStore` under the read lock.
- `QCW-FR-002` **The adapter method.** `ConnectionStore::range_count(
  field, lower, upper) -> Result<u64, ErrorCode>`: `range_ids`'s length
  with the same errors, defaulted `Unsupported`; `Memory`, `Relation`,
  and `Reminder` answer it over their `Ordered` index through
  `uuid_pair_bounds`.
- `QCW-FR-003` **Eligibility.** As "Context": no `group_by`, all
  `COUNT(*)`, a range field, a filter of at most one lower and one
  upper bound on it and nothing else (empty allowed).
- `QCW-FR-004` **The answer.** One `AggregateGroup { key: [], values:
  [I64(count); aggregates.len()] }`, truncated to `limit`; the count is
  `range_count` between `range_bounds` of the two sides.
- `QCW-FR-005` **Identity, proven.** The eligible count equals the
  decode path's count: by construction (the index holds one pair per
  live record and the bounds are the predicates), and proven on
  `PlannerFixture` (zero `get`s, zero scans, one `range_count`; the
  decode path's answer identical; the empty filter; two bounds;
  `limit: Some(0)`; an ineligible shape and a refusing adapter both
  decoding, still exact), on the generic `Ordered` (`range_count` equal
  to `range_by`'s length, bounds and inversion), and over a real socket
  via SQL on `Memory`, `Relation`, and `Reminder` (no filter, each
  comparator, two-sided, `=`, an empty range; the ineligible shapes —
  a second field, `Ne`, two lower bounds, the `due_at` equality that
  plans the bucket — still exact; through runtime `Insert`, re-keying
  `Replace`, and `Delete`). Every pre-existing test unmodified.
- `QCW-FR-006` **Everything else unchanged.** `evaluate_aggregate`,
  `plan_query`, `indexed_candidates`, the wire (`PROTOCOL_VERSION` 27),
  the clients, `sql.rs`: untouched.

## Considered options

- **(a) `COUNT(*)` over a pure range — implemented.** One generic
  method, one adapter method, two free functions, one match.
- **(b) (a) plus `MIN`/`MAX` of the range field** — the walk's first
  and last key, no decode either; a second round with its own shape
  (`MIN(due_at) WHERE due_at > now`, "the next due").
- **(c) Decline.** The count keeps decoding half the table.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus
`MIN`/`MAX` of the range field; **(c)** decline and revert.

## Proposed shape

`src/generic/query.rs`: `RangeBy::range_count`. `src/generic/store.rs`:
`Ordered`'s impl over `guarded_range`. `src/generic/production.rs`:
`range_count`. `src/server/serve.rs`: `ConnectionStore::range_count`
(defaulted), `counted_walk_applies`, `counted_walk`, the `Aggregate`
arm's match. `src/server/{memory,relation,reminder}.rs`: `range_count`.

## Data/state and invariants

- **Count invariant**: the index holds exactly one pair per live record
  (`ADR-0059`), so the pairs within the bounds are exactly the records
  every predicate admits when the predicates are those bounds.
- **Consistency class**: better than the decode path's — one count
  under one read lock, with no id-then-`get` window.
- **Cost**: one pair visit per record in range; no decode.

## Errors, failure, recovery, and observability

No new `ErrorCode`; a refusal degrades to the decode path. Nothing to
recover. The path taken is not observable on the wire.

## Security, privacy, and compatibility

A read, gated as `Aggregate` already is. The answer is the same
number; nothing becomes obtainable that was not. No wire change.

## Acceptance criteria

1. `counted_walk_applies` unit tests: the eligible and ineligible shapes
   in `QCW-FR-003`.
2. `dispatch` on `PlannerFixture`: the count from the index with zero
   `get`s and scans; the decode path's identical answer; empty filter;
   two bounds; `limit: Some(0)`; an ineligible shape and a refusing
   adapter decoding.
3. `Ordered::range_count` equal to `range_by`'s length.
4. Integration (`tests/server_sql_integration.rs`): `QCW-FR-005`'s
   shapes on all three range-indexed domains through runtime writes.
5. Measured (`RESULTS.md`): the `reminder-due` `due-count` row and the
   `memory-planner` `index-range` `count(*)` row before and after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-COUNTED-WALK`.
- Decision: `ADR-0081`.
- Specification: `SERVER-001` v0.66.0 / `FR-078` (extends `FR-038`'s
  `Aggregate` and `FR-072`'s `RangeBy`).
- Requirements: `QCW-FR-001`–`006`.

## Open questions

- **`MIN`/`MAX` of the range field** — option (b). *Taken: `ADR-0082`
  (2026-09-21), widened to `SUM`/`AVG`/`MIN`/`MAX` of the range field —
  `docs/design/SERVER-QUERY-PLANNER-KEYED-WALK-DESIGN.md`.*
- **An "exact" equality index** — a capability flag so a bucket's
  length could count too; a protocol round.
- **Plan metrics** — still named, still not bundled.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as the
  slice `ADR-0080`'s own `due-count` measurement named. Read from
  `main` after PR #280 this pass.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.66.0 /
  `FR-078`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `serve.rs` +2
  (`counted_walk_applies_only_to_a_pure_range_count`,
  `dispatch_counts_a_pure_range_from_the_index_without_reading_a_record`),
  `src/generic/memory.rs` +1 (`range_count_is_the_range_s_length`),
  `tests/server_sql_integration.rs` +1
  (`a_pure_range_count_from_the_index_matches_the_decoded_count_exactly`)
  (lib 630, up from 627; SQL 56, up from 55); 913 tests across
  39 targets, 0 failed, every pre-existing test unmodified. Criterion 5,
  `RESULTS.md`: `reminder-due` `due-count` 31,831.0 → 290.0 µs;
  `memory-planner` `index-range` `count(*)` 976.6 → 105.4.
  Still no independent review — owed.
