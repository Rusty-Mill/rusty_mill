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

`TaskStore` owns `tasks.journal`. `replace_batch` validates and encodes every
replacement, commits one redo batch, applies all task puts with store syncing
deferred, syncs the task store once, publishes the derived full-text and tag
changes, and finally checkpoints the journal. Startup replays every committed
batch into the durable task stack before rebuilding derived indexes and
checkpointing.

`Service::trash_task` preserves an already-present child deletion timestamp and
bumps the update timestamp and version of every child and the parent just as
individual saves did. It submits those replacements as one batch. Restoration,
move, purge, list deletion, and other writes retain their existing behavior.

The durable boundary is `Journal::commit`: a refusal before it changes nothing;
a successful commit may be replayed as a whole after interruption at any later
prefix. The derived indexes are not changed while a batch is only partly
applied. Tests simulate process loss by dropping isolated synthetic stores at
each apply prefix; this verifies redo/reopen behavior, not physical power-loss
properties of a particular filesystem or device.

## Consequences

- Acknowledged trash work survives and converges to the complete intended
  parent/child set on reopen.
- A pre-commit refusal leaves the previous committed records and indexes.
- Replay is idempotent and unrelated tasks are untouched.
- The journal is an additive sidecar, not a task-record migration. Existing
  `tasks.mmap` stores create an empty journal on first open.
- A post-commit error cannot promise rollback: the redo entry is authoritative
  and startup completes it.
