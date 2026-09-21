# Server Filtered Page: Walk-Past-Rejects for a Mixed-Filter `FilteredPage` (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0077`; the `ADR-0059`/`ADR-0076` precedent — design and
  implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as `ADR-0076`'s
  own option (b), the round after PR #276 merged (a); the fork below
  stays open for the owner at review.
- Date: 2026-09-21
- Related: `ADR-0076`/`docs/design/SERVER-FILTERED-PAGE-WALK-DESIGN.md`
  (whose option (b) and first Non-goal this round is — "a real, bounded
  extension (walk-and-filter until `limit` matches or the index ends),
  but a different cost shape (unbounded in the worst case) and a
  different proof"), `ADR-0073`/`ADR-0075` (`plan_query`'s
  equality-first rule, which this round's eligibility defers to),
  `ADR-0068` (`FilteredPage`), `ADR-0059` (`Ordered`/`page_by`, reused
  unchanged), `ADR-0055` (`Page`, whose consistency class the walk
  keeps), `docs/FUTURE-GROWTH.md` (SQL-parity item 1: "the
  walk-past-rejects extension of `ADR-0076`'s bounded `FilteredPage`
  walk for a *mixed* filter").
- Supersedes/Superseded by: none. Additive over `ADR-0076`: the same
  four `serve.rs` helpers, widened — `bounded_walk_applies` takes the
  schema and defers to `plan_query`; `bounded_walk_start` takes
  `order_by` and reads bounds on that field alone; `bounded_filtered_page`
  takes `order_by`, walks in chunks, and continues past rejects; one new
  helper, `walked_key`. The two adapter overrides pass `order_by` and
  `describe()`. No generic-layer, wire, protocol-version,
  `FieldCapabilities`, client, or file-format change; `PageBy`/`page_by`
  reused as is. Every eligible request returns the **identical
  sequence** the default returns (`FPM-FR-004`).

## Purpose and scope

`ADR-0076` made the `since`-shaped listing — `WHERE updated_at_unix_ms >
since ORDER BY updated_at_unix_ms LIMIT 50` — cost the page, and named
plainly what it did not: a *mixed* filter (`… AND category = 'x'`) still
took the `ADR-0075` path, reading every in-range record before
`page_rows` cut the page, because the prefix property that let the walk
stop at the first reject holds only when every predicate is a bound on
the walked key. The `since-mixed` bench row this round added and
measured on the pre-change code first (`RESULTS.md`) is that cost: a
50-row page whose bound admits ~99% of 100K records and whose second
predicate admits 1% of those, within sight of the full-scan control.

Read against the walk: a predicate on another field, or `!=` on the
walked field, can reject a row *anywhere* in the walk, but it cannot
change where the walk *ends* — only a bound on the walked key can, and
those still cut at the first reject exactly as before. So the walk
keeps going past the other rejects until `limit` rows match, a cut
predicate rejects, or the index ends, resuming each chunk strictly
after the last `(key, id)` it read. The cost becomes the page divided by
the rejects' selectivity — O(page) in the common case, every in-range
record in the worst, never more than the default read anyway.

One rule keeps this from fighting the planner. When the filter carries an
equality on a field the domain declares `filter_eq: true`, `plan_query`'s
equality-first rule (`ADR-0073`, kept verbatim by `ADR-0075`) reads that
bucket, and every filtered read that planned it before this round still
does: the walk's eligibility is `order_by` is the range field **and**
the filter does not plan `IndexEq`. Which is cheaper for a given
request — the bucket, sorted and cut, or the walk past rejects — depends
on selectivities the planner has never estimated (`ADR-0073`'s own
Non-goal), so this round does not guess; it measures the rule's cost
(`since-eq`, `RESULTS.md`) and names it.

Scope, exactly: eligibility (`FPM-FR-001`); the walked key and the
resume cursor (`FPM-FR-002`); the chunked walk, its cut, and its
rejects (`FPM-FR-003`); sequence identity with the default, proven
(`FPM-FR-004`); the equality-index filter unchanged (`FPM-FR-005`); no
other surface changed (`FPM-FR-006`).

## Non-goals

- **Intersecting the equality index with the range** — a filter that
  plans `IndexEq` keeps the bucket even when the bound would admit
  almost nothing of it. That is a selectivity decision, `ADR-0073`'s
  declined cost model; named, with the `since-eq` row as its measured
  cost.
- **A chunk size other than `limit`.** Each chunk is one `page_by` of
  `limit` pairs; a `LIMIT 1` against a rare match walks one pair per
  call. A growing chunk would trade lock acquisitions for overshoot;
  not measured, not taken.
- **Descending walks, a wire cursor for `Query`, the same walk for
  `Query`/`Aggregate` with `LIMIT`, a generic `page_pairs_by`.** The
  resume cursor is read off the record, not the index, precisely so no
  generic change is needed (`FPM-FR-002`).
- **Any change to `Page` (`ADR-0055`), `page_by`, `Ordered`,
  `plan_query`, or `indexed_candidates`.** The override lives in the
  two adapters and calls the same `page_by` `Page` does.

## Context and terminology

Read from `main` after PR #276 (`SERVER-001` v0.61.0) this pass:

- **`bounded_walk_applies(order_by, range_field, filter)`** was
  `range_field == Some(order_by)` and every predicate a non-`Ne` bound on
  `order_by`. It becomes `range_field == Some(order_by)` and
  `plan_query(schema, range_field, filter)` is not `IndexEq`.
- **`bounded_walk_start(after, filter)`** computed a cursor from every
  `Gt`/`Ge`/`Eq` predicate — correct when every predicate was on the
  walked field, `Malformed` the moment a `Str` equality on another
  field reached it. It takes `order_by` and considers that field's
  predicates alone.
- **`bounded_filtered_page`** walked one chunk and `take_while`d. It
  loops.
- **Cut and reject.** A *cut* predicate is a non-`Ne` comparison on the
  walked field: over rows ascending in `(key, id)`, once one fails, every
  later row fails it too (`ADR-0076`'s prefix property, unchanged). A
  *reject* is every other predicate — another field's, any comparator;
  or `Ne` on the walked field, which rejects one key's run and admits
  everything after. A row failing a cut ends the page; a row failing
  only rejects is skipped.
- **The resume cursor.** `page_by` returns ids, not pairs; the walked
  key is read off each record through the same `get` the row needs
  anyway (`walked_key`). The cursor after a chunk is the greatest
  `(key, id)` read, never less than the cursor before it.
- **`Page`'s consistency class** (`ORD-FR-005`): the walk names ids
  under one read lock and reads each under its own. Under `ADR-0076` a
  vanished id shortened the page; now the walk simply continues past
  it and back-fills from the next pair. A record re-keyed between the
  walk and its read is read at the new key, which — if later — is where
  the next chunk resumes, passing over whatever lay between. A chunk
  that advances the cursor by nothing (every id vanished, or re-keyed
  to before the cursor) ends the page rather than walk the same pairs
  again, so the walk always terminates.

## Requirements

- `FPM-FR-001` **Eligibility.** An adapter with `range_field() ==
  Some(f)` answers a `FilteredPage` by the bounded walk iff `order_by ==
  f` and `plan_query(describe(), range_field(), filter)` is not
  `QueryPlan::IndexEq` — i.e. no predicate is an `Eq` on a field the
  domain declares `filter_eq: true`. An empty filter, bounds alone,
  `Ne` on `f`, and predicates on unindexed fields are all eligible.
  Decided by `bounded_walk_applies(order_by, range_field, schema,
  filter)`, pure over its inputs.
- `FPM-FR-002` **Walked key and resume cursor.** `walked_key(fields,
  order_by)` is the record's own `I64` value under `order_by`
  (`Malformed` otherwise — unreachable for an adapter's own records).
  `bounded_walk_start(order_by, after, filter)` is `ADR-0076`'s
  `FPW-FR-002` restricted to predicates on `order_by`; another field's
  predicate contributes nothing. After each chunk the cursor is the
  greatest `(walked_key, id)` read, or unchanged if none exceeded it.
- `FPM-FR-003` **Walk, cut, rejects.** Loop: `page_by(cursor, limit)` →
  for each id, `get` (a vanished id is skipped) → if any cut predicate
  fails, return the page → advance the cursor → if every predicate
  holds, append; at `limit` rows return the page. After the chunk, if
  it held fewer than `limit` ids (the index ended) or advanced the
  cursor by nothing, return the page. For a filter of cuts alone the
  first chunk decides: at most `limit` pairs walked and records read —
  `ADR-0076`'s cost, unchanged. For a mixed filter, the pairs walked and
  records read are those up to the `limit`-th match (or the cut, or
  the end), never more than the default's candidate set.
- `FPM-FR-004` **Sequence identity, proven.** For every eligible
  request, with no concurrent writer, the override returns the
  identical `Vec<PageRow>` `filtered_page_by_candidates` returns.
  Proven in-process on an `I64`-keyed fixture (rejects filling across
  chunks with the exact `get` count asserted; the cut; the index's end;
  `Ne` on the walked field; a cursor with rejects; eleven shapes
  against the default; the three concurrency windows — vanished,
  re-keyed, a chunk advancing nothing — each pinned; a non-`I64` key
  `Malformed`), on `Memory` against that exact oracle (`ADR-0076`'s 26
  shapes plus six mixed ones, two more pinned), and over a real socket
  via SQL on `Memory` and `Relation` (an unindexed equality that rejects
  nothing, one that rejects everything, one that admits one row past
  two rejects, `!=` on the walked field alone and with other rejects,
  with and without `LIMIT`, after runtime `Insert`, re-keying `Replace`,
  and `Delete`).
- `FPM-FR-005` **The equality-index filter unchanged.** A filter that
  plans `IndexEq` answers through `filtered_page_by_candidates`, the
  bucket, byte-for-byte v0.61.0 — proven by the same oracles (the
  override's answer equals the default's because it *is* the default
  there) and by every pre-existing filtered-page test passing
  unmodified. `ADR-0076`'s own eligibility test is rewritten for the
  new signature (its `Ne`/second-field/`Malformed` expectations were
  the very contract this round changes), and its `bounded_walk_start`
  test gains the `order_by` argument and one assertion; no other test
  changed.
- `FPM-FR-006` **Everything else unchanged.** No generic-layer change;
  no `Request`/`Response`/`ErrorCode` variant; `PROTOCOL_VERSION` stays
  27; `FieldCapabilities`/`describe()`/`sql.rs`/`client.rs`/
  `clients/python/**`/`SERVER-002` untouched; `Page`, `page_by`,
  `Ordered`, `plan_query`, `indexed_candidates`,
  `filtered_page_by_candidates` untouched; the `filtered_page` trait
  signature untouched.

## Considered options

- **(a) Walk past rejects, equality-first kept — implemented.** Every
  `order_by`-on-the-range-field page walks unless the filter plans the
  declared equality index. Cost: a loop where there was a `take_while`;
  the resume cursor read off the record; the bucket kept even where a
  tight bound would have beaten it.
- **(b) (a) plus intersecting the equality index with the range** —
  when a filter carries both, read the bucket and keep only ids the
  range admits (or walk the range and keep only bucket members),
  whichever is smaller. Needs a size for each: the bucket's is one
  `filter_eq` call away, the range's is a `BTreeSet::range` count — the
  first selectivity estimate this crate would keep. A real round, not
  this one.
- **(c) Decline.** The mixed page keeps reading every in-range record;
  a consumer that needs it cheap issues `Page` with a cursor and
  filters client-side — which is exactly what (a) does server-side.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus the
equality/range intersection; **(c)** decline and revert.

## Proposed shape

`src/server/serve.rs`: `pub fn bounded_walk_applies(order_by,
range_field, schema: &DomainSchema, filter) -> bool` (`FPM-FR-001`).
`pub fn walked_key(fields, order_by) -> Result<i64, ErrorCode>`
(`FPM-FR-002`). `pub fn bounded_walk_start(order_by, after, filter)`
(`FPM-FR-002`). `pub fn bounded_filtered_page<S>(store, walk, order_by,
after, limit, filter) -> Result<Vec<PageRow>, ErrorCode>` (`FPM-FR-003`)
— the loop.

`src/server/memory.rs`/`relation.rs`: the override passes
`&self.describe()` to `bounded_walk_applies` and `order_by` to
`bounded_filtered_page`; nothing else.

`benches/server.rs`: `since_mixed_page_request(range_field, eq_field)`
and two `measure_planner_pair` rows — `since-mixed` (the unindexed
`source`, the walk) and `since-eq` (the declared `category`, the rule's
cost) — both against the `created_at_unix_ms` + `source` full-scan
control; and every control page (`planner_requests`, `since_page_request`,
this round's) ordered by `created_at_unix_ms`, since a control ordered
by the range field would itself walk from this round on.

## Data/state and invariants

- **Sequence invariant** (`FPM-FR-004`): the default sorts the filtered
  set by `(key, id)` and takes the first `limit` strictly after `after`.
  The walk visits pairs in that order strictly after `start ≥ after`;
  rows before `start` fail a lower bound or are not after `after`, so
  the default skips them too; every visited row failing only rejects is
  one the default drops too; the first row failing a cut is where every
  later row fails it, so nothing after it is in the default's set; and
  the walk stops at `limit` matches, the default's own truncation. Both
  read the same records through the same `get`, so fields match.
- **Termination**: each iteration either returns, or strictly advances
  the cursor (a read pair greater than it), or observes a chunk shorter
  than `limit`; a chunk that neither advances nor ends returns. The
  index is finite and the cursor monotone, so the loop ends.
- **Cost**: pairs walked ≤ the position of the `limit`-th match (or the
  cut, or the end) rounded up to a chunk; records read = pairs walked
  minus vanished ids. Never more than `ADR-0075`'s candidate set, which
  the default reads in full.
- **Consistency class**: `Page`'s, as under `ADR-0076`, with one change
  named in "Context": a vanished id is back-filled rather than
  shortening the page.

## Errors, failure, recovery, and observability

No new `ErrorCode`; the `Malformed` arms in `bounded_walk_start` and
`walked_key` are unreachable through `dispatch` and an adapter's own
records. Lock poisoning panics where it does today, once per chunk. A
read throughout; nothing to recover. The path taken is not observable
on the wire, as with every plan.

## Security, privacy, and compatibility

A read, gated exactly as `FilteredPage` is (protocol 26, `FPG-FR-007`).
The walk returns the identical sequence; nothing becomes obtainable
that was not. The worst-case read is the default's own. No wire
change; no protocol bump; a client of any version observes only speed.

## Acceptance criteria

1. `bounded_walk_applies` unit tests: eligible for the range field with
   an empty filter, bounds, `=`, `Ne`, an unindexed equality, `Ne` on an
   indexed field; ineligible for an equality on either declared index,
   another `order_by`, `range_field: None`.
2. `walked_key` and `bounded_filtered_page` unit tests on an `I64`-keyed
   fixture: rejects filling across chunks with the exact `get` count;
   the cut ending the page short; the index's end; `Ne` as a reject; a
   cursor composed with a bound and rejects; every shape equal to
   `filtered_page_by_candidates`; a vanished id back-filled; a re-keyed
   row moving the next chunk's start; a chunk advancing nothing ending
   the page with the `get` count asserted; a non-`I64` key `Malformed`.
   `bounded_walk_start` ignores another field's predicate.
3. In-process sequence identity on `Memory` against
   `filtered_page_by_candidates` for `ADR-0076`'s matrix plus the mixed
   shapes in `FPM-FR-004`, two more answers pinned.
4. Integration (`tests/server_sql_integration.rs`, real socket, SQL):
   the `FPM-FR-004` shapes on `Memory` and `Relation` as the exact
   sorted-oracle sequence, through runtime writes; the declared-index
   equality still exact (`FPM-FR-005`). Every pre-existing test
   unmodified except the two named in `FPM-FR-005`.
5. Measured (`benches/server.rs` `memory-planner` `since-mixed` and
   `since-eq` rows, `RESULTS.md`): `FilteredPage` `WHERE
   updated_at_unix_ms >= 1000 AND source = 'c7' ORDER BY
   updated_at_unix_ms LIMIT 50` **before** this round (every in-range
   record read, then paged) and **after** (the walk past rejects), with
   the `created_at_unix_ms` + `source` full-scan control in both runs,
   the rows added and measured on the pre-change code first; and the
   `category = 'c7'` twin in both runs, unchanged by construction. The
   prediction: after ≈ 100 chunks of 50 (the 1% selectivity) — a few
   milliseconds — against before ≈ the control.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench binary built before and after, run back to back
with nothing else on the machine. Independent review: written and
built by Claude; a fresh Codex inspection of design and code owed.

## Traceability

- Roadmap: `SERVER-FILTERED-PAGE-WALK-MIXED`.
- Decision: `ADR-0077`.
- Specification: `SERVER-001` v0.62.0 / `FR-074` (extends `FR-073`).
- Requirements: `FPM-FR-001`–`006`.

## Open questions

- **Intersecting the equality index with the range** — option (b).
  *Taken: `ADR-0078` (2026-09-21) — and it needed no estimate: both
  indexes answer ids without a decode, so the intersection is exact;
  what stays open there is a width guard on the range's id walk.*
- **A growing chunk** — measured only if a `LIMIT 1`-against-rare-match
  shape ever shows up in a consumer.
- **The same walk for `Query`/`Aggregate` with `LIMIT`** — unchanged
  from `ADR-0076`, not asked for.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction, as
  `ADR-0076`'s own option (b), the round after PR #276 merged (a). Read
  from `main` after PR #276 this pass, including that
  `bounded_walk_start` would answer `Malformed` for any `Str` equality
  it saw — the one latent bug the widening surfaced, fixed by giving
  it `order_by`. The `since-mixed`/`since-eq` bench rows were added and
  measured on the pre-change code first, so the before/after is one
  harness. Three pairs of runs were needed: the first overlapped this
  session's own compilations on a 4-core container (discarded); the
  second, idle, showed the *unchanged* full-scan controls moving 40× —
  not noise but a harness bug this round created, every control page
  being ordered by `updated_at_unix_ms` and so, under the widened
  eligibility, walking too; the controls were re-paged by
  `created_at_unix_ms` (identical values, identical sequence), both
  binaries rebuilt on the fixed harness, and run back to back with
  nothing else running — the numbers below are that third pair. The
  second pair's control rows are kept in `RESULTS.md` as the real
  requests they are (an unlooked-for ~49× and the walk's worst case).
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.62.0 /
  `FR-074`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `serve.rs` +2 new
  (`walked_key_reads_the_order_by_field_as_i64`,
  `bounded_filtered_page_walks_past_rejects_until_the_page_fills`) and
  2 rewritten/extended, `memory.rs` +6 shapes and 2 pinned answers in
  the existing oracle test (lib 619, up from 617),
  `tests/server_sql_integration.rs` +1
  (`mixed_filtered_page_walks_past_rejects_and_returns_the_exact_sequence`;
  53, up from 52); 899 tests across 39 targets, 0 failed;
  `fmt`/`clippy -D warnings` clean. Criterion 5, `RESULTS.md`: the
  mixed `since`-shaped page went from 129,732.5 µs (control
  150,205.5 µs) to 3,933.2 µs (control 139,143.4 µs),
  ~33×; the `since-eq` twin 1,368.8 → 1,100.4 µs,
  unchanged within noise as predicted. Still no independent review —
  owed.
