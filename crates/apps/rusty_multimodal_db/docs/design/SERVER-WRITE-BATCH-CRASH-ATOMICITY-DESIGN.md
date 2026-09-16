# Crash-Atomic `WriteBatch`: Journal Format Version 2 (Accepted)

- Status: **Accepted as designed** (2026-09-14, `ADR-0063`, option (a) —
  extend the existing journal to format version 2 with a kind byte,
  reuse `CommitGroup` as-is; (b) a separate journal file, (c) decline,
  both declined). Implemented on the same branch as `SERVER-001` v0.52.0
  / FR-062. **Mechanism corrected during implementation, before any
  code was written from the original sketch** — see `ADR-0063`'s own
  "Acceptance and implementation" for the full account: `ReplaceIf`
  guards are journaled raw (unresolved), not pre-decided into a new
  `DecidedWriteOp` type as this document's Requirements below describe;
  replay re-runs the identical `prepare_write`/`apply_prepared` pair
  instead, which is simpler and gives the identical guarantee — a
  pre-decision step turned out to conflict with `CommitGroup`'s own
  append-before-exclusive-section discipline (`ADR-0026`). The
  Requirements section is kept as originally written, not rewritten,
  per this project's "layer findings, don't rewrite history" convention.
- Date: 2026-09-14
- Related: `ADR-0060`/`docs/design/SERVER-WRITE-BATCH-DESIGN.md`
  (`Request::WriteBatch`, whose own Consequences named this exact gap),
  `ADR-0025`/`ADR-0026`/`docs/design/SERVER-TRANSACTION-SESSION-DESIGN.md`
  Part B / `docs/design/SERVER-JOURNAL-GROUP-COMMIT-DESIGN.md` (the
  journal and group-commit discipline this reuses), `ADR-0062`
  (Definition of Done, item 4).
- Supersedes/Superseded by: none. Server-side only — no `Request`/
  `Response` change, no protocol-version bump. `src/server/journal.rs`
  and `src/server/{memory,entity,relation}.rs`'s `write_batch` only.

## Purpose and scope

`Request::WriteBatch`'s atomic mode (`ADR-0060`) is precondition- and
isolation-atomic but not crash-atomic: each op's own storage log
`fsync`s individually, so a process crash mid-batch leaves a partial
result on disk. This round closes exactly that gap, for exactly the
three domains it applies to (`Memory`/`Entity`/`Relation` — the only
`write_batch` overriders with an atomic path today), and only when the
adapter is already running journaled (`with_journal`, opt-in per
binary via e.g. `SERVER_TXN_JOURNAL_PATH`).

Scope, exactly: `JOURNAL_FORMAT_VERSION` 1 → 2 with a leading kind byte
per entry (`WBJ-FR-001`); guard resolution moved inside the exclusive
section, before the journal write (`WBJ-FR-002`); the journal write and
replay path itself (`WBJ-FR-003`–`004`); a real crash-injection test
proving the all-or-nothing property across a process kill
(`WBJ-FR-005`).

## Non-goals

- **Crash-atomicity against a storage I/O error during apply.** A slot
  write itself failing mid-batch (not a process crash, a real write
  error) still needs the storage rollback primitive `ADR-0060` named as
  a separate, larger, undecided round. This closes the crash-recovery
  half only.
- **Journaling `WriteBatch` on a non-journaled adapter.** A server run
  without `with_journal` gets exactly today's precondition-/isolation-
  atomic-only behavior, unchanged. Whether every binary should default
  to journaled is a separate decision.
- **`Dog`/`Order`/`Employee`'s journal path.** Untouched — their journal
  only ever carries `TransactionOp` batches (kind `0`); this round adds
  kind `1` for `Memory`/`Entity`/`Relation`'s `WriteOp` batches only.
- **The pipelined (non-atomic) `write_batch` path.** Unchanged — each
  op already stands on its own, already individually durable via its
  own log's `fsync`.
- **A storage rollback primitive.** Named in `ADR-0060`, still not
  built; out of scope here.

## Context and terminology

Read from `src/server/journal.rs`, `src/server/memory.rs` (identical in
`entity.rs`/`relation.rs`) as they stand at `SERVER-001` v0.51.0:

- `BatchJournal::{open, append_unsynced}` are hard-typed to
  `Vec<TransactionOp>` — `journal.rs:210,287`. `JOURNAL_FORMAT_VERSION =
  1`, magic `TXNJRNL\0`, header 12 bytes, then `[u32 LE len][codec(Vec<
  TransactionOp>)]` entries, no kind byte.
- `MemoryConnectionStore`/`EntityConnectionStore`/
  `RelationConnectionStore` each already carry `journal: Option<
  CommitGroup>` and a `with_journal` constructor
  (`memory.rs:81-110`, mirrored in `entity.rs`/`relation.rs`) — built
  for `Request::Transaction` (`apply_batch(inner, updates)` at
  `memory.rs:806`), proven by `dog.rs`'s own journal tests
  (`with_journal_replays_onto_pre_batch_files_and_is_a_no_op_on_post_batch_files`).
- `write_batch`'s atomic path (`memory.rs:629-656`) calls
  `self.store.with_exclusive(...)` directly — `self.journal` is never
  consulted, even when `Some`.
- `PreparedWrite` (`memory.rs:306-316`) already resolves every op's
  shape (field decoding, `ReplaceIf`'s guard as a `Predicate`, `Link`'s
  label) **before** the exclusive section — but `ReplaceIf`'s guard
  itself is evaluated **inside** `apply_prepared`, against live state,
  at apply time — not resolved to a concrete outcome before that point.
- `memory_server.rs:182-190`: `SERVER_TXN_JOURNAL_PATH` set →
  `with_journal`; unset → plain `new` — the real, already-shipped
  opt-in switch this round's guarantee is conditional on.
- `src/generic/insert_log.rs`'s own `LOG_VERSION_1` → `LOG_VERSION_2`
  precedent: a leading kind byte added, a version-1 file still read
  (every entry assumed one kind), rewritten as version 2 on next
  append — the exact format-evolution shape this round reuses.

## Requirements

- `WBJ-FR-001` **Format version 2.** `JOURNAL_FORMAT_VERSION` 1 → 2.
  Each entry gains a leading kind byte: `0` `TransactionOp` batch
  (today's only shape — a version-1 file with no kind byte is read as
  if every entry were kind `0`, matching `insert_log.rs`'s identical
  precedent); `1` a `Vec<DecidedWriteOp>` — a new enum, structurally
  `WriteOp` with `ReplaceIf`'s `guard` field removed (every entry
  already unconditional by construction — see `WBJ-FR-002`).
  `BatchJournal`/`CommitGroup` become generic over the entry payload
  (`BatchJournal<T>`/`CommitGroup<T>`, `T: Serialize +
  DeserializeOwned`) so `Dog`/`Order`/`Employee` instantiate `T =
  TransactionOp` (kind `0`, unchanged bytes) and `Memory`/`Entity`/
  `Relation` instantiate `T = DecidedWriteOp` (kind `1`) through the
  identical `open`/`append_unsynced`/`truncate` API — no behavior
  change to the `TransactionOp` path, verified by its own existing
  tests passing unmodified.
- `WBJ-FR-002` **Guard resolution before the journal write.** In
  `write_batch`'s atomic path, when `self.journal.is_some()`: prepare
  every op (unchanged); under the exclusive section, evaluate each
  `ReplaceIf`'s guard against live state — on failure, abort the whole
  batch with nothing written and nothing journaled (unchanged
  behavior); on success, convert it to a concrete `DecidedWriteOp::
  Replace` (the guard has done its job — the entry that gets journaled
  is unconditional); append the full `Vec<DecidedWriteOp>` via
  `CommitGroup`, `fsync`, *then* apply every decided op. A crash before
  the `fsync` returns: nothing durable, nothing applied — as if the
  batch never arrived. A crash after: the journal holds the decided
  batch for replay.
- `WBJ-FR-003` **Replay.** `with_journal`'s open-time replay
  (`memory.rs:95-110`, mirrored) gains a kind-`1` arm: apply each
  `DecidedWriteOp` the same way `apply_prepared` already applies a
  `PreparedWrite` (both now unconditional — `ReplaceIf` never appears
  post-decision), idempotently (re-applying an already-applied insert/
  replace/delete/link is a no-op or already-true outcome, matching
  every underlying per-op log's own established idempotency). Kind-`0`
  entries replay exactly as today (`apply_batch`, unchanged).
  Checkpoint-by-size and `CheckpointFlush` on truncate are unchanged.
- `WBJ-FR-004` **No adapter behavior change when not journaled.** When
  `self.journal.is_none()`, `write_batch`'s atomic path is byte-for-byte
  what it is today — `WBJ-FR-002`'s guard-resolution-then-journal
  sequence only runs when a journal is configured.
- `WBJ-FR-005` **Crash-injection proof.** A real test that starts an
  atomic multi-op batch under a journaled adapter, kills the process (or
  simulates the crash point precisely — see Verification plan) after
  the journal `fsync` but before every op is applied, reopens via
  `with_journal` on the same files, and asserts every op's effect is
  present — the property a partial-apply-then-crash test today would
  currently violate (a regression test for the gap this round closes).

## Considered options

Mirrors `ADR-0063`'s own fork — (a) format version 2 with a kind byte,
reusing `CommitGroup` generically (proposed); (b) a wholly separate
`WriteBatchJournal` file/type; (c) decline.

## Proposed shape

`src/server/journal.rs` (`BatchJournal<T>`/`CommitGroup<T>` made
generic, `JOURNAL_FORMAT_VERSION` 2, the kind byte, `DecidedWriteOp`);
`src/server/{memory,entity,relation}.rs` (`write_batch`'s atomic path
gains the journal branch, `with_journal`'s replay gains the kind-`1`
arm). No other file changes — `protocol.rs`, `dog.rs`/`order.rs`/
`employee.rs`, and every client are untouched.

## Data/state and invariants

- A version-2 journal's kind-`0` entries are byte-identical in meaning
  to a version-1 journal's entries — the format change is additive, not
  a reinterpretation.
- Every kind-`1` entry is unconditional by construction — no entry ever
  carries a `Predicate`. Replay never re-evaluates a guard, so replay
  order and original-apply order agree by construction, not by luck.
- Two consecutive opens of the same journal file (no writes between)
  produce identical replay results — the existing idempotency property
  `TransactionOp`'s own tests already establish, now proven for kind-`1`
  too.

## Errors, failure, recovery, and observability

A malformed or foreign-tag kind-`1` entry is `JournalError::Format`,
matching kind-`0`'s existing treatment. A torn tail (length prefix or
payload incomplete) is dropped and truncated, unchanged. `write_batch`'s
existing `Response::TransactionFailed`/`Response::BatchResults` shapes
are unchanged — this round changes only what happens to a batch that
already `fsync`'d its way to `Ok` before a subsequent crash.

## Security, privacy, and compatibility

Server-side durability only; no wire or protocol-version change, so no
client — old or new — observes any difference except a stronger crash
guarantee on a journaled server. A pre-this-round journal file (version
1) opens and replays exactly as before under the new reader.

## Acceptance criteria

1. `BatchJournal<TransactionOp>`'s existing test suite
   (`journal.rs`'s own `mod tests`) passes unmodified after
   genericization — proof the refactor changed no behavior for the
   `Dog`/`Order`/`Employee` path.
2. A version-1 journal file (hand-written, no kind byte) opens under
   the new format-2 reader and replays identically to today.
3. `WBJ-FR-005`'s crash-injection test: an atomic `Memory`/`Entity`/
   `Relation` `write_batch` under `with_journal`, the process
   interrupted after the journal `fsync`, reopened via `with_journal` —
   every op's effect present, none missing, none duplicated.
4. A `ReplaceIf` whose guard would have passed at prepare time but no
   longer holds by the time replay runs (a real interleaving this
   round's guard-before-journal ordering must rule out by construction)
   never occurs — proven by the journaled entry carrying no guard at
   all, checked by the `DecidedWriteOp` type itself (no `Predicate`
   variant exists to misuse).
5. `cargo test --all-features` green; no existing `WriteBatch` test
   (`tests/server_memory_integration.rs`'s `write_batch_pipelined_
   applies_each_and_atomic_is_all_or_nothing`, etc.) changes behavior.

## Verification plan

`cargo test --all-features` (journal unit tests + the new
crash-injection integration test); `cargo clippy --all-features -- -D
warnings`; `cargo fmt --all --check`. The crash-injection test itself:
since a real `kill -9` mid-process is awkward to script deterministically
in a Rust test, the implementation round should decide between (i) a
real subprocess killed via `taskkill`/`kill` at a controlled point (a
sleep or channel signal placed exactly after the journal `fsync`
returns, before apply begins) or (ii) an in-process fault-injection hook
that returns early after the journal write, skipping apply, then a
fresh `with_journal` open on the same files proves replay recovers it —
matching the precedent `dog.rs`'s own journal tests already use for the
identical `TransactionOp` case (torn-tail and crash-recovery tests
there are in-process, not real process kills).

## Traceability

- Roadmap: `SERVER-WRITE-BATCH-CRASH-ATOMICITY-DESIGN`,
  `SERVER-WRITE-BATCH-CRASH-ATOMICITY`.
- Decision: `ADR-0063`.
- Specification: `SERVER-001` version TBD on acceptance.
- Requirements: `WBJ-FR-001`–`005`.

## Open questions

- **Should journaling become the default** for `memory_server`/
  `entity_server`/a `Relation`-serving binary, now that it would carry a
  real crash-atomicity benefit for `WriteBatch` too, not just
  `Request::Transaction`? Not decided here — a separate, smaller
  question once this round lands.
- **The storage-I/O-failure half of crash-atomicity** (a rollback
  primitive) — still the larger, undecided round `ADR-0060` named.

## Change history

- 2026-09-14: initial proposal, design only.
- 2026-09-14: the owner picked option (a). Implemented on the same
  branch — see `ADR-0063`'s own "Acceptance and implementation" for the
  real mechanism (journals the raw `WriteOp` list, not a pre-decided
  `DecidedWriteOp`) and why it changed from this document's own sketch.
  `SERVER-001` v0.52.0 / FR-062. Proven: `journal.rs`'s existing tests
  unmodified, a new kind-0/kind-1 round-trip test, and a real
  crash-injection test (`write_batch_atomic_is_crash_atomic_via_the_
  journal`, `src/server/memory.rs`) replaying a genuinely committed
  atomic batch's journal onto fresh pre-batch store files.
