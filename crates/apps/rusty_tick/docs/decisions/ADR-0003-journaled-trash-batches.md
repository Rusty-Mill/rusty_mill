# ADR-0003: Journal parent/child transitions as one durable batch

## Status

Accepted (2026-10-02).

## Context

Trashing, restoring, or moving a task also changes some or all of its current
direct subtasks. The former implementation replaced each child and then the
parent independently. A crash or storage failure between replacements could
therefore persist only a prefix.
Unlike list placement, independent deletion timestamps contain no invariant
from which startup can distinguish an interrupted family operation from a
child that was deliberately trashed earlier.

The embedded storage engine already provides a redo `Journal`: a caller syncs
one batch before applying its idempotent whole-record puts, syncs the affected
store, and checkpoints afterward. An uncheckpointed accepted batch is returned
on open and may safely be applied again.

## Decision

`TaskStore` owns `tasks.journal`. The crate-private batch entry point is used by
trash, restore, and parent moves. `replace_batch` rejects duplicate ids, then
validates and encodes every replacement, commits one redo batch, applies all
task puts with store syncing deferred, syncs the task store once, checkpoints
the journal, and only then publishes the derived full-text and tag changes.
Startup replays every committed batch into the durable task stack before
rebuilding derived indexes and checkpointing. Ordinary one-record operations
continue to use the direct store path.

The engine's group-commit barrier syncs the insert log and then flushes the
mapped sort-order slots before returning success. The journal remains
authoritative until that complete barrier succeeds and its later checkpoint
succeeds. Thus a move PATCH that changes both `list_id` and `sort_order` cannot
retire redo after syncing only the record: an interruption after record append,
during slot flush, or around journal retirement leaves redo for an idempotent
whole-record replay on open.

`Service::trash_task` preserves an already-present child deletion timestamp and
bumps the update timestamp and version of every child and the parent just as
individual saves did. Restore revives only children trashed with the parent;
when the old list is unavailable, it rehomes every child while preserving a
separately trashed child's deletion timestamp. A parent move journals the
children and parent together, including a simultaneous parent sort-order
change. Purge, list deletion, and other writes retain their existing behavior.

Validation failures are definite pre-commit refusals and change nothing. Any
I/O error returned by `Journal::commit` is ambiguous: its write may have landed
before its sync failed. Such an error, or any apply, store-sync, or checkpoint
error after acceptance, fences the live store. It retains a complete
pre-operation read view and refuses every later mutation until reopen; thus a
later operation can neither retire an earlier redo nor be overwritten by it.
An ordinary task insert, replacement, or deletion durability error is fenced
for the same reason: its append may have left an ambiguous partial frame.
The engine's core and `Ordered` layers publish their in-memory record and
index changes only after the fallible persistence call succeeds. Ordinary
writes therefore capture the retained full-store view only on an error;
successful single-record writes remain proportional to that record and do not
clone or scan unrelated tasks. Batch operations still capture their complete
pre-operation view before journal acceptance because a batch may partially
apply after that point.
Compound service operations check this readiness before their first side
effect, so they cannot mutate the tag or list registries before discovering a
task-store fence. Duplicate and missing-record validation refusals remain
definite and do not poison the store.
The derived indexes are not changed until checkpoint succeeds. Deterministic
tests exercise the real operation path at a partial journal frame, journal
sync, mid-apply, store group-sync, and checkpoint post-rename boundary, plus a
torn ordinary append. These process/syscall experiments verify redo/reopen
behavior, not physical power-loss properties of a particular filesystem or
device.

Opening inspects the journal before the task store. When authoritative redo is
pending, it may remove an empty insert log or an exact, incomplete prefix of
the expected insert-log header left by interrupted creation. Established logs,
valid entries, foreign headers, and all other malformed data remain subject to
the engine's normal parsing and corruption refusal.

## Consequences

- Acknowledged trash, restore, and move work survives and converges to the
  complete intended parent/child set on reopen.
- A definite pre-commit refusal leaves the previous committed records and
  indexes; an ambiguous or post-acceptance failure requires reopen.
- Replay is idempotent and unrelated tasks are untouched.
- The journal is an additive sidecar, not a task-record migration. Existing
  `tasks.mmap` stores create an empty journal on first open.
- A post-commit error cannot promise rollback: the redo entry is authoritative
  and startup completes it.
