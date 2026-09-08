# ADR-0057: A global edge count — `Request::CountEdges` answered from the adjacency, at protocol 21

- Status: **Proposed** (2026-09-07). Proposed and implemented on one
  branch, the `ADR-0046`–`ADR-0055` cadence; the owner's options are in
  "Considered options" below.
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-COUNT-EDGES-DESIGN.md` (the full design),
  `ADR-0039` (`MultiNeighbors`), `ADR-0047`/`ADR-0050` (edges kept
  under both endpoints, foreign labels included),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 5).
- Supersedes/Superseded by: none. Appends `Request::CountEdges` (30) and
  `Response::Count` (19) at protocol 21; no format change.

## Context

The hub needs the number of edges under one label for its counts and
stats; the wire could only answer per record, one round trip each.
Every edge is already kept under both endpoints, so the number is one
read of the adjacency.

## Decision

Add `MultiNeighbors::edge_count` (the degree sum halved; `None` for an
unknown label), `ConnectionStore::count_edges` on `Memory`/`Entity`,
and `Request::CountEdges { relation }` answered by `Response::Count {
count }` at protocol 21 — a read gated as `Query`, `Malformed` for an
unknown label, `Unsupported` from a domain without labelled relations.
Both clients gain `count_edges`. Per-record counts, `Employee`'s
single-label stack, every-label-at-once, and cross-table totals are
named non-goals.

## Consequences

- Positive: the hub's `memory_entities` count is one round trip.
- Positive: additive; no format change; a pre-21 client unaffected.
- Named, not hidden: one label per request; `Employee` answers
  `Unsupported`.

## Considered options

**(a) Accept as designed** — one request per label from the adjacency.
**(b) Counts on `RelationDescriptor`** — a live number on a cached
schema read. **(c) An `Aggregate` over edges** — a new aggregation
domain for one count. **(d) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.47.0 / FR-057, `SERVER-002` v0.10.0 —
  `src/generic/{query,store,production}.rs`,
  `src/server/{protocol,serve,audit,memory,entity,client}.rs`,
  `clients/python/**`, two vectors (64 pinned); tests: `generic/memory`
  +1, `server_memory_integration` +1, `server_protocol_version` +1,
  `server_dog_integration` +1, `server_python_client` extended; every
  acceptance criterion 1–3 holds.
