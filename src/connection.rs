use crate::ddl::parse_create_table;
use crate::dml_insert::parse_insert;
use crate::dml_select::parse_select;
use crate::engine::{execute_create_table, execute_insert, execute_select};
use crate::error::{Error, Result};
use crate::storage::Database;
use crate::token::{tokenize, Token};
use crate::value::Value;

/// A connection to a database.
///
/// Currently supports only an in-memory backend. `execute` recognizes
/// `CREATE TABLE` and `INSERT`; `query_row` recognizes `SELECT`. The full
/// `rusqlite`-shaped `Statement`/`Row` API is tracked separately as
/// `parity-gap` issues in `gap-analysis.md`'s Part B.
pub struct Connection {
    db: Database,
    open: bool,
}

impl Connection {
    /// Opens a new in-memory connection.
    pub fn open_in_memory() -> Result<Connection> {
        Ok(Connection {
            db: Database::new(),
            open: true,
        })
    }

    /// Returns whether the connection is still open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Closes the connection.
    pub fn close(mut self) -> Result<()> {
        self.check_open()?;
        self.open = false;
        Ok(())
    }

    /// Executes a `CREATE TABLE` or `INSERT` statement, returning the
    /// number of rows affected (`0` for `CREATE TABLE`).
    pub fn execute(&mut self, sql: &str) -> Result<usize> {
        self.check_open()?;
        let tokens = tokenize(sql)?;
        match leading_keyword(&tokens) {
            Some(kw) if kw.eq_ignore_ascii_case("CREATE") => {
                let create = parse_create_table(&tokens)?;
                execute_create_table(&mut self.db, &create)?;
                Ok(0)
            }
            Some(kw) if kw.eq_ignore_ascii_case("INSERT") => {
                let insert = parse_insert(&tokens)?;
                execute_insert(&mut self.db, &insert)
            }
            _ => Err(Error::UnrecognizedStatement(sql.to_string())),
        }
    }

    /// Executes a `SELECT` expected to return exactly one row, returning
    /// that row's values in the statement's result-column order. Errors
    /// with [`Error::QueryReturnedNoRows`] if the query matched no rows.
    pub fn query_row(&self, sql: &str) -> Result<Vec<Value>> {
        self.check_open()?;
        let tokens = tokenize(sql)?;
        let select = parse_select(&tokens)?;
        let (_, mut rows) = execute_select(&self.db, &select)?;
        if rows.is_empty() {
            return Err(Error::QueryReturnedNoRows);
        }
        Ok(rows.remove(0))
    }

    fn check_open(&self) -> Result<()> {
        if !self.open {
            return Err(Error::ConnectionClosed);
        }
        Ok(())
    }
}

fn leading_keyword(tokens: &[Token]) -> Option<&str> {
    match tokens.first() {
        Some(Token::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_starts_open() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.is_open());
    }

    #[test]
    fn close_marks_connection_closed() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.close().is_ok());
    }

    #[test]
    fn execute_and_query_row_round_trip() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let affected = conn.execute("INSERT INTO t VALUES (1, 'x')").unwrap();
        assert_eq!(affected, 1);

        let row = conn.query_row("SELECT * FROM t WHERE a = 1").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Text("x".into())]);
    }

    #[test]
    fn query_row_with_no_matches_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert_eq!(
            conn.query_row("SELECT * FROM t WHERE a = 1"),
            Err(Error::QueryReturnedNoRows)
        );
    }

    #[test]
    fn execute_on_unrecognized_statement_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            conn.execute("DROP TABLE t"),
            Err(Error::UnrecognizedStatement(_))
        ));
    }
}
