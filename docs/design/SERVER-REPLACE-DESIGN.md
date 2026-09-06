# Server Whole-Record Replacement Design (Proposed)

- Status: **Proposed** (2026-09-06; implementation follows on the same
  branch — the `ADR-0046`/`ADR-0047`/`ADR-0048` cadence — so the owner
  accepts or amends a working, tested shape). Options at acceptance:
  see `ADR-0049`.
- Date: 2026-09-06
- Related: `ADR-0046`/`docs/design/SERVER-INSERT-DESIGN.md` (runtime
  record insertion — the insert log this design reuses for the new
  version of a record, the whole-list validation `Replace` shares with
  `Insert`, and the `Request::Insert` body shape `Request::Replace`
  copies exactly), `ADR-0048`/`docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`
  (the round that named this one: "whole-record replacement — the
  consumer's `update_memory` … the next round"), `ADR-0037`/`ADR-0040`
  (`Entity`'s `NameIndex` — the one derived index that must *move*
  keys, not just add them), `ADR-0013` (`UpdateField`'s `Ok`/`NotFound`
  reply shape, reused), `ADR-0034` (the "new client capability, no wire
  primitive" shape `upsert` follows).
- Supersedes/Superseded by: none. Appends one `Request` variant at
  protocol 15; changes no existing variant, no `Response`, no
  `ErrorCode`, no existing domain's read behavior.

## Purpose and scope

After `ADR-0046`/`ADR-0047`/`ADR-0048` a running store can gain
records, edges, and a `memories` table — but a record, once inserted,
can change in exactly one field: its scannable one, through
`UpdateField`. The consumer's `update_memory` (`db/queries.rs`) rewrites
`content`, `category`, `tags`, `metadata`, and `sensitive` in place;
its `entity_upsert` (`entity.rs`) is insert-or-replace on a derived id;
`reclassify` rewrites `memory_type`. None of those has a backend path:
`ADR-0048`'s `Memory` adapter refuses every field but `access_count`
with `Unsupported`, and says so.

This round adds the one missing write: **replace a record whole**, by
id, with `Insert`'s exact body. It is deliberately not "update N fields
of a record" — see "Considered options" — because whole replacement is
what the consumer sends, what the insert log already knows how to
store, and what every derived index can be told to follow with the
old and the new record in hand.

Scope, exactly: `Replace<R>` in `crate::generic::query` and
`GenericMmapStore::replace` over the insert log (`REP-FR-001`–`003`),
the forwards through every layer and `GenericProductionStore::replace`
(`REP-FR-004`), `ConnectionStore::replace_record` with `Memory`/
`Reminder`/`Entity` implementing it (`REP-FR-005`), `Request::Replace`
at protocol 15 with `Insert`'s gates (`REP-FR-006`), `SchemaDrivenClient
::replace` + a client-side `upsert`, and the Python client's `replace`
(`REP-FR-007`), one golden vector and `SERVER-002` v0.4.0 (`REP-FR-008`).

## Non-goals

- **Partial (multi-field) updates** — `UpdateField` stays the one-field
  write; a `Request::Update { id, fields }` that patches a subset is
  not added. The consumer always has the whole record when it edits
  (it reads, edits, writes), so the partial form buys nothing today
  and would need per-field "settable" capability flags in
  `DomainSchema` (a wire change to a struct — rule 1 forbids it; only
  a new descriptor variant could carry it).
- **Changing a record's id** — the id is the key of every index, slot,
  and edge; a "rename" is an insert plus a deletion, and there is no
  deletion.
- **Runtime deletion** — still none. `status`/`deleted_at`-style
  lifecycle stays the caller's field.
- **Edge changes** — edges are not fields; a replaced record keeps
  every neighbor, parent link, and child link it had. `Reversed`'s
  `ChildOf` relation *is* a field (`manager_id`), so a replace that
  changes it moves the child between parents — that is a field change
  the layer follows, not an edge write.
- **Staging a replace in a session** — never staged, never journaled,
  exactly `Insert`'s posture (`SessionOpen` inside a session).
- **Log compaction** — the log grows by one entry per replace as per
  insert; the fold at the next `open` is still the only compaction.

## Context and terminology

- **The insert log** (`INS-FR-003`): `<path>.inserts`, tagged,
  length-prefixed `bincode` records, folded into the blob at the next
  `open`. A replace appends the *new version* to this same log.
- **Log wins** (`REP-FR-003`): the fold now applies a logged record
  over the blob's copy *by id, in place* instead of skipping it. Before
  this round the skip and the overwrite were indistinguishable — a
  logged id the blob held was always the identical record — so no
  existing directory folds differently.
- **A slot is authoritative for its field**: `GenericMmapStore::get`
  overlays the slot's value on the blob's record; the blob's copy of
  the scannable field is never read back. A replace therefore writes
  the new scannable value into the slot in place (the `UpdateField`
  path) and lets the blob's copy be whatever the log says.

## Requirements

- `REP-FR-001` **Trait and error.** `query::Replace<R: Record>` with
  `fn replace(&mut self, record: R) -> Result<(), ReplaceError<R::Id>>`;
  `ReplaceError<Id>::{NotFound(Id), Durability(DurabilityError)}` in
  `crate::generic`.
- `REP-FR-002` **`GenericMmapStore::replace`.** An unknown id is
  `NotFound` with nothing written. Else: the new version is appended
  to the insert log and `sync_data`ed; the scannable value is written
  into the record's existing slot in place (no new slot); the id moves
  from the old indexed value's bucket to the new one's (when they
  differ; an emptied bucket is removed); the record is swapped. `Ok`
  means the new version is durable. **The one window, named**: the log
  lands before the slot write, so a crash between them leaves every
  field but the scannable one replaced; the next `open` folds the log
  and keeps the slot's value — the same state an `UpdateField` that
  landed and a `replace` that did not would leave.
- `REP-FR-003` **The fold applies the log over the blob.** `merge_log`
  keeps `records`' order, overwrites a held id in place with the logged
  version, appends a new id; a later entry for the same id wins. A
  replay of an entry the blob already carries is a no-op, so an
  interrupted fold stays idempotent. `read_portable_records` reports
  the same set.
- `REP-FR-004` **Every layer forwards.** `BaseStore` swaps; `Indexed`
  moves the bucket (old value read from the inner store before the
  move — `S: GetById<R>`); `Scanned` rewrites its cache slot;
  `MmapScanned` rewrites its own slot in place; `Symmetric`/
  `MultiSymmetric` forward with every edge intact; `NameIndex` drops
  the old keys the new version lacks and adds the new ones, normalized
  as at build; `Reversed` moves a child whose parent changed.
  `GenericProductionStore::replace` under the write lock.
- `REP-FR-005` **The adapter method.** `ConnectionStore::replace_record
  (&self, id, fields) -> Result<ReplaceOutcome, ErrorCode>` with
  `ReplaceOutcome::{Replaced, NotFound}` and a default of
  `Err(Unsupported)`. `Memory`, `Reminder`, and `Entity` implement it
  with **exactly `insert_record`'s validation** (the same
  `*_from_fields`: every field once, of its kind, the domain's rule) and
  then one `replace` under the store's lock; `Durability` → `Storage`.
  `Dog`/`Order`/`Employee` keep the default.
- `REP-FR-006` **The request.** `Request::Replace { id, fields }` at 23,
  `PROTOCOL_VERSION` 15. `dispatch` answers `Ok` for `Replaced` and
  `NotFound` for `NotFound` (`UpdateField`'s shape — no new `Response`,
  no new `ErrorCode`); `handle_connection` refuses `ReadOnly`
  (`Unauthorized`, the fifth write), `SessionOpen` inside a session,
  `Malformed` below 15 (rule 3); `audit::RequestKind::Replace`.
- `REP-FR-007` **Clients.** `SchemaDrivenClient::replace(&mut self, id,
  &[(&str, ScanValue)]) -> Result<bool, _>` (`true` replaced, `false`
  not found; names resolved through the schema; `Unsupported("replace")`
  below 15 with no frame). `SchemaDrivenClient::upsert` — insert, and
  only on `Duplicate` a replace; `true` created, `false` replaced; two
  round trips at most, no wire primitive (`ADR-0034`'s shape). Python
  `Client.replace` with the same `bool`, `PROTOCOL_VERSION = 15`.
- `REP-FR-008` **Pins.** One golden vector (`Request/Replace`; 53
  pinned); `SERVER-002` v0.4.0 (§5.6 row 23, §7 item 12, §8 row 15 and
  rule 3's list, the §4 `Hello` example at 15); the literal pins moved.

## Considered options

- **(a) Whole-record replace with `Insert`'s body, over the insert log
  — proposed.** One new trait, one new request, no new response or
  code, no new file, and every derived index has both versions in
  hand to move correctly. The consumer sends whole records anyway.
- **(b) Partial update — `Request::Update { id, fields }` patching a
  subset.** Closer to SQL's `UPDATE … SET`, but needs a "settable"
  flag per field the schema struct cannot gain (rule 1), a merge step
  per adapter, and a definition of what a partial `tags` means; the
  whole form subsumes it for a read-edit-write caller.
- **(c) Delete-then-insert.** Would need runtime deletion first — a
  round of its own with its own tombstone story — and would lose every
  edge of the record in between.
- **(d) A server-side `Upsert` request.** One round trip instead of
  two for the consumer's `entity_upsert`, but a second write variant to
  gate, audit, and specify for a saving that a local network does not
  notice; the client-side `upsert` delivers the shape now and leaves
  the wire primitive available later if the round trip ever matters.

## Proposed shape

`src/generic/query.rs` — `Replace`. `src/generic/mod.rs` —
`ReplaceError`. `src/generic/mmap_store.rs` — `replace`, `merge_log`'s
overwrite, the trait impl. `src/generic/store.rs` — seven forwards.
`src/generic/mmap_scanned.rs` — one. `src/generic/production.rs` —
`replace`. `src/server/protocol.rs` — the variant, the constant, the
table row, the vector. `src/server/serve.rs` — `ReplaceOutcome`, the
trait method, the `dispatch` arm, the three gates. `src/server/audit.rs`
— `RequestKind::Replace`. `src/server/{memory,reminder,entity}.rs` —
`replace_record`. `src/server/client.rs` — `replace`, `upsert`.
`clients/python/**` — `Replace`, `Client.replace`, the driver.

Wire: `Replace` = `[0x17 0 0 0]` · `id` (16-byte `Uuid`) · `fields`
(`u64`-LE length, then `(u16 tag, ScanValue)` pairs) — byte-for-byte
`Insert`'s body under a different index.

## Data/state and invariants

- The insert log may now hold several entries for one id; the fold
  applies them in order. The blob after a fold holds one record per id.
- A record's slot position never changes across a replace; the slot
  count equals the record count as before.
- `NameIndex`: after a replace, every normalized key of the new version
  maps to the id and no key exclusive to the old version does.
- `Reversed`: a child is in exactly its current parent's list.

## Errors, failure, recovery, and observability

`NotFound` on the wire for an unknown id (a normal outcome, as for
`UpdateField`); `Malformed`/`UnknownField`/`Unsupported` from the
adapter's validation before any write; `Storage` for a log append
failure with nothing applied. Recovery is the fold. The named window
(`REP-FR-002`) is the only partial state and is bounded to one field.

## Security, privacy, and compatibility

A write: `ReadOnly` refused, sessions refused, gated below 15. No
content rewrite for older connections (no new response). A version-14
client is served exactly as before; the fold's new semantics change no
existing directory's reopen (see "Log wins").

## Acceptance criteria

1. Store: on `Order`'s core a replace changes a non-scannable field, the
   indexed field (buckets move), and the scannable field (the slot,
   in place, no new slot); an unknown id is refused with nothing
   written; a portable reopen serves the new version with the log gone
   and a second reopen writing nothing; a log with two versions of one
   id folds to the later one. On `Memory`, `Entity` (name keys move,
   kind moves, every edge under every label survives), and `Employee`
   (a child moves between parents through `Reversed`) the same through
   the real stacks.
2. Wire, on `Memory`: `replace` with all eleven fields; every read sees
   the new version at once (`get`, `filter_eq` on the new and the old
   category, `WHERE sensitive`); an unknown id is `Ok(false)` with
   nothing created; a negative counter, a short list, and an unknown
   name are refused with nothing written; `upsert` replaces then
   creates; a restart serves the replaced version. On `Entity`:
   `upsert` replaces whole, the new alias resolves and the old label
   does not, `neighbors`/`JOIN … ON relates_to` unchanged, a restart
   serves it. From Python: replace, resolve by the new alias, the edge
   intact, an unknown id `False`; the Rust client sees the alias.
3. Gates: `Malformed` at 14 and silent with nothing replaced, served at
   15; `Unsupported("replace")` client-side at 1; `Unauthorized` for
   `ReadOnly`; `SessionOpen` inside a session; `Unsupported` on `Dog`.
4. One vector added, every earlier vector byte-identical; `SERVER-002`
   v0.4.0; the Python suite passes offline and live.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite;
`cargo doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-REPLACE-DESIGN`, `SERVER-REPLACE`. Decision:
  `ADR-0049`.
- Specification: `SERVER-001` v0.39.0 / `FR-049`; `SERVER-002` v0.4.0.
- Requirements: `REP-FR-001`–`008`.

## Open questions

- **Runtime deletion** — the last of `ADR-0036`'s three clauses ("no
  runtime deletion") still standing; a tombstone in the insert log is
  the obvious shape now that the log carries versions.
- **A server-side `Upsert`** — option (d); revisit if the second round
  trip ever shows in a measurement.
- **Compaction** of the insert and edge logs — unchanged; the trigger
  (`SERVER-INSERT-DESIGN.md`) is a log noticeably larger than its blob.
- **`memory_entities`** — the `ADR-0045` implementation round, next in
  the line.

## Change history

- 2026-09-06: Initial proposal; implementation follows on the same
  branch. The fourteenth round in the `rusty_remind_me`-motivated line;
  the one `ADR-0048` named as next.
