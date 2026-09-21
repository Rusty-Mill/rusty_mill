# ADR-0078: Query Planner Step Four — Intersecting the Equality Index with the Range

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`/`ADR-0077` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  `ADR-0077`'s own option (b); the fork below is held open for the
  owner at review.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-INTERSECT-DESIGN.md` (the
  full design), `ADR-0077` (whose option (b) and first Non-goal this
  is), `ADR-0075` (whose Open questions carried "index intersection —
  the first cost-model question with a concrete shape; declined three
  rounds running"), `ADR-0073` (`plan_query`'s equality-first rule,
  amended here in one respect), `ADR-0074` (`indexed_candidates`),
  `ADR-0059` (`Ordered`/`range_by`), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive to `serve.rs`: a fourth
  `QueryPlan` variant, one `query_candidates` arm, `intersect_ids`,
  `bounded_walk_applies` widened by one variant; no generic, wire,
  protocol, adapter, or client change.

## Context

Three rounds declined to intersect the equality index with the range
because choosing between "bucket, re-check the bound" and "walk,
re-check the equality" needs a selectivity estimate, and this crate
keeps none. `ADR-0077` measured the cost of not choosing: a filter
carrying both reads the whole bucket. On the 100K planner table, `WHERE
category = 'c7' AND 50000 <= updated_at_unix_ms < 51000` admits ten
records and reads 1,000 — 1,008.5 µs for the `Query`, measured
on the pre-change code first.

Read against the code: no choice is needed. `filter_eq` and `range_ids`
both answer *ids* without decoding a record, so the two lists can be
intersected exactly — the smaller hashed, the larger filtered — and
only the members read. The cost is id-level work on both lists plus a
decode per record in *both*, never a decode per record in *either*. The
one thing this is not is a cost model: a wide range (`updated_at >=
1000`) makes the walk's id list ~99,000 long, and that id-walk is paid
unconditionally — the `since-eq` row measures it.

## Decision

Implement: `QueryPlan::IndexIntersect { eq, lower, upper }`, planned
whenever the first eligible equality and a bound on the range field
are both present (each found exactly as before, independently);
`query_candidates` fetches both id lists, intersects, and reads the
intersection; a walk refusal reads the bucket alone, a bucket refusal
scans; `bounded_walk_applies` treats the intersection like the bucket
(the default body answers the page). Every consumer's result *set* is
unchanged by construction and proven in-process, on `Memory`, and over
a socket.

The fork, held for the owner:

- **(a) As implemented.** Intersect whenever both apply; no estimate.
- **(b) (a) plus a width guard** — skip the walk when the range is
  "wide". Any guard is an estimate (a `count()` is O(k) itself, a
  threshold is a magic number): the first real cost-model round, if a
  consumer ever pays the wide-range id walk in anger.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the narrow equality-plus-range shape reads its own
  rows — `query` 1,008.5 → 174.3 µs, `count(*)`
  1,028.7 → 91.5, the page 1,085.8 →
  101.4; every consumer gains it through `indexed_candidates`
  with no call-site change; `ADR-0075`'s "declined three rounds
  running" open question closes.
- Named, not hidden: the wide-range case pays the walk's id list —
  the `since-eq` page 1,270.7 → 5,030.0 µs — with no guard;
  option (b).
- Named, not hidden: the one amendment to equality-first — an
  eligible equality no longer *hides* a bound beside it. No filter
  that planned `IndexEq` reads *more* than it did; the result set is
  identical.
- Named, not hidden: one pre-existing `plan_query` assertion ("an
  eligible equality anywhere in the filter wins over a range") is
  rewritten to the intersection; no other pre-existing test changed.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.63.0 /
  `FR-075`; see the design's change history for the proof and the
  before/after measurement. Builder: Claude, under the host-takeover
  convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — lib 622 (up from 619), `server_sql_integration`
  54 (up from 53), every other target unchanged and green, 903
  tests across 39 targets, 0 failed. Measured (`benches/server.rs`'s
  new `memory-planner` `eq-range` rows and the existing `since-eq` row,
  100K `Memory` records over a real loopback socket, an idle 4-core
  Linux container, the rows added and measured on the pre-change code
  first): `eq-range` `query` 1,008.5 → 174.3 µs,
  `count(*)` 1,028.7 → 91.5, `fpage-50` 1,085.8 →
  101.4 (full-scan controls ~140,000 both runs); `since-eq`
  `fpage-50` 1,270.7 → 5,030.0 — `RESULTS.md`. The fork above
  remains the owner's at review; (a) is what merges if the PR merges
  unchanged.
- 2026-09-21: option (b) taken the same day as `ADR-0079`
  (`docs/design/SERVER-QUERY-PLANNER-INTERSECT-BUDGET-DESIGN.md`) —
  read as a *budget* on the walk rather than an estimate of the range:
  the walk is abandoned at the first id past ten per bucket id, so the
  worst case above is bounded without a count or a size threshold.
