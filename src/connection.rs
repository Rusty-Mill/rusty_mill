use crate::config::{DbConfig, Limit};
use crate::ddl::{parse_create_table, ColumnDef};
use crate::dml_insert::parse_insert;
use crate::dml_select::parse_select;
use crate::engine::{execute_create_table, execute_insert, execute_select};
use crate::error::{Error, Result};
use crate::row::Row;
use crate::storage::Database;
use crate::token::{tokenize, Token};
use crate::value::Value;
use std::collections::HashMap;

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
    db_config: HashMap<DbConfig, bool>,
    limits: HashMap<Limit, i32>,
    errmsg: Option<String>,
    busy_timeout: Option<std::time::Duration>,
    busy_handler: Option<fn(i32) -> bool>,
}

impl Connection {
    /// Opens a new in-memory connection.
    pub fn open_in_memory() -> Result<Connection> {
        Ok(Connection {
            db: Database::new(),
            open: true,
            last_changes: 0,
            total_changes: 0,
            db_config: HashMap::new(),
            limits: HashMap::new(),
            errmsg: None,
            busy_timeout: None,
            busy_handler: None,
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

    /// Returns whether `config` is currently enabled. Defaults to `false`
    /// for any option that hasn't been set. **Not enforced**: setting
    /// `EnableForeignKeys`, for example, doesn't make the engine actually
    /// check foreign keys yet — there's no foreign-key constraint
    /// tracking in the storage layer to enforce. Stored honestly as a
    /// flag, not silently ignored, so a future PR that adds real
    /// enforcement has something to read.
    pub fn db_config(&self, config: DbConfig) -> bool {
        self.db_config.get(&config).copied().unwrap_or(false)
    }

    /// Sets `config`'s enabled state. See [`Connection::db_config`] for
    /// what "not enforced yet" means here.
    pub fn set_db_config(&mut self, config: DbConfig, enabled: bool) -> Result<()> {
        self.db_config.insert(config, enabled);
        Ok(())
    }

    /// Returns `limit`'s current value, or `-1` if it hasn't been set
    /// (matching SQLite's convention that a negative limit means
    /// "unset"/"query current value only"). **Not enforced**: no
    /// operation currently checks these limits before proceeding.
    pub fn limit(&self, limit: Limit) -> i32 {
        self.limits.get(&limit).copied().unwrap_or(-1)
    }

    /// Sets `limit`'s value, returning its previous value.
    pub fn set_limit(&mut self, limit: Limit, value: i32) -> i32 {
        let previous = self.limit(limit);
        self.limits.insert(limit, value);
        previous
    }

    /// No-op: there's no prepared-statement cache yet — `prepare_cached`
    /// isn't implemented (it needs a real `Statement` type, tracked
    /// separately; see the note on `prepare*` above).
    pub fn set_prepared_statement_cache_capacity(&mut self, _capacity: usize) {}

    /// No-op, for the same reason as
    /// [`Connection::set_prepared_statement_cache_capacity`].
    pub fn flush_prepared_statement_cache(&mut self) {}

    /// No-op: this engine has no page cache to flush (see
    /// `ARCHITECTURE.md` — storage is a plain in-memory `HashMap`, not a
    /// paged cache over a file).
    pub fn cache_flush(&self) -> Result<()> {
        Ok(())
    }

    /// Sets a custom error message on the connection. In real SQLite,
    /// this is how a custom function/virtual-table implementation
    /// attaches a detailed message to the error SQLite itself will
    /// report next. This crate has no such C-level error-reporting path
    /// for custom functions/vtabs to hook into (neither exists yet), so
    /// unlike `rusqlite::Connection::set_errmsg` this is paired with a
    /// getter ([`Connection::errmsg`]) — otherwise a set value would be
    /// unobservable and this method pointless.
    pub fn set_errmsg(&mut self, msg: &str) {
        self.errmsg = Some(msg.to_string());
    }

    /// Returns the message most recently set via
    /// [`Connection::set_errmsg`], if any.
    pub fn errmsg(&self) -> Option<&str> {
        self.errmsg.as_deref()
    }

    /// Sets how long a busy operation would wait before giving up.
    /// **Never actually waited on**: this crate's single-writer in-memory
    /// model has no lock contention to wait out — there's nothing that
    /// would ever make [`Connection::is_busy`] observe `true`, so this
    /// value is stored but never consulted. Stored honestly rather than
    /// silently ignored, same reasoning as `db_config`/`limit`.
    pub fn busy_timeout(&mut self, timeout: std::time::Duration) -> Result<()> {
        self.busy_timeout = Some(timeout);
        Ok(())
    }

    /// Sets a callback to run when a busy operation would otherwise
    /// block. Same caveat as [`Connection::busy_timeout`]: never actually
    /// invoked, since nothing in this engine blocks.
    pub fn busy_handler(&mut self, callback: Option<fn(i32) -> bool>) -> Result<()> {
        self.busy_handler = callback;
        Ok(())
    }

    /// Snapshots table state for [`crate::Transaction`]/[`crate::Savepoint`]
    /// rollback support.
    pub(crate) fn snapshot_db(&self) -> std::collections::HashMap<String, crate::storage::Table> {
        self.db.snapshot()
    }

    /// Restores table state previously captured by
    /// [`Connection::snapshot_db`].
    pub(crate) fn restore_db(
        &mut self,
        snapshot: std::collections::HashMap<String, crate::storage::Table>,
    ) {
        self.db.restore(snapshot);
    }

    /// Begins a transaction, returning a guard that rolls back on drop
    /// unless [`crate::Transaction::commit`] is called first (or its drop
    /// behavior is changed via [`crate::Transaction::set_drop_behavior`]).
    pub fn transaction(&mut self) -> Result<crate::transaction::Transaction<'_>> {
        crate::transaction::Transaction::new(self)
    }

    /// Like [`Connection::transaction`], but the given `behavior` is
    /// accepted for API compatibility only — this crate's single-writer
    /// in-memory model doesn't distinguish `Deferred`/`Immediate`/
    /// `Exclusive` locking, so all three behave identically today.
    pub fn transaction_with_behavior(
        &mut self,
        _behavior: crate::transaction::TransactionBehavior,
    ) -> Result<crate::transaction::Transaction<'_>> {
        crate::transaction::Transaction::new(self)
    }

    /// Like [`Connection::transaction`], but doesn't check whether a
    /// transaction is already active (this crate has no such check to
    /// skip yet — the two are equivalent today, kept as separate methods
    /// for API-shape parity).
    pub fn unchecked_transaction(&mut self) -> Result<crate::transaction::Transaction<'_>> {
        crate::transaction::Transaction::new(self)
    }

    /// Begins a savepoint with an auto-generated name.
    pub fn savepoint(&mut self) -> Result<crate::transaction::Savepoint<'_>> {
        crate::transaction::Savepoint::new(self, None)
    }

    /// Begins a savepoint with the given name.
    pub fn savepoint_with_name(&mut self, name: &str) -> Result<crate::transaction::Savepoint<'_>> {
        crate::transaction::Savepoint::new(self, Some(name.to_string()))
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

    #[test]
    fn db_config_defaults_to_false_and_round_trips() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(!conn.db_config(DbConfig::EnableForeignKeys));
        conn.set_db_config(DbConfig::EnableForeignKeys, true)
            .unwrap();
        assert!(conn.db_config(DbConfig::EnableForeignKeys));
    }

    #[test]
    fn limit_defaults_to_negative_one_and_round_trips() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.limit(Limit::Length), -1);
        let previous = conn.set_limit(Limit::Length, 1000);
        assert_eq!(previous, -1);
        assert_eq!(conn.limit(Limit::Length), 1000);
    }

    #[test]
    fn errmsg_defaults_to_none_and_round_trips() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.errmsg(), None);
        conn.set_errmsg("custom error");
        assert_eq!(conn.errmsg(), Some("custom error"));
    }

    #[test]
    fn busy_timeout_and_handler_are_settable() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        conn.busy_handler(Some(|_retries| false)).unwrap();
        // Never invoked -- there's no blocking path in this engine to
        // invoke them from. This test only confirms both are settable
        // without erroring.
    }
}
