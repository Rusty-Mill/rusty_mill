# Server SQL `ORDER BY`: Compile to the Existing Ordered Page (Accepted)

- Status: **Accepted as designed** (2026-09-14, `ADR-0061`, option (a) —
  one field, `U32`/`I64` only, ascending only, `WHERE`/`JOIN`/aggregate
  combinations refused; (b) a filter-bearing `Page`, (c) decline, both
  declined). Implemented on the same branch as `SERVER-001` v0.51.0 /
  `FR-061`.
- Date: 2026-09-14
- Related: `ADR-0034`/`docs/design/SERVER-SQL-SELECT-DESIGN.md` (the
  `SELECT` subset this extends — named `ORDER BY` a Non-goal), `ADR-0055`/
  `docs/design/SERVER-PAGE-DESIGN.md` (`Request::Page`, protocol 20 — the
  wire primitive this round compiles to, and the document that named
  this round as its own open revisit trigger), `ADR-0059`/
  `docs/design/SERVER-ORDERED-INDEX-DESIGN.md` (`Ordered<S, R, Marker>`,
  the index `Page` already answers `Memory`/`Relation` through in ~155
  µs regardless of table size), `ADR-0044`/`ADR-0045`/
  `docs/design/SERVER-SQL-JOIN-DESIGN.md` (`JOIN`'s own grammar, whose
  Non-goals already exclude `ORDER BY`), `docs/FUTURE-GROWTH.md` ("Path
  to SQLite/DuckDB parity," item 1).
- Supersedes/Superseded by: none. Additive to `src/server/sql.rs` and
  `src/server/client.rs`'s `SchemaDrivenClient` only — no `Request`/
  `Response` variant, no protocol-version bump, no server-side
  (`src/server/mod.rs`) or storage-layer change. Every existing SQL
  query compiles identically; only text containing a new keyword
  (`ORDER`) is affected.

## Purpose and scope

`Request::Page` (`ADR-0055`) already gives the wire a real, race-safe
ordered keyset walk over one orderable field, and `Ordered` (`ADR-0059`)
already makes it fast for `Memory`/`Relation`. Nothing on the *SQL* side
reaches it: `SchemaDrivenClient::query` compiles every non-`JOIN`,
non-aggregate `SELECT` to `Request::Query`, which has no order at all —
`ADR-0034`'s own module doc names `ORDER BY` a Non-goal, written before
`Page` existed. This round closes that specific gap and no other:
`ORDER BY <field>` in SQL text, for a plain query (no `WHERE`, no
`JOIN`, no `GROUP BY`/aggregate), compiled to `Request::Page`.

Scope, exactly: the grammar addition and its three parse-time
exclusions (`OBY-FR-001`–`002`); client-side field-kind and limit
validation (`OBY-FR-003`); compiling a `LIMIT`-bearing ordered query to
one `Page` round trip, and a `LIMIT`-free one to a loop of `Page` calls
until exhausted (`OBY-FR-004`); the existing `QueryResult::Rows` shape,
unchanged (`OBY-FR-005`). Python is unaffected — `clients/python`'s
`Client.query` never parsed SQL text (structured `select`/`where`
arguments only; `sql.rs`'s own module doc: "the server never sees SQL
text at all; this module is used only by [the Rust] client") and
already has its own `Client.page` (`ADR-0055`, `PAG-FR-005`) for an
ordered walk today.

## Non-goals

- **`ORDER BY` combined with `WHERE`.** `Request::Page` takes no filter
  argument. Refused at parse time (`OrderByWithFilter`), not silently
  unfiltered or unordered. `ADR-0061`'s option (b) is the follow-on if a
  consumer ever needs both.
- **`ORDER BY` combined with `JOIN`.** Already a `JOIN` Non-goal
  (`SERVER-SQL-JOIN-DESIGN.md`); refused here too (`OrderByWithJoin`)
  since this round's grammar addition is otherwise reachable from a
  `JOIN` query's token stream.
- **`ORDER BY` combined with `GROUP BY`/an aggregate column.** Refused
  (`OrderByWithAggregate`) — grouped/aggregated output is not the raw
  per-record field `Page` walks.
- **Descending order.** Ascending only, matching `Page` itself
  (`ADR-0055`'s own Non-goal, unchanged; `DESC` is one grammar token and
  one `descending: bool` argument away if ever wanted, on both this
  layer and `Page`'s).
- **Multiple `ORDER BY` fields.** One field, matching `Page`'s own
  single-field cursor shape (`(value, id)`).
- **Ordering by a `Str`/`Bool` field.** `U32`/`I64` only — `Page`'s own
  restriction, checked here client-side before any round trip
  (`ClientError::Unsupported("order by")`), the same posture
  `SchemaDrivenClient::page` already has.
- **A new wire primitive.** `Request::Page` is reused exactly as it is;
  no protocol-version bump.
- **Any change to `Request::Page`, `Ordered`, or `page_by_scan`.** This
  round is a pure consumer of the existing primitive.

## Context and terminology

Read from `src/server/sql.rs`, `src/server/protocol.rs`, and
`src/server/client.rs` as they stand at the crate's current head
(`SERVER-001` v0.50.0), not assumed:

- `sql.rs`'s grammar today: `query := "SELECT" columns "FROM" table_ref
  [join_clause] [where_clause] [group_by_clause] [limit_clause]`.
  `ORDER`/`BY` are not both reserved — `BY` already is (`GROUP BY`);
  `ORDER` is not yet in `is_keyword`'s list.
- `ParsedQuery` carries `columns`, `table`, `alias`, `join`,
  `conditions`, `group_by`, `limit` — no order field yet.
- `SchemaDrivenClient::query(sql) -> Result<QueryResult, ClientError>`
  parses, then routes: `join.is_some()` → `query_join`; `group_by`
  non-empty or any `Aggregate` column → `query_aggregate`; otherwise →
  `query_rows`, which compiles to `Request::Query { select, filter,
  limit }` and returns `QueryResult::Rows(Vec<QueryRow>)`, `QueryRow =
  (RecordId, Vec<(String, ScanValue)>)`.
- `Request::Page { order_by: FieldRef, after: Option<(ScanValue,
  RecordId)>, limit: u64 }` (protocol 20) is answered by the existing
  `Response::Rows` — **every field of the record**, not the `SELECT`
  list's projection; `Page` has no `Selection` argument. A caller
  compiling `SELECT name FROM memory ORDER BY updated_at` must project
  down to `name` itself after the round trip.
- `SchemaDrivenClient::page(order_by, after, limit)` already validates
  `order_by`'s kind (`U32`/`I64`) and a zero `limit`
  (`Unsupported("page order")`/`("page limit")`) and gates on protocol
  ≥ 20 (`Unsupported("page")`) — this round's validation mirrors it
  under its own message strings so a caller can tell which surface
  refused.
- `Page`'s own invariant (`SERVER-PAGE-DESIGN.md`): two consecutive
  pages, the second cursored by the first's last `(value, id)`, are
  disjoint and together cover every record whose key is greater than
  the first cursor, regardless of concurrent writes to records outside
  that range. An empty page ends the walk.

## Requirements

- `OBY-FR-001` **Grammar.** `src/server/sql.rs`:
  ```text
  query            := "SELECT" columns "FROM" table_ref [join_clause] [where_clause] [group_by_clause] [order_by_clause] [limit_clause]
  order_by_clause  := "ORDER" "BY" ident
  ```
  `ParsedQuery` gains `pub order_by: Option<String>`. `is_keyword` gains
  `"ORDER"` (`"BY"` is already reserved). Parsed after `group_by`,
  before `limit`, matching the grammar order above.
- `OBY-FR-002` **Parse-time exclusions**, checked before `parse` returns
  (alongside the existing `validate_qualifiers` pass): `order_by.is_some()`
  with a non-empty `conditions` → `SqlParseError::OrderByWithFilter`;
  with `join.is_some()` → `OrderByWithJoin`; with a non-empty `group_by`
  or any `ParsedColumnItem::Aggregate` in `columns` → `OrderByWithAggregate`.
  Each a new `SqlParseError` variant with its own `Display` message,
  matching `JoinWithAggregate`'s existing style.
- `OBY-FR-003` **Client-side validation**, in a new `query_ordered`
  compile path, before any round trip: the named field resolved via the
  existing `self.field(name)` (`ClientError::UnknownField` if absent);
  `ClientError::Unsupported("order by")` unless its kind is `U32`/`I64`;
  `ClientError::Unsupported("order by limit")` for a `LIMIT 0`;
  `ClientError::Unsupported("order by")` below protocol 20 (`Page`'s own
  gate — rule 4), checked before parsing the field so a pre-20 server
  never round-trips.
- `OBY-FR-004` **Compilation.** `SchemaDrivenClient::query` gains a
  fourth branch, checked after the existing `join`/aggregate branches
  (mutually exclusive with both by `OBY-FR-002`): `parsed.order_by.is_some()`
  → `query_ordered(parsed)`.
  - `LIMIT n` present (`n > 0`, checked by `OBY-FR-003`): one
    `Request::Page { order_by, after: None, limit: n as u64 }` round
    trip.
  - `LIMIT` absent: loop `Request::Page` calls with a fixed internal
    chunk size (`ORDER_BY_PAGE_CHUNK: u64 = 1_000`, a private constant
    in `client.rs`), `after` advanced to the previous page's last row's
    `(order_by value, id)`, until a page returns fewer rows than the
    chunk size (the walk's own exhaustion signal, `SERVER-PAGE-DESIGN.md`'s
    "the empty page ends a walk" generalized to "a short page ends a
    walk"), concatenating every row in order. Named, not hidden: this
    is *N* round trips for an *N*-chunk table, a materially different
    cost from every other `SELECT`, which is always one round trip.
  - Each returned row (every field, per `Page`'s shape) is projected
    down to the `SELECT` list — `ParsedColumns::All` keeps every field;
    `ParsedColumns::Named` keeps exactly the named fields, resolved the
    same way `query_rows` resolves them today — before being pushed
    into the result `Vec<QueryRow>`.
- `OBY-FR-005` **Result shape.** `QueryResult::Rows(Vec<QueryRow>)` —
  the identical variant `query_rows` already returns. No new
  `QueryResult` variant; every existing caller matching on `QueryResult`
  needs no change to keep compiling.
- `OBY-FR-006` **No Python, no wire, no server change.** `clients/python`
  is untouched (`Client.page` already exists); `src/server/protocol.rs`,
  `src/server/mod.rs`, and every storage-layer file are untouched — this
  round only teaches the Rust SQL front end to reach the primitive that
  already exists.

## Considered options

Mirrors `ADR-0061`'s own fork:

- **(a) As scoped above — proposed.** Smallest real step; zero wire or
  server risk; reuses `Page`/`Ordered` unchanged.
- **(b) A filter-bearing `Page`.** Closes the gap fully but needs a real
  evaluation-strategy design (filter-before-or-after-index-walk) this
  crate has not answered, plus a protocol bump — a legitimate, larger
  follow-on.
- **(c) Decline.** `SchemaDrivenClient::page`/Python `Client.page`
  remain the only ordered-walk surface; SQL text stays without it.

## Proposed shape

`src/server/sql.rs` (`order_by_clause`, `ParsedQuery::order_by`,
`is_keyword`'s `"ORDER"`, three new `SqlParseError` variants and their
`Display` arms, the exclusion checks); `src/server/client.rs`
(`query`'s fourth branch, `query_ordered`, `ORDER_BY_PAGE_CHUNK`). No
other file changes — `protocol.rs`, `mod.rs`, every adapter, and
`clients/python/**` are all untouched.

## Data/state and invariants

- An ordered-query result is exactly the rows `Page` would return across
  however many pages it took, in the identical ascending `(order_by
  value, id)` order `Page` itself guarantees — this round adds no new
  ordering logic, only the loop and the projection.
- Two ordered queries against a table with no writes between them return
  identical rows in identical order; a write between pages can add,
  remove, or move a row exactly as `Page`'s own invariant already
  describes (a concurrent insert/delete never shifts or repeats a row
  already served).

## Errors, failure, recovery, and observability

Every refusal above is either a parse-time `ClientError::Sql` (the three
new `SqlParseError` variants) or a client-side `ClientError::Unsupported`
(kind, limit, or version) — no frame sent for any of them, matching
`SchemaDrivenClient::page`'s own posture. A mid-walk round-trip failure
(network error, or a `Response::Err`) surfaces as today's
`ClientError::Server`/IO variant, propagated out of `query_ordered`
immediately — a partial walk's already-collected rows are dropped with
the error, not returned partially (matching every other fallible
multi-step client method in this crate, e.g. `write_batch`'s
all-results-or-error shape... except `write_batch` batches server-side;
here the loop is purely client-side, so this is a new, named case: a
caller retrying after a network error mid-walk re-starts from the
beginning, not from the last successful page, since no partial cursor
is returned on error). A read throughout: nothing to recover
server-side.

## Security, privacy, and compatibility

A read, gated exactly as `Page` (`Query`'s own posture: authentication
only, never overlaid by a session, never read-set-tracked). No wire
format change; a pre-20 client/server pairing is unaffected exactly as
`Page` already is — `ORDER BY` in SQL text is refused locally
(`OBY-FR-003`) rather than ever attempted against a server that cannot
answer it. Requires no auditing/access-log change — every `Page` round
trip `query_ordered` issues passes through the identical `dispatch`
path (and identical audit/access-log entries) as any other `Page` call.

## Acceptance criteria

1. `sql::parse`: `SELECT * FROM memory ORDER BY updated_at LIMIT 5`
   parses with `order_by: Some("updated_at".into())`; combined with a
   `WHERE`, a `JOIN`, a `GROUP BY`, or an aggregate column each fail
   with the matching new `SqlParseError` variant and no others; `ORDER`
   is case-insensitive and rejected as a bare identifier/alias
   elsewhere in the grammar exactly as `GROUP`/`BY` already are.
2. `query_ordered` against a real socket, `Memory`: a `LIMIT`-bearing
   ordered query returns exactly one `Page` round trip's worth, in
   ascending order, matching `SchemaDrivenClient::page`'s own result for
   the identical field/limit; a `LIMIT`-free query over a table sized
   to force at least three internal `Page` calls returns every row,
   still in order, with no duplicate or missing row across the seam
   between chunks (mirrors `SERVER-PAGE-DESIGN.md`'s own disjoint-pages
   acceptance test); a named `SELECT` column list projects down
   correctly even though `Page` itself returned every field.
3. Client-side refusals with no frame sent: a `Str`/`Bool` order field,
   a `LIMIT 0`, and protocol < 20, each the documented `Unsupported`
   string; an unknown order field `UnknownField`.
4. No existing SQL test's parsed shape or compiled request changes —
   every pre-existing `sql.rs`/`client.rs` SQL test passes unmodified.

## Verification plan

`cargo test --all-features` (unit `sql.rs`/`client.rs` tests plus
`tests/server_sql_integration.rs`, extended); `cargo clippy
--all-features -- -D warnings` (lib/bins/tests/examples — this session's
Windows environment could not run `--all-targets` due to a pre-existing,
documented Linux-only bench target unrelated to this round,
`benches/scan_ages_crossover.rs`); `cargo fmt --all --check`.

## Traceability

- Roadmap: `SERVER-SQL-ORDER-BY-DESIGN`, `SERVER-SQL-ORDER-BY`.
- Decision: `ADR-0061`.
- Specification: `SERVER-001` v0.51.0 / `FR-061`.
- Requirements: `OBY-FR-001`–`006`.

## Open questions

- **A filter-bearing `Page`** (`ADR-0061` option (b)) — the natural
  follow-on if `WHERE` + `ORDER BY` together is ever needed; needs its
  own evaluation-strategy design, not decided here.
- **Descending order** — one grammar token and one `Page` field away,
  named but not built, matching `SERVER-PAGE-DESIGN.md`'s own open
  question verbatim.
- **`ORDER_BY_PAGE_CHUNK`'s value (proposed 1,000)** — not measured
  against a real table in this design-only round; the implementation
  round should confirm it against `Page`'s own measured ~155 µs/page
  (`ADR-0059`) rather than treat 1,000 as anything but a starting
  guess.

## Change history

- 2026-09-14: initial proposal, design only. Selected by the planning
  session's own reconciliation pass (`docs/PROJECT-STATUS.md`, this
  date) after finding no roadmap/spec/traceability row short of
  `Implemented`/`Verified` — the cheapest, most concretely pre-scoped
  gap remaining on `docs/FUTURE-GROWTH.md`'s SQL-parity checklist.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch: `src/server/sql.rs`/`src/server/client.rs` as designed, no
  deviation. Proven: 5 new `sql.rs` unit tests, 6 new
  `tests/server_sql_integration.rs` integration tests over a real
  socket (`LIMIT`-bearing round trip matching `page()`, named-column
  projection, a 2,500-record no-`LIMIT` multi-page walk with no
  duplicate/missing row, every client-side refusal, the protocol-20
  gate). `SERVER-001` v0.51.0 / `FR-061`. `ORDER_BY_PAGE_CHUNK` shipped
  at the proposed 1,000 unmeasured — still an open question below.
