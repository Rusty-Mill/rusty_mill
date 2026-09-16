# `vtab` module — scoped gap analysis

Follow-up to `gap-analysis.md`'s `vtab` module row, which deferred sizing
because the module is large (~25 items) and self-contained. This is that
deferred pass, done per issue #38's own acceptance criterion ("break this
out into its own gap-analysis.md pass before implementing").

## RustyMill/baileyrd sibling check

Checked before writing anything: `baileyrd/rusty_dbs` (a from-scratch
port target for a different project, pre-code — nothing to reuse) and
`baileyrd/rusty_sqlite` (a thin wrapper *over the real `rusqlite` crate*
— its FTS5 virtual-table support rides on SQLite's own C-level vtab
engine via plain `CREATE VIRTUAL TABLE ... USING fts5(...)` SQL text, so
there's no from-scratch Rust vtab *implementation* to port; wrapping a
real SQLite build sidesteps the exact problem this crate needs to solve).
No sibling repo implements a from-scratch virtual-table subsystem.

## Why this is architecturally different from every other gap closed so far

Every other Part B item closed this project was an **additive** slice:
a new method, trait, or module bolted onto the existing engine without
changing how `Database`/`execute_select*` work internally. `vtab` isn't
that. Real `rusqlite`'s vtab traits (`VTab::connect`/`best_index`,
`VTabCursor::filter`/`next`/`eof`/`column`/`rowid`) are Rust-side
implementations of C callbacks that **SQLite's own query planner and
virtual machine invoke** while executing a query — there's no
"this crate's query planner" to invoke them from, because this crate's
storage (`storage.rs`) is a concrete `HashMap<String, Table>` with
`Table::rows: Vec<Vec<Value>>`, and `engine.rs`'s `execute_select*`
functions scan that `Vec` directly. There is no trait boundary today
that a virtual table's row source could stand in for.

**This means nearly every item below is blocked on one prerequisite
architectural decision** (row 1), not on writing more trait
definitions. That decision changes how the storage/execution layer is
shaped internally — not a breaking change to any already-shipped public
signature, but a foundational redesign, which is exactly the kind of
call this project's own conventions route to a human rather than
deciding unilaterally mid-implementation.

## Full real API surface (rusqlite 0.40.2, verified via docs.rs)

| Item | Kind | What it does |
| --- | --- | --- |
| `VTab` | trait | Eponymous-only virtual table instance (the minimal read case) |
| `VTabCursor` | trait | Row-at-a-time cursor: `filter`/`next`/`eof`/`column`/`rowid` |
| `CreateVTab` | trait | Extends `VTab` for tables created via `CREATE VIRTUAL TABLE` |
| `UpdateVTab` | trait | `INSERT`/`UPDATE`/`DELETE` support on a vtab |
| `TransactionVTab` | trait | Writable vtab with transaction (begin/commit/rollback) support |
| `Module<T>` | struct | Bundles a `VTab` impl's callbacks into a registrable module |
| `Context` | struct | `VTabCursor::column`'s result-reporting handle |
| `Values`/`ValueIter`/`InValues` | struct | Wraps `filter`'s argument values |
| `Filters` | struct | Wraps `VTabCursor::filter`'s arguments + `best_index`'s requested usage |
| `IndexInfo` | struct | Passed to/from `VTab::best_index` — the constraint-pushdown negotiation |
| `IndexConstraint`/`IndexConstraintIter` | struct | One `WHERE`-clause constraint `best_index` can see/use |
| `IndexConstraintUsage`/`IndexConstraintAndUsageIter` | struct | What `best_index` tells `filter` about a constraint it's using |
| `IndexConstraintOp` | enum | `=`/`<`/`>`/`LIKE`/etc. constraint operator codes |
| `IndexFlags` | struct | Scan flags (e.g. unique-row guarantee) |
| `OrderBy`/`OrderByIter` | struct | `ORDER BY` columns `best_index` can see |
| `DistinctMode` | enum | Whether/how the query wants `DISTINCT` rows |
| `ConflictMode` | enum | `INSERT`/`UPDATE` conflict-resolution mode for `UpdateVTab` |
| `VTabConfig` | enum | Config flags a vtab can set (e.g. constraint support) |
| `VTabKind` | enum | Eponymous vs. `CREATE VIRTUAL TABLE`-backed classification |
| `VTabConnection`/`ConnectionRef` | struct | Restricted connection handle passed into vtab callbacks |
| `Inserts`/`Updates` | struct | Wraps `UpdateVTab::insert`/`update`'s arguments |
| `sqlite3_vtab`/`sqlite3_vtab_cursor` | struct | Raw FFI structs — not meaningful without C interop |
| `dequote`/`escape_double_quote` | fn | String helpers for `CREATE VIRTUAL TABLE` argument text |
| `parameter`/`parse_boolean` | fn | Parses `key=value` module-argument text |
| `vtab::array` (`load_module`, `Array`) | module | Built-in "bind a Rust `Vec` as a query-able table" vtab |
| `vtab::csvtab` | module | Example/optional: query a CSV file as a table |
| `vtab::series` | module | Example/optional: `generate_series`-style row generator |

## Proposed split (sequenced — later issues depend on earlier ones)

1. **`vtab`: `TableSource` abstraction (architectural prerequisite)** —
   `needs-human`. The actual decision: introduce a trait boundary in the
   storage/execution layer that a virtual row source can implement
   alongside the concrete `Table`, and decide how far `execute_select*`
   needs to change (materialize-then-filter today; a cursor protocol
   needs at least an opt-in pull-based path). Blocks everything else.
2. **`vtab`: `VTab`/`VTabCursor`/`Context`/`Values` core traits
   (eponymous-only, read-only)** — the smallest meaningful slice once
   (1) exists. No `CREATE VIRTUAL TABLE` parsing needed yet (eponymous
   tables are addressed by name directly, like a real table).
3. **`vtab`: `Connection::create_module` + `Module<T>` registration** —
   a named-module registry, mirroring `create_scalar_function`'s
   `HashMap<String, _>` pattern but registering a table factory instead
   of a function.
4. **`vtab`: `CREATE VIRTUAL TABLE` parsing + `CreateVTab` trait** —
   new DDL grammar (`ddl.rs`/`token.rs` currently only parse
   `CREATE TABLE`), plus `dequote`/`escape_double_quote`/`parameter`/
   `parse_boolean` for its argument text.
5. **`vtab`: `IndexInfo`/`IndexConstraint`/`best_index` constraint
   pushdown** — genuinely separate capability from basic scanning;
   meaningful only once the engine has *some* notion of pushing a
   `WHERE` constraint down to a scan source instead of materializing
   the whole table first. Likely the largest single piece.
6. **`vtab`: `UpdateVTab`/`TransactionVTab` (writable virtual
   tables)** — `INSERT`/`UPDATE`/`DELETE` through a vtab; depends on
   (2) and (4).
7. **`vtab`: built-in `array` module (`rarray!`-equivalent)** — the
   first real vtab to build once (1)–(3) exist; small and
   self-contained (bind a `Vec` as a one-column table), good proof that
   the abstraction actually works end-to-end.
8. **`vtab::csvtab`/`vtab::series`** — optional/example modules even in
   real `rusqlite` (not core vtab functionality). Filed as one
   low-priority issue rather than expanded, since neither is a
   parity-critical gap.

Each is filed as its own `parity-gap` issue referencing this doc and its
predecessor(s) via `Depends on`. Issue #38 is closed once these are
filed — this document *is* its deliverable; the sub-issues carry the
actual implementation work forward.
