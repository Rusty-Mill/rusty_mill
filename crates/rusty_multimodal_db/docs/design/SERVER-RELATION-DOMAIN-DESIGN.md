# Server Relation Domain Design (Accepted)

- Status: **Accepted** (2026-09-07, `ADR-0058` option (a), as designed —
  a record table; a directed edge layer with or without metadata, and
  declining, all declined). Implemented on the same branch as
  `SERVER-001` v0.48.0 / FR-058, PR #220. No wire change.
- Date: 2026-09-07
- Related: `ADR-0036`/`ADR-0048` (the front-door domain precedent:
  `Reminder`'s one-index/one-scan stack, `Memory`'s projection),
  `ADR-0042` F4 (directed edges, named and deferred since), `ADR-0047`
  (undirected open labels), `ADR-0050` (foreign labels), `ADR-0054`/
  `ADR-0055`/`ADR-0056` (the merge, the page, the sentinels this table
  reuses), `docs/reports/2026-09-07-hub-spike-report.md` (gap 4, the
  last).
- Supersedes/Superseded by: none. Adds a seventh domain and a third
  table on `memory_server`; no `Request`/`Response`, no format change.

## Purpose and scope

The hub's `entity_relations` is the last of the spike's five gaps: a
**directed** edge under an **open label**, `subject --relation-->
object`, with four more columns — `created_at`, `updated_at`,
`node_id`, `deleted_at` — that it syncs by last-writer-wins and pages by
`(updated_at, id)`. Every edge primitive this crate has is undirected,
label-only, and metadata-free, so the spike's adapter returned `Err`.

The observation that closes it: an edge with metadata that is inserted,
replaced, merged, paged, counted, and soft-deleted **is a record**. This
round models the table as a table — a seventh domain, `Relation`, on
`Reminder`'s stack shape — and every capability it needs already
exists: `Insert`/`Replace`/`ReplaceIf` (`ADR-0046`/`0049`/`0054`),
`Delete` (`ADR-0051`), `Page` by `updated_at` (`ADR-0055`), `Query` on
any field (out-edges, in-edges, one label), `Aggregate` (counts),
`Compact`, and `ADR-0056`'s sentinels for the two nullable columns.

Scope, exactly: the `Relation` record and its stack (`REL-FR-001`–
`003`), the adapter (`REL-FR-004`), the third table on `memory_server`
(`REL-FR-005`).

## Non-goals

- **A directed edge layer** (`DirectedSymmetric`, out/in adjacency,
  `LinkDirected`, `OutNeighbors`) — the graph-primitive answer, a
  storage and wire design of its own for a table whose consumer never
  walks it as a graph server-side; declined in favour of the record
  shape, and named as the day a caller needs an O(1) out-neighbour
  read on a table too large for `FilterEq subject` plus a client filter.
- **Referential integrity** — `subject`/`object` are the consumer's own
  id strings, stored verbatim, asserting nothing about the `entity`
  table, exactly as the consumer's foreign-key-less SQLite table does;
  a `JOIN` across is not a relation, so none is declared.
- **Uniqueness of `(subject, relation, object)`** — the consumer's table
  keys on `id`; a duplicate triple is two records, as there.
- **A fixed label set** — any non-empty string; the consumer's is open.
- **Cross-table edge counts** — `Aggregate COUNT(*) WHERE relation = …`
  answers a per-label count; `CountEdges` is for edge layers.

## Context and terminology

- **Record table**: a domain whose records are the consumer's rows,
  one durable stack, no edge layer.
- **Out-edges / in-edges**: `FilterEq subject` (the one index) / `Query
  WHERE object = …` (a full scan, milliseconds at target scale).

## Requirements

- `REL-FR-001` **Record.** `Relation { id, subject, relation, object,
  created_at_unix_ms, updated_at_unix_ms, node_id, deleted_at_unix_ms
  }`; `SCHEMA_TAG` `relation::Relation`; `subject`/`relation`/`object`
  non-empty; `node_id` `""` unattributed; `deleted_at_unix_ms` `0`
  live, non-negative.
- `REL-FR-002` **Fields.** `subject` the `IndexedField` (`SubjectField`,
  `String`); `updated_at_unix_ms` the `ScannableField`
  (`UpdatedAtField`, `i64`).
- `REL-FR-003` **Stack.** `RelationProductionStack =
  GenericMmapStore<Relation, SubjectField, UpdatedAtField>`;
  `create_`/`open_…_portable`/`open_or_create_relation_production_stack`.
- `REL-FR-004` **Adapter.** `RelationConnectionStore` (`table_name`
  `relation`), seven wire fields in tag order (`subject` 0 … `deleted_at
  _unix_ms` 6), `subject` the one `filter_eq`, `updated_at_unix_ms` the
  one `scan`/`update`, every write of every front-door domain, every
  edge request `Unsupported`, `CheckpointFlush` for the journal.
- `REL-FR-005` **Served.** `memory_server` serves `relation` as its
  third table — `<dir>/relations.mmap` under `SERVER_DATA_DIR`, two
  sample edges in scratch.

## Considered options

- **(a) A record table — proposed.** Zero new primitives; every hub
  method over it is a request that exists.
- **(b) A directed edge layer with metadata** — a new adjacency shape,
  new blobs and logs, five or six new requests, edge metadata the
  crate has never carried; the right shape for a graph the server
  walks, which this consumer's is not.
- **(c) Directed edges without metadata** (`LinkDirected` on
  `MultiSymmetric`) — cannot carry the sync columns, so the hub still
  cannot merge or page them.
- **(d) Decline** — the last gap stays open.

## Proposed shape

`src/generic/relation.rs` (new), `src/server/relation.rs` (new),
`src/generic/mod.rs`/`src/server/mod.rs` (the modules),
`src/server/journal.rs` (`CheckpointFlush`), `src/bin/memory_server.rs`
(the third table). No `src/server/{protocol,serve,client}.rs`,
`clients/python/**`, or fixture change; `PROTOCOL_VERSION` stays 21.

## Data/state and invariants

- Every write and read on `relation` is the same request it is on
  `memory`; the table's only rules are the three non-empty strings and
  the non-negative stamp.

## Errors, failure, recovery, and observability

`Malformed` for an empty endpoint or label, a negative stamp, or a
malformed field list; `Unsupported` for every edge request; otherwise
every front-door domain's answers.

## Security, privacy, and compatibility

No wire change; a third table on the two-table server, invisible to a
connection that never sends `Use relation`.

## Acceptance criteria

1. Store: out-edges by subject through the index, stamps through the
   scan, a runtime insert and a delete surviving the portable reopen,
   open-or-create reopening rather than recreating.
2. Adapter: seven fields in tag order with the stated capabilities;
   insert refusing an empty subject, a negative stamp, and a missing
   field with nothing written; replace moving the index; a guarded
   replace refusing an older stamp; delete and its repeat.
3. Wire, three tables: `ListTables` naming `relation` third; three
   edges inserted and an empty subject refused; out-edges by
   `FilterEq`, in-edges and one label by `Query`, a page by
   `updated_at`, last-writer-wins by `ReplaceIf`, a count by
   `Aggregate`, a delete; the winner and the deletion surviving a
   restart.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-RELATION-DOMAIN-DESIGN`, `SERVER-RELATION-DOMAIN`.
  Decision: `ADR-0058`.
- Specification: `SERVER-001` v0.48.0 / `FR-058`. No `SERVER-002`
  change.
- Requirements: `REL-FR-001`–`005`.

## Open questions

- **A directed edge layer** — the day a caller needs O(1) out-neighbour
  reads on a table too large for the index-plus-filter walk.
- **The consumer's adapter** — `apply_record(EntityRelation)` and
  `pull_entity_relations` map onto this table one-to-one; its spike
  branch should be updated.
- **Everything `FR-056`/`FR-057` name** — unchanged.

## Change history

- 2026-09-07: Accepted as designed (option (a)), after PR #220. No
  content change.
- 2026-09-07: Implemented as `SERVER-001` v0.48.0 / FR-058, landed as
  designed (PR #220).
- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The twenty-third round in the `rusty_remind_me`-motivated
  line; the hub spike's fourth and last gap.
