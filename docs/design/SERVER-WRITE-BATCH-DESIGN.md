# Server Write Batch Design (Proposed)

- Status: **Proposed** (2026-09-08). The twenty-fifth round in the
  `rusty_remind_me`-motivated line; design only, because the batch's
  atomicity guarantee is a fork the owner picks (as `ADR-0056` was).
- Date: 2026-09-08
- Related: `ADR-0013`/`SERVER-TRANSACTION-DESIGN.md` (the field-update
  batch this generalizes), `ADR-0024`/`SERVER-TRANSACTION-SESSION-DESIGN.md`
  (the buffered session), `ADR-0025`/`ADR-0026` (the redo journal and
  group commit whose redo-suffices argument this round revisits),
  `ADR-0046`–`ADR-0051` (the runtime writes this would batch),
  `ADR-0054` (`ReplaceIf`, the per-record atomic compare-and-replace),
  `docs/reports/2026-09-07-hub-spike-report.md` (the sync write path).
- Decision: `ADR-0060`.

## Purpose and scope

The hub's sync push applies a batch of records — a memory (insert or
last-writer-wins replace), an entity, a mention link, a relation — one
at a time. On this backend each record is one or two round trips
(`get` then `insert`/`replace`, or `link`), and the batch has no
single-request form: the only batch on the wire, `Request::Transaction`
(`ADR-0013`), carries `TransactionOp` — an `UpdateField` of one
scannable field (`access_count` on `Memory`) — and nothing else. A
memory upsert, a link, a delete cannot ride it.

This round asks what a batch of the *runtime write* requests
(`Insert`, `Replace`, `ReplaceIf`, `Delete`, `Link`) should be: one
request carrying many such writes, applied under one acquisition of the
table's write lock, in place of a round trip per record.

## The decision this round exists to make

The writes this batches are **not** the idempotent fixed-slot overwrites
`TransactionOp` is. An insert appends to the insert log and can return
`Duplicate`; a delete appends a tombstone; a link appends to the edge
log. Three consequences fork the design, and the owner picks the fork:

1. **What "batch" guarantees on a precondition failure.** The existing
   `Transaction` validates every op first, then applies, so a bad op
   means nothing is applied. That still holds for these writes at
   *validation* (a malformed field list, an unknown record for a link).
   It does **not** hold at *apply*: once op 1 has appended to a log, op 3
   failing with a `Storage` (durability) error cannot be rolled back —
   the append-only logs have no undo. So strict all-or-nothing is not
   free; it is bounded by "atomic against precondition failures, not
   against an I/O failure mid-apply", unless a storage-layer rollback
   primitive is built (a separate, larger round).

2. **Whether cross-record atomicity is even wanted here.** The hub's
   sync is **per-record last-writer-wins**: each record independently
   wins or loses by its `updated_at`, and `ReplaceIf` (`ADR-0054`)
   already makes that one record's compare-and-replace atomic against a
   concurrent writer. A partially applied push is not a corruption — the
   records that applied are correct, and the next push retries the rest.
   So the value of *cross-record* atomicity for this consumer is thin;
   the value of *fewer round trips* is the whole point.

3. **The journal.** The redo journal (`ADR-0025`) is a `Vec<TransactionOp>`
   per entry, and its "redo on open suffices" argument (`journal.rs`)
   rests explicitly on every op being an idempotent slot overwrite of a
   never-deleted id. Journaling heterogeneous writes needs a new entry
   shape (`JOURNAL_FORMAT_VERSION` from 1 to 2) **and** an idempotent
   redo for each: an insert of an id that redo finds already present, a
   delete of an already-absent id, a link that already exists — each
   must replay to the same state, not error. Real work, and only needed
   if the batch is crash-atomic.

## Considered options

- **(a) Pipelined batch — recommended.** `Request::WriteBatch { ops:
  Vec<WriteOp> }`, `WriteOp` an enum over the five runtime writes,
  answered `Response::BatchResults { results: Vec<WriteResult> }` — one
  outcome per op, in order, each the same outcome its single-shot
  request gives (`Inserted`/`Duplicate`, `Replaced`/`NotFound`,
  `Replaced`/`GuardFailed`/`NotFound`, `Deleted`/`NotFound`,
  `Linked`/`AlreadyLinked`, or an `ErrorCode`). Applied under one write
  lock so the batch sees a consistent table, but each op stands or falls
  on its own — no cross-record atomicity, no rollback question, no
  journal-format change (each op journals exactly as its single-shot
  form does today). Matches the hub's per-record LWW; cuts the push from
  ~2N round trips to N-in-one. The honest, minimal answer.
- **(b) Atomic batch.** The same request answered `Ok` or
  `BatchFailed { index, code }` — all-or-nothing. Requires the
  precondition-validate-then-apply discipline extended to every write
  (feasible), the `JOURNAL_FORMAT_VERSION` bump with idempotent redo
  (real work), and still carries the "not atomic against a mid-apply I/O
  failure" caveat unless a storage rollback primitive is built (larger).
  Buys cross-record atomicity the per-record-LWW consumer does not need.
- **(c) Both — pipelined by default, atomic under a flag.** Option (a)'s
  request with an `atomic: bool`; the flag selects (b)'s guarantee. The
  most surface for a guarantee no current consumer asks for.
- **(d) Extend the session instead.** Let `Begin`/stage/`Commit` stage
  the runtime writes, not just `UpdateField`. This is (b)'s atomicity
  plus the interactive round-trip cost the hub's push does not want, and
  entangles read-your-writes/snapshot isolation with insert/delete
  semantics that have no read-set. Wrong shape for this consumer.
- **(e) Decline.** The push stays N records at ~2 round trips each;
  `ReplaceIf` already gave it per-record atomicity, which is what
  correctness needs.

## Recommendation

**(a).** The consumer's writes are already per-record atomic through
`ReplaceIf`; the unmet need is round trips, not a distributed
transaction. Option (a) delivers the round-trip win with no journal
format change, no rollback question, and no guarantee the consumer would
have to reason about beyond "each op is its own single-shot write,
batched." If a later consumer needs true all-or-nothing, (b) or (c) is a
clean follow-on that adds a guarantee without taking one away.

## Non-goals

- Not ACID across records (unless the owner picks (b)/(c)); see the
  fork above.
- Not a new op *kind* — `WriteOp` wraps the five requests that already
  exist, verbatim; nothing new to validate or apply per op.
- Not a session change — `WriteBatch` is a single request/response like
  `Transaction`, gated `SessionOpen` inside a session, never staged.
- Not a storage-layer rollback primitive — named as the round that would
  make (b) atomic against a mid-apply I/O failure, not built here.

## Proposed shape (option (a))

- Wire: `Request::WriteBatch { ops: Vec<WriteOp> }` (index 31),
  `Response::BatchResults { results: Vec<WriteResult> }` (index 20),
  `PROTOCOL_VERSION` 22. `WriteOp` an enum: `Insert { id, fields }`,
  `Replace { id, fields }`, `ReplaceIf { id, fields, guard }`,
  `Delete { id }`, `Link { left, right, relation }` — each the body of
  its single-shot request. `WriteResult` an enum mirroring the
  single-shot outcomes plus `Err(ErrorCode)`.
- Server: `ConnectionStore::write_batch(&self, &[WriteOp]) ->
  Vec<WriteResult>`, default looping the existing
  `insert_record`/`replace_record`/`replace_record_if`/`delete_record`/
  `link_records` under one `with_exclusive`; a bound `MAX_BATCH_OPS`
  (like `MAX_STAGED_OPS`) caps how long one connection holds the lock.
  Gated as a write: `Unauthorized` for `ReadOnly`, `SessionOpen` inside
  a session, server-`Malformed` below 22.
- Clients: `SchemaDrivenClient::write_batch(&[WriteOp]) ->
  Vec<WriteResult>`; Python `Client.write_batch(ops)`.
- Journal: unchanged — each op inside the batch journals through its own
  path exactly as the single-shot request does; no format bump. (This is
  the property (a) buys and (b) gives up.)

## Open questions

- **`MAX_BATCH_OPS`.** A bound on ops per batch (beyond `MAX_FRAME_BYTES`)
  to cap lock-hold latency for other connections — a number to pick from
  a benchmark once implemented, as `SERVER-TRANSACTION-DESIGN.md` left
  `MAX_STAGED_OPS`.
- **Per-op vs first-error reporting** if (a): a full `Vec<WriteResult>`
  (proposed) versus stopping at the first `Err`. The full vector is
  friendlier to a puller that retries only the failures.
- **The atomic follow-on** if (b)/(c) is ever wanted: the journal format
  v2 and idempotent-redo design, and whether a storage rollback
  primitive is worth building for mid-apply I/O atomicity.

## Change history

- 2026-09-08: Initial proposal, design only. The owner picks the
  atomicity fork ((a) pipelined recommended, (b) atomic, (c) both,
  (d) session, (e) decline); implementation follows the pick.
