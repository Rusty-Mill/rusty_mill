# ADR-0060: Batch the runtime writes in one request

- Status: **Accepted as designed** (2026-09-08 — the owner picked option
  (c): pipelined by default, atomic under a flag; (a) pipelined-only,
  (b) atomic-only, (d) session, and (e) decline declined). Proposed and
  then implemented on one branch once the pick was made.
- Date: 2026-09-08
- Deciders: baileyrd
- Related: `docs/design/SERVER-WRITE-BATCH-DESIGN.md` (the full design),
  `ADR-0013` (the field-update batch this generalizes), `ADR-0024`–
  `ADR-0026` (the session and redo journal it revisits), `ADR-0046`–
  `ADR-0051` (the runtime writes), `ADR-0054` (`ReplaceIf`, the
  per-record atomic compare-and-replace the consumer already has),
  `docs/reports/2026-09-07-hub-spike-report.md`.
- Supersedes/Superseded by: none. Would append one `Request` and one
  `Response` variant at protocol 22.

## Context

The hub's sync push applies a batch of heterogeneous records one at a
time — ~2 round trips per record — because the only batch on the wire
(`Request::Transaction`, `ADR-0013`) carries field updates only, not the
runtime writes (`Insert`/`Replace`/`ReplaceIf`/`Delete`/`Link`,
`ADR-0046`–`0051`) the push is made of. The unmet need is round trips.
Correctness is already covered per record: the hub's sync is
last-writer-wins per record, and `ReplaceIf` (`ADR-0054`) makes one
record's compare-and-replace atomic against a concurrent writer.

## Decision

Propose `Request::WriteBatch { ops: Vec<WriteOp> }` — `WriteOp` an enum
over the five runtime writes, each carrying the body of its single-shot
request — applied under one acquisition of the table's write lock, in
place of a round trip per record. Recommend **option (a), pipelined**:
answered `Response::BatchResults { results: Vec<WriteResult> }`, one
outcome per op in order, each identical to that op's single-shot
outcome; each op stands on its own, no cross-record atomicity, no
journal-format change. The atomicity guarantee is the fork, held for the
owner:

- **(a) Pipelined (recommended)** — per-op results, not atomic; matches
  the per-record LWW; no journal change.
- **(b) Atomic** — all-or-nothing (`Ok`/`BatchFailed { index, code }`);
  needs `JOURNAL_FORMAT_VERSION` 2 with idempotent redo, and is still
  not atomic against a mid-apply I/O failure without a storage rollback
  primitive (a larger round).
- **(c) Both** — (a) with an `atomic` flag selecting (b).
- **(d) Extend the session** to stage runtime writes — (b)'s atomicity
  plus interactive round trips the push does not want.
- **(e) Decline** — the push stays ~2N round trips; `ReplaceIf` already
  gave it per-record atomicity.

## Consequences

- Positive (a): the push goes from ~2N round trips to one batch, with no
  new guarantee to reason about — each op is its single-shot write,
  batched — and no journal, storage, or rollback change.
- Named, not hidden: (a) is not cross-record atomic. For a per-record
  LWW consumer that is the right trade; a consumer that needs
  all-or-nothing is the trigger for (b)/(c), a clean follow-on.
- Named, not hidden: even (b) is atomic only against precondition
  failures and (when journaled) crashes, never against a mid-apply
  durability error, until a storage rollback primitive exists.
- A wire change either way: protocol 22, one request and one response
  variant; a pre-22 client is unaffected (rule 3).

## Acceptance and implementation

- 2026-09-08: proposed, design only.
- 2026-09-08: the owner picked option (c). Implemented on the same branch
  as `SERVER-001` v0.50.0 / FR-060 — `WriteOp`/`WriteResult` and the two
  wire variants at protocol 22, `ConnectionStore::{apply_write_op,
  write_batch}` (pipelined default) with the atomic single-`with_exclusive`
  override on `Memory`/`Entity`/`Relation`, both clients, the pins and
  `SERVER-002` v0.11.0. The atomic flag is precondition- and
  isolation-atomic; crash-atomicity across the batch stays the named
  storage follow-on (option (c)'s journal-format path was scoped to that
  follow-on rather than built here, since the append logs fsync per op
  and per-op durability already holds). (PR #229.)
