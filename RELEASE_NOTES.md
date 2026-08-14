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

## PR #87 — Add trace module: TraceEvent/TraceEventCodes/StmtRef/ConnRef/trace_v2 (closes #39)
**2026-08-14** · [#87](https://github.com/baileyrd/rusty_-rusqlite/pull/87)

Re-investigated #39, which had been left open after an earlier pass
found it blocked on `Statement` not existing yet. `Statement` now
exists (issue #25's PR), so the blocker no longer applies — implemented.

- **Added:** `TraceEventCodes` (bitmask: `STMT`/`PROFILE`/`CLOSE`),
  `TraceEvent` (`Stmt`/`Profile`/`Close`), `StmtRef`, `ConnRef`, and
  `Connection::trace_v2` — a single callback unifying real SQLite's
  separate `trace`/`profile` callbacks (both of which
  [`Connection::trace`]/[`Connection::profile`] from PR #75 still
  provide unchanged; `trace_v2` is additive).
- **`StmtRef`/`ConnRef` are simplified from real `rusqlite`'s:** the
  real types wrap a raw `sqlite3_stmt`/`sqlite3` C handle so a callback
  can query things like `expanded_sql()` off it. This engine has no such
  handle — `StmtRef` exposes just the SQL text (`sql()`), `ConnRef` just
  read-only `Connection` methods (`is_open()`).
- **No `Row` event kind:** real SQLite fires it once per row as a
  statement steps incrementally. This engine's queries run to completion
  in one call (no virtual machine to step — see `ARCHITECTURE.md`), so
  there's no per-row moment to fire it at.
- **`config_log`/`log` still not implemented, on purpose:** `config_log`
  hooks SQLite's internal C-level diagnostic log, which has no
  equivalent in this from-scratch engine (no `libsqlite3-sys` dependency,
  no internal log stream) — implementing it as inert scaffolding would
  misrepresent it as more functional than it could ever be, unlike
  genuinely-inert-but-honest settings like `busy_timeout`.
- 7 new unit tests (259 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #86 — Add window functions (closes #19)
**2026-08-14** · [#86](https://github.com/baileyrd/rusty_-rusqlite/pull/86)

Closes the remaining piece of #19 — PR #74 already shipped aggregate
functions and collation registration; this adds window functions.

- **Added:** `SUM(a) OVER (PARTITION BY b)`-style window select lists —
  new `SelectColumns::Window`/`WindowCall` AST, parser support (`OVER
  (PARTITION BY col, ...)` after any aggregate-shaped call), and
  `execute_select_with_window` (dispatched from `Connection::run_select`
  alongside the existing aggregate/plain paths).
- **Added:** `Connection::create_window_function`/`remove_window_function`.
- **Scope, stated plainly:** only `PARTITION BY`, no `ORDER BY` or frame
  clause (`ROWS`/`RANGE BETWEEN ...`) — every row in a partition gets
  the same whole-partition aggregate value, not a running/cumulative
  one. Building real per-row-varying results (running totals, `RANK`,
  `LAG`/`LEAD`) needs per-partition ordering and frame-boundary
  machinery — a comparable amount of new grammar and execution logic to
  the vtab epic (#38), not a small addition. `ROW_NUMBER`/`RANK`/
  `DENSE_RANK`/`NTILE`/`LAG`/`LEAD` aren't supported for the same
  reason — they're inherently row-position-dependent, not whole-partition
  aggregates.
- **Design deviation:** real `rusqlite::Connection::create_window_function`
  takes a `WindowAggregate` trait (`step`/`inverse`/`value`/`finalize`)
  so SQLite can slide a frame's boundaries incrementally. Since this
  crate's window functions only ever compute over a whole partition
  (no frame to slide), `create_window_function` is a thin alias over
  the same registry `create_aggregate_function` already uses — any
  aggregate is automatically usable as a window function too.
- **Partition lookup is linear, not hashed:** `Value` doesn't implement
  `Hash`/`Eq` (a `Real(f64)` payload can't), so partition grouping scans
  a `Vec<(Vec<Value>, accumulator)>` instead of using a `HashMap`. Fine
  at this crate's table scale; would need revisiting for large tables.
- 11 new unit tests (252 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #85 — Add params!/named_params!/prepare_and_bind! macros (closes #42)
**2026-08-14** · [#85](https://github.com/baileyrd/rusty_-rusqlite/pull/85)

- **Added:** `params!`, `named_params!`, `prepare_and_bind!`,
  `prepare_cached_and_bind!` — all four macros #42 named, built on
  `Params`/`BindIndex` (#44) and `Statement::raw_bind_parameter` (#25).
- **`named_params!` syntax deviation, stated plainly:** uses `name =>
  value` pairs instead of real `rusqlite`'s `name: value` — `macro_rules!`
  only allows `=>`/`,`/`;` to follow an `expr` fragment in a matcher, and
  `:` isn't one of them.
- **`prepare_cached_and_bind!` deviation:** identical to
  `prepare_and_bind!` — this crate has no prepared-statement cache to
  consult yet (same documented no-op status as
  `Connection::set_prepared_statement_cache_capacity`).
- **New:** `NamedParams<'a>(&'a [(&'a str, Value)])`, a `Params` impl
  that binds by name (via `BindIndex`) rather than position — the type
  `named_params!` produces.
- **A genuine borrow-checker subtlety, worth recording:** `prepare_and_bind!`
  expands to a plain block, not a closure — a closure would trap the
  `Statement<'_>` it returns (which borrows from `conn`) inside its own
  scope, since a basic closure can't express returning a borrow of its
  own captured environment past the call. The block form lets `?`
  propagate through whichever function the macro is invoked in instead.
- 6 new unit tests (241 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #84 — Add BindIndex/Params/Name traits (closes #44)
**2026-08-14** · [#84](https://github.com/baileyrd/rusty_-rusqlite/pull/84)

- **Added:** a new `params` module with `BindIndex` (resolve a `usize` or
  `&str` name to a bound-parameter index, via `Statement::parameter_index`)
  and `Params` (bind a whole positional value set at once — implemented
  for `()`, `&[T]`, `[T; N]`, and tuples up to 4 elements).
  `RowIndex`/`OptionalExtension` — the other two traits #44 named — were
  already implemented earlier in this project's history; this closes the
  remaining gap.
- **Provenance caveat, stated plainly:** #44 also named a top-level
  `Name` trait, but no such trait could be confirmed in real `rusqlite`'s
  current public API. Implemented as a best-effort interpretation — "the
  name half of a named-parameter pair" (`&str`/`String` → the name text)
  — documented in `params.rs` as unverified rather than presented as a
  faithful port.
- **Added, consuming the new traits:** `Statement::bind_parameter`
  (`raw_bind_parameter` + `BindIndex` name resolution in one call),
  `Statement::execute_with_params`/`query_map_with_params`, and
  `Connection::execute_with_params`/`query_map_with_params` — all new
  methods alongside the existing no-params ones, not signature changes.
- 12 new unit tests (235 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #83 — Add real `?`/`:name` parameter binding (closes #25)
**2026-08-14** · [#83](https://github.com/baileyrd/rusty_-rusqlite/pull/83)

This makes the parameter-marker design decision every Statement-adjacent
issue (#25, #26–#30, #42, #44, part of #39/#43) had been flagged as
blocked on since early in this project's history, and implements it —
see `docs/adr/0002-parameter-markers.md` for the full decision record.

- **Added:** the tokenizer recognizes `?`/`?N`/`:name`/`@name`/`$name` as
  a new `Token::Param`; the AST gains `ParamMarker` and `Expr::Parameter`.
- **The authorized breaking change:** `Insert::rows` changes from
  `Vec<Vec<Value>>` to `Vec<Vec<Expr>>` (each slot is now
  `Expr::Literal`/`Expr::Parameter`) — this is the exact change #25's
  original triage flagged as needing human sign-off, now made
  deliberately per the ADR.
- **Added:** `Statement::raw_bind_parameter`/`clear_bindings`, closing
  #25. `Connection::prepare` resolves every marker to a 1-based index
  (SQLite's own numbering: bare `?` auto-increments, `?N` claims index
  `N` and bumps the counter past it, a repeated `:name`/`@name`/`$name`
  reuses its first index) once, at prepare time. `execute`/`query*`
  substitute bound values (or `Value::Null` for unbound, matching real
  SQLite) into a fully-concrete copy before handing it to the existing
  engine/eval functions — **their already-shipped signatures didn't
  change**, only gained the one unavoidable new `Expr::Parameter` match
  arm.
- **Updated to be real:** `Statement::parameter_count`/`parameter_name`/
  `parameter_index` (previously always `0`/`None`, honestly, since no
  parameters could exist) and `Statement::expanded_sql` (previously
  always the original text; now does real value substitution via a
  string-literal-aware text scan, independent of but
  index-assignment-consistent with the AST-level resolution).
- **Added:** `ToSql for &str` (the existing `ToSql for str` impl doesn't
  satisfy a `T: ToSql` bound, since that implies `T: Sized` and `str`
  isn't) — needed for `stmt.raw_bind_parameter(1, "text")` to work
  directly.
- **Discovered along the way:** this crate's `WHERE` grammar only
  supports a single comparison — no `AND`/`OR` combining multiple
  conditions. Pre-existing, unrelated to this change; worked around in
  tests needing two parameters in one statement by using a function
  call's argument list instead. Not fixed here — a separate,
  already-known gap in the expression grammar, not a parameter-binding
  concern.
- Unblocks (not implemented here — natural follow-ups): #44's
  `BindIndex`/`Params`/`Name` traits, #42's `params!`/`named_params!`
  macros, #43's `params_from_iter`, and #39's `TraceEvent`/`ConnRef`/
  `StmtRef` (still needs its own scoping pass even so).
- 16 new unit tests (223 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #82 — Add MAIN_DB/TEMP_DB constants (part of issue 43)
**2026-08-14** · [#82](https://github.com/baileyrd/rusty_-rusqlite/pull/82)

- **Added:** top-level `MAIN_DB`/`TEMP_DB` string constants, plus wired
  the existing hardcoded `"main"` literals in `Connection::db_name`/
  `require_main_database`/`fire_update_hook` and `Statement::execute`'s
  read-only check to use `MAIN_DB` instead.
- **Scope note:** issue #43 also covers `version`/`version_number`
  (explicitly flagged `needs-human` in the issue body — this crate isn't
  wrapping a real SQLite build to report a version from, so the
  versioning-scheme question needs an explicit decision, not a silently
  invented number) and `params_from_iter` (blocked on the same
  parameter-binding decision as issue #25). Neither is implemented here;
  `MAIN_DB`/`TEMP_DB` were the one genuinely unblocked piece — found by
  re-reading the issue body closely rather than treating its
  `needs-human` label as covering all of it.
- 2 new unit tests (207 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #81 — Add Statement parameter introspection + diagnostics (closes #29, #30)
**2026-08-14** · [#81](https://github.com/baileyrd/rusty_-rusqlite/pull/81)

- **Added:** `Statement::parameter_count`/`parameter_name`/
  `parameter_index` (issue #29) — all honestly report `0`/`None` since
  `Statement` doesn't support parameter binding yet (see `statement.rs`'s
  module doc comment), so no statement can ever have any.
- **Added:** `Statement::expanded_sql`/`readonly`/`is_explain`/
  `get_status`/`reset_status`/`finalize` (issue #30). `expanded_sql` is
  just the original SQL text — there's nothing bound to substitute in.
  `readonly` distinguishes `SELECT` from `CREATE TABLE`/`INSERT`.
  `is_explain` always reports `0` (not `EXPLAIN`) — this crate's parser
  doesn't recognize the `EXPLAIN` keyword at all yet.
  `get_status`/`reset_status` (plus the new `StatementStatus` type) are
  stored-but-inert — this engine has no virtual machine to count
  fetch/sort/index operations for, same "not enforced, not silently
  dropped" treatment already given to `Connection::busy_timeout`.
  `finalize` is a no-op consuming method — no separate C-level statement
  handle exists to release.
- This closes out the `Statement`/`Connection::prepare` group started in
  PR #79 (issues #26–#30) — everything reachable without the
  parameter-marker decision flagged in #25 is now implemented.
- 6 new unit tests (205 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #80 — Add Statement::query/query_and_then/exists/raw_query/column_index (closes #27, #28)
**2026-08-14** · [#80](https://github.com/baileyrd/rusty_-rusqlite/pull/80)

- **Found:** PR #79's "Closes #26, #27, #28" only actually auto-closed
  #26 — GitHub's issue-linking keyword apparently only links the first
  number in a comma-separated list, not all of them. #27/#28 stayed
  open, which is correct: PR #79 only covered part of each (`query_map`/
  `query_row`/`query_one` for #27, `column_names`/`column_count`/
  `column_name` for #28) — this PR finishes both.
- **Added:** `Statement::query`/`query_and_then` (lazy `Rows`-based, the
  same shape as real `rusqlite`, unlike PR #79's eager
  `query_map`/`query_row`/`query_one`), `exists`, `raw_query` (identical
  to `query` here, since there's no params-binding step to skip), and
  `column_index`.
- **`columns`/`columns_with_metadata`/`column_metadata` not provided:**
  checked against real `rusqlite` 0.40.2 docs — all three are behind
  opt-in Cargo features (`column_decltype`/`column_metadata`), not part
  of the default API surface this crate targets.
  `column_metadata` in particular returns a raw `&CStr`-tuple straight
  out of SQLite's C API, with no honest equivalent here.
- **Lesson for future multi-issue PRs:** use a separate `Closes #N` per
  issue (or verify after merge) rather than one comma-separated list —
  GitHub's keyword linking doesn't reliably chain through commas.
- 5 new unit tests (199 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #79 — Add Statement: prepare, execute/query, column introspection (closes #26; part of #27, #28)
**2026-08-14** · [#79](https://github.com/baileyrd/rusty_-rusqlite/pull/79)

- **Added:** `Connection::prepare` and a new `Statement` type —
  `execute`/`query_map`/`query_row`/`query_one`/`column_names`/
  `column_count`/`column_name`/`is_query`. Tokenizes/parses SQL once;
  a prepared `INSERT`/`SELECT` can be run repeatedly without re-parsing.
- **Scope, stated plainly:** real `rusqlite::Statement::execute`/`query*`
  always take a `params: impl Params` argument. This crate's tokenizer
  doesn't recognize `?`/`:name` parameter markers yet (the same blocker
  flagged in #25 — representing them needs an AST decision that would
  change the already-shipped `Insert::rows` field), so `Statement` only
  supports parameter-free SQL: no `params` argument, because nothing can
  bind into one yet. This wasn't as fully blocked by #25 as first
  assumed, though — re-reading #26/#27/#28's own "Depends on" lines (just
  `A7`/`A8`, not the parameter-binding issue #42/#39 explicitly cited)
  turned up real, shippable scope: parsing once and reusing the parsed
  form is the actual performance point of a prepared statement,
  independent of parameter binding.
- **Also out of scope for now:** unlike `Connection::execute`,
  `Statement::execute` doesn't fire `trace`/`profile`/`commit_hook`/
  `update_hook`/the authorizer, or update `last_insert_rowid`/`changes`/
  `total_changes` — wiring a prepared statement into that hook machinery
  is real work, left for a deliberate follow-up rather than folded into
  an already-large first cut. `Statement::execute` does still respect
  `OpenFlags::READ_ONLY` and persist to a file-backed connection, since
  those are correctness guarantees, not observability.
- 13 new unit tests (194 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #78 — Add Connection::transaction_state/set_transaction_behavior (closes #13)
**2026-08-14** · [#78](https://github.com/baileyrd/rusty_-rusqlite/pull/78)

- **Added:** `Connection::transaction_state`/`transaction_behavior`/
  `set_transaction_behavior`, plus the `TransactionState` type
  (`None`/`Write`).
- `Transaction::new` increments a new `transaction_depth` counter on the
  connection; `commit`/`rollback`/a drop-triggered finish all funnel
  through one `mark_finished` helper that decrements it, so tracking
  can't drift out of sync with whichever path actually ran. `Savepoint`
  wraps `Transaction`, so nested savepoints are covered by the same
  counter with no extra plumbing.
- **Design deviation, stated plainly:** real SQLite's `TransactionState`
  distinguishes a `Read` lock from a `Write` lock. This crate's
  single-writer in-memory snapshot model has no separate read/write lock
  state — any open transaction (at any nesting depth) reports as
  `Write`.
- **Not enforced:** `set_transaction_behavior` stores the default for
  future `Connection::transaction()` calls, same "accepted for API-shape
  parity only" treatment already given to `transaction_with_behavior`'s
  explicit override — this crate's transactions don't distinguish
  `Deferred`/`Immediate`/`Exclusive` locking.
- This was the scope `#63` (transaction/savepoint management) left open,
  originally deferred as needing "a larger redesign" for a
  transaction-state flag — revisited and found narrower than that: a
  simple depth counter, incremented/decremented at the one funnel point
  every finish path already goes through, was enough.
- 4 new unit tests (181 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #77 — Add rowid tracking + Connection::last_insert_rowid (closes #14)
**2026-08-14** · [#77](https://github.com/baileyrd/rusty_-rusqlite/pull/77)

- **Added:** `Table` now tracks each row's SQLite-style rowid
  (`row_ids: Vec<i64>`, index-aligned with `rows`) — monotonically
  increasing per table, assigned in `Database::insert_row`, never reused
  (no `DELETE` yet, so the reuse question doesn't arise). Persisted
  through `serialize`/`deserialize`, so a file-backed connection (PR #76)
  keeps assigning rowids correctly across reopens.
- **Added:** `Connection::last_insert_rowid()` — the rowid of the most
  recent successful `INSERT` on this connection, across any table (`0`
  before any `INSERT`). For a multi-row `INSERT`, this is the last row's
  rowid. Unaffected by a vetoed/rolled-back `commit_hook`.
- **Non-breaking additions, not signature changes:** `Database::insert_row`
  and `execute_insert` keep their existing `Result<()>`/`Result<usize>`
  signatures untouched — the new rowid-returning behavior lives in new
  `Database::insert_row_returning_rowid`/`engine::execute_insert_returning_rowids`
  functions instead, following this project's established pattern for
  extending an already-shipped signature without breaking it.
- **Improved as a direct consequence:** `Connection::update_hook`'s
  `rowid` argument (added in PR #75) now reports the row's real,
  persistent rowid instead of PR #75's row-position placeholder, now
  that storage actually tracks one.
- **Scope note:** `Connection::blob_open`'s existing `row_index`
  parameter still addresses by row position, not rowid — left alone
  deliberately (see `blob.rs`'s updated doc comment) since reinterpreting
  an already-shipped parameter's meaning is exactly the kind of
  behavior change this project treats as needing its own deliberate
  follow-up, not something to fold into an unrelated PR.
- 9 new unit tests (177 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #76 — Add file-backed Connection::open + OpenFlags (closes #11)
**2026-08-14** · [#76](https://github.com/baileyrd/rusty_-rusqlite/pull/76)

- **Added:** `Connection::open`/`open_with_flags`/`open_with_flags_and_vfs`
  (file-backed) and `open_in_memory_with_flags`/
  `open_in_memory_with_flags_and_vfs`, plus the `OpenFlags` type (a
  hand-rolled bitmask — no new dependency).
- **Added:** `Connection::flush`, and `Connection::path`/`is_readonly`
  now report real state instead of always `None`/`false`.
- **Design deviation, stated plainly:** the file `open` reads/writes is
  this crate's own binary format (`serialize.rs`), not a real SQLite
  database file — matching `ARCHITECTURE.md`'s non-goal of matching
  SQLite's on-disk format/C ABI. Persistence is write-through: the full
  database is re-serialized and the file rewritten after every
  successful `execute` call, not incrementally at the page level like
  real SQLite — simple and correct at this engine's scale, same tradeoff
  as `Database::snapshot`.
- Only `OpenFlags::READ_ONLY` (enforced: `execute` on a read-only
  connection errors) and `CREATE` (enforced: opening a nonexistent path
  without it errors instead of silently starting empty) change behavior;
  `URI`/`NO_MUTEX`/`FULL_MUTEX`/`SHARED_CACHE`/`PRIVATE_CACHE` are
  accepted for shape parity but inert — no URI parsing, shared-cache
  mode, or per-connection-vs-shared mutex distinction to vary.
- **Scope note:** `from_handle`/`from_handle_owned` (wrapping a raw C
  `sqlite3*`) aren't implemented — there's no C handle to wrap in a
  pure-Rust engine with no `libsqlite3-sys` dependency, per
  `ARCHITECTURE.md`'s own non-goals. `*_and_vfs` variants accept and
  ignore the VFS name — no pluggable I/O backend exists for one to
  select between.
- New errors: `Error::Io`, `Error::DatabaseDoesNotExist`,
  `Error::ReadOnlyConnection`.
- 10 new unit tests (168 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #75 — Add commit/rollback/update/authorizer/trace/profile/progress hooks (closes #20)
**2026-08-14** · [#75](https://github.com/baileyrd/rusty_-rusqlite/pull/75)

- **Added:** `Connection::commit_hook`/`rollback_hook`/`update_hook`/
  `authorizer`/`trace`/`profile`/`progress_handler`, plus the
  `hooks::Action`/`AuthContext`/`Authorization`/`TransactionOperation`
  types.
- `commit_hook` fires once per top-level `execute` call (this crate's
  `is_autocommit` is always `true`, so there's no explicit-transaction
  boundary distinct from a single statement to defer to); returning
  `true` rolls back the statement's changes and fires `rollback_hook`.
  `rollback_hook` also fires from `Transaction::rollback`/
  `Savepoint::rollback`/a drop-triggered rollback.
- `update_hook` fires once per row inserted, as `(action, "main",
  table_name, rowid)` — `rowid` is the row's position within the table
  (this crate's storage has no real SQLite rowid concept yet, same
  deviation as `Blob`'s row addressing). Only `Action::Insert` can fire
  today; `Update`/`Delete` have no statements to trigger them.
- `authorizer` runs once per `execute`/`query_*` call with the whole
  target table (real SQLite's authorizer is column-granular during
  statement preparation — no per-column read tracking here to offer
  that).
- `trace`/`profile` fire with the raw SQL text (and, for `profile`, the
  elapsed `Duration`) around every `execute`/`query_*` call.
- **Not fully enforced:** `progress_handler` fires once, before a
  statement starts (so it can prevent a statement from running), not
  periodically during execution — this engine has no VM instruction loop
  to interrupt mid-statement the way real SQLite's does.
- **Design, kept `&self`-compatible on purpose:** `trace`/`profile`/
  `authorizer`/`progress_handler` fire from `query_row`/`query_one`/
  `query_map`, which take `&self` — an already-shipped signature this
  project won't break. Their hook storage uses `RefCell` for interior
  mutability instead. `commit_hook`/`rollback_hook`/`update_hook` only
  fire from `&mut self` paths (`execute`, `Transaction`), so they're
  plain fields.
- Excludes `wal_hook` (deferred with the rest of WAL — see
  `ARCHITECTURE.md`'s non-goals).
- 17 new unit tests (158 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #74 — Add whole-table aggregate functions + collation registration (part of issue 19)
**2026-08-14** · [#74](https://github.com/baileyrd/rusty_-rusqlite/pull/74)

- **Added:** aggregate select lists — `SELECT COUNT(*), SUM(a) FROM t
  WHERE ...` — via a new `SelectColumns::Aggregates` AST variant, folding
  every row matching the `WHERE` filter into one output row. No `GROUP
  BY` yet, so this is whole-table aggregation only, not grouped.
- **Added:** `Connection::create_aggregate_function`/
  `remove_aggregate_function` and the `Aggregate` type (starting
  accumulator + `step`/`finalize` closures) for custom aggregates.
  `COUNT`/`SUM`/`MIN`/`MAX` are seeded as built-ins on every new
  connection, matching how real SQLite treats them as engine-core rather
  than something a caller must register.
- **Added:** `Connection::create_collation`/`remove_collation` for
  registering a named text-comparison function.
- **Design deviation, stated plainly:** `Aggregate` isn't
  `rusqlite::functions::Aggregate<A, T>`'s generic trait with an
  associated state type — it's a plain `Value` accumulator plus two
  closures, which can't express something like `AVG` (needs a running
  sum *and* count) or `GROUP_CONCAT`. Not provided as a built-in here.
- **Not enforced:** collations are stored but never consulted — there's
  no `COLLATE name` clause in the `WHERE`/`ORDER BY` grammar yet for a
  query to opt into one. Same "stored honestly, not silently discarded"
  treatment as `busy_timeout`/`db_config`.
- **Scope note:** issue #19 also covers window functions
  (`create_window_function`, `WindowAggregate`, `OVER`/`PARTITION BY`
  parsing). Left for a follow-up — that's a comparable amount of new
  parsing/execution machinery to the vtab epic (#38), not a small
  addition on top of this PR. Issue stays open; see its tracking
  comment.
- 15 new unit tests (141 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #73 — Add ZeroBlob (finishes issue 21)
**2026-08-14** · [#73](https://github.com/baileyrd/rusty_-rusqlite/pull/73)

- **Added:** `ZeroBlob(usize)`, a `ToSql` marker that inserts as an
  `N`-byte zero-filled `BLOB` — the usual pattern for allocating a blob
  upfront to write into incrementally via `Blob::write_at`. Closes out
  the last unimplemented piece of #21's gap description
  (`blob::Blob`/`ZeroBlob`) that PR #72 left out.
- **Design note:** unlike real SQLite's `zeroblob()`, which lets the
  engine defer allocating the zero-filled buffer, this crate's storage
  already keeps every value fully materialized in memory, so
  `ZeroBlob::to_sql` just allocates the `Vec<u8>` directly — there's no
  lazy-allocation win here, only the API-parity convenience of not
  writing `Value::Blob(vec![0; n])` by hand.
- 2 new unit tests (126 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

---

## PR #72 — Add Connection::blob_open / incremental BLOB I/O (part of issue 21)
**2026-08-14** · [#72](https://github.com/baileyrd/rusty_-rusqlite/pull/72)

- **Added:** `Connection::blob_open(table, column, row_index, read_only)`
  returning a `Blob` handle with `len`/`is_empty`/`is_read_only`/
  `read_all`/`read_at`/`write_at`. `write_at` can't resize the blob
  (matching real SQLite's `sqlite3_blob_write` constraint) and errors on
  a read-only handle.
- **Design deviation, stated plainly:** real SQLite (and `rusqlite::Blob`)
  addresses a blob by rowid. This crate's storage has no rowid concept
  yet (same gap flagged in #14's `last_insert_rowid` discussion), so
  `Blob` is addressed by `row_index` — a row's plain position within
  `Table::rows` at open time — which is only stable as long as no earlier
  row is removed. `Blob` also doesn't implement `std::io::{Read, Write,
  Seek}` like `rusqlite::Blob`; `read_at`/`write_at` cover the same
  random-access use case directly.
- **Storage layer:** added `Database::cell_mut` (mutable single-cell
  access by table/row-index/column-index) to support in-place writes.
- **New errors:** `Error::IndexOutOfBounds`, `Error::ReadOnlyBlob`.
- 11 new unit tests (124 total); all passing. `cargo clippy -- -D
  warnings` and `cargo fmt --check` clean.

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
