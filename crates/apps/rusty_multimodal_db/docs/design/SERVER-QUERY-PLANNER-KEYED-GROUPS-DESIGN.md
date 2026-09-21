# Server Query Planner, Step Nine: `GROUP BY` the Walked Key (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0084`; the `ADR-0059`/`ADR-0076`–`ADR-0083` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as `ADR-0082`'s
  own option (b); the fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0082`/`docs/design/SERVER-QUERY-PLANNER-KEYED-WALK-DESIGN.md`
  (whose option (b) and second Non-goal this is — "`GROUP BY` the range
  field — buckets over the walked keys are possible (`GROUP BY due_at`
  is the run-length of equal keys) but a new shape; named, not taken"),
  `ADR-0035` (`Aggregate`'s keyed buckets: a keyed bucket that matches
  zero rows never appears; `limit` truncates the group count),
  `ADR-0074` (`Aggregate`'s group order "unspecified" — the contract
  this round's one restriction is read against), `ADR-0083` (the
  tightest bounds the walk runs between), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive to `src/server/serve.rs`:
  `keyed_walk_applies` admits a `group_by` of exactly the range field;
  `keyed_walk` yields one group per run of equal keys. No generic-layer,
  adapter, wire, protocol-version, or client change; `evaluate_aggregate`
  untouched. Every group is the decode path's (`QKG-FR-004`).

## Purpose and scope

`ADR-0082` reduced `SUM`/`AVG`/`MIN`/`MAX` of the range field over the
walked keys and named the next shape: `GROUP BY` that same field. The
walk yields the keys *in order*, so a group per distinct key is a run of
equal adjacent keys — no bucket lookup, no decode, one pass. Against
the decode path, which buckets every decoded row by a linear search
over the groups found so far (`evaluate_aggregate`'s `find`), the walk
is also asymptotically better: the decode path is quadratic in the
number of groups (`due-hist-5k`, 5,000 groups, 38,647.5 µs before this
round against `due-hist`'s 2,201.8 for 1,000).

This round: a `Request::Aggregate` whose `group_by` is exactly the
range field, whose filter is nothing but bounds on it (or empty), and
whose every aggregate is `COUNT(*)` or a reduction of that field,
answers from `range_keys` as one group per run — the key in the field's
kind, each column reduced over the run exactly as `evaluate_aggregate`
reduces a bucket's rows.

Scope, exactly: eligibility (`QKG-FR-001`); the groups (`QKG-FR-002`);
the one shape held back (`QKG-FR-003`); identity, proven (`QKG-FR-004`);
no other surface changed (`QKG-FR-005`).

## Non-goals

- **`GROUP BY` any other field, or the key beside another** — the
  other field's value is not in the index; a decode.
- **A bucketed `GROUP BY`** (`GROUP BY due_at / 86_400_000`, "per
  day") — no expression exists on the wire; a protocol round.
- **Fixing `evaluate_aggregate`'s quadratic bucket search** — a real,
  separate improvement to the decode path (a `HashMap` over the key);
  named, not taken here, since it is not a planner question.
- **Any change to the wire, `plan_query`, or `counted_walk`.**

## Context and terminology

Read from `main` after PR #283 (`SERVER-001` v0.68.0) this pass:

- **`keyed_walk_applies`** required `group_by` empty. It now accepts
  `group_by == [range_field]` too; anything else is ineligible.
- **`keyed_walk`** built one group over every walked key. Grouped, it
  splits the keys by runs of equality (`slice::chunk_by`) and builds one
  group per run: `key = [(field, key in the schema's kind)]`, `values`
  each column over the run (`COUNT` the run's length; `SUM` `key × n`
  as the `i64` sum; `AVG` `F64(key)`; `MIN`/`MAX` the key). No run over
  nothing: an empty walk yields no groups, as a keyed bucket that
  matches zero rows never appears (`ADR-0035`).
- **Group order and `limit`.** The decode path's groups appear in the
  order their first row was read. Under a bound the candidates are the
  same walk (`IndexRange`), so its groups ascend by key exactly as the
  runs do — sequence-identical. With no bound the decode path scans in
  storage order, so its groups are the same *set* in another order —
  within `ADR-0074`'s "unspecified order" contract — but a `limit`
  there would truncate a different set. That one shape (grouped, no
  bound, a `limit`) takes the decode path (`QKG-FR-003`), so every
  answer this round gives is one the decode path gives.
- **Count-only, grouped**: the keys, not `counted_walk` (a count has no
  runs).

## Requirements

- `QKG-FR-001` **Eligibility.** `keyed_walk_applies` with `group_by`
  empty or exactly `[range_field]`; the rest as `QKW-FR-003`
  (`ADR-0082`) with `ADR-0083`'s bounds.
- `QKG-FR-002` **The groups.** As "Context": one `AggregateGroup` per
  run of equal keys, ascending, keyed in the field's kind, each column
  reduced over the run as `evaluate_aggregate` reduces rows; none over
  an empty walk; `limit`-truncated.
- `QKG-FR-003` **The one shape held back.** Grouped, no bound, and a
  `limit`: the decode path.
- `QKG-FR-004` **Identity, proven.** Under a bound every grouped answer
  equals the decode path's sequence; with no bound, its set — proven on
  `PlannerFixture` over a six-row table with repeated keys (zero
  decodes, one keys walk; the decode path's identical groups over a
  range, a two-sided range, a `limit`, and nothing; the runs' values
  pinned in the `U32` kind; the same set over everything; the held-back
  shape decoding; count-only grouped taking the keys), by the
  eligibility matrix, and over a real socket via SQL on `Memory` (two
  memories re-keyed onto one stamp so a run exists: no filter, one- and
  two-sided, over nothing, an equality; a second field in the filter
  and the key grouped beside another field still exact through the
  decode path; `LIMIT` with and without a bound) and `Reminder` (whose
  fixture already repeats a due stamp). Every pre-existing test
  unmodified.
- `QKG-FR-005` **Everything else unchanged.** `evaluate_aggregate`,
  `plan_query`, `counted_walk`, the wire (`PROTOCOL_VERSION` 27), the
  clients, `sql.rs`: untouched.

## Considered options

- **(a) `GROUP BY` the range field as runs — implemented.** One
  eligibility line, one `chunk_by`.
- **(b) (a) plus a `HashMap` bucket in `evaluate_aggregate`** — the
  decode path's own quadratic search fixed for every `GROUP BY`; a
  separate round, not a planner one.
- **(c) Decline.** `GROUP BY due_at` keeps decoding and searching.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus the
decode path's bucket fixed; **(c)** decline and revert.

## Proposed shape

`src/server/serve.rs`: `keyed_walk_applies`, `keyed_walk`, the
`Aggregate` arm's comment. `benches/server.rs`: `reminder-due`
`due-hist` (1,000 groups) and `due-hist-5k` (5,000 groups) rows.

## Data/state and invariants

- **Run invariant**: the walk yields keys ascending, so every record
  with one key is one contiguous run; a run's length is that key's
  count and its reductions are the key's.
- **Cost**: one pair visit and one `i64` per record in range, one pass
  over the keys; the response is one group per distinct key, as before.

## Errors, failure, recovery, and observability

No new `ErrorCode`; a refusal degrades to the decode path. The path
taken is not observable on the wire.

## Security, privacy, and compatibility

A read, gated as `Aggregate` already is; the same groups. No wire
change.

## Acceptance criteria

1. `keyed_walk_applies`: the range field alone as `group_by`, with and
   without bounds, beside each reduction; not another field, not the
   key beside another.
2. `dispatch` on `PlannerFixture` with repeated keys: `QKG-FR-004`'s
   shapes.
3. Integration (`tests/server_sql_integration.rs`): `QKG-FR-004`'s
   shapes on `Memory` and `Reminder`.
4. Measured (`RESULTS.md`): `due-hist` and `due-hist-5k` before and
   after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-KEYED-GROUPS`.
- Decision: `ADR-0084`.
- Specification: `SERVER-001` v0.69.0 / `FR-081` (extends `FR-079`).
- Requirements: `QKG-FR-001`–`005`.

## Open questions

- **The decode path's bucket** — option (b); `evaluate_aggregate`'s
  linear `find` is quadratic in groups for every `GROUP BY`, not only
  this one. *Taken: `ADR-0085` (2026-09-21) —
  `docs/design/SERVER-AGGREGATE-HASHED-BUCKET-DESIGN.md`.*
- **A fold that never materializes the keys** — unchanged from
  `ADR-0082`.
- **Plan metrics** — still named, still not bundled. *Taken: `ADR-0086`
  (2026-09-21) — `docs/design/SERVER-QUERY-PLAN-METRICS-DESIGN.md`.*

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as
  `ADR-0082`'s own option (b). Read from `main` after PR #283 this pass.
  The `due-hist`/`due-hist-5k` bench rows were added and measured on
  the pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.69.0 /
  `FR-081`, exactly the "Proposed shape". Acceptance criteria 1–3 are
  the tests: `serve.rs` +2
  (`keyed_walk_applies_to_a_group_by_of_the_range_field`,
  `dispatch_groups_the_walked_keys_by_run_without_reading_a_record`;
  `PlannerFixture` gained a rows override), `tests/server_sql_integration.rs`
  +1 (`a_group_by_of_the_range_field_from_the_walked_keys_matches_the_decoded_groups`)
  (lib 637, up from 635; SQL 59, up from 58); 923 tests across
  39 targets, 0 failed, every pre-existing test unmodified. Criterion 4,
  `RESULTS.md`: `due-hist` 2,491.3 → 299.8 µs; `due-hist-5k`
  38,638.8 → 1,437.1. Still no independent review — owed.
