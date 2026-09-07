# Server Runtime Deletion Design (Proposed)

- Status: **Proposed** (2026-09-07; implementation follows on the same
  branch — the `ADR-0046`–`ADR-0050` cadence — so the owner accepts or
  amends a working, tested shape). Options at acceptance: see
  `ADR-0051`.
- Date: 2026-09-07
- Related: `ADR-0036` (whose "no runtime deletion" was the last of its
  three fixed-shape clauses still standing after `ADR-0046`/`ADR-0049`),
  `ADR-0046`/`docs/design/SERVER-INSERT-DESIGN.md` (the insert log this
  design versions to carry tombstones), `ADR-0047` (the edge log and
  label manifest, whose folds now honor tombstones), `ADR-0049` (the
  fold's "log wins" rule, which a tombstone extends), `ADR-0050`
  (`serve_tables` and foreign labels — the cross-table cascade), the
  consumer's `delete_memory` (`db/queries.rs:397`, a hard `DELETE FROM
  memories WHERE id = ?` when sync is off) and entity delete
  (`entity.rs:610`, `DELETE FROM entities WHERE id = ?` then `DELETE
  FROM memory_entities WHERE entity_id = ?` at `:622`).
- Supersedes/Superseded by: none. Appends one `Request` variant at
  protocol 17; versions the insert log (2) with a backward-compatible
  reader; changes no existing variant, no `Response`, no `ErrorCode`.

## Purpose and scope

After fourteen rounds a running store can gain records and edges, a
record can change whole, and two tables can share one connection. It
still cannot lose a record. The consumer deletes memories, deletes and
merges entities, and clears `memory_entities` when an entity goes.
`ADR-0048`'s `Memory` named `deleted_at` as "no runtime deletion,
still"; this round removes the "still".

Scope, exactly: `Delete<R>` and `Detach<R>` in `crate::generic::query`;
a tombstone entry in the insert log (format version 2) and in every
edge log, honored by every fold (`DEL-FR-003`/`004`); `GenericMmapStore::
delete` retiring the slot (`DEL-FR-002`); forwards through every layer,
relation layers dropping every edge touching the id (`DEL-FR-004`), and
`MultiSymmetric::detach` for a foreign id (`DEL-FR-005`);
`ConnectionStore::{delete_record, detach_record}` with `Memory`/
`Reminder`/`Entity` implementing the first and `Memory` the second;
`Request::Delete` at protocol 17 with `Insert`'s gates (`DEL-FR-006`);
the cross-table cascade on a `serve_tables` server (`DEL-FR-007`); both
clients' `delete` (`DEL-FR-008`); one vector and `SERVER-002` v0.6.0
(`DEL-FR-009`).

## Non-goals

- **Soft deletion / `deleted_at`** — the consumer soft-deletes only
  when sync is on, for replication's sake; a backend stores what it is
  handed, and `status` already carries lifecycle. A hard delete is the
  primitive; a tombstone *field* is the caller's.
- **Compaction** — a retired slot's bytes stay in the mmap file and the
  blob shrinks only at the next fold; the file never shrinks. The
  trigger named since `ADR-0046` (a log noticeably larger than its
  blob) is unchanged.
- **Cascading record deletion** — deleting a parent (`Employee`'s
  manager) does not delete its children; their `manager_id` field now
  names an id with no record, which `parent`'s three-way outcome
  already distinguishes (`Parent`/`NoParent`/`NotFound`). Only *edges*
  cascade.
- **Unlinking a single edge** — a delete takes every edge of a record;
  removing one edge while both records stay is a different write, still
  open since `ADR-0047`.
- **Batch or filtered deletes** (`DELETE … WHERE`) — one id per request.
- **Transactions over deletes** — never staged, never journaled, exactly
  `Insert`/`Replace`/`Link`.

## Context and terminology

- **Tombstone**: a log entry naming an id instead of carrying a record
  (or edge). The insert log gains a per-entry kind byte for it: format
  version 2. Version-1 logs (every log written before this round) read
  as items only and are rewritten as version 2 on their next append —
  never a mix. The edge logs share the format, so an edge tombstone
  ("every edge touching this id") rides the same code.
- **Retired slot**: the mmap slot's marker byte cleared, so the next
  open skips it as it skips a slot a crash never committed, and the
  fast scan path stands down (`is_gapless` fails; the fallback reads
  only live slots). Nothing else in the slot changes.
- **Cascade**: on a `serve_tables` server, a delete in table *T* is
  followed by `detach_record` on every other table for each relation
  whose `target_table` is *T* — the consumer's `DELETE FROM
  memory_entities WHERE entity_id = ?`.

## Requirements

- `DEL-FR-001` **Traits and error.** `query::Delete<R>` with `fn delete
  (&mut self, id) -> Result<(), DeleteError<R::Id>>`; `query::Detach<R>`
  with `fn detach(&mut self, relation, id) -> Result<usize, DeleteError
  <R::Id>>`; `DeleteError<Id>::{NotFound(Id), Durability(DurabilityError)}`.
- `DEL-FR-002` **`GenericMmapStore::delete`.** An unknown id is
  `NotFound` with nothing written. Else: a tombstone is appended to the
  insert log and `sync_data`ed; the slot's marker is cleared; the id
  leaves its index bucket, the position index, and the record map.
  `Ok` means the deletion is durable. A later insert of the same id
  appends a fresh slot and a later log entry, both of which win.
- `DEL-FR-003` **Log format version 2.** Each entry is `kind: u8` (0
  item, 1 tombstone) + `u32` length + payload; the header's version
  field says 2. `append_tombstone`; `read_entries` yields `LogEntry::
  {Item, Tombstone}`; `read_items` yields items only. A version-1 file
  reads unchanged and is upgraded (temp file + rename) on its next
  append. An unknown kind is `RecordBlobUnreadable`.
- `DEL-FR-004` **The folds honor tombstones.** `merge_log`: a tombstone
  removes the record it names (later positions shift; an unknown id is
  a no-op, so a replay stays idempotent), a later item for the same id
  re-adds it. `merge_edge_lists`: an edge tombstone removes every edge
  touching the id in either orientation. Every layer forwards `delete`:
  `BaseStore` drops; `Indexed` leaves the bucket; `Scanned` retires its
  cache slot (swap with the last, repoint); `MmapScanned` clears its own
  marker; `Symmetric`/`MultiSymmetric` log an edge tombstone (per
  label, only where the id has edges) then drop the adjacency both
  ways; `NameIndex` leaves every key; `Reversed` leaves the parent's
  children list. `GenericProductionStore::{delete, detach}` under the
  write lock.
- `DEL-FR-005` **`MultiSymmetric::detach(label, id)`** — every edge
  under `label` touching an id this store may hold no record for
  (a foreign label's far end); tombstone logged first when there is a
  path; the count dropped; `0` and nothing logged when there is none;
  `NotFound` for a label the layer lacks. Forwarded by `NameIndex`.
- `DEL-FR-006` **The request.** `Request::Delete { id }` at 26,
  `PROTOCOL_VERSION` 17; `ConnectionStore::delete_record(&self, id) ->
  Result<DeleteOutcome, ErrorCode>` (`Deleted`/`NotFound`; default
  `Unsupported`; `Memory`/`Reminder`/`Entity` implement; `Durability` →
  `Storage`); `dispatch` answers `Ok`/`NotFound`; `handle_connection`
  refuses `ReadOnly` (`Unauthorized`, the sixth write), `SessionOpen`
  inside a session, `Malformed` below 17; `audit::RequestKind::Delete`.
- `DEL-FR-007` **The cascade.** `ConnectionStore::detach_record(&self,
  relation, id) -> Result<usize, ErrorCode>` (default `Unsupported`;
  `Memory` implements it for `mentions`, `Malformed` for any other
  label). `delete_across` in `handle_connection`: the table's own delete,
  then — only on `Ok` — every other table's `detach_record` for each
  relation whose `target_table` is this table; `Unsupported`/`Malformed`
  from a detach are skipped, `Storage` is reported in the delete's
  place. The one partial state is named: a crash between the two steps
  leaves edges to a record no table holds, which every read already
  skips (`evaluate_join`'s `get` miss, `neighbors` returning an id
  `get` answers `None` for).
- `DEL-FR-008` **Clients.** `SchemaDrivenClient::delete(id) -> Result<
  bool, _>` (`true` gone, `false` not found; `Unsupported("delete")`
  below 17, no frame); Python `Client.delete` the same, `PROTOCOL_VERSION
  = 17`.
- `DEL-FR-009` **Pins.** One golden vector (`Request/Delete`; 57 pinned);
  `SERVER-002` v0.6.0; the literal pins moved.

## Considered options

- **(a) A tombstone in the ordered insert log, the slot retired, edges
  cascading — proposed.** One log, one ordering, one fold rule for all
  three record-set writes; deletion and re-insertion of one id compose
  without a second file.
- **(b) A separate `<path>.deletes` log of ids.** Two logs lose the order
  between a delete and a later re-insert of the same id; a timestamp per
  entry would restore it at the cost of a clock.
- **(c) Soft delete — a `deleted` flag folded into every read.** Every
  read path filters; the slot and record stay; nothing ever goes; the
  consumer's own soft delete is a replication concern, not storage's.
- **(d) Rewrite the blob on every delete.** Durable and simple, and
  O(records) per delete on a table meant to hold tens of thousands.
- **(e) Decline** — keep the last clause of `ADR-0036`.

## Proposed shape

`src/generic/insert_log.rs` (version 2, `LogEntry`, `append_tombstone`,
`read_entries`, `read_raw`, the upgrade), `src/generic/slot_file.rs`
(`clear_marker`), `src/generic/query.rs` (`Delete`, `Detach`),
`src/generic/mod.rs` (`DeleteError`), `src/generic/mmap_store.rs`
(`delete`, `merge_log` over entries), `src/generic/store.rs`
(`merge_edge_lists` over entries, `drop_edges_of`, eight impls),
`src/generic/mmap_scanned.rs`, `src/generic/production.rs`;
`src/server/protocol.rs`, `src/server/serve.rs` (`DeleteOutcome`, the
two trait methods, the arm, the gates, `delete_across`),
`src/server/audit.rs`, `src/server/{memory,reminder,entity}.rs`,
`src/server/client.rs`, `clients/python/**`.

Wire: `Delete` = `[0x1a 0 0 0]` · `id` (16-byte `Uuid`).

## Data/state and invariants

- After a fold, the blob holds no record a tombstone removed unless a
  later item re-added it; an edge blob holds no edge touching a
  tombstoned id unless a later link re-added it.
- A retired slot is never in the position index; the slot count is ≥
  the live count; `is_gapless` is false while any slot is retired.
- A deleted record has no edges in its own table; on a `serve_tables`
  server it has none in any table whose relation targets its table,
  once the cascade completes.

## Errors, failure, recovery, and observability

`NotFound` on the wire for an unknown id; `Unsupported` from a domain
with no delete; `Storage` when the tombstone cannot be written (nothing
applied) or when a cascade's detach fails (the record already gone —
the one partial state). Recovery is the fold, now honoring tombstones.

## Security, privacy, and compatibility

A write: `ReadOnly` refused, sessions refused, gated below 17. The log
format change is backward compatible in both directions that matter: a
version-2 reader reads version 1; a version-1 file is upgraded only when
this build appends to it. A build older than this round cannot read a
version-2 log (version mismatch, refused by name) — a downgrade after a
delete is not supported, and is named.

## Acceptance criteria

1. Store: on `Order`'s core a delete removes the record from every read,
   retires the slot (the fast scan path stands down, only live slots are
   read), refuses a repeat, survives a portable reopen with the log
   folded away, and the id inserts again with the re-insert winning
   across another reopen; a log of tombstones and items folds in order.
   `Symmetric` drops both ends and the fold agrees; `MultiSymmetric::
   detach` drops a foreign id's edges and nothing else. Through the real
   stacks: `Entity` (names, kind, every neighbor's list, portable reopen,
   re-insert with no edges), `Memory` (a deleted memory's `mentions`
   edge, a detached entity's edges, reopen), `Employee` (a child leaves
   its manager's list, its collaboration edge goes from the other end).
2. Wire, two tables: deleting a memory removes it from every read and
   from the entity's side of `mentions`; the repeat is `Ok(false)`;
   deleting an entity from the entity table detaches every memory's
   edge to it with the memories intact and the cross-table join
   shrinking; the deleted memory's id inserts again with no edges; a
   restart serves all of it. `Malformed` at 16 and silent with nothing
   deleted, served at 17 (`Ok` then `NotFound`); `Unsupported("delete")`
   at 1; `Unauthorized` for `ReadOnly`; `SessionOpen` in a session;
   `Unsupported` on `Dog`; from Python a delete and its `False` repeat.
3. Log: items and tombstones interleave in order; a version-1 log reads
   as items and upgrades on its next append; an undecodable entry is
   still an error.
4. One vector added, every earlier vector byte-identical; `SERVER-002`
   v0.6.0; the Python suite passes offline and live.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-DELETE-DESIGN`, `SERVER-DELETE`. Decision:
  `ADR-0051`.
- Specification: `SERVER-001` v0.41.0 / `FR-051`; `SERVER-002` v0.6.0.
- Requirements: `DEL-FR-001`–`009`.

## Open questions

- **Compaction** — now that records, edges, and tombstones all
  accumulate in logs and retired slots, the trigger is the same but the
  case is stronger; a `compact()` that rewrites the mmap file and blob
  from the live set is the shape.
- **Unlinking one edge** — the remaining edge write.
- **Entity merge** — the consumer's `merge_entities` repoints
  `memory_entities` then deletes the loser; here that is a `detach`,
  N links, and a delete — a client composition today, a server request
  if the round trips ever matter.
- **Downgrade** — a version-2 log is unreadable to a pre-round build;
  named, not solved.

## Change history

- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The sixteenth round in the `rusty_remind_me`-motivated line;
  the last of `ADR-0036`'s three clauses.
