# ADR-0018: Fair admission to shared slots

- **Status:** Accepted for Stage 3d
- **Date:** 2026-10-25

## Decision

Slots stay kernel locks in one directory (Stage 3a). Waiting for one becomes
a queue in that same directory, with no daemon and no table that a crash can
leave behind:

- **Tickets.** A moderator that wants a slot of a class holds one ticket for
  it: a locked `queue/<id>.lock` beside its record. When the lock is free,
  the waiter is dead and the ticket is removed. All decisions are made under
  `admit.lock`, so one moderator at a time sees and changes the queue, the
  owners and the slots.
- **Rank, not strict priority.** A ticket's rank is the time it was taken
  less its `priority_secs`. Priority is precedence measured in time, so a
  ticket can be passed only by tickets taken at most the largest priority
  difference after it. A low priority waits longer but never starves. Strict
  priority classes would need aging added on top. This rule has aging built
  in.
- **No head-of-line blocking.** The queue is walked in rank order, and each
  ticket that fits takes a free slot. A ticket that does not fit leaves the
  slot to the next. A slot given to an earlier ticket stays reserved for it
  until its moderator next polls.
- **Ceiling per parent, counted once.** Each held agent slot counts once,
  toward the parent of the task that holds it. A parent's tasks share the
  parent's `agents` ceiling, so two tasks under a ceiling of two hold two
  slots between them, not four. The machine capacity still bounds everyone.
  The parent is the goal (its BBP namespace), and its ceiling is the hosting
  configuration's `slots.ceiling`. Orch reads that and passes it on, as it
  does the lease.
- **Lease-aware.** Two different tasks whose lease write paths overlap, by
  path component, never hold agent slots at once. A task's own harnesses
  share their writes. A waiting task keeps its paths against later tickets,
  so a task whose writes are busy waits its turn and is not passed forever.
  The write paths are the ones the task was frozen under (ADR-0017).
- **Validators** are served in rank order alone.

`bbp mod --claim FILE` carries a task's parent, ceiling, priority and write
paths. orch-bbp writes it into the task's manifest beside `agents.json`.
Like the rest of the pool configuration, it is frozen with the task.

## Not decided here

- No ceiling for validator slots, and no nesting deeper than goal → task. A
  `Grant` field and a chain of ancestors come when a delegating parent first
  exists at runtime. Today every goal's assignment is a root.
- A reserved slot waits up to one poll interval for its moderator. The slot
  directory does not wake waiters.
- Read leases never conflict. Two tasks can read what one of them writes.
- Ceilings come from each claimant's own claim. Two configurations naming
  one parent with different ceilings each bound only their own tasks.

## Consequences

The slot directory gains `admit.lock`, `queue/` and `<class>-<n>.owner`.
A moderator without `--claim` queues fairly with no parent, no priority and no
writes. A slot taken outside admission still counts against capacity.
