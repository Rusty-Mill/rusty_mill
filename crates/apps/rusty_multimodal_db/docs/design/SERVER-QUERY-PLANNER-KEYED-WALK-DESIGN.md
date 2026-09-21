# Server Query Planner, Step Seven: Aggregates Over the Walked Key (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0082`; the `ADR-0059`/`ADR-0076`–`ADR-0081` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as `ADR-0081`'s
  own option (b); the fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0081`/`docs/design/SERVER-QUERY-PLANNER-COUNTED-WALK-DESIGN.md`
  (whose option (b) and first Non-goal this is — "`MIN`/`MAX` of the
  range field itself is the walk's first/last key — a real, unbuilt
  slice"), `ADR-0035` (`Aggregate`'s five functions and their empty-set
  answers, reproduced exactly here), `ADR-0075`/`ADR-0079`/`ADR-0081`
  (`RangeBy`, extended by one method; `guarded_range`, now over a
  shared `guarded_pairs`), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `RangeBy::range_keys`
  (generic, `Ordered` the one implementation, exposed on
  `GenericProductionStore`), a defaulted `ConnectionStore::range_keys`
  (`Memory`/`Relation`/`Reminder`), `keyed_walk_applies`/`keyed_walk`
  wrapping `ADR-0081`'s `counted_walk`, the `Aggregate` arm asking the
  wider one. No wire, protocol-version, `FieldCapabilities`, client, or
  file-format change; `evaluate_aggregate` untouched. Every answer is
  the decode path's (`QKW-FR-005`).

## Purpose and scope

`ADR-0081` counted a pure range from the sorted index and named the
next slice in the same breath: `MIN`/`MAX` of the range field are the
walk's first and last key. Read against the index, more follows for
free — the walk yields `(key, id)` pairs, so every aggregate whose
input is the key itself (`SUM`, `AVG`, `MIN`, `MAX` of the range field)
is a reduction over the keys the walk already visits, with no record
read. The consumer's "next due" (`MIN(due_at) WHERE due_at > now`) is
one such.

This round: a `Request::Aggregate` with no `GROUP BY`, a filter of at
most one bound per side on the range field (or empty), and every
aggregate either `COUNT(*)` or `SUM`/`AVG`/`MIN`/`MAX` *of the range
field* answers from `range_keys` — reduced exactly as
`evaluate_aggregate` reduces decoded rows, empties included — and a
count-only request still takes `ADR-0081`'s `range_count` (no keys
materialized).

Scope, exactly: the generic keys (`QKW-FR-001`); the adapter method
(`QKW-FR-002`); eligibility (`QKW-FR-003`); the reductions
(`QKW-FR-004`); identity, proven (`QKW-FR-005`); no other surface
changed (`QKW-FR-006`).

## Non-goals

- **An aggregate over any other field** — its value is not in the
  index; a decode. `MAX(created_at) WHERE updated_at > since` decodes.
- **`GROUP BY` the range field** — buckets over the walked keys are
  possible (`GROUP BY due_at` is the run-length of equal keys) but a
  new shape; named, not taken.
- **Bound tightening**, two bounds on a side, `Ne`: the decode path, as
  under `ADR-0081`. *Two bounds on a side taken: `ADR-0083`
  (2026-09-21) — the walk runs between the tightest two; `Ne` still
  decodes.*
- **Any change to `evaluate_aggregate`, `plan_query`, or the wire.**

## Context and terminology

Read from `main` after PR #281 (`SERVER-001` v0.66.0) this pass:

- **`Ordered::guarded_range`** yielded ids; it now maps over a
  `guarded_pairs` iterator that both it and `range_keys` share — the
  same panic guard, the same walk.
- **`evaluate_aggregate`'s reductions** (`ADR-0035`): `COUNT` the row
  count; `SUM` the `i64` sum of `numeric_i64`; `AVG` an `F64` mean,
  `0.0` over no rows; `MIN`/`MAX` the extreme row value in the field's
  own kind, `0` (as `U32(0)` or `I64(0)` by the schema's kind) over no
  rows. `keyed_walk` reproduces each over the keys.
- **`keyed_walk_applies`**: `counted_walk_applies`'s shape with each
  aggregate mapped to `COUNT(*)` if it is `COUNT(*)` or one of the four
  reductions of the range field; anything else is ineligible.
- **`keyed_walk`**: delegates to `counted_walk` when every column is
  `COUNT(*)`; otherwise one `range_keys`, every column reduced over it.

## Requirements

- `QKW-FR-001` **The generic keys.** `RangeBy::range_keys(lower, upper)
  -> Vec<Key>` on `Ordered`: the keys within the bounds, ascending, one
  visit per pair, no record read; empty over an inverted or empty
  range. Exposed on `GenericProductionStore` under the read lock.
- `QKW-FR-002` **The adapter method.** `ConnectionStore::range_keys(
  field, lower, upper) -> Result<Vec<i64>, ErrorCode>` (every shipped
  range field is `I64`), `range_ids`'s errors, defaulted `Unsupported`;
  `Memory`, `Relation`, `Reminder` answer it through `uuid_pair_bounds`.
- `QKW-FR-003` **Eligibility.** As "Context".
- `QKW-FR-004` **The reductions.** `COUNT` → `I64(len)`; `SUM` →
  `I64(sum)`; `AVG` → `F64(sum / len)`, `F64(0.0)` over no keys;
  `MIN`/`MAX` → the first/last key in the field's schema kind (`U32` or
  `I64`), `0` in that kind over no keys. One group, an empty key, the
  values in column order, `limit`-truncated as `evaluate_aggregate`.
- `QKW-FR-005` **Identity, proven.** Every eligible answer equals the
  decode path's, proven on `PlannerFixture` (zero decodes, one keys
  walk; the decode path's identical groups over a range, everything,
  and nothing; the five columns pinned in the `U32` fixture's kind; the
  empties pinned; a count-only request taking `counted_walk`; an
  aggregate over another field decoding), on the generic `Ordered`
  (`range_keys` ascending, bounded, inverted), and over a real socket
  via SQL on `Memory` and `Reminder` (`SUM`/`AVG`/`MIN`/`MAX` of the
  range field with no filter, one- and two-sided, over nothing; the
  ineligible shapes — a second predicate, an aggregate over another
  field — still exact; the "next due"; a re-keying `Replace` moving the
  extreme). Every pre-existing test unmodified.
- `QKW-FR-006` **Everything else unchanged.** `evaluate_aggregate`,
  `plan_query`, `counted_walk`, the wire (`PROTOCOL_VERSION` 27), the
  clients, `sql.rs`: untouched.

## Considered options

- **(a) The four reductions of the range field — implemented.** One
  generic method, one adapter method, two free functions.
- **(b) (a) plus `GROUP BY` the range field** — buckets by equal key
  over the walked keys; a second round with its own answer shape.
- **(c) Decline.** `MIN(due_at)` keeps decoding every candidate.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus
`GROUP BY` the range field; **(c)** decline and revert.

## Proposed shape

`src/generic/query.rs`: `RangeBy::range_keys`. `src/generic/store.rs`:
`guarded_pairs` (private), `guarded_range` and `range_keys` over it.
`src/generic/production.rs`: `range_keys`. `src/server/serve.rs`:
`ConnectionStore::range_keys` (defaulted), `keyed_walk_applies`,
`keyed_walk`, the `Aggregate` arm. `src/server/{memory,relation,reminder}.rs`:
`range_keys`. `benches/server.rs`: `reminder-due` `due-next` and
`due-mean` rows.

## Data/state and invariants

- **Reduction invariant**: the keys the walk yields are exactly the
  range-field values of the records the predicates admit
  (`ADR-0081`'s count invariant, applied to the key rather than the
  count), so each reduction over them equals the same reduction over
  the decoded rows' values.
- **Cost**: one pair visit and one `i64` per record in range; no
  decode. A wide range materializes a `Vec<i64>` of its size — 8 bytes
  per key against ~200 bytes decoded before.
- **Consistency class**: one walk under one read lock, as `ADR-0081`.

## Errors, failure, recovery, and observability

No new `ErrorCode`; a refusal degrades to the decode path. The path
taken is not observable on the wire.

## Security, privacy, and compatibility

A read, gated as `Aggregate` already is; the same numbers. No wire
change.

## Acceptance criteria

1. `keyed_walk_applies` unit tests: the four reductions of the range
   field, alone and beside `COUNT(*)`, over the pure-range shapes; not
   over another field, a `group_by`, a second predicate.
2. `dispatch` on `PlannerFixture`: zero decodes, the decode path's
   identical groups, the five columns and the empties pinned in the
   field's kind, count-only taking `counted_walk`, another field
   decoding.
3. `Ordered::range_keys` ascending, bounded, inverted.
4. Integration (`tests/server_sql_integration.rs`): `QKW-FR-005`'s
   shapes on `Memory` and `Reminder`.
5. Measured (`RESULTS.md`): `reminder-due` `due-next` (`MIN(due_at)
   WHERE due_at > 50000`) and `due-mean` (`AVG(due_at) WHERE due_at <=
   50000`) before and after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-KEYED-WALK`.
- Decision: `ADR-0082`.
- Specification: `SERVER-001` v0.67.0 / `FR-079` (extends `FR-078`).
- Requirements: `QKW-FR-001`–`006`.

## Open questions

- **`GROUP BY` the range field** — option (b). *Taken: `ADR-0084`
  (2026-09-21) — one group per run of equal walked keys —
  `docs/design/SERVER-QUERY-PLANNER-KEYED-GROUPS-DESIGN.md`.*
- **A fold that never materializes the keys** — `range_keys` fills a
  `Vec<i64>` (8 bytes per key); the `due-next` row's ~600 µs against
  `due-count`'s ~290 µs is that `Vec`. A generic `range_fold` would
  close it; taken only if a consumer aggregates a wide range in anger.
  *Taken: `ADR-0087` (2026-09-21) — `RangeBy::range_fold`, `KeyStats`,
  `ConnectionStore::range_stats`; the ungrouped walk materializes
  nothing — `docs/design/SERVER-QUERY-PLANNER-RANGE-FOLD-DESIGN.md`.*
- **An "exact" equality index** — unchanged from `ADR-0081`.
- **Plan metrics** — still named, still not bundled. *Taken: `ADR-0086`
  (2026-09-21) — `docs/design/SERVER-QUERY-PLAN-METRICS-DESIGN.md`.*

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as
  `ADR-0081`'s own option (b) — widened from `MIN`/`MAX` to the four
  reductions once read against the index, since the walk yields the
  keys `SUM`/`AVG` need too. Read from `main` after PR #281 this pass.
  The `due-next`/`due-mean` bench rows were added and measured on the
  pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.67.0 /
  `FR-079`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `serve.rs` +2
  (`keyed_walk_applies_only_to_aggregates_over_the_range_field`,
  `dispatch_reduces_the_range_field_from_the_walked_keys_without_reading_a_record`),
  `src/generic/memory.rs` +1 (`range_keys_are_the_range_s_keys_in_order`),
  `tests/server_sql_integration.rs` +1
  (`aggregates_over_the_range_field_from_the_walked_keys_match_the_decoded_answers`)
  (lib 633, up from 630; SQL 57, up from 56); 917 tests across
  39 targets, 0 failed, every pre-existing test unmodified. Criterion 5,
  `RESULTS.md`: `due-next` 26,800.6 → 624.7 µs; `due-mean`
  30,822.6 → 679.5. Still no independent review — owed.
