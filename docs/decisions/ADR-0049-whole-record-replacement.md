# ADR-0049: Whole-record replacement — `Replace<R>` over the insert log, `Request::Replace` at protocol 15

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-06 — the owner's "accept as designed", option (a): whole
  replace over the insert log, `Insert`'s body and gates,
  `UpdateField`'s reply, a client-side `upsert`; (b) partial update,
  (c) delete-then-insert, (d) a server-side `Upsert`, and (e) decline
  all declined. Recorded in "Acceptance and implementation" below.)
  Proposed and implemented on one branch, the `ADR-0046`/`ADR-0047`/
  `ADR-0048` cadence.
- Date: 2026-09-06
- Deciders: baileyrd
- Related: `docs/design/SERVER-REPLACE-DESIGN.md` (the full design),
  `ADR-0046` (the insert log and `Insert`'s validation, both reused),
  `ADR-0048` (named this round as next), `ADR-0013` (`UpdateField`'s
  `Ok`/`NotFound` reply, reused), `ADR-0034` (client-side capability
  with no wire primitive — `upsert`).
- Supersedes/Superseded by: none. Appends `Request::Replace` (23) at
  protocol 15; no new `Response`, no new `ErrorCode`, no new file.

## Context

A record could change in one field only — its scannable one, through
`UpdateField`. The consumer's `update_memory` rewrites five fields at
once, `entity_upsert` is insert-or-replace, `reclassify` rewrites
`memory_type`; `ADR-0048`'s `Memory` adapter refused all of them with
`Unsupported` and named this round as the fix. The insert log
(`ADR-0046`) already stores whole records and folds them into the blob
at the next open; the only thing it could not do was carry a *second*
version of an id, because the fold skipped a held id instead of
applying it.

## Decision

Add `Replace<R>` — replace a record whole, by id, with `Insert`'s exact
body — implemented on `GenericMmapStore` by appending the new version
to the insert log, rewriting the scannable slot in place, and moving
the index bucket; make the fold apply a logged record *over* the blob's
copy by id (the log is the later fact); forward through every layer
with the old and new record in hand so `NameIndex` moves keys and
`Reversed` moves a child; `ConnectionStore::replace_record` with
`Memory`/`Reminder`/`Entity` implementing it through the same
validation as insert; `Request::Replace` at 23, protocol 15, answered
`Ok`/`NotFound` and gated exactly as `Insert`; `SchemaDrivenClient::
replace` and a client-side `upsert`; Python `Client.replace`.

## Consequences

- Positive: the consumer's `update_memory`, `entity_upsert`, and
  `reclassify` have a backend path; every prior round's "read-only
  after insert" is now "replaceable whole".
- Positive: no new file, no new response, no new error code; one
  appended request. The fold's new "log wins" rule changes no existing
  directory's reopen.
- Named, not hidden: the log entry lands before the slot write, so a
  crash between them leaves one field (the scannable one) at its
  previous value — the same state an `UpdateField` that landed and a
  replace that did not would leave; bounded to one field.
- Named, not hidden: `upsert` is two round trips and not atomic across
  them; harmless without runtime deletion, revisited if measured.
- Still no runtime deletion, no partial update, no id change.

## Considered options

**(a) Accept as designed** — whole replace over the insert log,
`Insert`'s body and gates, `UpdateField`'s reply, client-side `upsert`.
**(b) Partial update** (`Update { id, fields }` patching a subset) —
needs a per-field settable flag the schema struct cannot gain under
rule 1, and a merge per adapter. **(c) Delete-then-insert** — needs
deletion first and loses edges in between. **(d) Also a server-side
`Upsert` request** — one round trip instead of two; deferred until
measured. **(e) Decline.**

## Acceptance and implementation

- 2026-09-06: proposed and implemented on the same branch as
  `SERVER-001` v0.39.0 / FR-049, `SERVER-002` v0.4.0 —
  `src/generic/{query,mod,mmap_store,store,mmap_scanned,production}.rs`,
  `src/server/{protocol,serve,audit,memory,reminder,entity,client}.rs`,
  `clients/python/**`, one vector (53 pinned); tests: `mmap_store` +2,
  `memory` +1, `entity` +1, `employee_impl` +1 (store); `memory`/
  `reminder`/`entity` adapters +1 each; integration `server_memory_
  integration` +1, `server_entity_integration` +1, `server_protocol_
  version` +1, `server_dog_integration` +1, `server_auth_integration`/
  `server_transaction_integration`/`server_python_client` extended;
  every acceptance criterion 1–4 holds. (This PR.)
- 2026-09-06: accepted as designed (option (a); (b)–(e) declined). No
  change to the implementation. Next in the line: the `ADR-0045`
  implementation round (`memory_entities` as the first cross-table
  link); runtime deletion stays the last standing clause.
