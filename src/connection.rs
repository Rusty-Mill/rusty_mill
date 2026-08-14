use crate::ddl::{parse_create_table, ColumnDef};
use crate::dml_insert::parse_insert;
use crate::dml_select::parse_select;
use crate::engine::{execute_create_table, execute_insert, execute_select};
use crate::error::{Error, Result};
use crate::row::Row;
use crate::storage::Database;
use crate::token::{tokenize, Token};
use crate::value::Value;

/// A table column's schema, as returned by [`Connection::column_metadata`].
/// A subset of `rusqlite`'s equivalent (no collation sequence or
/// auto-increment flag — this crate doesn't track either yet).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnMetadata {
    pub type_name: Option<String>,
    pub not_null: bool,
    pub primary_key: bool,
}

impl From<&ColumnDef> for ColumnMetadata {
    fn from(def: &ColumnDef) -> ColumnMetadata {
        ColumnMetadata {
            type_name: def.type_name.clone(),
            not_null: def.not_null,
            primary_key: def.primary_key,
        }
    }
}

/// A connection to a database.
///
/// Currently supports only an in-memory backend. `execute`/`execute_batch`
/// recognize `CREATE TABLE` and `INSERT`; `query_row`/`query_one`/
/// `query_map` recognize `SELECT`. `prepare*` (returning a reusable,
/// bindable `Statement`) isn't implemented yet — it's blocked on the same
/// parameter-marker design decision as issue #25 (see that issue's
/// comments): binding `?`-style parameters requires the parser to
/// represent them in the AST, which isn't decided yet. The full
/// `rusqlite`-shaped `Statement` API is tracked separately as
/// `parity-gap` issues in `gap-analysis.md`'s Part B.
pub struct Connection {
    db: Database,
    open: bool,
    last_changes: usize,
    total_changes: usize,
}

impl Connection {
    /// Opens a new in-memory connection.
    pub fn open_in_memory() -> Result<Connection> {
        Ok(Connection {
            db: Database::new(),
            open: true,
            last_changes: 0,
            total_changes: 0,
        })
    }

    /// Returns the path to the database file, or `None` for an in-memory
    /// connection. Always `None` today — this crate has no on-disk
    /// backend yet (see `ARCHITECTURE.md`'s non-goals).
    pub fn path(&self) -> Option<&str> {
        None
    }

    /// Returns whether the connection is currently in autocommit mode
    /// (i.e. not inside an explicit transaction). Always `true` today —
    /// explicit transactions aren't implemented yet (tracked as a
    /// separate `parity-gap` issue).
    pub fn is_autocommit(&self) -> bool {
        true
    }

    /// Returns whether the connection currently has a statement mid-step
    /// (i.e. locked by an unfinished query). Always `false` today — this
    /// crate's queries run to completion synchronously, so there's no
    /// mid-step state to be busy in.
    pub fn is_busy(&self) -> bool {
        false
    }

    /// Returns whether `db_name` (only `"main"` exists) is read-only.
    /// Always `Ok(false)` for `"main"` — there's no read-only-open mode
    /// yet.
    pub fn is_readonly(&self, db_name: &str) -> Result<bool> {
        self.require_main_database(db_name)?;
        Ok(false)
    }

    /// Returns whether the connection's current operation has been
    /// interrupted. Always `false` today — there's no interrupt handle
    /// (`Connection::get_interrupt_handle`) to trigger one yet.
    pub fn is_interrupted(&self) -> bool {
        false
    }

    /// Returns the name of the database at `index` (`0` is always
    /// `"main"`). Errors for any other index — this crate has no
    /// `ATTACH` support, so no other database ever exists.
    pub fn db_name(&self, index: usize) -> Result<String> {
        if index == 0 {
            Ok("main".to_string())
        } else {
            Err(Error::NoSuchDatabase(format!("index {index}")))
        }
    }

    /// Returns whether `table` has a column named `column`.
    pub fn column_exists(&self, table: &str, column: &str) -> Result<bool> {
        let table = self.db.table(table)?;
        Ok(table.column_names.iter().any(|c| c == column))
    }

    /// Returns whether `table` exists.
    pub fn table_exists(&self, table: &str) -> bool {
        self.db.table(table).is_ok()
    }

    /// Returns `column`'s schema within `table`.
    pub fn column_metadata(&self, table: &str, column: &str) -> Result<ColumnMetadata> {
        let table = self.db.table(table)?;
        table
            .columns
            .iter()
            .find(|c| c.name == column)
            .map(ColumnMetadata::from)
            .ok_or_else(|| Error::UnknownColumn(column.to_string()))
    }

    /// Returns the number of rows changed by the most recent
    /// `execute`/`execute_batch` call (`0` for `CREATE TABLE`, matching
    /// `execute`'s own return value for that statement type).
    pub fn changes(&self) -> usize {
        self.last_changes
    }

    /// Returns the cumulative number of rows changed since this
    /// connection was opened.
    pub fn total_changes(&self) -> usize {
        self.total_changes
    }

    fn require_main_database(&self, db_name: &str) -> Result<()> {
        if db_name == "main" {
            Ok(())
        } else {
            Err(Error::NoSuchDatabase(db_name.to_string()))
        }
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
    /// number of rows affected (`0` for `CREATE TABLE`). Updates
    /// [`Connection::changes`]/[`Connection::total_changes`].
    pub fn execute(&mut self, sql: &str) -> Result<usize> {
        self.check_open()?;
        let tokens = tokenize(sql)?;
        let affected = match leading_keyword(&tokens) {
            Some(kw) if kw.eq_ignore_ascii_case("CREATE") => {
                let create = parse_create_table(&tokens)?;
                execute_create_table(&mut self.db, &create)?;
                0
            }
            Some(kw) if kw.eq_ignore_ascii_case("INSERT") => {
                let insert = parse_insert(&tokens)?;
                execute_insert(&mut self.db, &insert)?
            }
            _ => return Err(Error::UnrecognizedStatement(sql.to_string())),
        };
        self.last_changes = affected;
        self.total_changes += affected;
        Ok(affected)
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

    /// Like [`Connection::query_row`], but maps the single matching row
    /// through `f` instead of returning its raw values.
    pub fn query_one<T, F>(&self, sql: &str, f: F) -> Result<T>
    where
        F: FnOnce(Row<'_>) -> Result<T>,
    {
        self.check_open()?;
        let tokens = tokenize(sql)?;
        let select = parse_select(&tokens)?;
        let (columns, mut rows) = execute_select(&self.db, &select)?;
        if rows.is_empty() {
            return Err(Error::QueryReturnedNoRows);
        }
        let values = rows.remove(0);
        f(Row::new(&columns, &values))
    }

    /// Executes a `SELECT`, mapping every matching row through `f` and
    /// collecting the results.
    pub fn query_map<T, F>(&self, sql: &str, mut f: F) -> Result<Vec<T>>
    where
        F: FnMut(Row<'_>) -> Result<T>,
    {
        self.check_open()?;
        let tokens = tokenize(sql)?;
        let select = parse_select(&tokens)?;
        let (columns, rows) = execute_select(&self.db, &select)?;
        rows.iter()
            .map(|values| f(Row::new(&columns, values)))
            .collect()
    }

    /// Executes each `;`-separated statement in `sql` in turn via
    /// [`Connection::execute`]. Unlike `rusqlite::Connection::execute_batch`,
    /// this crate's tokenizer doesn't yet split on `;` inside string
    /// literals containing the character — not a concern for the
    /// statement types currently supported (`CREATE TABLE`/`INSERT`
    /// literals are simple enough that this hasn't come up), but worth
    /// revisiting if a future statement type's literals can contain `;`.
    pub fn execute_batch(&mut self, sql: &str) -> Result<()> {
        self.check_open()?;
        for statement in sql.split(';') {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            self.execute(statement)?;
        }
        Ok(())
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

    #[test]
    fn query_one_maps_single_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (7)").unwrap();
        let doubled: i64 = conn
            .query_one("SELECT * FROM t", |row| row.get::<i64>(0).map(|n| n * 2))
            .unwrap();
        assert_eq!(doubled, 14);
    }

    #[test]
    fn query_map_collects_all_matching_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();
        let values: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn execute_batch_runs_each_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (a INTEGER); INSERT INTO t VALUES (1); INSERT INTO t VALUES (2);",
        )
        .unwrap();
        let values: Vec<i64> = conn
            .query_map("SELECT * FROM t", |row| row.get::<i64>(0))
            .unwrap();
        assert_eq!(values, vec![1, 2]);
    }

    #[test]
    fn metadata_defaults_reflect_no_transaction_no_disk() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.path(), None);
        assert!(conn.is_autocommit());
        assert!(!conn.is_busy());
        assert!(!conn.is_interrupted());
        assert!(!conn.is_readonly("main").unwrap());
        assert_eq!(conn.db_name(0).unwrap(), "main");
        assert!(matches!(conn.db_name(1), Err(Error::NoSuchDatabase(_))));
    }

    #[test]
    fn table_and_column_existence_checks() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(conn.table_exists("t"));
        assert!(!conn.table_exists("missing"));
        assert!(conn.column_exists("t", "a").unwrap());
        assert!(!conn.column_exists("t", "z").unwrap());
        assert!(conn.column_exists("missing", "a").is_err());
    }

    #[test]
    fn column_metadata_reflects_declared_constraints() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        let id_meta = conn.column_metadata("t", "id").unwrap();
        assert!(id_meta.primary_key);
        assert!(!id_meta.not_null);
        assert_eq!(id_meta.type_name, Some("INTEGER".to_string()));

        let name_meta = conn.column_metadata("t", "name").unwrap();
        assert!(name_meta.not_null);
        assert!(!name_meta.primary_key);
    }

    #[test]
    fn changes_and_total_changes_track_execute() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert_eq!(conn.changes(), 0);
        assert_eq!(conn.total_changes(), 0);

        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();
        assert_eq!(conn.changes(), 2);
        assert_eq!(conn.total_changes(), 2);

        conn.execute("INSERT INTO t VALUES (3)").unwrap();
        assert_eq!(conn.changes(), 1);
        assert_eq!(conn.total_changes(), 3);
    }
}
