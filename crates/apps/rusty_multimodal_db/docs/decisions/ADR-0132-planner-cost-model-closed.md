# ADR-0132: The Planner Keeps Exact Counts and Two Measured Constants; No Statistics (Decision)

- Status: **Accepted** — on the owner's "go with recommendations". Closes the
  "cost model with statistics" item in `docs/FUTURE-GROWTH.md` as a decision,
  with the condition that would reopen it. No code, no wire change.
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0073`..`0079` (the planner steps), `ADR-0126` (scan budget),
  `ADR-0129` (equality intersection), `docs/FUTURE-GROWTH.md`.

## Context

The growth list names "a real cost model with statistics" for the planner. A
statistics-driven optimizer means maintained histograms or distinct counts,
refreshed on write, used to *estimate* selectivity and pick a plan.

## What the planner already knows exactly

Every choice it makes today compares numbers it can read for free, not guesses:

- an equality bucket's length (`filter_eq` returns the ids) and the table's size
  (`record_count`), which decide whether reading more buckets pays
  (`b * EQ_INTERSECT_RATIO >= n`, `ADR-0129`);
- a range walk's cost against the bucket it would narrow (`INTERSECT_WALK_BUDGET`,
  `ADR-0079`), abandoned at a fixed number of ids per bucket id;
- the sorted index's own count between bounds (`ADR-0081`);
- the plan a request takes, and the table size, for the scan budget (`ADR-0126`).

Two constants (a decode is ~25 id visits, ~10 ids of walk per bucket id) were
measured, not tuned. The measured wins (7,824 µs → 509 µs on 100k `Entity`
records) came from *using* exact numbers, not from estimating them.

## Decision

Do not add statistics. An estimate is only worth its upkeep where the exact
number is unavailable. Here it is available wherever an index exists, and where
no index exists there is no alternative plan to choose between: a filter on an
unindexed field is a scan under any estimate. Statistics would add a structure
kept consistent under every write path (and the journal, MVCC and change log),
to price plans the planner cannot take.

## What would reopen it

A concrete workload where the planner has two real plans and the exact numbers
do not separate them: chiefly an unindexed field that is hot enough that a new
index is being weighed against the scan (then the answer is the index, which is
`ADR-0075`'s "a wrap and a `range_field` change, when a consumer asks"), or a
join order over three or more tables (the wire has two-table joins only). A
benchmark showing a mis-pick from the current rules is the bar, as `ADR-0129`
had.

## Consequences

- The item leaves the open list as a decision; no new maintenance surface.
- The two constants stay constants, not settings. Their measurements live in
  `RESULTS.md`.
