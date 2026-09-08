# Server Edge Count Design (Proposed)

- Status: **Proposed** (2026-09-07) — implemented on the same branch as
  `SERVER-001` v0.47.0 / FR-057 and `SERVER-002` v0.10.0, the
  `ADR-0046`–`ADR-0055` cadence, for the owner to accept as designed or
  send back.
- Date: 2026-09-07
- Related: `ADR-0039` (`MultiNeighbors`, `NeighborsByRelation`, the
  unknown-label rule reused), `ADR-0047` (runtime links, kept under
  both endpoints), `ADR-0050` (foreign labels, kept the same way),
  `ADR-0055` (`Page`, the read-gating precedent),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 5).
- Supersedes/Superseded by: none. Appends one `Request` and one
  `Response` variant at protocol 21; no format change.

## Purpose and scope

The hub's `count_tables("memory_entities")` and `stats.memory_entities`
need the number of edges under one label. The wire could only answer
per record: one `NeighborsByRelation` round trip per memory, summed
client-side — the spike's gap 5, O(records) round trips for one
number. Every edge is already kept under both endpoints, so the count
is one read of the adjacency.

Scope, exactly: `MultiNeighbors::edge_count` (`CNT-FR-001`);
`ConnectionStore::count_edges` on `Memory`/`Entity` (`CNT-FR-002`);
`Request::CountEdges`/`Response::Count` at protocol 21 (`CNT-FR-003`);
both clients (`CNT-FR-004`); pins and `SERVER-002` v0.10.0
(`CNT-FR-005`).

## Non-goals

- **Counting per record, or per endpoint kind** — `NeighborsByRelation`
  already gives a record's degree.
- **`Employee`'s single-label `Symmetric` stack** — a `Neighbors`
  (marker-typed) count is a second trait method for a reference domain;
  `Unsupported` there, named.
- **Counting edges of every label at once** — `ListRelationKinds` then
  one `CountEdges` per label; a `Vec` answer is one appended request if
  ever wanted.
- **A total across tables** — a cross-table label is counted by the
  table that holds it (`Memory`'s `mentions`), once.

## Context and terminology

- **Kept under both endpoints**: `MultiSymmetric`'s adjacency stores
  `(a, b)` under `a` and under `b`, foreign labels included (the far
  endpoint's neighbours are this table's records that point at it), and
  `link` refuses self-loops. So the degree sum over a label is exactly
  twice its edge count.

## Requirements

- `CNT-FR-001` **Trait.** `MultiNeighbors::edge_count(&self, relation)
  -> Option<usize>`: `None` for a label the store has no relation
  under, else the degree sum halved. `MultiSymmetric` implements,
  `NameIndex` forwards; `GenericProductionStore::count_edges` reads it
  under the lock.
- `CNT-FR-002` **Adapter.** `ConnectionStore::count_edges(&self,
  relation) -> Result<u64, ErrorCode>`, default `Unsupported`; `Memory`
  and `Entity` answer, `Malformed` for an unknown label.
- `CNT-FR-003` **Wire.** `Request::CountEdges { relation: String }`
  (30), `Response::Count { count: u64 }` (19), `PROTOCOL_VERSION` 21,
  table row 21; a read gated as `Page` (`Malformed` below 21, rule 3);
  `audit::RequestKind::CountEdges`; `outcome_of` `Ok`.
- `CNT-FR-004` **Clients.** `SchemaDrivenClient::count_edges(relation)
  -> Result<u64, _>` (`Unsupported("count_edges")` below 21); Python
  `Client.count_edges(relation) -> int`, `PROTOCOL_VERSION = 21`.
- `CNT-FR-005` **Pins.** Two golden vectors (`Request/CountEdges`,
  `Response/Count`; 64 pinned); `SERVER-002` v0.10.0; the literal
  `Hello` pins at 21.

## Considered options

- **(a) One request per label answered from the adjacency —
  proposed.** One read, exact, foreign labels included; the smallest
  shape.
- **(b) Counts on `RelationDescriptor`** — `DescribeRelations` is a
  schema read cached by clients; a live number does not belong there.
- **(c) An `Aggregate` over edges** — a new aggregation domain for one
  `COUNT(*)`.
- **(d) Decline** — the O(records) loop stays.

## Proposed shape

`src/generic/query.rs`, `src/generic/store.rs` (two impls),
`src/generic/production.rs`; `src/server/protocol.rs`,
`src/server/serve.rs`, `src/server/audit.rs`,
`src/server/{memory,entity}.rs`, `src/server/client.rs`,
`clients/python/**`. Wire: `CountEdges` = `[0x1e 0 0 0]` · `relation`;
`Count` = `[0x13 0 0 0]` · `u64`.

## Data/state and invariants

- `count_edges(l)` equals the number of distinct undirected edges under
  `l`, before and after any link, delete, cascade, or compaction.

## Errors, failure, recovery, and observability

`Malformed` for an unknown label; `Unsupported` from a domain without
labelled relations. A read; nothing to recover.

## Security, privacy, and compatibility

A read, gated as `Query`. No format change; a pre-21 client unaffected.

## Acceptance criteria

1. Store: two seeded `mentions` count 2; a runtime link 3; deleting a
   record with two edges 1; the count survives a portable reopen; an
   unknown label is `None`.
2. Wire, two tables: the three sample mentions; 4 after a link; an
   unknown label `Malformed`; `relates_to` on `entity` 0; after
   deleting an entity with two mentions, 2. `Malformed` at 20 and
   silent, served at 21 with an unknown label `Malformed`,
   `Unsupported("count_edges")` at 1, `Unsupported` on `Dog`. From
   Python, the one runtime edge and an unknown label.
3. Two vectors added, every earlier vector byte-identical; `SERVER-002`
   v0.10.0; the Python suite passes offline and live.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-COUNT-EDGES-DESIGN`, `SERVER-COUNT-EDGES`. Decision:
  `ADR-0057`.
- Specification: `SERVER-001` v0.47.0 / `FR-057`; `SERVER-002` v0.10.0.
- Requirements: `CNT-FR-001`–`005`.

## Open questions

- **Directed open-label edges** — the spike's gap 4, the last, and a
  design of its own.
- **`Employee`'s count** — a marker-typed `Neighbors` count if a caller
  ever wants it.

## Change history

- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The twenty-second round in the `rusty_remind_me`-motivated
  line; the hub spike's fifth gap.
