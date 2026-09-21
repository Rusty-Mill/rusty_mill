# ADR-0090: `WHERE id = '<uuid>'` as a Point Read

- Status: **Proposed and implemented on one branch** (2026-09-21; the
  `ADR-0059`/`ADR-0076`–`ADR-0089` precedent). Selected under the
  owner's standing "keep working the future-growth list" instruction as
  the "`WHERE id = …`" SQL-parity item, read as a client-side
  compilation rather than the wire change the item assumed. Ships in
  `ADR-0089`'s PR (the branch is one), so it waits on the owner's
  review with it.
- Date: 2026-09-21
- Deciders: baileyrd
- Related: `docs/design/SERVER-SQL-ID-PREDICATE-DESIGN.md` (the full
  design), `ADR-0034` (the client-side SQL compiler), `ADR-0073` (a
  `Predicate` names a field; an id is not one), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive to `src/server/client.rs`;
  the predicate evaluator moves to `protocol.rs` (re-exported from
  `serve.rs`). No server, wire, or Python change.

## Context

`WHERE id = '<uuid>'` was `UnknownField("id")` at compile time because
a `Predicate` carries a field tag and the id is not a field. The
request the clause means — `GetById` — has been on the wire since
version 1; the item needed a compiler, not a protocol.

## Decision

Implement: `split_id_conditions` pulls the one `id = '<uuid>'` out of
the `WHERE` (only when the schema has no field called `id`; `Eq` and a
UUID string literal only, at most once, else a client-side `Sql`
error); `query_rows` answers it with one `GetById`, re-checks the other
predicates with the server's own `predicate_matches` (moved to
`protocol.rs` so the `client` feature can share it), applies the
`SELECT` list, and honours `LIMIT 0`. `ORDER BY`/`GROUP BY`/`JOIN` with
`id` keep their refusal. Proven over a socket on `Memory`.

The fork, for the owner:

- **(a) As implemented.** Client-side, no wire change.
- **(b) An `id` pseudo-field in `Predicate`** — a wire round.
- **(c) Decline and revert.**

## Consequences

- Positive (a): the most common point lookup in SQL form, one round
  trip, no new bytes on the wire; the evaluator now has one home.
- Named, not hidden: `id IN (…)` and the Python client's tuple form are
  not covered; a point read is not a planned read and does not show in
  `dogserver_query_plans_total`.

## Acceptance and implementation

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.75.0 /
  `FR-087`; see the design's change history. Builder: Claude, under the
  host-takeover convention; independent Codex inspection owed.
- 2026-09-21: implemented, same branch, no deviation. `cargo fmt -p
  rusty_multimodal_db -- --check` clean; `cargo clippy -p
  rusty_multimodal_db --features server,research --all-targets -- -D
  warnings` clean; `cargo test -p rusty_multimodal_db --features
  server,research` — `server_sql_integration` 63 (up from 62), every
  other target unchanged and green, 938 tests across 39 targets, 0
  failed. Not measured: a point read is `GetById`'s existing cost.
