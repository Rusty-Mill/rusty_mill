# ADR-0003: Journal parent/child trash as one durable batch

## Status

Accepted (2026-10-02).

## Context

Trashing a task also trashes its current direct subtasks. The former
implementation replaced each child and then the parent independently. A crash
or storage failure between replacements could therefore persist only a prefix.
Unlike list placement, independent deletion timestamps contain no invariant
from which startup can distinguish an interrupted family operation from a
child that was deliberately trashed earlier.

The embedded storage engine already provides a redo `Journal`: a caller syncs
one batch before applying its idempotent whole-record puts, syncs the affected
store, and checkpoints afterward. An uncheckpointed accepted batch is returned
on open and may safely be applied again.

## Decision

`TaskStore` owns `tasks.journal`. The batch entry point is crate-private and is
used only by the trash transition, whose replacements do not change list or
sort slots. `replace_batch` rejects duplicate ids, then validates and encodes every
replacement, commits one redo batch, applies all task puts with store syncing
deferred, syncs the task store once, checkpoints the journal, and only then
publishes the derived full-text and tag changes. Startup replays every committed
batch into the durable task stack before rebuilding derived indexes and
checkpointing.

`Service::trash_task` preserves an already-present child deletion timestamp and
bumps the update timestamp and version of every child and the parent just as
individual saves did. It submits those replacements as one batch. Restoration,
move, purge, list deletion, and other writes retain their existing behavior.

Validation failures are definite pre-commit refusals and change nothing. Any
I/O error returned by `Journal::commit` is ambiguous: its write may have landed
before its sync failed. Such an error, or any apply, store-sync, or checkpoint
error after acceptance, fences the live store. It retains a complete
pre-operation read view and refuses every later mutation until reopen; thus a
later operation can neither retire an earlier redo nor be overwritten by it.
The derived indexes are not changed until checkpoint succeeds. Tests simulate
process loss by dropping isolated synthetic stores at
each apply prefix; this verifies redo/reopen behavior, not physical power-loss
properties of a particular filesystem or device.

Opening inspects the journal before the task store. When authoritative redo is
pending, it may remove an empty insert log or an exact, incomplete prefix of
the expected insert-log header left by interrupted creation. Established logs,
valid entries, foreign headers, and all other malformed data remain subject to
the engine's normal parsing and corruption refusal.

## Consequences

- Acknowledged trash work survives and converges to the complete intended
  parent/child set on reopen.
- A definite pre-commit refusal leaves the previous committed records and
  indexes; an ambiguous or post-acceptance failure requires reopen.
- Replay is idempotent and unrelated tasks are untouched.
- The journal is an additive sidecar, not a task-record migration. Existing
  `tasks.mmap` stores create an empty journal on first open.
- A post-commit error cannot promise rollback: the redo entry is authoritative
  and startup completes it.
