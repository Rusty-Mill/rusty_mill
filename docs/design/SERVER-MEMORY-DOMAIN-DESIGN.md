# Server Memory Domain Design (Accepted)

- Status: **Accepted** (2026-09-06, `ADR-0048` option (a), as designed —
  the eleven-field projection, `Reminder`'s stack, no wire change; the
  whole table, `memory_associations` now, replacement first, and
  declining all declined). Implemented on the same branch as
  `SERVER-001` v0.38.0 / FR-048, PR #200.
- Date: 2026-09-06
- Related: `ADR-0036`/`docs/design/SERVER-REMINDER-DOMAIN-DESIGN.md`
  (the round that scoped `Memory` *out* — "schema-less memory content …
  a genuinely different kind of database" — and whose stack shape this
  domain reuses exactly), `ADR-0037`/`ADR-0039` (`Entity` — the
  open-`String` classification precedent for `category`, the plain
  `i64` counter precedent for `access_count`), `ADR-0041` (`ScanValue::
  StrList`, which `tags` rides on), `ADR-0045` (whose gate (ii) — "a
  front-door second table is designed (a `Memory` domain is the
  obvious candidate)" — this round meets), `ADR-0046`/`ADR-0047` (the
  two rounds that made a table writable at runtime, without which a
  `memories` table would be a fixture nobody could add to).
- Supersedes/Superseded by: none. Adds one new domain; changes no
  existing `Request`/`Response` variant, no `PROTOCOL_VERSION`, no
  `ErrorCode`, no existing domain's behavior. `SERVER-002` is unchanged.

## Purpose and scope

The `rusty_remind_me` line so far gave this crate a `Reminder` table
(`ADR-0036`), an `Entity` graph (`ADR-0037`–`ADR-0042`), and — in the
two rounds immediately before this one — the ability to create records
and edges in a running store (`ADR-0046`, `ADR-0047`). What it has not
given the consumer is the table the consumer is *about*: `memories`,
the thirty-column table every one of its tools reads or writes. Every
preceding round either scoped it out explicitly (`ADR-0036`) or named
it as the next thing (`ADR-0045` gate (ii), `ADR-0046`'s and
`ADR-0047`'s open questions).

This round designs and builds that table as this crate's sixth domain
and third front-door one — as a **bounded projection**, not the whole
thirty columns. The consumer's source at `29602f1` was read again for
this document: `schema_tables.sql:56-64` (the table), `db/queries.rs:
98-140` (`add_memory` — what a memory *is* at creation), `db/queries.rs`
`list_filters` (which columns every list filters on: `category`,
`source`, `tags`, `sensitive`), and the `access_count` bump on every
retrieval. The eleven fields below are the ones those paths touch;
the nineteen omitted are named, each with its reason, in "Non-goals".

Scope of this round, exactly: the `Memory` record and its durable
stack (`MEM-FR-001`–`005`), a `ConnectionStore` adapter and journal
support (`MEM-FR-006`), a `memory_server` binary (`MEM-FR-007`), and
the tests that prove the consumer's read and write shapes over a real
socket (`MEM-FR-008`). **No wire change of any kind**: protocol stays
14, `SERVER-002` stays 0.3.0, every existing vector is untouched.

## Non-goals

Each omission below is a column (or column group) of the consumer's
`memories` table, with the reason it is not in this projection. None
is a judgment that the column is unimportant; each is either
something this crate's wire cannot carry today, or something a named
later round owns.

- **`decay_rate`, `vitality`, `base_weight`** — the retrieval-scoring
  triple. All `f64`; no stored `ValueKind` carries a float. Adding one
  is a wire change (a new `ScanValue` variant, a protocol bump) and a
  design question of its own (does `UpdateField` move it? does
  `Aggregate` average it?). Not this round.
- **`subject`, `predicate`, `object`** — the SPO decomposition
  (`remind_me_decompose`). Nullable; the wire has no null. An empty
  string would conflate "not decomposed" with "decomposed to nothing".
  Not this round.
- **`superseded_by`** — a nullable self-reference. Same null problem,
  and it is really a directed edge (`ADR-0042` F4), which the edge
  machinery does not have.
- **`node_id`, `client`** — sync bookkeeping the consumer's own
  replication layer owns; a backend stores what it is handed.
- **`doc_id`, `chunk_index`** — document chunking; a second table's
  foreign key, `ADR-0045`'s territory.
- **`remind_at`** — the `Reminder` domain's job (`ADR-0036`), and a
  nullable timestamp besides.
- **`deleted_at`** — soft deletion. This crate still has no runtime
  deletion of any kind, and `status` (`active`/`archived`/…) already
  carries the consumer's lifecycle state as an open string.
- **`embedding`, full-text search** — named non-goals since
  `ADR-0036`; unchanged.
- **Whole-record replacement** — the consumer's `update_memory`
  rewrites `content`/`category`/`tags`/`metadata`/`sensitive` in
  place. `UpdateField` moves one `Copy` scannable value; there is no
  request that replaces a record. This is the **next round** in the
  line, not this one, and this document names it so it is not
  rediscovered: a `Request::Replace { id, fields }` mirroring `Insert`
  over an existing id, riding the insert log's format.
- **`memory_entities`, `memory_associations`** — the consumer's two
  edge tables. The first is cross-table (memory → entity), `ADR-0045`'s
  round. The second (memory ↔ memory) could be a `MultiSymmetric` over
  this record exactly as `Entity` has; it is left for the round that
  also gives this table its second table, so `Memory`'s stack changes
  once, not twice.

## Context and terminology

- **Projection**: a record type carrying a chosen subset of a source
  table's columns, with the mapping stated. The consumer's `id`
  (`mem_<uuid4 simple>`, opaque per its `ADR-0016`) maps to `Uuid` by
  stripping the prefix on the way in and restoring it on the way out.
- **Front-door domain**: unconditional in `crate::generic`, `server`-
  gated alone in `crate::server` (`Reminder`, `Entity`), as opposed to
  `research`-gated reference material (`Order`, `Employee`).
- **The one-index, one-scan stack**: `GenericMmapStore<R, IndexMarker,
  ScanMarker>` — one equality-filterable `IndexedField`, one durably-
  mutable `ScannableField`, no relation layer. `Reminder`'s shape.

## Requirements

- `MEM-FR-001` **The record.** `crate::generic::memory::Memory { id:
  Uuid, content: String, category: String, tags: Vec<String>, source:
  String, metadata_json: String, created_at_unix_ms: i64,
  updated_at_unix_ms: i64, memory_type: String, status: String,
  sensitive: bool, access_count: i64 }`, `SCHEMA_TAG = "memory::Memory"`,
  unconditional (compiled under default features).
- `MEM-FR-002` **`category` is the `IndexedField`** — the consumer
  indexes it and every list filters on it. An open `String`, not an
  enum (`Entity::kind`'s precedent, `ADR-0039`): the consumer's
  categories are user-supplied.
- `MEM-FR-003` **`access_count` is the `ScannableField`** — a plain
  `i64`, durably mutable through `UpdateField`, with one domain rule:
  never negative (`Malformed` on the wire, both in `UpdateField` and in
  a `Transaction`/session precondition path).
- `MEM-FR-004` **Everything else is read-only after insert.** `content`,
  `tags`, `source`, `metadata_json`, both timestamps, `memory_type`,
  `status`, `sensitive` are readable by `GetById`/`ScanAll`/`Query`/
  `Aggregate` and refused by `UpdateField` with `Unsupported`.
  `metadata_json` is carried as text the store never parses — the
  honest way to hold "schema-less content" in a fixed-schema record.
- `MEM-FR-005` **No relation of either kind.** `MemoryProductionStack =
  GenericMmapStore<Memory, CategoryField, AccessCountField>`;
  `create_memory_production_stack`/`open_memory_production_stack`/
  `open_memory_production_stack_portable` mirror `Reminder`'s three.
  Every relation request (`Parent`, `Children`, `Neighbors`,
  `NeighborsByRelation`, `ListRelationKinds`, `Link`, `Join`) is
  `Unsupported`; `describe_relations` reports none.
- `MEM-FR-006` **The adapter.** `crate::server::memory::
  MemoryConnectionStore` wrapping `GenericProductionStore<
  MemoryProductionStack>`, `server`-gated alone; `new`/`with_journal`;
  `describe` reports eleven fields in tag order with `category`
  `filter_eq`, `access_count` `scan` + `update`, `tags` as `StrList`;
  `insert_record` requires every field exactly once (`Malformed`/
  `UnknownField`), `Duplicate` on a present id; `apply_transaction` on
  the journaled and plain paths; `CheckpointFlush` for the stack.
- `MEM-FR-007` **A real binary.** `src/bin/memory_server.rs`,
  `required-features = ["server"]`, `dog_server`'s env-var surface
  unchanged, default `127.0.0.1:7881`.
- `MEM-FR-008` **Proof over a socket.** `tests/server_memory_integration
  .rs`: the consumer's read shapes (`get`, `filter_eq category`, `WHERE
  sensitive = false`, `WHERE source = '…'`, `GROUP BY category`), the
  write shapes (`insert` with all eleven fields, `access_count` bump,
  the negative refused, a `content` update `Unsupported`), and a
  **second server on the same directory serving the inserted memory**
  (the `ADR-0046` fold, now on a third domain).

## Considered options

- **(a) A bounded eleven-field projection, `Reminder`'s stack shape,
  no wire change — proposed.** Buildable entirely from what the
  library already has; the omitted columns each have a stated reason
  and, where a later round owns them, a name.
- **(b) The whole thirty-column table.** Needs a float `ValueKind`, a
  null story, a directed edge, and soft deletion — four wire-level
  designs — before a single memory can be stored. The projection
  delivers the consumer's `add`/`get`/`list` paths now and leaves each
  of those four as its own decision.
- **(c) Also add `memory_associations` as a `MultiSymmetric` this
  round.** Cheap in isolation (`Entity`'s stack), but `memory_entities`
  — the edge the consumer actually traverses — is cross-table and
  needs `ADR-0045`; adding one edge kind now and the other later
  changes `Memory`'s stack twice.
- **(d) Wait for whole-record replacement first.** `update_memory` is
  real consumer surface, but so is `add_memory`, and a table that can
  be inserted into and read is useful before it can be edited; the
  replacement round has a smaller diff *with* a domain that needs it.

## Proposed shape

`src/generic/memory.rs` — the record, `CategoryField`, `AccessCountField`,
the stack alias, the three constructors, one unit test walking create/
get/filter/scan/update/insert/portable reopen. `src/server/memory.rs`
— the field tags (`FIELD_CONTENT` 0 … `FIELD_ACCESS_COUNT` 10),
`READ_ONLY_FIELDS`, `valid_access_count`, the adapter with one
`fields_of` as the single source of the wire shape, four unit tests.
`src/server/journal.rs` — `CheckpointFlush` for the stack. `src/bin/
memory_server.rs` — three sample memories. `tests/server_memory_
integration.rs` — three tests. `Cargo.toml` — the `[[bin]]`/`[[test]]`
entries only, no dependency.

Wire shape of one memory, as `describe` reports it:

| tag | name | kind | filter_eq | scan | update |
|---|---|---|---|---|---|
| 0 | `content` | `Str` | | | |
| 1 | `category` | `Str` | yes | | |
| 2 | `tags` | `StrList` | | | |
| 3 | `source` | `Str` | | | |
| 4 | `metadata_json` | `Str` | | | |
| 5 | `created_at_unix_ms` | `I64` | | | |
| 6 | `updated_at_unix_ms` | `I64` | | | |
| 7 | `memory_type` | `Str` | | | |
| 8 | `status` | `Str` | | | |
| 9 | `sensitive` | `Bool` | | | |
| 10 | `access_count` | `I64` | | yes | yes |

## Data/state and invariants

- One directory: `memories.mmap` (+ `.records`, `.inserts` per
  `ADR-0046`). No edge files, no manifest.
- `access_count >= 0` always; enforced at the adapter, the store holds
  whatever it is given (a library caller is trusted, as everywhere).
- The id bridge is lossless: `mem_` + `Uuid::simple()`.

## Errors, failure, recovery, and observability

Nothing new: `Malformed` for a negative `access_count` or a bad field
list, `UnknownField` for a tag ≥ 11, `Unsupported` for a read-only
field or any relation request, `Duplicate`/`Storage` from `Insert`
exactly as `ADR-0046` defined them. Recovery is the insert-log fold.

## Security, privacy, and compatibility

`sensitive: bool` is stored and filterable but **not enforced** — the
consumer's redaction of sensitive memories is its own policy layer,
as it is today over SQLite. No wire change, so every existing client
at any protocol version is unaffected; a version-1 client sees the
eleven fields through `DescribeSchema` as it would any domain.

## Acceptance criteria

1. Under default features `Memory` compiles and its unit test passes;
   under `server` the adapter's four and the integration suite's three
   pass; `cargo test --all-features` is green throughout.
2. Over a socket: `DescribeSchema` reports the eleven names in tag
   order; `get` returns every field; `filter_eq category`, `WHERE
   sensitive = false`, `WHERE source = 'import'`, `GROUP BY category`
   return the consumer's shapes; `insert` with all eleven fields is
   `Inserted`; `access_count` bumps; a negative is `Malformed`; a
   `content` update is `Unsupported`; a second server on the same
   directory serves the inserted memory.
3. Every relation request on `Memory` is `Unsupported`, client- and
   server-side.
4. No change to `src/server/protocol.rs`, `tests/fixtures/wire-
   vectors.txt`, `clients/python/**`, or `SERVER-002`.

## Verification plan

`cargo test --all-features` (unit + integration), `cargo test`
(default — proves front-door status), `cargo test --features client`,
clippy `-D warnings` on the same three, `cargo fmt --check`, the
Python vector suite (unchanged, proving criterion 4), `cargo doc
--all-features --no-deps` at the baseline warning count.

## Traceability

- Roadmap: `SERVER-MEMORY-DOMAIN-DESIGN`, `SERVER-MEMORY-DOMAIN`.
  Decision: `ADR-0048`.
- Specification: `SERVER-001` v0.38.0 / `FR-048`. `SERVER-002`
  unchanged.
- Requirements: `MEM-FR-001`–`008`.

## Open questions

- **Whole-record replacement** — the consumer's `update_memory`; the
  next round (a `Request::Replace` over the insert log's format).
  *Resolved by `ADR-0049` / `SERVER-001` v0.39.0: `Request::Replace` at
  protocol 15; every `Memory` field is now replaceable whole.*
- **A float `ValueKind`** for the scoring triple — a wire design of its
  own; until then the consumer keeps scoring in its own layer.
- **Null** — three nullable columns are omitted for want of it; an
  `Option`-carrying `ScanValue` is a protocol bump.
- **`memory_entities`** — cross-table, the `ADR-0045` implementation
  round, for which this domain is now the second table. *Resolved by
  `ADR-0050` / `SERVER-001` v0.40.0: `Memory::mentions`, a foreign
  label to `entity`, joined across at protocol 16.*
- **Soft deletion** — `deleted_at`; or the first runtime deletion.

## Change history

- 2026-09-06: Accepted as designed (option (a)), after PR #200. No
  content change.
- 2026-09-06: Implemented as `SERVER-001` v0.38.0 / FR-048, landed as
  designed (PR #200).
- 2026-09-06: Initial proposal; implementation follows on the same
  branch. The thirteenth round in the `rusty_remind_me`-motivated
  line; `ADR-0045` gate (ii) met.
