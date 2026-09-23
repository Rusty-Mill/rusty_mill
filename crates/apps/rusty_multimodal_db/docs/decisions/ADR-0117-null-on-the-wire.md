# ADR-0117: `ScanValue::Null` at Protocol 31

- Status: **Proposed and implemented on one branch; the owner chose
  it** (2026-09-23, "1-5", item 3 — null on the wire). **Wire change:
  protocol 30 → 31**, `SERVER-002` 0.20.0.
- Date: 2026-09-23
- Deciders: baileyrd
- Related: `ADR-0056` (option N1, the sentinels; N2, this variant,
  named as the general answer), `ADR-0041` (`StrList`, the precedent
  for stripping a value variant below its version), `ADR-0022` (the
  compatibility rules), `ADR-0043` (the Python client and the wire
  vectors).
- Supersedes/Superseded by: none. Additive: `ScanValue::Null` (6).

## Context

The wire had no null. `ADR-0056` chose sentinels for `Memory`'s
`deleted_at` (`0`) and `node_id` (`""`) because both are lossless for
those columns, and deferred a nullable `ScanValue` "until a column
arrives with no lossless sentinel". The owner asked for the variant
now. The concern stated before building: no shipped column is nullable,
so the variant has no producer today; it is built so the first such
column costs a field flag, not a protocol bump and a client round.

## Decision

- `NUL-FR-001` — `ScanValue::Null`, index 6, a unit variant: the index
  alone on the wire. `PROTOCOL_VERSION` 31. A server never emits it
  today and answers `Malformed` wherever a request carries it and a
  value is read — a write, a predicate, a transaction op, a guard —
  through `value_matches_kind`, which no kind matches. Equality
  (`Eq`/`Ne`) compares it as any value; ordering never holds.
- `NUL-FR-002` — `downgrade_for_version` strips a `Null` pair from
  `Record` and `Rows` (and `RowsClamped` through `Rows`) on a connection
  negotiated below 31, exactly as a `StrList` pair below 11: rule 3,
  one more variant. `Schema` is untouched: no `ValueKind` changes, since
  nullability is a property of a field, not a kind, and no field has
  it yet.
- `NUL-FR-003` — the wire vectors gain `Record(Null)` and `Rows(Null)`
  at 31; the Python client gains `Null` (index 6), reads it as `None`,
  sends `None` as `Null`, and declares 31; `SERVER-002` 0.20.0
  documents the variant, the downgrade, and the version.
- Not built, on evidence: a nullable field flag in `FieldCapabilities`
  and a `NULL` literal in the SQL front end. Both wait for the first
  nullable column; a flag no field sets would be a lie in every
  `DescribeSchema`.

## Consequences

- Positive: the first nullable column is a field-level change; no
  client needs a new protocol for it.
- Negative / tradeoffs: a protocol bump with no observable behaviour
  change for any current client — the cost the deferral in `ADR-0056`
  was avoiding; every client pin moves to 31.
- Named, not hidden: the sentinels stay. A client cannot ask for
  `deleted_at` as `Null` yet; that is the nullable-field round.

## Acceptance and implementation

- 2026-09-23: implemented on `claude/pr-276-multimodal-db-growth-4lkjx8` as `SERVER-001` v0.96.0 / `FR-109`, `SERVER-002` 0.20.0, in one PR with `ADR-0115`, `ADR-0116`, `ADR-0118`, `ADR-0119`.
  `cargo fmt -p rusty_multimodal_db -- --check` clean; `cargo clippy -p rusty_multimodal_db --features server,research --all-targets -- -D warnings` clean; `cargo test -p rusty_multimodal_db --features server,research` — «TESTS» tests across «TARGETS» targets, 0 failed; Python «PY» tests OK. Builder: Claude.
