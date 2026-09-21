# ADR-0079: Query Planner Step Five — A Budget on the Intersection's Range Walk

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0078` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  `ADR-0078`'s own option (b); the fork below is held open for the
  owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-INTERSECT-BUDGET-DESIGN.md`
  (the full design), `ADR-0078` (whose option (b) and measured worst
  case this answers), `ADR-0075` (`RangeBy`, extended by one method),
  `ADR-0073` (`plan_query`, untouched), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `RangeBy::range_by_limited`
  (generic, `Ordered` the one implementation), a defaulted
  `ConnectionStore::range_ids_limited` (`Memory`/`Relation`), one
  constant and one changed arm in `serve.rs`; no wire, protocol,
  client, or file-format change.

## Context

`ADR-0078` intersected the equality bucket with the range walk exactly
and without an estimate, and measured its own worst case honestly: on a
`since`-shaped bound the walk's id list is ~99,000 long, and visiting it
cost ~4 ms against ~1.3 ms for the bucket alone — the intersection lost
by ~4×. Its option (b), a width guard, came with its own objection: a
`count()` is O(k) itself, and a fixed threshold on a table's size is a
magic number — the first estimate this crate would keep.

Read against the code: a guard need not estimate. The walk is an
iterator; it can be *abandoned* at the first pair past a budget, having
cost exactly the budget and materialized nothing. Set the budget in
proportion to the bucket — ten id visits per bucket id, from this
crate's own measured ratio of a decode (~1 µs) to an id visit (~40 ns)
— and the worst case for a filter carrying both indexes is the bucket's
cost plus a bounded id walk (~1.4× the bucket alone), whatever the
range's width; the intersection is still taken, exactly, whenever the
range fits.

## Decision

Implement: `RangeBy::range_by_limited(lower, upper, limit) ->
Option<Vec<Id>>` on `Ordered` (the whole range if it fits, `None` at
the first id past the budget, no `Vec` built for a walk that does not
fit), exposed on `GenericProductionStore`; `ConnectionStore::
range_ids_limited` with an `Unsupported` default, implemented by
`Memory` and `Relation`; `INTERSECT_WALK_BUDGET = 10`; the
`IndexIntersect` arm reads the bucket (empty → nothing walked), walks
with `budget = |bucket| × 10`, intersects on `Some`, and reads the
bucket alone on `None` or a refusal. `plan_query` untouched. Every
consumer's result set is unchanged on every path, proven.

The fork, held for the owner:

- **(a) As implemented.** A fixed, measured budget ratio.
- **(b) The budget as an operator setting** —
  `SERVER_INTERSECT_WALK_BUDGET` through `ServeOptions::from_env`,
  default 10. A settings round.
- **(c) Decline and revert.** The intersection pays the whole walk.

## Consequences

- Positive (a): the wide `since-eq` page, `ADR-0078`'s worst case,
  5,170.9 → 1,468.9 µs; the narrow `eq-range` rows unchanged
  (97.5 → 129.8, 116.2 → 131.7,
  111.2 → 106.7); an empty bucket now walks nothing;
  `docs/FUTURE-GROWTH.md` drops "a width guard … the first
  selectivity estimate".
- Named, not hidden: one constant, this crate's first cost ratio,
  measured on one machine; the number a future cost model would
  replace, and option (b)'s setting if a deployment's ratio differs.
- Named, not hidden: the bound is ~1.4× the bucket alone in the worst
  case, not 1×; a range between the budget and the crossover (10× to
  ~25× the bucket) forgoes an intersection that would have won by a
  small margin.
- Named, not hidden: a generic-layer addition (`range_by_limited`),
  registered under this FR as `ADR-0059`/`ADR-0075` did.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.64.0 /
  `FR-076`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 625 (up from 622), every other target
  unchanged and green, 906 tests across 39 targets, 0 failed,
  every pre-existing test unmodified. Measured (`benches/server.rs`,
  100K `Memory` records over a real loopback socket, an idle 4-core
  Linux container, both binaries back to back): `since-eq` `fpage-50`
  5,170.9 → 1,468.9 µs; `eq-range` `query`/`count(*)`/`fpage-50`
  97.5/116.2/111.2 →
  129.8/131.7/106.7 — `RESULTS.md`. The fork
  above remains the owner's at review; (a) is what merges if the PR
  merges unchanged.
