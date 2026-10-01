# ADR-0129: The Equality Plan Intersects Every Indexed Bucket the Filter Names

- Status: **Proposed and implemented on one branch; the owner chose it**
  (2026-09-30, "and the multi-year items" — the cost-based optimizer's first
  step). No wire change.
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0073` (the equality plan: the first `Eq` on an indexed field
  in wire order), `ADR-0078`/`0079` (the bucket-and-range intersection and
  the crate's first cost constant), `ADR-0126` (`record_count`).
- Supersedes/Superseded by: refines `ADR-0073`'s "first in wire order".
  Additive: `equality_bucket`, `EQ_INTERSECT_RATIO`.

## Context

`Entity` declares two equality-indexed fields (`label`, `kind`), and the
other tables one each. A `WHERE kind = 'x' AND label = 'y'` read the bucket
of whichever predicate came first on the wire and decoded every record in it,
so the cost depended on the order the client happened to write the predicates:
a 20,000-record `kind` bucket first meant 20,000 decodes to find the one
record a one-id `label` bucket would have named. There is no statistic to
estimate from, and none is needed: a bucket's size is known exactly the
moment it is read as ids.

## Decision

`equality_bucket` reads the plan's own bucket; if the filter carries `Eq` on
*other* fields the schema also declares `filter_eq: true` (one predicate per
field), it reads those buckets as ids too and intersects them, smallest
first (`intersect_ids`, `ADR-0078`'s exact set logic), so only the records in
every bucket are decoded. Every predicate is still re-checked over what comes
back, so the answer is the same set on every path; a second index that
refuses is skipped, and an empty bucket ends the read. The intersect-with-a-
range plan (`ADR-0078`) starts from that narrower bucket, so its walk budget
shrinks with it.

**The one rule that keeps it from being worse than before** (a second
constant, `EQ_INTERSECT_RATIO = 25`, derived from `ADR-0079`'s measurement of
~1 µs per decode against ~40 ns per id visit): the extra buckets are read
only when `first_bucket_len * 25 >= record_count()`. An extra bucket is at
most the table, so below that ratio the id-level work it might cost exceeds
the decodes it could save, and the first bucket alone is read, exactly as
before. An adapter that cannot report its size (`record_count` `None`) never
trips the rule. Two known numbers are compared; nothing is estimated.

## Measurements

One run, one machine, release build, `Entity`, 100,000 records, `kind` five
values (20,000 each), `label` unique; best of 30 over the loopback socket
including the round trip:

| Query | Before (first bucket only) | Always intersect | This rule |
|---|---|---|---|
| `kind = 'concept' AND label = 'entity 12345'` (20k bucket first) | 7,824 µs | 592 µs | 509 µs |
| `label = 'entity 12345' AND kind = 'concept'` (1-id bucket first) | 293 µs | 562 µs | 250 µs |

Intersecting always is 15× better in the first row and 2× worse in the
second; the rule takes the first row's win and declines the second's loss.

## Consequences

- Positive: a two-index filter costs the intersection, not the first bucket,
  whatever order the predicates are written in.
- Negative / tradeoffs: the rule is conservative (a `Vec` copy of ids is
  cheaper than the 40 ns it is charged), so some profitable intersections
  are skipped; the order of the returned rows is the larger bucket's, not the
  first's (a `Query` is unordered, `ADR-0034`).
- Not done: a general cost model (no statistics exist, and this table shape
  offers at most one choice per predicate); an intersection of an equality
  with more than one range; `Ne`/`IN`-style predicates. The optimizer item in
  `FUTURE-GROWTH.md` stays open beyond this step.

## Acceptance and implementation

- 2026-09-30: `SERVER-001` v0.105.0 / `FR-118`. Four unit tests
  (`serve.rs`: the intersection decodes only the record in both, a small
  first bucket is read alone, an empty or refusing second bucket, a repeated
  or unindexed field adds none) and `tests/server_planner_eq_integration.rs`
  over a real `Entity` server. fmt, clippy (`--features server,research -D
  warnings`) and the crate's tests clean. Builder: Claude; independent
  inspection owed.
