# Server Guarded Replace Design (Proposed)

- Status: **Proposed** (2026-09-07) — implemented on the same branch as
  `SERVER-001` v0.44.0 / FR-054 and `SERVER-002` v0.8.0, the
  `ADR-0046`–`ADR-0053` cadence, for the owner to accept as designed or
  send back.
- Date: 2026-09-07
- Related: `ADR-0049`/`docs/design/SERVER-REPLACE-DESIGN.md` (the
  unconditional replace this round guards), `ADR-0034` (`Predicate`/
  `CompareOp`, reused as the guard, and `SQL-FR-007`'s validation rules),
  `ADR-0046` (`ErrorCode::Duplicate`, the precedent for a normal outcome
  carried as a code), `ADR-0032` (`ServeOptions`),
  `docs/reports/2026-09-07-hub-spike-report.md` (gap 1, the round's
  evidence).
- Supersedes/Superseded by: none. Appends one `Request` variant and one
  `ErrorCode` at protocol 19; changes no file format.

## Purpose and scope

The consumer's hub merges records from many nodes by last-writer-wins:
`INSERT … ON CONFLICT DO UPDATE … WHERE excluded.updated_at >
memories.updated_at`, one atomic statement. Against this crate the
spike had to do it as `get`, compare, `Replace` — three round trips and
not atomic: a second writer can land between the comparison and the
write and be silently overwritten by an older version. The spike named
it the first of its gaps. This round closes it with the smallest
addition that is also general: a replace with a **guard**, a predicate
evaluated against the stored record under the table's write lock.

Scope, exactly: `GenericProductionStore::replace_if` (`GRD-FR-001`);
`GuardedReplace` (`GRD-FR-002`); `ConnectionStore::replace_record_if`
on `Memory`/`Reminder`/`Entity` (`GRD-FR-003`); guard validation as a
query predicate (`GRD-FR-004`); `Request::ReplaceIf` and
`ErrorCode::GuardFailed` at protocol 19 with `Replace`'s gates
(`GRD-FR-005`); both clients (`GRD-FR-006`); the pins and `SERVER-002`
v0.8.0 (`GRD-FR-007`).

## Non-goals

- **A store-layer trait through every layer.** The guard needs the
  stored record, the comparison, and the write under one lock; the
  production store already holds that lock. One method there, over the
  existing `GetById` and `Replace` bounds, is the whole mechanism — no
  new trait, no eight forwarding impls, no format change.
- **A server-side upsert.** Insert-or-replace stays the client's two
  calls; a guarded *insert* has no stored record to guard against.
- **Multi-predicate guards, or guards over other records.** One
  predicate over the record being replaced. `AND` of several is a
  later `Vec<Predicate>` if a caller needs it; the wire shape leaves
  room (a `Vec` is one more field, appended).
- **Returning the stored record on refusal.** A refused caller that
  wants the winner reads it; the response stays `Replace`'s.
- **A version counter the server maintains.** The guard field is the
  caller's (`updated_at`, or any `U32`/`I64` it keeps); the server
  compares, it does not count.

## Context and terminology

- **Guard**: a `Predicate { field, op, value }` — a `Query` filter's
  own type — evaluated against the *stored* record's wire shape. It
  holds when `stored.field op value` is true. Last-writer-wins is
  `{ updated_at, Lt, mine }`; compare-and-swap on a version field is
  `{ version, Eq, expected }`.
- **Atomic**: the read of the stored record, the guard, and the write
  happen under one acquisition of the table's write lock, so no other
  writer's change can land in between.
- **Refused**: the guard did not hold; nothing was written. A normal
  outcome, not a failure — carried as `ErrorCode::GuardFailed` because
  it is neither `Ok` nor `NotFound` (`Duplicate`'s reason).

## Requirements

- `GRD-FR-001` **`GenericProductionStore::replace_if(record, guard:
  FnOnce(&R) -> bool)`** under the write lock: `get` the stored record
  (`NotFound` if absent, the guard never called), evaluate the guard,
  `replace` only if it holds. Bounds: `S: Replace<R> + GetById<R>`.
- `GRD-FR-002` **`crate::generic::GuardedReplace { Replaced, Refused }`**,
  the method's `Ok`; `ReplaceError` its `Err`.
- `GRD-FR-003` **`ConnectionStore::replace_record_if(id, fields, &guard)
  -> Result<ReplaceIfOutcome, ErrorCode>`** with `ReplaceIfOutcome {
  Replaced, NotFound, GuardFailed }`; default `Unsupported`; `Memory`/
  `Reminder`/`Entity` validate `fields` as `replace_record` does and
  evaluate the guard with `predicate_matches` over their own `fields_of`
  of the stored record (`Reminder`/`Entity` gain `fields_of`, factored
  from `get`).
- `GRD-FR-004` **Guard validation**: `validate_predicate(schema,
  guard)` — factored out of `validate_query`, the same rule
  (`UnknownField` for an unknown tag; `Malformed` for a value of another
  kind or an ordering comparator on a field that is not `U32`/`I64`) —
  runs in `dispatch` before the adapter is called; nothing evaluated on
  refusal.
- `GRD-FR-005` **The request.** `Request::ReplaceIf { id, fields, guard:
  Predicate }` (28), `ErrorCode::GuardFailed` (13), `PROTOCOL_VERSION`
  19; answered `Ok` / `NotFound` / `Err { GuardFailed }`;
  `handle_connection` refuses `ReadOnly` (`Unauthorized`, the eighth
  write), `SessionOpen` inside a session, `Malformed` below 19;
  `audit::RequestKind::ReplaceIf`; `GuardFailed` only ever answers
  `ReplaceIf`, so no downgrade arm.
- `GRD-FR-006` **Clients.** `SchemaDrivenClient::replace_if(id, fields,
  (field, CompareOp, value)) -> Result<GuardedReplace, _>` with
  `client::GuardedReplace { Replaced, Refused, NotFound }`; the guard
  field resolved by name, an ordering comparator on a non-numeric field
  refused locally (`Unsupported("ordering guard")`, the `SQL-FR-010`
  rule), `Unsupported("replace_if")` below 19. Python
  `Client.replace_if(...)` returning `"replaced"`/`"refused"`/
  `"notfound"`, `PROTOCOL_VERSION = 19`.
- `GRD-FR-007` **Pins.** Two golden vectors (`Request/ReplaceIf`,
  `Response/Err(GuardFailed)`; 61 pinned); `SERVER-002` v0.8.0; the
  literal `Hello` pins moved to 19.

## Considered options

- **(a) A guarded replace — one predicate over the stored record,
  evaluated under the write lock — proposed.** Reuses the query
  predicate's type and validation; the mechanism is one method on the
  production store; general enough for compare-and-swap.
- **(b) A dedicated `ReplaceIfNewer { field }`.** Exactly last-writer-
  wins and nothing else; a second shape the day a caller wants `Eq`.
- **(c) A server-maintained version counter with `Replace { expected_
  version }`.** Adds a field to every record and a format change, for a
  guarantee the caller's own timestamp already provides.
- **(d) Leave it client-side.** The hub's merge stays non-atomic.
- **(e) Decline.**

## Proposed shape

`src/generic/mod.rs` (`GuardedReplace`), `src/generic/production.rs`
(`replace_if`); `src/server/protocol.rs` (the variant, the code, the
constant, the table row, two vectors), `src/server/serve.rs`
(`ReplaceIfOutcome`, the trait method, `validate_predicate`,
`predicate_matches` made `pub`, one `dispatch` arm, three gates),
`src/server/audit.rs`, `src/server/{memory,reminder,entity}.rs`,
`src/server/client.rs`, `clients/python/**`.

Wire: `ReplaceIf` = `[0x1c 0 0 0]` · `id` · `fields` (as `Replace`) ·
`guard.field` (`u16`) · `guard.op` (`u32`) · `guard.value`
(`ScanValue`). `Err { GuardFailed }` = `[0x08 0 0 0]` · `[0x0d 0 0 0]`
· message.

## Data/state and invariants

- After `Replaced`, the record is the new version exactly as after
  `Replace`; after `Refused` or `NotFound`, no file changed.
- Between the guard and the write no other writer runs: the outcome a
  caller sees is the outcome of its guard against the record as it was
  at that instant.
- A guard is validated before it is evaluated; a `Str`/`Bool`/list
  field never meets an ordering comparator.

## Errors, failure, recovery, and observability

`UnknownField`/`Malformed` from validation (nothing evaluated);
`GuardFailed` when the guard did not hold; `NotFound` for a missing id;
`Storage` if the durable core could not persist the new version (the
guard held); `Unsupported` from a domain without replacement. The audit
and access logs carry the request kind and the outcome.

## Security, privacy, and compatibility

A write: `ReadOnly` refused, sessions refused, gated below 19. No file
format change. A pre-19 client is unaffected; `GuardFailed` never
reaches it.

## Acceptance criteria

1. Store: on `Order`, a guard `stored.created_at < 5_000` replaces; an
   older version's guard fails with nothing written; an unknown id is
   `NotFound` with the guard never called.
2. Adapter (`Memory`): `updated_at < mine` replaces a newer version and
   refuses an older one with the stored record intact; an unknown id is
   `NotFound`; an invalid field list is `Malformed` before any guard.
3. Wire (`memory_server`'s stack over a socket): a newer version is
   `Replaced`; an older one and an equal timestamp are `Refused` with
   nothing written; `Eq` on the stored timestamp holds (compare-and-
   swap); an unknown id is `NotFound`; a guard value of the wrong kind
   is the server's `Malformed`; an ordering guard on `content` and an
   unknown guard field are refused by the client with no frame; the
   winner survives a restart. `Malformed` at 18 and silent, served at
   19; `Unsupported("replace_if")` at 1; `Unauthorized` for `ReadOnly`;
   `SessionOpen` in a session; `Unsupported` on `Dog`. From Python:
   holds, refused with the stored value intact, unknown id.
4. Two vectors added, every earlier vector byte-identical; `SERVER-002`
   v0.8.0; the Python suite passes offline and live.

## Verification plan

`cargo test --all-features` / default / `client`; clippy `-D warnings`
on the same three; `cargo fmt --check`; the Python vector suite; `cargo
doc --all-features --no-deps` at the baseline.

## Traceability

- Roadmap: `SERVER-GUARDED-REPLACE-DESIGN`, `SERVER-GUARDED-REPLACE`.
  Decision: `ADR-0054`.
- Specification: `SERVER-001` v0.44.0 / `FR-054`; `SERVER-002` v0.8.0.
- Requirements: `GRD-FR-001`–`007`.

## Open questions

- **A range-scannable `updated_at` with an ordered page** — the spike's
  gap 2 and the next round: every hub pull is a full-table scan today.
- **Several predicates in one guard** — a `Vec<Predicate>` if a caller
  ever needs `AND`; not speculated now.
- **`deleted_at`/`node_id` in the projection**, **directed open-label
  edges**, **a global edge count** — the spike's gaps 3–5, unchanged.
- **`ADR-0053`'s open questions** (the other binaries, a binary-level
  test harness) — unchanged.

## Change history

- 2026-09-07: Initial proposal; implementation follows on the same
  branch. The nineteenth round in the `rusty_remind_me`-motivated line;
  the hub spike's first gap.
