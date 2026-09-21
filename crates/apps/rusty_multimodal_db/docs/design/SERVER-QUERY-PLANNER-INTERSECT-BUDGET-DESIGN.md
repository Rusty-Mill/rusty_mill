# Server Query Planner, Step Five: A Budget on the Intersection's Range Walk (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0079`; the `ADR-0059`/`ADR-0076`–`ADR-0078` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as `ADR-0078`'s
  own option (b); the fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0078`/`docs/design/SERVER-QUERY-PLANNER-INTERSECT-DESIGN.md`
  (whose option (b) and first Non-goal this is — "a width guard … any
  such guard is an estimate"; whose `since-eq` row measured the
  intersection *losing* to the bucket alone by ~4× on a wide range),
  `ADR-0075` (`RangeBy`/`range_by`, extended by one method),
  `ADR-0073` (`plan_query`, untouched), `docs/FUTURE-GROWTH.md` (SQL-parity
  item 1: "a width guard on the equality-plus-range intersection …
  would be the first selectivity estimate this crate keeps").
- Supersedes/Superseded by: none. Additive: one method on the generic
  `RangeBy` trait (`range_by_limited`, implemented by `Ordered`, exposed
  on `GenericProductionStore`), one defaulted `ConnectionStore` method
  (`range_ids_limited`, implemented by `Memory`/`Relation`), one
  constant and one changed arm in `serve.rs`. No wire, protocol-version,
  `FieldCapabilities`, client, or file-format change; `plan_query`
  untouched. Every consumer returns the identical *set* (`QPB-FR-004`).

## Purpose and scope

`ADR-0078` intersected the equality bucket with the range walk
whenever a filter carried both, exactly and without an estimate — and
measured its own worst case: for the `since`-shaped bound
(`updated_at >= 1000`), the walk's id list is ~99,000 long, and
visiting it plus a hash lookup per id cost ~4 ms against ~1.3 ms for
the bucket's 1,000 decodes alone. The intersection lost by ~4× there.
`ADR-0078`'s option (b) named the fix and its objection in one breath:
a width guard skips the walk when the range is wide, but "any such
guard is an estimate — a `count()` is O(k) itself, and a fixed
threshold is a magic number."

Read against the code: a guard need not estimate anything. The walk is
an iterator over `(key, id)` pairs; it can be *abandoned* at the first
pair past a budget, having cost exactly the budget and materialized
nothing. Set the budget in proportion to the bucket — `B` ids per
bucket id — and the worst case for a filter carrying both indexes is
the bucket's own cost plus `B × |bucket|` id visits, whatever the
range's width; the intersection is still taken, exactly, whenever the
range fits. No count, no size, no estimate: a bound on work, decided by
the work itself.

The one number, `INTERSECT_WALK_BUDGET = 10`, is a ratio, not a
threshold on any table's size: it is the number of id visits worth one
record decode, halved, taken from this crate's own measurement
(`RESULTS.md`, step four: a decode ~1 µs, an id visit plus hash lookup
~40 ns, crossover ~25). Named plainly as this crate's first cost ratio,
and the one number a future cost model would replace.

Scope, exactly: the generic budgeted walk (`QPB-FR-001`); the adapter
method (`QPB-FR-002`); the intersection arm's use of it (`QPB-FR-003`);
set identity, proven (`QPB-FR-004`); the budget's bound on cost, stated
(`QPB-FR-005`); no other surface changed (`QPB-FR-006`).

## Non-goals

- **A selectivity estimate, a count, or a table-size threshold.** The
  budget bounds work; it estimates nothing.
- **Making the budget a setting** (`ServeOptions`/an environment
  variable). One measured ratio, one constant; option (b).
- **A budget on `IndexRange` alone.** A range with no equality beside
  it has nothing cheaper to fall back to; it walks as under
  `ADR-0075`.
- **Any change to `plan_query`, `IndexEq`, `IndexRange`, the
  `FilteredPage` walk, or the wire.**

## Context and terminology

Read from `main` after PR #278 (`SERVER-001` v0.63.0) this pass:

- **`RangeBy::range_by`** collects every id in the range. `Ordered`'s
  implementation guards the two `BTreeSet::range` panics; that guard
  is now a private `guarded_range` iterator both methods share.
- **`range_by_limited(lower, upper, limit) -> Option<Vec<Id>>`**: the
  whole range if it holds at most `limit` ids, `None` at the first id
  past it. `Some` is exact — the same `Vec` `range_by` would return.
- **`ConnectionStore::range_ids_limited`**: `range_ids`'s budgeted
  twin, `Ok(None)` past the budget, the same errors otherwise;
  defaulted to `Unsupported`, implemented by the two range-indexed
  adapters through `uuid_pair_bounds` exactly as `range_ids` is.
- **The intersection arm**: the bucket first; an empty bucket walks
  nothing (the answer is empty); otherwise `range_ids_limited` with
  `budget = |bucket| × INTERSECT_WALK_BUDGET`; `Some` → intersect,
  `None` → the bucket alone; a refusal → the bucket alone, as under
  `ADR-0078`.

## Requirements

- `QPB-FR-001` **The generic budgeted walk.** `RangeBy::range_by_limited`
  on `Ordered`: the ids within the bounds, ascending by `(key, id)`,
  if there are at most `limit` of them; `None` as soon as one more is
  seen, at most `limit + 1` pairs visited and no `Vec` returned. An
  inverted or empty range is `Some(empty)`. Exposed on
  `GenericProductionStore` under the read lock.
- `QPB-FR-002` **The adapter method.** `ConnectionStore::range_ids_limited(
  field, lower, upper, limit) -> Result<Option<Vec<RecordId>>, ErrorCode>`:
  `range_ids`'s contract with `Ok(None)` past the budget. `Memory` and
  `Relation` answer it over their `Ordered` index; every other domain
  keeps the `Unsupported` default.
- `QPB-FR-003` **The intersection arm.** For `IndexIntersect`: read the
  bucket's ids; if empty, the candidates are empty and nothing is
  walked; otherwise walk with `budget = |bucket| ×
  INTERSECT_WALK_BUDGET` (saturating): `Some(walked)` →
  `intersect_ids(bucket, walked)`; `None`, or any refusal → the bucket
  alone. `INTERSECT_WALK_BUDGET = 10`.
- `QPB-FR-004` **Set identity, proven.** Every consumer's result set is
  the full scan's on every path — within budget, past it, empty
  bucket, refusal — by construction (every predicate re-checked) and
  proven on `PlannerFixture` (with `get`/scan/walk counts, the
  over-budget path forced by a fixture flag since three rows can never
  exceed a real budget), on `Memory`'s adapter (`range_ids_limited`
  exactly at, under, and over the budget; `Unsupported`; `Malformed`),
  on the generic `Ordered` (`range_by_limited` exactly at, under, and
  over; empty; inverted), and over a real socket via the existing
  `ADR-0078` SQL test, which now exercises the within-budget path
  unmodified.
- `QPB-FR-005` **The bound.** For a filter carrying both indexes, the
  work is at most `|bucket|` decodes plus `INTERSECT_WALK_BUDGET ×
  |bucket| + 1` id visits and hash lookups — on this crate's own
  measurement, at most ~1.4× the bucket alone — whatever the range's
  width; and exactly `|bucket ∩ range|` decodes plus the two id lists'
  visits whenever the range fits the budget, as under `ADR-0078`.
- `QPB-FR-006` **Everything else unchanged.** `plan_query`, `IndexEq`,
  `IndexRange`, `FullScan`, the `FilteredPage` walk, the wire
  (`PROTOCOL_VERSION` 27), the clients, `sql.rs`: untouched.

## Considered options

- **(a) A fixed budget ratio — implemented.** One constant, measured
  once; the worst case bounded at ~1.4× the bucket alone.
- **(b) The budget as an operator setting.** `SERVER_INTERSECT_WALK_BUDGET`
  through `ServeOptions::from_env`, default 10. Real if a deployment's
  decode/visit ratio differs enough to matter; a settings round, and a
  number an operator would have to measure.
- **(c) Decline.** The intersection pays the whole id walk, as
  `ADR-0078` measured.

The owner's shorthand: **(a)** as implemented; **(b)** the budget as a
setting; **(c)** decline and revert.

## Proposed shape

`src/generic/query.rs`: `RangeBy::range_by_limited`. `src/generic/store.rs`:
`Ordered::guarded_range` (private), `range_by` and `range_by_limited`
over it. `src/generic/production.rs`: `range_by_limited`.
`src/server/serve.rs`: `ConnectionStore::range_ids_limited` (defaulted),
`INTERSECT_WALK_BUDGET`, the `IndexIntersect` arm. `src/server/memory.rs`
and `relation.rs`: `range_ids_limited`.

## Data/state and invariants

- **Set invariant**: unchanged from `ADR-0078` — the intersection is
  exact when taken, and the bucket alone is what `IndexEq` read; every
  predicate is re-checked on every path.
- **Work bound** (`QPB-FR-005`): the walk visits at most `budget + 1`
  pairs before abandoning; `Some` is returned only for a range that
  fits, so no `Vec` longer than `budget` is ever built for the walk.
- **Consistency class**: unchanged.

## Errors, failure, recovery, and observability

No new `ErrorCode`; `range_ids_limited` shares `range_ids`'s.
Refusals degrade as under `ADR-0078`. The path taken is not
observable on the wire.

## Security, privacy, and compatibility

A read, gated as each consumer already is; a subset of what `IndexEq`
read. No wire change; no protocol bump.

## Acceptance criteria

1. `Ordered::range_by_limited` unit test: exactly at the budget →
   `Some` (the whole range); under → `Some`; over and zero → `None`;
   an empty range fits any budget; inverted → `Some(empty)`.
2. `PlannerFixture` test: within budget → the intersection, one `get`
   per id in both, one budgeted walk; over budget (forced) → the bucket
   alone, every bucket id read, no scan; empty bucket → nothing walked,
   nothing read; `dispatch` identical on both.
3. `MemoryConnectionStore::range_ids_limited`: at/under/over the
   budget; `Unsupported` for another field; `Malformed` for a non-`I64`
   bound.
4. Every pre-existing test unmodified (the `ADR-0078` SQL test covers
   the socket path within budget).
5. Measured (`RESULTS.md`): the wide `since-eq` page, `ADR-0078`'s
   worst case, **before** (the whole ~99,000-id walk) and **after** (the
   walk abandoned at 10,000 ids, the bucket read) — the prediction:
   after ≈ the bucket alone plus ~0.4 ms; and the narrow `eq-range`
   rows unchanged (the range fits the budget).

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench binary before and after, back to back on an idle
machine. Independent review: written and built by Claude; a fresh
Codex inspection owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-INTERSECT-BUDGET`.
- Decision: `ADR-0079`.
- Specification: `SERVER-001` v0.64.0 / `FR-076` (extends `FR-075`;
  the `src/generic/` addition registered under the same FR,
  `ADR-0059`/FR-059's and `ADR-0075`/FR-072's precedent).
- Requirements: `QPB-FR-001`–`006`.

## Open questions

- **The budget as a setting** — option (b).
- **Plan metrics** — a per-plan counter (and now a "walk abandoned"
  counter) in `ServerMetrics`; still named, still not bundled. *Taken: `ADR-0086` (2026-09-21) —
  `docs/design/SERVER-QUERY-PLAN-METRICS-DESIGN.md`.*
- **Bound tightening** and **two equalities** — unchanged.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as
  `ADR-0078`'s own option (b) — read differently: a *budget* on work,
  not an estimate of size, needs no count and no threshold on any
  table. Read from `main` after PR #278 this pass.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.64.0 /
  `FR-076`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `src/generic/memory.rs` +1
  (`range_by_limited_answers_the_whole_range_within_the_budget_and_none_past_it`),
  `src/server/serve.rs` +1
  (`query_candidates_intersection_yields_to_the_bucket_past_the_walk_budget`),
  `src/server/memory.rs` +1
  (`range_ids_limited_walks_within_the_budget_and_yields_past_it`)
  (lib 625, up from 622); 906 tests across 39 targets, 0 failed,
  every pre-existing test unmodified; `fmt`/`clippy -D warnings`
  clean. Criterion 5, `RESULTS.md`: the wide `since-eq` page
  5,170.9 → 1,468.9 µs; the narrow `eq-range` `query`/
  `count(*)`/`fpage-50` 97.5/116.2/111.2 →
  129.8/131.7/106.7, unchanged within noise.
  Still no independent review — owed.
