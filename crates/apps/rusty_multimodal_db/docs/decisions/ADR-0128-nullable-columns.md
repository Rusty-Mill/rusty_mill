# ADR-0128: Nullable Columns as a Wire View Over Sentinels (Protocol 32)

- Status: **Proposed and implemented on one branch; the owner chose it**
  (2026-09-30, "nullable column"). **Wire change: protocol 31 → 32**,
  `SERVER-002` 0.21.0.
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0117` (`ScanValue::Null` at 31, no producer; named "the
  first nullable column's round" as the follow-up), `ADR-0056` (the
  sentinels: `Memory`'s `deleted_at_unix_ms` is `0`, `node_id` is `""`),
  `ADR-0041` (a value variant stripped below its version), `ADR-0022` (the
  compatibility rules), `ADR-0126`/`0127` (the rounds before).
- Supersedes/Superseded by: completes `ADR-0117`'s deferred flag; keeps
  `ADR-0056`'s storage. Additive: `nullable::{NullableField, to_storage,
  to_wire}`, `ConnectionStore::nullable_fields`, `Request::DescribeNullable`
  (38), `Response::NullableFields` (25), SQL `IS [NOT] NULL`,
  `ConnectOptions::max_protocol_version`.

## Context

`ADR-0117` put `Null` on the wire and deferred the field-level part: "a
flag no field sets would be a lie in every `DescribeSchema`". Two columns
now want it. Both are lossless as sentinels (`ADR-0056`), and the stored
layout (`memory::Memory@2`) must not change: that is a migration, and a
sentinel carries no information a `NULL` would not.

## Decision

**Nullable is a wire view, not a storage change.** An adapter names its
nullable fields and their stored sentinel (`nullable_fields`); `Memory`
names `deleted_at_unix_ms` (`0`) and `node_id` (`""`). `handle_connection`
translates at the edge, only for a connection negotiated at 32 or above:

- `to_storage`: a `Null` a request carries for such a field is the
  sentinel, in every value position (a write, `UpdateField`, `Transaction`,
  `FilterEq`, a predicate or guard). Only `Eq`/`Ne` mean "is (not) null";
  a bound against `Null` is `Malformed`, as `ADR-0117` said (ordering never
  holds). `Null` for a field that is not nullable is refused as before.
- `to_wire`: a stored sentinel in `Record`, `Rows`, `RowsClamped`,
  `JoinedRows` (the right side only when it is this table's), `ScanValues`
  of that field, or a `Groups` key of that field is `Null`.

Everything between the two — validation, the planner, sessions, MVCC, the
journal — sees the sentinels it always saw. Below 32 the wire is unchanged:
the client still sees `0` and `""`. `Request::DescribeNullable` lists the
tags (a new request rather than a `FieldCapabilities` flag, because that
struct is encoded positionally and a new field would break every client's
`Schema` decode; new schema information has arrived as a new request since
`DescribeRelations`). SQL: `field IS [NOT] NULL` compiles to `Eq`/`Ne`
against `Null`; `= NULL` is not accepted (never true in SQL).

## Consequences

- Positive: no migration, no index or journal change, no cost for a table
  with no nullable field; a real `NULL` for the two columns whose sentinel
  was always a stand-in.
- Negative / tradeoffs: a client at 32 sees `Null` where it saw a sentinel,
  so upgrading the client is a behaviour change for those two fields (the
  reference client's `max_protocol_version(31)` keeps the old view);
  aggregates read the sentinel (`SUM(deleted_at)` adds the zeros, `MIN` is
  `0` if any row is null); a real `0` in `deleted_at` cannot be stored
  (`ADR-0056` already said none is).
- Named, not hidden: `node_id`'s and `deleted_at`'s existing tests moved to
  a connection at 31 (they document the sentinels); the 32 behaviour has
  its own test file. A column with no lossless sentinel would need a real
  nullable storage layout: a different round.

## Acceptance and implementation

- 2026-09-30: `SERVER-001` v0.104.0 / `FR-117`, `SERVER-002` 0.21.0.
  `tests/server_nullable_integration.rs` (Null at 32 and the sentinel at
  31 for the same record; `Null` written and `IS NULL`/`IS NOT NULL`
  partitioning; `GROUP BY` keyed `Null`; only equality takes `Null`; the
  request `Malformed` below 32), unit tests for the translation and the SQL
  parse, two new golden vectors and the regenerated fixture, the Python
  client and its vectors. fmt, clippy (`--features server,research -D
  warnings`) and the crate's tests clean. Builder: Claude; independent
  inspection owed.
