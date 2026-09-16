# Server Filtered Ordered Page: `Request::FilteredPage` — Combining `WHERE` with `ORDER BY` (Implemented)

- Status: **Accepted as designed and implemented** (2026-09-15,
  `ADR-0068`) — implemented in the same session the design was
  proposed. See `ADR-0068`'s own "Acceptance and implementation"
  section for the full implementation record, including two real
  corrections found while implementing (`FPG-FR-004`'s self-
  contradictory text; the Python client Non-goal below, corrected).
- Related: `docs/FUTURE-GROWTH.md`'s "Path to SQLite/DuckDB parity," item
  1 ("Still absent: ... `ORDER BY` combined with a filter"), `ADR-0061`
  (`SERVER-SQL-ORDER-BY-DESIGN.md`, whose own option (b) — "thread a
  `WHERE` filter through to a new, filter-bearing `Page` variant... a
  real evaluation-strategy design question this crate has never
  answered" — is this round's direct trigger), `ADR-0055`
  (`SERVER-PAGE-DESIGN.md`, `Request::Page`, protocol 20, whose own
  Non-goals never even named filtering), `ADR-0059`
  (`SERVER-ORDERED-INDEX-DESIGN.md`, the memory-only `Ordered<S, R,
  Marker>` index behind `Memory`/`Relation`'s `page`, which carries no
  field data and so cannot test a predicate without an extra fetch per
  candidate), `ADR-0034` (`SERVER-SQL-SELECT-DESIGN.md`, `Request::Query`,
  `Predicate`/`CompareOp`, `validate_predicate`/`predicate_matches` —
  the exact filter-evaluation logic this round reuses unchanged).

## Purpose and scope

`docs/FUTURE-GROWTH.md`'s SQL-parity checklist, item 1, names the gap
directly: *"`ORDER BY` combined with a filter"* is still absent.
`ADR-0061` built `ORDER BY` itself (compiled to `Request::Page`,
protocol 20, reused unchanged) but excludes any query that also has a
`WHERE` clause at parse time (`OrderByWithFilter`) — `Request::Page`
takes no filter argument at all. `ADR-0061`'s own Considered options
named the follow-on precisely: a new, filter-bearing `Page` shape,
gated on **one real, unanswered evaluation-strategy question**: does
filtering happen before or interleaved with the ordered walk, and what
does that cost look like on a domain with no index over the ordered
field at all (five of this crate's six domains today)?

This round proposes closing that gap: a client whose query needs both
`WHERE` and `ORDER BY` — e.g. `SELECT * FROM memory WHERE category =
'preference' ORDER BY updated_at_unix_ms LIMIT 10` — compiles to one
request instead of being refused at parse time.

## Non-goals

- **A query planner or optimizer.** This round adds one more primitive
  with a fixed, always-correct evaluation strategy (see Decision) —
  not a cost-based chooser between strategies. `docs/FUTURE-GROWTH.md`'s
  own broader "no query planner or optimizer" gap stays exactly as open
  as it is today.
- **Pushing the filter into `Memory`/`Relation`'s `Ordered` index.**
  The `BTreeSet<(key, id)>` the index walks carries no other field
  data — testing a predicate against a candidate needs a `get()` the
  unfiltered fast path never pays. This round's default evaluation
  strategy does not attempt that optimization; see Decision and
  Consequences for the cost this leaves on the table, named plainly.
- **Descending order, a second orderable field, or a range filter on
  the order-by field itself treated specially.** All three are
  `Request::Page`'s own already-declined Non-goals (`ADR-0055`,
  `ADR-0059`); this round changes none of them. A range on the order-by
  field is expressible as an ordinary `Predicate` in the new `filter`
  list, evaluated the same as every other predicate — not given a
  cheaper index-range-scan path.
- **A size cap on `filter`.** `Request::Query`'s own `filter:
  Vec<Predicate>` has never had one; this round does not introduce the
  first one for this shape either — the SQL grammar's own `AND`-chain
  parsing is the practical bound, matching `Query`'s precedent exactly.
- ~~A Python client change.~~ **Corrected during implementation: this
  was wrong.** `ADR-0061` never gave the Python reference client a SQL
  *compiler*, but every wire-touching round since `ADR-0057`
  (`count_edges`) has given it a matching raw request-level method
  (`page`, `write_batch`, `metrics`, `backup`, `fetch_snapshot`, ...) —
  `SERVER-002` §10's own changelog confirms this every time. Corrected:
  `clients/python/` gains `Client.filtered_page`, exercised by
  `driver.py` and `tests/server_python_client.rs`, matching precedent.

## Context

Read directly from `src/server/{protocol,serve,sql,client}.rs`,
`src/generic/{traits,query,store}.rs`, `ADR-0055`/`ADR-0059`/`ADR-0061`
and their design docs, and `docs/traceability/TRACEABILITY.md`, as they
stand at `SERVER-001` v0.55.0 / `PROTOCOL_VERSION = 25`:

- **`Request::Page { order_by, after, limit }` carries no filter field,
  and cannot gain one in place.** `SERVER-002`'s own compatibility
  rule 1 is absolute: *"existing indices, fields, and struct layouts
  never change."* `bincode`'s struct encoding is positional (field
  order, no tags) — appending a field to `Page`'s existing struct
  would silently re-encode every already-pinned `Request/Page` golden
  vector differently, breaking every prior protocol version's own
  fixture line. `ADR-0061`'s own phrasing — *"a new, filter-bearing
  `Page` variant (or an optional field on the existing one)"* — is
  read here as two names for the same one legal shape: a **new**
  request variant. Mutating `Page` in place is not actually available
  under this crate's own wire contract.
- **`Request::Query { select, filter, limit }` already carries
  everything a filter needs.** `filter: Vec<Predicate>`,
  `Predicate { field: FieldRef, op: CompareOp, value: ScanValue }`, and
  the two functions that give it meaning — `validate_predicate`
  (`UnknownField` for an unknown tag, `Malformed` for a kind mismatch
  or an ordering comparator against a non-`U32`/`I64` field) and
  `predicate_matches` (the actual per-row test) — are the **one** place
  this crate evaluates a predicate today, also reused by
  `Request::ReplaceIf`'s guard. This round reuses both unchanged; no
  second predicate evaluator is introduced anywhere.
- **`Request::Query` itself is already unconditionally a full scan, with
  no ordering promise.** `evaluate_query`'s own doc comment: *"`limit`
  truncates whatever order the input arrived in, not a meaningful
  top-N (no `ORDER BY`)."* So a client wanting sorted, filtered,
  limited rows today has no correct way to get them in one request at
  all — `Query` gives filtered-but-unordered, `Page` gives
  ordered-but-unfiltered, and there is no third shape.
- **`Request::Page`'s own default evaluation (`page_by_scan`) is a
  full scan today, for every domain that doesn't override it.**
  `ConnectionStore::page`'s default calls `page_by_scan`, which calls
  `Self::page_keys` (default: `scan_all()` then extract each record's
  sort key — **already materializing every field of every record**),
  then `page_ids` (a pure `(key, id)` selection: `select_nth_unstable`
  then a sort of just the winning page), then `Self::get` for only the
  winning ids. Only `Memory`/`Relation` override `page` itself (via
  `Ordered<S, R, Marker>`, `ADR-0059`); `Entity` overrides only
  `page_keys` for a cheaper key-only read (no full field
  materialization); `Reminder`/`Dog`/`Order`/`Employee` use the pure
  scan-based default for both. Every one of the six domains answers an
  **unfiltered** `Page` request today — `Unsupported` never occurs for
  it currently.
- **The `Ordered` index (`Memory`/`Relation`) carries no field data at
  all.** `Ordered<S, R, Marker>` is a `BTreeSet<(R::Key, R::Id)>` built
  at open and kept exact through every write; `PageBy::page_by` is a
  pure range walk (`Excluded(after)..Unbounded`, `.take(limit)`)
  mapping `(key, id) -> id`. Testing a predicate against a candidate
  drawn from this index needs a separate `get()` per candidate — a
  cost the unfiltered fast path never pays today (it calls `get()`
  only for the final `limit` winners, having already picked them by
  key alone).
- **`ADR-0059` already named a *range* filter on the order-by field
  itself as a future open question — never an arbitrary predicate on a
  different field.** Confirmed by reading `ADR-0059` directly: its own
  Non-goals list "a range filter on the wire" alongside descending
  order and SQL `ORDER BY` as three related-but-separate future
  questions, none built. This round is the first to consider an
  arbitrary `WHERE` predicate (any field, any of the four comparators)
  combined with the walk.
- **`sql.rs`'s `WHERE` grammar is already complete and fully reusable,
  independent of `ORDER BY` parsing.** `condition := (qualifier.)?ident
  comparator literal`, `AND`-chained into `ParsedQuery::conditions:
  Vec<ParsedCondition>`, parsed *before* `order_by_clause` in the same
  token stream. `validate_order_by`'s three checks —
  `!query.conditions.is_empty()` → `OrderByWithFilter`,
  `query.join.is_some()` → `OrderByWithJoin`, an aggregate/`GROUP BY`
  column → `OrderByWithAggregate` — are independent parse-time
  refusals; removing the first one is a one-line change once the
  compile step downstream knows what to do with `conditions`.
- **`SchemaDrivenClient::query_ordered`'s pagination loop already
  handles "fewer rows than asked for signals exhaustion."** With
  `LIMIT` present: one `fetch_page` call. With `LIMIT` absent: loop
  `fetch_page` with `ORDER_BY_PAGE_CHUNK = 1_000`, advancing `after` to
  the last row's `(order_by value, id)`, until a page comes back
  shorter than the chunk size. This signal is orthogonal to filtering
  — it already means "the server has told me there is nothing more,"
  not "I asked for N and got exactly N" — so a filtered `Page`
  response that returns fewer than `limit` **matching** rows because
  the underlying table is exhausted needs no new client-side signal at
  all.

## Requirements

- `FPG-FR-001` **A new `Request::FilteredPage { order_by: FieldRef,
  after: Option<(ScanValue, RecordId)>, limit: u64, filter:
  Vec<Predicate> }`, answered `Response::Rows`** (reused unchanged —
  the identical shape `Request::Page`/`Request::Query` already answer
  with). `PROTOCOL_VERSION` 25 → 26.
- `FPG-FR-002` **Validated exactly as `Page` and `Query` already are,
  composed, not reinvented**: `validate_page`'s existing four checks
  (`order_by` known/orderable, `after`'s kind, `limit != 0`) plus
  `filter`'s own predicates each checked via the existing
  `validate_predicate` — no new validation function.
- `FPG-FR-003` **One shared default evaluation strategy, correct for
  every domain with zero adapter overrides required this round**:
  filter-then-order — `scan_all()`, keep only rows `predicate_matches`
  every filter predicate for (reusing `evaluate_query`'s own filter
  step unchanged), then apply `Page`'s existing `page_key`/`page_ids`
  selection to the filtered subset. Every domain answers a
  `FilteredPage` request through this one default; no
  `ConnectionStore::filtered_page` override exists on any adapter this
  round (see Decision for why, and the named cost).
- `FPG-FR-004` **`ConnectionStore::filtered_page`, a trait method with
  a *working* default** (`page`'s own precedent — not `backup`/
  `fetch_snapshot`/`compact`'s `Unsupported`-by-default opt-in-capability
  shape, since this round's default correctly answers every domain, not
  none): `scan_all()` → filter via `predicate_matches` → `page_rows`.
  A trait method rather than inline `dispatch` logic (unlike `Query`)
  so a future round can still override it for a domain with a cheaper
  path, the same extensibility `page`/`page_keys` already have.
- `FPG-FR-005` **`sql.rs`'s `OrderByWithFilter` exclusion removed**;
  `WHERE` + `ORDER BY` compiles to `Request::FilteredPage` instead of
  the parse-time refusal. `OrderByWithJoin`/`OrderByWithAggregate`
  unchanged — still refused, for the reasons `ADR-0061` already gave.
- `FPG-FR-006` **`SchemaDrivenClient`'s ordered-query compile path
  reused, not duplicated**: the existing `query_ordered`/pagination-loop
  shape gains one field (`filter`) threaded through to a new
  `fetch_filtered_page`, mirroring `fetch_page` exactly; no new
  predicate-evaluation code on the client — the server is the one
  place a predicate is ever tested, for this request as for `Query`.
- `FPG-FR-007` **Gated like `Page`**: `Malformed` below protocol 26,
  the identical version-gate shape every prior wire addition uses;
  gated as a **read** (`Query`/`Page`'s own precedent) — no
  `SessionOpen` gate, no `ReadOnly` restriction.

## Considered options

- **(a) `Request::FilteredPage`, a new request variant, filter-then-order
  evaluated via one shared default (`scan_all` → filter → sort-and-page)
  — recommended, as scoped above.** Closes the gap fully: one request,
  correctly ordered *and* filtered *and* limited results, in one round
  trip (or one chunked loop for an unbounded `LIMIT`). Reuses
  `validate_predicate`/`predicate_matches` — the crate's one existing
  predicate evaluator — unchanged; no second copy of comparator logic
  anywhere, client or server. Real, named cost: for `Memory`/`Relation`,
  a filtered request loses the `Ordered` index's speed advantage this
  round — it falls back to the same full-scan-then-sort cost every
  other domain already pays for an unfiltered `Page`, since the shared
  default does not consult the index at all. This is a **regression
  only relative to what an indexed domain's *unfiltered* `Page` already
  achieves** — the *unfiltered* fast path is completely untouched by
  this round (`Request::Page` itself gains no field, no behavior
  change) — and it is no worse, in the worst case, than what
  `Request::Query` already costs on every domain today for the
  identical filter. A protocol bump (25 → 26) and one new
  request/dispatch arm are the wire cost.
- **(b) Client-side filtering over the existing, unfiltered
  `Request::Page`.** Remove `OrderByWithFilter`, compile `WHERE` +
  `ORDER BY` to the exact same unfiltered `fetch_page` loop `ADR-0061`
  already built, and test each returned page's rows against a **new,
  second** predicate evaluator written client-side in
  `src/server/client.rs` (this crate's `predicate_matches` is
  `serve.rs`-private, `server`-feature-gated; the `client`-feature-only
  build cannot reach it, so a filtered client needs its own copy of
  `CompareOp`/`ScanValue` comparison logic — real, new duplication, not
  reuse). Zero wire change, zero protocol bump, zero server change —
  matching `ADR-0061`'s own minimal-footprint precedent most closely.
  Real, named cost, worse than (a) in two ways: every non-matching row
  the walk visits is still sent over the wire before being discarded
  client-side (bandwidth (a) never pays, since the server filters
  before responding), and the crate now carries two independently
  maintained predicate evaluators that must agree on every comparator
  and kind-mismatch edge case forever, or a filtered `Query` and a
  filtered `ORDER BY` silently disagree on the same `WHERE` clause.
- **(c) Decline entirely this round.** `ORDER BY` + `WHERE` stays
  refused at parse time exactly as `ADR-0061` left it;
  `docs/FUTURE-GROWTH.md`'s gap stays open. Zero cost, zero risk. A
  caller needing both today already has two ways to work around it
  without this round: fetch an unfiltered ordered page and filter
  client-side by hand (exactly what (b) would automate), or fetch a
  filtered, unordered `Query` result and sort it client-side (viable
  when the whole filtered result set is known to be small).

The owner's shorthand: **(a)** the new request variant, server-side
filtering, as proposed; **(b)** client-side filtering over the existing
`Page`, zero wire change; **(c)** decline, `ORDER BY` + `WHERE` stays
unavailable.

## Proposed shape

`src/server/protocol.rs`: `Request::FilteredPage { order_by: FieldRef,
after: Option<(ScanValue, RecordId)>, limit: u64, filter:
Vec<Predicate> }` — `Page`'s three existing fields plus `filter`,
answered `Response::Rows` (reused, no new response variant).
`PROTOCOL_VERSION` 25 → 26.

`src/server/serve.rs`:

- `ConnectionStore::filtered_page(&self, order_by, after, limit,
  filter: &[Predicate]) -> Result<Vec<PageRow>, ErrorCode>`, default:

  ```text
  fn filtered_page(&self, order_by, after, limit, filter) -> Result<Vec<PageRow>, ErrorCode> {
      let rows = self.scan_all();
      let filtered: Vec<_> = rows
          .into_iter()
          .filter(|(_, fields)| filter.iter().all(|p| predicate_matches(fields, p)))
          .collect();
      Ok(page_rows(filtered, order_by, after, limit))
  }
  ```

  reusing `evaluate_query`'s own filter predicate (`predicate_matches`,
  unchanged) and `Page`'s own row-selection helper (`page_rows` or the
  equivalent already backing `page_by_scan`, adjusted to take an
  already-filtered `Vec<PageRow>` instead of a full scan) — no new
  comparator or sort-key logic.
- `validate_filtered_page(schema, order_by, after, limit, filter) ->
  Result<(), ErrorCode>`: calls the existing `validate_page` checks
  plus `filter.iter().try_for_each(|p| validate_predicate(schema, p))`
  — a thin composition, not a new rule set.
- A `Request::FilteredPage` dispatch arm: `validate_filtered_page(...)`
  then `store.filtered_page(order_by, after, limit as usize, &filter)`
  → `Response::Rows` or `err_response`. Gated `Malformed` below
  protocol 26 in `handle_connection`, the `Page`/`Backup` precedent
  exactly; a read, so no `SessionOpen` gate.

`src/server/sql.rs`: `validate_order_by` drops its
`!query.conditions.is_empty()` check; the compile step (currently
`query_ordered`'s own trigger condition) gains a branch: `order_by:
Some(_)` + non-empty `conditions` compiles `conditions` into
`Vec<Predicate>` (reusing whatever conversion `Query`'s own `WHERE`
compile step already does — the same `ParsedCondition -> Predicate`
step, not a new one) and calls the new client method instead of
`query_rows`/`query_ordered`.

`src/server/client.rs`: `fetch_filtered_page(order_by, after, limit,
filter) -> Result<Vec<QueryRow>, ClientError>`, the exact structure
`fetch_page` already has plus one more field on the wire request; the
existing chunked-loop-when-`LIMIT`-is-absent shape in `query_ordered`
is generalized to call whichever of `fetch_page`/`fetch_filtered_page`
applies, both signaling exhaustion identically (a page shorter than
`ORDER_BY_PAGE_CHUNK`).

## Data/state and invariants

- `FilteredPage`'s result is a strict subset-and-reorder of what
  `Query` with the identical `filter` (and no `ORDER BY`) would have
  returned, unordered — `predicate_matches` is called with the
  identical arguments either way, so the *set* of matching records is
  provably identical; only the order and the pagination differ.
- The shared default's cost is bounded by one full `scan_all()` per
  request — no worse, in the worst case, than `Request::Query` already
  costs today for the identical filter, on every domain. A highly
  selective filter does not change this ceiling (the default already
  visits every record regardless of how many match); it only changes
  how much of the *filtered* subset gets thrown away by `page_rows`'s
  own after-cursor/limit selection.
- No adapter's `Ordered` index is read, written, or otherwise touched
  by this round — the unfiltered `Page`/`Ordered` fast path is
  provably unaffected, since `Request::Page`'s own struct, dispatch
  arm, and every adapter override of `page`/`page_keys` are untouched
  by this design.

## Errors, failure, recovery, and observability

- An unknown `order_by` tag, a non-`U32`/`I64` `order_by` kind, a
  cursor value of the wrong kind, or `limit == 0`: the identical
  `validate_page` refusals `Page` already gives, `UnknownField`/
  `Malformed` before any scan.
- An unknown filter field tag: `UnknownField`, from `validate_predicate`
  — the identical refusal `Query`'s own filter already gives for the
  same input.
- A filter value of the wrong kind, or an ordering comparator
  (`Lt`/`Le`/`Gt`/`Ge`) against a non-`U32`/`I64` field: `Malformed`,
  from `validate_predicate` — again, `Query`'s own existing rule,
  unchanged.
- Below protocol 26: `Malformed`, the version-gate precedent every
  prior wire addition already follows.

## Security, privacy, and compatibility

- No new attack surface: `FilteredPage` is a **read**, gated exactly as
  `Query`/`Page` already are (any authenticated class, no `ReadOnly`
  restriction) — it discloses nothing a `Query` with the identical
  filter could not already disclose, just in a different (sorted,
  paginated) shape.
- Wire, append-only, hard-to-reverse-once-shipped: `PROTOCOL_VERSION`
  bump, one new request variant. Exactly the class of decision
  `WORKFLOW.md` requires design-first, owner-accepted before
  implementation — the same tier `ADR-0055`/`ADR-0061` themselves were
  treated at, not `ADR-0065`'s heavier "new attack-surface category"
  tier (this round introduces no new write, no new credential, no new
  filesystem surface).

## Acceptance criteria

1. `WHERE` + `ORDER BY` in one SQL query compiles to
   `Request::FilteredPage` and returns rows sorted ascending by the
   `ORDER BY` field, filtered by every `WHERE` predicate, honoring
   `LIMIT` — matching, row for row, what a client-side filter-then-sort
   over the equivalent `Query` result would produce.
2. `FilteredPage`'s result set (ignoring order) is identical to what
   `Query` with the identical `filter` and no `select`/`limit`
   restriction would return, for a real seeded table — the "same
   predicate, same answer" cross-check.
3. Every existing `validate_page`/`validate_predicate` refusal still
   fires on the identical malformed input, through the new request.
4. `FilteredPage` below protocol 26 is `Malformed`; a version-10-style
   hand-negotiated connection cannot send it.
5. `Request::Page` (unfiltered) and every existing `Page`-related test
   are unaffected — the fast `Ordered`-index path for `Memory`/`Relation`
   still answers an unfiltered `Page` exactly as fast as before this
   round (no regression to already-shipped behavior).
6. An `ORDER BY` + `JOIN` or `ORDER BY` + `GROUP BY`/aggregate query is
   still refused at parse time exactly as `ADR-0061` left it — only the
   `WHERE` exclusion is lifted.

## Verification plan

`cargo test --all-features` (new `sql.rs` unit tests for the lifted
exclusion and the `ParsedCondition -> Predicate` compile step; new
`tests/server_sql_integration.rs` cases — a filtered ordered round
trip matching `page()` plus a client-side filter, a no-`LIMIT` filtered
multi-page walk, every existing `validate_page`/`validate_predicate`
refusal reachable through the new request, the protocol-26 gate); new
golden vectors for `Request::FilteredPage` in `src/server/protocol.rs`,
regenerated into `tests/fixtures/wire-vectors.txt`; `cargo fmt`/`cargo
clippy --all-features -- -D warnings` clean.

## Traceability

- Roadmap: `SERVER-FILTERED-PAGE-DESIGN` (this document, `Implemented`),
  `SERVER-FILTERED-PAGE` (implementation, `Implemented`).
- `docs/FUTURE-GROWTH.md`'s SQL-parity item 1 updated once implemented
  — "`ORDER BY` combined with a filter" moves from "still absent" to
  named, bounded, and built, the identical treatment `Backup`/`Metrics`/
  `Replication`/schema migration each received in "Operational
  maturity."

## Open questions — resolved during implementation

- **Should `Memory`/`Relation` override `filtered_page` to at least
  narrow candidates via the `Ordered` index when the filter's *only*
  predicate happens to be a range on the order-by field itself**?
  **Resolved: not this round**, exactly as recommended — the shared
  default gives a correct, if not maximally fast, answer for every
  domain including this case; a targeted optimization for one narrow
  predicate shape stays a separable follow-on, not a blocker.
- **Is `page_rows`/`page_ids`'s existing selection helper directly
  reusable over an already-filtered `Vec<PageRow>`, or does it need a
  small signature adjustment?** **Resolved: directly reusable, no
  change needed** — `page_rows(rows: Vec<PageRow>, order_by, after,
  limit)` already takes a materialized row list and pages it; the
  shared default's filtered subset is exactly that shape.
- **Should the SQL grammar also accept `ORDER BY` + `WHERE` +
  `LIMIT`-absent (an unbounded filtered walk)?** **Resolved: yes**,
  exactly as recommended — `query_ordered`'s existing chunked-loop
  shape (`ORDER_BY_PAGE_CHUNK`) is reused unchanged for the filtered
  case, routed through a new `fetch_ordered_page` dispatcher that
  picks `fetch_page` (filter empty) or `fetch_filtered_page` (filter
  non-empty) per page.

## Change history

- 2026-09-14: initial proposal, design only.
- 2026-09-15: the owner picked option (a); implemented the same
  session. See `ADR-0068`'s own "Acceptance and implementation"
  section for the full record.
