# Server Memory Sync Fields Design (Accepted)

- Status: **Accepted** (2026-09-07, `ADR-0056` option (a) — N1 sentinels
  plus L1 a schema-tag bump with a distinct failure and no upgrade; the
  owner's pick after a design-only proposal, PR #216). Implemented as
  `SERVER-001` v0.46.0 / FR-056 on the branch that followed the pick.
- Date: 2026-09-07
- Related: `ADR-0048`/`docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md` (the
  projection this widens, and its "Null" non-goal), `ADR-0053` (the
  durable directory this must not break), `STORAGE-015`/
  `docs/design/GENERIC-STORE-PORTABILITY-DESIGN.md` (the record blob),
  `ADR-0046` (the insert log), `ADR-0051` (hard deletion, which stays),
  `ADR-0055` (`Page`, which makes `deleted_at` walkable),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 3, and gap 4's
  `compact_tombstones`/`count_by_origin_node`/`stats.tombstones`).
- Supersedes/Superseded by: none proposed. Depending on the option:
  a `Memory` layout change with a schema-tag bump, or a record-blob and
  insert-log format bump; no wire change under the recommended option.

## Purpose and scope

The hub keeps two things about a memory that the projection does not:
**when it was soft-deleted** (`deleted_at`, a nullable timestamp — a
tombstone other nodes must still see until a cutoff, then purge) and
**which node wrote it** (`node_id`, a nullable string). Without them
three of the fourteen `HubStore` methods cannot be honest:
`compact_tombstones` cannot identify what to purge, `stats.tombstones`
is always zero, `count_by_origin_node` is always empty. The spike named
this gap 3.

Two decisions are bundled because the second is forced by the first:

1. **How this wire says "no value."** Every `ScanValue` is a value; the
   projection has omitted every nullable column so far.
2. **How a record's layout may change once directories are durable.**
   The record blob is `Vec<Memory>` in `bincode` with no per-field
   markers: a blob written with twelve fields read as fourteen does not
   fail cleanly — it mis-decodes. The schema tag in the blob, the insert
   log, every edge blob, and the label manifest identifies the *type*,
   not its layout.

Scope, if accepted: two fields on `Memory` (`deleted_at_unix_ms`,
`node_id`), their wire tags 11 and 12, `describe`, validation, the
adapters' `fields_of`/`from_fields`, the sample data, and whichever
layout mechanism the chosen option names. `Delete` stays the hard
delete; the server attaches no meaning to `deleted_at` — a soft-deleted
memory is data, filtered and purged by the client.

## Non-goals

- **Server-side soft deletion semantics** — no request marks a record
  deleted, no read hides one. The hub's model is "the record with a
  timestamp", and that is exactly what the field is.
- **A `PurgeBefore` request** — `Query`/`Page` over `deleted_at` plus
  `Delete` per id is the client's loop; a server request is a later
  round if the loop's round trips matter.
- **The scoring floats, the SPO triple, the capture ids** — the other
  fourteen omitted columns stay omitted; the floats still need a float
  `ValueKind`, the rest a caller.
- **Changing the scannable or indexed field** — `category` and
  `access_count` keep their roles; `deleted_at` is walked by `Page`
  (`ADR-0055`) and filtered by `Query`, both full scans, milliseconds
  at target scale.

## Context and terminology

- **Sentinel**: a value of the field's own kind that the domain never
  produces, standing for "none": `0` for a millisecond timestamp (the
  epoch is not a deletion time), `""` for a node id (a node has a name).
  Lossless for these two columns; the consumer already renders both
  through `Option` at its own edge.
- **Layout**: the byte shape of one record in the blob and the log —
  the field list and order `bincode` writes. Today unversioned.
- **Schema tag**: `SchemaTag::SCHEMA_TAG`, hashed into every companion
  header; a mismatch is `RecordBlobUnreadable`, distinct and named.

## Considered options — the null representation

- **(N1) Sentinels, documented per field — recommended.**
  `deleted_at_unix_ms: i64` (`0` = live; validated `>= 0`), `node_id:
  String` (`""` = unattributed). No wire change; `Page` by
  `deleted_at` (live rows first, then tombstones oldest-first),
  `Query WHERE deleted_at_unix_ms > 0 AND deleted_at_unix_ms < cutoff`
  for the purge loop, `Aggregate COUNT WHERE deleted_at_unix_ms > 0`
  for `stats.tombstones`, `GROUP BY node_id` for `count_by_origin_node`
  all work today. Cost: a convention a caller must know, stated in the
  schema docs and the field's descriptor docs.
- **(N2) A nullable `ScanValue`** — `ScanValue::Null` plus a `nullable`
  bit on `FieldDescriptor`, at a protocol bump. The general answer, and
  the one every later nullable column would use; but it touches every
  read and write path, every predicate rule (`NULL` ordering, `IS
  NULL`), both clients, and the golden fixture, for two fields whose
  sentinels lose nothing. Named as the round to run the day a column
  arrives with no lossless sentinel.
- **(N3) A presence field per nullable column** (`has_deleted_at:
  Bool`) — two fields per column, and a pair that can disagree.

## Considered options — the layout change

- **(L1) A tag bump and a distinct failure, no upgrade — recommended
  for this round.** `SCHEMA_TAG = "memory::Memory@2"`. A directory
  written at the old layout fails on open with `RecordBlobUnreadable`
  naming the record blob — never a mis-decode, never a silent
  recreation (`ADR-0053`'s rule) — and `memory_server` says so on its
  startup line with the remedy: re-push from the consumer, whose SQLite
  is the source of truth for every memory a hub holds. Cost: nothing
  but the string. Why it is enough now: `ADR-0053` landed today, no
  deployment holds a layout-1 directory, and a sync target with a
  source of truth elsewhere is rebuilt by a full push. Why it is not
  enough forever: the day a directory cannot be re-pushed, (L2) is the
  round — named as the trigger.
- **(L2) A layout version on the record type, with an upgrade.**
  `SchemaTag::LAYOUT: u32` (default 1) written into a version-3 record
  blob and a version-3 insert log (the two files that carry record
  bodies; edge blobs and the manifest carry ids only and stay at their
  versions); a mismatch is a new `DurabilityError::LayoutMismatch {
  path, found, expected }`; a version-2 blob or log reads as layout 1.
  `GenericMmapStore::upgrade_layout::<Old, New>(path, map)`: read the
  blob and fold the log as `Old`, map each record, write the version-3
  blob and clear the log — two atomic temp-and-rename writes, the slot
  file untouched (the scannable field is unchanged), edge files
  untouched. `Memory` keeps a private `MemoryLayout1` as the decoding
  type. `open_or_create_memory_production_stack` upgrades automatically
  when it finds layout 1 and the startup line reports it. The
  principled mechanism, and two format bumps with compatibility readers
  — a round of its own.
- **(L3) Tolerant decoding** — impossible: `bincode` fixint writes no
  field markers, so a shorter body cannot be told from a corrupt one.
- **(L4) A tag bump and a rewrite of every tagged companion** — blob,
  log, each edge blob and edge log, the manifest: five or more files
  whose rewrite is atomic each but not as a set, for an outcome (L2)
  gets with two.

## Proposed shape (N1 + L1)

`src/generic/memory.rs` (two fields, `SCHEMA_TAG` bumped, docs),
`src/server/memory.rs` (`FIELD_DELETED_AT` 11, `FIELD_NODE_ID` 12,
`fields_of`, `memory_from_fields` with `deleted_at_unix_ms >= 0`,
`describe`), `src/bin/memory_server.rs` (sample data; the startup-line
remedy for a layout-1 directory), every test that spells out a
`Memory` or its eleven wire fields (`server_memory_integration`,
`server/memory`, `generic/memory`, the Python driver if it touches
`memory`). No `src/server/{protocol,serve,client}.rs`,
`clients/python/**`, or fixture change; `PROTOCOL_VERSION` stays 20.
The consumer's spike adapter sends the two fields and stops returning
`Err` from `compact_tombstones`.

## Acceptance criteria (for the implementation round)

1. A twelve-field directory opened by the new build fails with
   `RecordBlobUnreadable` naming the record blob; nothing is rewritten
   or recreated; `memory_server` reports it with the remedy.
2. Over the wire: insert with `deleted_at_unix_ms = 0`, `node_id = ""`;
   replace with a timestamp and a node; `Page` by `deleted_at_unix_ms`
   lists live rows first; `Query WHERE deleted_at_unix_ms > 0 AND
   deleted_at_unix_ms < cutoff` finds the purge set; `GROUP BY node_id`
   counts per node with `""` as its own group; a negative `deleted_at`
   is `Malformed`; the two fields survive a restart.
3. `describe` reports thirteen fields in tag order; every earlier
   golden vector byte-identical (no wire change).

## Open questions

- **Which option** — *resolved*: (a), N1 + L1.
- **(L2)'s trigger** — the first durable directory that cannot be
  re-pushed.
- **A `PurgeBefore` request** — when the client loop's round trips are
  measured and found wanting.
- **The remaining spike gaps** — directed open-label edges (gap 4), a
  global edge count (gap 5).

## Change history

- 2026-09-07: Implemented as `SERVER-001` v0.46.0 / FR-056 (N1 + L1),
  landed as designed.
- 2026-09-07: Accepted — the owner picked option (a) after PR #216.
- 2026-09-07: Initial proposal, design only; implementation waits for
  the owner's option. The twenty-first round in the
  `rusty_remind_me`-motivated line; the hub spike's third gap.
