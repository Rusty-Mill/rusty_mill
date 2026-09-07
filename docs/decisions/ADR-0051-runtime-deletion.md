# ADR-0051: Runtime deletion — tombstones in the insert and edge logs, `Request::Delete` at protocol 17, the cross-table cascade

- Status: **Proposed** (2026-09-07; implemented on the same branch,
  the `ADR-0046`–`ADR-0050` cadence — the owner accepts or amends a
  working, tested shape). See "Considered options".
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-DELETE-DESIGN.md` (the full design),
  `ADR-0036` (the last of whose three fixed-shape clauses this closes),
  `ADR-0046` (the insert log, versioned here), `ADR-0047` (the edge
  logs, whose folds now honor tombstones), `ADR-0050` (`serve_tables`
  and foreign labels, which the cascade rides).
- Supersedes/Superseded by: none. Appends `Request::Delete` (26) at
  protocol 17; insert-log format version 2 with a backward-compatible
  reader; no new `Response`, no new `ErrorCode`, no new file.

## Context

`ADR-0036` fixed three things about a served table: no runtime
insertion, no runtime edge creation, no runtime deletion. `ADR-0046`
and `ADR-0047` removed the first two; `ADR-0049` made a record
replaceable; `ADR-0050` put two tables on one connection. Deletion was
the clause still standing, and the consumer's `delete_memory`, entity
delete, and entity merge all need it — the entity paths together with
`DELETE FROM memory_entities WHERE entity_id = ?`, a cross-table
cascade. The insert log (`ADR-0046`) already carries whole records in
order and folds them into the blob at the next open; what it lacked
was a way to say "this id is gone".

## Decision

Give the insert log a per-entry kind byte (format version 2 — a
version-1 log still reads, and is upgraded on its next append) so it
can carry a **tombstone**; make every fold honor one (a record
tombstone removes the record, an edge tombstone every edge touching the
id; a later item re-adds); implement `GenericMmapStore::delete` as
tombstone-then-retire-the-slot; forward `Delete` through every layer
with relation layers dropping every edge of the id and logging its own
tombstone; add `MultiSymmetric::detach` for a foreign id; on the wire
`Request::Delete { id }` at 26, protocol 17, answered `Ok`/`NotFound`
with `Insert`'s gates; on a `serve_tables` server, cascade the delete
into every other table's relation that targets this one via
`ConnectionStore::detach_record`; `delete` on both clients.

## Consequences

- Positive: the consumer's every write path now has a backend —
  `ADR-0036`'s last clause is gone; an id can be deleted and inserted
  again, in order, with no second file.
- Positive: no new response, no new error code, no new file; a pre-round
  directory reopens unchanged and its logs upgrade lazily.
- Named, not hidden: a build older than this round cannot read a
  version-2 log; a downgrade after a delete is unsupported.
- Named, not hidden: the cascade is two steps; a crash between them
  leaves edges to a record no table holds, which every read already
  skips. A `Storage` failure in the cascade is reported with the record
  already gone.
- Named, not hidden: retired slots and tombstones accumulate until the
  next open's fold; the mmap file never shrinks. Compaction's case is
  stronger and still open.
- Records do not cascade: a deleted `Employee` manager leaves its
  reports' `manager_id` naming no record.

## Considered options

**(a) Accept as designed** — tombstones in the ordered log, the slot
retired, edges cascading, one request. **(b) A separate deletes log**
— loses delete/re-insert ordering. **(c) Soft delete as a folded flag**
— nothing ever goes. **(d) Rewrite the blob per delete** — O(records)
per delete. **(e) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.41.0 / FR-051, `SERVER-002` v0.6.0 —
  `src/generic/{insert_log,slot_file,query,mod,mmap_store,store,
  mmap_scanned,production}.rs`, `src/server/{protocol,serve,audit,
  memory,reminder,entity,client}.rs`, `clients/python/**`, one vector
  (57 pinned); tests: `insert_log` +2 (one pre-existing reframed),
  `mmap_store` +2, `store` +2, `entity` +1, `memory` +1, `employee_impl`
  +1, three adapters +1 each, `server_memory_integration` +1 (the
  two-table delete suite with a restart), `server_protocol_version` +1,
  `server_dog_integration` +1, `server_auth_integration`/`server_
  transaction_integration`/`server_python_client` extended; every
  acceptance criterion 1–4 holds. (This PR.)
