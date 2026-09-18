# Server Query Planner, Step One: An Equality-Index Path for `Request::Query` (Proposed)

- Status: **Proposed** (2026-09-18, `ADR-0073`). Design only — no code in
  this round.
- Date: 2026-09-18
- Related: `ADR-0034`/`docs/design/SERVER-SQL-SELECT-DESIGN.md` (the
  `SELECT` subset and `Request::Query` this round changes the evaluation
  of — its own Non-goals name "a query planner or cost-based optimizer"
  and its own Open questions name "whether `Query` should ever consult
  an existing index (`FilterEq`'s own index...) as a cheap-path
  optimization — the first real step toward the query planner"),
  `ADR-0068`/`docs/design/SERVER-FILTERED-PAGE-DESIGN.md` (the most
  recent round to weigh "shared default vs. index" for a filtered read,
  and the precedent for naming the index's lost advantage plainly),
  `ADR-0059`/`docs/design/SERVER-ORDERED-INDEX-DESIGN.md` (`Ordered`, the
  *range* index this round deliberately does not reach), `ADR-0011`
  (`DomainSchema`/`FieldCapabilities`, whose `filter_eq` flag is the
  planner's only input), `docs/FUTURE-GROWTH.md` ("Path to
  SQLite/DuckDB parity", item 1: "Still absent: any query planner or
  optimizer").
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs`'s
  `dispatch` `Query` arm only — no `Request`/`Response` variant, no
  protocol-version bump, no `ConnectionStore` method added or changed,
  no adapter edit, no client (`src/server/client.rs`, `clients/python`)
  or `src/server/sql.rs` change. Every existing `Query` returns the same
  *set* of rows; see "Data/state and invariants" for the two observable
  differences this round names rather than hides.

## Purpose and scope

`Request::Query` (`ADR-0034`, protocol 8) is answered today by exactly
one strategy, regardless of what the `WHERE` clause says:
`dispatch` validates against the schema, then calls
`ConnectionStore::scan_all()` — every record, every field — and hands
the whole table to `evaluate_query` to filter, project, and truncate
(`src/server/serve.rs`, the `Request::Query` arm and `evaluate_query`;
`SQL-FR-004`/`SQL-FR-006`). `SERVER-SQL-SELECT-DESIGN.md`'s own
Non-goals stated the cost plainly when the subset was first built: "even
a `WHERE <indexed-field> = ...` predicate `Request::FilterEq` could
answer via its existing index pays the same O(n) cost as every other
`Query`." Every SQL round since (`ADR-0035` aggregation, `ADR-0044`
`JOIN`, `ADR-0061` `ORDER BY`, `ADR-0068` `WHERE` + `ORDER BY`) inherited
that Non-goal unchanged and named the planner as separate, later work.

This round takes exactly the first step those documents named and no
other: when a `Query`'s `filter` contains an equality predicate on a
field the domain's own `describe()` reports `filter_eq: true` for,
`dispatch` asks the adapter's existing `filter_eq(field, value)` for the
candidate ids and reads only those records, instead of every record.
Every predicate is then re-evaluated over the fetched rows by the
unchanged `evaluate_query`, so the result *set* is identical to the full
scan's by construction — the index only narrows what is read, never
what is returned.

Scope, exactly: a plan choice in `dispatch` (`QPL-FR-001`); the
candidate fetch (`QPL-FR-002`); the mandatory re-check (`QPL-FR-003`);
the fallback when an adapter's index refuses (`QPL-FR-004`); the
result-set-equality invariant and its per-domain proof (`QPL-FR-005`);
the two named observable differences (`QPL-FR-006`); and zero change to
every other surface (`QPL-FR-007`).

## Non-goals

- **`WHERE id = <uuid>` via `GetById`.** `SERVER-SQL-SELECT-DESIGN.md`'s
  Non-goals mention this as a second cheap path, but read literally
  against the code it is not reachable: `Predicate { field: FieldRef,
  op, value }` (`src/server/protocol.rs`) addresses a *field tag*, and
  no domain's `describe()` exposes the record id as a field —
  `src/server/sql.rs` has no id pseudo-column either. Making `id`
  addressable in a `WHERE` clause is a grammar-and-schema question
  (which tag? which `ScanValue` kind carries a `Uuid`? — none does
  today), not a planner one. Named, not solved; a later round's call.
- **Range predicates via the `Ordered` index** (`ADR-0059`). `Memory`/
  `Relation` hold a sorted index over `updated_at_unix_ms` that could
  answer `WHERE updated_at > …` as a range walk. That index is reached
  today only through `ConnectionStore::page`, has no "every id whose
  key is in this range" primitive, and its `OrderedField` marker is
  not surfaced in `DomainSchema` at all — reaching it from `Query` is a
  second, distinct planner step with its own trait-surface question,
  not bundled here.
- **A cost model or statistics.** This round's rule is "use the index
  whenever the schema says one exists," full stop. That is always at
  least as cheap as the full scan in reads performed: an index lookup
  returns *k* ids and the path then performs *k* `get`s, against the
  scan's *n* — and *k* ≤ *n* always. The one case where it is not a
  win is *k* ≈ *n* (an equality value nearly every record shares),
  where the cost is the same *n* reads plus one hash lookup. No
  selectivity estimate, no histogram, no "is this index worth it"
  decision — those are the cost-based optimizer `docs/FUTURE-GROWTH.md`
  names, and this round is deliberately not it.
- **Combining two indexes.** A `filter` with two eligible equality
  predicates uses one (`QPL-FR-001`'s deterministic choice) and
  re-checks the other over the fetched rows. Intersecting two id lists
  is a real, small optimization left for a round with a reason to want
  it.
- **`Request::Aggregate`, `Request::FilteredPage`, `Request::Join`.** All
  three also start from `scan_all()` and apply the same
  `predicate_matches` filter (`evaluate_aggregate`, the default
  `filtered_page`, `evaluate_join`'s `left_filter`), so all three could
  adopt the identical candidate step. Named as `ADR-0073`'s option (b)
  and left out of (a) so this round changes one request's evaluation
  and proves it, not four.
- **Any change to what an adapter's `filter_eq` means.** `Entity`'s
  `label` lookup is case- and whitespace-insensitive by design
  (`ENT3-FR-005`, `ADR-0040`; `src/server/entity.rs` routes
  `FIELD_LABEL` to `find_by_name`). This round does not "fix" that to
  exact matching — it relies on the re-check (`QPL-FR-003`) to give
  `Query` exact-`Eq` semantics over whatever superset the index hands
  back. `Request::FilterEq`'s own behavior is untouched.
- **Client or wire change.** `src/server/sql.rs`, `SchemaDrivenClient`,
  `clients/python`, `protocol.rs`, `SERVER-002`: all untouched. A client
  cannot ask for or observe the plan.
- **`Ne`, `Lt`, `Le`, `Gt`, `Ge`.** Only `CompareOp::Eq` can use an
  equality index. Every other comparator stays a full scan.

## Context and terminology

Read from `src/server/serve.rs`, `src/server/protocol.rs`, the six
domain adapters under `src/server/`, and `src/generic/` as they stand
at the crate's current head (`SERVER-001` v0.50.0-line, protocol 27
after `ADR-0072`), not assumed:

- **Today's `Query` evaluation** (`serve.rs`, `dispatch`):
  `validate_query(&store.describe(), &select, &filter)` — unknown tag →
  `UnknownField`, kind mismatch or ordering comparator on a `Str`/`Bool`
  field → `Malformed` (`SQL-FR-007`) — then
  `evaluate_query(store.scan_all(), &select, &filter, limit)`.
  `evaluate_query` keeps a row iff *every* predicate `predicate_matches`
  it, projects with `select_fields`, then `truncate(limit)` — "`limit`
  truncates whatever order the input arrived in, not a meaningful
  top-N" (its own doc comment).
- **`scan_all` is not one snapshot.** `MemoryConnectionStore::scan_all`
  (and `Entity`/`Relation`/`Order`/`Employee`/`Reminder`'s, identically)
  is `self.store.all_ids::<R>()` followed by a per-id `self.get(id)`,
  each a separate `read()` acquisition of the store's `RwLock`
  (`GenericProductionStore::all_ids`/`get`), with `filter_map` dropping
  any id whose `get` returns `None`. A record deleted between the id
  listing and its read is silently absent; a record replaced in
  between is read in its new form. This is the consistency class every
  `Query` already has. `Dog`'s `scan_all` is the same shape over
  `ProductionStore`.
- **`ConnectionStore::filter_eq(field, &value) -> Result<Vec<RecordId>,
  ErrorCode>`** — "equality filter on an indexed field.
  `Err(UnknownField)` for a tag this adapter doesn't recognize at all;
  `Err(Unsupported)` for a recognized field with no equality index
  in-process; `Err(Malformed)` if `value`'s variant doesn't match the
  field's real type" (its trait doc). Implemented by every adapter;
  `Dog`'s unconditionally answers `Unsupported`.
- **`DomainSchema` / `FieldCapabilities { filter_eq, scan, update }`**
  (`ADR-0011`): each adapter's `describe()` reports, per field, whether
  `filter_eq` is answerable. Read from each adapter's `describe()` and
  cross-checked against its `filter_eq` arms this design pass:

  | Adapter | `filter_eq: true` fields | `filter_eq` implementation |
  |---|---|---|
  | `Memory` | `category` | `GenericProductionStore::filter_eq::<Memory, CategoryField>` — the generic `HashMap<IndexValue, Vec<Id>>` index |
  | `Entity` | `kind`, `label` | `kind` → the generic index; `label` → `find_by_name` (case/whitespace-insensitive, includes aliases — a **superset** of exact `Eq`) |
  | `Relation` | `subject` | the generic index |
  | `Order` | `status` | the generic index (after `status_from_u32`) |
  | `Employee` | `department` | the generic index (after `department_from_u32`) |
  | `Reminder` | `due_at_unix_ms` | the generic index |
  | `Dog` | none (`breed`/`age` both `false`) | `Err(Unsupported)` unconditionally |

  Every `true` flag has a real arm behind it; every field without an
  arm is `false`. No adapter currently lies in either direction. The
  planner's *only* input is this flag (`QPL-FR-001`); `QPL-FR-004`
  covers the day an adapter does lie.
- **The generic equality index is maintained under runtime writes.**
  `GenericMmapStore::insert`/`replace`/`delete`
  (`src/generic/mmap_store.rs`) each update `self.index` — insert pushes
  the new id under its value; replace removes it from the old value's
  bucket (dropping an emptied bucket) and pushes under the new; delete
  removes it. So the ids `filter_eq` returns reflect every `Insert`/
  `Replace`/`Delete`/`WriteBatch`/session `Commit` that has applied, the
  same way `all_ids` does. `QPL-FR-005` proves this from the outside
  rather than trusting it.
- **`ADR-0068`'s precedent** for the same question one request over:
  `filtered_page`'s default is "`scan_all` → filter → sort-and-page",
  and its design named plainly that the `Ordered` index "gives no speed
  advantage to a *filtered* request this round." That round chose a
  trait method with a shared default so a domain *could* later narrow
  candidates more cheaply. This round is that "later," for `Query`,
  and finds it needs no trait method at all — every adapter already
  exposes the two primitives the narrowing needs (`filter_eq`, `get`),
  so the plan can live in `dispatch` with zero adapter edits (see
  "Proposed shape").
- **Measured precedent for why an index path matters at all**:
  `ADR-0059` measured an unindexed `Page` at 292 ms per page of 50 at
  100K `Memory` records and the `Ordered` index at 152–160 µs — the
  cost of materializing every record versus touching only the ones
  returned. `Query` on an indexed equality pays the former shape today.
  This design-only round makes no new measurement; the implementation
  round must (acceptance criterion 5).

## Requirements

- `QPL-FR-001` **Plan choice, schema-driven, deterministic.** After
  `validate_query` succeeds and before any store read, `dispatch`
  chooses one of two plans from `filter` and `store.describe()` alone:
  `IndexEq(i)` if there exists a predicate `filter[i]` with `op ==
  CompareOp::Eq` whose `field`'s `FieldDescriptor` reports
  `capabilities.filter_eq == true` — the **first** such predicate in
  `filter`'s wire order when more than one qualifies; otherwise
  `FullScan`. An empty `filter` is always `FullScan`. The choice depends
  on nothing else — not table size, not the literal's value, not any
  prior request.
- `QPL-FR-002` **Candidate fetch.** `FullScan` reads `store.scan_all()`
  exactly as today. `IndexEq(i)` calls
  `store.filter_eq(filter[i].field, &filter[i].value)`; on `Ok(ids)` it
  reads `store.get(id)` for each id in the returned order, keeping
  `(id, fields)` for every `Some` and silently skipping every `None`
  (a record deleted between the index lookup and its read — the
  identical drop `scan_all`'s own `filter_map` already performs).
- `QPL-FR-003` **Every predicate is re-checked; the index only
  narrows.** Whichever plan produced the candidate rows, they pass
  through the **unchanged** `evaluate_query(rows, &select, &filter,
  limit)` — including `filter[i]` itself. The index is never trusted to
  have applied its own predicate exactly: `Entity`'s `label` index
  returns case-/whitespace-insensitive and alias matches that exact
  `predicate_matches` rejects, and the re-check is what makes `Query`'s
  `Eq` mean the same thing on every plan. No second filter/project/
  limit implementation is written; `SQL-FR-006`'s "written once, not
  duplicated" property is preserved exactly.
- `QPL-FR-004` **Index refusal falls back, never surfaces.** If
  `filter_eq` returns `Err(_)` for a field the schema claimed
  `filter_eq: true` for (a contract mismatch between an adapter's
  `describe()` and its `filter_eq` arms — none exists in the crate
  today, per the table above), `dispatch` executes `FullScan` for that
  request and returns the correct result. `Query`'s error surface
  (`SQL-FR-007`: `UnknownField`/`Malformed` from validation only) is
  unchanged — a planner-internal refusal is never a client-visible
  error. The implementation round adds a `debug_assert!`/test-fixture
  check that the shipped adapters never take this branch, so the
  fallback is a safety net, not a load-bearing path.
- `QPL-FR-005` **Result-set equality, proven per domain.** For every
  `Query` on every domain, the multiset of `(id, projected fields)`
  rows returned under `IndexEq` equals the multiset the same request
  returns under `FullScan` with `limit: None`, at any instant with no
  concurrent write. Proven, not asserted: an integration test per
  shipped indexed field (the seven in the table above) issues the
  equality `Query` over a real socket and cross-checks it against the
  full scan's own filtered result — before and after runtime `Insert`,
  `Replace` (moving a record into and out of the indexed value), and
  `Delete` — and `Entity`'s `label` case additionally proves a literal
  differing only in case/whitespace/alias from the stored label returns
  **no** rows via `Query` (exact `Eq`) even though `Request::FilterEq`
  returns the record.
- `QPL-FR-006` **Two observable differences, named.** (1) Row *order*
  within `Response::Rows` may differ between the two plans (index
  bucket order vs. `all_ids` order). Both are "unspecified order"
  under `SQL-FR-004`/`SQL-FR-006`'s existing contract; no test in the
  crate pins `Query` row order, and this round adds none. (2) With
  `limit: Some(n)` and more than *n* matching rows, *which* *n* rows
  are returned may differ from what the same request returned before
  this round. Permitted by the existing "truncates that same
  unspecified order, not a meaningful top-N" wording; still a behavior
  change a client relying on accidental `all_ids` order would notice.
  Recorded in `SERVER-001`'s change history at implementation time,
  not left for a reader to discover.
- `QPL-FR-007` **Everything else unchanged.** No `ConnectionStore`
  method added, removed, or re-signatured; no adapter file edited; no
  `Request`/`Response`/`ErrorCode` variant; no `PROTOCOL_VERSION` bump;
  `src/server/sql.rs`, `src/server/client.rs`, `clients/python/**`,
  `SERVER-002` untouched. `Request::Aggregate`, `FilteredPage`, `Join`,
  and `Request::FilterEq` itself evaluate exactly as before. `Dog`
  (`filter_eq: false` on every field) always plans `FullScan` and is
  byte-for-byte unaffected.

## Considered options

- **(a) An equality-index candidate step in `dispatch`, schema-driven,
  re-checked, for `Request::Query` only — recommended, as scoped
  above.** One new private function beside `evaluate_query`; zero
  trait, adapter, wire, or client change; result set identical by
  construction (`QPL-FR-003`); consistency class identical to today
  (`QPL-FR-002`). Real, named cost: two observable order/`LIMIT`
  differences (`QPL-FR-006`), and a `filter` whose eligible equality
  value is shared by nearly every record gains nothing (one hash
  lookup more than today, the same *n* reads).
- **(b) The same step, applied to every `scan_all`-then-filter
  consumer — `Query`, `Aggregate`, the default `filtered_page`, and
  `Join`'s left side.** One shared `candidate_rows(store, filter)`
  helper replacing four `scan_all()` calls. Larger win per line of
  code, but four evaluation paths change in one round, `Join`'s left
  filter has its own `left_filter`/`right_filter` shape to thread, and
  `filtered_page` is a trait method with a default that adapters may
  already override — every one of them needs `QPL-FR-005`'s own
  per-domain cross-check. A legitimate follow-on once (a) has proven
  the step on the simplest consumer.
- **(c) Decline.** `Query` stays an unconditional full scan;
  `docs/FUTURE-GROWTH.md`'s "any query planner or optimizer" line
  stays exactly as written. Zero risk. A caller who knows a field is
  indexed can already call `Request::FilterEq` and then `GetById`
  per id by hand — which is precisely the plan this round would
  automate server-side, minus the round trips.

The owner's shorthand: **(a)** `Query` only, as proposed; **(b)** the
same step for `Query`/`Aggregate`/`FilteredPage`/`Join` together;
**(c)** decline.

## Proposed shape

`src/server/serve.rs` only:

- A private `enum QueryPlan { FullScan, IndexEq(usize) }` and
  `fn plan_query(schema: &DomainSchema, filter: &[Predicate]) -> QueryPlan`
  — pure, `QPL-FR-001`'s rule verbatim, unit-tested in isolation like
  `evaluate_query` already is.
- A private `fn query_candidates<S: ConnectionStore + ?Sized>(store: &S,
  plan: QueryPlan, filter: &[Predicate]) -> Vec<(RecordId,
  Vec<(FieldRef, ScanValue)>)>` — `QPL-FR-002` and `QPL-FR-004`: the
  `scan_all()` call or the `filter_eq` + per-id `get` loop with the
  `Err(_) => scan_all()` fallback.
- The `Request::Query` arm becomes: validate (unchanged) → `plan_query`
  → `query_candidates` → `evaluate_query` (unchanged). Three lines
  where today there is one.

Why not a `ConnectionStore` trait method with a default, the
`filtered_page` shape? Because no adapter needs to override anything:
the narrowing composes two methods every adapter already implements,
and the schema already tells `dispatch` when to use them. A trait
method would add surface (`QPL-FR-007` forbids it) to enable an override
no domain has a reason to write. If a later round wants an adapter-
specific candidate strategy (the `Ordered` range walk, say), it can
introduce the trait method then, with a default that calls exactly this
function.

`FixtureStore` (the `serve.rs` test double) gains a configurable
in-memory equality index and a matching `describe()` flag, so the plan
choice, the re-check over a deliberately-superset index, and the
`Err(_)` fallback are each unit-testable without a real domain.

## Data/state and invariants

- **Result-set invariant** (`QPL-FR-005`): for a `filter` with no
  concurrent writer, `IndexEq` and `FullScan` produce the same multiset
  of `(id, projected fields)` before `limit`. Holds because (i)
  `filter_eq(f, v)` returns a superset of `{ id | predicate_matches(
  get(id), Eq(f, v)) }` for every shipped adapter — exact for the
  generic `HashMap` index, a normalized superset for `Entity::label` —
  and (ii) `evaluate_query` re-applies `Eq(f, v)` and every other
  predicate exactly. A `filter_eq` that returned a **subset** of the
  exact matches would break the invariant silently; no shipped adapter
  does, and `QPL-FR-005`'s tests are what would catch one.
- **Consistency class, unchanged**: the index lookup and each `get` are
  separate lock acquisitions, exactly as `all_ids` and each `get` are
  today. A concurrent `Delete` between them drops the row from the
  result (as today); a concurrent `Replace` that moves a record *out*
  of the indexed value between the lookup and its read is caught by the
  re-check and dropped; one that moves a record *into* the value after
  the lookup is missed (as the full scan would miss a record inserted
  after `all_ids`). No new anomaly is introduced and none is removed.
- **Session posture, unchanged**: `Query` "always reads committed
  state, unconditionally" (`SQL-FR-009`, `ISO-FR`/`MVCC2-FR-006`'s
  intercepts key on `GetById` alone), and `store.get` here is the
  adapter's own committed read, not the session intercept — a `Query`
  inside a read-your-writes or MVCC session behaves identically on
  either plan.

## Errors, failure, recovery, and observability

No new `ErrorCode`, no new failure mode visible to a client: every
rejection `Query` can produce still comes from `validate_query` before
any read (`SQL-FR-007`), and a planner-internal `filter_eq` refusal is
absorbed by the `FullScan` fallback (`QPL-FR-004`). Lock poisoning
panics exactly where it does today (`LOCK_POISONED` in
`GenericProductionStore`). Nothing to recover — a read throughout.

Observability: the plan taken is not reported on the wire, in the
access log, or in `Request::Metrics` this round. A per-plan counter
(`queries_index_path_total`/`queries_full_scan_total`) would be a
one-line `ServerMetrics` addition and is named as an open question,
not bundled — `ADR-0064`'s bounded counter set was a deliberate choice
and growing it deserves its own sentence in that ADR.

## Security, privacy, and compatibility

A read, gated exactly as `Query` is today (authentication only; never
overlaid or read-set-tracked by a session). The index path reads
strictly fewer records than the scan and returns the same set, so no
record a client could not already obtain becomes obtainable. No wire
format change; no protocol-version bump; a pre-existing client of any
version observes only `QPL-FR-006`'s two order-level differences.
Audit and access-log entries are unchanged — one `Query` request is
one entry regardless of plan.

## Acceptance criteria

1. `plan_query` unit tests: empty filter → `FullScan`; a single
   eligible `Eq` → `IndexEq(0)`; two eligible `Eq`s → `IndexEq` of the
   first; an `Eq` on a `filter_eq: false` field → `FullScan`; a `Ne`/
   `Lt`/`Le`/`Gt`/`Ge` on an indexed field → `FullScan`; an ineligible
   predicate before an eligible one → `IndexEq` of the eligible one's
   index.
2. `query_candidates` against `FixtureStore`: `IndexEq` reads only the
   index's ids (asserted via the fixture's read counter, not inferred);
   a fixture index deliberately returning a superset yields, after
   `evaluate_query`, exactly the exact-match rows; a fixture index
   returning `Err(Unsupported)` despite `filter_eq: true` falls back to
   the full scan with the identical result and no error.
3. Integration (`tests/server_sql_integration.rs`, real socket, real
   SQL text via `SchemaDrivenClient::query`): for each of the seven
   shipped indexed fields, `SELECT * FROM <table> WHERE <field> = <v>`
   returns the same row set as the full scan's filtered result, before
   and after an `Insert` into the value, a `Replace` moving a record
   into and another out of the value, and a `Delete` of a matching
   record; `Entity`: `WHERE label = 'grace hopper'` against a stored
   `Grace Hopper` returns zero rows while `Request::FilterEq` returns
   the record; a two-predicate filter (`kind = … AND mention_count >
   …`) returns exactly the rows satisfying both.
4. Every pre-existing `Query`/SQL test passes unmodified — none pins
   row order or a `LIMIT` subset on an indexed-equality filter (checked
   this design pass: the only `LIMIT`-bearing filtered queries in
   `tests/server_sql_integration.rs` are `ORDER BY` ones, which compile
   to `Page`/`FilteredPage`, not `Query`); `Dog`'s `Query` tests are
   byte-for-byte unaffected.
5. Measured, recorded in `RESULTS.md` (the `ADR-0059` precedent): at
   100K `Memory` records with `category` spread over ≥ 100 values, a
   `WHERE category = <v>` `Query` under `IndexEq` versus the same
   request with the planner forced to `FullScan`, wall-clock per
   request — the implementation round reports the numbers; this
   design-only round predicts the shape (O(*k*) reads vs. O(*n*)) and
   nothing more.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`
(the `serve.rs` unit tests plus `tests/server_sql_integration.rs`,
extended — that target is `required-features = ["server", "research"]`
because it covers `Order`/`Employee`). Independent review: this design
was written by Claude while Codex's sandbox could not spawn processes
(re-confirmed this session — `exec_command … timed out connecting
runner pipe-in`); per this crate's host-takeover convention a fresh
Codex session should review both this design and its implementation
once that is restored, and the implementation round's own inspection
must not cite this session as having covered it.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-DESIGN`, `SERVER-QUERY-PLANNER`.
- Decision: `ADR-0073`.
- Specification: `SERVER-001`'s next minor / FR at implementation
  (extends `FR-037`'s `Request::Query`).
- Requirements: `QPL-FR-001`–`007`.

## Open questions

- **Metrics for the plan taken** — a two-counter addition to
  `ServerMetrics` (`ADR-0064`) so an operator can see whether queries
  are hitting the index; named, not bundled.
- **`Ordered` as a range index for `Query`** — the second planner step;
  needs an "ids in key range" primitive `Ordered` does not expose and a
  way for `DomainSchema` to say a field is range-indexed. Not decided
  here.
- **`WHERE id = …`** — a grammar/schema question (no `ScanValue` kind
  carries a `Uuid`; no adapter exposes `id` as a field), not a planner
  one. Not decided here.
- **Option (b)'s four-consumer generalization** — the natural next
  round if (a) proves out.
- **Tie-break when two predicates are eligible** — `QPL-FR-001` picks
  the first in wire order for determinism, not selectivity. A client
  that knows one index is far more selective can order its `WHERE`
  clause accordingly; whether the server should ever choose instead is
  exactly the cost-model question this round declines.

## Change history

- 2026-09-18: initial proposal, design only. Selected by the owner
  ("let's tackle sql" → the cheap-path planner slice, over `OR`/
  parentheses, a server-side `Request::Sql { text }`, and `ORDER BY` +
  `JOIN`/`GROUP BY`) as the smallest, most concretely pre-scoped step
  on `docs/FUTURE-GROWTH.md`'s SQL-parity item 1 that every prior SQL
  design doc had already named as the next one. Every claim about the
  current code above was read from `main` at `6c6f8dfc9` (post-`ADR-
  0072`, protocol 27) during this pass, including the discovery that
  the "`WHERE id = …` via `GetById`" cheap path `SERVER-SQL-SELECT-
  DESIGN.md` mentions is not expressible in today's `Predicate`.
