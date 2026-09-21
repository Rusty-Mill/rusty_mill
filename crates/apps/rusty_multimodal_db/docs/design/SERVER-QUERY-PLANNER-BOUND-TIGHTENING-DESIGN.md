# Server Query Planner, Step Eight: Bound Tightening (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0083`; the `ADR-0059`/`ADR-0076`–`ADR-0082` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the "bound
  tightening" open question every planner round since `ADR-0075` has
  carried; the fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0075`/`docs/design/SERVER-QUERY-PLANNER-RANGE-DESIGN.md`
  (whose `QPR-FR-003` took "the first lower and first upper in wire
  order, no tightening" and whose Open questions carried "bound
  tightening (`a > 1 AND a > 5`) — the first cost-model question with a
  concrete shape"), `ADR-0081`/`ADR-0082` (whose walk-only paths took
  "at most one bound per side" so that the walk stayed exact — the
  limitation this round removes), `ADR-0076` (`bounded_walk_start`,
  which already takes the tightest lower bound for the page walk — the
  precedent), `docs/FUTURE-GROWTH.md` ("never tightens two bounds on one
  field").
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs`:
  one free function, `tightest_bounds`, used by `plan_query` and the
  counted/keyed walks; `counted_walk_applies` loses its one-per-side
  rule. No generic-layer, wire, protocol-version, adapter, or client
  change. Every result is unchanged (`QBT-FR-004`).

## Purpose and scope

Since `ADR-0075` a `WHERE` with two lower bounds on the range field
walked from the *first* in wire order and re-checked the other over
decoded rows: `updated_at >= 40000 AND updated_at >= 50000` walked
10,000 records it would then drop. `ADR-0081` and `ADR-0082` then
declined a second bound per side outright — the walk had to be exact,
and "tightening" had been filed under the cost model. It is not one:
which of two bounds on one side is tighter is decided by comparing
their literals to each other, never to the table, and the tightest
implies the rest. `bounded_walk_start` (`ADR-0076`) already does this
for the page walk's lower side.

This round takes it everywhere the planner picks a bound: `plan_query`
walks between the tightest lower and tightest upper; the counted and
keyed walks accept any number of bounds per side and walk between the
tightest two. Deterministic, value-blind with respect to the table,
and exact.

Scope, exactly: the selection (`QBT-FR-001`); the walk-only paths
widened (`QBT-FR-002`); the plan (`QBT-FR-003`); identity, proven
(`QBT-FR-004`); no other surface changed (`QBT-FR-005`).

## Non-goals

- **Any estimate.** Nothing about the table is consulted.
- **Contradiction detection as an error.** `a = 3 AND a > 3` walks an
  empty range and answers nothing — as `ADR-0075`'s guarded `range_by`
  already did for `> 5 AND < 3`. No new `ErrorCode`.
- **Simplifying the filter.** The redundant predicate stays in the
  filter and is still re-checked on the decode paths; only the walk's
  bounds change.
- **`Ne`**, a second field, `plan_query`'s equality-first rule, the
  intersection and its budget: unchanged.

## Context and terminology

Read from `main` after PR #282 (`SERVER-001` v0.67.0) this pass:

- **`plan_query`** found `lower`/`upper` as the first `Gt`/`Ge`/`Eq`
  and first `Lt`/`Le`/`Eq` on the range field in wire order.
- **`counted_walk_applies`** counted bounds per side and refused two.
- **`counted_walk`/`keyed_walk`** took the first per side.
- **Tightest lower**: the greatest literal; at a tie, `Gt` (exclusive)
  over `Ge`/`Eq` (inclusive). **Tightest upper**: the least literal; at
  a tie, `Lt` over `Le`/`Eq`. Literals compared as `i128` through
  `page_key_value`, so `U32` and `I64` share one order.

## Requirements

- `QBT-FR-001` **The selection.** `tightest_bounds(range_field, filter)
  -> (Option<usize>, Option<usize>)`: the positions of the tightest
  lower and tightest upper bound on `range_field` as defined above;
  `None` for a side with no bound or without a range field; other
  fields' predicates and `Ne` ignored. Pure.
- `QBT-FR-002` **The walk-only paths.** `counted_walk_applies` accepts
  any number of `Gt`/`Ge`/`Lt`/`Le`/`Eq` on the range field (still
  nothing else); `counted_walk` and `keyed_walk` walk between the
  tightest two.
- `QBT-FR-003` **The plan.** `plan_query`'s `IndexRange`/`IndexIntersect`
  carry `tightest_bounds`' positions.
- `QBT-FR-004` **Identity, proven.** Every result is the full scan's —
  by construction (the tightest bound implies every other on its side,
  so the walk admits exactly the records every bound admits, and the
  decode paths still re-check every predicate) — and proven on
  `PlannerFixture` (the plan's positions; the tight range alone read,
  one `get` per admitted id; `dispatch` equal to the scan's rows; a
  pure-range count with two bounds per side from the index), by
  `tightest_bounds`' unit matrix, and over a real socket via SQL on
  `Memory` and `Reminder` (redundant lower/upper bounds, `Eq` among
  bounds, a contradiction, and a second field beside redundant bounds —
  `Query`, `COUNT(*)`, and the ordered page each exact). Two
  pre-existing assertions are rewritten to the new rule
  ("two lower bounds: the first in wire order, no tightening" in the
  `ADR-0075` plan test; the two "ineligible" shapes in `ADR-0081`'s
  eligibility test), named; no other pre-existing test changed.
- `QBT-FR-005` **Everything else unchanged.** No generic, adapter,
  wire (`PROTOCOL_VERSION` 27), or client change.

## Considered options

- **(a) Tighten at the planner — implemented.** One pure function,
  three call sites.
- **(b) (a) plus rejecting a contradiction at validation** — `a = 3 AND
  a > 3` as `Malformed` before any walk. A wire-visible behavior change
  (a request that answered empty would answer an error); a protocol
  round if ever wanted.
- **(c) Decline.** The first bound in wire order keeps winning.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus
contradiction as an error; **(c)** decline and revert.

## Proposed shape

`src/server/serve.rs`: `pub fn tightest_bounds`; `plan_query`,
`counted_walk_applies`, `counted_walk`, `keyed_walk` over it.
`benches/server.rs`: a `range-tight` `measure_planner_pair` row — the
1% range with a redundant looser lower bound ahead of it.

## Data/state and invariants

- **Implication invariant**: for two lower bounds `p`, `q` on one key
  with `p` tighter, every value admitted by `p` is admitted by `q`;
  likewise for upper bounds. Hence the walk between the tightest two
  admits exactly the intersection of all bounds.
- **Cost**: the walk visits only the tight range; the decode paths
  still re-check every predicate, now over fewer candidates.

## Errors, failure, recovery, and observability

No new `ErrorCode`. A contradiction walks an empty range. The path
taken is not observable on the wire.

## Security, privacy, and compatibility

A read; a subset of what was read; the same results. No wire change.

## Acceptance criteria

1. `tightest_bounds` unit matrix.
2. `PlannerFixture`: the plan's positions, the tight range alone read,
   `dispatch` equal to the scan's, the count from the index.
3. The two rewritten pre-existing assertions, named.
4. Integration on `Memory` and `Reminder`.
5. Measured (`RESULTS.md`): the `range-tight` rows before and after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-BOUND-TIGHTENING`.
- Decision: `ADR-0083`.
- Specification: `SERVER-001` v0.68.0 / `FR-080` (extends `FR-072`,
  `FR-078`, `FR-079`).
- Requirements: `QBT-FR-001`–`005`.

## Open questions

- **Contradiction as an error** — option (b).
- **Plan metrics** — still named, still not bundled.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as the
  open question every planner round since `ADR-0075` carried — read as
  a comparison between literals, not a cost model. Read from `main`
  after PR #282 this pass. The `range-tight` bench row was added and
  measured on the pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.68.0 /
  `FR-080`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `serve.rs` +2
  (`tightest_bounds_picks_the_greatest_lower_and_least_upper`,
  `a_looser_bound_beside_a_tighter_one_walks_the_tight_range_only`)
  and two assertions rewritten, `tests/server_sql_integration.rs` +1
  (`redundant_and_contradictory_bounds_on_the_range_field_answer_exactly`)
  (lib 635, up from 633; SQL 58, up from 57); 920 tests across
  39 targets, 0 failed. Criterion 5, `RESULTS.md`: `range-tight`
  `query` 12,231.3 → 1,369.8 µs, `count(*)` 13,136.5 →
  58.7, `fpage-50` 130.2 → 114.9. Still no
  independent review — owed.
