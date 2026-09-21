# ADR-0077: Walk-Past-Rejects for a Mixed-Filter `FilteredPage`

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076` precedent). Selected under the owner's standing
  "keep working the future-growth list" instruction as `ADR-0076`'s own
  option (b), the round after PR #276 merged (a); the fork below is
  held open for the owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-FILTERED-PAGE-WALK-MIXED-DESIGN.md` (the
  full design), `ADR-0076` (whose option (b) and first Non-goal this
  is), `ADR-0073`/`ADR-0075` (`plan_query`'s equality-first rule, which
  eligibility defers to), `ADR-0068` (`FilteredPage`), `ADR-0059`
  (`Ordered`/`page_by`, reused unchanged), `ADR-0055` (`Page`, whose
  consistency class the walk keeps), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive over `ADR-0076` — the same
  helpers widened, one new (`walked_key`), the two adapter overrides
  passing two more arguments; no generic, wire, protocol,
  `FieldCapabilities`, or client change.

## Context

`ADR-0076` made a `FilteredPage` ordered by `Memory`/`Relation`'s
`updated_at_unix_ms` whose filter is only bounds on that field cost the
page, and named that a *mixed* filter (`… AND category = 'x'`) still
paid the `ADR-0075` cost: every in-range record read, then `page_rows`
cuts the page. The `since-mixed` bench row, added and measured on the
pre-change code first, is that cost — 129,732.5 µs for a 50-row
page whose bound admits ~99% of 100K records and whose second predicate
admits 1% of those, against a 150,205.5 µs full-scan control.

Read against the code: a predicate on another field, or `!=` on the
walked field, can reject a row anywhere but cannot change where the walk
*ends* — only a bound on the walked key can, and those cut at the first
reject exactly as before. So the walk continues past the other rejects
until `limit` rows match, resuming each chunk strictly after the last
`(key, id)` it read — the key read off the record itself, so `page_by`
needs no pair-returning twin. Widening eligibility surfaced one latent
bug: `bounded_walk_start` computed a cursor from every `Gt`/`Ge`/`Eq`
predicate and would have answered `Malformed` for a `Str` equality on
another field; it now takes `order_by` and reads that field's bounds
alone.

The one rule that keeps this from fighting the planner: a filter that
plans the declared equality index (`plan_query`'s equality-first rule,
`ADR-0073`) keeps the bucket. Which is cheaper for a given request is a
selectivity question this crate has never estimated; the rule's cost is
measured (`since-eq`), not guessed.

## Decision

Implement: `bounded_walk_applies(order_by, range_field, schema, filter)`
— eligible iff `order_by` is the range field and the filter does not
plan `IndexEq`; `walked_key`; `bounded_walk_start(order_by, after,
filter)`; `bounded_filtered_page(store, walk, order_by, after, limit,
filter)` — chunks of `limit` through `page_by`, a *cut* (non-`Ne` bound
on the walked field) ending the page, a *reject* (anything else)
skipped, the cursor the greatest pair read. The eligible page returns
the identical sequence the default returns, proven in-process on a
fixture and on `Memory` against that exact oracle, and over a socket via
SQL on both domains.

The fork, held for the owner:

- **(a) As implemented.** Walk past rejects; equality-first kept.
- **(b) (a) plus intersecting the equality index with the range** —
  the first selectivity estimate (bucket size vs. range count). A
  second round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the mixed `since`-shaped listing costs the page divided
  by the rejects' selectivity — 3,933.2 µs against
  129,732.5 µs before, ~33× — and never more than the
  default read; `docs/FUTURE-GROWTH.md` drops "the walk-past-rejects
  extension … for a *mixed* filter".
- Named, not hidden: the worst case is every in-range record, as the
  default's was; a filter that plans the declared equality index keeps
  the bucket even where a tight bound would have beaten it
  (1,100.4 µs for the `category` twin, the rule's measured cost).
- Named, not hidden: `Page`'s consistency class, as under `ADR-0076`,
  with one change — a concurrently deleted id is back-filled from the
  next pair rather than shortening the page; a record re-keyed later
  between walk and read moves the next chunk's start.
- Named, not hidden: `ADR-0076`'s eligibility and start-cursor unit
  tests are rewritten for the new signatures (their `Ne`/second-field
  expectations were the contract this round changes); no other
  pre-existing test changed.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.62.0 /
  `FR-074`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 619 (up from 617), `server_sql_integration` 53
  (up from 52), every other target unchanged and green, 899 tests
  across 39 targets, 0 failed. Measured (acceptance criterion 5,
  `benches/server.rs`'s new `memory-planner` `since-mixed` row —
  `FilteredPage` `WHERE updated_at_unix_ms >= 1000 AND source = 'c7'
  ORDER BY updated_at_unix_ms LIMIT 50` over the same 100K `Memory`
  table, vs. the `created_at_unix_ms` + `source` full-scan control,
  over a real loopback socket, the rows added and measured on the
  pre-change code first, both binaries run back to back on an idle
  4-core Linux container): before this round 129,732.5 µs (control
  150,205.5 µs), after 3,933.2 µs (control 139,143.4 µs)
  — ~33×; the `since-eq` twin 1,368.8 → 1,100.4 µs,
  unchanged by construction — `RESULTS.md`. The fork above remains the
  owner's at review; (a) is what merges if the PR merges unchanged.
- 2026-09-21: option (b) taken the same day as `ADR-0078`
  (`docs/design/SERVER-QUERY-PLANNER-INTERSECT-DESIGN.md`), which
  found no estimate was needed: both indexes answer ids without a
  decode, so the intersection is exact. `bounded_walk_applies` now
  also stands aside for the intersection plan.
