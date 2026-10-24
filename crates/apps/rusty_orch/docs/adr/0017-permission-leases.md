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

- **Confinement at freeze.** The first admission freezes the task's pool with
  each assigned member's sandbox met with the lease, after resolving both sides
  through symlinks. Nothing is kept beneath a kept write root, so a harness
  cannot swap a path another rule names. An assigned member with no sandbox,
  or a `checkout` the lease does not reach, is refused: a lease never runs
  unconfined.
- **Frozen reach.** A later fence whose lease reaches other paths, or none, is
  refused as permanent. Expanding access needs a new grant and a task started
  under it, so a running confined process never gains reach. This is the
  roadmap's "restart with a revised sandbox spec", taken at task granularity.
- **Expiry.** An ended lease is refused as `Unavailable`, so a lapsed holder
  stops driving the task without failing it. Each moderator the host starts has
  its `--max-wall-secs` cut to what is left of the lease, so BBP kills its
  harnesses and runner at the end even with no Orch process watching. A later
  run renews the lease and restarts the moderator. Cancel is never refused for
  an ended or different lease: it only removes authority.

## Not decided here

- No network flag. Confined harnesses already have internet sockets only and
  no switch exists in BBP; a ledger flag without an OS boundary would claim
  enforcement it does not have.
- No in-place re-confinement of a live task, and no shortening of a running
  moderator's lifetime by a later, shorter renewal.

## Consequences

Assignment records gain a v2 schema in `assignments-v2.mmap`. A goal saved
only as v1 still loads, as unleased, and its next save writes v2; the v1 file
is never written again.
