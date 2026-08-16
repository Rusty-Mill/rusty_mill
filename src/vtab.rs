//! Virtual tables (issues #91/#93, built on issue #90's `TableSource`
//! abstraction). See `docs/gap-analysis-vtab.md` and
//! `docs/adr/0003-tablesource.md`.
//!
//! Two ways to register one, via [`crate::Connection`]:
//! - [`crate::Connection::create_module`] — a ready-made [`VTab`]
//!   instance, usable directly by name (e.g. `SELECT * FROM name`), no
//!   `CREATE VIRTUAL TABLE` needed. The eponymous case.
//! - [`crate::Connection::register_module`] — a [`CreateVTab`] type,
//!   instantiated fresh by each `CREATE VIRTUAL TABLE table_name USING
//!   module_name(args...)` that names it.
//!
//! **Scope, stated plainly:** mirrors real `rusqlite::vtab::VTab`/
//! `VTabCursor`/`Context`/`CreateVTab`'s *shape* (a `VTab` that
//! `open()`s a `VTabCursor`, driven `filter` → loop
//! `next`/`eof`/`column`), not its mechanics — there's no C
//! `sqlite3_module`/`sqlite3_vtab_cursor` FFI bridge to build, since
//! this crate has no C engine invoking these callbacks (see
//! `docs/adr/0003-tablesource.md`'s explanation of why that's true).
//! Real deviations from `rusqlite`'s shape, all because the capability
//! they'd represent doesn't exist here yet:
//! - **No `Values`/`ValueIter` in `filter`'s signature.** Real SQLite's
//!   `xFilter` receives pre-negotiated bound constraint values from
//!   `best_index`; this crate has no such negotiation (issue #94's
//!   resolution), so `filter` receives the whole `WHERE`-clause
//!   [`Expr`] instead, same "opportunistic hint, not a contract" rule
//!   as [`TableSource::scan`].
//! - **No `VTabCursor::rowid`.** [`TableSource::scan`]'s return shape
//!   (`Vec<Vec<Value>>`, positional only) has no rowid to report.
//! - **No `Module<T>` type.** Real `rusqlite::vtab::create_module`
//!   takes a `&'static Module<T>` carrying a `'static`-lifetime
//!   C-callback bundle plus optional `aux` data across the C FFI
//!   boundary. This crate has neither — `register_module`'s registry
//!   lives directly in `Connection`'s own fields, so the type
//!   parameter alone determines a module's behavior. Considered
//!   introducing `Module<T>` anyway (per #92's own note that #93 might
//!   be where it "earns its keep") and concluded it still wouldn't:
//!   there's no static-lifetime/aux-data ceremony a wrapper type would
//!   actually carry here.

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

/// A [`VTab`] that can be constructed from `CREATE VIRTUAL TABLE
/// table_name USING module_name(args...)` (issue #93). Extends
/// [`VTab`] the same way real `rusqlite::vtab::CreateVTab` extends
/// `VTab`.
pub trait CreateVTab: VTab {
    /// Constructs a new instance from the module's argument list —
    /// each already reconstructed from its source tokens (see
    /// [`crate::CreateVirtualTable`]'s doc comment for the honest
    /// caveat about exact-text fidelity). Use [`dequote`]/[`parameter`]/
    /// [`parse_boolean`] to interpret them, the same way a real
    /// `rusqlite` vtab module would.
    fn connect(args: &[String]) -> Result<Self>
    where
        Self: Sized;
}

/// Type-erased factory for a [`CreateVTab`] implementor, registered by
/// module name via [`crate::Connection::register_module`] and
/// instantiated by `CREATE VIRTUAL TABLE ... USING module_name(args)`.
/// Needed because [`CreateVTab::connect`]'s `-> Result<Self>` (`Self:
/// Sized`) isn't object-safe on its own — the same type-erasure step
/// [`VTabTableSource`] already does for a ready-made [`VTab`] instance,
/// just triggered by parsed `CREATE VIRTUAL TABLE` args instead of a
/// value the caller already built.
pub(crate) trait VTabModule {
    fn connect(&self, args: &[String]) -> Result<Box<dyn TableSource>>;
}

pub(crate) struct CreateVTabModule<T> {
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> CreateVTabModule<T> {
    pub(crate) fn new() -> CreateVTabModule<T> {
        CreateVTabModule {
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: CreateVTab + 'static> VTabModule for CreateVTabModule<T> {
    fn connect(&self, args: &[String]) -> Result<Box<dyn TableSource>> {
        let vtab = T::connect(args)?;
        Ok(Box::new(VTabTableSource::new(vtab)))
    }
}

/// Strips matching surrounding quote characters (`'`, `"`, `` ` ``, or
/// `[`...`]`) from `s` and un-escapes doubled inner quotes. Returns `s`
/// unchanged if it isn't quoted. Mirrors real
/// `rusqlite::vtab::dequote`.
pub fn dequote(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() < 2 {
        return s.to_string();
    }
    let inner = &s[1..s.len() - 1];
    match (bytes[0], bytes[bytes.len() - 1]) {
        (b'\'', b'\'') => inner.replace("''", "'"),
        (b'"', b'"') => inner.replace("\"\"", "\""),
        (b'`', b'`') => inner.replace("``", "`"),
        (b'[', b']') => inner.to_string(),
        _ => s.to_string(),
    }
}

/// Doubles every `"` in `s`, for safely embedding it inside a
/// double-quoted identifier/string. Mirrors real
/// `rusqlite::vtab::escape_double_quote`.
pub fn escape_double_quote(s: &str) -> String {
    s.replace('"', "\"\"")
}

/// Splits a `key=value` (or `key = value`) module argument into
/// `(key, value)`, both trimmed. Returns `None` if `s` has no `=`.
/// Mirrors real `rusqlite::vtab::parameter`.
pub fn parameter(s: &str) -> Option<(&str, &str)> {
    let idx = s.find('=')?;
    Some((s[..idx].trim(), s[idx + 1..].trim()))
}

/// Parses a boolean-ish module argument value: `1`/`0`, `true`/`false`,
/// `yes`/`no`, `on`/`off` (case-insensitive). Mirrors real
/// `rusqlite::vtab::parse_boolean`.
pub fn parse_boolean(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
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

    #[test]
    fn dequote_strips_matching_quotes_and_unescapes_doubled_inner_quotes() {
        assert_eq!(dequote("'it''s'"), "it's");
        assert_eq!(dequote("\"say \"\"hi\"\"\""), "say \"hi\"");
        assert_eq!(dequote("`ident`"), "ident");
        assert_eq!(dequote("[bracketed]"), "bracketed");
    }

    #[test]
    fn dequote_leaves_unquoted_or_mismatched_text_alone() {
        assert_eq!(dequote("bare"), "bare");
        assert_eq!(dequote("'mismatched\""), "'mismatched\"");
        assert_eq!(dequote(""), "");
        assert_eq!(dequote("x"), "x");
    }

    #[test]
    fn escape_double_quote_doubles_every_quote() {
        assert_eq!(escape_double_quote("say \"hi\""), "say \"\"hi\"\"");
        assert_eq!(escape_double_quote("plain"), "plain");
    }

    #[test]
    fn parameter_splits_key_equals_value() {
        assert_eq!(parameter("tokenize=porter"), Some(("tokenize", "porter")));
        assert_eq!(
            parameter("tokenize = 'porter'"),
            Some(("tokenize", "'porter'"))
        );
        assert_eq!(parameter("no_equals_sign"), None);
    }

    #[test]
    fn parse_boolean_accepts_common_spellings() {
        for truthy in ["1", "true", "TRUE", "yes", "on"] {
            assert_eq!(parse_boolean(truthy), Some(true), "{truthy}");
        }
        for falsy in ["0", "false", "FALSE", "no", "off"] {
            assert_eq!(parse_boolean(falsy), Some(false), "{falsy}");
        }
        assert_eq!(parse_boolean("maybe"), None);
    }
}
