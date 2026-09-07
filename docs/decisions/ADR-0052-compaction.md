# ADR-0052: Compaction — the reopen fold in place, retired slots reclaimed, `Request::Compact` at protocol 18

- Status: **Proposed** (2026-09-07; implemented on the same branch,
  the `ADR-0046`–`ADR-0051` cadence — the owner accepts or amends a
  working, tested shape). See "Considered options".
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-COMPACT-DESIGN.md` (the full design),
  `ADR-0046`/`ADR-0047` (the logs, and the revisit trigger they named),
  `ADR-0051` (retired slots — the round that made this one's case),
  `STORAGE-015` (the blob's atomic write, reused).
- Supersedes/Superseded by: none. Appends `Request::Compact` (27) and
  `Response::Compacted` (18) at protocol 18; no file format change.

## Context

Every runtime write since `ADR-0046` appends: records and tombstones
to the insert log, edges and tombstones to the edge logs, and a delete
retires a slot the mmap file keeps. Only a reopen folds the logs, and
nothing ever drops a retired slot — the mmap file never shrinks, and a
long-running server never folds. Five rounds named this as the
operational gap; deletion made it the strongest remaining case.

## Decision

Add `Compact`: the reopen fold, done in place under the table's write
lock — the blob rewritten from the live records (each with its slot's
current value), the slot file rewritten gaplessly via a temp file and
rename, the insert log removed, in that order so a crash anywhere
leaves files the reopen path handles; every layer forwarding, the
relation layers folding their edge logs into their blobs (edges
rewritten sorted); a `CompactionReport` summed across the stack;
`Request::Compact` answered by `Response::Compacted` with the counts,
gated as every write; `compact` on both clients. No threshold, no
timer, no background thread: an operator's request.

## Consequences

- Positive: the one operational gap in the line is closed; the mmap
  file can shrink for the first time; a supervising process has the
  numbers to decide when to call it.
- Positive: no file format change, no new error code; a compacted
  directory is exactly what a reopen would have produced.
- Named, not hidden: readers and writers wait for the duration —
  milliseconds at this crate's target scale, and a stop-the-world
  pause nonetheless, so it is never triggered by an ordinary write.
- Named, not hidden: neighbor order after a later reopen is sorted
  rather than insertion order; no read depends on it.
- The policy of when to compact is the caller's.

## Considered options

**(a) Accept as designed** — an explicit request doing the reopen fold
in place plus a gapless slot rewrite, under the write lock. **(b)
Automatic past a threshold** — a pause inside an ordinary write and a
policy the server should not own. **(c) Reopen-only, but dropping
retired slots there** — leaves a long-running server unfolded. **(d)
Online compaction with a shadow file** — real machinery for a
millisecond pause. **(e) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.42.0 / FR-052, `SERVER-002` v0.7.0 —
  `src/generic/{slot_file,query,mod,mmap_store,store,mmap_scanned,
  production}.rs`, `src/server/{protocol,serve,audit,memory,reminder,
  entity,client}.rs`, `clients/python/**`, two vectors (59 pinned);
  tests: `mmap_store` +1, `store` +1, `entity` +1, `employee_impl` +1,
  `server/memory` +1, `server_memory_integration` +1, `server_protocol_
  version` +1, `server_dog_integration` +1, `server_auth_integration`/
  `server_transaction_integration`/`server_python_client` extended;
  every acceptance criterion 1–3 holds. (This PR.)
