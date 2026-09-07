# Server Tables Design — more than one table on one connection (Proposed)

- Status: **Proposed** (2026-09-07; implementation follows on the same
  branch — the `ADR-0046`–`ADR-0049` cadence — so the owner accepts or
  amends a working, tested shape). Options at acceptance: see
  `ADR-0050`. This is the implementation unit `ADR-0045` gated; Part B
  of `docs/design/SERVER-SQL-JOIN-DESIGN.md` is the accepted direction
  it builds, restated here where this round adds to it.
- Date: 2026-09-07
- Related: `ADR-0045`/`docs/design/SERVER-SQL-JOIN-DESIGN.md` Part B
  (`TBL-FR-001`–`006`, the accepted shape: `serve_tables`, per-connection
  `Use`, `ListTables`, cross-table `Join` via `right_table`/
  `target_table`, sessions per connection), `ADR-0044` (Part A — the
  `JoinSpec::right_table` and `RelationDescriptor::target_table` fields
  carried since protocol 12 for exactly this), `ADR-0048` (`Memory` —
  gate (ii)'s second table, whose stack this round changes once, as
  that design said it would), `ADR-0047` (`MultiSymmetric`'s edge log
  and label manifest, reused unchanged for the cross-table edge),
  `ADR-0022` (per-connection state precedent).
- Supersedes/Superseded by: none. Appends two `Request` variants and one
  `Response` variant at protocol 16; changes no existing variant, no
  `ErrorCode`, no on-disk format.

## Purpose and scope

`ADR-0045` recorded the shape of a multi-table connection and gated its
implementation on "a second table someone needs". `ADR-0048` built that
table — `Memory` — and named the consumer's `memory_entities` link as
the first cross-table relation. This round builds the gated unit.

The consumer's `memory_entities` (`entity.rs:145`, `INSERT OR IGNORE
INTO memory_entities (memory_id, entity_id, created_at)`) is a directed
many-to-many link, memory → entity, read from both ends: the entities
a memory mentions, the memories that mention an entity
(`entity.rs:327-347`), and memories sharing an entity (`expansion.rs:
200`). Every one of its joins crosses the two tables. So the instance
this round needs is exactly Part B's shape plus one thing Part B did
not have: **a relation whose far endpoint lives in another table**.

Scope, exactly: `TBL-FR-001`–`006` as accepted, with the two additions
below (`TBL-FR-007` the foreign relation, `TBL-FR-008` `Memory`'s
`mentions`) and the client half (`TBL-FR-009`), one design decision
each and no others.

## Non-goals

- **Per-table authorization** — one `ServeOptions` per server, as
  accepted; a `ReadOnly` token is read-only on every table.
- **Multi-table sessions or journals** — a session's staged writes
  belong to the table selected at `Begin`; `Use` inside one is
  `SessionOpen` (`TBL-FR-006`, the proposed of the two alternatives).
- **A table name on any pre-16 request** — rule 1; the per-connection
  variable is the whole mechanism.
- **Directed edges** — `mentions` is stored as `MultiSymmetric` stores
  every edge, both ways. The direction is carried by the *types*: the
  near end is always a `Memory`, the far end always an `Entity`, which
  is why reading it from the entity side is meaningful and needs no
  reverse index. A same-typed directed edge (`ADR-0042` F4) is still
  open.
- **Edge metadata** — the consumer's `created_at` on the link is not
  stored; still open since `ADR-0047`.
- **The `Order → Customer` instance** — `Customer` still has no store;
  `TBL-FR-005`'s research-gated instance is not built. `Memory →
  Entity` is the instance.
- **Cross-table `Aggregate`/`Query`** — a `Query` is one table's; a
  cross-table join is the only cross-table read.
- **The access log's table field** — Part B suggested recording the
  table in the access event; not added, to keep the event's pinned
  format unchanged. Named.

## Context and terminology

- **Table**: one `ConnectionStore` registered under a name with
  `serve_tables`. `serve` is the one-table case, the name from the new
  `ConnectionStore::table_name`.
- **Primary**: the table a connection starts on, so a connection that
  never sends `Use` is byte-for-byte a pre-16 connection.
- **Foreign label** (`TBL-FR-007`): a `MultiSymmetric` label whose far
  endpoint (`b` in `link(label, a, b)`) is another table's record. The
  layer checks only `a` against its own store; the *server* checks `b`
  against the table the relation's descriptor names, because only the
  server holds both adapters. A domain constant, set by the domain's
  constructors, never by a runtime `link`, not in the manifest.

## Requirements

`TBL-FR-001`–`006` are Part B's, implemented as written (see
`docs/design/SERVER-SQL-JOIN-DESIGN.md`), with these particulars:

- `TBL-FR-001` — `pub fn serve_tables(listener, tables: Vec<(String,
  Arc<dyn ConnectionStore>)>, primary: usize, options)`; `serve` calls
  it with one table named by `ConnectionStore::table_name()` (a new
  trait method with a default of `"table"`; every shipped adapter
  overrides it with its domain name). Every call site unchanged.
- `TBL-FR-002` — `Request::Use { table }` at 24; `handle_connection`
  keeps `table: usize` beside `negotiated`/`session`, default
  `primary`, and resolves the adapter per request; `Ok`/`Malformed`;
  `SessionOpen` inside a session; `Malformed` below 16; not a write. A
  one-table `dispatch` answers `Use` `Ok` only for its own name.
- `TBL-FR-003` — `Request::ListTables` at 25 → `Response::Tables {
  names, primary }` at 17; `Malformed` below 16.
- `TBL-FR-004` — `JoinSpec::right_table: Some(name)`: `validate_join`
  takes the right table's schema, requires the relation's
  `target_table == name` (`Malformed` otherwise, and when the server
  registered no such table); `evaluate_join` takes two adapters — the
  relation the left's, `get` the right's. Not version-gated beyond 12:
  no new shape crosses the wire.
- `TBL-FR-005` — the instance is `Memory → Entity`, not `Order →
  Customer` (a non-goal above).
- `TBL-FR-006` — `Use` inside a session is `SessionOpen`.
- `TBL-FR-007` **Foreign labels.** `MultiSymmetric::with_foreign_labels
  (&[&str])` and `is_foreign(label)`; `link` on a foreign label skips
  the far-end existence check and nothing else (label validity, near
  end, self-loop, the log, label creation all unchanged). Server-side,
  `Request::Link` under a relation whose descriptor names a
  `target_table` has `right` checked in that table before dispatch:
  `RecordNotFound` when absent, `Unsupported` when the server
  registered no such table.
- `TBL-FR-008` **`Memory::mentions`.** `MemoryProductionStack` becomes
  `MultiSymmetric<GenericMmapStore<Memory, …>, Memory>` with
  `MEMORY_RELATION_LABELS = ["mentions"]` foreign to
  `MEMORY_FOREIGN_TABLE = "entity"`; the three constructors take the
  `(memory, entity)` edge list (`create`/`open`) or read it from the
  files (`open_portable`). `MemoryConnectionStore`: `neighbors`,
  `neighbors_by_relation`, `list_relation_kinds` over the stack;
  `describe_relations` lists `mentions` alone, `target_table: Some
  ("entity")` — the unfiltered `neighbors` is *not* listed, since its
  far side is never this table's rows; `link_records` accepts
  `mentions` only (`Malformed` otherwise); `relations.neighbors: true`;
  `table_name() == "memory"`. A `memory_server` binary now serves
  `memory` (primary) and `entity`.
- `TBL-FR-009` **Clients.** `SchemaDrivenClient::{list_tables, use_table,
  table}` (`Unsupported` below 16, no frame; `use_table` re-fetches the
  schema and relation list); a SQL `JOIN <other> b ON <relation>`
  compiles to `right_table: Some(other)` when the relation's descriptor
  names that table, with the right side resolved against that table's
  schema (fetched once by `Use`/`DescribeSchema`/`Use` back, cached);
  every mismatch is `ClientError::Sql`. Python `Client.{list_tables,
  use_table, table}` and `join` crossing tables the same way.
- `TBL-FR-010` **Pins.** Three golden vectors (`Request/Use`,
  `Request/ListTables`, `Response/Tables`; 56 pinned); `SERVER-002`
  v0.5.0; the literal pins moved.

## Considered options

- **(a) As accepted plus foreign labels on `MultiSymmetric` — proposed.**
  The cross-table edge reuses the edge blob, edge log, and manifest
  unchanged; one field and one skipped check in the layer; the far-end
  check moves to the one place that has both adapters.
- **(b) A new directed cross-table relation layer.** A `HashMap<Id,
  Vec<ForeignId>>` with its own blob and reverse index — more code for
  the same reads, and a second edge format to specify.
- **(c) Store the link on the entity side too** (a `mentioned_by` label
  on `Entity`). Two copies of one edge to keep in step across two
  stores with no transaction spanning them; the symmetric storage
  already gives the reverse read from the memory table.
- **(d) `Order → Customer` first.** Research-gated, needs a `Customer`
  store nobody asked for; `Memory → Entity` is the consumer's join.
- **(e) Decline** — leave `ADR-0045` gated.

## Proposed shape

`src/generic/store.rs` (`foreign_labels`, `with_foreign_labels`,
`is_foreign`, the one skipped check), `src/generic/memory.rs` (the
stack, the constants, the constructors), `src/server/protocol.rs` (two
requests, one response, the constant, the row, three vectors),
`src/server/serve.rs` (`table_name`, `serve_tables`, `handle_connection`
over tables, `Use`/`ListTables`, `join_across`, `link_across`,
`validate_join`/`evaluate_join` over two adapters), `src/server/audit.rs`
(two kinds), `src/server/{dog,order,employee,reminder,entity}.rs`
(`table_name`), `src/server/memory.rs` (the relation), `src/server/
client.rs`, `clients/python/**`, `src/bin/memory_server.rs`.

Wire: `Use` = `[0x18 0 0 0]` · `String`; `ListTables` = `[0x19 0 0 0]`;
`Tables` = `[0x11 0 0 0]` · `Vec<String>` · `String`.

## Data/state and invariants

- No on-disk change: `memories.mmap.mentions.edges` (+ `.inserts`) and
  `memories.mmap.relations` are `ADR-0047`'s files.
- A connection with no `Use` is byte-for-byte a pre-16 connection.
- A foreign edge's near end is always this table's record; its far
  end was checked against the target table when the server linked it.
  A far-end record replaced later still resolves (ids are stable);
  there is no deletion to dangle it.
- `Join` never crosses tables unless the descriptor says it does.

## Errors, failure, recovery, and observability

Unknown table → `Malformed`; `Use` in a session → `SessionOpen`; a
cross-table join naming the wrong table, or one the server lacks →
`Malformed`; a foreign link's far end absent → `RecordNotFound`, its
table unregistered → `Unsupported`. No new `ErrorCode`.

## Security, privacy, and compatibility

One `ServeOptions` for every table; the authentication gate is per
connection, before any `Use`. Protocol 16 appends only; `Malformed`
below it for both requests; cross-table `Join` needs no new shape and
is served to any connection at 12 or above whose relation descriptor
names a table.

## Acceptance criteria

1. `serve` unchanged (every existing suite green); `serve_tables` with
   `memory` and `entity` serves both; a connection with no `Use` sees
   the primary; `ListTables` lists both with the primary.
2. `Use` switches every table-less request (the schema, `get`, a
   `Query`) and back; an unknown name is `Malformed` with the table
   unchanged; `Use` inside a session is `SessionOpen`; both requests
   `Malformed` at 15 and silent, served at 16; the Rust client refuses
   both at 1 with no frame.
3. `SELECT m.content, e.label FROM memory m JOIN entity e ON mentions`
   returns each memory with its entity's label in one round trip, a
   right-side `WHERE e.kind = …` resolving against the entity schema; a
   `JOIN memory x ON mentions` and a `JOIN` on the wrong table are
   refused client-side; `mentions` reads from both ends.
4. A runtime `link` to an entity is checked in the entity table
   (`RecordNotFound` for an unknown one), `Malformed` for a label the
   domain lacks, `Unsupported` on a server with no entity table; a
   second server on the same directory serves the edge and the join.
5. `MultiSymmetric`: a foreign label skips only the far-end check; a
   local label keeps every rule. `Memory`'s stack seeds, links, and
   reopens its `mentions` edges portably.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-TABLES-DESIGN`, `SERVER-TABLES`. Decision:
  `ADR-0050` (implementing `ADR-0045`).
- Specification: `SERVER-001` v0.40.0 / `FR-050`; `SERVER-002` v0.5.0.
- Requirements: `TBL-FR-001`–`010` (`001`–`006` from Part B).

## Open questions

- **Runtime deletion** — a deleted entity would leave `mentions` edges
  pointing at nothing; the consumer's `DELETE FROM memory_entities
  WHERE entity_id = ?` (`entity.rs:622`) is the shape to mirror when
  deletion arrives.
- **Edge metadata** (`created_at` on the link) — still open.
- **A cross-table `Aggregate`** (`COUNT` of memories per entity) — the
  consumer's `entity.rs:397` `LEFT JOIN … GROUP BY`; a join-then-group
  is client-side today.
- **The access log's table field** — deferred; the event format is
  pinned.

## Change history

- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The fifteenth round in the `rusty_remind_me`-motivated line;
  the unit `ADR-0045` gated.
