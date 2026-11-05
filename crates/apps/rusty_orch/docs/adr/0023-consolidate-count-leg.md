# ADR-0023: `rusty_orch consolidate`, D44's count leg

- **Status:** Accepted (Let Em Loose D44, build defaults D110 and D113)
- **Date:** 2026-11-05

## Context

D44 has orch schedule an erosion cleanup after 5 accepted claims, or when
cumulative erosion reaches half the limit. Under D110 only the count leg is
built: the half-limit leg waits on Nano, and nothing is approved for it to
measure against while erosion is report-only (D42). D113 fixes the inputs:
explicit `bbp open` files, a `--cleanup` erosion profile based at the head
orch measured, and repos matched by the opaque `TaskOpened.repo`.

## Decision

- `rusty_orch consolidate` is a read-only JSON plan by default, like
  `maintain`. `--execute` writes the journal and runs `bbp open --mode
  claims` as a subprocess, so orch gains no dependency on the host crate.
- The journal lives in `<git common dir>/rusty-orch-consolidate/`, under a
  lock, and is bound to one `--repo` and BBP directory; any other pair is
  refused.
- The cleanup marker is each counted task's log length when the last
  cleanup opened or was skipped. An acceptance counts if its event comes
  after that point. Logs are append-only, so this is exact, and the
  journal stays small.
- Consolidate tasks are the ones the journal records opening, not tasks
  with a matching name. Their claims never count, and while one is open no
  new cleanup is due.
- The open is written ahead as `pending`. Whether it landed is read from
  the store, not from `bbp open`'s output: if the task exists, the marker
  moves; if it doesn't, the pending entry is dropped and the next run plans
  afresh.
- The zero-floor skip (both base shares 0) moves the marker and opens
  nothing. The levels are `concentration_bp.base` and `duplication_bp.base`
  from `erosion check --base HEAD --report-only`. They are read from the
  commit, so uncommitted changes in the checkout don't affect them.

## Consequences

- An operator or schedule runs `consolidate --execute` after claims are
  accepted. orch doesn't watch the store for them.
- Adding the half-limit leg changes only `due`; the journal and the open
  path stay as they are.
