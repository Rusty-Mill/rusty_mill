# Server SQL: `WHERE id = '<uuid>'` as a Point Read (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0090`; the `ADR-0059`/`ADR-0076`–`ADR-0089` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the "`WHERE id
  = …` (not expressible in today's `Predicate`)" item of
  `docs/FUTURE-GROWTH.md`'s SQL-parity line — read again, it needs no
  new `Predicate`: it is `GetById`, which the wire has had since
  version 1. **No wire change.** It ships on the same branch as
  `ADR-0089` (protocol 28), so its PR is the one awaiting the owner's
  review; on its own it would have been self-merged as the non-wire
  rounds were.
- Date: 2026-09-21
- Related: `ADR-0034`/`docs/design/SERVER-SQL-SELECT-DESIGN.md` (the
  client-side SQL compiler and `SQL-FR-002`'s name resolution),
  `ADR-0073` (`Query`'s candidate step — a `Predicate` names a schema
  field by tag, which an id is not), `SERVER-001` FR-001 (`GetById`),
  `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive to `src/server/client.rs`:
  `split_id_conditions` and a point-read branch in `query_rows`; the
  predicate evaluator (`predicate_matches`/`compare`) moves from
  `serve.rs` to `protocol.rs` so the client shares the server's exact
  semantics under the `client` feature (`serve.rs` re-exports it). No
  server, wire, or Python change.

## Purpose and scope

`SELECT … FROM t WHERE id = '<uuid>'` has been a parse-time
`UnknownField("id")`: a `Predicate` carries a field tag, and the record
id is not a field. The request the clause means already exists —
`GetById` — so the compiler answers it with one point read: the record
if it exists, the other `WHERE` predicates re-checked client-side with
the server's own evaluator, the `SELECT` list applied, `LIMIT 0` empty.

Scope, exactly: the split (`SID-FR-001`); the point read (`SID-FR-002`);
identity, proven (`SID-FR-003`); no other surface changed (`SID-FR-004`).

## Non-goals

- **`id` in `ORDER BY`, `GROUP BY`, `JOIN`, or an aggregate**: those
  paths keep `UnknownField("id")`. A point read has one row; ordering
  or grouping it is a no-op not worth a code path.
- **`id IN (…)`**, `id != …`, ordering on ids: refused client-side with
  a plain `Sql` error; the wire has no id-set read.
- **A field literally named `id`** in some future schema: the schema
  wins — the split applies only when the schema has no such field.

## Context and terminology

Read from the branch after `ADR-0089` this pass:

- **`split_id_conditions(&conditions)`**: when the schema has no field
  `id`, pulls the one `id = '<uuid>'` condition out (`Eq` and a UUID
  string literal only; a second `id` condition, another comparator, or
  a non-UUID literal is `ClientError::Sql` with no frame) and returns
  the rest.
- **`query_rows`**: after the split, `Some(id)` → `Request::GetById`;
  `Record { id, fields }` whose fields satisfy every remaining
  predicate (`predicate_matches`, now in `protocol.rs`) is the one row,
  projected to the `SELECT` list; `NotFound` or a failed re-check is no
  row; `LIMIT 0` is no row before any round trip. `None` → the
  unchanged `Query` path.
- **`predicate_matches`/`compare`/`ordering_matches`** live in
  `protocol.rs` (no server-only dependency); `serve.rs` re-exports
  `predicate_matches` so every server call site is unchanged.

## Requirements

- `SID-FR-001` **The split.** As "Context".
- `SID-FR-002` **The point read.** As "Context".
- `SID-FR-003` **Identity, proven.** The point read's row is the scan's
  row for that id — proven over a real socket on `Memory` (the row
  equal to `SELECT *`'s; a second predicate that holds and one that
  does not; the `SELECT` list; an unknown id; `LIMIT 0`; `!=`, a
  non-UUID, and two ids refused client-side). Every pre-existing test
  unmodified; the moved evaluator is covered by every existing filter
  test.
- `SID-FR-004` **Everything else unchanged.** The server, the wire
  (`PROTOCOL_VERSION` 28 as `ADR-0089` set it), the Python client.

## Considered options

- **(a) Compile to `GetById` client-side — implemented.** No wire
  change.
- **(b) An `id` pseudo-field in `Predicate`** — a wire round, and a
  planner that would have to special-case it.
- **(c) Decline.** `UnknownField("id")` stays.

The owner's shorthand: **(a)** as implemented; **(b)** an id predicate
on the wire; **(c)** decline and revert.

## Proposed shape

`src/server/protocol.rs`: `predicate_matches`, `compare`,
`ordering_matches`. `src/server/serve.rs`: the re-export.
`src/server/client.rs`: `split_id_conditions`, the branch in
`query_rows`.

## Data/state and invariants

- The point read answers exactly the rows `Query` with the same
  predicates would, restricted to that id: by construction, the same
  evaluator over the same record.

## Errors, failure, recovery, and observability

Three new `ClientError::Sql` messages, client-side, no frame. The point
read is a `GetById` in the access log and in `requests_total`; it is
not a planned read, so `dogserver_query_plans_total` does not count it.

## Security, privacy, and compatibility

`GetById`'s own gate; nothing new is reachable.

## Acceptance criteria

1. Integration (`tests/server_sql_integration.rs`): `SID-FR-003`'s
   shapes.
2. Every existing filter test green with the evaluator moved.
3. Not measured: a point read is `GetById`'s existing cost (`RESULTS.md`
   has it since `ADR-0043`); nothing new to compare.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`.
Independent review owed.

## Traceability

- Roadmap: `SERVER-SQL-ID-PREDICATE`.
- Decision: `ADR-0090`.
- Specification: `SERVER-001` v0.75.0 / `FR-087` (extends `FR-034`).
- Requirements: `SID-FR-001`–`004`.

## Open questions

- **`id IN (…)`** — a batch point read; a wire round if wanted.
- **The Python client** — its `query` takes `(field, op, value)`
  tuples, not SQL; an `id` tuple could compile the same way.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction — the
  future-growth item read as a client-side compilation, not a wire
  change. On the same branch and PR as `ADR-0089`, awaiting the owner's
  review.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.75.0 /
  `FR-087`, exactly the "Proposed shape". Acceptance criterion 1 is
  `tests/server_sql_integration.rs` +1
  (`where_id_equals_is_a_point_read_that_matches_the_scan`); criterion
  2 the unchanged suite (lib 646; SQL 63, up from 62); 938 tests
  across 39 targets, 0 failed. Still no independent review — owed.
