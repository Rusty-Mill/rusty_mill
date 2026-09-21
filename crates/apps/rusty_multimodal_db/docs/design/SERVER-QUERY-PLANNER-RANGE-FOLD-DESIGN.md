# Server Query Planner, Step Ten: A Fold That Materializes No Key (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0087`; the `ADR-0059`/`ADR-0076`–`ADR-0086` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the "fold that
  never materializes the keys" open question `ADR-0082` named from its
  own measurement and every round since carried; the fork below stays
  open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0082`/`docs/design/SERVER-QUERY-PLANNER-KEYED-WALK-DESIGN.md`
  (whose open question this is — "`range_keys` fills a `Vec<i64>` (8
  bytes per key); the `due-next` row's ~600 µs against `due-count`'s
  ~290 µs is that `Vec`. A generic `range_fold` would close it"),
  `ADR-0081` (`range_count`, the precedent for a walk that keeps
  nothing), `ADR-0084` (the grouped walk, which still needs the keys),
  `ADR-0075`/`ADR-0079` (`RangeBy`, extended by one method),
  `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `RangeBy::range_fold`
  (generic, `Ordered` the one implementation, exposed on
  `GenericProductionStore`), `server::KeyStats` and a defaulted
  `ConnectionStore::range_stats` (`Memory`/`Relation`/`Reminder`);
  `keyed_walk`'s ungrouped shape takes the fold. No wire,
  protocol-version, `FieldCapabilities`, client, or file-format change.
  Every answer is unchanged (`QRF-FR-004`).

## Purpose and scope

`ADR-0082` answered `SUM`/`AVG`/`MIN`/`MAX` of the range field from the
walked keys and measured what was left: a `Vec<i64>` of every key in
range, filled and then reduced. Every one of those reductions is a
fold — a count, a sum, a least, a greatest — so the keys need never
exist. `range_count` (`ADR-0081`) already walks without keeping
anything; this round gives the walk a fold, and the ungrouped keyed
aggregate a one-pass `KeyStats` instead of a `Vec`.

Scope, exactly: the generic fold (`QRF-FR-001`); the adapter method
(`QRF-FR-002`); the ungrouped walk over it (`QRF-FR-003`); identity,
proven (`QRF-FR-004`); no other surface changed (`QRF-FR-005`).

## Non-goals

- **The grouped walk** (`ADR-0084`): a group per run of equal keys
  needs the keys, or a fold that emits a group at each key change — a
  real refinement, not taken here; `GROUP BY` the key still materializes.
- **Any change to eligibility, `plan_query`, or the wire.**

## Context and terminology

Read from `main` after PR #286 (`SERVER-001` v0.71.0) this pass:

- **`RangeBy::range_fold(lower, upper, init, f) -> B`**: `f` applied to
  `init` and each key within the bounds, ascending, over the same
  `guarded_pairs` iterator `range_keys` and `range_count` walk; nothing
  collected.
- **`KeyStats { count, sum, min, max }`** (`serve.rs`): the one-pass
  reduction; `min`/`max` `None` over an empty range; `with(&key)` the
  fold step. `sum` is the same `i64` `+` as `evaluate_aggregate`'s
  `Sum`.
- **`ConnectionStore::range_stats(field, lower, upper) ->
  Result<KeyStats, ErrorCode>`**: `range_keys`'s bounds and errors,
  folded; defaulted `Unsupported`.
- **`keyed_walk`**: every column now reduces from a `KeyStats` —
  grouped, one per run of the still-materialized keys; ungrouped, the
  adapter's one `range_stats`. The reductions are exactly `ADR-0082`'s
  (`COUNT` the count, `SUM` the sum, `AVG` `sum / count` or `0.0`,
  `MIN`/`MAX` the extremes in the field's kind or `0`).

## Requirements

- `QRF-FR-001` **The generic fold.** As "Context"; exposed on
  `GenericProductionStore` under the read lock.
- `QRF-FR-002` **The adapter method.** As "Context"; `Memory`,
  `Relation`, `Reminder` answer it through `uuid_pair_bounds` and
  `range_fold`.
- `QRF-FR-003` **The ungrouped walk.** `keyed_walk` with no `group_by`
  and at least one non-`COUNT` column takes one `range_stats`; the
  grouped walk and the count-only walk are unchanged.
- `QRF-FR-004` **Identity, proven.** Every answer equals `ADR-0082`'s
  (and so the decode path's): by construction (`KeyStats` over the keys
  is the same reduction as the reductions over the `Vec`), and proven on
  `PlannerFixture` (one fold, zero keys materialized, zero decodes; the
  decode path's identical groups over a range, everything, and nothing;
  the values and the empties pinned in the `U32` kind; the grouped walk
  still taking the keys), on the generic `Ordered` (`range_fold` visits
  exactly `range_keys`' keys, bounds and inversion included; a count
  and a sum in one pass), and over a real socket via SQL on `Memory` and
  `Reminder` (the five columns, no filter, one- and two-sided, over
  nothing). Every pre-existing test unmodified.
- `QRF-FR-005` **Everything else unchanged.** Eligibility,
  `plan_query`, `counted_walk`, the grouped walk's keys, the wire
  (`PROTOCOL_VERSION` 27), the clients: untouched.

## Considered options

- **(a) A generic fold and a `KeyStats` — implemented.** One generic
  method, one adapter method, one struct.
- **(b) (a) plus a run-emitting fold for the grouped walk** — a fold
  that yields a group at each key change, so `GROUP BY` the key
  materializes nothing either; a second round.
- **(c) Decline.** The `Vec` stays.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus the
grouped fold; **(c)** decline and revert.

## Proposed shape

`src/generic/query.rs`: `RangeBy::range_fold`. `src/generic/store.rs`:
`Ordered`'s impl over `guarded_pairs`. `src/generic/production.rs`:
`range_fold`. `src/server/serve.rs`: `KeyStats`,
`ConnectionStore::range_stats` (defaulted), `keyed_walk`.
`src/server/{memory,relation,reminder}.rs`: `range_stats`.

## Data/state and invariants

- **Fold invariant**: `range_fold(l, u, init, f) == range_keys(l,
  u).iter().fold(init, f)` for every `l`, `u`, `init`, `f` — the same
  iterator, folded instead of collected.
- **Cost**: one pair visit and one `KeyStats::with` per key; no
  allocation proportional to the range.

## Errors, failure, recovery, and observability

No new `ErrorCode`; a refusal degrades to the decode path. Not
observable on the wire (`plan_of` classifies it as `keyed_walk`, as
before).

## Security, privacy, and compatibility

The same numbers; no wire change.

## Acceptance criteria

1. `KeyStats::with` and `dispatch` on `PlannerFixture`: `QRF-FR-004`'s
   shapes.
2. `Ordered::range_fold` visits `range_keys`' keys.
3. Integration (`tests/server_sql_integration.rs`): the five columns on
   `Memory` and `Reminder`.
4. Measured (`RESULTS.md`): `reminder-due` `due-next` and `due-mean`
   before and after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-RANGE-FOLD`.
- Decision: `ADR-0087`.
- Specification: `SERVER-001` v0.72.0 / `FR-084` (extends `FR-079`).
- Requirements: `QRF-FR-001`–`005`.

## Open questions

- **A run-emitting fold for the grouped walk** — option (b).
- **A "walk abandoned" counter** — unchanged from `ADR-0086`.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as the
  open question `ADR-0082` named from its own measurement. Read from
  `main` after PR #286 this pass. The existing `due-next`/`due-mean`
  rows were measured on the pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.72.0 /
  `FR-084`, exactly the "Proposed shape". Acceptance criteria 1–3 are
  the tests: `serve.rs` +1
  (`dispatch_reduces_the_range_field_from_one_fold_without_materializing_keys`;
  `PlannerFixture` gained `range_stats` and two counters),
  `src/generic/memory.rs` +1 (`range_fold_visits_the_range_s_keys_in_order`),
  `tests/server_sql_integration.rs` +1
  (`ungrouped_reductions_from_one_fold_match_the_decoded_answers`)
  (lib 642, up from 640; SQL 61, up from 60); 931 tests across
  39 targets, 0 failed, every pre-existing test unmodified. Criterion 4,
  `RESULTS.md`: `due-next` 610.3 → 581.6 µs; `due-mean`
  705.9 → 574.0. Still no independent review — owed.
