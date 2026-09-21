# Server Reminder Due Index: `Reminder` Under `Ordered` (Proposed and implemented)

- Status: **Proposed and implemented on one branch** (2026-09-21,
  `ADR-0080`; the `ADR-0059`/`ADR-0076`–`ADR-0079` precedent — design
  and implementation in one PR). Selected under the owner's standing
  instruction to keep working the future-growth list, as the
  "`Reminder` under `Ordered` — a wrap and a `range_field` change, if a
  consumer asks" open question `ADR-0075` carried and the "a range on
  `Reminder::due_at_unix_ms` (an equality index, not an `Ordered` one)"
  gap `docs/FUTURE-GROWTH.md` names; the fork below stays open for the
  owner at review.
- Date: 2026-09-21
- Related: `ADR-0059`/`docs/design/SERVER-ORDERED-INDEX-DESIGN.md`
  (`Ordered`, `PageBy`; whose Non-goals named "`Reminder::due_at_unix_ms`
  … wrapping `Reminder` in `Ordered` is a one-line stack change and an
  adapter `page` override, when a consumer asks"), `ADR-0036`
  (`Reminder`, `RMD-FR-001`–`007`, whose `RMD-FR-003` chose the
  equality index for `due_at` and sent range search through a full
  scan "the actually common case"), `ADR-0075`–`ADR-0079` (the range
  path, the walks, the intersection, its budget — every one of which
  `Reminder` now reaches through `range_field`), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. Additive: `DueAtOrder` and an
  `OrderedField` impl on `Reminder`; `ReminderProductionStack` becomes
  `Ordered<ReminderCore, Reminder, DueAtOrder>` with `ReminderCore` the
  former alias; the three constructors wrap in `Ordered::new` and gain
  a portable-reopen twin; the adapter overrides `page`,
  `filtered_page`, `range_field`, `range_ids`, `range_ids_limited`. No
  file-format, wire, protocol-version, `FieldCapabilities`, or client
  change; `due_at_unix_ms`'s `filter_eq` index stays. Every request
  returns the identical set or sequence (`RDO-FR-004`).

## Purpose and scope

`Reminder` (`ADR-0036`) is the consumer's own domain, and its own spec
named the common query plainly: "range/ordering search (`due_at <
now`, the actually common case) goes through `Request::Query`/
`Request::Aggregate` instead, which already filter on any field" — a
full scan. Every planner round since (`ADR-0075`'s range path,
`ADR-0076`/`0077`'s page walks, `ADR-0078`/`0079`'s intersection)
reached `Memory` and `Relation` through one declaration,
`range_field`, and left `Reminder` on the scan for want of an `Ordered`
index. `ADR-0059` said what it would take: "a one-line stack change and
an adapter `page` override, when a consumer asks."

This round is that change. `ReminderProductionStack` gains an `Ordered`
wrap on `due_at_unix_ms`; the adapter declares it as its range field
and overrides `page`/`filtered_page` exactly as `Memory`'s does. Every
planner step lands at once: `WHERE due_at_unix_ms <= now` walks the
index; `… AND status = 0 ORDER BY due_at_unix_ms LIMIT 50` — the
"what is due" page — is the bounded walk past `status` rejects; `WHERE
due_at_unix_ms = k AND …` keeps the bucket, intersected with any bound
beside it under the budget.

Scope, exactly: the stack (`RDO-FR-001`); the adapter's range surface
(`RDO-FR-002`); the page walks (`RDO-FR-003`); identity, proven
(`RDO-FR-004`); the cost, measured (`RDO-FR-005`); no other surface
changed (`RDO-FR-006`).

## Non-goals

- **Dropping `due_at_unix_ms`'s equality index.** `DueAtField` stays;
  `filter_eq` on `due_at` is the bucket, as `RMD-FR-003` shipped it,
  and the planner's equality-first rule keeps it. A single-key range
  walk would answer the same ids; retiring the `HashMap` is a separate
  decision with its own memory measurement.
- **A `FieldCapabilities` flag for the range**, descending walks, a
  cursor on `Query`: unchanged from `ADR-0075`.
- **`Dog`, `Order`, `Employee`, `Entity` under `Ordered`**: each its
  own wrap, when a consumer asks — this round answers the one domain
  whose consumer's common query was named at its birth.

## Context and terminology

Read from `main` after PR #279 (`SERVER-001` v0.64.0) this pass:

- **`ReminderProductionStack`** was `GenericMmapStore<Reminder,
  DueAtField, StatusField>` directly; `create`/`open` returned it;
  the reminder integration test and the generic tests reopened it
  through `GenericMmapStore::open_portable`, an inherent method the
  wrapper does not have — hence a named
  `open_reminder_production_stack_portable`, `Memory`'s own shape.
- **`Ordered<S, R, Marker>`** forwards every trait the core provides
  (`Insert`/`Replace`/`Delete`/`UpdateField`/`FilterEq`/`ScanField`/
  `Flush`/`Compact`/…), so the adapter's writes, journal checkpoint,
  MVCC hooks, backup, and snapshot are untouched by the wrap; the
  index is rebuilt from the records at every open (`ADR-0059`'s named
  open-time cost) and tracks every write.
- **The adapter** had no `page` override (the scan path) and no range
  surface. It gains `Memory`'s five methods, over `DueAtOrder`.

## Requirements

- `RDO-FR-001` **The stack.** `DueAtOrder`; `impl OrderedField<DueAtOrder>
  for Reminder { type Key = i64; order_key = due_at_unix_ms }`;
  `ReminderCore = GenericMmapStore<Reminder, DueAtField, StatusField>`;
  `ReminderProductionStack = Ordered<ReminderCore, Reminder,
  DueAtOrder>`; `create_`/`open_reminder_production_stack` wrap the
  core in `Ordered::new`; `open_reminder_production_stack_portable(path)`
  new. No file written or read differently.
- `RDO-FR-002` **The range surface.** `range_field()` is
  `Some(FIELD_DUE_AT)`; `range_ids`/`range_ids_limited` walk the index
  between `uuid_pair_bounds` for `FIELD_DUE_AT` and refuse any other
  field; `page` by `FIELD_DUE_AT` is `page_by` from the cursor
  (`Malformed` for a non-`I64` cursor), any other field the scan path.
- `RDO-FR-003` **The page walks.** `filtered_page` is `Memory`'s
  override verbatim over `DueAtOrder`: the bounded walk when
  `bounded_walk_applies` (`FPW`/`FPM`), the default body otherwise.
- `RDO-FR-004` **Identity, proven.** Every request returns what it
  returned before: the generic stack's page and range walks in
  `(due_at, id)` order from a cursor, an insert landing by its stamp,
  a status update leaving the order, a portable reopen rebuilding it;
  the adapter's page equal to `page_by_scan`'s, its `filtered_page`
  equal to `filtered_page_by_candidates` over five shapes with one
  pinned; and over a real socket via SQL — the due-now listing with a
  `status` equality and inequality, one- and two-sided bounds, the
  `due_at` equality alone and intersected, with and without `LIMIT`,
  `COUNT(*)` beside each, ids pinned, through a `Transaction` status
  update. Every pre-existing test unmodified except the two portable
  reopens renamed to the new constructor.
- `RDO-FR-005` **The cost, measured.** `benches/server.rs` gains a
  `reminder-due` group over a 100K `Reminder` table (every fourth
  pending): the due-now page, a due count, and a narrow due window,
  each measured on the pre-change code (a full scan) first.
- `RDO-FR-006` **Everything else unchanged.** No wire, protocol
  (`PROTOCOL_VERSION` 27), `FieldCapabilities`, client, `sql.rs`, or
  journal/MVCC/backup change; `Memory`/`Relation` untouched; the
  reminder binary and the hub differential tests unchanged.

## Considered options

- **(a) `Ordered` on `due_at_unix_ms`, the equality index kept —
  implemented.** One wrap, five adapter methods; every planner round
  reaches `Reminder`.
- **(b) (a) and retire `DueAtField`'s `HashMap`**, answering `filter_eq`
  as a one-key range walk. Less memory at open, one fewer index to
  keep; a `GenericMmapStore` type-parameter change and a memory
  measurement. A second round.
- **(c) Decline.** The consumer's common query stays a full scan.

The owner's shorthand: **(a)** as implemented; **(b)** (a) plus retiring
the equality index; **(c)** decline and revert.

## Proposed shape

`src/generic/reminder.rs`: `DueAtOrder`, the `OrderedField` impl,
`ReminderCore`, the wrapped alias, the three constructors.
`src/server/reminder.rs`: `page`, `filtered_page`, `range_field`,
`range_ids`, `range_ids_limited`. `src/server/journal.rs`: the
`CheckpointFlush` impl's comment. `tests/server_reminder_integration.rs`:
the portable reopen. `benches/server.rs`: `start_reminder_planner_server`,
`bench_reminder_due`.

## Data/state and invariants

- **Index invariant**: one `(due_at_unix_ms, id)` pair per live
  record, maintained by `Ordered`'s forwarding of every write — the
  `ADR-0059` invariant, unchanged.
- **Open-time cost**: one decode per record to build the index, as
  `Memory` pays (~81 ms per 100K records, `ADR-0059`).
- **Consistency class**: `Page`'s for the walks, as on `Memory`.

## Errors, failure, recovery, and observability

No new `ErrorCode`. A memory structure: nothing to recover; a crash
costs the rebuild at the next open. The plan taken is not observable
on the wire.

## Security, privacy, and compatibility

A read path. No format, wire, or protocol change; every existing
directory reopens unchanged (the files are the core's); every client
observes only speed.

## Acceptance criteria

1. Generic (`src/generic/reminder.rs`): `page_by` from none and from a
   strict cursor in `(due_at, id)` order with a tie; `range_by` one-
   and two-sided; `range_by_limited` over budget; a status update
   leaves the order; a portable reopen rebuilds it.
2. Adapter (`src/server/reminder.rs`): `range_field`; `page` by
   `due_at` equal to the scan path's, from a cursor, `Malformed` for a
   wrong-kind cursor; `range_ids`/`range_ids_limited`; another field
   refused; `filtered_page` equal to the default body over five shapes,
   the `status`-reject walk pinned.
3. Every pre-existing test passes, the two portable reopens renamed.
4. Integration (`tests/server_sql_integration.rs`): the shapes in
   `RDO-FR-004` exact, `COUNT(*)` beside each, ids pinned, through a
   `Transaction` status update.
5. Measured (`RESULTS.md`): the `reminder-due` rows before and after.

## Verification plan

`cargo fmt -p rusty_multimodal_db -- --check`; `cargo clippy -p
rusty_multimodal_db --features server,research --all-targets -- -D
warnings`; `cargo test -p rusty_multimodal_db --features server,research`;
the `server` bench before and after. Independent review owed.

## Traceability

- Roadmap: `SERVER-REMINDER-DUE-INDEX`.
- Decision: `ADR-0080`.
- Specification: `SERVER-001` v0.65.0 / `FR-077` (extends `FR-039`'s
  `Reminder` and `FR-059`'s `Ordered`).
- Requirements: `RDO-FR-001`–`006`.

## Open questions

- **Retiring `DueAtField`'s `HashMap`** — option (b).
- **A decode-free count** — `COUNT(*)` whose every predicate is a bound
  on the walked field is the walk's length; today `Aggregate` decodes
  every candidate (the `due-count` row: half the table). A planner
  slice for `Aggregate` alone.
- **`Entity` under `Ordered`** (`mention_count`?) — when a consumer
  asks.

## Change history

- 2026-09-21: proposed and implemented on
  `claude/pr-276-multimodal-db-growth-4lkjx8` under the owner's
  standing "keep working the future-growth list" instruction. Read
  from `main` after PR #279 this pass, including that `Ordered`
  forwards every trait the journal, MVCC, backup, and snapshot paths
  need, so the wrap touches none of them, and that the one inherent
  method callers used on the bare core (`open_portable`) needs a named
  constructor. The `reminder-due` bench rows were added and measured
  on the pre-change code first.
- 2026-09-21: implemented on the same branch as `SERVER-001` v0.65.0 /
  `FR-077`, exactly the "Proposed shape". Acceptance criteria 1–4 are
  the tests: `src/generic/reminder.rs` +1, `src/server/reminder.rs` +1,
  `tests/server_sql_integration.rs` +1 (lib 627, up from 625; SQL
  55, up from 54); 909 tests across 39 targets, 0 failed.
  Criterion 5, `RESULTS.md`: the due-now page 53,278.9 → 121.9
  µs, the due count 52,913.6 → 22,312.4, the due window
  56,733.3 → 476.6. Still no independent review — owed.
