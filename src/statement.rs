//! `Statement`: a prepared, reusable SQL statement (Part B gap rows
//! "Statement: execution", "Statement: querying", "Statement: column
//! introspection").
//!
//! **Scope, stated plainly:** real `rusqlite::Statement::execute`/`query*`
//! always take a `params: impl Params` argument — even a parameter-free
//! statement is called as `stmt.execute([])`. This crate's tokenizer
//! doesn't recognize `?`/`:name` parameter markers at all yet (the same
//! blocker flagged in issue #25: representing them needs a parser-level
//! AST decision that would change the already-shipped `Insert::rows`
//! field), so [`Statement`] only supports parameter-free SQL — there's no
//! `params` argument to plumb through because nothing can bind into one
//! yet. What *is* real here: `Connection::prepare` tokenizes/parses once,
//! and [`Statement::execute`]/[`Statement::query_map`] reuse that parsed
//! form on every call, skipping re-tokenizing/re-parsing — the actual
//! performance point of a prepared statement, independent of parameter
//! binding.
//!
//! **`columns`/`columns_with_metadata`/`column_metadata` aren't
//! provided:** in real `rusqlite`, all three are behind opt-in Cargo
//! features (`column_decltype`/`column_metadata`), not part of the
//! default API surface this crate targets. `column_metadata` in
//! particular returns a raw `&CStr`-tuple straight out of SQLite's C
//! API, which has no honest equivalent in a from-scratch engine with no
//! C interop. [`Statement::column_names`]/[`Statement::column_name`]/
//! [`Statement::column_index`]/[`Statement::column_count`] (all part of
//! the default surface) cover the rest of column introspection.
//!
//! **Also out of scope for now:** unlike [`crate::Connection::execute`],
//! [`Statement::execute`] doesn't fire `trace`/`profile`/`commit_hook`/
//! `update_hook`/the authorizer, or update `last_insert_rowid`/`changes`/
//! `total_changes`. Wiring a prepared statement into the same hook
//! machinery `execute` uses is real, valuable work — left for a
//! deliberate follow-up rather than folded into an already-large first
//! cut. [`Statement::execute`] does still respect
//! [`crate::OpenFlags::READ_ONLY`] and persist to a file-backed
//! connection (see [`crate::Connection::open`]), since those are
//! correctness guarantees, not observability.

use crate::connection::{leading_keyword, Connection};
use crate::ddl::{parse_create_table, CreateTable};
use crate::dml_insert::{parse_insert, Insert};
use crate::dml_select::{parse_select, Select, SelectColumns};
use crate::engine::{
    describe_aggregate_call, execute_create_table, execute_insert_returning_rowids,
};
use crate::error::{Error, Result};
use crate::row::Row;
use crate::rows::{AndThenRows, Rows};
use crate::token::tokenize;
use crate::value::Value;

enum StatementKind {
    CreateTable(CreateTable),
    Insert(Insert),
    Select(Select),
}

/// A prepared, reusable SQL statement, created via [`Connection::prepare`].
pub struct Statement<'conn> {
    conn: &'conn mut Connection,
    kind: StatementKind,
    sql: String,
    /// The most recent [`Statement::query`]/[`Statement::raw_query`]
    /// result set, kept alive on `self` so the [`Rows`] handed back can
    /// borrow from it instead of the query needing to return owned data.
    last_result: Option<(Vec<String>, Vec<Vec<Value>>)>,
}

impl<'conn> Statement<'conn> {
    pub(crate) fn prepare(conn: &'conn mut Connection, sql: &str) -> Result<Statement<'conn>> {
        let tokens = tokenize(sql)?;
        let kind = match leading_keyword(&tokens) {
            Some(kw) if kw.eq_ignore_ascii_case("CREATE") => {
                StatementKind::CreateTable(parse_create_table(&tokens)?)
            }
            Some(kw) if kw.eq_ignore_ascii_case("INSERT") => {
                StatementKind::Insert(parse_insert(&tokens)?)
            }
            Some(kw) if kw.eq_ignore_ascii_case("SELECT") => {
                StatementKind::Select(parse_select(&tokens)?)
            }
            _ => return Err(Error::UnrecognizedStatement(sql.to_string())),
        };
        Ok(Statement {
            conn,
            kind,
            sql: sql.to_string(),
            last_result: None,
        })
    }

    /// Runs this statement (`CREATE TABLE`/`INSERT`), returning the number
    /// of rows affected (`0` for `CREATE TABLE`). Errors if this is a
    /// `SELECT` — use [`Statement::query_map`]/[`Statement::query_row`]/
    /// [`Statement::query_one`] instead.
    pub fn execute(&mut self) -> Result<usize> {
        if self.conn.is_readonly("main")? {
            return Err(Error::ReadOnlyConnection);
        }
        let affected = match &self.kind {
            StatementKind::CreateTable(create) => {
                execute_create_table(self.conn.db_mut(), create)?;
                0
            }
            StatementKind::Insert(insert) => {
                execute_insert_returning_rowids(self.conn.db_mut(), insert)?.len()
            }
            StatementKind::Select(_) => {
                return Err(Error::UnrecognizedStatement(
                    "execute() called on a SELECT statement -- use query*() instead".to_string(),
                ))
            }
        };
        self.conn.flush()?;
        Ok(affected)
    }

    fn select(&self) -> Result<&Select> {
        match &self.kind {
            StatementKind::Select(select) => Ok(select),
            _ => Err(Error::UnrecognizedStatement(
                "query*() called on a non-SELECT statement -- use execute() instead".to_string(),
            )),
        }
    }

    /// Runs this `SELECT`, mapping every matching row through `f`.
    pub fn query_map<T, F>(&self, mut f: F) -> Result<Vec<T>>
    where
        F: FnMut(Row<'_>) -> Result<T>,
    {
        let (columns, rows) = self.conn.run_select(self.select()?)?;
        rows.iter()
            .map(|values| f(Row::new(&columns, values)))
            .collect()
    }

    /// Runs this `SELECT`, expecting exactly one row, returning its
    /// values in result-column order. Errors with
    /// [`Error::QueryReturnedNoRows`] if no row matched.
    pub fn query_row(&self) -> Result<Vec<Value>> {
        let (_, mut rows) = self.conn.run_select(self.select()?)?;
        if rows.is_empty() {
            return Err(Error::QueryReturnedNoRows);
        }
        Ok(rows.remove(0))
    }

    /// Like [`Statement::query_row`], but maps the single matching row
    /// through `f` instead of returning its raw values.
    pub fn query_one<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Row<'_>) -> Result<T>,
    {
        let (columns, mut rows) = self.conn.run_select(self.select()?)?;
        if rows.is_empty() {
            return Err(Error::QueryReturnedNoRows);
        }
        let values = rows.remove(0);
        f(Row::new(&columns, &values))
    }

    /// Runs this `SELECT`, returning a lazy [`Rows`] iterator over the
    /// result set. Unlike [`Statement::query_map`] (which eagerly
    /// collects into a `Vec`), this is the same shape as real
    /// `rusqlite::Statement::query`.
    pub fn query(&mut self) -> Result<Rows<'_>> {
        let result = self.conn.run_select(self.select()?)?;
        self.last_result = Some(result);
        let (columns, rows) = self.last_result.as_ref().expect("just assigned Some above");
        Ok(Rows::new(columns, rows))
    }

    /// Like [`Statement::query`], with each row mapped through a
    /// fallible-in-any-error-type closure — see [`AndThenRows`].
    pub fn query_and_then<T, E, F>(&mut self, f: F) -> Result<AndThenRows<'_, F>>
    where
        F: FnMut(Row<'_>) -> std::result::Result<T, E>,
        E: From<Error>,
    {
        Ok(self.query()?.and_then(f))
    }

    /// Runs this `SELECT`, returning whether it matched at least one row.
    pub fn exists(&self) -> Result<bool> {
        let (_, rows) = self.conn.run_select(self.select()?)?;
        Ok(!rows.is_empty())
    }

    /// Like [`Statement::query`]. Real `rusqlite::Statement::raw_query`
    /// skips the params-binding step `query` otherwise requires; since
    /// [`Statement`] has no parameter binding to skip (see this module's
    /// doc comment), the two are identical here — kept as a separate
    /// method purely for name-level parity with call sites migrating
    /// from `rusqlite`.
    pub fn raw_query(&mut self) -> Result<Rows<'_>> {
        self.query()
    }

    /// This statement's result-column names, in order. Errors if this
    /// isn't a `SELECT`.
    pub fn column_names(&self) -> Result<Vec<String>> {
        match &self.select()?.columns {
            SelectColumns::All => Ok(self
                .conn
                .db()
                .table(&self.select()?.table_name)?
                .column_names
                .clone()),
            SelectColumns::Named(names) => Ok(names.clone()),
            SelectColumns::Aggregates(calls) => {
                Ok(calls.iter().map(describe_aggregate_call).collect())
            }
        }
    }

    /// The number of columns in this statement's result set. Errors if
    /// this isn't a `SELECT`.
    pub fn column_count(&self) -> Result<usize> {
        Ok(self.column_names()?.len())
    }

    /// The name of the result column at `index`. Errors if this isn't a
    /// `SELECT`, or if `index` is out of range.
    pub fn column_name(&self, index: usize) -> Result<String> {
        let names = self.column_names()?;
        let len = names.len();
        names
            .into_iter()
            .nth(index)
            .ok_or(Error::IndexOutOfBounds { index, len })
    }

    /// The position of the result column named `name`. Errors if this
    /// isn't a `SELECT`, or if no result column has that name.
    pub fn column_index(&self, name: &str) -> Result<usize> {
        let names = self.column_names()?;
        names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error::UnknownColumn(name.to_string()))
    }

    /// Returns whether this statement is a `SELECT` (and so is run via
    /// [`Statement::query_map`]/[`Statement::query_row`]/
    /// [`Statement::query_one`] rather than [`Statement::execute`]).
    pub fn is_query(&self) -> bool {
        matches!(self.kind, StatementKind::Select(_))
    }

    /// The number of `?`/`:name`-style parameters in this statement.
    /// Always `0` — [`Statement`] doesn't support parameter binding yet
    /// (see this module's doc comment), so no statement can have any.
    pub fn parameter_count(&self) -> usize {
        0
    }

    /// The name of the parameter at `index` (1-based, matching SQLite's
    /// own convention), if any. Always `None` — see
    /// [`Statement::parameter_count`].
    pub fn parameter_name(&self, _index: usize) -> Option<&str> {
        None
    }

    /// The index of the parameter named `name`, if this statement has
    /// one. Always `Ok(None)` — see [`Statement::parameter_count`].
    pub fn parameter_index(&self, _name: &str) -> Result<Option<usize>> {
        Ok(None)
    }

    /// This statement's original SQL text. Real
    /// `rusqlite::Statement::expanded_sql` substitutes bound parameter
    /// values into the text; since [`Statement`] never has any bound
    /// parameters to substitute (see this module's doc comment), this is
    /// always just the SQL [`Connection::prepare`] was given.
    pub fn expanded_sql(&self) -> Option<String> {
        Some(self.sql.clone())
    }

    /// Returns whether this statement can't modify the database — `true`
    /// for a `SELECT`, `false` for `CREATE TABLE`/`INSERT`.
    pub fn readonly(&self) -> bool {
        self.is_query()
    }

    /// Returns whether this statement is `EXPLAIN`/`EXPLAIN QUERY PLAN`
    /// (`0` = neither, `1` = `EXPLAIN`, `2` = `EXPLAIN QUERY PLAN`,
    /// matching `sqlite3_stmt_isexplain`'s convention). Always `0` — this
    /// crate's parser doesn't recognize the `EXPLAIN` keyword at all yet,
    /// so no statement can be one.
    pub fn is_explain(&self) -> i32 {
        0
    }

    /// A per-statement execution counter, as SQLite's
    /// `sqlite3_stmt_status` would report. Always `0` — this crate's
    /// engine has no virtual machine (see `ARCHITECTURE.md`) to count
    /// fetch/sort/index/etc. operations for; stored as an honest `0`
    /// rather than omitted, matching the "not enforced, not silently
    /// dropped" treatment already given to `Connection::busy_timeout`.
    pub fn get_status(&self, _status: StatementStatus) -> i32 {
        0
    }

    /// The mirror of [`Statement::get_status`]: resets its counters. A
    /// no-op, for the same reason `get_status` always reports `0`.
    pub fn reset_status(&self) {}

    /// Finalizes this statement, consuming it. A no-op beyond dropping
    /// the guard — there's no separate C-level statement handle to
    /// release, so this exists purely for call-site parity with real
    /// `rusqlite`.
    pub fn finalize(self) -> Result<()> {
        Ok(())
    }
}

/// A counter [`Statement::get_status`] would report on, mirroring
/// SQLite's `SQLITE_STMTSTATUS_*` constants. Inert scaffolding today —
/// see [`Statement::get_status`]'s doc comment for why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementStatus {
    FullscanStep,
    Sort,
    AutoIndex,
    VmStep,
    RunExplainQueryPlan,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_and_executes_create_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        let mut stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(!stmt.is_query());
        assert_eq!(stmt.execute().unwrap(), 0);
        assert!(conn.table_exists("t"));
    }

    #[test]
    fn prepared_insert_is_reusable_across_multiple_executes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(stmt.execute().unwrap(), 1);
        assert_eq!(stmt.execute().unwrap(), 1);
        assert_eq!(stmt.execute().unwrap(), 1);

        let values: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 1, 1]);
    }

    #[test]
    fn execute_on_select_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert!(stmt.execute().is_err());
    }

    #[test]
    fn query_map_on_non_select_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        let stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(stmt.query_map(|row: Row<'_>| row.get::<i64>(0)).is_err());
    }

    #[test]
    fn query_map_runs_the_prepared_select() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let stmt = conn.prepare("SELECT * FROM t WHERE a = 2").unwrap();
        let values: Vec<i64> = stmt.query_map(|row| row.get(0)).unwrap();
        assert_eq!(values, vec![2]);
    }

    #[test]
    fn query_row_and_query_one_work() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (7)").unwrap();

        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.query_row().unwrap(), vec![Value::Integer(7)]);

        let doubled: i64 = stmt
            .query_one(|row| row.get::<i64>(0).map(|n| n * 2))
            .unwrap();
        assert_eq!(doubled, 14);
    }

    #[test]
    fn query_row_with_no_matches_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.query_row(), Err(Error::QueryReturnedNoRows));
    }

    #[test]
    fn column_names_for_select_star() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.column_names().unwrap(), vec!["a", "b"]);
        assert_eq!(stmt.column_count().unwrap(), 2);
        assert_eq!(stmt.column_name(1).unwrap(), "b");
    }

    #[test]
    fn column_names_for_named_columns() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn.prepare("SELECT b FROM t").unwrap();
        assert_eq!(stmt.column_names().unwrap(), vec!["b"]);
    }

    #[test]
    fn column_names_for_aggregate_select() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT COUNT(*), SUM(a) FROM t").unwrap();
        assert_eq!(stmt.column_names().unwrap(), vec!["COUNT(*)", "SUM(a)"]);
    }

    #[test]
    fn column_name_out_of_range_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(
            stmt.column_name(5),
            Err(Error::IndexOutOfBounds { index: 5, len: 1 })
        );
    }

    #[test]
    fn column_names_on_non_select_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        let stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(stmt.column_names().is_err());
    }

    #[test]
    fn execute_on_read_only_connection_is_an_error() {
        let mut conn = Connection::open_in_memory_with_flags(crate::OpenFlags::READ_ONLY).unwrap();
        assert!(conn.prepare("CREATE TABLE t (a INTEGER)").is_ok());
        let mut stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert_eq!(stmt.execute(), Err(Error::ReadOnlyConnection));
    }

    #[test]
    fn query_returns_a_lazy_rows_iterator() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        let values: Result<Vec<i64>> = stmt
            .query()
            .unwrap()
            .map(|r| r.and_then(|row| row.get::<i64>(0)))
            .collect();
        assert_eq!(values.unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn raw_query_behaves_like_query() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (5)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        let values: Result<Vec<i64>> = stmt
            .raw_query()
            .unwrap()
            .map(|r| r.and_then(|row| row.get::<i64>(0)))
            .collect();
        assert_eq!(values.unwrap(), vec![5]);
    }

    #[test]
    fn query_and_then_propagates_custom_errors() {
        #[derive(Debug, PartialEq)]
        enum MyError {
            Inner(Error),
            TooBig,
        }
        impl From<Error> for MyError {
            fn from(e: Error) -> MyError {
                MyError::Inner(e)
            }
        }

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (5)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        let result: std::result::Result<Vec<i64>, MyError> = stmt
            .query_and_then(|row| {
                let n = row.get::<i64>(0)?;
                if n > 3 {
                    Err(MyError::TooBig)
                } else {
                    Ok(n)
                }
            })
            .unwrap()
            .collect();
        assert_eq!(result, Err(MyError::TooBig));
    }

    #[test]
    fn exists_reflects_whether_any_row_matched() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        assert!(conn
            .prepare("SELECT * FROM t WHERE a = 1")
            .unwrap()
            .exists()
            .unwrap());
        assert!(!conn
            .prepare("SELECT * FROM t WHERE a = 2")
            .unwrap()
            .exists()
            .unwrap());
    }

    #[test]
    fn column_index_finds_a_named_column() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.column_index("b").unwrap(), 1);
        assert!(stmt.column_index("missing").is_err());
    }

    #[test]
    fn parameter_introspection_always_reports_none_or_zero() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();

        assert_eq!(stmt.parameter_count(), 0);
        assert_eq!(stmt.parameter_name(0), None);
        assert_eq!(stmt.parameter_index("anything").unwrap(), None);
    }

    #[test]
    fn expanded_sql_returns_the_original_text() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE a = 1").unwrap();
        assert_eq!(
            stmt.expanded_sql(),
            Some("SELECT * FROM t WHERE a = 1".to_string())
        );
    }

    #[test]
    fn readonly_distinguishes_select_from_mutating_statements() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let select = conn.prepare("SELECT * FROM t").unwrap();
        assert!(select.readonly());

        let create = conn.prepare("CREATE TABLE t2 (a INTEGER)").unwrap();
        assert!(!create.readonly());

        let insert = conn.prepare("INSERT INTO t VALUES (1)").unwrap();
        assert!(!insert.readonly());
    }

    #[test]
    fn is_explain_is_always_false() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.is_explain(), 0);
    }

    #[test]
    fn status_is_inert() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.get_status(StatementStatus::FullscanStep), 0);
        stmt.reset_status();
        assert_eq!(stmt.get_status(StatementStatus::Sort), 0);
    }

    #[test]
    fn finalize_consumes_the_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert!(stmt.finalize().is_ok());
    }
}
