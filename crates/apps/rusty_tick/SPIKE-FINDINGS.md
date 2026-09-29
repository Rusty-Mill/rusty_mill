# rusty_tick storage spike: findings

Question: which of the gaps in [#382](https://github.com/Rusty-Mill/rusty_mill/issues/382)
actually block a task manager on `rusty_multimodal_db_engine`?

Method: `Task` record on `GenericMmapStore<Task, ByList, SortOrder>` with two
`Ordered` layers, plus full text and tags derived in memory at open
(`src/store.rs`); probes in `tests/spike.rs`. One machine, one run, release
build for the scale numbers.

## Verdicts

| # | Gap in #382 | Verdict | Evidence |
|---|---|---|---|
| 1 | One `IndexedField` per record | **Not a blocker.** Index `list_id` only; filter status/priority within the list. | `gap1_status_filter_needs_a_scan_within_the_list` |
| 2 | Multi-key filters | **Not a blocker for per-list ranges**: an `OrderedField` with `Key = (Uuid, i64)` gives per-list due-date and sort-order ranges. Intersecting two *different* queries (e.g. tag AND due range) is still done by hand and was not probed. | `gap2_due_range_per_list_via_composite_key` |
| 3 | Full-text search | **Confirmed gap**: no prefix matching, so type-ahead (`grocer` for `groceries`) finds nothing. Phrase search works. | `gap3_fulltext_phrase_and_missing_prefix` |
| 4 | Change feed for sync | **Not probed.** Needs its own design; see below. | none |
| 5 | Manual ordering | **Not a blocker**: `sort_order` as one marker that is both `ScannableField` and `OrderedField` gives an in-place, durable single-slot reorder. Gap-based (fractional) ordering is app-side. | `gap5_reorder_moves_one_task_and_survives_reopen` |
| 6 | Multi-user | **Partly probed**: reopen of 100k tasks is 0.88 s. Idle-store close/eviction not tested. | `scale_open_and_query_time` |

## New findings

- **F2, stacked `Ordered` layers hide the inner index.** With
  `Ordered<Ordered<Core, SortOrder>, DueAt>`, only the outer layer answers
  `RangeBy`; the inner one is reachable only through `.inner()`. It works
  but is easy to miss. Ask: forward `RangeBy` for other markers, or document it.
- **F3, per-write durability cost.** 100k inserts took 21.5 s (about 215 µs
  each, one durable write per insert). Fine for interactive use; bulk import
  should use the engine's group commit (not exercised here).
- **F4, derived indexes are memory-only.** Two `Ordered` indexes, the full-text
  index and the tag map are rebuilt at every open (0.88 s at 100k tasks). Fine
  for a personal instance; it bounds how many tasks a store can hold.
- **F5, cross-list smart lists cost one range query per list.** The `(list, due)`
  key that makes per-list ranges cheap does not serve "everything due today
  across all lists"; `Service::today` issues one range per non-archived list and
  merges. Fine for tens of lists. A second ordered index on due date alone would
  make it one query, at the price of another in-memory index.
- The scannable field must be a fixed-width value (`i64` here), and a field
  that is both scannable and ordered must use one marker for both.

## Scale numbers (100k tasks, 50 lists, release build)

| Operation | Time |
|---|---|
| Insert 100,000 (durable per write) | 21.5 s |
| Reopen (rebuild derived indexes) | 0.88 s |
| Due-range + list scan + full-text (100k hits) | 163 ms combined |

## Recommendation for #382

1. Close gaps 1, 2 and 5 as "supported today; recipe in `rusty_tick`".
2. Keep gap 3 (prefix and AND/NOT queries) as the one confirmed engine change.
3. Add F2 (forward `RangeBy` through stacked `Ordered`, or document `.inner()`).
4. Design the change feed (gap 4) with sync in mind before deciding whether it
   belongs in the engine or the app; the existing `Journal` sequence counters
   are the starting point.
5. Probe multi-user store eviction (gap 6) before a server milestone.

## Follow-up (2026-09-29): gaps 3 and 6

- **Gap 3, prefix, AND, NOT and column filters** now exist on the engine's
  `fulltext::Query` (`any_of_prefix`, `all_of`, `except`, `in_columns`) and are
  differentially tested against FTS5. `rusty_tick` can replace its whole-word
  search with `any_of_prefix` for type-ahead.
- **Gap 6, one store directory per user**, measured in `rusty_multimodal_db_engine`
  (`tests/store_lifecycle.rs`, release build, one machine, one run, records of a
  few dozen bytes with one equality index and one ordered index):

  | Records per store | Open | Close | Idle resident |
  |---|---|---|---|
  | 0 | 0.2 ms | 5 µs | 4 KiB |
  | 100 | 0.9 ms | 15 µs | 27 KiB |
  | 1,000 | 2.3 ms | 0.1 ms | 257 KiB |
  | 10,000 | 15.6 ms | 1.7 ms | 2.6 MiB |

  Open and memory are linear in the records (about 1.5 µs and 260 bytes each), so
  an LRU of open stores is cheap to build in the app: 1,000 idle users of 1,000
  tasks is about 250 MiB. Closing is dropping the store and its `DirLock`; every
  write was durable when it returned, so eviction loses nothing. The lock also
  refuses a second handle in the same process, so an idle store can be closed
  while nothing else has it open. The store does not lock by itself, so take the
  `DirLock` first. The pool stays app-side: which user to evict and when is policy.
  `src/pool.rs` is that pool (`StorePool`: least recently used closed first,
  `DirLock` per directory, validated user keys). Nothing calls it yet, because
  the API has one token and no user identity.
