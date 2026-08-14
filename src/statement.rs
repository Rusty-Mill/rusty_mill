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
        Ok(Statement { conn, kind })
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

    /// Returns whether this statement is a `SELECT` (and so is run via
    /// [`Statement::query_map`]/[`Statement::query_row`]/
    /// [`Statement::query_one`] rather than [`Statement::execute`]).
    pub fn is_query(&self) -> bool {
        matches!(self.kind, StatementKind::Select(_))
    }
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
}
