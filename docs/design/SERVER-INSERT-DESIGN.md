# Server Runtime Record Insertion (Proposed)

- Status: **Proposed** (2026-09-06). Implemented in the same delivery
  cycle under the owner's standing "continue maturing `multimodal_db`
  so it can back `rusty_remind_me`" mandate — the `ADR-0011` precedent
  (design and implementation in one cycle, reasoned through in the
  ADR's own Context rather than defaulted to silently), not the
  two-PR propose-then-accept cadence `ADR-0036`–`ADR-0045` followed.
  Every change is additive and reversible: one appended request, one
  appended error code, one new trait, one new companion file that
  only exists once something has been inserted. See `ADR-0046` for
  the acceptance options the owner still decides between.
- Date: 2026-09-06
- Related: `ADR-0036`/`docs/design/SERVER-REMINDER-DOMAIN-DESIGN.md`
  (the "no runtime deletion, fixed schema" invariant this round
  narrows to "no runtime deletion"), `ADR-0037`/`ADR-0039`/`ADR-0040`/
  `ADR-0042` (`Entity` — whose `entity_id(name)` exists so a *caller*
  can mint the id of a record it is about to create), `ADR-0045`
  (gate (ii): a `Memory` domain is the obvious second table — which is
  useless without a way to add a memory), `STORAGE-015` (the record
  blob this round's insert log sits beside), `STORAGE-017` (`SlotFile::
  append_committed_slots`, the `O_APPEND` slot append every durable
  layer already uses at `open` and now uses at runtime), `ADR-0025`/
  `ADR-0026` (the batch journal — the length-prefixed, torn-tail-
  tolerant file shape the insert log copies), `ADR-0022`/`SERVER-002`
  (the append-only wire rules; protocol 13), `ADR-0043` (the Python
  reference client, which gains `insert`).

## Purpose and scope

Every round in the `rusty_remind_me`-motivated line since `ADR-0036`
has shaped a *record* — `Reminder`, `Entity`, aliases, a derived id,
a join — and every one of them has run over a record set fixed at
`create`/`open`. This crate has never had a way to add a record to a
running store: no `Request` variant, no `ConnectionStore` method, no
trait in `crate::generic::query`, no method on `GenericMmapStore`.
`SERVER-REMINDER-DOMAIN-DESIGN.md` named the invariant as "no runtime
deletion, fixed schema"; it was in fact "no runtime deletion, no
runtime insertion, fixed schema", and the second clause is the one a
backend cannot live with.

Read against the consumer's own source (`baileyrd/rusty_remind_me` at
`29602f1`, the same shallow clone `ADR-0042` read), the three tool
calls this crate has been shaped to answer — `remind_me_add`,
`remind_me_set_reminder`, `remind_me_entity_upsert` — are all
*creates*: `db/queries.rs:98` `add_memory` mints `mem_<uuid4>` and
`INSERT`s; `entity.rs`'s upsert derives an id from the normalized name
and inserts on miss. A store that can only serve records it was
started with cannot back any of them. This is the strictly-prior gap:
a `Memory` domain (`ADR-0045`'s gate (ii)) without insertion would be
a table nothing can write to, while insertion without a `Memory`
domain already makes `Reminder` and `Entity` usable for
`set_reminder`/`entity_upsert` today.

This document proposes runtime record insertion, end to end:

1. a storage-layer primitive — `Insert<R>` in `crate::generic::query`,
   implemented by `GenericMmapStore` (a committed slot appended
   through the existing `O_APPEND` path, the record itself appended to
   a new companion **insert log** at `<path>.inserts`, durable before
   the call returns) and forwarded by every composition layer a
   front-door stack uses;
2. a server primitive — `ConnectionStore::insert_record` with a
   default that answers `Unsupported`, implemented by the two front-
   door adapters, `Reminder` and `Entity`;
3. a wire primitive — `Request::Insert { id, fields }` at protocol 13,
   answered `Ok` or a new `ErrorCode::Duplicate`, gated server-side
   below 13 exactly as sessions and `Join` are;
4. client surface — `SchemaDrivenClient::insert` and the Python
   reference client's `Client.insert`, both refusing below 13 with no
   frame sent.

## Non-goals

- **Not deletion.** "No runtime deletion" stands. A tombstone status
  (`ReminderStatus::Cancelled`) remains the pattern; the consumer's own
  `deleted_at` is a column, not a row removal, so this is not the gap
  it looks like.
- **Not relation (edge) insertion.** A record inserted into `Entity`
  has no `relates_to`/`mentioned_with` neighbors and no way, this
  round, to acquire any over the wire — `Symmetric`/`MultiSymmetric`
  forward `Insert` with an empty adjacency and their edge blobs are
  untouched. Named as the next round in this line (the consumer's
  `annotate`/`memory_entities` path), not folded in: it needs its own
  wire shape and its own edge-log durability story.
- **Not a server-minted id.** The client supplies the `RecordId`.
  `uuid`'s OS-randomness feature is deliberately off in this crate
  (reproducible generation, `Cargo.toml`), and the consumer mints its
  own ids on both of its paths — `mem_<uuid4>` for memories, a content
  hash for entities (`ADR-0042` finding; `crate::generic::entity::
  entity_id` exists for exactly this). A server-minted id would be a
  third scheme nobody asked for.
- **Not inside a session or a `Transaction` batch.** `TransactionOp`
  cannot grow a variant (rule 1), and an `Insert` while a session is
  open is answered `SessionOpen` — the `Transaction`-inside-a-session
  precedent. A consumer wanting "insert then update atomically" sends
  the insert with its final field values; every field of a record is
  set at insert, so there is nothing an immediately-following update
  would add.
- **Not journaled.** The redo journal (`ADR-0025`) records
  `TransactionOp` batches. An insert is durable on its own — the log
  entry is `fsync`ed before the slot is appended — so it needs no
  journal entry to survive a crash, and adding one would mean a second
  entry kind in a format that has none.
- **Not SQL `INSERT INTO`.** `src/server/sql.rs` is a read-only
  `SELECT` subset (`ADR-0034`); a write grammar is a different
  decision. `insert` is a method call with named fields.
- **Not `Dog`, `Order`, or `Employee`.** `Dog`'s `ProductionStore` is
  the bespoke `DogRecord` store, not `crate::generic`; `Order`/
  `Employee` are `research`-gated reference material. All three keep
  the trait's default and answer `Unsupported`. (`Order`'s stack —
  `Reversed<MmapScanned<GenericMmapStore>>` — *does* gain the store-
  level forwards, and its tests exercise them, so the generic library
  is covered end to end even though no research adapter exposes it.)
- **Not runtime compaction of the insert log.** The log is folded into
  the record blob at the next `open`/`open_portable` (see "Proposed
  shape"), so a long-running process's log grows by one entry per
  insert until it is reopened. A runtime `compact` would need the
  blob's persisted order preserved across an unordered `HashMap`, a
  small piece of bookkeeping this round does not add; the revisit
  trigger is a deployment whose log is measured to matter.
- **Not the `Memory` domain.** `ADR-0045` gate (ii) — its own round,
  next, now that a table can be written to.

## Context and terminology

### What a durable stack is made of today

`GenericMmapStore<R, IndexMarker, ScanMarker>` (`src/generic/
mmap_store.rs`) owns `records: HashMap<Id, R>`, one equality index,
`position_index: HashMap<Id, usize>`, and a `SlotFile` — the `.mmap`
file of `(id, scan value, commit marker)` slots. Its record *content*
lives in the companion blob `<path>.records` (`STORAGE-015`): one
`bincode` `Vec<R>` image, fingerprinted, rewritten whole by
`open` whenever the caller's record set differs from what the blob
holds, never touched by `update`. `open` already handles "a record
with no slot yet" by appending a committed slot through
`SlotFile::append_committed_slots` — one `O_APPEND` `write_all` per
slot, then a re-map (`STORAGE-017`).

Above it, every layer holds derived state built once at construction
from the record set: `Symmetric`/`MultiSymmetric` (adjacency, from an
edge list), `NameIndex` (normalized keys, from the records),
`Reversed` (children-of, from `parent_id`), `MmapScanned` (its own
slot file and `position_index`), and the in-memory `BaseStore`/
`Indexed`/`Scanned`.

So the pieces of a runtime insert already exist; what is missing is
(a) a way to persist one record's content without rewriting the whole
blob, and (b) each layer learning about a record it was not built
with.

### What the consumer writes

| Consumer path | Creates | Id minted by | This crate's counterpart |
|---|---|---|---|
| `add_memory` (`db/queries.rs:98`) | a `memories` row | client, `mem_<uuid4>` | none — `Memory` domain, `ADR-0045` (ii) |
| `set_reminder` (`reminders.rs`) | `remind_at` on a memory | client | `Reminder` (`ADR-0036`) — needs `Insert` |
| `entity_upsert` (`entity.rs`) | an `entities` row on miss | client, content hash | `Entity` (`ADR-0037`+) — needs `Insert`; `entity_id` (`ADR-0042`) is the id |

### Terminology

- **Insert log** — `<path>.inserts`, the append-only companion file
  holding every record inserted since the blob was last rewritten.
- **Fold** — `open`/`open_portable` merging the log's records into the
  record set, rewriting the blob, and truncating the log.
- **Duplicate** — an insert whose `id` already has a record; refused
  with nothing written, at every layer and on the wire.

## Requirements

- `INS-FR-001` **The `Insert<R>` trait.** `crate::generic::query::
  Insert<R: Record>` with one method, `fn insert(&mut self, record: R)
  -> Result<(), InsertError<R::Id>>`. `crate::generic::InsertError<Id>
  ::{Duplicate(Id), Durability(DurabilityError)}` beside `NotFound`.
  A `Duplicate` refusal writes nothing, at every layer.
- `INS-FR-002` **`GenericMmapStore::insert`.** Refuse a duplicate id;
  append the encoded record to the insert log and `sync_data` it;
  append one committed slot seeded from the record's scannable value
  through `SlotFile::append_committed_slots` (the `O_APPEND` path);
  then update `records`, `index`, and `position_index`. `Ok` means the
  record is durable. `get`/`filter_eq`/`scan`/`update`/`all_ids` see
  it immediately.
- `INS-FR-003` **The insert log format.** `<path>.inserts`: the same
  28-byte tagged header the blob uses (`GENINSL\0` magic, version 1,
  a fingerprint field that is always zero because the log is
  fingerprinted per entry rather than whole, then `R::SCHEMA_TAG`'s
  hash), followed by entries of `u32` little-endian length + `bincode`
  `R`. A torn tail is dropped at read (the journal's rule); a wrong
  magic, version, or tag is `DurabilityError::RecordBlobUnreadable`
  naming the log path. A log written for one record type is refused
  when read as another.
- `INS-FR-004` **Fold at open.** `read_portable_records(path)` returns
  the blob's records in persisted order followed by the log's records
  in log order, skipping a log entry whose id the blob already holds
  (a fold that crashed between the blob rewrite and the log
  truncation is therefore idempotent). `open(records, path)` merges the
  log into `records` the same way before its existing reconciliation,
  so the blob rewrite that already happens for a changed record set
  covers the inserted records, and truncates the log only after the
  blob is current. `open_portable` inherits both. `create` starts with
  no log. A reopen with an empty or absent log writes nothing — the
  `STORAGE-015-FR-003` promise holds for every store nothing was
  inserted into.
- `INS-FR-005` **Every layer forwards.** `Symmetric`, `MultiSymmetric`
  (empty adjacency for the new id), `NameIndex` (its keys added),
  `Reversed` (indexed under its parent when `parent_id` is `Some`),
  `MmapScanned` (its own committed slot appended after the inner
  insert succeeds), and `BaseStore`/`Indexed`/`Scanned` (their maps)
  implement `Insert<R>` over an inner `Insert<R>`. Each layer computes
  what it needs from the record *before* forwarding, so no layer adds
  a `Clone` bound. `GenericProductionStore::insert<R>` takes the write
  lock.
- `INS-FR-006` **`ConnectionStore::insert_record`.** `fn insert_record
  (&self, id: RecordId, fields: Vec<(FieldRef, ScanValue)>) ->
  Result<InsertOutcome, ErrorCode>` with a default of `Err(ErrorCode::
  Unsupported)`; `InsertOutcome::{Inserted, Duplicate}`. `Reminder` and
  `Entity` implement it. Validation before any write: every tag the
  schema describes present exactly once with a value of its
  `ValueKind` (`Malformed` for a missing, repeated, or wrong-kind
  field; `UnknownField` for a tag the schema does not describe), then
  the domain's own rule (`Reminder`: the `status` discriminant, the
  same `status_from_u32` check `update_field` makes).
- `INS-FR-007` **`Request::Insert` at protocol 13.** `Request::Insert {
  id: RecordId, fields: Vec<(FieldRef, ScanValue)> }` appended at index
  21 — the shape of `Response::Record`, so a client can read a record
  and write one with the same code. Answered `Response::Ok`; a
  duplicate id is `Err { Duplicate }` with the new `ErrorCode::
  Duplicate` at index 11; `Unsupported` from an adapter without insert;
  `Malformed`/`UnknownField` per `INS-FR-006`. `Malformed` on a
  connection negotiated below 13 (rule 3, the session/`Join`
  precedent). `Duplicate` only ever answers `Insert`, so no
  `downgrade_for_version` arm. Refused `Unauthorized` for a `ReadOnly`
  token — the third write beside `UpdateField`/`Transaction`.
  `SessionOpen` while a session is open. `audit::RequestKind::Insert`.
- `INS-FR-008` **Clients.** `SchemaDrivenClient::insert(&mut self, id,
  fields: &[(&str, ScanValue)]) -> Result<(), ClientError>` — names
  resolved through the discovered schema (`UnknownField` locally), the
  server's `Duplicate` surfaced as `ClientError::Server(ErrorCode::
  Duplicate, _)`, `ClientError::Unsupported("insert")` below 13 with no
  frame sent. `clients/python`: `Client.insert(record_id, fields)`,
  `PROTOCOL_VERSION = 13`, `ErrorCode.Duplicate`.
- `INS-FR-009` **Pins.** Two golden vectors (`Request/Insert`,
  `Response/Err(Duplicate)`), `PROTOCOL_VERSION` 12 → 13, protocol
  table row 13, `SERVER-002` updated (§5.6, §5.7, §5.2 `ErrorCode`, §7,
  §8, §10), the three version pins moved.

## Considered options

**Where the inserted record's content lives.**
(a) **(proposed)** an append-only insert log folded into the blob at
the next open — O(record) per insert, one `fsync`, the blob's
persisted-order and single-image properties untouched, and a reopen
compacts for free through machinery that already exists. (b) Rewrite
the blob on every insert — O(n) per insert (76 B/record measured in
`STORAGE-015`: ~7.6 MB written per insert at 100 K records), and the
temp-then-rename dance on every write; correct but unusable past toy
sizes. (c) Change the blob to a per-record framed format — a
`BLOB_VERSION` bump touching every reader and `is_current_at`'s
fingerprint, for a benefit (a) delivers without a format change.
(d) Keep inserted records in memory only until a `flush` — an insert
that returns `Ok` and is then lost to a crash is exactly the promise
this crate's durability line has never broken.

**Who mints the id.** (a) **(proposed)** the client — see Non-goals.
(b) The server, `Response::Id` — needs OS randomness or a counter with
a persistence story of its own, and conflicts with `entity_id`'s
derived-id design.

**How the wire carries a record.** (a) **(proposed)** `Vec<(FieldRef,
ScanValue)>`, `Response::Record`'s own shape, every field required.
(b) Positional `Vec<ScanValue>` in schema order — one fewer `u16` per
field, but a silent misalignment when a schema adds a field, the
exact failure `FieldRef` tags exist to prevent. (c) Optional fields
with defaults — a per-domain default policy on the server for a
consumer that always sets every field anyway.

**Where the version gate sits.** (a) **(proposed)** server-side
`Malformed` below 13 — the session/`Join` precedent for a request
that mutates. (b) Client-side only, `Query`'s posture — chosen there
because a read has no consequence when it slips through; a write
does.

**Inside a session.** (a) **(proposed)** `SessionOpen`. (b) Stage it —
`TransactionOp` cannot grow (rule 1), so staging would need a parallel
staged-insert list, a second apply path in every adapter, and a
journal entry kind; all for a consumer whose inserts carry final
values.

## Proposed shape

### `src/generic/` (`INS-FR-001`–`005`)

```rust
// query.rs
pub trait Insert<R: Record> {
    fn insert(&mut self, record: R) -> Result<(), InsertError<R::Id>>;
}

// mod.rs
pub enum InsertError<Id> {
    Duplicate(Id),
    Durability(DurabilityError),
}
```

`GenericMmapStore::insert` (in the `SchemaTag`-bounded block, since it
writes the tagged log header on first use):

```text
if records.contains_key(id) -> Err(Duplicate(id))
insert_log::append(&log_path(path), &record)?   // header on first use, entry, sync_data
position = file.append_committed_slots([(id, scannable_value)])?[0]
records.insert(id, record); index[indexed_value].push(id); position_index.insert(id, position)
```

`read_portable_records`: `record_blob::read` then `insert_log::read`,
appending each log record whose id is not yet present. `open`: the
same merge applied to the caller's `records` before `build_indexes`;
after the existing blob rewrite (which now always fires when the log
was non-empty, because the merged set's fingerprint differs), the log
is truncated. A new `insert_log` module (`pub(crate)`, beside
`record_blob`) owns the header, append, read, and truncate.

Layer impls, each `where S: Insert<R>`:

| Layer | Before forwarding | After `inner.insert` succeeds |
|---|---|---|
| `BaseStore` | duplicate check | `records.insert` |
| `Indexed` | `indexed_value().clone()` | `index[value].push(id)` |
| `Scanned` | `scannable_value()` | `position_index[id] = cache.len(); cache.push` |
| `Symmetric`/`MultiSymmetric` | — | — (no edge; `neighbors` answers `[]`) |
| `NameIndex` | `index_keys()` | each normalized key's bucket gains `id` |
| `Reversed` | `parent_id()` | `children_of[parent].push(id)` if `Some` |
| `MmapScanned` | `scannable_value()` | `file.append_committed_slots`; `position_index` |

`GenericProductionStore::insert<R>(&self, record: R)` under the write
lock — the `update` shape.

### `src/server/` (`INS-FR-006`–`007`)

```rust
pub enum InsertOutcome { Inserted, Duplicate }

pub trait ConnectionStore {
    // ...
    fn insert_record(&self, _id: RecordId, _fields: Vec<(FieldRef, ScanValue)>)
        -> Result<InsertOutcome, ErrorCode> { Err(ErrorCode::Unsupported) }
}
```

`ReminderConnectionStore::insert_record`: a `reminder_from_fields(id,
fields) -> Result<Reminder, ErrorCode>` helper does the schema check
of `INS-FR-006` and the discriminant check, then `store.insert`;
`InsertError::Duplicate` → `Ok(Duplicate)`. `InsertError::Durability`
— the log or slot append failed, nothing applied — has no exact code
on the wire today: `Unsupported` and `Malformed` would both lie.
**Proposed:** carry it as `ErrorCode::Journal`, whose documented
meaning ("could not be made durable before applying; nothing was
applied") is precisely this case and which already rides an `Err`
shape; a dedicated `Storage` code for a path no test reaches without
fault injection is speculative this round. Named in `ADR-0046` as the
one reuse a reader might question, and in "Open questions" below.

`EntityConnectionStore::insert_record`: the same shape over four
fields, `aliases` accepted as a `StrList` (the first *write* of a
`StrList` — `ENT4-FR-003`'s "read-only" describes `UpdateField`, not
a whole-record insert, and the design text of `ADR-0041` names
aliases as "durable for free, the record blob is `Vec<Entity>`
serialized whole", which the log preserves).

`dispatch`: `Request::Insert { id, fields } => match store.
insert_record(id, fields) { Ok(Inserted) => Ok, Ok(Duplicate) =>
err(Duplicate), Err(code) => err(code) }`. `handle_connection`: the
`ReadOnly` gate's `matches!` gains `Insert`; `Insert if session.
is_some()` → `SessionOpen`; `Insert if negotiated < 13` → `Malformed`.
`outcome_of` unchanged (an `Err` is an `Err`). `error_message(
Duplicate)`.

### `src/server/client.rs` and `clients/python` (`INS-FR-008`)

```rust
pub fn insert(&mut self, id: RecordId, fields: &[(&str, ScanValue)])
    -> Result<(), ClientError>
```

Resolves every name through `self.field(name)` (so an unknown name is
`ClientError::UnknownField` locally), refuses below 13, sends
`Request::Insert`, maps `Ok` → `Ok(())`, `Err` → `ClientError::Server`.
No capability flag is consulted — a field's `update`/`scan`/
`filter_eq` flags describe what can be done *to* a stored field, not
whether it can be set at creation; every field can. Python mirrors
it: `Client.insert(record_id, fields: Sequence[Tuple[str, Any]])`,
values coerced through `_to_scan_value` by the field's kind.

## Data/state and invariants

- A record is in `records` iff it has a committed slot in `position_
  index` iff its content is in the blob or the log. `insert`
  establishes the third before the second before the first; a crash at
  any point leaves a state `open` already reconciles (a log entry with
  no slot gets one appended; a slot with no record is the existing
  "stale" case).
- The log holds only records the blob does not, except transiently
  between a fold's blob rewrite and its truncation — and the fold
  skips those by id.
- `InsertError::Duplicate` at any layer means nothing was written at
  any layer: the innermost store checks first, and every outer layer
  mutates only after the inner `Ok`. The one exception — `MmapScanned`
  appending its slot after the inner insert — cannot produce a
  duplicate (the inner store already refused one) and on an I/O
  failure leaves a record the next `open` reconciles by appending the
  missing slot.
- No existing file changes format. A store nothing was inserted into
  has no `.inserts` file and behaves byte-for-byte as before.
- The fold widens the record set the core holds, so a relationship
  layer built from a *caller-supplied* list at reopen (`Reversed::new`,
  `MmapScanned::open` in the `research`-gated `Order` helpers) knows
  only what that list knows. The contract is unchanged in kind — a
  caller's list has always had to be current — and every `*_portable`
  helper builds from `read_portable_records`, which includes the log.
  Named on `open`'s own doc comment.

## Errors, failure, recovery, and observability

- `InsertError::Duplicate` / `ErrorCode::Duplicate` — the id has a
  record; nothing written.
- `InsertError::Durability(Io)` — the log or slot append failed; over
  the wire `ErrorCode::Journal` (see "Proposed shape" for why).
- `RecordBlobUnreadable { path: <path>.inserts, cause }` at
  `read_portable_records`/`open` — a foreign, wrong-version, or
  wrong-type log, refused by name, never silently ignored (the blob's
  own rule).
- A torn log tail (a crash mid-entry) is dropped at read, like the
  journal's; the entry it belonged to was never acknowledged.
- Every `Insert` reaches the access log as `RequestKind::Insert` with
  its outcome; a refused `ReadOnly` insert reaches the audit log as
  `Refused { Unauthorized }`, the `UpdateField` shape.

## Security, privacy, and compatibility

- A `ReadOnly` token cannot insert; an unauthenticated connection on
  an authenticated server cannot reach `dispatch` at all — unchanged
  gates, one more request under them.
- Input at the boundary: the adapter validates the whole field list
  against the schema before the store sees a byte; an oversized
  `fields` is bounded by the frame cap (16 MiB) like any request.
- Wire: append-only (rule 1), one version (rule 2), `Malformed` below
  13 (rule 3), the client never sends below 13 (rule 4). Every vector
  ≤ 12 is byte-identical.
- On disk: no existing file's format changes; the new file is created
  on first insert only.

## Acceptance criteria

1. `GenericMmapStore::insert` on a fresh `Reminder` store: the record is
   readable by `get`/`filter_eq`/`scan`/`all_ids` immediately; `update`
   on it works; a second insert of the same id is `Duplicate` and
   changes nothing; after drop, `open_portable(path)` returns the
   original records followed by the inserted one, the log is empty,
   and the blob holds all of them (a second `open_portable` writes
   nothing).
2. A log with a torn trailing entry replays every complete entry; a
   log written for another `SchemaTag` is refused by name; a fold that
   crashed after the blob rewrite (simulated: a log entry duplicating a
   blob record) yields no duplicate.
3. Every layer: `Entity` (`NameIndex<MultiSymmetric<GenericMmapStore>>`)
   — an inserted entity is found by `find_by_name` under its label and
   each alias, has no neighbors under either label, and survives
   `open_entity_production_stack_portable`; `Order` (`Reversed<
   MmapScanned<GenericMmapStore>>`, `research`) — an inserted order
   appears under its customer's `children` and both durable fields
   round-trip through reopen; the in-memory `Dog`/`Order` generic
   stacks accept an insert and refuse a duplicate.
4. Over a real socket on `Reminder` and `Entity`: `insert` then `get`
   returns the fields sent; `Duplicate` on repeat with the record
   unchanged; a missing, repeated, wrong-kind, or unknown field is
   `Malformed`/`UnknownField` with nothing inserted; a bad `status`
   discriminant is `Malformed`; a `ReadOnly` token is `Unauthorized`;
   inside a session `SessionOpen` and the session still commits; the
   server restarted on the same directory serves the inserted record.
5. `Hello { 12 }` and a silent client are answered `Malformed` for
   `Insert` with the connection still open; the Rust client refuses
   below 13 with no frame; a pre-hello version-1 server never receives
   one.
6. The Python client inserts a record the Rust client then reads; the
   fixture gains exactly two lines at 13 and every earlier line is
   unchanged.
7. `Dog`'s adapter answers `Unsupported`; `dispatch` on the unit
   fixture answers `Ok`/`Duplicate`/`Unsupported` per the adapter.
8. Full sweep green: `cargo fmt --all -- --check`, `cargo clippy
   --all-targets --all-features -- -D warnings`, `cargo test`, `cargo
   test --all-features`, `cargo test --features client`, `cargo doc
   --all-features --no-deps` with no new warning, the Python vector
   test.

## Verification plan

Unit tests in `src/generic/insert_log.rs` (format, torn tail, foreign
tag, empty/absent), `src/generic/mmap_store.rs` (criterion 1, the
fold, the crashed-fold idempotence), `src/generic/store.rs`
(`BaseStore`/`Indexed`/`Scanned`/`NameIndex`/`Reversed` forwards),
`src/generic/mmap_scanned.rs` (its slot), `src/generic/reminder.rs`
and `src/generic/entity.rs` (the stacks, criterion 3), `src/generic/
order_customer.rs` (`research`, criterion 3), `src/server/reminder.rs`
and `src/server/entity.rs` (field validation, criterion 4's refusals
without a socket), `src/server/serve.rs` (`dispatch`, the gates),
`src/server/protocol.rs` (the two vectors, the pin). Integration:
`tests/server_reminder_integration.rs` and `tests/server_entity_
integration.rs` (criterion 4 over a socket, the restart), `tests/
server_protocol_version.rs` (criterion 5), `tests/server_python_
client.rs` (criterion 6), `tests/server_dog_integration.rs`
(criterion 7), `tests/server_auth_integration.rs` (`ReadOnly`),
`tests/server_transaction_integration.rs` (`SessionOpen`).

## Traceability

- Roadmap: `SERVER-INSERT-DESIGN` (this document), `SERVER-INSERT`
  (the implementation).
- Decision: `ADR-0046`.
- Specification: `SERVER-001` v0.36.0 / `FR-046`; `SERVER-002` v0.2.0.
- Requirements: `INS-FR-001`–`009` above.

## Open questions

- Whether `ErrorCode::Journal` is the right carrier for an insert's
  durability failure (see "Proposed shape") or a dedicated `Storage`
  code should be appended at 13 while the version is being bumped
  anyway — the owner's call at acceptance; adding it later costs a
  version.
- Whether `Order`/`Employee` should expose `insert_record` now that
  their stacks support it — they are `research`-gated reference
  material, so the answer is "when a test needs it", not "for
  completeness".
- Runtime compaction (see Non-goals): revisit when a deployment's log
  size is measured, not before.
- Relation insertion — the next round in this line.

## Change history

- 2026-09-06: Initial proposal, implemented in the same cycle (see
  Status). The eleventh round in the `rusty_remind_me`-motivated line
  `ADR-0036` started; the first that changes what a running store can
  hold.
