# ADR-0054: A guarded replace — `Request::ReplaceIf` with a predicate over the stored record, at protocol 19

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-07 — the owner's "accept as designed", option (a): a guarded
  replace with one query predicate over the stored record, evaluated
  under the write lock; (b) a dedicated `ReplaceIfNewer`, (c) a
  server-maintained version counter, (d) client-side, and (e) decline
  declined. Recorded in "Acceptance and implementation" below.)
  Proposed and implemented on one branch, the `ADR-0046`–`ADR-0053`
  cadence.
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-GUARDED-REPLACE-DESIGN.md` (the full
  design), `ADR-0049` (the unconditional replace), `ADR-0034`
  (`Predicate`, reused), `ADR-0046` (`Duplicate`, the precedent for a
  normal outcome as a code), `docs/reports/2026-09-07-hub-spike-report.md`
  (gap 1).
- Supersedes/Superseded by: none. Appends `Request::ReplaceIf` (28) and
  `ErrorCode::GuardFailed` (13) at protocol 19; no file format change.

## Context

The consumer's hub merges records by last-writer-wins in one atomic
SQL statement. Against this crate the spike had to read, compare, and
`Replace` — not atomic, so a concurrent writer can be silently
overwritten by an older version. The spike named it the first of its
gaps.

## Decision

Add a guarded replace: `Request::ReplaceIf` carries `Replace`'s body
plus one `Predicate` — a `Query` filter's own type — that the server
evaluates against the stored record under the table's write lock,
replacing only if it holds. The guard is validated exactly as a query
predicate. Answered `Ok`, `NotFound`, or `Err { GuardFailed }` (nothing
written in the latter two). The mechanism is one method on the
production store over its existing bounds; no new store trait, no
layer changes, no version counter, no format change. Both clients gain
`replace_if`.

## Consequences

- Positive: the hub's merge becomes one atomic round trip; compare-
  and-swap on any numeric field comes for free (`Eq`).
- Positive: additive; a pre-19 client never sees the new code.
- Named, not hidden: one predicate per guard; a refused caller reads
  the winner itself; an upsert stays the client's two calls.
- Named, not hidden: `GuardFailed` is an `ErrorCode` for a normal
  outcome — `Duplicate`'s precedent, and both clients map it to a value.

## Considered options

**(a) Accept as designed** — a guarded replace with a query predicate,
under the write lock. **(b) A dedicated `ReplaceIfNewer`** — a second
shape the day `Eq` is wanted. **(c) A server-maintained version
counter** — a format change for what the caller's timestamp gives.
**(d) Leave it client-side.** **(e) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.44.0 / FR-054, `SERVER-002` v0.8.0 —
  `src/generic/{mod,production}.rs`, `src/server/{protocol,serve,audit,
  memory,reminder,entity,client}.rs`, `clients/python/**`, two vectors
  (61 pinned); tests: `production` +1, `server/memory` +1,
  `server_memory_integration` +1, `server_protocol_version` +1,
  `server_dog_integration` +1, `server_auth_integration`/
  `server_transaction_integration`/`server_python_client` extended;
  every acceptance criterion 1–4 holds. (PR #212.)
- 2026-09-07: accepted as designed (option (a); (b)–(e) declined). No
  change to the implementation. Next in the line: a range-scannable
  `updated_at` with an ordered page.
