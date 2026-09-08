# ADR-0058: The `Relation` domain — the hub's directed, open-label edges as a record table

- Status: **Proposed** (2026-09-07). Proposed and implemented on one
  branch, the `ADR-0046`–`ADR-0057` cadence; the owner's options are in
  "Considered options" below.
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-RELATION-DOMAIN-DESIGN.md` (the full
  design), `ADR-0036`/`ADR-0048` (the front-door domain precedent),
  `ADR-0042` F4 (directed edges, deferred since), `ADR-0054`–`ADR-0056`
  (the merge, the page, the sentinels reused),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 4).
- Supersedes/Superseded by: none. A seventh domain and a third table on
  `memory_server`; no wire or format change.

## Context

The hub's `entity_relations` is a directed edge under an open label
with four sync columns, merged by last-writer-wins and paged by
`updated_at`. This crate's edge layers are undirected, label-only, and
metadata-free; the spike's adapter returned `Err` for every read and
write of it — the last of its five gaps.

## Decision

Model the table as a table. `Relation` is a seventh domain on
`Reminder`'s stack shape — `subject` indexed, `updated_at_unix_ms`
scannable, `node_id`/`deleted_at_unix_ms` with `ADR-0056`'s sentinels,
endpoints the consumer's own id strings stored verbatim — served by
`memory_server` as its third table, `relation`. Every hub method over
it is a request that already exists: insert, replace, the guarded
merge, delete, the ordered page, `Query` for out-edges, in-edges, and
one label, `Aggregate` for counts, compaction. No directed edge layer,
no referential integrity, no triple uniqueness, no fixed label set.

## Consequences

- Positive: the fifth and last spike gap closes with zero new
  primitives and no wire change; the consumer's adapter maps one-to-one.
- Positive: additive; a two-table connection never sees the third.
- Named, not hidden: in-edges are a full scan; a graph the server walks
  would want the edge layer, named as the trigger.
- Named, not hidden: nothing checks that an endpoint names an entity —
  exactly the consumer's own posture.

## Considered options

**(a) Accept as designed** — a record table. **(b) A directed edge
layer with metadata** — the graph-primitive answer, a storage and wire
design of its own. **(c) Directed edges without metadata** — cannot
carry the sync columns. **(d) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.48.0 / FR-058 — `src/generic/relation.rs`,
  `src/server/relation.rs`, the module declarations,
  `src/server/journal.rs`, `src/bin/memory_server.rs`; tests:
  `generic/relation` +1, `server/relation` +2, `server_memory_integration`
  +1 (the three-table server); every acceptance criterion 1–3 holds.
