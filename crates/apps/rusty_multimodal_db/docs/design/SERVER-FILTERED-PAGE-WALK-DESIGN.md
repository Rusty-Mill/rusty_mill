# Server Filtered Page: The O(page) Bounded Walk for a Range-Filtered `FilteredPage` (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-20,
  `ADR-0076`; the `ADR-0059` precedent — design and implementation in
  one PR). Selected under the owner's standing instruction to keep
  working the future-growth list; the fork below stays open for the
  owner at review.
- Date: 2026-09-20
- Related: `ADR-0075`/`docs/design/SERVER-QUERY-PLANNER-RANGE-DESIGN.md`
  (whose option (b) this round is, and whose Non-goals scoped it out
  "so that (a) changes one helper and proves it, not one helper and two
  adapter overrides"), `ADR-0068`/`docs/design/SERVER-FILTERED-PAGE-DESIGN.md`
  (`FilteredPage`, whose design chose "a trait method with a shared
  default so a domain *could* later narrow candidates more cheaply" —
  this is that override), `ADR-0059`/`docs/design/SERVER-ORDERED-INDEX-DESIGN.md`
  (`Ordered`, `PageBy::page_by` — the walk this round reuses unchanged),
  `ADR-0055` (`Page`, the unfiltered fast path whose consistency class
  this round inherits), `docs/FUTURE-GROWTH.md` (SQL-parity item 1:
  "the O(page) bounded walk for a `since`-shaped `FilteredPage`
  (`ADR-0075`'s own option (b), not taken)").
- Supersedes/Superseded by: none. Additive: one free function
  factoring the `filtered_page` trait default's body (so an override
  can fall back to it), two free helpers, and two adapter overrides
  (`Memory`, `Relation`). No generic-layer, wire, protocol-version,
  `FieldCapabilities`, client, or file-format change; `PageBy`/`page_by`
  reused as is. Every eligible request returns the **identical
  sequence** the default returns (`FPW-FR-004`).

## Purpose and scope

`ADR-0075` gave every filtered read a range path: a `WHERE` on
`Memory`/`Relation`'s `updated_at_unix_ms` walks the `Ordered` index
between the bounds instead of scanning. For `FilteredPage` it named
plainly what that still is not: the shared candidate step reads *every*
in-range record, then `page_rows` sorts them and cuts the page. For the
`since`-shaped listing — `WHERE updated_at_unix_ms > since ORDER BY
updated_at_unix_ms LIMIT 50`, the shape a consumer syncing "what changed
since my last pull" issues — a bound admitting most of the table costs
*k* ≈ *n* reads for 50 rows. `RESULTS.md`'s step-three section measured
the 1%-selective case only; this round's bench row (`index-since`,
measured first on the pre-change code) is the 99% case.

Yet `Memory`/`Relation` already answer the *unfiltered* `Page` by that
field as `page_by(cursor, limit)`: a walk from just past the cursor,
`limit` ids, no decode of anything else (`ORD-FR-005`). A filtered page
whose `order_by` is that same field and whose filter is *only* bounds
on it is the same walk from a composed cursor: the later of the client's
cursor and the tightest lower bound, ending early at the first row an
upper bound rejects — which, because the walk is ascending on the very
key the bounds are on, is where every later row would be rejected too.

Scope, exactly: eligibility (`FPW-FR-001`); the start cursor
(`FPW-FR-002`); the walk and its prefix cut (`FPW-FR-003`); sequence
identity with the default, proven (`FPW-FR-004`); everything ineligible
unchanged (`FPW-FR-005`); no other surface changed (`FPW-FR-006`).

## Non-goals

- **A filter on any other field alongside the bounds** (`WHERE
  updated_at > since AND category = 'x' ORDER BY updated_at`). The
  prefix property (`FPW-FR-003`) holds only when every predicate is on
  the walked key; a second field's predicate can reject rows anywhere,
  so the page would have to keep walking past rejects until it fills —
  a real, bounded extension (walk-and-filter until `limit` matches or
  the index ends), but a different cost shape (unbounded in the worst
  case) and a different proof. Named; ineligible here, the default
  answers it (`FPW-FR-005`).
- **`Ne` on the walked field.** A `!=` splits the key space in two;
  ineligible, default path.
- **`order_by` on any field other than the range field**, and every
  domain without one (`Dog`, `Order`, `Employee`, `Reminder`, `Entity`):
  the default, unchanged.
- **Descending walks, a wire cursor for `Query`, a generic
  `range_by(..).take(limit)`.** `page_by` already is the limited walk;
  no generic change is needed or made.
- **Any change to `Page` (`ADR-0055`), `page_by`, `Ordered`, the
  planner, or `indexed_candidates`.** The override lives in the two
  adapters and calls the same `page_by` `Page` does.

## Context and terminology

Read from `main` after `ADR-0075` (`SERVER-001` v0.60.0) this pass:

- **`ConnectionStore::filtered_page`'s default body**: `indexed_candidates`
  (now walking the range for a bound on the range field, `QPR-FR-004`)
  → keep rows every predicate matches → `page_rows(filtered, order_by,
  after, limit)`, which sorts the filtered set by `(key, id)` and
  returns the first `limit` strictly after `after`. No adapter overrides
  it. A default body cannot be called from an override in Rust, so the
  body is factored into a free `filtered_page_by_candidates` this round
  and the default becomes a one-line call to it (`FPW-FR-005`).
- **`page_by(after: Option<(Key, Id)>, limit)`** (`ORD-FR-002`): ids
  strictly after the cursor pair, ascending, at most `limit`. `Memory`'s
  and `Relation`'s `page` overrides call it for `order_by ==
  FIELD_UPDATED_AT` and then `get` each id, `filter_map`-dropping an id
  whose record vanished — `Page`'s consistency class.
- **Bounds as cursors.** `page_by`'s cursor is "strictly after a pair".
  A lower bound on the key is one: `key > k` is the cursor `(k, MAX_ID)`
  (nothing with key *k* lies strictly after `(k, MAX_ID)`; everything
  with key > *k* does); `key >= k` and `key = k` are the cursor
  `(k - 1, MAX_ID)` — nothing lies strictly between `(k - 1, MAX_ID)` and
  `(k, MIN_ID)` — except when *k* is `i64::MIN`, where there is no
  cursor at all (unbounded start). `MAX_ID` is `Uuid::max()`, the same
  sentinel `uuid_pair_bounds` uses.
- **Prefix property.** Over rows in ascending `(key, id)` order, a
  predicate of the form `key < u`, `key <= u`, or `key = u` (as an upper
  bound) holds for a prefix and fails for the rest; a lower-bound
  predicate holds for every row at or after its cursor. So, for a walk
  that starts at or after every lower bound's cursor, the rows
  satisfying *all* the predicates are exactly a prefix of the walk.
- **The `since` shape in this crate's own tests**:
  `tests/server_hub_differential.rs`'s `count_since` twin is `WHERE
  updated_at_unix_ms > t` (an `Aggregate`, not paged); the paged listing
  the hub issues over `Page` today is the unfiltered cursor walk. This
  round makes the filtered form cost the same as the unfiltered one.

## Requirements

- `FPW-FR-001` **Eligibility.** An adapter with `range_field() ==
  Some(f)` answers a `FilteredPage` by the bounded walk iff `order_by ==
  f` and every predicate in `filter` has `field == f` and `op != Ne`.
  An empty `filter` is eligible (it is the unfiltered `Page`, and the
  walk answers it identically). Anything else takes the default
  (`FPW-FR-005`). Decided by one pure `bounded_walk_applies(order_by,
  range_field, filter)`.
- `FPW-FR-002` **Start cursor.** `bounded_walk_start(after, filter)`
  computes the cursor the walk begins strictly after: the maximum, in
  `(key, id)` order, of the client's `after` cursor (if any) and every
  lower-bound predicate's cursor per "Context" (`Gt k` → `(k, MAX_ID)`;
  `Ge k`/`Eq k` → `(k - 1, MAX_ID)`, or no cursor when `k == i64::MIN`);
  `None` when none applies. `Lt`/`Le` contribute nothing here (they are
  the cut, `FPW-FR-003`). A bound whose literal is not `I64`, or a
  cursor whose value is not `I64`, is `Malformed` — unreachable through
  `dispatch`, which validated both, and present so the helper is honest
  as surface. Taking the *tightest* lower bound is not the planner's
  declined "bound tightening": every bound here is on one key, and the
  maximum is exact by construction, not a cost estimate.
- `FPW-FR-003` **Walk and cut.** `page_by(start, limit)` → `get` each id
  in walk order, dropping an id whose record vanished (`Page`'s own
  class) → keep the longest prefix of rows every predicate matches
  (`take_while`) → that is the page. Cost: one `page_by` walk of at most
  `limit` pairs and at most `limit` `get`s, regardless of how many
  records the bounds admit. A row failing an upper bound ends the page
  because every later row fails it too (prefix property); a page
  shorter than `limit` for that reason has no further matches, exactly
  as the default's `page_rows` would have returned no more.
- `FPW-FR-004` **Sequence identity, proven.** For every eligible
  request, with no concurrent writer, the override returns the
  identical `Vec<PageRow>` — same ids, same order, same fields — that
  `filtered_page_by_candidates` returns for it. Proven in-process on
  `Memory` and `Relation` against that exact oracle (which a test can
  call directly) over every comparator, two-sided bounds, `=`, a client
  cursor combined with a lower bound in both orders (cursor later /
  bound later), a cursor past every row, an empty filter, `limit`
  smaller than / equal to / larger than the match count, an inverted
  range, and `i64::MIN`/`i64::MAX` literals; and over a real socket via
  SQL (`WHERE updated_at_unix_ms … ORDER BY updated_at_unix_ms LIMIT
  n`) against the sorted oracle rows, after runtime `Insert`,
  re-keying `Replace`, and `Delete`.
- `FPW-FR-005` **Ineligible requests unchanged.** `Ne`, any predicate
  on another field, another `order_by`, or a domain with no range field
  → the factored default, byte-for-byte the v0.60.0 behavior. Proven
  by the same in-process oracle (the override's answer equals the
  default's because it *is* the default) and by the existing filtered-
  page tests passing unmodified.
- `FPW-FR-006` **Everything else unchanged.** No generic-layer change;
  no `Request`/`Response`/`ErrorCode` variant; `PROTOCOL_VERSION` stays
  27; `FieldCapabilities`/`describe()`/`sql.rs`/`client.rs`/
  `clients/python/**`/`SERVER-002` untouched; `Page`, `page_by`,
  `Ordered`, `plan_query`, `indexed_candidates` untouched; the
  `filtered_page` trait *signature* untouched (only its default body
  moves into a free function it now calls).

## Considered options

- **(a) The bounded walk as scoped above — implemented.** Two
  overrides of about ten lines each over one shared `bounded_filtered_page`
  helper; `page_by` reused; the eligible page costs the page.
  Cost: a third code path answers `FilteredPage` on two domains (the
  default, the range candidate step inside it, and this walk), each
  proven equal to the next.
- **(b) Also walk-and-filter past rejects when other predicates are
  present.** Fills the page for `WHERE updated_at > since AND category
  = 'x'` by continuing the walk until `limit` matches — O(page) in the
  common case, O(*k*) in the worst. Real and bounded; a second round
  once (a)'s shape has proven out, with its own worst-case measurement.
- **(c) Decline.** The `since`-shaped filtered page keeps reading every
  in-range record; a consumer that wants the cheap walk uses `Page`
  with a cursor and stops when the key passes its bound — which is
  exactly what (a) does server-side.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus
walk-past-rejects for mixed filters; **(c)** decline and revert.

## Proposed shape

`src/server/serve.rs`: `pub fn filtered_page_by_candidates<S>(store,
order_by, after, limit, filter) -> Vec<PageRow>` — the former default
body verbatim; the trait default calls it. `pub fn bounded_walk_applies(
order_by, range_field, filter) -> bool` (`FPW-FR-001`). `pub fn
bounded_walk_start(after, filter) -> Result<Option<(i64, RecordId)>,
ErrorCode>` (`FPW-FR-002`), beside `uuid_pair_bounds` since it is the
same `i64`/`Uuid`-specific mapping. `pub fn bounded_filtered_page<S>(
store, walk: impl Fn(Option<(i64, RecordId)>, usize) -> Vec<RecordId>,
after, limit, filter) -> Result<Vec<PageRow>, ErrorCode>` (`FPW-FR-003`)
— the start, the walk through the adapter-supplied closure, the `get`s,
the `take_while`.

`src/server/memory.rs`/`relation.rs`: `fn filtered_page(..)` override —
`if !bounded_walk_applies(order_by, self.range_field(), filter) { return
Ok(filtered_page_by_candidates(self, ..)); }` then
`bounded_filtered_page(self, |start, limit| self.store.page_by::<Memory,
UpdatedAtOrder>(start, limit), after, limit, filter)` (`Relation` over
`UpdatedAtField`).

## Data/state and invariants

- **Sequence invariant** (`FPW-FR-004`): the default sorts the filtered
  set by `(key, id)` and takes the first `limit` strictly after `after`.
  The walk visits pairs in that same `(key, id)` order strictly after
  `start`, where `start` ≥ `after`; every visited row at or after
  `start` satisfies every lower bound (by the cursor construction), so
  the rows before `start` that the default would have skipped are
  exactly the ones failing a lower bound or not strictly after `after`
  — none of which the default returns either; and the prefix cut
  removes exactly the rows failing an upper bound, after which no row
  matches. Hence the two sequences are equal, fields included, since
  both read the same records through the same `get`.
- **Consistency class**: `Page`'s (`ORD-FR-005`), not the default's —
  the walk names ids under one read lock, then `get`s each under its
  own; an id deleted in between is dropped and the page is shorter by
  one (the default, having materialized every row before paging,
  would have filled the page from the next match). A record re-keyed
  in between may be read with a value the predicates reject, ending
  the page early. Both are the class the unfiltered fast path already
  has; a client re-issues with the last returned cursor, as it does
  for `Page`.
- **`Eq` as a bound**: `key = u` is both the lower cursor `(u - 1, MAX)`
  and an upper cut (`take_while` on `= u`), so the page is exactly the
  key-*u* records strictly after `after`, in id order.
- **Inverted or empty ranges** (`> 5 AND < 3`): the walk starts after
  `(5, MAX)` and the first row fails `< 3` — an empty page, no error;
  the default's answer too.

## Errors, failure, recovery, and observability

No new `ErrorCode`; the two `Malformed` arms in `bounded_walk_start`
are unreachable through `dispatch` (`validate_filtered_page` already
checked the cursor's and every literal's kind). Lock poisoning panics
where it does today. A read throughout; nothing to recover. The path
taken is not observable on the wire, as with every plan.

## Security, privacy, and compatibility

A read, gated exactly as `FilteredPage` is (protocol 26, `FPG-FR-007`).
The walk reads no more than `limit` records and returns the identical
sequence; nothing becomes obtainable that was not. No wire change; no
protocol bump; a client of any version observes only speed.

## Acceptance criteria

1. `bounded_walk_applies` unit tests: eligible for the range field with
   an empty filter, one bound, two bounds, `=`; ineligible for `Ne`, a
   second field, another `order_by`, `range_field: None`.
2. `bounded_walk_start` unit tests: no cursor, no bounds → `None`; `Gt
   k` → `(k, max)`; `Ge k`/`Eq k` → `(k - 1, max)`; `Ge i64::MIN` → `None`;
   cursor later than the bound → the cursor; bound later than the
   cursor → the bound; `Lt`/`Le` ignored; a non-`I64` literal or cursor
   → `Malformed`.
3. In-process sequence identity on `Memory` and `Relation` (adapter
   unit tests) against `filtered_page_by_candidates` for the matrix in
   `FPW-FR-004`, plus the ineligible shapes (`FPW-FR-005`) — asserting
   both that the answers are equal and, for the eligible ones, that the
   page was walked (the adapter's `page_by`-backed path, observed via
   an `Ordered`-index tie the scan-and-sort would order identically —
   so the test asserts equality only, and the cost claim is criterion
   5's).
4. Integration (`tests/server_sql_integration.rs`, real socket, SQL):
   `WHERE updated_at_unix_ms {>,>=,<,<=,=} v ORDER BY updated_at_unix_ms
   [LIMIT n]` and a two-sided range on `Memory` and `Relation` as the
   exact sorted-oracle sequence, after runtime `Insert`, re-keying
   `Replace`, and `Delete`; `!=` and a mixed filter (`AND category = …`)
   still exact through the default. Every pre-existing test unmodified.
5. Measured (`benches/server.rs` `memory-planner` `index-since` row,
   `RESULTS.md`): `FilteredPage` `WHERE updated_at_unix_ms >= 1000
   ORDER BY updated_at_unix_ms LIMIT 50` (admitting ~99% of 100K
   records) **before** this round (walk every in-range record, then
   page) and **after** (the bounded walk), with the `created_at_unix_ms`
   full-scan control in both runs. The prediction: after ≈ the
   unfiltered `Page` cost `ADR-0059` measured (~150–160 µs plus the
   filter), before ≈ the full-scan control.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
`cargo bench --bench server` before and after. Independent review:
written and built by Claude while Codex's sandbox still cannot spawn
processes; a fresh Codex inspection of design and code owed.

## Traceability

- Roadmap: `SERVER-FILTERED-PAGE-WALK`.
- Decision: `ADR-0076`.
- Specification: `SERVER-001` v0.61.0 / `FR-073` (extends `FR-068`'s
  `FilteredPage` and `FR-072`'s range path).
- Requirements: `FPW-FR-001`–`006`.

## Open questions

- **Walk-past-rejects for mixed filters** — option (b). *Taken:
  `ADR-0077` (2026-09-21), the round after PR #276 merged this one —
  `docs/design/SERVER-FILTERED-PAGE-WALK-MIXED-DESIGN.md`. It also
  found and fixed the one latent gap here: `bounded_walk_start`
  computed a cursor from every `Gt`/`Ge`/`Eq` predicate, correct only
  while `FPW-FR-001` kept every predicate on the walked field.*
- **The same walk for `Query`/`Aggregate` with `LIMIT`** — a `Query`
  has no order, so a `LIMIT` there is "any *n*"; the walk would give a
  cheap, ordered *n*. Not asked for.
- **A wire cursor on `Query`** — unchanged, not asked for.

## Change history

- 2026-09-20: proposed and implemented on `claude/planner-filtered-page-walk`
  under the owner's standing "keep working the future-growth list"
  instruction, as `ADR-0075`'s own option (b) — the smallest slice that
  directly continues the planner line. Read from `main` after PRs #273/
  #274 this pass, including that the client has no cursor-bearing
  `filtered_page` call and no test sends a raw `FilteredPage` with a
  cursor — hence the in-process oracle for cursor composition. The
  `index-since` bench row was added and measured on the pre-change
  code first, so the before/after is one harness.
- 2026-09-20: implemented on the same branch as `SERVER-001` v0.61.0 /
  `FR-073`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the new tests: `serve.rs` +2
  (`bounded_walk_applies_only_to_bounds_on_the_ordered_field`,
  `bounded_walk_start_is_the_later_of_the_cursor_and_the_tightest_lower_bound`),
  `memory.rs` +1
  (`filtered_page_bounded_walk_returns_the_default_body_s_exact_sequence`
  — 26 shapes against `filtered_page_by_candidates`, three answers
  pinned independently; lib 617, up from 614),
  `tests/server_sql_integration.rs` +1
  (`since_shaped_filtered_page_walks_the_index_and_returns_the_exact_sequence`;
  52, up from 51); 896 tests across 39 targets, 0 failed,
  every pre-existing test unmodified; `fmt`/`clippy -D warnings` clean.
  Criterion 5, `RESULTS.md`: the `since`-shaped page went from
  123,436.1 µs (control 146,098.2 µs) to 89.4 µs
  (control 143,975.6 µs), ~1,380×. One deviation from the
  criterion-3 wording: the in-process test asserts sequence equality
  and pins three answers; it does not observe which path ran (no
  counter on the real adapter), so the cost claim rests on criterion
  5 alone, as the criterion itself anticipated. Still no independent
  review — owed.
- 2026-09-21: merged unchanged as PR #276 (option (a) confirmed by the
  owner's merge); option (b) taken as `ADR-0077` the same day.
