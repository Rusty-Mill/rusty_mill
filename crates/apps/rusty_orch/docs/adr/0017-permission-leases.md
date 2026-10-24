# ADR-0017: Permission leases on assignment grants

- **Status:** Accepted for Stage 3c
- **Date:** 2026-10-24

## Decision

A `Grant` carries an optional `Lease`: absolute, normalized read and write
paths and an optional end in unix seconds. `orch-core` checks it as data only:
a leased parent delegates or re-grants only a lease it covers (write inside
write, read inside read or write, an end no later than its own), and `restore`
re-checks that for every currently bound child. An unleased grant is what every
grant was before this ADR; only the hosting configuration bounds its workers.

Orch grants the lease. The hosting configuration's `lease` object
(`{read, write, ttl_secs}`) is read by `rusty_orch`, never by the host: each run
takes the goal's assignment over with the lease and a fresh end, persists it in
the assignment record, then passes it on the `Fence` of every host command.

The BBP host enforces the fence's lease, not its configuration:

- **Canonical leases.** Every lease path must exist and be its own canonical
  path. `orch-core` compares a child's lease with its parent's by spelling;
  with no symlink along any leased path, that spelling is what the kernel
  opens, so a child lexically inside its parent is inside it in fact.
- **Confinement at freeze.** The first admission freezes the task's pool with
  each assigned member's sandbox met with the lease, the sandbox's paths
  resolved through symlinks. An assigned member with no sandbox, or a
  `checkout` the lease does not reach, is refused: a lease never runs
  unconfined.
- **No swappable roots.** Landlock opens each rule's root and follows a
  symlink there, so a root lying beneath any harness's write root is one that
  harness can redirect before a later launch, its own or another role's. Such
  a pool is refused, not pruned, so no member is widened or silently cut:
  by orch-bbp at freeze over the members' paths, and by `bbp mod` at start
  over every root it adds too (system roots, its own directory, the
  repository's git directory, the work directory).
- **Frozen reach.** A later fence whose lease reaches other paths, or none, is
  refused as permanent. Expanding access needs a new grant and a task started
  under it, so a running confined process never gains reach. This is the
  roadmap's "restart with a revised sandbox spec", taken at task granularity.
- **Expiry is an absolute deadline.** An ended lease is refused as
  `Unavailable`, so a lapsed holder stops driving the task without failing it.
  The lease's end goes to `bbp mod --deadline-unix` as an absolute time, so
  startup and health checks spend none of it. No end grace runs past it, the
  loop stops at it as at the wall, and each harness's `bbp slot-hold` holder,
  which leads the harness's process group, kills the group at it, so the
  deadline holds even when the moderator has died. A later run renews the
  lease and restarts the moderator. Cancel is never refused for an ended or
  different lease: it only removes authority.

## Not decided here

- No network flag. Confined harnesses already have internet sockets only and
  no switch exists in BBP; a ledger flag without an OS boundary would claim
  enforcement it does not have.
- No in-place re-confinement of a live task, and no shortening of a running
  moderator's deadline by a later, shorter renewal.
- Writers outside the task: another task's harness or any other local process
  that can write above a leased root can still replace it. Leases bound this
  task's own harnesses; pinning root identity through launch would cover the
  rest and is not built.
- A process that leaves its harness's process group escapes the holder's
  kill, as it escapes cleanup today.

## Consequences

Assignment records gain a v2 schema in `assignments-v2.mmap`. A goal saved
only as v1 still loads, as unleased, and its next save writes v2; the v1 file
is never written again.
