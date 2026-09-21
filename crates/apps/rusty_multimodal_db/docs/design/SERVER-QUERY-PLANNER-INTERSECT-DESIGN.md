# Server Query Planner, Step Four: Intersecting the Equality Index with the Range (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0078`; the `ADR-0059`/`ADR-0076`/`ADR-0077` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as `ADR-0077`'s
  own option (b); the fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0077`/`docs/design/SERVER-FILTERED-PAGE-WALK-MIXED-DESIGN.md`
  (whose option (b) and first Non-goal this is — "intersecting the
  equality index with the range … a selectivity decision"),
  `ADR-0075`/`docs/design/SERVER-QUERY-PLANNER-RANGE-DESIGN.md` (whose
  Open questions name "bound tightening / index intersection — the
  first cost-model question with a concrete shape (`IndexEq ∩
  IndexRange`); declined three rounds running"), `ADR-0073`
  (`plan_query`, the equality-first rule this round amends),
  `ADR-0074` (`indexed_candidates`, through which every consumer gains
  this with no call-site change), `ADR-0059` (`Ordered`/`range_by`),
  `docs/FUTURE-GROWTH.md` (SQL-parity item 1: "intersecting the
  equality index with the range in a `FilteredPage` … the first
  selectivity estimate this crate has declined to keep").
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs`: a
  fourth `QueryPlan` variant, one arm in `query_candidates`, one free
  helper (`intersect_ids`), and `bounded_walk_applies`'s match widened
  by one variant. No generic-layer, wire, protocol-version,
  `FieldCapabilities`, adapter, client, or file-format change. Every
  consumer returns the identical *set* (`QPI-FR-004`).

## Purpose and scope

`ADR-0077` named it plainly: a filter carrying both an equality on a
declared `filter_eq` field and a bound on the range field planned the
bucket, read every record in it, and re-checked the bound per row —
correct, and for the narrow-range case wasteful. `WHERE category = 'c7'
AND 50000 <= updated_at_unix_ms < 51000` on the 100K planner table
admits ten records; the bucket holds 1,000; the `eq-range` bench rows
this round added and measured on the pre-change code first read all
1,000 (`RESULTS.md`). The alternative, walking the range and re-checking
the equality, reads 1,000 too. The rounds before declined to choose
between them because choosing needs a selectivity estimate, and this
crate keeps none.

Read against the code: neither needs choosing. `filter_eq` returns the
bucket's *ids* without reading a record; `range_ids` returns the walk's
*ids* without reading a record (a `BTreeSet::range` over `(key, id)`
pairs). The intersection of two exact id lists is exact — hash the
smaller, filter the larger — and only its members are read. No estimate
is taken because none is needed: the cost is id-level work on both
lists plus a decode per record in both, never a decode per record in
either.

What this is *not*: a cost model. When the range admits most of the
table (`since`-shaped, `updated_at >= 1000`), the walk's id list is
~99,000 pairs long — id-level work that the `since-eq` row measures
against the bucket-alone cost it replaces. That row is this round's
honest worst case, in the table below.

Scope, exactly: the plan (`QPI-FR-001`); the candidate step
(`QPI-FR-002`); every consumer and the `FilteredPage` walk's
eligibility (`QPI-FR-003`); set identity, proven (`QPI-FR-004`); the
degradations (`QPI-FR-005`); no other surface changed (`QPI-FR-006`).

## Non-goals

- **A selectivity estimate or cost model.** The intersection is taken
  whenever both indexes apply, unconditionally; where the range is wide
  its id walk is paid. Measured (`since-eq`), named, not decided.
- **Bound tightening** (`a > 1 AND a > 5`): the first bound in wire
  order still supplies each side, as under `ADR-0075`.
- **Two equalities intersected** (`category = … AND label = …`): the
  first eligible equality in wire order, as under `ADR-0073`.
- **A range on any field but the adapter's one `range_field`**, a
  second `Ordered` wrap, `Reminder` under `Ordered`: unchanged.
- **Any change to `plan_query`'s inputs**, `FieldCapabilities`, the
  wire, or an adapter.

## Context and terminology

Read from `main` after PR #277 (`SERVER-001` v0.62.0) this pass:

- **`plan_query`** returned `IndexEq(i)` for the first eligible
  equality *before looking for a bound at all*; the range branch ran
  only when no equality was eligible. It now finds both independently
  and combines them.
- **`query_candidates`** fetched one id list per plan and read each
  id. It now fetches two for `IndexIntersect`, intersects, and reads
  the intersection.
- **`bounded_walk_applies`** (`ADR-0077`) was "not `IndexEq`"; it is
  now "`FullScan` or `IndexRange`", so a filter planning the
  intersection answers its `FilteredPage` through the default body —
  the intersection, sorted, cut — and not the walk. The `since-eq`
  page moves from the bucket to the intersection.
- **`intersect_ids(a, b)`**: the ids in both, in the larger list's
  order (the smaller is hashed). The order of a candidate set is
  unspecified for `Query`/`Aggregate`/`Join` and irrelevant for
  `FilteredPage` (`page_rows` sorts), as every planner round has
  named.

## Requirements

- `QPI-FR-001` **The plan.** `plan_query(schema, range_field, filter)`
  finds `eq` (the first `Eq` in wire order on a `filter_eq: true`
  field) and, independently, `lower`/`upper` (the first `Gt`/`Ge`/`Eq`
  and the first `Lt`/`Le`/`Eq` on `range_field`). Both →
  `IndexIntersect { eq, lower, upper }`; `eq` alone → `IndexEq(eq)`;
  a bound alone → `IndexRange { lower, upper }`; neither → `FullScan`.
  Deterministic and value-blind.
- `QPI-FR-002` **The candidate step.** `IndexIntersect` asks
  `filter_eq` for the bucket's ids and `range_ids` for the walk's ids,
  reads no record for either, and reads each id of
  `intersect_ids(bucket, walked)` through `get` (a vanished id dropped,
  the class every plan has).
- `QPI-FR-003` **Every consumer.** Through `indexed_candidates`,
  `Query`, `Aggregate`, the `filtered_page` default body, and `Join`'s
  left side gain the intersection with no call-site change. A
  `FilteredPage` ordered by the range field whose filter plans the
  intersection answers through the default body (the intersection,
  sorted, cut), not `ADR-0077`'s walk.
- `QPI-FR-004` **Set identity, proven.** For every filter, the set of
  rows every consumer returns is the full scan's — by construction,
  since every consumer re-checks every predicate over the candidates —
  and proven: in-process on `PlannerFixture` (the intersection reads
  exactly the ids in both, one `get` each, no scan; `dispatch` answers
  the identical rows on the intersection, the bucket alone, and the
  scan), on `Memory` (the existing oracle test's `category` shape now
  takes this path and still equals the default), and over a real socket
  via SQL on `Memory` and `Relation` (`Query`, `COUNT(*)`, and the
  ordered page for one-sided, two-sided, `=`, empty-bucket, and
  empty-range shapes, with the ids pinned, through runtime `Insert`,
  re-keying `Replace`, and `Delete`).
- `QPI-FR-005` **Degradations.** A `range_ids` refusal on a declared
  range field reads the bucket alone — exactly what `IndexEq` read
  before this round; a `filter_eq` refusal on a declared index scans —
  exactly what `IndexEq` always did. Neither surfaces an error.
- `QPI-FR-006` **Everything else unchanged.** No generic-layer change;
  no `Request`/`Response`/`ErrorCode` variant; `PROTOCOL_VERSION` stays
  27; no adapter, client, `sql.rs`, or `SERVER-002` change; `IndexEq`,
  `IndexRange`, and `FullScan` plan and read exactly as before for
  every filter that does not carry both.

## Considered options

- **(a) Intersect whenever both apply — implemented.** No estimate;
  the wide-range worst case is the walk's id list, measured.
- **(b) (a) with a guard: skip the walk when the range is "wide".** Any
  such guard is an estimate — a `BTreeSet::range(..).count()` is O(k)
  itself, and a fixed threshold is a magic number. The first real cost
  model, if a consumer ever pays the wide-range id walk in anger.
- **(c) Decline.** The bucket keeps re-checking the bound per row.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus a
width guard, the first estimate; **(c)** decline and revert.

## Proposed shape

`src/server/serve.rs`: `QueryPlan::IndexIntersect { eq, lower, upper }`;
`plan_query` finds `eq` and the bounds independently and matches on
both; `query_candidates` gains the arm above with `bucket`/`walk`
closures shared with `IndexEq`/`IndexRange`; `pub fn intersect_ids(a,
b) -> Vec<RecordId>`; `bounded_walk_applies` matches `FullScan |
IndexRange`.

`benches/server.rs`: an `eq-range` `measure_planner_pair` row —
`intersect_filter(FIELD_CATEGORY, FIELD_UPDATED_AT)` (the 1% equality
and the 1% range, ten records) against the `source` + `created_at`
full-scan control — for `query`, `count(*)`, and `fpage-50`.

## Data/state and invariants

- **Set invariant** (`QPI-FR-004`): every row in the full scan's
  answer is in the bucket (it matches the equality) and in the walk
  (it matches the bounds), hence in the intersection; every row in the
  intersection is re-checked against every predicate. Equal sets.
- **Cost**: `|bucket| + |walk|` id-level operations (one hash insert
  per id of the smaller, one lookup per id of the larger) plus
  `|bucket ∩ walk|` decodes. Before: `|bucket|` decodes. A decode is
  ~1 µs on the planner table (`RESULTS.md`, step one); an id operation
  is tens of nanoseconds; so the intersection wins whenever the walk
  is shorter than roughly fifty times the bucket, and pays a bounded
  id-walk otherwise.
- **Consistency class**: unchanged — each id is read under its own
  `get`, a vanished one dropped, as every plan does.

## Errors, failure, recovery, and observability

No new `ErrorCode`; the two refusals degrade (`QPI-FR-005`), never
surface. Lock acquisitions: one for the bucket, one for the walk, one
per `get` — as `IndexEq` plus `IndexRange` would. The plan taken is
not observable on the wire, as with every plan.

## Security, privacy, and compatibility

A read, gated exactly as each consumer already is. The candidate set is
a subset of what `IndexEq` read; nothing becomes obtainable that was
not. No wire change; no protocol bump.

## Acceptance criteria

1. `plan_query` unit tests: both present (one side, both sides, `Eq`
   on the range field as both, either wire order, two eligible
   equalities → the first) → `IndexIntersect`; the equality with no
   range field, no bound, an ordering on a non-range field, or `Ne` on
   the range field → `IndexEq`; a bound alone → `IndexRange`; the
   existing "equality wins over a range" assertion rewritten to the
   intersection, named.
2. `intersect_ids` and `query_candidates` unit tests on
   `PlannerFixture`: exactly the ids in both, one `get` each, no scan;
   a walk refusal reads the bucket alone; a bucket refusal scans;
   `dispatch` answers the identical rows on all three.
3. `bounded_walk_applies`: a filter planning the intersection is
   ineligible for the walk (the existing "declared index keeps the
   bucket" assertion, unchanged in wording, now covers it).
4. Integration (`tests/server_sql_integration.rs`, real socket, SQL):
   `Query`, `COUNT(*)`, and the ordered page for the shapes in
   `QPI-FR-004` on `Memory` and `Relation`, ids pinned, through runtime
   writes. Every pre-existing test unmodified except the one
   `plan_query` assertion named in criterion 1.
5. Measured (`benches/server.rs` `memory-planner` `eq-range` rows and
   the existing `since-eq` row, `RESULTS.md`): `query`/`count(*)`/
   `fpage-50` for `WHERE category = 'c7' AND 50000 <=
   updated_at_unix_ms < 51000` **before** (the bucket, 1,000 read) and
   **after** (the intersection, 10 read), with the full-scan control in
   both runs; and `since-eq` (the wide range) before and after — the
   worst case, reported whichever way it goes.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench binary built before and after, run on an idle
machine. Independent review: written and built by Claude; a fresh Codex
inspection owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-INTERSECT`.
- Decision: `ADR-0078`.
- Specification: `SERVER-001` v0.63.0 / `FR-075` (extends `FR-070`,
  `FR-072`, `FR-074`).
- Requirements: `QPI-FR-001`–`006`.

## Open questions

- **A width guard** — option (b). *Taken: `ADR-0079` (2026-09-21),
  as a budget on the walk (abandoned past ten ids per bucket id)
  rather than an estimate of the range — no count, no size threshold.*
- **Plan metrics** — a per-plan counter in `ServerMetrics`, now four
  plans wide; still named, still not bundled. *Taken: `ADR-0086` (2026-09-21) —
  `docs/design/SERVER-QUERY-PLAN-METRICS-DESIGN.md`.*
- **Bound tightening** and **two equalities** — unchanged.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as
  `ADR-0077`'s own option (b) and the "index intersection" open
  question `ADR-0075` carried. Read from `main` after PR #277 this
  pass: that both `filter_eq` and `range_ids` answer ids without a
  decode is what makes the intersection exact and estimate-free. The
  `eq-range` bench rows were added and measured on the pre-change code
  first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.63.0 /
  `FR-075`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `serve.rs` +3
  (`plan_query_intersects_an_indexed_equality_with_a_range`,
  `intersect_ids_keeps_the_larger_list_s_order`,
  `query_candidates_intersection_reads_only_the_ids_in_both`) and one
  assertion rewritten (lib 622, up from 619),
  `tests/server_sql_integration.rs` +1
  (`an_indexed_equality_with_a_range_returns_the_exact_set_count_and_sequence`;
  54, up from 53); 903 tests across 39 targets, 0 failed;
  `fmt`/`clippy -D warnings` clean. Criterion 5, `RESULTS.md`: the
  narrow `eq-range` page 1,085.8 → 101.4 µs (`query`
  1,008.5 → 174.3, `count(*)` 1,028.7 →
  91.5); the wide `since-eq` page 1,270.7 → 5,030.0 µs.
  Still no independent review — owed.
