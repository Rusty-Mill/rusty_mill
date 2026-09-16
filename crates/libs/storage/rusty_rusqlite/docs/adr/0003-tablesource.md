# ADR-0003: `TableSource` — the `vtab` epic's architectural prerequisite

Status: Accepted
Date: 2026-08-14

## Context

Issue #38 (the `vtab` module epic) was split, per its own acceptance
criterion, into 8 sequenced sub-issues documented in
`docs/gap-analysis-vtab.md`. That document's central finding: unlike
every other Part B gap closed in this project, `vtab` isn't an additive
slice. Real `rusqlite`'s `VTab`/`VTabCursor` traits are Rust-side
implementations of `extern "C"` callbacks stored in a `sqlite3_module`
struct — SQLite's own C query planner and virtual machine invoke them
directly while compiling and executing a query. `rusqlite` itself
contributes no scanning, filtering, or planning logic; it's a thin
`unsafe` FFI trampoline (`Module<T>` builds the C struct; each function
pointer is a generic `rust_*::<T>` wrapper that reinterprets a
`*mut sqlite3_vtab` as `*mut T` via a `#[repr(C)]`-with-`sqlite3_vtab`-
as-first-field layout trick, calls the safe Rust trait method, and
converts the result back to a C error code).

This crate has no C engine to invoke callbacks from. `storage.rs`'s
`Database` is a concrete `HashMap<String, Table>`; `engine.rs`'s
`execute_select_with_functions`/`_with_aggregates`/`_with_window` all
read `table.rows: &Vec<Vec<Value>>` directly. There is no trait boundary
a virtual row source could stand in for. Issue #90 is the decision that
creates one — the prerequisite every other `vtab` sub-issue (#91–#97)
depends on.

## Decision

1. **A new `TableSource` trait** in `storage.rs`:

   ```rust
   pub trait TableSource {
       fn column_names(&self) -> &[String];
       fn scan(&self, filter: Option<&Expr>) -> Result<Vec<Vec<Value>>>;
   }
   ```

   `Table` implements it trivially (`scan` ignores `filter`, clones
   `self.rows`) — no behavior change for native tables.

2. **Eager, not cursor-based.** `scan()` returns a fully materialized
   `Vec<Vec<Value>>` in one call — no `next`/`eof`/pull-based iteration
   protocol like real SQLite's `xNext`/`xEof`/`xColumn`. This crate's
   engine already materializes a table's rows before filtering; a lazy
   cursor would mean rewriting all three `execute_select*` functions for
   a case (huge or streaming virtual tables) this crate has no concrete
   need for yet. An honest, stated subset — same treatment as `ToSql`
   being non-fallible or `DropBehavior` missing `Ignore`.

3. **`filter: Option<&Expr>` folds issue #94 (`best_index` constraint
   pushdown) into this same signature, deliberately simplified.** The
   engine already builds the `WHERE`-clause `Expr` tree before scanning;
   passing it through costs nothing and lets a virtual table
   opportunistically skip rows it can cheaply exclude. It's a **hint,
   not a contract**: the engine still re-evaluates `filter` against
   every row `scan()` returns, so ignoring it entirely is always
   correct, just unoptimized. No `IndexConstraintUsage` negotiation, no
   `idxNum`/cost comparison, no `ORDER BY`/`DISTINCT` pushdown — there's
   no query planner on this crate's side to negotiate a plan with, since
   there's only ever one plan (scan this table). Issue #94 stays open
   only as a placeholder for a future, more structured pushdown need
   (e.g. passing parsed equality constraints instead of a raw `Expr`
   tree) — not as a faithful `IndexInfo` port.

4. **Read-only on purpose.** No `insert`/`update`/`delete` on
   `TableSource` — that's issue #95's `UpdateVTab`-equivalent, as a
   separate trait once this lands.

5. **`Database` gains a second, `pub(crate)`-only lookup path** used by
   the `SELECT` scan functions, tried after the existing native-table
   lookup:

   ```rust
   pub fn scan(&self, table_name: &str, filter: Option<&Expr>)
       -> Result<(Vec<String>, Vec<Vec<Value>>)>;
   pub(crate) fn register_virtual_table(&mut self, name: String, source: Box<dyn TableSource>);
   ```

   `register_virtual_table` has no public wrapper yet — issue #92
   (`Connection::create_module`) adds one. `Database::table`/`Table`
   themselves are **untouched**: nothing that reads `table.rows`/
   `table.column_names` directly today (`blob.rs`, `pragma_table_info`,
   `serialize.rs`) needs to change, since none of those operations make
   sense for a virtual table anyway (no blob I/O or serialization for a
   virtual row source — an explicit, stated gap rather than a silent
   omission).

## Alternatives considered

- **Port `IndexInfo`/`IndexConstraintUsage`/cost-based plan
  negotiation faithfully.** Rejected: that machinery exists in real
  SQLite specifically so its C planner can compare a vtab's proposed
  plan against alternatives (a real B-tree index, a different join
  order, ...). This crate has no planner making that comparison — there
  is only ever one plan (scan the named table) — so porting the
  negotiation protocol would add real complexity to model a decision
  that never actually has more than one option.
- **A lazy cursor protocol (`next`/`eof`/`column` methods) instead of
  eager `scan()`.** Rejected for now: would require rewriting the three
  existing `execute_select*` functions to pull row-by-row instead of
  iterating a materialized `Vec`, for a use case (huge/streaming vtabs)
  with no concrete driving example yet. Revisit if one shows up.
- **Change `Database::table`'s return type or `Table`'s shape** to
  route everything through `TableSource`. Rejected: `Table`'s fields are
  `pub` and re-exported; several existing features (`blob.rs`'s
  `cell_mut`, `pragma_table_info`, `serialize.rs`) fundamentally need
  concrete in-memory access that a trait object can't provide. Adding a
  second, narrower lookup path for the one caller (`SELECT` scanning)
  that can meaningfully generalize keeps the blast radius to that one
  path, consistent with every other non-breaking addition in this
  project's history.

## Consequences

- Unblocks #91 (core `VTab`/`VTabCursor` traits), which implements
  `TableSource` for a user-defined virtual table.
- Substantially shrinks #94's remaining scope — it's satisfied by this
  ADR's `filter` parameter rather than needing its own design pass.
- `blob`/`pragma_table_info`/`serialize` support for virtual tables is
  now an explicitly out-of-scope gap, not an oversight — worth its own
  issue if a real need for it ever comes up.
