# ADR-0075: Query Planner Step Three — A Range Path Through the `Ordered` Index

- Status: **Accepted as designed and implemented** (2026-09-20 — the
  owner picked option (a): the range candidate step for every consumer
  through the shared helper; (b) the O(page) `FilteredPage` walk on top
  and (c) decline both declined). Proposed, accepted, and implemented
  the same day — `SERVER-001` v0.60.0 / `FR-072`, no deviation (see
  "Acceptance and implementation").
- Date: 2026-09-20
- Deciders: baileyrd
- Related: `docs/design/SERVER-QUERY-PLANNER-RANGE-DESIGN.md` (the full
  design), `ADR-0073`/`docs/design/SERVER-QUERY-PLANNER-DESIGN.md` (step
  one — whose Non-goals scoped this out as "a second, distinct planner
  step with its own trait-surface question" and whose Open questions
  named its two needs: an "ids in key range" primitive and a way to
  declare a field range-indexed), `ADR-0074`/
  `docs/design/SERVER-QUERY-PLANNER-CONSUMERS-DESIGN.md` (step two —
  the shared `indexed_candidates` helper this round extends),
  `ADR-0059`/`docs/design/SERVER-ORDERED-INDEX-DESIGN.md` (`Ordered`,
  the `BTreeSet<(key, id)>` walked here — "a range [is] one walk of
  the same set, unrequested"), `ADR-0055`/`ADR-0068` (`Page`/
  `FilteredPage`), `ADR-0011` (`FieldCapabilities`, deliberately not
  extended), `docs/FUTURE-GROWTH.md` (SQL-parity item 1: "Still absent:
  … a range path through the `Ordered` index").
- Supersedes/Superseded by: none. Additive — one new generic trait and
  impl, two defaulted `ConnectionStore` methods, two adapter overrides,
  one plan variant; no wire, protocol-version, `FieldCapabilities`,
  client, or file-format change.

## Context

`ADR-0073` and `ADR-0074` gave every filtered read the server offers —
`Query`, `Aggregate`, the `filtered_page` default, `Join`'s left side —
one shared candidate step: an `Eq` on a `describe()`-declared indexed
field reads the index's bucket instead of the table, every predicate
is re-checked, and the result set is identical by construction.
Measured: ~420–580 µs vs. ~100 ms for a 1,000-row answer over 100K
records. Every *ordering* predicate (`<`, `<=`, `>`, `>=`) still plans
`FullScan`.

Yet `Memory` and `Relation` already hold the structure a range wants:
`Ordered` (`ADR-0059`) keeps a `BTreeSet<(updated_at_unix_ms, id)>`
exact through every write and answers `page_by` as
`range((Excluded(cursor), Unbounded)).take(limit)`. A `WHERE
updated_at_unix_ms > …` is that walk without the `take`. Today the
index is reached only through unfiltered `Page`; the same predicate
inside any filtered read decodes all *n* records.

Read against the code this pass, the step needs exactly what
`ADR-0073` said it would: (1) a primitive `Ordered` does not expose —
"every id in this key range" — which is a ten-line `BTreeSet::range`
walk that must guard the two inputs std panics on (start > end; equal
and both `Excluded`), and must take bounds on `(key, id)` *pairs*
because `R::Id` is generic and has no `MIN`/`MAX` (the adapter, which
knows `Uuid`, supplies `nil()`/`max()`); and (2) a way to declare the
field. The declaration is the fork this round actually resolves:
`FieldCapabilities` is wire shape (a fourth flag is protocol 28, a
`downgrade_for_version` rewrite, a `SERVER-002` revision, golden
vectors, a Python-client change) for a fact a client cannot act on —
so the design declares it server-side, as a defaulted `ConnectionStore`
method pair (`range_field`, `range_ids`), the same server-only surface
`page_keys`/`filtered_page` already are.

Equality stays first: a filter that plans `IndexEq` today plans
`IndexEq` after this round, so the blast radius is exactly "requests
that full-scanned before."

Codex's sandbox still cannot spawn processes on this machine
(unresolved since 2026-09-17), so this design, like `ADR-0073`'s and
`ADR-0074`'s, is written by Claude under the host-takeover convention
with independent review owed.

## Decision

Propose: a `RangeBy` trait in `src/generic/query.rs` implemented by
`Ordered` and exposed on `GenericProductionStore`; `range_field()` /
`range_ids(field, lower, upper)` on `ConnectionStore` with defaults
(`None` / `Unsupported`), overridden by `Memory` and `Relation` for
`updated_at_unix_ms`; `QueryPlan::IndexRange { lower, upper }` chosen
by `plan_query` after the unchanged equality rule — the first `Gt`/
`Ge`/`Eq` on the range field supplies the lower bound, the first `Lt`/
`Le`/`Eq` the upper; `query_candidates` walks the range and reads each
id, falling back to the scan on any adapter refusal; every consumer
re-checks every predicate exactly as today, through the one
`indexed_candidates` helper they already share.

The fork, held for the owner:

- **(a) The range candidate step, through the shared helper, for
  every consumer (recommended).** Every filtered read shape narrows on
  an ordering predicate over `updated_at_unix_ms` at once, with no
  call-site, wire, or client change. Cost: `Query`/`Aggregate`/`Join`
  gain the same two order-level differences the equality path has
  (named, inside existing contracts); `FilteredPage` has none; a bound
  admitting most of the table gains nothing; a `since`-shaped
  `FilteredPage` still reads every in-range record before `page_rows`
  cuts the page.
- **(b) (a) plus the O(page) bounded walk for `FilteredPage` on
  `Memory`/`Relation`.** An adapter override for `order_by ==
  range_field` with only bounds on that field: one walk from
  `max(cursor, lower)` with `take(limit)`. The `since`-shaped listing
  then costs the page. Larger — two overrides with their own cursor/
  bound composition and sequence-identity proofs.
- **(c) Decline.** Ordering predicates stay full scans.

## Consequences

- Positive (a): `WHERE updated_at_unix_ms > v` (and `>=`, `<`, `<=`,
  two-sided, `=`) on `Memory`/`Relation` reads *k* records instead of
  *n* in `Query`, `Aggregate`, `FilteredPage`, and `Join`; the first of
  step one's two open questions closes; `docs/FUTURE-GROWTH.md` can
  drop "a range path through the `Ordered` index" from its still-absent
  list.
- Positive: `PROTOCOL_VERSION` stays 27; `FieldCapabilities` and every
  `describe()` are untouched; the four other domains and `Dog` change by
  zero lines.
- Named, not hidden: `crate::generic` is touched for the third time
  after `STORAGE-012` closed it — additively, one trait, one impl, one
  accessor, under `SERVER-001`'s next FR, exactly `ADR-0059`'s
  precedent.
- Named, not hidden: still no cost model — no bound tightening, no
  `IndexEq ∩ IndexRange`; equality-first is a compatibility rule, not a
  selectivity claim.
- Named, not hidden: the range-index declaration is not visible on the
  wire; a client that wants to *see* it is a protocol round this ADR
  does not take.
- Named, not hidden: `Reminder::due_at_unix_ms` stays a full scan on a
  range — its index is the equality `HashMap`, not `Ordered`.

## Acceptance and implementation

- 2026-09-20: proposed, design only.
- 2026-09-20: the owner picked option (a) the same session, recorded in
  the design PR (#273) before merge (the `ADR-0073`/`ADR-0074`
  precedent). Implementation not yet started — it follows on its own
  branch, extending `SERVER-001` at its next minor (`FR-072` on
  `FR-070`/`FR-071`), with `QPR-FR-005`'s per-consumer, per-domain
  proofs and acceptance criterion 7's measurement as its exit gate.
  Builder: Claude directly, under the host-takeover convention;
  independent Codex inspection owed, not claimed.
- 2026-09-20: implemented on `claude/planner-range-impl` —
  `SERVER-001` v0.60.0 / `SERVER-001-FR-072`, **no deviation** from the
  accepted design: `RangeBy` + `impl RangeBy for Ordered` +
  `GenericProductionStore::range_by` (`src/generic/`), `range_field`/
  `range_ids` defaults, `QueryPlan::IndexRange`, `plan_query`/
  `range_bounds`/`query_candidates`/`indexed_candidates`,
  `uuid_pair_bounds` (`serve.rs`), the `Memory`/`Relation` overrides.
  Proven: 1 new `generic/memory.rs` unit test, 6 new `serve.rs` unit
  tests (lib 614, up from 607), 4 new real-socket
  `tests/server_sql_integration.rs` tests (51, up from 47), 1 new
  `tests/server_memory_integration.rs` test (15, up from 14); 892 tests
  across 39 targets, 0 failed, every pre-existing test unmodified;
  `fmt`/`clippy -D warnings` clean. Measured (`benches/server.rs`
  `memory-planner` `index-range` rows, `RESULTS.md`): on the same 100K
  `Memory` table, the 1%-selective two-sided range
  `50000 <= updated_at_unix_ms < 51000` costs `Query` 543.6 µs
  vs. 94,205.6 µs through the never-range-indexed `created_at_unix_ms`
  control (~173×), `COUNT(*)` 400.6 µs vs. 84,703.1 µs
  (~211×), `FilteredPage` `LIMIT 50` 498.9 µs vs.
  89,241.0 µs (~179×), over a real loopback socket. Still no
  independent review — Codex's sandbox cannot spawn processes; a fresh
  Codex inspection of the design and this diff is owed and must not be
  reported as done.
