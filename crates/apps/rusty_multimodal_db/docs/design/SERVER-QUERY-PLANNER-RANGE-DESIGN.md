# Server Query Planner, Step Three: A Range Path Through the `Ordered` Index (Accepted)

- Status: **Accepted as designed and implemented** (2026-09-20,
  `ADR-0075`, option (a) — the range candidate step for every consumer
  through the shared helper; (b) the O(page) `FilteredPage` walk on top
  and (c) decline, both declined). Design PR #273; implemented on
  `claude/planner-range-impl` as `SERVER-001` v0.60.0 / `FR-072`, no
  deviation — see "Change history".
- Date: 2026-09-20
- Related: `ADR-0073`/`docs/design/SERVER-QUERY-PLANNER-DESIGN.md`
  (step one — its own Non-goals name "Range predicates via the
  `Ordered` index" as "a second, distinct planner step with its own
  trait-surface question, not bundled here", and its Open questions
  name the two things it needs: "an 'ids in key range' primitive
  `Ordered` does not expose and a way for `DomainSchema` to say a field
  is range-indexed"), `ADR-0074`/`docs/design/SERVER-QUERY-PLANNER-CONSUMERS-DESIGN.md`
  (step two — the one shared `indexed_candidates` helper this round
  extends, so every consumer gains the range path at once),
  `ADR-0059`/`docs/design/SERVER-ORDERED-INDEX-DESIGN.md` (`Ordered`,
  the `BTreeSet<(key, id)>` this round walks by range — whose own
  Consequences say "descending and a range are one walk each of the
  same set, unrequested" and whose Open questions leave "a range on the
  wire" standing), `ADR-0055`/`ADR-0068` (`Page`/`FilteredPage`, the
  only reader of that index today), `ADR-0011` (`DomainSchema`/
  `FieldCapabilities`, which this round deliberately does *not*
  extend — see "Considered options"), `docs/FUTURE-GROWTH.md`
  ("Path to SQLite/DuckDB parity", item 1: "Still absent: … a range
  path through the `Ordered` index").
- Supersedes/Superseded by: none. Additive: one new trait and one new
  trait impl in `src/generic/` (`RangeBy`, on `Ordered`), one new
  `GenericProductionStore` accessor, two new `ConnectionStore` methods
  with defaults (`range_field`, `range_ids`), two adapter overrides
  (`Memory`, `Relation`), and a third `QueryPlan` variant in
  `src/server/serve.rs`. No `Request`/`Response`/`ErrorCode` variant,
  no protocol-version bump (`PROTOCOL_VERSION` stays 27), no
  `FieldCapabilities` change, no client (`src/server/client.rs`,
  `clients/python`) or `src/server/sql.rs` change, no file-format
  change. Every existing request returns the same *set* of rows; see
  "Data/state and invariants" for the observable differences this
  round names rather than hides.

## Purpose and scope

After `ADR-0073` and `ADR-0074`, every filtered read the server offers
— `Query`, `Aggregate`, the `filtered_page` default, `Join`'s left
side — narrows its candidate set through one declared *equality* index
when the `WHERE` clause carries an `Eq` on a `filter_eq: true` field,
and re-checks every predicate over what comes back. Every *ordering*
predicate (`Lt`/`Le`/`Gt`/`Ge`) still plans `FullScan`:
`docs/FUTURE-GROWTH.md` names it — "a range path through the `Ordered`
index" — as the next thing absent, and step one's Non-goals scoped it
out explicitly as "a second, distinct planner step with its own
trait-surface question."

Yet `Memory` and `Relation` already hold exactly the structure a range
predicate wants: `Ordered<S, R, Marker>` (`ADR-0059`,
`src/generic/store.rs`) keeps a `BTreeSet<(key, id)>` over
`updated_at_unix_ms`, exact through every insert, replace, delete, and
re-keying update, and answers `PageBy::page_by` as
`self.index.range((Excluded(cursor), Unbounded)).take(limit)`. A `WHERE
updated_at_unix_ms > 1700000000000` is the same walk without the
`take` — "one walk of the same set," as `ADR-0059`'s Consequences put
it. Today that index is reached only through `ConnectionStore::page`
(unfiltered `Page` ordered by that field); the identical predicate
inside a `Query`, an `Aggregate`, a `FilteredPage`, or a `Join` decodes
all *n* records and discards the ones outside the range.

This round takes exactly that step and no other: when a filter carries
an ordering predicate (or an `Eq`) on the one field an adapter reports
as range-indexed, the shared candidate step walks the `Ordered` index
between the predicate's bounds and reads only those records. Every
predicate is then re-checked over the fetched rows by the unchanged
consumer code, so the result *set* is identical to the full scan's by
construction — the index only narrows what is read, never what is
returned. The equality-index rule of steps one and two is unchanged
and takes priority, so **no request that planned `IndexEq` before this
round changes plan** — the range path applies only to requests that
planned `FullScan` before.

Scope, exactly: the "ids in key range" primitive `Ordered` lacks
(`QPR-FR-001`); the two `ConnectionStore` methods that let `dispatch`
learn which field is range-indexed and walk it (`QPR-FR-002`); the
third plan and its deterministic rule (`QPR-FR-003`); the candidate
fetch, bound mapping, and fallback (`QPR-FR-004`); the mandatory
re-check and the result-set invariant, proven per consumer and per
domain (`QPR-FR-005`); the observable differences, named
(`QPR-FR-006`); and zero change to every other surface (`QPR-FR-007`).

## Non-goals

- **A wire-visible "range-indexed" capability.** `FieldCapabilities {
  filter_eq, scan, update }` is `bincode`-serialized inside
  `Response::Schema`; adding a fourth flag is a protocol-version bump
  with a `downgrade_for_version` rewrite for every client negotiated
  below it, a `SERVER-002` wire-spec revision, new golden vectors, and
  a `clients/python` change — for a fact no client can act on (a client
  cannot ask for a plan). This round keeps the range-index declaration
  server-side, as a `ConnectionStore` method (`QPR-FR-002`), exactly as
  `filtered_page`/`page_keys` are server-side trait surface today. If a
  client ever needs to *see* it, that is the protocol round this Non-
  goal names; see "Considered options" for why it is not this one.
- **Descending walks / `ORDER BY … DESC`.** `range(..).rev()` is the
  same set walked backwards; nothing on the wire asks for it
  (`SERVER-PAGE-DESIGN.md`'s and `SERVER-ORDERED-INDEX-DESIGN.md`'s
  own standing open question). Untouched.
- **Bound tightening or index intersection.** A filter with two lower
  bounds on the range field (`a > 1 AND a > 5`) walks from the *first*
  in wire order and re-checks the other; a filter with an eligible `Eq`
  *and* a range predicate takes the equality index and re-checks the
  range. No "which bound is tighter", no `IndexEq ∩ IndexRange` — the
  cost-model question steps one and two declined, declined again.
- **A cost model or statistics.** The rule is "walk the range whenever
  the adapter says the field is range-indexed," full stop. Always at
  most as many reads as the full scan (*k* ids in the key range,
  *k* ≤ *n*); no win when the bound admits nearly every record.
- **The O(page) bounded walk for `FilteredPage`.** A `FilteredPage`
  whose `order_by` *is* the range field and whose filter is *only*
  bounds on it (`WHERE updated_at > since ORDER BY updated_at LIMIT 50`
  — the `since`-shaped sync listing) could be one `range_by` walk from
  `max(cursor, lower)` with `take(limit)`, touching only the page. This
  round's shared candidate step reads every record in the range and
  then lets `page_rows` sort and cut — *k* reads where the walk would
  do 50. Real, bounded, and the natural next slice; offered as option
  (b), not bundled into (a), so that (a) changes one helper and proves
  it, not one helper and two adapter overrides.
- **A second `Ordered` wrap** (`created_at_unix_ms`, say). One field
  per stack (`ADR-0059`'s own Non-goal); `range_field` returns at most
  one tag for the same reason. A second wrap is a second round, and the
  `Option<FieldRef>` becomes a `Vec` then, not now.
- **`Reminder::due_at_unix_ms`.** Its index is the generic `HashMap`
  equality index (`filter_eq: true`), not an `Ordered` wrap; a `WHERE
  due_at_unix_ms < now` stays a full scan. Wrapping `Reminder` in
  `Ordered` is exactly `ADR-0059`'s "a wrap each, if a caller wants
  one" — not asked for.
- **`WHERE id = …`, `ORDER BY` with `JOIN`/`GROUP BY`, `OR`,
  parentheses.** Unchanged from every prior SQL round's Non-goals.
- **Client or wire change.** `src/server/sql.rs`, `SchemaDrivenClient`,
  `clients/python`, `protocol.rs`, `SERVER-002`: all untouched. Every
  comparator the grammar already parses (`SQL-FR-001`: `=`, `!=`, `<`,
  `<=`, `>`, `>=`) reaches the server exactly as before; the server
  simply answers some of them faster.
- **The `Ordered` file format, rebuild, or persistence.** Memory-only,
  rebuilt at open, one field per stack — all `ADR-0059`, all unchanged.

## Context and terminology

Read from `src/generic/{traits,query,store,production,memory,relation}.rs`
and `src/server/{serve,protocol,memory,relation}.rs` at `main`
`2d5e5f1fa` (post-`ADR-0074`, `SERVER-001` v0.59.0, protocol 27) during
this pass, not assumed:

- **`Ordered<S, R, Marker>`** (`src/generic/store.rs`): `inner: S`,
  `index: BTreeSet<(R::Key, R::Id)>`, built at wrap time from
  `inner.all_ids()` + `get`, kept exact by its own `Insert`/`Replace`/
  `Delete`/`UpdateField` impls (`ORD-FR-003`); every other trait
  forwarded. Its only reader is `impl PageBy<R, Marker> for Ordered`:
  `page_by(after, limit)` = `self.index.range((Excluded(cursor),
  Unbounded)).take(limit)` (or `iter().take(limit)` for no cursor).
  `R::Key: Ord + Copy` (`OrderedField`); `R::Id: Ord` required by the
  impl. Exactly two stacks wrap in it: `MemoryProductionStack =
  Ordered<MultiSymmetric<GenericMmapStore<Memory, CategoryField,
  AccessCountField>, Memory>, Memory, UpdatedAtOrder>` and
  `RelationProductionStack = Ordered<GenericMmapStore<Relation,
  SubjectField, UpdatedAtField>, Relation, UpdatedAtField>`. In both,
  `Ordered` is the *outermost* layer, so no other layer forwards
  `PageBy` and none will need to forward the new trait either
  (`grep "impl.*PageBy" src/generic/` finds the one impl).
- **`GenericProductionStore::page_by<R, Marker>`**
  (`src/generic/production.rs`): `self.inner.read().expect(LOCK_POISONED)
  .page_by(after, limit)` where `S: PageBy<R, Marker>` — one read-lock
  acquisition per call. The new accessor is its twin.
- **`BTreeSet::range`** panics "if range `start > end`" or "if `start
  == end` and both bounds are `Excluded`" (std docs). A range built
  from two client-supplied predicates (`a > 5 AND a < 3`) can be either;
  the new primitive must answer *empty*, never panic (`QPR-FR-001`).
- **The key/id pair, not the key, is what the set orders by.** A bound
  on a *key* alone must be expressed as a bound on a *pair*: "every
  pair with key ≥ *k*" is `Included((k, MIN_ID))`; "every pair with key
  > *k*" is `Excluded((k, MAX_ID))`; "≤ *k*" is `Included((k, MAX_ID))`;
  "< *k*" is `Excluded((k, MIN_ID))`. `R::Id` is generic (`Ord + Copy`
  only — no `MIN`/`MAX`), while both shipped ids are `Uuid`
  (`Uuid::nil()`/`Uuid::max()`). So the generic primitive takes bounds
  on *pairs* and stays sentinel-free; the adapter, which knows its id
  type, supplies the sentinels (`QPR-FR-001`/`QPR-FR-002`).
- **`Predicate { field, op: CompareOp, value: ScanValue }`**
  (`src/server/protocol.rs`): `CompareOp::{Eq, Ne, Lt, Le, Gt, Ge}`;
  `is_ordering()` is `Lt | Le | Gt | Ge`. `validate_predicate`
  (`SQL-FR-007`) already refuses an ordering comparator on a field that
  is not `U32`/`I64` and a literal of the wrong kind, before any read —
  so by the time a plan is chosen, a predicate on the range field is
  guaranteed to carry an `I64` literal (both range fields are `I64`).
- **The planner today** (`serve.rs`): `enum QueryPlan { FullScan,
  IndexEq(usize) }`; `plan_query(schema, filter)` = the first `Eq` on a
  `filter_eq: true` field, else `FullScan`; `query_candidates(store,
  plan, filter)` = `scan_all()` or `filter_eq` + per-id `get` with
  `Err(_) => scan_all()`; `indexed_candidates(store, schema, filter)` =
  `query_candidates(store, plan_query(schema, filter), filter)`, called
  from the `Query` arm, the `Aggregate` arm, the `filtered_page` trait
  default, and `evaluate_join`'s left loop. Every caller re-checks
  every predicate over the result (`QPL-FR-003`, `QPC-FR-001`).
- **The two range fields, as the adapters describe them today**:

  | Adapter | Field | Tag | `ValueKind` | `filter_eq` | `scan` | `update` | `Ordered` marker |
  |---|---|---|---|---|---|---|---|
  | `Memory` | `updated_at_unix_ms` | `FIELD_UPDATED_AT` = 6 | `I64` | `false` | `false` | `false` | `UpdatedAtOrder` (order only) |
  | `Relation` | `updated_at_unix_ms` | `FIELD_UPDATED_AT` = 4 | `I64` | `false` | `true` | `true` | `UpdatedAtField` (scannable **and** ordered — one marker, `ADR-0059`'s rule) |

  Neither is `filter_eq: true`, so an `Eq` on either plans `FullScan`
  today and is eligible for the range rule (a one-key walk) under this
  round. Every other domain (`Dog`, `Order`, `Employee`, `Reminder`,
  `Entity`) has no `Ordered` wrap and reports no range field.
- **`ConnectionStore::page` already does this walk, unfiltered**:
  `MemoryConnectionStore::page` (`ORD-FR-005`) matches `order_by ==
  FIELD_UPDATED_AT`, maps the `ScanValue::I64` cursor to `(i64, Uuid)`,
  calls `self.store.page_by::<Memory, UpdatedAtOrder>(cursor, limit)`,
  then `get`s each id. The new `range_ids` is the same shape with two
  bounds and no `take`. `Relation`'s is identical over `UpdatedAtField`.
- **Measured precedent**: `ADR-0059` measured the unfiltered `Page` at
  152–160 µs per page of 50 flat from 1K to 100K records once the walk
  replaced the scan; `ADR-0073`/`0074` measured the equality path at
  ~420–580 µs for a 1,000-row answer vs. ~100 ms for the scan at 100K.
  A range predicate selecting 1% of the table pays the latter shape
  today. This design-only round makes no new measurement; the
  implementation round must (acceptance criterion 6).

## Requirements

- `QPR-FR-001` **The range primitive.** A new trait in
  `src/generic/query.rs`, beside `PageBy`:

  ```rust
  pub trait RangeBy<R, Marker>
  where
      R: OrderedField<Marker>,
  {
      /// Every id whose `(key, id)` pair lies within `lower..upper`,
      /// ascending by `(key, id)` — `page_by` without the cursor-only
      /// lower bound and without the `take`. An inverted or empty
      /// range answers an empty `Vec`, never a panic.
      fn range_by(
          &self,
          lower: Bound<(R::Key, R::Id)>,
          upper: Bound<(R::Key, R::Id)>,
      ) -> Vec<R::Id>;
  }
  ```

  Implemented by `Ordered<S, R, Marker>` (`R::Id: Ord`) as one
  `self.index.range((lower, upper))` walk, guarded: if the bounds
  describe an empty set in the way `BTreeSet::range` would panic on —
  start pair > end pair, or equal pairs both `Excluded` — the impl
  returns `Vec::new()` before calling `range`. Exposed on
  `GenericProductionStore` as `range_by<R, Marker>(&self, lower,
  upper) -> Vec<R::Id> where S: RangeBy<R, Marker>`, one read-lock
  acquisition, `page_by`'s twin. `PageBy` and `page_by` are untouched
  — not re-expressed over the new trait, so `ADR-0059`'s measured path
  is byte-for-byte what it was.
- `QPR-FR-002` **Two `ConnectionStore` methods, defaulted.**
  `fn range_field(&self) -> Option<FieldRef> { None }` — the one field
  this adapter keeps an `Ordered` index over, if any; and
  `fn range_ids(&self, field: FieldRef, lower: Bound<ScanValue>,
  upper: Bound<ScanValue>) -> Result<Vec<RecordId>, ErrorCode> {
  Err(ErrorCode::Unsupported) }` — every id whose stored `field` value
  lies within the bounds, ascending by `(value, id)`. `Memory` and
  `Relation` override both: `range_field` answers
  `Some(FIELD_UPDATED_AT)`; `range_ids` answers `Unsupported` for any
  other tag, `Malformed` for a bound whose value is not
  `ScanValue::I64`, and otherwise maps each key bound to a pair bound
  with `Uuid::nil()`/`Uuid::max()` as in "Context" and calls
  `self.store.range_by::<Memory, UpdatedAtOrder>` (resp.
  `<Relation, UpdatedAtField>`). The defaults mean the four other
  adapters and `Dog` change by zero lines. The pair (declare + walk) is
  the trait-surface answer to step one's open question: server-side,
  like `page_keys`/`filtered_page`, not wire-side.
- `QPR-FR-003` **Plan choice, deterministic, equality first.**
  `QueryPlan` gains `IndexRange { lower: Option<usize>, upper:
  Option<usize> }` — positions in `filter` of the predicates that
  supply each bound, at least one `Some`. `plan_query` takes one more
  input, `range_field: Option<FieldRef>` (`store.range_field()`, read
  once per request beside `describe()`), and chooses, in this order:
  (1) `IndexEq(i)` by `QPL-FR-001`'s rule, unchanged; else (2) if
  `range_field == Some(f)`: `lower` = the first predicate in wire order
  with `field == f` and `op ∈ {Gt, Ge, Eq}`, `upper` = the first with
  `field == f` and `op ∈ {Lt, Le, Eq}` (one `Eq` may serve as both);
  `IndexRange { lower, upper }` if either is `Some`; else (3)
  `FullScan`. `Ne` on the range field never contributes a bound. The
  choice depends on nothing else — not table size, not the literals,
  not any prior request. Consequence, stated as a requirement: a
  `filter` that plans `IndexEq` under `QPL-FR-001` today plans
  `IndexEq` after this round; only filters that plan `FullScan` today
  can plan `IndexRange`.
- `QPR-FR-004` **Candidate fetch, bound mapping, fallback.**
  `query_candidates` handles `IndexRange` by mapping each supplying
  predicate's `op` to a `Bound<ScanValue>` over its `value` —
  lower: `Gt → Excluded`, `Ge → Included`, `Eq → Included`; upper: `Lt
  → Excluded`, `Le → Included`, `Eq → Included`; an absent side is
  `Unbounded` — then `store.range_ids(f, lower, upper)`; on `Ok(ids)`
  it reads `store.get(id)` for each id in the returned (ascending) order,
  keeping every `Some` and skipping every `None` (the identical drop
  `scan_all` and `IndexEq` already perform); on `Err(_)` (an adapter
  whose `range_field` and `range_ids` disagree — none shipped) it
  executes `FullScan` and returns the correct result with no client-
  visible error, `QPL-FR-004`'s shape exactly. `indexed_candidates`
  becomes `query_candidates(store, plan_query(schema,
  store.range_field(), filter), filter)`, so `Query`, `Aggregate`, the
  `filtered_page` default, and `Join`'s left side all gain the path
  through the one helper they already share — no call site changes.
- `QPR-FR-005` **Every predicate is re-checked; result-set equality
  proven per consumer and per domain.** Whichever plan produced the
  candidate rows, they pass through the unchanged `evaluate_query` /
  `evaluate_aggregate` / `filtered_page`'s filter + `page_rows` /
  `evaluate_join`'s `left_filter` — including the bound predicates
  themselves. For every request on `Memory` and `Relation`, the
  multiset of rows (groups, page rows, join pairs) returned under
  `IndexRange` equals the multiset the same request returns under
  `FullScan` with `limit: None`, at any instant with no concurrent
  write. Proven, not asserted: integration tests per domain issue each
  of `>`, `>=`, `<`, `<=`, a two-sided range, and `=` on the range
  field over a real socket, cross-checked against the full scan's own
  filtered result — using `created_at_unix_ms`, which the fixtures set
  to the *same* value per record and which is never range-indexed, as
  the equal-selectivity `FullScan` control, the `category`/`source`
  device of `ADR-0073`'s bench — before and after a runtime `Insert`
  into the range, a `Replace`/`ReplaceIf` that re-keys a record across
  a bound (in and out), and a `Delete` of an in-range record; plus an
  inverted range (`> 5 AND < 3`) and an empty both-excluded range
  (`> 5 AND < 5`) returning zero rows with no error; plus the
  `Aggregate` tally and the `FilteredPage` sequence over a range
  filter.
- `QPR-FR-006` **Observable differences, named.** (1) Rows for a
  range-planned `Query` (and pairs for a `Join`, groups for an
  `Aggregate`) now arrive in ascending `(updated_at, id)` order where
  they arrived in `all_ids` order before — both "unspecified order"
  under `SQL-FR-004`/`SQL-FR-006`/`AGG`/`JOIN`'s existing contracts,
  and a client must not rely on the new order either. (2) With `limit:
  Some(n)` and more than *n* matches, *which* *n* may differ from
  before, inside the same "truncates that same unspecified order"
  wording. (3) `FilteredPage`: **zero** observable difference —
  `page_rows` orders the filtered set by `(key, id)` before returning
  it (`QPC-FR-006`'s finding, unchanged). Recorded in `SERVER-001`'s
  change history at implementation time.
- `QPR-FR-007` **Everything else unchanged.** No `Request`/`Response`/
  `ErrorCode` variant; no `PROTOCOL_VERSION` bump; `FieldCapabilities`,
  `FieldDescriptor`, `DomainSchema`, `describe()` untouched on every
  adapter; `src/server/sql.rs`, `src/server/client.rs`,
  `clients/python/**`, `SERVER-002` untouched; `PageBy`/`page_by`/
  `ConnectionStore::page`/`page_keys` untouched; the `Ordered` index's
  contents, rebuild, and maintenance untouched; `Dog`, `Order`,
  `Employee`, `Reminder`, `Entity` plan exactly as before (their
  `range_field` is `None`) and are byte-for-byte unaffected.

## Considered options

- **(a) The range candidate step, through the shared helper, for
  every consumer — recommended, as scoped above.** One trait + one
  impl + one accessor in `src/generic/`, two defaulted trait methods,
  two adapter overrides, one plan variant; `Query`/`Aggregate`/
  `FilteredPage`/`Join` all narrow at once through `indexed_candidates`
  with no call-site change; result set identical by construction
  (`QPR-FR-005`); consistency class identical to today; equality-first
  ordering means no previously-indexed request changes plan. Real,
  named costs: two order-level differences on `Query`/`Aggregate`/
  `Join` (`QPR-FR-006`); a bound that admits most of the table gains
  nothing (*k* ≈ *n* reads plus a tree walk); and `FilteredPage`'s
  `since`-shaped listing still reads every in-range record before
  `page_rows` cuts the page.
- **(b) (a) plus the O(page) bounded walk for `FilteredPage` on
  `Memory`/`Relation`.** Override `filtered_page` in both adapters for
  the case `order_by == range_field` and every predicate is a bound on
  that field: one `range_by` walk from `max(after-cursor, lower)` to
  `upper` with `take(limit)`, `get` only the page, re-check the bounds
  (still needed — the cursor and the bound compose). `WHERE updated_at
  > since ORDER BY updated_at LIMIT 50` then costs the page, the
  `Page` number `ADR-0059` measured, instead of *k* reads. Larger:
  two adapter overrides with their own cursor/bound composition logic
  and their own sequence-identity proofs against the default; the
  natural next slice once (a) has proven the primitive.
- **(c) Decline.** Ordering predicates stay full scans;
  `docs/FUTURE-GROWTH.md`'s "a range path through the `Ordered`
  index" line stays exactly as written. Zero risk. A caller wanting
  the walk can already issue `Page` with a cursor and stop reading when
  the key passes its upper bound — client-side, one page per round
  trip — which is precisely what (a) automates server-side, unbounded.

Why a `ConnectionStore` method pair rather than a `FieldCapabilities`
flag, weighed and not taken: the flag is the more "schema-driven"
answer in spirit (`QPL-FR-001` plans from `describe()` alone), and
step one's open question phrased it that way. But `FieldCapabilities`
is wire shape; a fourth `bool` re-encodes every `Response::Schema`,
which is protocol 28, a `downgrade_for_version` content rewrite (the
`StrList` precedent, `ADR-0041`), a `SERVER-002` revision, new golden
vectors in `tests/fixtures/wire-vectors.txt`, and a Python-client
parse change — for a fact that changes nothing a client can send. The
method pair keeps the planner's determinism (`plan_query` still plans
from `(schema, range_field, filter)` alone, pure and unit-testable)
without any of that; if a client ever needs to *see* the flag, the
method's answer is what the flag would carry, and that round is
additive on top of this one.

The owner's shorthand: **(a)** the range step for every consumer, as
proposed; **(b)** (a) plus the O(page) `FilteredPage` walk on
`Memory`/`Relation`; **(c)** decline.

## Proposed shape

`src/generic/query.rs`: `pub trait RangeBy<R, Marker>` as in
`QPR-FR-001`, documented beside `PageBy` as its two-bound, no-limit
twin. `src/generic/store.rs`: `impl<S, R, Marker> RangeBy<R, Marker>
for Ordered<S, R, Marker> where R: OrderedField<Marker>, R::Id: Ord`
— the guarded `self.index.range((lower, upper))` walk, about ten lines
below the existing `PageBy` impl, plus one unit test beside
`page_by`'s. `src/generic/production.rs`: `pub fn range_by<R, Marker>`
beside `page_by`, one read-lock acquisition. This is the third time
`crate::generic` is touched after `STORAGE-012` closed it, and the
first since `ADR-0059` added `OrderedField`/`PageBy`/`Ordered` under
`SERVER-001` FR-059 — the same precedent, followed the same way:
additive, recorded here, registered under `SERVER-001`'s next FR
rather than a new storage spec.

`src/server/serve.rs`: `range_field`/`range_ids` on `ConnectionStore`
with the defaults in `QPR-FR-002`, documented beside `page`/
`page_keys`/`filtered_page`; `QueryPlan::IndexRange { lower, upper }`;
`plan_query(schema, range_field, filter)` per `QPR-FR-003` (the
existing `IndexEq` arm first, verbatim); `query_candidates`'s
`IndexRange` arm per `QPR-FR-004` with a small private `fn
range_bounds(filter, lower, upper) -> (Bound<ScanValue>,
Bound<ScanValue>)` for the op-to-bound mapping, unit-tested on its
own; `indexed_candidates` threads `store.range_field()`. `PlannerFixture`
gains a configurable range field and an in-memory sorted index (or a
refusal), so the plan choice, the bound mapping, the re-check, and the
`Err(_)` fallback are each unit-testable without a real domain, as the
equality path's are.

`src/server/memory.rs` and `src/server/relation.rs`: the two overrides
per `QPR-FR-002`, each about fifteen lines, mirroring the existing
`page` override's `order_by == FIELD_UPDATED_AT` / `ScanValue::I64`
match, plus one private `fn pair_bounds(lower: Bound<ScanValue>, upper:
Bound<ScanValue>) -> Result<(Bound<(i64, Uuid)>, Bound<(i64, Uuid)>),
ErrorCode>` shared by both (in `serve.rs`, `pub(crate)`, since the
mapping is id-type-specific to `Uuid` and both adapters use it).

Nothing else changes. The `Query`/`Aggregate` arms, the `filtered_page`
default body, and `evaluate_join`'s left loop already call
`indexed_candidates` and keep calling it.

## Data/state and invariants

- **Result-set invariant** (`QPR-FR-005`): for a `filter` with no
  concurrent writer, `IndexRange` and `FullScan` produce the same
  multiset of rows before `limit`. Holds because (i) `Ordered`'s index
  holds exactly one `(order_key, id)` pair per live record, equal to
  the record's stored field value at all times (`ORD-FR-003`: insert
  adds, replace swaps, delete removes, a same-marker `update` re-keys
  — and `Relation`'s one-marker rule is what makes the `update` case
  exact), so `range_ids` returns *exactly* the ids whose stored value
  satisfies the bound predicates — a superset of nothing and a subset
  of nothing, for those predicates; (ii) every consumer re-applies
  every predicate, so the other predicates filter exactly as before;
  (iii) `range_by`'s pair-bound mapping is exact for every key: with
  `MIN_ID = Uuid::nil()` ≤ every id ≤ `MAX_ID = Uuid::max()`,
  `Included((k, MIN))` admits every pair with key ≥ *k* and no pair
  with key < *k*; `Excluded((k, MAX))` admits every pair with key > *k*
  and none with key ≤ *k*; and symmetrically for the upper side. A
  `Uuid` equal to `nil()` or `max()` as a real record id is handled
  correctly by construction (`Included` admits it; `Excluded((k,
  MAX))` excludes the pair `(k, MAX)` itself, which has key = *k*, not
  > *k* — correct).
- **Consistency class, unchanged**: the range walk and each `get` are
  separate lock acquisitions, exactly as `all_ids`/`filter_eq` and
  each `get` are today. A concurrent `Delete` between them drops the
  row (as today); a concurrent `Replace`/`UpdateField` that moves a
  record's `updated_at` *out* of the range between the walk and its
  read is caught by the re-check and dropped; one that moves a record
  *into* the range after the walk is missed (as the full scan would
  miss a record inserted after `all_ids`). No new anomaly introduced,
  none removed.
- **Session posture, unchanged**: every consumer "always reads committed
  state" (`SQL-FR-009`; the read-your-writes / snapshot / MVCC
  intercepts key on `GetById` alone), and `store.get` here is the
  adapter's own committed read — identical on every plan.
- **Empty and inverted ranges are data, not errors**: `> 5 AND < 3`
  yields start pair `(5, MAX)` > end pair `(3, MIN)` → `range_by`
  answers empty; `> 5 AND < 5` yields `Excluded((5, MAX))` and
  `Excluded((5, MIN))`, start > end → empty; `>= 5 AND <= 5` yields
  `Included((5, MIN))..=Included((5, MAX))` → exactly the key-5
  records. `validate_predicate` has already passed each predicate
  individually, so a contradictory pair reaches the planner and must
  come back as zero rows, never a panic (`QPR-FR-001`'s guard) — and
  the full scan's own answer for that filter is zero rows too, so the
  invariant holds trivially.
- **Plan priority is a compatibility property**: `IndexEq` before
  `IndexRange` is not a selectivity claim (an equality bucket can be
  larger than a narrow range); it is what guarantees that every request
  which took the equality index under `ADR-0073`/`0074` still does, so
  this round's blast radius is exactly "requests that full-scanned
  before." Named in `QPR-FR-003`.

## Errors, failure, recovery, and observability

No new `ErrorCode`, no new client-visible failure: every rejection any
of the four consumers can produce still comes from validation before
any read (`SQL-FR-007` and its per-request twins), and a planner-
internal `range_ids` refusal is absorbed by the `FullScan` fallback
(`QPR-FR-004`). The `Malformed` and `Unsupported` arms `range_ids`
itself can answer are unreachable through `dispatch` — the tag is the
adapter's own `range_field` and the literal kind was validated — and
exist so the method is honest as trait surface, not because `dispatch`
can trigger them. Lock poisoning panics exactly where it does today.
Nothing to recover — a read throughout.

Observability: the plan taken is still not reported on the wire, in the
access log, or in `Request::Metrics` — step one's standing open
question, now three plans wide; still named, still not bundled.

## Security, privacy, and compatibility

A read, gated exactly as each consumer is today. The range path reads
strictly no more records than the scan and returns the same set, so no
record a client could not already obtain becomes obtainable. No wire
format change; no protocol-version bump; a pre-existing client of any
version observes only `QPR-FR-006`'s order-level differences. Audit and
access-log entries are unchanged — one request is one entry regardless
of plan. A client cannot request or observe the plan.

## Acceptance criteria

1. `Ordered::range_by` unit test (`src/generic/store.rs`): over a
   set with duplicate keys and distinct ids, each of `Included`/
   `Excluded`/`Unbounded` on each side returns exactly the expected
   ascending ids; an inverted range and a both-`Excluded` equal range
   return empty without panicking; `page_by`'s existing test is
   untouched and still passes.
2. `plan_query` unit tests: with `range_field: None`, every ordering
   predicate plans `FullScan` (today's behavior, pinned); with
   `Some(f)`: `f > v` → `IndexRange { lower: Some(0), upper: None }`;
   `f < v` → `{ None, Some(0) }`; `f >= a AND f <= b` → `{ Some(0),
   Some(1) }`; `f = v` → `{ Some(0), Some(0) }`; `f != v` → `FullScan`;
   `g > v` (not the range field) → `FullScan`; `f > 1 AND f > 5` →
   `lower: Some(0)`; an eligible `Eq` on an indexed field plus `f > v`
   → `IndexEq` of the equality (priority); an unindexed predicate
   before the bound → the bound's own position.
3. `range_bounds` unit tests: each of the five ops maps to the stated
   `Bound`; an absent side is `Unbounded`.
4. `query_candidates` against `PlannerFixture`: `IndexRange` reads only
   the in-range ids (asserted via the fixture's `get`/`scan_all`
   counters); a fixture range index returning `Err(Unsupported)`
   despite a declared `range_field` falls back to the full scan with
   the identical result and no error; and one `dispatch`-level test per
   consumer (`Query`, `Aggregate`, `filtered_page` default, `Join`)
   asserting the same rows/groups/sequence/pairs on either plan — the
   `ADR-0074` per-consumer shape.
5. Integration (`tests/server_sql_integration.rs` and
   `tests/server_memory_integration.rs`, real socket, real SQL text via
   `SchemaDrivenClient::query`): `QPR-FR-005`'s per-domain proof for
   `Memory` and `Relation` — `>`, `>=`, `<`, `<=`, two-sided, `=` on
   `updated_at_unix_ms` cross-checked against the `created_at_unix_ms`
   control and the unfiltered scan, before and after runtime `Insert`,
   re-keying `Replace`/`ReplaceIf`, and `Delete`; the inverted and
   both-excluded ranges answering zero rows; `COUNT(*)`/`MIN`/`MAX`
   grouped over a range filter equal to the tally over control rows;
   `WHERE updated_at_unix_ms > v ORDER BY created_at_unix_ms LIMIT n`
   (`FilteredPage`) as the identical sequence, cursor included; a
   `JOIN` whose left `WHERE` is a range on `Memory::updated_at_unix_ms`
   returning the same pair set as the unfiltered join filtered by hand;
   and one `Dog`/`Entity` control showing an ordering predicate on a
   domain with no range field still returns the full scan's rows.
6. Every pre-existing test passes unmodified (checked this design
   pass: no test in the crate pins `Query`/`Aggregate`/`Join` row order
   on an ordering predicate over `updated_at_unix_ms`; the `FilteredPage`
   tests pin sequences, which `QPR-FR-006` (3) guarantees unchanged).
7. Measured, recorded in `RESULTS.md` (the `ADR-0073`/`0074`
   precedent): `benches/server.rs`'s `memory-planner` harness — whose
   fixture already sets `created_at_unix_ms = updated_at_unix_ms = n`
   for record *n* of 100,000 — gains a two-sided 1%-selective range
   (`updated_at_unix_ms >= 50000 AND updated_at_unix_ms < 51000`, 1,000
   rows) under `IndexRange` versus the identical bounds on
   `created_at_unix_ms` (identical values, never range-indexed) as the
   `FullScan` control, for `query`, `count(*)`, and `fpage-50`; the
   implementation round reports the numbers — this design-only round
   predicts the shape (O(log *n* + *k*) reads vs. O(*n*)) and nothing
   more.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`
(the `generic/store.rs` and `serve.rs` unit tests plus
`tests/server_sql_integration.rs` and `tests/server_memory_integration.rs`,
extended); `cargo bench --bench server` for criterion 7. Independent
review: this design is written by Claude while Codex's sandbox still
cannot spawn processes on this machine (unresolved since 2026-09-17);
per this crate's host-takeover convention a fresh Codex session should
review both this design and its implementation once that is restored,
and the implementation round's own inspection must not cite this
session as having covered it.

## Traceability

- Roadmap: `SERVER-QUERY-PLANNER-RANGE-DESIGN`,
  `SERVER-QUERY-PLANNER-RANGE`.
- Decision: `ADR-0075`.
- Specification: `SERVER-001`'s next minor / FR at implementation
  (extends `FR-070`/`FR-071`'s planner; the `src/generic/` additions
  registered under the same FR, `ADR-0059`/FR-059's precedent).
- Requirements: `QPR-FR-001`–`007`.

## Open questions

- **The O(page) `FilteredPage` walk** — option (b); if (a) is picked,
  the next slice. *Taken: `ADR-0076` (2026-09-20), the round after this
  one — `docs/design/SERVER-FILTERED-PAGE-WALK-DESIGN.md`.*
- **A wire-visible range capability** — the `FieldCapabilities` flag,
  when a client has a reason to see it; a protocol round.
- **Descending walks** — `ORDER BY … DESC` on the wire, then
  `range(..).rev()`; unchanged from `ADR-0055`/`0059`.
- **Bound tightening / index intersection** — the first cost-model
  question with a concrete shape (`a > 1 AND a > 5`; `IndexEq ∩
  IndexRange`); declined three rounds running.
- **Plan metrics** — a per-plan counter in `ServerMetrics`, now three
  plans wide; still named, still not bundled.
- **A second `Ordered` wrap** (`created_at_unix_ms`) and `Reminder`
  under `Ordered` — each a wrap and a `range_field` change, if a
  consumer asks.

## Change history

- 2026-09-20: initial proposal, design only. Selected by the owner as
  the next round from a four-way survey of `docs/FUTURE-GROWTH.md`'s
  remaining items (the `Ordered` range path, MVCC + journal on one
  table, metrics depth, `WHERE id = …`) — the smallest step that
  directly continues `ADR-0073`/`0074`'s planner line and closes the
  first of step one's two named open questions. Every claim about the
  current code above was read from `main` at `2d5e5f1fa` during this
  pass, including the `BTreeSet::range` panic conditions that make
  `QPR-FR-001`'s guard necessary and the `R::Id`-has-no-`MIN`/`MAX`
  fact that puts the sentinel mapping in the adapter rather than the
  generic layer.
- 2026-09-20: the owner picked option (a) the same session, before the
  design PR (#273) merged; recorded here and in `ADR-0075`.
  Implementation to follow on its own branch — Claude as builder
  (Codex's sandbox still cannot spawn processes), with independent
  Codex inspection of both this design and the code owed once that is
  restored.
- 2026-09-20: implemented on `claude/planner-range-impl` as `SERVER-001`
  v0.60.0 / `FR-072`, exactly the "Proposed shape": `RangeBy` beside
  `PageBy` in `src/generic/query.rs`; `impl RangeBy for Ordered` in
  `src/generic/store.rs` (the guarded `BTreeSet::range` walk — the
  guard proven directly against the equal-and-both-`Excluded` input std
  panics on); `GenericProductionStore::range_by`; `range_field`/
  `range_ids` defaults, `QueryPlan::IndexRange`, `plan_query(schema,
  range_field, filter)`, `range_bounds`, the `IndexRange` arm of
  `query_candidates`, `indexed_candidates` threading `range_field`, and
  `uuid_pair_bounds`/`UuidPairBounds` in `src/server/serve.rs`; the two
  overrides in `src/server/memory.rs`/`relation.rs`. **No deviation.**
  Acceptance criteria 1–6 are the new tests: `generic/memory.rs` +1
  (`range_by_updated_at_walks_the_sorted_index_between_two_bounds`),
  `serve.rs` +6 (`plan_query_walks_a_range_only_after_the_equality_rule`,
  `range_bounds_maps_each_comparator_to_its_bound`,
  `query_candidates_range_path_reads_only_the_walked_ids`,
  `query_candidates_range_refusal_falls_back_to_a_full_scan_without_an_error`,
  `every_consumer_returns_the_same_result_on_the_range_walk_and_the_scan`,
  `uuid_pair_bounds_bracket_each_key_with_the_id_sentinels`; lib 614,
  up from 607), `tests/server_sql_integration.rs` +4 (51, up from 47),
  `tests/server_memory_integration.rs` +1 (15, up from 14); the full
  sweep 892 tests across 39 targets, 0 failed, every pre-existing test
  unmodified — `QPR-FR-006`'s anticipated order-pin relaxation was not
  needed. `fmt`/`clippy --features server,research --all-targets -D
  warnings` clean. Criterion 7: `benches/server.rs`'s `memory-planner`
  rows gained an `index-range` pair, recorded in `RESULTS.md` —
  `Query` 543.6 µs vs. 94,205.6 µs (~173×),
  `COUNT(*)` 400.6 µs vs. 84,703.1 µs (~211×),
  `FilteredPage` `LIMIT 50` 498.9 µs vs. 89,241.0 µs
  (~179×), over a real loopback socket. One doc change beyond
  the design's list: the `filtered_page` trait default's doc comment,
  which said the `Ordered` index "is still not consulted", now says
  what is and is not walked. Still no independent review — Codex's
  sandbox cannot spawn processes; owed.
