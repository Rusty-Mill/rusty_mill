# Server Query Planner, Step Two: The Equality-Index Candidate Step for `Aggregate`, `FilteredPage`, and `Join` (Proposed)

- Status: **Proposed** (2026-09-19, `ADR-0074`). Design only — no code in
  this round.
- Date: 2026-09-19
- Related: `ADR-0073`/`docs/design/SERVER-QUERY-PLANNER-DESIGN.md` (step
  one — `plan_query`/`query_candidates` for `Request::Query` alone, whose
  own option (b) and Non-goals named exactly this round: "all three
  could adopt the identical candidate step… left out of (a) so this
  round changes one request's evaluation and proves it, not four"),
  `ADR-0035`/`docs/design/SERVER-SQL-AGGREGATE-DESIGN.md`
  (`Request::Aggregate`, whose Non-goals inherit `ADR-0034`'s "no query
  planner" unchanged and whose own text fixes group order as
  unspecified), `ADR-0068`/`docs/design/SERVER-FILTERED-PAGE-DESIGN.md`
  (`Request::FilteredPage`, whose default evaluation "does not consult
  the index at all" and which named "a future round can override it for
  a domain that can narrow candidates more cheaply" — this round narrows
  candidates for every domain without an override), `ADR-0044`/
  `docs/design/SERVER-SQL-JOIN-DESIGN.md` (`Request::Join`, whose left
  side is the same `scan_all`-then-filter shape), `docs/FUTURE-GROWTH.md`
  ("Path to SQLite/DuckDB parity", item 1 — since `ADR-0073`: "`Query`
  only (`Aggregate`/`FilteredPage`/`Join` still scan)").
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs` only
  — the `Aggregate` dispatch arm, the `ConnectionStore::filtered_page`
  default *body* (signature unchanged), and `evaluate_join`'s left-side
  loop. No `Request`/`Response` variant, no protocol-version bump, no
  trait method added or re-signatured, no adapter edit, no client or
  `sql.rs` change. Every request returns the same *set* of rows/groups/
  pairs; see "Data/state and invariants" for the observable differences
  this round names rather than hides (there are two, both inside
  existing "unspecified order" contracts, and one consumer has none).

## Purpose and scope

`ADR-0073` made the planner real for exactly one request: `dispatch`'s
`Request::Query` arm now plans `IndexEq` for the first `Eq` predicate on
a `describe()`-declared `filter_eq: true` field and reads that index's
bucket through the adapter's own `filter_eq` + per-id `get` instead of
`scan_all`, then re-checks every predicate with the unchanged
`evaluate_query` — measured at 576 µs vs. 104.5 ms for the same
1,000-row, 1%-selective equality over 100K `Memory` records
(`RESULTS.md`). It deliberately left the three other `scan_all`-then-
`predicate_matches` consumers on the full scan so that one request's
evaluation could change and be proven before four did.

Read from `src/server/serve.rs` at the crate's current head (`SERVER-001`
v0.58.0), those three are:

- **`Request::Aggregate`** — `dispatch` calls
  `evaluate_aggregate(store.scan_all(), &group_by, &filter, &aggregates,
  limit, &schema)`; `evaluate_aggregate` filters every row with
  `predicate_matches` before bucketing.
- **`Request::FilteredPage`** — `dispatch` calls
  `store.filtered_page(order_by, after, limit, &filter)`; the trait
  default (the only implementation — no adapter overrides it; `Memory`/
  `Entity`/`Relation` override `page_keys` for the *unfiltered* `Page`
  path, not this) is `self.scan_all()` → keep rows passing every
  predicate → `page_rows` (sort by `(key, id)`, cursor, limit).
- **`Request::Join`** — `evaluate_join(store, right_store, &spec)`
  iterates `store.scan_all()`, skips rows failing `spec.left_filter`,
  then fetches right rows per left row through the relation methods and
  `right_store.get`.

This round applies step one's candidate step — the same `plan_query` and
`query_candidates`, unchanged — to all three, through one shared helper,
and proves each one the way step one was proven. It changes what is
*read*; each consumer's own filter re-check (already present in all
three) keeps what is *returned* identical.

Scope, exactly: one shared helper (`QPC-FR-001`); the `Aggregate` arm
(`QPC-FR-002`); the `filtered_page` default body (`QPC-FR-003`); the
`Join` left side (`QPC-FR-004`); per-consumer result-set equality proofs
(`QPC-FR-005`); the named observable differences (`QPC-FR-006`); zero
change to every other surface (`QPC-FR-007`).

## Non-goals

- **`Request::Page` and `page_keys`.** Unfiltered — there is no
  predicate to plan on. `Memory`/`Relation`'s `Ordered` fast path and
  the other domains' `page_keys` default are untouched.
- **`Join`'s right side.** Right rows are fetched by id from the
  relation (`neighbors`/`neighbors_by_relation`/`parent`/`children`) and
  filtered by `right_filter` per row — there is no scan to narrow.
  Unchanged.
- **A cost model, selectivity, or combining two indexes.** Step one's
  Non-goals, inherited verbatim: use the declared index whenever the
  filter has an eligible `Eq`, first in wire order, full stop.
- **The `Ordered` range index as a candidate source** (`FILTERED-PAGE-
  DESIGN.md`'s own open question about narrowing "when the filter's only
  predicate is on the ordered field"). A different primitive with a
  different trait-surface question; not this round.
- **Any adapter override of `filtered_page`.** The default body changes
  for every domain at once; no domain gains an override. If a later
  round wants the `Ordered` walk for a filtered page, that is where an
  override would go — the trait method already exists for it
  (`FPG-FR-004`'s own reason for being a trait method).
- **`WHERE id = …`, `Ne`/range predicates, client or wire change,
  `sql.rs` change.** All as step one.

## Context and terminology

Read from `src/server/serve.rs` at head, not assumed:

- **`plan_query(schema, filter) -> QueryPlan::{FullScan, IndexEq(i)}`**
  and **`query_candidates(store, plan, filter) -> Vec<(RecordId,
  Vec<(FieldRef, ScanValue)>)>`** — step one's two private functions
  beside `evaluate_query`. `query_candidates` falls back to `scan_all`
  on a `filter_eq` `Err`. Both are already generic over `S:
  ConnectionStore + ?Sized`, so they can be called from a trait
  default method with `self`.
- **`evaluate_aggregate`** re-applies `filter` to every input row
  (`rows.into_iter().filter(|(_, fields)| filter.iter().all(|p|
  predicate_matches(fields, p)))`) before bucketing — the same re-check
  `evaluate_query` performs, so a superset index (`Entity::label`) is
  corrected here too. Buckets are a `Vec` in **first-seen order**; a
  `group_by`-less request has exactly one implicit bucket regardless of
  input; `limit` truncates the group list after reduction.
  `SERVER-SQL-AGGREGATE-DESIGN.md` (Non-goals): "`Response::Groups`
  carries groups in whatever unspecified order the grouping computation
  produces them in… `LIMIT` truncates that same unspecified order, not a
  meaningful top-N."
- **`filtered_page` default** keeps rows passing every predicate, then
  `page_rows(filtered, order_by, after, limit)`, which computes each
  row's `(page_key, id)`, selects the page's ids with `page_ids`
  (strictly after the cursor, ascending, first `limit`), and returns
  those rows in that order. **Output order is a function of the
  filtered *set* alone**, never of input order.
- **`evaluate_join`** — for each `scan_all` row passing `left_filter`,
  the related ids via the adapter's relation method, one `right_store.
  get` per id, `right_filter`, both projections; `limit` truncates the
  pair count *and* stops the loop. `SERVER-SQL-JOIN-DESIGN.md`: "Rows
  come out in `scan_all` order then relation order." `scan_all`'s order
  is itself unspecified (`SQL-FR-004`), so this is a description of the
  mechanism, not a promise of a specific order — but it is a sentence
  this round must reword.
- **Cross-table `Join`** (`ADR-0050`): `handle_connection` calls
  `evaluate_join(left, right, spec)` with two different stores; the left
  side's plan uses the *left* store's schema. `dispatch`'s own arm is the
  within-one-table case (`store, store`).
- **Which existing tests touch an indexed-field filter on these three
  consumers** (read this pass): `tests/server_entity_integration.rs`
  groups on `kind` (`filter_eq: true`) against a hand-computed tally,
  and joins `entity` on `relates_to`/`mentioned_with`/`neighbors` with
  `WHERE` clauses on the left side. Whether any asserts a specific
  group/pair *order* (as opposed to a set) is for the implementation
  round to find by running them; an assertion that pins an order the
  contract calls unspecified is relaxed to a set comparison, and that
  relaxation is recorded as such — the planner is not bent to preserve
  accidental `all_ids` order.

## Requirements

- `QPC-FR-001` **One shared helper.** A private
  `fn indexed_candidates<S: ConnectionStore + ?Sized>(store: &S,
  schema: &DomainSchema, filter: &[Predicate]) -> Vec<(RecordId,
  Vec<(FieldRef, ScanValue)>)>` in `serve.rs`, defined as
  `query_candidates(store, plan_query(schema, filter), filter)`.
  `Request::Query`'s arm is rewritten to call it — a pure refactor with
  identical behavior, so all four consumers share one call site shape.
- `QPC-FR-002` **`Aggregate`.** `dispatch`'s arm passes
  `indexed_candidates(store, &schema, &filter)` to `evaluate_aggregate`
  in place of `store.scan_all()`; `evaluate_aggregate` itself is
  unchanged. A `group_by`-less request still yields exactly one bucket
  (the implicit-bucket branch does not depend on the input).
- `QPC-FR-003` **`FilteredPage`.** The `ConnectionStore::filtered_page`
  default body becomes: `indexed_candidates(self, &self.describe(),
  filter)`, keep rows passing every predicate (the existing re-check,
  unchanged), then `page_rows` as today. The method's signature, its
  doc's contract, `validate_filtered_page`, and `page_rows` are
  untouched; no adapter gains or needs an override. The one extra
  `self.describe()` per request is the same call `dispatch` already
  made for validation — cheap, and a trait default cannot receive the
  schema without a signature change `QPC-FR-007` forbids.
- `QPC-FR-004` **`Join`.** `evaluate_join`'s outer loop iterates
  `indexed_candidates(store, &store.describe(), &spec.left_filter)` in
  place of `store.scan_all()`; the `left_filter` re-check inside the
  loop stays (it is what corrects a superset index); the right side is
  untouched. For a cross-table join the plan is made against the left
  store's own `describe()`, which is the schema `left_filter` was
  validated against.
- `QPC-FR-005` **Result-set equality, proven per consumer.** Over a
  real socket, for every domain with a declared index the consumer can
  reach:
  - `Aggregate`: `SELECT k, COUNT(*) … WHERE f = v GROUP BY k` (and a
    `SUM`/`MIN`/`MAX` variant) returns, as a *set* of `(key, values)`,
    exactly the tally computed in the test from the unfiltered `SELECT *`
    filtered by exact `ScanValue` equality on `f` — including the
    `Entity::label` case, where the normalized index returns a superset
    and exact `Eq` must still govern the counts; and a `group_by`-less
    `COUNT(*) WHERE f = v` matching the exact-equality row count.
  - `FilteredPage`: `WHERE f = v ORDER BY <field> LIMIT n` returns
    **exactly the same sequence** as `page_rows` computed in the test
    over the exact-equality-filtered `SELECT *` rows, for a first page
    and for a cursored second page, and a no-`LIMIT` chunked walk
    covers every matching row once — the sequence, not just the set,
    because this consumer's order is contract.
  - `Join`: `… JOIN … ON <relation> WHERE a.f = v` returns, as a *set*
    of `(left_id, right_id)` pairs with their projections, exactly the
    pairs of the unfiltered join (no `left_filter` → `FullScan`, the
    oracle) whose left row has `f == v` by exact equality; also across
    two tables (`memory` → `entity` via `mentions`) with an indexed
    left filter on `category`.
  Each also after runtime `Insert`/`Replace`/`Delete` on the indexed
  value, on at least one domain per consumer.
- `QPC-FR-006` **Observable differences, named.** `FilteredPage`: none
  — output order is `page_rows`'s `(key, id)` order over the filtered set,
  identical on either plan. `Aggregate`: group order within
  `Response::Groups`, and which groups a `limit` keeps, may differ from
  v0.58.0 — both inside `AGG`'s existing "whatever unspecified order…
  `LIMIT` truncates that same unspecified order" contract; the implicit
  single bucket is unaffected. `Join`: pair order and which pairs a
  `limit` keeps may differ — inside `scan_all`'s own unspecified-order
  contract; `SERVER-SQL-JOIN-DESIGN.md`'s "in `scan_all` order then
  relation order" is reworded to "in candidate order (the plan's — see
  `ADR-0074`) then relation order." Any existing test that pins one of
  these orders is relaxed to a set comparison and the relaxation
  recorded in `ADR-0074`'s implementation notes.
- `QPC-FR-007` **Everything else unchanged.** No `ConnectionStore`
  method added, removed, or re-signatured (`filtered_page`'s default
  *body* changes, nothing else about it); no adapter file edited; no
  `Request`/`Response`/`ErrorCode` variant; `PROTOCOL_VERSION` stays 27;
  `sql.rs`, `client.rs`, `clients/python/**`, `SERVER-002` untouched.
  `Query`'s behavior is identical to v0.58.0. `Page`/`page_keys`/
  `FilterEq` evaluate exactly as before. `Dog` (no declared index) plans
  `FullScan` everywhere and is byte-for-byte unaffected.

## Considered options

- **(a) All three consumers through the shared helper — recommended,
  as scoped above.** One helper, three call-site substitutions, each
  consumer's existing re-check doing the correctness work. Real, named
  cost: two consumers gain the same order-level differences `Query`
  already has (`QPC-FR-006`), and the round proves three evaluation
  paths instead of one — the reason step one deferred it, now paid.
- **(b) `FilteredPage` only.** The one consumer with zero observable
  difference and the one the `rusty_remind_me` hub's paged, filtered
  listings hit hardest (`FILTERED-PAGE-DESIGN.md` named its lost index
  advantage plainly). Smallest change; leaves `Aggregate` and `Join` on
  the full scan for another round.
- **(c) Decline.** `Query` alone keeps the planner; the other three stay
  as `ADR-0073` left them, and `docs/FUTURE-GROWTH.md` keeps saying so.

The owner's shorthand: **(a)** all three; **(b)** `FilteredPage` only;
**(c)** decline.

## Proposed shape

`src/server/serve.rs` only:

- `fn indexed_candidates(...)` beside `plan_query`/`query_candidates`
  (`QPC-FR-001`); the `Query` arm calls it.
- The `Aggregate` arm: `store.scan_all()` → `indexed_candidates(store,
  &schema, &filter)` (`QPC-FR-002`).
- `ConnectionStore::filtered_page`'s default body: `self.scan_all()` →
  `indexed_candidates(self, &self.describe(), filter)`; its doc comment
  loses "`Memory`/`Relation`'s `Ordered` index is not consulted here"
  as the whole story and gains the equality-index sentence
  (`QPC-FR-003`).
- `evaluate_join`: `store.scan_all()` → `indexed_candidates(store,
  &store.describe(), &spec.left_filter)` (`QPC-FR-004`).
- `PlannerFixture` (step one's test double) gains whatever relation
  stubs the `Join` unit test needs (`neighbors` returning a fixed list
  instead of `Unsupported`), so all three consumers have an in-process
  test that counts `get`/`scan_all` calls the way step one's do.

No other file changes for the code. Docs at implementation: `SERVER-001`
next minor / FR (extending `FR-038` `Aggregate`, `FR-068` `FilteredPage`,
`FR-045` `Join`), `SERVER-SQL-JOIN-DESIGN.md`'s one reworded sentence,
`SERVER-FILTERED-PAGE-DESIGN.md`'s Consequences (the "loses the
`Ordered` index's speed advantage" line gains "but since `ADR-0074`
narrows candidates through a declared equality index"), `RESULTS.md`,
and the usual roadmap/status/traceability rows.

## Data/state and invariants

- **Result-set invariant, per consumer**: with no concurrent writer,
  each consumer's output *set* (groups; page sequence; pairs) is
  identical under `IndexEq` and `FullScan`. Holds for the same reason as
  step one — every shipped `filter_eq` returns a superset of the exact
  `Eq` matches, and every consumer re-applies the full predicate list
  itself — plus one consumer-specific fact each: `evaluate_aggregate`'s
  buckets are keyed by value (a group's *contents* do not depend on
  encounter order, only the group list's order does); `page_rows` sorts;
  `evaluate_join`'s pair set is the product of the left set and the
  relation, independent of left order.
- **Consistency class, unchanged**: the candidate fetch is `filter_eq`
  then per-id `get`, the same separate-acquisition shape `scan_all`
  already has (`ADR-0073`, "Context").
- **`FilteredPage`'s keyset invariant** (`SERVER-PAGE-DESIGN.md`: two
  consecutive pages are disjoint and together cover every key after the
  first cursor) is a property of `page_ids` over the filtered set and is
  untouched by how that set was gathered.
- **Session posture, unchanged**: none of the three is ever overlaid or
  read-set-tracked (`SQL-FR-009`, `AGG-FR-009`, `JOIN`'s and `FPG`'s
  own gating); `store.get` in the candidate fetch is the committed read.

## Errors, failure, recovery, and observability

No new `ErrorCode`; every rejection still comes from the existing
validators before any read. A `filter_eq` refusal falls back to the scan
inside `query_candidates` and never surfaces. No new metric — `ADR-0073`'s
open question about plan-taken counters now covers four consumers and
is worth slightly more; still not bundled.

## Security, privacy, and compatibility

Reads, gated exactly as each request is today. Strictly fewer records
read, identical sets returned. No wire change; `PROTOCOL_VERSION` stays
27; audit/access-log entries unchanged (one request, one entry,
regardless of plan).

## Acceptance criteria

1. `PlannerFixture` unit tests: `Aggregate` through `dispatch` with an
   indexed `Eq` reads only the bucket (`get` counter, zero `scan_all`)
   and returns the same groups as with the index unavailable; the
   `filtered_page` default reads only the bucket and returns the
   identical sequence either way; `evaluate_join` reads only the bucket
   on the left and returns the same pair set either way; a superset
   index yields the exact-match result for all three; a refusing index
   falls back with no error for all three.
2. Integration (`tests/server_sql_integration.rs` and the cross-table
   case in `tests/server_memory_integration.rs`, real sockets): every
   `QPC-FR-005` proof above.
3. Every pre-existing test passes unmodified **or** is recorded in
   `ADR-0074` as an order-pin on an unspecified order that was relaxed
   to a set comparison — with the test name and the sentence in the
   governing design doc that makes the order unspecified.
4. Measured (`RESULTS.md`, extending `benches/server.rs`'s
   `memory-planner` rows on the same 100K `Memory` table, `category` vs.
   the unindexed `source` twin): `COUNT(*) … WHERE f = 'c7'`
   (`Aggregate`), and `WHERE f = 'c7' ORDER BY updated_at_unix_ms LIMIT
   50` (`FilteredPage`), each under both plans. `Join` is not measured
   this round — its left side is the same candidate fetch `Query`
   measured, and its cost is dominated by the per-left-row right-side
   lookups this round does not touch; named, not hidden.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
`cargo bench -p rusty_multimodal_db --features server,research --bench
server` for criterion 4. Independent review: as with `ADR-0073`, Codex's
sandbox cannot spawn processes on this machine (its runner logs on as
the sandbox user and never connects its pipe — investigated 2026-09-19,
unresolved); this design was written by Claude, and a fresh Codex review
of it and of `ADR-0073`'s implementation is owed and not claimed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-CONSUMERS-DESIGN`,
  `SERVER-QUERY-PLANNER-CONSUMERS`.
- Decision: `ADR-0074`.
- Specification: `SERVER-001`'s next minor / FR at implementation
  (extends `FR-038`, `FR-045`, `FR-068`; builds on `FR-070`).
- Requirements: `QPC-FR-001`–`007`.

## Open questions

- **Plan-taken metrics** — `ADR-0073`'s open question, now spanning
  four consumers.
- **The `Ordered` range walk as a `FilteredPage` candidate source** —
  `FILTERED-PAGE-DESIGN.md`'s own open question; would be an adapter
  override of `filtered_page` for `Memory`/`Relation` when the filter is
  on the ordered field. Not this round.
- **Right-side narrowing for `Join`** — none exists to do; right rows
  come from the relation by id. Named so nobody looks for it.

## Change history

- 2026-09-19: initial proposal, design only — the follow-on `ADR-0073`
  option (b) named, taken as the owner's pick for the next round after
  the planner's step one merged (PR #243) and measured. Every claim about
  the current code read from `main` at `6b84b1831` this pass.
