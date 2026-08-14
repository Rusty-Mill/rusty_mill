# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

Tracks changes by merged PR against `main`, reverse chronological, one
entry per PR.

---

## PR #71 — Add Connection::backup/restore (closes #22)
**2026-08-14** · [#71](https://github.com/baileyrd/rusty_-rusqlite/pull/71)

- **Added:** `Connection::backup`/`restore` — copies full table state
  between two connections, built on `serialize`/`deserialize` from #69.
- **Design, kept simple on purpose:** real `rusqlite::Connection::backup`
  (via `backup::Backup`/`Progress`/`StepResult`) copies incrementally,
  page by page, so a caller can observe/pause progress on a large file.
  This engine's storage has no page concept to step through, so `backup`
  is a single all-at-once copy — no `Backup`/`Progress`/`StepResult`
  types, since there's no multi-step operation for them to describe.
- 3 new unit tests (113 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #70 — Add scalar SQL functions (closes #18)
**2026-08-14** · [#70](https://github.com/baileyrd/rusty_-rusqlite/pull/70)

- **Added:** `Connection::create_scalar_function`/`remove_function`,
  callable from `WHERE` filters (e.g. `WHERE UPPER(name) = 'X'`).
- **Added:** `Expr::FunctionCall` to the `SELECT` parser's expression
  tree, plus parser support for `IDENT(args...)` call syntax.
- **Design, kept non-breaking on purpose:** rather than changing already-
  shipped `evaluate`/`evaluate_bool`/`execute_select`'s signatures (which
  would break every existing caller), added `evaluate_with_functions`/
  `evaluate_bool_with_functions`/`execute_select_with_functions`
  alongside them. The originals are now defined in terms of the new ones
  with an empty function registry, so there's one implementation, not two
  drifting copies — and `Expr::FunctionCall` reaching plain `evaluate`
  errors with `FunctionNotFound` rather than panicking or silently doing
  nothing.
- **Known limitation, stated plainly:** functions only work in `WHERE` —
  result-column projection with function calls (`SELECT UPPER(name)
  FROM t`) isn't supported, since `SelectColumns::Named` is a plain
  column-name list, not a list of expressions. Also unlike
  `rusqlite::Connection::create_scalar_function`, no `FunctionFlags` (no
  query planner here to use deterministic/innocuous markers) and a raw
  `Fn(&[Value]) -> Result<Value>` signature rather than one derived from
  `ToSql`/`FromSql`.
- **Caught by testing, not just reasoning:** a first draft of the
  "removed function is no longer found" test passed against an *empty*
  table, where the `WHERE` filter is never evaluated at all (so the
  removed function's absence was never actually exercised) — silently
  proving nothing. Fixed by inserting a row before asserting.
- 11 new unit tests (110 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #69 — Add Connection::serialize/deserialize (closes #17)
**2026-08-14** · [#69](https://github.com/baileyrd/rusty_-rusqlite/pull/69)

- **Added:** `Connection::serialize`/`deserialize`, backed by a
  hand-rolled binary encoding of `Database`'s table state
  (`src/serialize.rs`) — magic bytes, then length-prefixed tables,
  columns, and values.
- **Design deviation, stated plainly:** **not byte-compatible with real
  SQLite's file format** — this crate has no page/B-tree file format at
  all. Adding a real dependency (a serde-based crate) to build a more
  conventional format would be a new-dependency decision needing
  sign-off, so this is hand-rolled with only `std`. No
  `deserialize_bytes`/`deserialize_read_exact` split either — this
  crate's format has no ownership-transfer or partial-read story (real
  SQLite's does, tied to its C memory model) for those to distinguish.
- **Added:** `Database::tables`/`insert_table_raw` (read/raw-write access
  for the serializer) and `Error::Deserialize`.
- 6 new unit tests (99 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #68 — Add RowIndex/OptionalExtension (part of issue 44)
**2026-08-14** · [#68](https://github.com/baileyrd/rusty_-rusqlite/pull/68)

- **Added:** `RowIndex` (trait + `usize`/`&str` impls) and
  `OptionalExtension` (turns `Err(QueryReturnedNoRows)` into `Ok(None)`
  for callers that treat "no matching row" as normal, not a failure).
- **Reverted mid-PR, worth recording:** first attempt wired `RowIndex`
  into `Row::get`/`get_unwrap`/`get_ref`/`get_ref_unwrap` (changing them
  from `get<T>(usize)` to `get<T, I: RowIndex>(idx: I)`) so column
  lookups could take a name or a position. That breaks every existing
  `row.get::<i64>(0)`-style turbofish call site crate-wide — Rust
  doesn't infer a trailing type parameter once a leading one is given
  explicitly and the method takes more than one. Caught this by actually
  building, not just reasoning about it — reverted `Row`'s signatures
  back to `usize`-only (matching what's already shipped since #59) and
  kept `RowIndex` defined but unconsumed. Wiring it in is a breaking-API
  decision that needs sign-off, not something to push through because
  the trait itself was easy to write.
- **Issue 44's remaining scope:** `BindIndex`/`Params`/`Name` are
  parameter-binding traits, blocked on the same `?`-marker design
  decision as issue #25. Left open, not folded into this PR.
- 9 new/changed unit tests (93 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #67 — Add starter pragma support (closes #16)
**2026-08-14** · [#67](https://github.com/baileyrd/rusty_-rusqlite/pull/67)

- **Added:** `Connection::pragma_query_value`/`pragma_table_info`/
  `pragma_update`/`pragma_update_and_check` for exactly two pragmas —
  `foreign_keys` (routes to the existing `DbConfig::EnableForeignKeys`
  flag from #64) and `table_info` (reads the real column schema captured
  since #62's `Table::columns` addition). Any other pragma name errors
  with `UnrecognizedStatement` — full pragma coverage is its own future
  gap-analysis pass, not this issue's scope.
- **Design deviation, stated plainly:** real `rusqlite` routes
  `table_info` through its generic `pragma(name, value, f)` method rather
  than a dedicated method. This crate has a dedicated
  `pragma_table_info` instead, and doesn't implement the fully generic
  `pragma`/`pragma_query` (no-value multi-row) methods — narrower, but
  covers both pragmas the issue names.
- 4 new unit tests (89 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #66 — Add Connection busy handling (closes #23)
**2026-08-14** · [#66](https://github.com/baileyrd/rusty_-rusqlite/pull/66)

- **Added:** `Connection::busy_timeout`/`busy_handler`. Stored honestly
  (same pattern as `db_config`/`limit`) but **never invoked**: this
  crate's single-writer in-memory model has no lock contention to wait
  out, so `is_busy` can never observe `true` and neither setting has
  anything to trigger it.
- 1 new unit test (86 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #65 — Add Connection::set_errmsg (closes #24)
**2026-08-14** · [#65](https://github.com/baileyrd/rusty_-rusqlite/pull/65)

- **Added:** `Connection::set_errmsg`/`errmsg`. Paired with a getter,
  unlike `rusqlite::Connection::set_errmsg` — this crate has no
  custom-function/vtab C-level error-reporting path for the setter to
  feed into (neither exists yet), so without a getter a set value would
  be unobservable and the method pointless.
- 1 new unit test (85 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #64 — Add Connection configuration knobs (closes #15)
**2026-08-14** · [#64](https://github.com/baileyrd/rusty_-rusqlite/pull/64)

- **Added:** `Connection::db_config`/`set_db_config`/`limit`/`set_limit`/
  `set_prepared_statement_cache_capacity`/`flush_prepared_statement_cache`/
  `cache_flush`. `db_config`/`limit` are genuinely stored (a real
  `HashMap`, round-trips correctly) but **not enforced** anywhere in the
  engine yet — each doc comment says so explicitly. The three
  cache-related methods are no-ops: there's no prepared-statement cache
  (`prepare_cached` isn't implemented) or page cache (storage is a plain
  `HashMap`, not a paged file cache) for them to act on.
- **Added:** `Hash` derive on `DbConfig`/`Limit` so they work as
  `HashMap` keys.
- **Bonus, no new PR needed:** issues #33 (`Transaction`'s own methods)
  and #34 (`Savepoint`'s `Deref` nesting) turned out to already be fully
  satisfied by #63 — closed with explanatory comments rather than
  re-implementing something that already existed.
- 2 new unit tests (84 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #63 — Add Transaction/Savepoint (part of #13)
**2026-08-14** · [#63](https://github.com/baileyrd/rusty_-rusqlite/pull/63)

- **Added:** `Connection::transaction`/`transaction_with_behavior`/
  `unchecked_transaction`/`savepoint`/`savepoint_with_name`, plus
  `Transaction`/`Savepoint`/`DropBehavior`/`TransactionBehavior`. Real
  rollback: each guard snapshots table state on entry and restores it on
  drop unless `commit` was called (or `DropBehavior::Commit` was set) —
  not a no-op stub.
- **Added:** `Database::snapshot`/`restore` on the storage layer,
  backing the above. Full-clone based (documented as such) rather than a
  copy-on-write/undo-log, appropriate for this engine's current scale.
- **Design note:** `rusqlite::DropBehavior::Ignore` has no equivalent
  here — this crate's guards are ownership-based, so there's no "still
  open, nothing references it" state for `Ignore` to leave a transaction
  in.
- **Issue #13's remaining scope:** `transaction_state`/
  `set_transaction_behavior` are not implemented — they'd need a
  persistent "am I inside a transaction" flag on `Connection` itself,
  which the current borrow-based guard design (the guard holds `&mut
  Connection`, so `Connection` itself has no such flag) doesn't have
  anywhere to put without a larger redesign. Left open, not folded into
  this PR's "closes" list.
- 6 new unit tests (82 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #62 — Add Connection metadata introspection (part of #14)
**2026-08-14** · [#62](https://github.com/baileyrd/rusty_-rusqlite/pull/62)

- **Added:** `Connection::path`/`is_autocommit`/`is_busy`/`is_readonly`/
  `is_interrupted`/`db_name`/`column_exists`/`table_exists`/
  `column_metadata`/`changes`/`total_changes`, plus `ColumnMetadata`.
  `changes`/`total_changes` are tracked for real (updated on every
  `execute`); the rest are honest constants given what this crate
  currently has no concept of (transactions, concurrent access, `ATTACH`,
  on-disk files) — each documents why, rather than silently returning a
  plausible-looking value with no backing state.
- **Added:** `Table::columns: Vec<ColumnDef>` (alongside the existing
  `column_names: Vec<String>`, not replacing it) so `column_metadata` has
  declared type/constraint data to report — purely additive to an
  existing internal type.
- **Deferred, stated plainly:** `last_insert_rowid` is not implemented —
  this crate's storage has no implicit rowid concept yet (rows are plain
  `Vec<Value>` with no per-row identifier), so faking a value would be
  worse than omitting the method. Needs a storage-layer decision, not a
  quick add.
- 4 new unit tests (76 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #61 — Add Connection query_one/query_map/execute_batch (closes #12)
**2026-08-14** · [#61](https://github.com/baileyrd/rusty_-rusqlite/pull/61)

- **Added:** `Connection::query_one`/`query_map`/`execute_batch` — typed
  single-row and multi-row query access via `Row`, plus running each
  `;`-separated statement in a batch through `execute`.
- **Known limitation, stated plainly:** `execute_batch` splits on literal
  `;` characters, so a string literal containing `;` would currently be
  split incorrectly. Not a concern for today's supported statement types
  (`CREATE TABLE`/`INSERT`), but worth revisiting once statements with
  richer string literals are supported.
- **Scope note:** `prepare*` (returning a reusable, bindable `Statement`)
  is still not implemented — blocked on the same parameter-marker design
  decision flagged on issue #25. `Connection::query_row`/`query_one`/
  `query_map` cover the immediate-execution query path in the meantime.
- 4 new unit tests (72 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #60 — Add Rows/MappedRows/AndThenRows iterators (closes #32)
**2026-08-14** · [#60](https://github.com/baileyrd/rusty_-rusqlite/pull/60)

- **Added:** `Rows`/`MappedRows`/`AndThenRows` — thin iterator wrappers
  over a multi-row result set. `Rows::mapped`/`Rows::and_then` adapt to
  `MappedRows`/`AndThenRows`, matching `rusqlite`'s combinator naming;
  `AndThenRows` supports any error type `E: From<Error>`, not just
  `Error` itself.
- **Known limitation:** not yet wired to `Connection` — there's no
  multi-row query method on `Connection` yet (only `query_row` for a
  single row). That's part of the existing "Connection: query execution"
  issue (#12), not this one.
- 3 new unit tests (69 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #59 — Add Row accessors (closes #31)
**2026-08-14** · [#59](https://github.com/baileyrd/rusty_-rusqlite/pull/59)

- **Added:** `Row`/`Row::get`/`get_unwrap`/`get_ref`/`get_ref_unwrap`/
  `column_index` — a borrowed view over one result row with typed access
  via `FromSql`. `get_unwrap`/`get_ref_unwrap` panic on error by design,
  mirroring `rusqlite`'s documented panicking contract for these two
  methods rather than being incidental `unwrap()` use.
- **Added:** `Error::FromSql`/`From<FromSqlError>` so column-conversion
  failures compose into the crate's error type.
- **Known limitation, intentional:** `rusqlite::Row::get_pointer` (a raw
  FFI-handle accessor) is not implemented — there's no C backend to
  expose a pointer into, so it doesn't apply here.
- 6 new unit tests (66 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #58 — Add DbConfig/Limit enums (closes #41)
**2026-08-14** · [#58](https://github.com/baileyrd/rusty_-rusqlite/pull/58)

- **Added:** `DbConfig`/`Limit` — definitions only, matching `rusqlite`'s
  `config`/`limits` module enums. Nothing reads or enforces these yet;
  that's `Connection`'s configuration-knobs issue (#15).
- 2 new unit tests (60 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #57 — Add hooks::Wal/CheckpointMode types (closes #40)
**2026-08-14** · [#57](https://github.com/baileyrd/rusty_-rusqlite/pull/57)

- **Added:** `CheckpointMode`/`Wal` — inert scaffolding for the
  not-yet-implemented `Connection::wal_hook`. This crate has no WAL
  support (per `ARCHITECTURE.md`'s non-goals), so nothing constructs a
  `Wal` yet; these types exist so that decision doesn't block on WAL
  support landing first.
- **Also flagged:** issue #25 (`Statement` parameter binding) is deferred
  and labeled `needs-human` — implementing it requires `?`-marker syntax
  the tokenizer/parser don't support, which means changing already-shipped
  `Insert::rows`'s element type from `Value` to something parameter-aware.
  That's a breaking change to a merged public field (#48), so it's a
  stop-and-ask per this loop's own rule rather than a silent reshape.
- 2 new unit tests (58 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #56 — Add FromSql trait (closes #37)
**2026-08-14** · [#56](https://github.com/baileyrd/rusty_-rusqlite/pull/56)

- **Added:** `FromSql`/`FromSqlError`/`FromSqlResult` — converts a stored
  `Value` back into a Rust type (`Value`/`i64`/`i32`/`f64`/`bool`/
  `String`/`Vec<u8>`/`Option<T>`), erroring on storage-class mismatch or
  (for `i32`) out-of-range values.
- 6 new unit tests (56 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #55 — Add ToSql trait (closes #36)
**2026-08-14** · [#55](https://github.com/baileyrd/rusty_-rusqlite/pull/55)

- **Added:** `ToSql` trait + blanket impls for `Value`/`i64`/`i32`/`f64`/
  `bool`/`String`/`str`/`Vec<u8>`/`Option<T>`.
- **Design deviation, stated plainly:** unlike `rusqlite::ToSql`,
  `to_sql` here isn't fallible — none of these impls have a failure case,
  so wrapping the return in `Result` would be error handling for a
  scenario that can't happen. A future impl that genuinely can fail can
  introduce a fallible variant then, without forcing today's impls to
  pretend they can fail.
- 3 new unit tests (51 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #54 — Add ValueRef (closes #35)
**2026-08-14** · [#54](https://github.com/baileyrd/rusty_-rusqlite/pull/54)

- **Added:** `ValueRef`/`Value::as_ref`/`ValueRef::to_owned` — a
  borrowed, non-owning view over `Value` that avoids cloning `Text`/`Blob`
  payloads. `Value`/`Type` already existed from `A1`; this issue's `Value`/
  `Type` portion was effectively already satisfied, so this PR is scoped
  to just the missing `ValueRef` piece.
- 2 new unit tests (48 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #53 — Wire Connection to the execution engine (closes #10)
**2026-08-14** · [#53](https://github.com/baileyrd/rusty_-rusqlite/pull/53)

- **Changed:** `Connection::execute` and `Connection::query_row` are now
  real — they tokenize, parse, and dispatch to the engine (`CREATE
  TABLE`/`INSERT` for `execute`, `SELECT` for `query_row`) instead of
  being stubs. This closes out foundation-tier Part A entirely (`A1`–`A8`
  from `gap-analysis.md`): a full `CREATE TABLE` → `INSERT` → `SELECT`
  round trip now works through the public `Connection` API.
- **Added:** `Error::Token`/`Parse`/`UnrecognizedStatement`/
  `QueryReturnedNoRows`, plus `From<TokenError>`/`From<ParseError>` for
  `Error` so the tokenizer/parser's own error types compose with `?`
  instead of needing per-call-site `map_err`.
- **Known limitation:** only `CREATE TABLE`/`INSERT`/`SELECT` are wired
  up; other statement types return `UnrecognizedStatement`. The
  `rusqlite`-shaped `Statement`/`Row` API (multi-row iteration, prepared
  statements, parameter binding) is Part B scope — tracked in the
  existing open `parity-gap` issues (#11 onward), not part of this PR.
- 4 new unit tests (46 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #52 — Add execution engine (closes #9)
**2026-08-14** · [#52](https://github.com/baileyrd/rusty_-rusqlite/pull/52)

- **Added:** `execute_create_table`/`execute_insert`/`execute_select` —
  ties the parser ASTs, storage layer, and expression evaluator together
  into a real query path: single-table scan + `WHERE` filter + column
  projection. `INSERT`'s explicit column list (reordered or partial) is
  now expanded into full table-definition order, missing columns filled
  with `NULL`.
- This is the last of foundation-tier Part A except `A8` (wiring
  `Connection`/`Statement` to this engine) — `CREATE TABLE`/`INSERT`/
  `SELECT` now work end-to-end against the in-memory backend via direct
  function calls; only the public `Connection` API surface is still
  stubbed.
- 5 new unit tests (43 total); all passing. `cargo clippy -- -D warnings`
  and `cargo fmt --check` clean.

---

## PR #51 — Add expression evaluator (closes #8)
**2026-08-14** · [#51](https://github.com/baileyrd/rusty_-rusqlite/pull/51)

- **Added:** `evaluate`/`evaluate_bool` — evaluates the `SELECT` parser's
  `Expr` tree (literals, column refs, the six comparison operators)
  against a single row, following SQLite's storage-class ordering for
  cross-type comparisons and truthiness rules for boolean filtering.
- **Added:** `Error::UnknownColumn`.
- 6 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #50 — Add in-memory storage backend (closes #7)
**2026-08-14** · [#50](https://github.com/baileyrd/rusty_-rusqlite/pull/50)

- **Added:** `Database`/`Table` — in-memory table storage: `create_table`
  (from a parsed `CreateTable`), `insert_row`, `table` (schema + row scan).
  Deliberately in-memory-only per `ARCHITECTURE.md`'s non-goals.
- **Added:** `Error::TableAlreadyExists`/`TableNotFound`/
  `ColumnCountMismatch` — extends the crate's error type rather than
  introducing a second one for the storage layer. `Error` now derives
  `PartialEq` so tests can assert on it directly.
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #49 — Add single-table SELECT parser (closes #6)
**2026-08-14** · [#49](https://github.com/baileyrd/rusty_-rusqlite/pull/49)

- **Added:** `parse_select`/`Select`/`SelectColumns`/`Expr`/`BinaryOp` —
  parses `SELECT * | cols FROM table [WHERE <comparison>]` for a single
  table, no joins/aggregates/subqueries. `WHERE` parses into an `Expr`
  tree but isn't evaluated yet (that's `A6`).
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #48 — Add INSERT parser (closes #5)
**2026-08-14** · [#48](https://github.com/baileyrd/rusty_-rusqlite/pull/48)

- **Added:** `parse_insert`/`Insert` — parses `INSERT INTO ... [(cols)]
  VALUES (...)` with literal values (no expressions yet — that's `A6`).
  Split from the original combined `INSERT`/`SELECT` gap row per the
  skill's issue-sizing rule.
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #47 — Add CREATE TABLE parser (closes #4)
**2026-08-14** · [#47](https://github.com/baileyrd/rusty_-rusqlite/pull/47)

- **Added:** `parse_create_table`/`CreateTable`/`ColumnDef`/`ParseError` —
  parses `CREATE TABLE` with a column list, declared type names, and
  `PRIMARY KEY`/`NOT NULL` constraints. No other statement types yet
  (`INSERT`/`SELECT` are A4a/A4b).
- 5 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #46 — Add SQL tokenizer (closes #3)
**2026-08-14** · [#46](https://github.com/baileyrd/rusty_-rusqlite/pull/46)

- **Added:** `tokenize`/`Token`/`TokenError` — lexes identifiers, keywords,
  integer/real/string/blob literals, and punctuation/operators. No
  statement-level grammar yet (that's the parser, A3/A4).
- 6 new unit tests; all passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## PR #45 — Add Value/Type model (closes #2)
**2026-08-14** · [#45](https://github.com/baileyrd/rusty_-rusqlite/pull/45)

- **Added:** `Value`/`Type` — the SQLite five-storage-class model
  (Null/Integer/Real/Text/Blob), foundation for the tokenizer, parser, and
  storage layer that build on it.
- 2 new unit tests; both passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.

---

## Initial commit — repo-config governance files + crate skeleton
**2026-08-14** · (pushed directly — no default branch existed yet to PR against)

- **Added:** standard governance file set (README, CONTRIBUTING,
  CODE_OF_CONDUCT, SECURITY, CHANGELOG, RELEASE_NOTES, ARCHITECTURE, ADR
  seed, PR/issue templates, CI workflow) via the repo-config process.
- **Added:** minimal crate skeleton (`Cargo.toml`, `Connection::open_in_memory`/
  `close`) — no SQL parsing or execution yet. This is the foundation the
  parity-loop gap-analysis and issue backlog build on top of.
- **Known limitation:** the repo-config skill's own `.github/` template
  payload (PR templates, issue templates, CI workflow) was missing from its
  asset bundle at the time this was applied; those files were hand-written
  here to match the skill's documented conventions instead.
- 2 unit tests; both passing. `cargo clippy -- -D warnings` and
  `cargo fmt --check` clean.
