# Server Aggregate: A Hashed Bucket for the Decode Path (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0085`; the `ADR-0059`/`ADR-0076`–`ADR-0084` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as `ADR-0084`'s
  own option (b); the fork below stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0084`/`docs/design/SERVER-QUERY-PLANNER-KEYED-GROUPS-DESIGN.md`
  (whose option (b) and third Non-goal this is — "fixing
  `evaluate_aggregate`'s quadratic bucket search — a real, separate
  improvement to the decode path (a `HashMap` over the key); named, not
  taken here, since it is not a planner question"), `ADR-0035`
  (`Request::Aggregate`, `evaluate_aggregate`'s buckets and first-seen
  group order, `limit` over the group count), `ADR-0041` (`StrList`
  refused as a group key at validation), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Internal to `src/server/serve.rs`:
  `evaluate_aggregate` finds each row's bucket through a hash of its
  key instead of a linear search over the buckets so far; one private
  `BucketKey`. No planner, generic-layer, adapter, wire,
  protocol-version, or client change. Every group is the same, in the
  same order (`AGB-FR-002`).

## Purpose and scope

`ADR-0035` built `Aggregate` as "a full scan then a bucket, with no
optimizer of any kind", and the bucket was the simplest thing: a `Vec`
of `(key, rows)` searched linearly for each row's key. That is
quadratic in the number of groups. `ADR-0084` measured it on the way to
its own walk: 1,000 due-stamp groups cost 2,491.3 µs and 5,000 cost
38,638.8 — 5× the groups, ~16× the time — and took the pure-range
`GROUP BY` off that path entirely. Every other `GROUP BY` (a second
predicate beside the key, any other key, a composite key) still decodes
and still searches.

This round replaces the search with a `HashMap` from the row's key to
its bucket's slot. The buckets themselves, their order (first seen),
their contents, and `limit`'s truncation are untouched; only the lookup
changes. Linear in the rows.

Scope, exactly: the key and the lookup (`AGB-FR-001`); identity, proven
(`AGB-FR-002`); no other surface changed (`AGB-FR-003`).

## Non-goals

- **Any planner change.** Which rows are decoded is `ADR-0074`–
  `ADR-0084`'s business; this round only buckets them faster.
- **Sorting groups**, a `HAVING`, a bucketed key (`due_at / day`): the
  wire has no such thing; protocol rounds.
- **A `Hash`/`Eq` on `ScanValue` itself.** `F64` makes that a
  wire-type decision; the private `BucketKey` keeps it local.

## Context and terminology

Read from `main` after PR #284 (`SERVER-001` v0.69.0) this pass:

- **`evaluate_aggregate`**: filter → bucket → reduce → `limit`. The
  bucket step matched each row's key (`group_by`'s fields in order,
  skipping a field the row lacks) against every bucket's key with
  `ScanValue`'s `==` until it found one. It now hashes the key.
- **`BucketKey`**: a private mirror of `ScanValue` that is `Eq + Hash`
  — `U32`, `I64`, `Bool`, `Str`, `StrList` as themselves; `F64` by its
  bit pattern. A group key never holds `F64` (no `ValueKind::F64`
  exists, so no stored field is one) or `StrList` (`validate_aggregate`
  refuses it), so for every key that can occur `BucketKey` equality is
  `ScanValue` equality exactly. The two impossible variants keep the
  conversion total — no `unreachable!`, no `Eq` on `f64`.
- **The whole-table bucket** (`group_by` empty) takes the same one
  bucket as before with no hash at all.

## Requirements

- `AGB-FR-001` **The key and the lookup.** Each filtered row's key is
  hashed as `Vec<(FieldRef, BucketKey)>` into a slot index; a new key
  opens a new bucket at the end. First-seen order, as before.
- `AGB-FR-002` **Identity, proven.** The groups, their order, their
  values, and `limit`'s truncation are exactly the linear search's —
  by construction (equality is the same relation on every key that can
  occur; insertion is at the end as before) and proven in-process
  (rows arriving out of key order, a composite `Str`+`U32` key, a row
  lacking the group field keying on what it has, `limit` in first-seen
  order, every kind against `ScanValue`'s own `==`) and over a real
  socket via SQL on `Reminder` (the key beside a second predicate — the
  shape `ADR-0084` cannot walk — a composite key over a scan, `LIMIT`
  in first-seen order, the whole-table bucket over rows and over
  nothing). Every pre-existing `evaluate_aggregate` test unmodified.
- `AGB-FR-003` **Everything else unchanged.** The planner, the walks,
  the wire (`PROTOCOL_VERSION` 27), the clients, `sql.rs`: untouched.

## Considered options

- **(a) A `HashMap` slot index beside the buckets — implemented.** One
  private enum, one map.
- **(b) `Hash + Eq` on `ScanValue` itself** — a wire-type decision over
  `F64`'s semantics; declined here.
- **(c) Decline.** Every decoded `GROUP BY` stays quadratic in groups.

The owner's shorthand: **(a)** as implemented; **(b)** derive on the
wire type instead; **(c)** decline and revert.

## Proposed shape

`src/server/serve.rs`: `BucketKey`, `evaluate_aggregate`'s bucket
step. `benches/server.rs`: a `reminder-due` `due-hist-5k-pending` row
— `ADR-0084`'s 5,000-stamp histogram with `status = pending` beside it,
so the walk cannot answer it and the decode path buckets 1,250 groups.

## Data/state and invariants

- **Bucket invariant**: `slots[key] == i` iff `buckets[i].0 == key`;
  every row lands in exactly one bucket, the one its key opened.
- **Cost**: one hash and one lookup per row; one `BucketKey` clone of
  the key per row (a `Str` key clones its string, as the linear search
  cloned it into `key` already).

## Errors, failure, recovery, and observability

Nothing new; not observable on the wire.

## Security, privacy, and compatibility

The same groups; no wire change.

## Acceptance criteria

1. In-process: `AGB-FR-002`'s shapes.
2. Integration (`tests/server_sql_integration.rs`): `AGB-FR-002`'s
   shapes on `Reminder`.
3. Measured (`RESULTS.md`): `due-hist-5k-pending` before and after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after, back to back. Independent review
owed.

## Traceability

- Roadmap: `SERVER-AGGREGATE-HASHED-BUCKET`.
- Decision: `ADR-0085`.
- Specification: `SERVER-001` v0.70.0 / `FR-082` (amends `FR-038`'s
  `AGG-FR-007`).
- Requirements: `AGB-FR-001`–`003`.

## Open questions

- **Plan metrics** — still named, still not bundled. *Taken: `ADR-0086`
  (2026-09-21) — `docs/design/SERVER-QUERY-PLAN-METRICS-DESIGN.md`.*
- **A fold that never materializes the keys** — unchanged from
  `ADR-0082`.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as
  `ADR-0084`'s own option (b). Read from `main` after PR #284 this pass.
  The `due-hist-5k-pending` bench row was added and measured on the
  pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.70.0 /
  `FR-082`, exactly the "Proposed shape". Acceptance criteria 1–2 are
  the tests: `serve.rs` +1
  (`evaluate_aggregate_hashed_buckets_are_the_linear_search_s_buckets_in_first_seen_order`),
  `tests/server_sql_integration.rs` +1
  (`group_by_through_the_decode_path_buckets_exactly_over_a_socket`)
  (lib 638, up from 637; SQL 60, up from 59); 925 tests across
  39 targets, 0 failed, every pre-existing test unmodified. Criterion 3,
  `RESULTS.md`: `due-hist-5k-pending` 4,543.6 → 2,572.7 µs. Still
  no independent review — owed.
