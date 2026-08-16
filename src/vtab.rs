//! Eponymous, read-only virtual tables (issue #91 — the smallest
//! meaningful slice of the `vtab` epic, built on issue #90's
//! `TableSource` abstraction). See `docs/gap-analysis-vtab.md` and
//! `docs/adr/0003-tablesource.md`.
//!
//! **Scope, stated plainly:** mirrors real `rusqlite::vtab::VTab`/
//! `VTabCursor`/`Context`'s *shape* (a `VTab` that `open()`s a
//! `VTabCursor`, driven `filter` → loop `next`/`eof`/`column`), not its
//! mechanics — there's no C `sqlite3_module`/`sqlite3_vtab_cursor` FFI
//! bridge to build, since this crate has no C engine invoking these
//! callbacks (see `docs/adr/0003-tablesource.md`'s explanation of why
//! that's true). Two real deviations from `rusqlite`'s shape, both
//! because the capability they'd represent doesn't exist here yet:
//! - **No `Values`/`ValueIter` in `filter`'s signature.** Real SQLite's
//!   `xFilter` receives pre-negotiated bound constraint values from
//!   `best_index`; this crate has no such negotiation (issue #94's
//!   resolution), so `filter` receives the whole `WHERE`-clause
//!   [`Expr`] instead, same "opportunistic hint, not a contract" rule
//!   as [`TableSource::scan`].
//! - **No `VTabCursor::rowid`.** [`TableSource::scan`]'s return shape
//!   (`Vec<Vec<Value>>`, positional only) has no rowid to report.
//!
//! **Also out of scope:** `CREATE VIRTUAL TABLE` support (issue #93) —
//! a `VTab` is only usable eponymously, registered directly by name via
//! [`crate::Connection::create_module`].

use crate::dml_select::Expr;
use crate::error::Result;
use crate::storage::TableSource;
use crate::tosql::ToSql;
use crate::value::Value;

/// An eponymous, read-only virtual table. Mirrors real
/// `rusqlite::vtab::VTab`'s shape — see this module's doc comment for
/// what's deliberately left out.
pub trait VTab {
    type Cursor: VTabCursor;

    /// This table's column names, in result-column order. Called once,
    /// when the table is wrapped by [`VTabTableSource::new`] — real
    /// SQLite declares a vtab's schema once too, via `xConnect`.
    fn column_names(&self) -> Vec<String>;

    /// Opens a new cursor for scanning this table from the start.
    fn open(&self) -> Result<Self::Cursor>;
}

/// A virtual table's row-at-a-time cursor. Mirrors real
/// `rusqlite::vtab::VTabCursor`'s shape.
pub trait VTabCursor {
    /// Begins the scan. `filter` is the query's `WHERE` clause, handed
    /// over whole — see this module's doc comment for why there's no
    /// `Values` (SQLite's `best_index`-negotiated bound values) here.
    /// Same "opportunistic hint, not a contract" rule as
    /// [`TableSource::scan`]: the engine still re-evaluates `filter`
    /// against every row this cursor produces, so ignoring it entirely
    /// is always correct, just unoptimized.
    fn filter(&mut self, filter: Option<&Expr>) -> Result<()>;

    /// Advances to the next row.
    fn next(&mut self) -> Result<()>;

    /// Whether the cursor has moved past the last row.
    fn eof(&self) -> bool;

    /// Reports the current row's value for column `i` via `ctx`.
    fn column(&self, ctx: &mut Context, i: usize) -> Result<()>;
}

/// Where [`VTabCursor::column`] reports a column's value for the
/// current row. A thin wrapper — unlike real `rusqlite::vtab::Context`,
/// there's no C `sqlite3_context*` to bridge to.
#[derive(Default)]
pub struct Context {
    value: Option<Value>,
}

impl Context {
    pub(crate) fn new() -> Context {
        Context::default()
    }

    /// Reports `value` as the current column's value.
    pub fn set_result<T: ToSql>(&mut self, value: &T) -> Result<()> {
        self.value = Some(value.to_sql());
        Ok(())
    }

    /// Consumes the reported value, defaulting to `NULL` if
    /// [`Context::set_result`] was never called — matching real
    /// SQLite's treatment of an unset result column.
    fn take(&mut self) -> Value {
        self.value.take().unwrap_or(Value::Null)
    }
}

/// Adapts a [`VTab`] into the [`TableSource`] the engine actually
/// consults: each [`TableSource::scan`] call opens a fresh cursor and
/// drives it to completion (`filter`, then loop `column`/`next` until
/// `eof`), eagerly materializing a `Vec<Vec<Value>>`. The `VTab`/
/// `VTabCursor` author writes idiomatic, `rusqlite`-shaped cursor code;
/// the engine still only ever sees a flat row list, per
/// `docs/adr/0003-tablesource.md`'s eager-scan decision.
pub struct VTabTableSource<T: VTab> {
    vtab: T,
    columns: Vec<String>,
}

impl<T: VTab> VTabTableSource<T> {
    /// Wraps `vtab`, capturing its column names once.
    pub fn new(vtab: T) -> VTabTableSource<T> {
        let columns = vtab.column_names();
        VTabTableSource { vtab, columns }
    }
}

impl<T: VTab> TableSource for VTabTableSource<T> {
    fn column_names(&self) -> &[String] {
        &self.columns
    }

    fn scan(&self, filter: Option<&Expr>) -> Result<Vec<Vec<Value>>> {
        let mut cursor = self.vtab.open()?;
        cursor.filter(filter)?;
        let mut rows = Vec::new();
        while !cursor.eof() {
            let mut row = Vec::with_capacity(self.columns.len());
            for i in 0..self.columns.len() {
                let mut ctx = Context::new();
                cursor.column(&mut ctx, i)?;
                row.push(ctx.take());
            }
            rows.push(row);
            cursor.next()?;
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dml_select::parse_select;
    use crate::engine::execute_select;
    use crate::storage::Database;
    use crate::token::tokenize;

    /// Example vtab: generates integers `start..end`, one column named
    /// `value` — the same spirit as real SQLite's own `generate_series`
    /// vtab example. Proves the `VTab`/`VTabCursor`/`Context` path works
    /// end-to-end: register → query → scan.
    struct RangeVTab {
        start: i64,
        end: i64,
    }

    impl VTab for RangeVTab {
        type Cursor = RangeCursor;

        fn column_names(&self) -> Vec<String> {
            vec!["value".to_string()]
        }

        fn open(&self) -> Result<RangeCursor> {
            Ok(RangeCursor {
                current: self.start,
                end: self.end,
            })
        }
    }

    struct RangeCursor {
        current: i64,
        end: i64,
    }

    impl VTabCursor for RangeCursor {
        fn filter(&mut self, _filter: Option<&Expr>) -> Result<()> {
            Ok(())
        }

        fn next(&mut self) -> Result<()> {
            self.current += 1;
            Ok(())
        }

        fn eof(&self) -> bool {
            self.current >= self.end
        }

        fn column(&self, ctx: &mut Context, i: usize) -> Result<()> {
            assert_eq!(i, 0, "RangeVTab only has one column");
            ctx.set_result(&self.current)
        }
    }

    #[test]
    fn range_vtab_scans_all_rows() {
        let source = VTabTableSource::new(RangeVTab { start: 1, end: 4 });
        let rows = source.scan(None).unwrap();
        assert_eq!(
            rows,
            vec![
                vec![Value::Integer(1)],
                vec![Value::Integer(2)],
                vec![Value::Integer(3)],
            ]
        );
    }

    #[test]
    fn range_vtab_column_names_are_declared_once() {
        let source = VTabTableSource::new(RangeVTab { start: 0, end: 0 });
        assert_eq!(source.column_names(), &["value".to_string()]);
    }

    #[test]
    fn range_vtab_registered_and_queried_end_to_end() {
        let mut db = Database::new();
        db.register_virtual_table(
            "range".to_string(),
            Box::new(VTabTableSource::new(RangeVTab { start: 1, end: 6 })),
        );

        let select =
            parse_select(&tokenize("SELECT * FROM range WHERE value = 3").unwrap()).unwrap();
        let (cols, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(cols, vec!["value"]);
        assert_eq!(rows, vec![vec![Value::Integer(3)]]);
    }

    #[test]
    fn range_vtab_projects_named_columns() {
        let mut db = Database::new();
        db.register_virtual_table(
            "range".to_string(),
            Box::new(VTabTableSource::new(RangeVTab { start: 1, end: 3 })),
        );

        let select = parse_select(&tokenize("SELECT value FROM range").unwrap()).unwrap();
        let (cols, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(cols, vec!["value"]);
        assert_eq!(rows, vec![vec![Value::Integer(1)], vec![Value::Integer(2)]]);
    }

    #[test]
    fn context_defaults_to_null_when_never_set() {
        let mut ctx = Context::new();
        assert_eq!(ctx.take(), Value::Null);
    }

    #[test]
    fn empty_range_scans_zero_rows() {
        let source = VTabTableSource::new(RangeVTab { start: 5, end: 5 });
        assert_eq!(source.scan(None).unwrap(), Vec::<Vec<Value>>::new());
    }
}
