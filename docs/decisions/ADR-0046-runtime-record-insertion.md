# ADR-0046: Runtime record insertion — `Insert<R>`, the insert log, `Request::Insert` at protocol 13

- Status: **Proposed, implemented in the same cycle** (2026-09-06).
  Under the owner's standing mandate for this line ("continue working
  to mature `multimodal_db` and ensure it can be used as a backend for
  `rusty_remind_me`"), this round proposes *and* lands the design —
  the `ADR-0011` precedent, taken deliberately rather than by default:
  every piece is additive and reversible (see Consequences), the gap
  it closes is the one every prior round in this line has named as
  the reason the crate cannot yet be a backend, and no owner check-in
  was available mid-session. The acceptance options below are still
  the owner's to pick; (b) or (c) is a revert of a bounded, self-
  contained set of files, recorded here so that is a real choice and
  not a formality.
- Date: 2026-09-06
- Deciders: baileyrd
- Related: `docs/design/SERVER-INSERT-DESIGN.md` (the full design),
  `ADR-0036` (the "fixed record set" invariant this narrows),
  `ADR-0042` (`entity_id` — a caller-minted id, the reason the client
  supplies the id here), `ADR-0045` (gate (ii): a `Memory` domain,
  now unblocked as the next round), `STORAGE-015` (the record blob the
  insert log folds into), `STORAGE-017` (`SlotFile::append_committed_
  slots`, reused at runtime), `ADR-0025` (the journal's file shape,
  copied for the log), `ADR-0022`/`SERVER-002` (protocol 13),
  `ADR-0043` (the Python reference client, which gains `insert`).
- Supersedes/Superseded by: none. Appends one `Request` variant, one
  `ErrorCode`, one `ConnectionStore` method with a default, one
  `crate::generic` trait and error type, one companion file that
  exists only after an insert. Changes no existing variant, file
  format, or domain behavior.

## Context

Every round since `ADR-0036` has shaped what a `Reminder` or `Entity`
record *is* and how it is read — and every one has run over a record
set fixed at `create`/`open`. This crate has never been able to add a
record to a running store: no request, no adapter method, no trait,
no store method. `ADR-0036`'s design named the invariant "no runtime
deletion, fixed schema"; it was three clauses, and the unnamed one —
no runtime insertion — is the one a backend cannot live with.

The consumer's three creating tool calls (`remind_me_add`,
`set_reminder`, `entity_upsert`) all mint an id client-side and
`INSERT` (`rusty_remind_me` `db/queries.rs:98`, `entity.rs`). A
`Memory` domain (`ADR-0045` gate (ii)) without insertion would be a
table nothing can write to; insertion without a `Memory` domain makes
`Reminder` and `Entity` usable for two of the three today. So this is
the strictly-prior round.

The pieces already exist: `open` appends a committed slot for a
record it has never seen (`STORAGE-017`'s `O_APPEND` path); the record
blob is a whole-set image rewritten whenever the set changes
(`STORAGE-015`); the journal shows a length-prefixed, torn-tail-
tolerant append file (`ADR-0025`). What is missing is persisting one
record's content without rewriting the blob, and each composition
layer learning about a record it was not built with.

## Decision

Adopt `docs/design/SERVER-INSERT-DESIGN.md` as proposed:

1. **`Insert<R>`** in `crate::generic::query` and `InsertError<Id>::
   {Duplicate, Durability}` in `crate::generic`. `GenericMmapStore::
   insert` refuses a duplicate, appends the record to a new companion
   **insert log** (`<path>.inserts`: the blob's 28-byte tagged header
   under `GENINSL\0`, then `u32`-length-prefixed `bincode` records,
   `sync_data` per entry), appends a committed slot through the
   existing `O_APPEND` path, then updates its maps. `Ok` means durable.
2. **Fold at open.** `read_portable_records` = blob then log (log
   entries already in the blob skipped); `open` merges the log into the
   caller's records, so the blob rewrite it already performs for a
   changed set covers the inserts, and truncates the log only after
   the blob is current. No runtime compaction this round.
3. **Every layer forwards** — `Symmetric`, `MultiSymmetric`,
   `NameIndex`, `Reversed`, `MmapScanned`, `BaseStore`/`Indexed`/
   `Scanned` — each mutating its own state only after the inner insert
   succeeds; `GenericProductionStore::insert` under the write lock.
4. **`ConnectionStore::insert_record`** with a default of
   `Unsupported`; `Reminder` and `Entity` implement it with full
   schema validation before any write (every described tag exactly
   once, right kind, the domain's own rule).
5. **`Request::Insert { id, fields }`** at index 21, protocol 13,
   answered `Ok` or the new **`ErrorCode::Duplicate`** (11). Server-
   gated `Malformed` below 13; `Unauthorized` for `ReadOnly`;
   `SessionOpen` inside a session; never journaled, never part of a
   `Transaction`. The client mints the id.
6. **`SchemaDrivenClient::insert`** and the Python client's `Client.
   insert`, both refusing below 13 with no frame sent.

One reuse a reader should see: an insert whose log or slot append
fails is answered `ErrorCode::Journal` — its documented meaning
("could not be made durable; nothing was applied") is exact, and a
dedicated `Storage` code for a path no test reaches without fault
injection was judged speculative. Option (d) below adds it.

## Consequences

- Positive: `Reminder` and `Entity` become writable tables — the
  consumer's `set_reminder` and `entity_upsert` have a real backend
  path for the first time, and `ADR-0045`'s `Memory` domain is
  unblocked.
- Positive: the store-level cost is one `fsync`ed log entry plus one
  `O_APPEND` slot per insert; no existing file format changes, and a
  store nothing was inserted into has no new file and unchanged
  behavior byte-for-byte.
- Positive: every existing client, vector, test, bench, and bin is
  untouched except the three version pins.
- Cost, named: the insert log grows until the next reopen. A long-
  running process with many inserts pays a one-time fold at restart;
  runtime compaction is the named revisit trigger.
- Cost, named: `ErrorCode::Journal` now has a second producer. If the
  owner prefers a distinct code, it is a one-variant append at 13
  *now* or a version bump *later*.
- Cost, named: a `ReadOnly` token's meaning widens to "cannot insert"
  — the only reading anyone could have intended, but a documented
  change to `TokenClass`'s contract.
- Named, not hidden: relation (edge) insertion is not here. An
  inserted `Entity` has no neighbors and cannot gain any over the
  wire until the next round.
- Named, not hidden: this ADR was implemented before acceptance. The
  files it touches are listed in `SERVER-001-FR-046` so a revert is
  mechanical.

## Considered options

- **(a) (proposed, implemented)** The design as written.
- **(b)** Store-level only — `Insert<R>` and the log, no wire change;
  a consumer embeds the crate. Rejected: the consumer is a separate
  process (`rusty_remind_me`'s MCP server), and the whole line since
  `ADR-0010` is a network boundary.
- **(c)** Rewrite the blob on every insert — O(n) per write; correct,
  unusable past toy sizes (`STORAGE-015`'s measured 76 B/record).
- **(d)** As (a) plus a dedicated `ErrorCode::Storage` appended at 13
  for the durability-failure path.
- **(e)** Decline — keep the fixed record set; the crate stays a
  benchmark harness with a read-mostly server.

## Acceptance and implementation

- Options offered at proposal: **(a) accept as implemented**; **(b)**
  accept the store layer, revert the wire (`src/server/**`, the two
  vectors, protocol 13, the clients); **(c)** revert all of it;
  **(d)** accept and append `ErrorCode::Storage` now, in the same
  version.
- 2026-09-06: proposed and implemented as `SERVER-001-FR-046`
  (v0.36.0) in the same PR, per the Status note above. See that
  requirement's entry and `docs/PROJECT-STATUS.md` for the file list
  and test counts.
