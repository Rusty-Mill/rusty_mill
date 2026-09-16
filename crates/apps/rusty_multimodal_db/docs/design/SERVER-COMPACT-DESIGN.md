# Server Compaction Design (Accepted)

- Status: **Accepted** (2026-09-07, `ADR-0052` option (a), as designed —
  an explicit request doing the reopen fold in place plus a gapless
  slot rewrite, under the write lock; automatic, reopen-only, online,
  and declining all declined). Implemented on the same branch as
  `SERVER-001` v0.42.0 / FR-052, `SERVER-002` v0.7.0, PR #206.
- Date: 2026-09-07
- Related: `ADR-0046`/`docs/design/SERVER-INSERT-DESIGN.md` (which first
  named "runtime compaction" as a revisit trigger: a log noticeably
  larger than its blob), `ADR-0047` (the edge logs, same trigger),
  `ADR-0051`/`docs/design/SERVER-DELETE-DESIGN.md` (retired slots and
  tombstones — the round that made the case strongest and named this
  one), `STORAGE-015` (the record blob's atomic temp-and-rename write,
  reused), `ADR-0032`/`SERVER-001` FR-035 (`ServeOptions`: the
  operator-facing surface this request joins).
- Supersedes/Superseded by: none. Appends one `Request` and one
  `Response` variant at protocol 18; changes no file format (every
  file a compaction writes is one the reopen path already writes).

## Purpose and scope

Since `ADR-0046` every runtime write has been append-only: an insert
or a replace appends to the insert log, a link to an edge log, a delete
a tombstone to both — and a delete additionally retires a slot the
mmap file keeps. All of it is reconciled only by the next `open`, which
folds each log into its blob and appends slots for records that lack
one but never removes a retired slot: the mmap file never shrinks, and
a long-running server never folds at all. Five rounds named this as the
one operational gap; this round closes it.

Scope, exactly: `Compact` in `crate::generic::query` and a
`CompactionReport` (`CMP-FR-001`); `GenericMmapStore::compact` — blob
from the live set, slot file rewritten gaplessly, log removed, in a
crash-safe order (`CMP-FR-002`); forwards through every layer, the
in-memory ones contributing nothing (`CMP-FR-003`); `Symmetric`/
`MultiSymmetric` folding their edge logs into their blobs
(`CMP-FR-004`); `GenericProductionStore::compact` under the write lock
(`CMP-FR-005`); `ConnectionStore::compact`, `Request::Compact` and
`Response::Compacted` at protocol 18 with `Insert`'s gates
(`CMP-FR-006`); both clients' `compact` (`CMP-FR-007`); two vectors and
`SERVER-002` v0.7.0 (`CMP-FR-008`).

## Non-goals

- **Automatic or background compaction** — no threshold, no timer, no
  thread. The request is an operator's, or a supervising process's,
  and it holds the table's write lock for its duration. A policy
  (compact when the log exceeds N× the blob) is a client's to run.
- **Online compaction** — readers and writers wait. A table meant to
  hold tens of thousands of records compacts in milliseconds; the
  lock is the simple, correct answer at this scale, and named.
- **Compacting the transaction journal** (`ADR-0025`) — it has its own
  checkpoint story (`CheckpointFlush`); untouched.
- **Reordering** — records keep their slot order; edges are rewritten
  sorted (see `CMP-FR-004`), the one observable change, to neighbor
  order after a *later reopen*, never to a live read.
- **Reclaiming disk from the blob** — the blob is rewritten from the
  live set, so it already shrinks; only the slot file needed the work.

## Context and terminology

- **The reopen fold**: `GenericMmapStore::open` folds the insert log
  into the blob and clears the log; `Symmetric::open`/`MultiSymmetric::
  open` do the same per edge blob. Compaction is that fold, done in
  place while the store stays open — plus the one thing the fold never
  did, dropping retired slots.
- **Slot rewrite**: `SlotFile::rewrite` writes a fresh gapless image at
  `<path>.compact`, releases the current mapping, renames the new file
  over the old, and maps it. Releasing the mapping first is what makes
  the rename valid on every platform this crate builds for.
- **Report**: `CompactionReport { records, slots_reclaimed,
  log_entries_folded, edge_logs_folded }`, summed across the layers of
  a stack; a stack with two slot files (`MmapScanned` over the core)
  counts both files' retired slots.

## Requirements

- `CMP-FR-001` **Trait and report.** `query::Compact` with `fn compact
  (&mut self) -> Result<CompactionReport, DurabilityError>`;
  `CompactionReport` in `crate::generic` with `absorb`.
- `CMP-FR-002` **`GenericMmapStore::compact`.** In this order: (1) the
  blob rewritten from the live records in position order, each carrying
  its slot's current scannable value (atomic temp-and-rename); (2) the
  slot file rewritten gaplessly in that order (`SlotFile::rewrite`);
  (3) the insert log removed; then the position index rebuilt. A crash
  after (1) reopens through the old slot file and a fold of the
  still-present log — idempotent, since every entry is already in the
  blob; a crash after (2) folds the log once more, idempotent likewise.
  `is_gapless` holds afterwards. Report: the live count, slots before
  minus after, the log's entry count.
- `CMP-FR-003` **Forwards.** `BaseStore` reports zero; `Indexed`,
  `Scanned`, `NameIndex`, `Reversed` forward; `MmapScanned` forwards
  then rewrites its own slot file and adds its reclaimed slots.
- `CMP-FR-004` **Edge blobs.** `Symmetric`/`MultiSymmetric` forward
  then, per blob with a path, rewrite it from the adjacency when its
  log holds anything or the blob is stale, and remove the log; the
  edge list is each undirected edge once as `(smaller, larger)`,
  sorted — deterministic, so a second compaction writes nothing.
  Requires `R::Id: Ord`. `edge_logs_folded` counts logs that held
  entries.
- `CMP-FR-005` **`GenericProductionStore::compact`** under the write
  lock.
- `CMP-FR-006` **The request.** `ConnectionStore::compact(&self) ->
  Result<CompactionReport, ErrorCode>` (default `Unsupported`;
  `Memory`/`Reminder`/`Entity` implement; a `DurabilityError` is
  `Storage`); `Request::Compact` (27), `Response::Compacted { records,
  slots_reclaimed, log_entries_folded, edge_logs_folded }` (18, all
  `u64`), `PROTOCOL_VERSION` 18; `dispatch` answers the report;
  `handle_connection` refuses `ReadOnly` (`Unauthorized`, the seventh
  write), `SessionOpen` inside a session, `Malformed` below 18;
  `audit::RequestKind::Compact`. On a `serve_tables` server it compacts
  the *selected* table only.
- `CMP-FR-007` **Clients.** `SchemaDrivenClient::compact() -> Result<
  CompactionReport, _>` (`Unsupported("compact")` below 18, no frame);
  Python `Client.compact()` returning the `Compacted` reply,
  `PROTOCOL_VERSION = 18`.
- `CMP-FR-008` **Pins.** Two golden vectors (`Request/Compact`,
  `Response/Compacted`; 59 pinned); `SERVER-002` v0.7.0; the literal
  pins moved.

## Considered options

- **(a) An explicit request that does the reopen fold in place plus a
  gapless slot rewrite, under the write lock — proposed.** Every file
  it writes is one the reopen path already writes; the crash order is
  the fold's own; the report tells an operator what it bought.
- **(b) Compact automatically past a threshold.** Puts a stop-the-world
  pause inside an ordinary write; the threshold is a policy the server
  has no business choosing.
- **(c) Compact only at reopen, but also drop retired slots there.**
  Closes the file-growth gap for restarting deployments only; a
  long-running server still never folds.
- **(d) Online compaction with a shadow file and a catch-up log.** The
  right shape at a scale this crate does not target; real machinery for
  a millisecond pause.
- **(e) Decline.**

## Proposed shape

`src/generic/slot_file.rs` (`rewrite`), `src/generic/query.rs`
(`Compact`), `src/generic/mod.rs` (`CompactionReport`),
`src/generic/mmap_store.rs` (`compact`), `src/generic/store.rs`
(`sorted_edges`, `compact_edge_blob`, seven impls),
`src/generic/mmap_scanned.rs`, `src/generic/production.rs`;
`src/server/protocol.rs`, `src/server/serve.rs`, `src/server/audit.rs`,
`src/server/{memory,reminder,entity}.rs`, `src/server/client.rs`,
`clients/python/**`.

Wire: `Compact` = `[0x1b 0 0 0]`; `Compacted` = `[0x12 0 0 0]` · four
`u64`-LE.

## Data/state and invariants

- After a compaction: no insert log, no edge log with entries, every
  slot file gapless, every blob equal to the live set. Every read
  answers exactly as before.
- The mmap file's length after compaction is `HEADER_LEN + live ×
  slot_width` — the first time since `ADR-0051` it can shrink.
- A second compaction with no intervening write reports the live count
  and zeros.

## Errors, failure, recovery, and observability

`Unsupported` from a domain with no compaction; `Storage` when a file
could not be written, renamed, mapped, or removed — the files are left
in a state the reopen path handles (`CMP-FR-002`'s order). The report
is the observability: what was reclaimed, per request.

## Security, privacy, and compatibility

A write: `ReadOnly` refused, sessions refused, gated below 18. No file
format changes; a compacted directory is exactly what a reopen would
have produced, so any build since `ADR-0051` reads it.

## Acceptance criteria

1. Store: after an insert, an update, a replace, and a delete on the
   core, `compact` reports the live count, one reclaimed slot, and
   three log entries; the log is gone, `is_gapless` holds, every read is
   unchanged, a second compaction reclaims nothing, writes after it
   land, and a portable reopen serves everything. `Symmetric` folds a
   runtime link and a tombstone into its blob with the adjacency
   unchanged and the reopened edge list agreeing. Through `Entity`'s
   stack an insert, a link under a new label, and a delete fold with
   every read unchanged across a portable reopen; through `Employee`'s
   both slot files reclaim and the collaboration log folds.
2. Wire, two tables: after a link and a delete, `compact` on `memory`
   reports three live records, one slot, one log entry, one edge log;
   every read agrees before and after; a second compaction reclaims
   nothing; `compact` on `entity` after a cascading delete; a restart on
   the compacted files serves the same data. `Malformed` at 17 and
   silent, `Compacted` at 18; `Unsupported("compact")` at 1;
   `Unauthorized` for `ReadOnly`; `SessionOpen` in a session;
   `Unsupported` on `Dog`; from Python the exact counts.
3. Two vectors added, every earlier vector byte-identical; `SERVER-002`
   v0.7.0; the Python suite passes offline and live.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-COMPACT-DESIGN`, `SERVER-COMPACT`. Decision:
  `ADR-0052`.
- Specification: `SERVER-001` v0.42.0 / `FR-052`; `SERVER-002` v0.7.0.
- Requirements: `CMP-FR-001`–`008`.

## Open questions

- **A policy** — when to compact. The report gives a client the
  numbers; a supervising process (the consumer's own maintenance loop,
  `maintenance.rs`) is the natural owner.
- **Neighbor order after reopen** — sorted after a compaction, insertion
  order before one; no read today depends on it, and the design says so.
- **Unlinking one edge** and **entity merge as a request** — unchanged
  from `ADR-0051`'s open questions.

## Change history

- 2026-09-07: Accepted as designed (option (a)), after PR #206. No
  content change.
- 2026-09-07: Implemented as `SERVER-001` v0.42.0 / FR-052, landed as
  designed (PR #206).
- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The seventeenth round in the `rusty_remind_me`-motivated
  line; the operational gap five rounds named.
