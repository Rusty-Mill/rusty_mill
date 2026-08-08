# ADR-0001: The replica executing a run is its sole writer

Status: Accepted
Date: 2026-08-05

## Context

ACP's high-availability guide calls for identical replicas behind a load balancer with no session
affinity: any replica must serve any request for any run. That means a run created on replica A
can be read on B, streamed from C and cancelled through D.

Concurrent writes to one run's state were therefore the obvious hazard, and the obvious answers —
compare-and-swap on every write, or a distributed lock per run — both cost a round trip on the
hot path and both have to be implemented correctly by every backend, including third-party ones.

## Decision

**Only the replica executing a run writes that run.** Everyone else reads snapshots and sends
control signals (`Notification::Resume`, `Notification::Cancel`) through the store's pub/sub; the
executing replica applies them and writes the result.

`Store::put_run` is therefore a plain overwrite. Implementors do not serialise concurrent writes
to the same run, because there are none.

## Alternatives considered

- **Compare-and-swap on `put_run`.** Correct without the invariant, and it makes every write a
  read-modify-write. It also pushes the hardest part of the contract onto every backend author,
  which matters now that `store-testkit` invites third-party backends.
- **A distributed lock per run.** Same cost, plus a lock service to operate and a new failure mode
  when the lock outlives its holder. The lease below is a weaker form of this that buys what is
  actually needed.
- **Session affinity at the load balancer.** Removes the problem by removing the property the
  design exists for.

## Consequences

The invariant's weak point is that a writer can *die*, leaving a run non-terminal forever with
nothing to consume a resume or apply a cancel. `Store::renew_lease` closes that: the executing
replica keeps renewing, so **a non-terminal run with no live lease has lost its writer** and is
reaped by whichever replica next reads it.

Three further rules follow and are load-bearing:

- **Terminal transitions apply exactly once**, so a cancellation racing a completion cannot
  rewrite the outcome.
- **The terminal event releases `sync` callers**, so anything a caller could reasonably read next
  — most sharply the session history — must be written before it goes out.
- **Storage failures fail the run.** Emitting is `async` and returns `Result` so a storage outage
  produces a failed run rather than a silently truncated one.

What this forecloses: any feature where two replicas legitimately write one run. Recovery works
around it rather than through it — an abandoned run is *replaced* by a fresh linked run under a
claimed lease, not resumed in place.
