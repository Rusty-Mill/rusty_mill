# ADR-0050: More than one table on one connection, built — `serve_tables`, `Use`/`ListTables` at protocol 16, `Memory → Entity` via a foreign `mentions` label

- Status: **Accepted as designed** (promoted from Proposed on
  2026-09-07 — the owner's "accept as designed", option (a): Part B as
  accepted plus foreign labels on `MultiSymmetric`, `Memory → Entity`
  the instance; (b) a new directed cross-table layer, (c) mirroring the
  link on the entity side, (d) `Order → Customer` first, and (e)
  decline all declined. Recorded in "Acceptance and implementation"
  below.) Proposed and implemented on one branch, the `ADR-0046`–
  `ADR-0049` cadence. Implements `ADR-0045`'s accepted direction.
- Date: 2026-09-07
- Deciders: baileyrd
- Related: `docs/design/SERVER-TABLES-DESIGN.md` (this round's design),
  `ADR-0045`/`docs/design/SERVER-SQL-JOIN-DESIGN.md` Part B (the
  accepted shape and gate), `ADR-0044` (`right_table`/`target_table`
  since protocol 12), `ADR-0048` (the second table), `ADR-0047` (the
  edge machinery reused).
- Supersedes/Superseded by: none. Closes `ADR-0045`'s gate.

## Context

`ADR-0045` accepted the multi-table shape and gated it on a second
table someone needs. `ADR-0048` built `Memory`, and the consumer's
`memory_entities` link — memory → entity, read from both ends, joined
across the two tables in every query that touches it — is the
instance. Part B had every piece of the wire and server shape but one:
a relation whose far endpoint is another table's record. Both relation
layers check both endpoints against their own store.

## Decision

Build Part B as accepted — `serve_tables`, `ConnectionStore::
table_name`, per-connection `Request::Use` (24), `Request::ListTables`
(25) → `Response::Tables` (17), protocol 16, cross-table `Join` with
the right table's schema and adapter, `Use` inside a session refused —
and add the one missing piece: **foreign labels** on `MultiSymmetric`
(`with_foreign_labels`), for which `link` skips only the far-end
existence check while the server checks that end against the table the
relation's descriptor names. `Memory`'s stack gains its one relation,
`mentions`, foreign to `entity`; `MemoryConnectionStore` serves it; the
`memory_server` binary serves both tables. Clients gain `list_tables`,
`use_table`, and cross-table SQL `JOIN`.

## Consequences

- Positive: the consumer's `memory_entities` path — link, both lookups,
  the cross-table join — has a backend in one round trip each, with no
  new file and no new edge format.
- Positive: `serve` and every pre-16 connection are unchanged byte for
  byte; the gate `ADR-0045` set is closed by the instance it named.
- Named, not hidden: the far-end check is the server's, not the
  layer's — a library caller linking a foreign id directly gets no
  check. Documented on the layer and the adapter.
- Named, not hidden: `mentions` has no `created_at`; edge metadata is
  still open; there is still no deletion to dangle an edge.
- `Order → Customer`, Part B's research instance, is not built.

## Considered options

**(a) Accept as designed** — Part B as accepted plus foreign labels.
**(b) A new directed cross-table layer** with its own blob and reverse
index. **(c) Mirror the link on the entity side** — two copies with no
transaction across two stores. **(d) `Order → Customer` first.**
**(e) Decline.**

## Acceptance and implementation

- 2026-09-07: proposed and implemented on the same branch as
  `SERVER-001` v0.40.0 / FR-050, `SERVER-002` v0.5.0 —
  `src/generic/{store,memory}.rs`, `src/server/{protocol,serve,audit,
  memory,dog,order,employee,reminder,entity,client}.rs`,
  `src/bin/memory_server.rs`, `clients/python/**`, three vectors (56
  pinned); tests: `store.rs` +1, `generic/memory.rs` +1, `server/
  memory.rs` +1 (rewritten), `server_memory_integration` +2 (one
  rewritten; the two-table suite with a restart), `server_protocol_
  version` +1, `server_transaction_integration`/`server_python_client`
  extended; every acceptance criterion 1–5 holds. (PR #202.)
- 2026-09-07: accepted as designed (option (a); (b)–(e) declined). No
  change to the implementation. `ADR-0045`'s gate is closed. Next in
  the line: runtime deletion, the last of `ADR-0036`'s three clauses.
