use crate::aggregate::{self, Aggregate};
use crate::config::{DbConfig, Limit, OpenFlags};
use crate::ddl::{
    parse_alter_table, parse_create_index, parse_create_table, parse_create_virtual_table,
    parse_drop_index, parse_drop_table, ColumnDef, CreateVirtualTable,
};
use crate::dml_insert::parse_insert;
use crate::dml_select::{parse_select, SelectColumns};
use crate::engine::{
    execute_alter_table, execute_create_index, execute_create_table, execute_drop_index,
    execute_drop_table, execute_insert_into_virtual_table, execute_insert_returning_rowids,
    execute_select_with_aggregates, execute_select_with_functions, execute_select_with_window,
};
use crate::error::{Error, Result};
use crate::eval::ScalarFn;
use crate::hooks::{Action, AuthContext, Authorization};
use crate::row::Row;
use crate::statement::{StatementCache, DEFAULT_STATEMENT_CACHE_CAPACITY};
use crate::storage::Database;
use crate::token::{tokenize, Token};
use crate::trace::{ConnRef, StmtRef, TraceEvent, TraceEventCodes};
use crate::value::Value;
use crate::vtab::{CreateVTab, CreateVTabModule, VTab, VTabModule, VTabTableSource};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A custom text-comparison function registered via
/// [`Connection::create_collation`].
type CollationFn = dyn Fn(&str, &str) -> Ordering;

type TraceFn = dyn FnMut(&str);
type ProfileFn = dyn FnMut(&str, Duration);
type TraceV2Fn = dyn FnMut(TraceEvent<'_>);
type AuthorizerFn = dyn FnMut(&AuthContext) -> Authorization;
type ProgressHandlerFn = dyn FnMut() -> bool;
type UpdateHookFn = dyn FnMut(Action, &str, &str, i64);

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

/// Interrupts a [`Connection`]'s next `execute`/query call, obtained via
/// [`Connection::get_interrupt_handle`] (issue #107). `Send`/`Sync` —
/// deliberately usable from a different thread than the one holding the
/// [`Connection`], matching real `rusqlite::vtab::InterruptHandle`'s own
/// point (a [`Connection`] itself isn't `Sync`, so this is the one way to
/// reach into it from elsewhere while it's busy).
///
/// See [`Connection::get_interrupt_handle`]'s doc comment for how this
/// crate's one-shot interrupt semantics differ from real SQLite's sticky
/// one.
#[derive(Clone)]
pub struct InterruptHandle {
    interrupted: Arc<AtomicBool>,
}

impl InterruptHandle {
    /// Arms the interrupt: the connection's next `execute`/query call
    /// fails with [`Error::Interrupted`], then the flag clears itself.
    pub fn interrupt(&self) {
        self.interrupted.store(true, AtomicOrdering::SeqCst);
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
    functions: HashMap<String, Box<ScalarFn>>,
    aggregates: HashMap<String, Aggregate>,
    collations: HashMap<String, Box<CollationFn>>,
    /// Registered via [`Connection::register_module`], consumed by
    /// `CREATE VIRTUAL TABLE ... USING module_name(args)`.
    vtab_modules: HashMap<String, Box<dyn VTabModule>>,
    commit_hook: Option<Box<dyn FnMut() -> bool>>,
    rollback_hook: Option<Box<dyn FnMut()>>,
    update_hook: Option<Box<UpdateHookFn>>,
    // `trace`/`profile`/`authorizer`/`progress_handler` fire from
    // `query_row`/`query_one`/`query_map`, which take `&self` (an
    // already-shipped signature this crate won't break to add hook
    // support — see the project's own breaking-change policy). `RefCell`
    // gives them interior mutability so a `&self` query method can still
    // invoke a `FnMut` callback. `commit_hook`/`rollback_hook`/
    // `update_hook` only fire from `&mut self` paths (`execute`,
    // `Transaction`), so they don't need it.
    trace_hook: RefCell<Option<Box<TraceFn>>>,
    profile_hook: RefCell<Option<Box<ProfileFn>>>,
    /// Registered via [`Connection::trace_v2`]; fires only for event
    /// kinds `mask` includes.
    trace_v2_hook: RefCell<Option<(TraceEventCodes, Box<TraceV2Fn>)>>,
    authorizer: RefCell<Option<Box<AuthorizerFn>>>,
    progress_handler: RefCell<Option<Box<ProgressHandlerFn>>>,
    /// The file this connection persists to, or `None` for a purely
    /// in-memory connection. Set by [`Connection::open`]/
    /// [`Connection::open_with_flags`].
    path: Option<PathBuf>,
    read_only: bool,
    last_insert_rowid: i64,
    transaction_depth: u32,
    default_transaction_behavior: crate::transaction::TransactionBehavior,
    /// Backs [`Connection::prepare_cached`] (issue #106).
    statement_cache: StatementCache,
    /// Backs [`Connection::get_interrupt_handle`] (issue #107). An `Arc`
    /// so a cloned [`InterruptHandle`] can flip it from anywhere —
    /// including another OS thread — while this connection is off doing
    /// something else.
    interrupted: Arc<AtomicBool>,
}

impl Connection {
    /// Opens a new in-memory connection.
    pub fn open_in_memory() -> Result<Connection> {
        Self::open_in_memory_with_flags(OpenFlags::default())
    }

    /// Like [`Connection::open_in_memory`], with `flags` controlling how
    /// the connection is opened — only [`OpenFlags::READ_ONLY`] changes
    /// behavior (see [`OpenFlags`]'s doc comment for which bits are inert).
    pub fn open_in_memory_with_flags(flags: OpenFlags) -> Result<Connection> {
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
            functions: HashMap::new(),
            aggregates: aggregate::builtins(),
            collations: HashMap::new(),
            vtab_modules: HashMap::new(),
            commit_hook: None,
            rollback_hook: None,
            update_hook: None,
            trace_hook: RefCell::new(None),
            profile_hook: RefCell::new(None),
            trace_v2_hook: RefCell::new(None),
            authorizer: RefCell::new(None),
            progress_handler: RefCell::new(None),
            path: None,
            read_only: flags.contains(OpenFlags::READ_ONLY),
            last_insert_rowid: 0,
            transaction_depth: 0,
            default_transaction_behavior: crate::transaction::TransactionBehavior::Deferred,
            statement_cache: StatementCache::new(DEFAULT_STATEMENT_CACHE_CAPACITY),
            interrupted: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Like [`Connection::open_in_memory_with_flags`], but `vfs` (a
    /// virtual-filesystem name in real SQLite) is accepted and ignored —
    /// this engine has no pluggable I/O backend for a VFS name to select
    /// between.
    pub fn open_in_memory_with_flags_and_vfs(flags: OpenFlags, _vfs: &str) -> Result<Connection> {
        Self::open_in_memory_with_flags(flags)
    }

    /// Opens (or creates) a file-backed connection at `path`.
    ///
    /// **Design deviation, stated plainly:** the file this writes is this
    /// crate's own binary format (see `serialize.rs`), not a real SQLite
    /// database file — `ARCHITECTURE.md`'s non-goals rule out matching
    /// SQLite's on-disk format. Persistence is write-through: the full
    /// database is re-serialized and the file is rewritten after every
    /// successful [`Connection::execute`] call, not incrementally at the
    /// page level like real SQLite — simple and correct for this engine's
    /// current scale, not the most efficient approach for a large
    /// database, same tradeoff already made for `Database::snapshot`.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Connection> {
        Self::open_with_flags(path, OpenFlags::default())
    }

    /// Like [`Connection::open`], with `flags` controlling how the
    /// connection is opened. Without [`OpenFlags::CREATE`], opening a
    /// path that doesn't exist yet fails with
    /// [`Error::DatabaseDoesNotExist`] instead of silently starting an
    /// empty database.
    pub fn open_with_flags<P: AsRef<Path>>(path: P, flags: OpenFlags) -> Result<Connection> {
        let path = path.as_ref().to_path_buf();
        let db = if path.exists() {
            let bytes = std::fs::read(&path).map_err(|e| Error::Io(e.to_string()))?;
            crate::serialize::deserialize(&bytes)?
        } else if flags.contains(OpenFlags::CREATE) {
            Database::new()
        } else {
            return Err(Error::DatabaseDoesNotExist(path.display().to_string()));
        };

        let mut conn = Self::open_in_memory_with_flags(flags)?;
        conn.db = db;
        conn.path = Some(path);
        Ok(conn)
    }

    /// Like [`Connection::open_with_flags`], but `vfs` is accepted and
    /// ignored — see [`Connection::open_in_memory_with_flags_and_vfs`].
    pub fn open_with_flags_and_vfs<P: AsRef<Path>>(
        path: P,
        flags: OpenFlags,
        _vfs: &str,
    ) -> Result<Connection> {
        Self::open_with_flags(path, flags)
    }

    /// Writes this connection's current state to its backing file, if it
    /// has one (a no-op for an in-memory connection). Called
    /// automatically after every successful [`Connection::execute`] — see
    /// [`Connection::open`]'s doc comment — so callers don't normally need
    /// this directly; exposed for explicit use (e.g. right before
    /// process exit).
    pub fn flush(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = crate::serialize::serialize(&self.db);
        std::fs::write(path, bytes).map_err(|e| Error::Io(e.to_string()))
    }

    /// Returns the path to the database file, or `None` for an in-memory
    /// connection.
    pub fn path(&self) -> Option<&str> {
        self.path.as_ref().and_then(|p| p.to_str())
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

    /// Returns whether `db_name` (only `"main"` exists) is read-only —
    /// i.e. whether this connection was opened with
    /// [`OpenFlags::READ_ONLY`].
    pub fn is_readonly(&self, db_name: &str) -> Result<bool> {
        self.require_main_database(db_name)?;
        Ok(self.read_only)
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
            Ok(crate::MAIN_DB.to_string())
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

    /// Returns the rowid of the most recent successful `INSERT` on this
    /// connection (across any table), or `0` if none has happened yet.
    /// For a multi-row `INSERT`, this is the last row's rowid.
    pub fn last_insert_rowid(&self) -> i64 {
        self.last_insert_rowid
    }

    fn require_main_database(&self, db_name: &str) -> Result<()> {
        if db_name == crate::MAIN_DB {
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

    /// Changes [`Connection::prepare_cached`]'s cache capacity (issue
    /// #106), evicting least-recently-used entries immediately if
    /// shrinking below the current entry count. `0` disables caching.
    pub fn set_prepared_statement_cache_capacity(&mut self, capacity: usize) {
        self.statement_cache.set_capacity(capacity);
    }

    /// Discards every entry in [`Connection::prepare_cached`]'s cache
    /// (issue #106). Capacity is unchanged — the next
    /// `prepare_cached` call for any SQL text re-parses and re-caches it.
    pub fn flush_prepared_statement_cache(&mut self) {
        self.statement_cache.clear();
    }

    /// No-op: this engine has no page cache to flush (see
    /// `ARCHITECTURE.md` — storage is a plain in-memory `HashMap`, not a
    /// paged cache over a file).
    pub fn cache_flush(&self) -> Result<()> {
        Ok(())
    }

    /// No-op, for the same reason as [`Connection::cache_flush`]: real
    /// `rusqlite::Connection::release_memory` asks SQLite's page cache to
    /// give back unused memory, and this engine has no page cache to
    /// release from (issue #107).
    pub fn release_memory(&self) -> Result<()> {
        Ok(())
    }

    /// Returns a handle that can interrupt this connection's next
    /// `execute`/query call from anywhere — including another OS thread
    /// while this connection is blocked doing something else (issue
    /// #107). Mirrors real `rusqlite::Connection::get_interrupt_handle`/
    /// `InterruptHandle`.
    ///
    /// **Design deviation, stated plainly:** real SQLite's interrupt
    /// flag is sticky — once tripped, every subsequent call keeps
    /// failing until the connection itself resets it (there's no public
    /// "un-interrupt" API; in practice a real app re-opens or just
    /// treats the connection as done). This crate's engine has no
    /// long-running C virtual machine for the flag to guard mid-step, so
    /// there's no equivalent "still running the statement that got
    /// interrupted" window to model faithfully — the honest simpler
    /// choice here is **one-shot**: [`InterruptHandle::interrupt`] fails
    /// exactly the next `execute`/query call on this connection (see
    /// [`Error::Interrupted`]), then automatically clears, so the
    /// connection is immediately usable again afterward. Checked only at
    /// [`Connection`]'s own `execute`/`execute_with_params`/`query_row`/
    /// `query_one`/`query_map`/`query_map_with_params` entry points —
    /// calling `execute`/`query*` directly on a [`crate::Statement`]
    /// obtained via [`Connection::prepare`]/[`Connection::prepare_cached`]
    /// doesn't observe it, the same "narrower than `Connection::execute`"
    /// scope [`crate::Statement::execute`]'s own doc comment already
    /// states for `trace`/`profile`/hooks.
    pub fn get_interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle {
            interrupted: Arc::clone(&self.interrupted),
        }
    }

    /// `Ok(())` unless [`InterruptHandle::interrupt`] was called since
    /// the last time this was checked, in which case it returns
    /// [`Error::Interrupted`] and clears the flag (see
    /// [`Connection::get_interrupt_handle`]'s doc comment for why this
    /// is one-shot rather than sticky).
    fn check_interrupted(&self) -> Result<()> {
        if self.interrupted.swap(false, AtomicOrdering::SeqCst) {
            Err(Error::Interrupted)
        } else {
            Ok(())
        }
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

    /// Runs a value-returning pragma query and maps its single result row
    /// through `f`. **Starter subset**: only `foreign_keys` is
    /// recognized (real SQLite has dozens of pragmas — full coverage is
    /// its own future gap-analysis pass, not this issue's scope).
    pub fn pragma_query_value<T, F>(&self, pragma_name: &str, f: F) -> Result<T>
    where
        F: FnOnce(Row<'_>) -> Result<T>,
    {
        match pragma_name {
            "foreign_keys" => {
                let columns = vec!["foreign_keys".to_string()];
                let values = vec![Value::Integer(
                    self.db_config(DbConfig::EnableForeignKeys) as i64
                )];
                f(Row::new(&columns, &values))
            }
            other => Err(Error::UnrecognizedStatement(format!("PRAGMA {other}"))),
        }
    }

    /// Runs `PRAGMA table_info(table_name)`, mapping each column's row
    /// through `f`. Columns match SQLite's real `table_info` shape:
    /// `cid`, `name`, `type`, `notnull`, `pk` (`dflt_value`/`cid`'s exact
    /// semantics around dropped columns are omitted — this crate has no
    /// concept of either).
    pub fn pragma_table_info<F>(&self, table_name: &str, mut f: F) -> Result<()>
    where
        F: FnMut(Row<'_>) -> Result<()>,
    {
        let table = self.db.table(table_name)?;
        let columns = vec![
            "cid".to_string(),
            "name".to_string(),
            "type".to_string(),
            "notnull".to_string(),
            "pk".to_string(),
        ];
        for (cid, col) in table.columns.iter().enumerate() {
            let values = vec![
                Value::Integer(cid as i64),
                Value::Text(col.name.clone()),
                Value::Text(col.type_name.clone().unwrap_or_default()),
                Value::Integer(col.not_null as i64),
                Value::Integer(col.primary_key as i64),
            ];
            f(Row::new(&columns, &values))?;
        }
        Ok(())
    }

    /// Sets a pragma's value. **Starter subset**: only `foreign_keys` is
    /// recognized.
    pub fn pragma_update(&mut self, pragma_name: &str, value: bool) -> Result<()> {
        match pragma_name {
            "foreign_keys" => self.set_db_config(DbConfig::EnableForeignKeys, value),
            other => Err(Error::UnrecognizedStatement(format!("PRAGMA {other}"))),
        }
    }

    /// Like [`Connection::pragma_update`], but reads the value back
    /// afterward and passes it through `f` to confirm what was actually
    /// applied.
    pub fn pragma_update_and_check<T, F>(
        &mut self,
        pragma_name: &str,
        value: bool,
        f: F,
    ) -> Result<T>
    where
        F: FnOnce(Row<'_>) -> Result<T>,
    {
        self.pragma_update(pragma_name, value)?;
        self.pragma_query_value(pragma_name, f)
    }

    /// Serializes this connection's full table state into this crate's
    /// own binary format (see `serialize.rs` — **not** byte-compatible
    /// with real SQLite's file format, which this crate doesn't
    /// implement).
    pub fn serialize(&self) -> Vec<u8> {
        crate::serialize::serialize(&self.db)
    }

    /// Replaces this connection's table state with what's encoded in
    /// `bytes` (as produced by [`Connection::serialize`]). Unlike
    /// `rusqlite::Connection::deserialize`, there's no separate
    /// `deserialize_bytes`/`deserialize_read_exact` variant — this
    /// crate's format has no ownership-transfer or partial-read story
    /// (real SQLite's does, tied to its C memory model) for those to
    /// distinguish.
    pub fn deserialize(&mut self, bytes: &[u8]) -> Result<()> {
        self.db = crate::serialize::deserialize(bytes)?;
        Ok(())
    }

    /// Registers a scalar SQL function callable from `WHERE` filter
    /// expressions (e.g. `WHERE UPPER(name) = 'X'`). Only usable in
    /// `WHERE` today — result-column projection with function calls
    /// (`SELECT UPPER(name) FROM t`) isn't supported yet, since
    /// `SelectColumns::Named` is a plain column-name list, not a list of
    /// expressions.
    ///
    /// **Design deviation, stated plainly:** unlike
    /// `rusqlite::Connection::create_scalar_function`, this takes a raw
    /// `Fn(&[Value]) -> Result<Value>` rather than a generic signature
    /// derived from `ToSql`/`FromSql` argument/return types, and there's
    /// no `FunctionFlags` (deterministic/innocuous markers the query
    /// planner would use — this engine has no query planner to use them).
    pub fn create_scalar_function<F>(&mut self, name: &str, f: F) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + 'static,
    {
        self.functions.insert(name.to_string(), Box::new(f));
        Ok(())
    }

    /// Unregisters a scalar function previously registered via
    /// [`Connection::create_scalar_function`]. A no-op (not an error) if
    /// `name` wasn't registered, matching `rusqlite`'s own tolerance of
    /// removing a function that isn't there.
    pub fn remove_function(&mut self, name: &str) -> Result<()> {
        self.functions.remove(name);
        Ok(())
    }

    /// Registers `vtab` as an eponymous, read-only virtual table —
    /// queryable directly by `name` (e.g. `SELECT * FROM name`), no
    /// `CREATE VIRTUAL TABLE` needed (issue #93 doesn't exist yet). See
    /// `src/vtab.rs`'s module doc comment and
    /// `docs/adr/0003-tablesource.md`.
    ///
    /// **Deviation from real `rusqlite::Connection::create_module`,
    /// stated plainly:** that registers a reusable *module* (a factory,
    /// `Module<T>`) instantiated afresh per `CREATE VIRTUAL TABLE ...
    /// USING module_name(args)` call. This crate has no such grammar
    /// yet, so `create_module` here takes one ready-made [`VTab`]
    /// instance directly and registers it under `name`. There's no
    /// separate `Module<T>` wrapper type: [`VTabTableSource`] (issue
    /// #91) already plays that role — a second, identically-purposed
    /// type would be indirection, not real parity. Revisit if issue
    /// #93 introduces a genuine multi-instantiation call site
    /// `Module<T>` would earn its keep for.
    ///
    /// Re-registering an already-used `name` replaces the previous
    /// virtual table, matching [`Connection::create_scalar_function`]'s
    /// overwrite-on-reregister behavior. Errors with
    /// [`Error::TableAlreadyExists`] if `name` already names a *native*
    /// table — silently allowing that would register a virtual table
    /// [`Database::scan`] can never reach (native tables are always
    /// checked first).
    pub fn create_module<T: VTab + 'static>(&mut self, name: &str, vtab: T) -> Result<()> {
        if self.table_exists(name) {
            return Err(Error::TableAlreadyExists(name.to_string()));
        }
        self.db_mut()
            .register_virtual_table(name.to_string(), Box::new(VTabTableSource::new(vtab)));
        Ok(())
    }

    /// Registers `T` as a virtual-table module under `module_name`,
    /// usable via `CREATE VIRTUAL TABLE table_name USING
    /// module_name(args...)` (issue #93). The factory counterpart to
    /// [`Connection::create_module`] (issue #92), which instead
    /// registers a single ready-made instance directly for the
    /// no-`CREATE VIRTUAL TABLE` (eponymous) case.
    ///
    /// Takes no `T` value, only a type parameter — [`CreateVTab::connect`]
    /// is effectively a free function of the type, invoked fresh for
    /// each `CREATE VIRTUAL TABLE ... USING module_name(...)` statement
    /// that names it. Re-registering an already-used `module_name`
    /// replaces the previous module.
    ///
    /// See `src/vtab.rs`'s module doc comment for why there's no
    /// `Module<T>` wrapper type here, unlike real
    /// `rusqlite::vtab::create_module`.
    pub fn register_module<T: CreateVTab + 'static>(&mut self, module_name: &str) -> Result<()> {
        self.vtab_modules.insert(
            module_name.to_string(),
            Box::new(CreateVTabModule::<T>::new()),
        );
        Ok(())
    }

    /// Registers an aggregate SQL function, usable in a whole-table
    /// aggregate select list (e.g. `SELECT MEDIAN(a) FROM t`) — see
    /// [`crate::Aggregate`] and [`crate::dml_select::SelectColumns::Aggregates`].
    /// `COUNT`/`SUM`/`MIN`/`MAX` are already registered on every new
    /// connection; registering under one of those names replaces the
    /// built-in.
    pub fn create_aggregate_function(&mut self, name: &str, aggregate: Aggregate) -> Result<()> {
        self.aggregates.insert(name.to_string(), aggregate);
        Ok(())
    }

    /// Unregisters an aggregate function (built-in or custom) previously
    /// registered via [`Connection::create_aggregate_function`] or seeded
    /// by default. A no-op if `name` wasn't registered.
    pub fn remove_aggregate_function(&mut self, name: &str) -> Result<()> {
        self.aggregates.remove(name);
        Ok(())
    }

    /// Registers `aggregate` as usable in a window function's `OVER (...)`
    /// clause (e.g. `SELECT SUM(a) OVER (PARTITION BY b) FROM t`) — see
    /// [`crate::dml_select::WindowCall`] for this crate's "whole
    /// partition, no `ORDER BY`/frame clause" scope.
    ///
    /// **Design deviation, stated plainly:** real
    /// `rusqlite::Connection::create_window_function` takes a
    /// `functions::WindowAggregate` trait (`step`/`inverse`/`value`/
    /// `finalize`), designed so SQLite can slide a frame's boundaries
    /// incrementally without recomputing from scratch. This crate's
    /// window functions only ever compute over a whole partition — no
    /// frame to slide — so there's no `inverse` step to provide, and any
    /// [`Aggregate`] already has everything a whole-partition window
    /// function needs. `create_window_function` is a thin alias over the
    /// same registry [`Connection::create_aggregate_function`] uses, not
    /// a separate one — registering under either name makes `name`
    /// usable both ways.
    pub fn create_window_function(&mut self, name: &str, aggregate: Aggregate) -> Result<()> {
        self.create_aggregate_function(name, aggregate)
    }

    /// Unregisters a window function (built-in or custom) previously
    /// registered via [`Connection::create_window_function`] or
    /// [`Connection::create_aggregate_function`] — the mirror of
    /// [`Connection::remove_aggregate_function`], since both draw from
    /// the same registry. A no-op if `name` wasn't registered.
    pub fn remove_window_function(&mut self, name: &str) -> Result<()> {
        self.remove_aggregate_function(name)
    }

    /// Registers a custom text-comparison function under `name`.
    ///
    /// **Stored, not enforced:** this crate's `WHERE`/`ORDER BY` parsing
    /// and evaluation (`eval::compare_values`) has no `COLLATE name`
    /// clause to opt into a registered collation — there's no comparison
    /// site that consults `collations` yet. Kept, same reasoning as
    /// `Connection::busy_timeout`/`db_config`: stored honestly so a future
    /// PR that adds `COLLATE` parsing has a registry to read from, rather
    /// than silently discarding what's registered.
    pub fn create_collation<F>(&mut self, name: &str, collation: F) -> Result<()>
    where
        F: Fn(&str, &str) -> Ordering + 'static,
    {
        self.collations
            .insert(name.to_string(), Box::new(collation));
        Ok(())
    }

    /// Unregisters a collation previously registered via
    /// [`Connection::create_collation`]. A no-op if `name` wasn't
    /// registered.
    pub fn remove_collation(&mut self, name: &str) -> Result<()> {
        self.collations.remove(name);
        Ok(())
    }

    /// Registers a callback to run just before each top-level [`Connection::execute`]
    /// statement's changes take effect. If the callback returns `true`,
    /// the changes are rolled back and `execute` returns
    /// [`Error::CommitHookVetoed`] instead. Pass `None` to unregister.
    ///
    /// **Fires per statement, not per explicit transaction:** this
    /// crate's [`Connection::is_autocommit`] is always `true` (there's no
    /// tracked distinction between "inside an explicit `BEGIN`" and not,
    /// at the connection level — see that method's doc comment), so this
    /// fires once for every `execute` call, including ones made through
    /// an open [`crate::Transaction`]/[`crate::Savepoint`] guard. Real
    /// SQLite only fires `commit_hook` at the actual `COMMIT` boundary.
    pub fn commit_hook<F>(&mut self, hook: Option<F>)
    where
        F: FnMut() -> bool + 'static,
    {
        self.commit_hook = hook.map(|f| Box::new(f) as Box<dyn FnMut() -> bool>);
    }

    /// Registers a callback to run whenever a rollback actually happens:
    /// [`crate::Transaction::rollback`]/[`crate::Savepoint::rollback`], a
    /// guard dropped with [`crate::DropBehavior::Rollback`] (the
    /// default), or a `commit_hook` veto. Pass `None` to unregister.
    pub fn rollback_hook<F>(&mut self, hook: Option<F>)
    where
        F: FnMut() + 'static,
    {
        self.rollback_hook = hook.map(|f| Box::new(f) as Box<dyn FnMut()>);
    }

    /// Registers a callback invoked once per row inserted by
    /// [`Connection::execute`]/[`Connection::execute_batch`], as
    /// `(action, db_name, table_name, rowid)`. `db_name` is always
    /// `"main"` (no `ATTACH` support). `rowid` is the row's real,
    /// persistent SQLite-style rowid — see [`Connection::last_insert_rowid`].
    /// Pass `None` to unregister.
    ///
    /// Only [`crate::hooks::Action::Insert`] can fire today; `Update`/
    /// `Delete` have no statements to trigger them yet.
    pub fn update_hook<F>(&mut self, hook: Option<F>)
    where
        F: FnMut(Action, &str, &str, i64) + 'static,
    {
        self.update_hook = hook.map(|f| Box::new(f) as Box<UpdateHookFn>);
    }

    /// Registers a callback consulted before each `execute`/`query_*`
    /// call runs, deciding whether the underlying `CREATE TABLE`/`INSERT`/
    /// `SELECT` is allowed. Returning anything but
    /// [`crate::hooks::Authorization::Allow`] fails the call with
    /// [`Error::AuthorizationDenied`] before it touches storage. Pass
    /// `None` to unregister.
    ///
    /// **Design deviation, stated plainly:** real SQLite's authorizer
    /// fires once per *column/table reference* during statement
    /// preparation (so it can deny reading one column while allowing the
    /// rest of the row). This engine has no per-column granularity to
    /// offer — it fires once per statement with the whole target table.
    pub fn authorizer<F>(&self, hook: Option<F>)
    where
        F: FnMut(&AuthContext) -> Authorization + 'static,
    {
        *self.authorizer.borrow_mut() = hook.map(|f| Box::new(f) as Box<AuthorizerFn>);
    }

    /// Registers a callback invoked with the raw SQL text of every
    /// `execute`/`query_*` call, before it runs. Pass `None` to
    /// unregister.
    pub fn trace<F>(&self, hook: Option<F>)
    where
        F: FnMut(&str) + 'static,
    {
        *self.trace_hook.borrow_mut() = hook.map(|f| Box::new(f) as Box<TraceFn>);
    }

    /// Registers a callback invoked with `(sql, elapsed)` after every
    /// `execute`/`query_*` call finishes (successfully or not — timing is
    /// reported either way, matching real SQLite's profile callback,
    /// which fires regardless of the statement's outcome). Pass `None` to
    /// unregister.
    pub fn profile<F>(&self, hook: Option<F>)
    where
        F: FnMut(&str, Duration) + 'static,
    {
        *self.profile_hook.borrow_mut() = hook.map(|f| Box::new(f) as Box<ProfileFn>);
    }

    /// Registers a single callback for the [`crate::TraceEventCodes`] event
    /// kinds `mask` selects, replacing real SQLite's separate `trace`/
    /// `profile` callbacks with one keyed by [`crate::TraceEvent`] — see
    /// `trace.rs`'s module doc comment for how this relates to
    /// [`Connection::trace`]/[`Connection::profile`] (both still work;
    /// this is additive, not a replacement for them) and for the `Row`
    /// event kind real SQLite has that this crate can't fire. Pass
    /// `None` to unregister.
    pub fn trace_v2<F>(&self, mask: TraceEventCodes, hook: Option<F>)
    where
        F: FnMut(TraceEvent<'_>) + 'static,
    {
        *self.trace_v2_hook.borrow_mut() = hook.map(|f| (mask, Box::new(f) as Box<TraceV2Fn>));
    }

    /// Registers a callback consulted before each `execute`/`query_*`
    /// call runs; returning `true` aborts it with
    /// [`Error::OperationAborted`] before it touches storage. Pass `None`
    /// to unregister.
    ///
    /// **Design deviation, stated plainly:** real SQLite calls this every
    /// `n_ops` virtual-machine instructions *during* a statement's
    /// execution, so a slow query can be interrupted partway through.
    /// This engine has no VM instruction loop to hook into (see
    /// `ARCHITECTURE.md`) — the closest honest approximation is calling
    /// it once, before the statement starts, which can prevent a
    /// statement from running at all but can't interrupt one already in
    /// progress. `n_ops` is accepted for signature compatibility but
    /// unused.
    pub fn progress_handler<F>(&self, _n_ops: u32, hook: Option<F>)
    where
        F: FnMut() -> bool + 'static,
    {
        *self.progress_handler.borrow_mut() = hook.map(|f| Box::new(f) as Box<ProgressHandlerFn>);
    }

    fn fire_trace(&self, sql: &str) {
        if let Some(hook) = self.trace_hook.borrow_mut().as_mut() {
            hook(sql);
        }
        if let Some((mask, hook)) = self.trace_v2_hook.borrow_mut().as_mut() {
            if mask.contains(TraceEventCodes::STMT) {
                // No parameter substitution to do -- `Connection::execute`/
                // `query_*` don't support parameters at all (see
                // `crate::Statement` for the type that does), so the
                // "expanded" SQL is always identical to the original here.
                hook(TraceEvent::Stmt(StmtRef::new(sql), sql));
            }
        }
    }

    fn fire_profile(&self, sql: &str, elapsed: Duration) {
        if let Some(hook) = self.profile_hook.borrow_mut().as_mut() {
            hook(sql, elapsed);
        }
        if let Some((mask, hook)) = self.trace_v2_hook.borrow_mut().as_mut() {
            if mask.contains(TraceEventCodes::PROFILE) {
                hook(TraceEvent::Profile(StmtRef::new(sql), elapsed));
            }
        }
    }

    fn fire_trace_v2_close(&self) {
        if let Some((mask, hook)) = self.trace_v2_hook.borrow_mut().as_mut() {
            if mask.contains(TraceEventCodes::CLOSE) {
                hook(TraceEvent::Close(ConnRef::new(self)));
            }
        }
    }

    fn should_abort_via_progress_handler(&self) -> bool {
        match self.progress_handler.borrow_mut().as_mut() {
            Some(hook) => hook(),
            None => false,
        }
    }

    fn check_authorized(&self, action: Action, table_name: &str) -> Result<()> {
        let mut authorizer = self.authorizer.borrow_mut();
        let Some(hook) = authorizer.as_mut() else {
            return Ok(());
        };
        let context = AuthContext {
            action,
            table_name: Some(table_name.to_string()),
        };
        match hook(&context) {
            Authorization::Allow => Ok(()),
            Authorization::Deny | Authorization::Ignore => Err(Error::AuthorizationDenied),
        }
    }

    fn fire_commit_hook_vetoed(&mut self) -> bool {
        match &mut self.commit_hook {
            Some(hook) => hook(),
            None => false,
        }
    }

    /// Instantiates `create.module_name` (via [`Connection::register_module`])
    /// with `create.args` and registers the result as `create.table_name`.
    /// Errors with [`Error::TableAlreadyExists`] if that name already
    /// names a native table (same reasoning as
    /// [`Connection::create_module`]: silently registering a virtual
    /// table [`crate::storage::Database::scan`] can never reach would
    /// just be confusing), or [`Error::ModuleNotFound`] if
    /// `create.module_name` isn't registered.
    fn execute_create_virtual_table(&mut self, create: CreateVirtualTable) -> Result<()> {
        if self.table_exists(&create.table_name) {
            return Err(Error::TableAlreadyExists(create.table_name));
        }
        let source = {
            let module = self
                .vtab_modules
                .get(&create.module_name)
                .ok_or_else(|| Error::ModuleNotFound(create.module_name.clone()))?;
            module.connect(&create.args)?
        };
        self.db.register_virtual_table(create.table_name, source);
        Ok(())
    }

    /// Fires the rollback hook, if one is registered. `pub(crate)` so
    /// [`crate::Transaction`]/[`crate::Savepoint`] (a different module)
    /// can call it when a real rollback happens.
    pub(crate) fn fire_rollback_hook(&mut self) {
        if let Some(hook) = &mut self.rollback_hook {
            hook();
        }
    }

    /// Increments the open-transaction depth. `pub(crate)` so
    /// [`crate::Transaction`] can call it on construction — [`crate::Savepoint`]
    /// wraps a `Transaction`, so nesting is covered without extra plumbing.
    pub(crate) fn increment_transaction_depth(&mut self) {
        self.transaction_depth += 1;
    }

    /// The mirror of [`Connection::increment_transaction_depth`], called
    /// once a [`crate::Transaction`]/[`crate::Savepoint`] guard finishes
    /// (however it finishes — commit, rollback, or a drop-triggered
    /// finish; see `Transaction::mark_finished`, the single call site
    /// that funnels all three through here).
    pub(crate) fn decrement_transaction_depth(&mut self) {
        self.transaction_depth = self.transaction_depth.saturating_sub(1);
    }

    /// Returns whether a transaction (or nested savepoint) is currently
    /// open on `db_name` (`None` also means `"main"` — the only database
    /// that exists, no `ATTACH` support).
    pub fn transaction_state(
        &self,
        db_name: Option<&str>,
    ) -> Result<crate::transaction::TransactionState> {
        if let Some(name) = db_name {
            self.require_main_database(name)?;
        }
        Ok(if self.transaction_depth > 0 {
            crate::transaction::TransactionState::Write
        } else {
            crate::transaction::TransactionState::None
        })
    }

    /// Returns the behavior new transactions default to (see
    /// [`Connection::set_transaction_behavior`]).
    pub fn transaction_behavior(&self) -> crate::transaction::TransactionBehavior {
        self.default_transaction_behavior
    }

    /// Sets the behavior [`Connection::transaction`] (as opposed to
    /// [`Connection::transaction_with_behavior`], which takes an explicit
    /// override) defaults to. **Not enforced**, same caveat as
    /// `transaction_with_behavior`'s own doc comment: this crate's
    /// single-writer in-memory model doesn't distinguish
    /// `Deferred`/`Immediate`/`Exclusive` locking, so this is stored for
    /// API-shape parity, not consulted by `transaction()`.
    pub fn set_transaction_behavior(&mut self, behavior: crate::transaction::TransactionBehavior) {
        self.default_transaction_behavior = behavior;
    }

    fn fire_update_hook(&mut self, action: Action, table_name: &str, rowid: i64) {
        if let Some(hook) = &mut self.update_hook {
            hook(action, crate::MAIN_DB, table_name, rowid);
        }
    }

    /// Copies this connection's full table state into `dest`, replacing
    /// whatever `dest` had. Part B gap row "Connection + backup module:
    /// backup/restore between connections".
    ///
    /// **Design, kept simple on purpose:** real `rusqlite::Connection::backup`
    /// (via `backup::Backup`/`Progress`/`StepResult`) copies a real SQLite
    /// database incrementally, page by page, so a caller can observe and
    /// pause progress on a large file. This engine's storage is a plain
    /// in-memory `HashMap` (see `ARCHITECTURE.md`) with no page concept to
    /// step through, so `backup` is a single all-at-once copy built on
    /// [`Connection::serialize`]/[`Connection::deserialize`] — there's no
    /// `Backup`/`Progress`/`StepResult` types because there's no
    /// multi-step operation for them to describe.
    pub fn backup(&self, dest: &mut Connection) -> Result<()> {
        dest.deserialize(&self.serialize())
    }

    /// Copies `source`'s full table state into this connection, replacing
    /// whatever this connection had. The mirror of [`Connection::backup`]
    /// — `a.backup(&mut b)` and `b.restore(&a)` do the same thing.
    pub fn restore(&mut self, source: &Connection) -> Result<()> {
        self.deserialize(&source.serialize())
    }

    /// Read-only access to this connection's storage, for modules (e.g.
    /// [`crate::blob`]) that need to look up table/row/column state
    /// without going through the SQL-text `execute`/`query_*` surface.
    pub(crate) fn db(&self) -> &Database {
        &self.db
    }

    /// Mutable access to this connection's storage. See [`Connection::db`].
    pub(crate) fn db_mut(&mut self) -> &mut Database {
        &mut self.db
    }

    /// Opens a handle for incremental byte-range reads (and, unless
    /// `read_only`, writes) into a single existing `BLOB` column value,
    /// without reading or rewriting the whole row. See [`crate::blob::Blob`]
    /// for the full API and its row-index-instead-of-rowid addressing
    /// deviation from `rusqlite::Connection::blob_open`.
    pub fn blob_open(
        &mut self,
        table: &str,
        column: &str,
        row_index: usize,
        read_only: bool,
    ) -> Result<crate::blob::Blob<'_>> {
        crate::blob::Blob::open(self, table, column, row_index, read_only)
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

    /// Prepares `sql` (`CREATE TABLE`/`INSERT`/`SELECT`), tokenizing and
    /// parsing it once so the returned [`crate::Statement`] can be
    /// executed/queried repeatedly without re-parsing. See
    /// [`crate::Statement`]'s module doc comment for what's deliberately
    /// out of scope in this first cut (parameter binding, hook firing).
    pub fn prepare(&mut self, sql: &str) -> Result<crate::statement::Statement<'_>> {
        crate::statement::Statement::prepare(self, sql)
    }

    /// Like [`Connection::prepare`], but reuses a cached parse of `sql` if
    /// this connection has prepared the exact same SQL text before,
    /// skipping tokenizing/parsing on a cache hit (issue #106). Capacity
    /// is controlled by [`Connection::set_prepared_statement_cache_capacity`]
    /// (default: 16, matching real `rusqlite`); [`Error`] on a parse
    /// failure, same as [`Connection::prepare`].
    ///
    /// See [`crate::statement::StatementCache`]'s doc comment for how
    /// this differs from real `rusqlite::Connection::prepare_cached`'s
    /// `CachedStatement`/`Drop`-based design — the short version: each
    /// call returns a fresh [`crate::Statement`] built from a clone of
    /// the cached parse, with empty bindings, not a shared live handle.
    pub fn prepare_cached(&mut self, sql: &str) -> Result<crate::statement::Statement<'_>> {
        if let Some((kind, param_names)) = self.statement_cache.get(sql) {
            return Ok(crate::statement::Statement::from_parsed(
                self,
                sql,
                kind,
                param_names,
            ));
        }
        let (kind, param_names) = crate::statement::parse_statement(sql)?;
        self.statement_cache
            .insert(sql, kind.clone(), param_names.clone());
        Ok(crate::statement::Statement::from_parsed(
            self,
            sql,
            kind,
            param_names,
        ))
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
        self.fire_trace_v2_close();
        self.open = false;
        Ok(())
    }

    /// Executes a `CREATE TABLE` or `INSERT` statement, returning the
    /// number of rows affected (`0` for `CREATE TABLE`). Updates
    /// [`Connection::changes`]/[`Connection::total_changes`].
    ///
    /// Fires (in order) `progress_handler`, `trace`, the statement's
    /// `authorizer` check, then — after the statement runs —
    /// `commit_hook` (rolling back and firing `rollback_hook` on a veto),
    /// `update_hook` per row inserted, and finally `profile`. See each
    /// hook setter's doc comment for exactly what it observes here.
    pub fn execute(&mut self, sql: &str) -> Result<usize> {
        self.check_open()?;
        self.check_interrupted()?;
        if self.read_only {
            return Err(Error::ReadOnlyConnection);
        }
        if self.should_abort_via_progress_handler() {
            return Err(Error::OperationAborted);
        }
        self.fire_trace(sql);
        let start = Instant::now();

        let tokens = tokenize(sql)?;
        let snapshot = self.commit_hook.is_some().then(|| self.db.snapshot());

        let (affected, table_name, action, rowids) = match leading_keyword(&tokens) {
            Some(kw) if kw.eq_ignore_ascii_case("CREATE") && is_create_virtual_table(&tokens) => {
                let create = parse_create_virtual_table(&tokens)?;
                let table_name = create.table_name.clone();
                self.check_authorized(Action::CreateTable, &table_name)?;
                self.execute_create_virtual_table(create)?;
                (0, table_name, Action::CreateTable, Vec::new())
            }
            Some(kw) if kw.eq_ignore_ascii_case("CREATE") && is_index_statement(&tokens) => {
                let create = parse_create_index(&tokens)?;
                self.check_authorized(Action::CreateIndex, &create.table_name)?;
                execute_create_index(&mut self.db, &create)?;
                (0, create.table_name, Action::CreateIndex, Vec::new())
            }
            Some(kw) if kw.eq_ignore_ascii_case("CREATE") => {
                let create = parse_create_table(&tokens)?;
                self.check_authorized(Action::CreateTable, &create.table_name)?;
                execute_create_table(&mut self.db, &create)?;
                (0, create.table_name, Action::CreateTable, Vec::new())
            }
            Some(kw) if kw.eq_ignore_ascii_case("INSERT") => {
                let insert = parse_insert(&tokens)?;
                self.check_authorized(Action::Insert, &insert.table_name)?;
                if self.table_exists(&insert.table_name) {
                    let rowids = execute_insert_returning_rowids(&mut self.db, &insert)?;
                    (rowids.len(), insert.table_name, Action::Insert, rowids)
                } else {
                    let affected = execute_insert_into_virtual_table(&mut self.db, &insert)?;
                    // Virtual tables have no rowid concept (see
                    // `src/vtab.rs`'s module doc comment):
                    // `update_hook` doesn't fire and
                    // `last_insert_rowid` doesn't change for these
                    // rows, since there's no real rowid to report --
                    // an empty `rowids` here reflects that honestly
                    // rather than inventing a placeholder value.
                    (affected, insert.table_name, Action::Insert, Vec::new())
                }
            }
            Some(kw) if kw.eq_ignore_ascii_case("DROP") && is_index_statement(&tokens) => {
                let drop = parse_drop_index(&tokens)?;
                self.check_authorized(Action::DropIndex, &drop.index_name)?;
                execute_drop_index(&mut self.db, &drop)?;
                (0, drop.index_name, Action::DropIndex, Vec::new())
            }
            Some(kw) if kw.eq_ignore_ascii_case("DROP") => {
                let drop = parse_drop_table(&tokens)?;
                self.check_authorized(Action::DropTable, &drop.table_name)?;
                execute_drop_table(&mut self.db, &drop)?;
                (0, drop.table_name, Action::DropTable, Vec::new())
            }
            Some(kw) if kw.eq_ignore_ascii_case("ALTER") => {
                let alter = parse_alter_table(&tokens)?;
                self.check_authorized(Action::AlterTable, &alter.table_name)?;
                execute_alter_table(&mut self.db, &alter)?;
                (0, alter.table_name, Action::AlterTable, Vec::new())
            }
            _ => return Err(Error::UnrecognizedStatement(sql.to_string())),
        };

        if self.fire_commit_hook_vetoed() {
            if let Some(snapshot) = snapshot {
                self.db.restore(snapshot);
            }
            self.fire_rollback_hook();
            self.fire_profile(sql, start.elapsed());
            return Err(Error::CommitHookVetoed);
        }

        if let Some(&last) = rowids.last() {
            self.last_insert_rowid = last;
        }
        for rowid in rowids {
            self.fire_update_hook(action, &table_name, rowid);
        }
        self.last_changes = affected;
        self.total_changes += affected;
        self.flush()?;
        self.fire_profile(sql, start.elapsed());
        Ok(affected)
    }

    /// Prepares `sql`, binds `params` (see [`crate::Params`]), and runs
    /// it in one call — the ergonomic counterpart to real
    /// `rusqlite::Connection::execute(sql, params)`. Kept as a new
    /// method rather than changing [`Connection::execute`]'s
    /// already-shipped no-params signature.
    ///
    /// **Narrower than [`Connection::execute`]:** built on
    /// [`crate::Statement::execute`], which — per that method's own doc
    /// comment — doesn't fire `trace`/`profile`/`commit_hook`/
    /// `update_hook`/the authorizer the way [`Connection::execute`]
    /// does.
    pub fn execute_with_params<P: crate::params::Params>(
        &mut self,
        sql: &str,
        params: P,
    ) -> Result<usize> {
        self.check_open()?;
        self.check_interrupted()?;
        let mut stmt = self.prepare(sql)?;
        stmt.execute_with_params(params)
    }

    /// Executes a `SELECT` expected to return exactly one row, returning
    /// that row's values in the statement's result-column order. Errors
    /// with [`Error::QueryReturnedNoRows`] if the query matched no rows.
    pub fn query_row(&self, sql: &str) -> Result<Vec<Value>> {
        self.check_open()?;
        self.check_interrupted()?;
        if self.should_abort_via_progress_handler() {
            return Err(Error::OperationAborted);
        }
        self.fire_trace(sql);
        let start = Instant::now();
        let tokens = tokenize(sql)?;
        let select = parse_select(&tokens)?;
        self.check_authorized(Action::Select, &select.table_name)?;
        let (_, mut rows) = self.run_select(&select)?;
        self.fire_profile(sql, start.elapsed());
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
        self.check_interrupted()?;
        if self.should_abort_via_progress_handler() {
            return Err(Error::OperationAborted);
        }
        self.fire_trace(sql);
        let start = Instant::now();
        let tokens = tokenize(sql)?;
        let select = parse_select(&tokens)?;
        self.check_authorized(Action::Select, &select.table_name)?;
        let (columns, mut rows) = self.run_select(&select)?;
        self.fire_profile(sql, start.elapsed());
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
        self.check_interrupted()?;
        if self.should_abort_via_progress_handler() {
            return Err(Error::OperationAborted);
        }
        self.fire_trace(sql);
        let start = Instant::now();
        let tokens = tokenize(sql)?;
        let select = parse_select(&tokens)?;
        self.check_authorized(Action::Select, &select.table_name)?;
        let (columns, rows) = self.run_select(&select)?;
        self.fire_profile(sql, start.elapsed());
        rows.iter()
            .map(|values| f(Row::new(&columns, values)))
            .collect()
    }

    /// Prepares `sql`, binds `params` (see [`crate::Params`]), and maps
    /// every matching row through `f`. The ergonomic counterpart to real
    /// `rusqlite::Connection`-adjacent `query_map(sql, params, f)` call
    /// sites — see [`Connection::execute_with_params`] for the same
    /// "narrower than the plain method" caveat (no `trace`/`profile`/
    /// authorizer here either).
    pub fn query_map_with_params<P, T, F>(&mut self, sql: &str, params: P, f: F) -> Result<Vec<T>>
    where
        P: crate::params::Params,
        F: FnMut(Row<'_>) -> Result<T>,
    {
        self.check_open()?;
        self.check_interrupted()?;
        let mut stmt = self.prepare(sql)?;
        stmt.query_map_with_params(params, f)
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

    /// Runs a parsed `SELECT`, dispatching to
    /// [`execute_select_with_aggregates`] for an aggregate select list
    /// ([`SelectColumns::Aggregates`]), [`execute_select_with_window`] for
    /// a window select list ([`SelectColumns::Window`]), and
    /// [`execute_select_with_functions`] for everything else.
    /// `pub(crate)` so [`crate::Statement`] (a different module) can
    /// reuse it for already-parsed `SELECT`s.
    pub(crate) fn run_select(
        &self,
        select: &crate::dml_select::Select,
    ) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
        match &select.columns {
            SelectColumns::Aggregates(_) => {
                execute_select_with_aggregates(&self.db, select, &self.functions, &self.aggregates)
            }
            SelectColumns::Window(_) => {
                execute_select_with_window(&self.db, select, &self.functions, &self.aggregates)
            }
            _ => execute_select_with_functions(&self.db, select, &self.functions),
        }
    }
}

/// `pub(crate)` so [`crate::Statement`] (a different module) can reuse it
/// to identify a statement's kind when preparing.
pub(crate) fn leading_keyword(tokens: &[Token]) -> Option<&str> {
    match tokens.first() {
        Some(Token::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Whether `tokens` starts `CREATE VIRTUAL TABLE ...` rather than plain
/// `CREATE TABLE ...` — both share the `CREATE` leading keyword, so
/// this peeks the second token to tell them apart.
fn is_create_virtual_table(tokens: &[Token]) -> bool {
    matches!(tokens.get(1), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("VIRTUAL"))
}

/// Whether `tokens` starts `CREATE INDEX ...`/`DROP INDEX ...` rather
/// than `CREATE TABLE ...`/`DROP TABLE ...` — both `CREATE`/`DROP` share
/// their leading keyword with the `TABLE` forms, so this peeks the
/// second token to tell them apart (issue #122).
fn is_index_statement(tokens: &[Token]) -> bool {
    matches!(tokens.get(1), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("INDEX"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

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
    fn where_clause_and_combines_conditions() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b INTEGER)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1, 1), (1, 2), (2, 1)")
            .unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT b FROM t WHERE a = 1 AND b = 1", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn where_clause_or_combines_conditions() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE a = 1 OR a = 3", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![1, 3]);
    }

    #[test]
    fn where_clause_not_negates_a_condition() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE NOT a = 1", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![2]);
    }

    #[test]
    fn where_clause_parens_override_default_precedence() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b INTEGER, c INTEGER)")
            .unwrap();
        // Row A: a=1 alone would satisfy an unparenthesized `a=1 OR b=1
        // AND c=0` (AND binds tighter, so that's `a=1 OR (b=1 AND
        // c=0)`), but fails `(a=1 OR b=1) AND c=0` since c=1 there.
        // Row B: satisfies both readings.
        conn.execute("INSERT INTO t VALUES (1, 0, 1), (0, 1, 0)")
            .unwrap();

        let without_parens: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE a = 1 OR b = 1 AND c = 0", |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(without_parens, vec![1, 0]);

        let with_parens: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE (a = 1 OR b = 1) AND c = 0", |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(with_parens, vec![0]);
    }

    #[test]
    fn where_clause_like_filters_by_pattern() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (name TEXT)").unwrap();
        conn.execute("INSERT INTO t VALUES ('alice'), ('bob'), ('alina')")
            .unwrap();

        let rows: Vec<String> = conn
            .query_map("SELECT name FROM t WHERE name LIKE 'al%'", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec!["alice", "alina"]);
    }

    #[test]
    fn where_clause_between_filters_inclusive_range() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (5), (10), (15)")
            .unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE a BETWEEN 5 AND 10", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![5, 10]);
    }

    #[test]
    fn where_clause_in_filters_by_list_membership() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3), (4)")
            .unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE a IN (2, 4)", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![2, 4]);
    }

    #[test]
    fn where_clause_not_in_excludes_list_members() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3), (4)")
            .unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT a FROM t WHERE a NOT IN (2, 4)", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![1, 3]);
    }

    #[test]
    fn where_clause_case_when_computes_a_value_used_in_the_filter() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let rows: Vec<i64> = conn
            .query_map(
                "SELECT a FROM t WHERE CASE WHEN a = 2 THEN 1 ELSE 0 END = 1",
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, vec![2]);
    }

    #[test]
    fn select_distinct_dedups_query_map_results() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (1), (3), (2)")
            .unwrap();

        let rows: Vec<i64> = conn
            .query_map("SELECT DISTINCT a FROM t", |row| row.get(0))
            .unwrap();
        assert_eq!(rows, vec![1, 2, 3]);
    }

    #[test]
    fn create_table_if_not_exists_is_a_no_op_on_an_existing_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        conn.execute("CREATE TABLE IF NOT EXISTS t (a INTEGER)")
            .unwrap();

        let row = conn.query_row("SELECT a FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1)]);
    }

    #[test]
    fn create_table_without_if_not_exists_still_errors_on_collision() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(matches!(
            conn.execute("CREATE TABLE t (a INTEGER)"),
            Err(Error::TableAlreadyExists(_))
        ));
    }

    #[test]
    fn drop_table_removes_the_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        let affected = conn.execute("DROP TABLE t").unwrap();
        assert_eq!(affected, 0);

        assert!(matches!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn drop_missing_table_without_if_exists_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            conn.execute("DROP TABLE t"),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn drop_missing_table_with_if_exists_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(conn.execute("DROP TABLE IF EXISTS t").is_ok());
    }

    #[test]
    fn insert_violating_primary_key_is_a_constraint_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        assert!(matches!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn insert_violating_not_null_is_a_constraint_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER NOT NULL)").unwrap();
        assert!(matches!(
            conn.execute("INSERT INTO t VALUES (NULL)"),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn insert_violating_check_is_a_constraint_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (age INTEGER CHECK (age <= 10))")
            .unwrap();
        assert!(matches!(
            conn.execute("INSERT INTO t VALUES (20)"),
            Err(Error::ConstraintViolation(_))
        ));

        conn.execute("INSERT INTO t VALUES (5)").unwrap();
        let row = conn.query_row("SELECT age FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(5)]);
    }

    #[test]
    fn execute_with_params_binds_and_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();

        let affected = conn
            .execute_with_params("INSERT INTO t VALUES (?, ?)", (1i64, "x"))
            .unwrap();
        assert_eq!(affected, 1);

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Text("x".into())]);
    }

    #[test]
    fn query_map_with_params_binds_and_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let values: Vec<i64> = conn
            .query_map_with_params("SELECT * FROM t WHERE a = ?", (2i64,), |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![2]);
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
        // `DROP TABLE` is a recognized statement now (issue #120) --
        // `UPDATE` isn't implemented yet, so it's this test's example
        // instead.
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            conn.execute("UPDATE t SET a = 1"),
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

    #[test]
    fn pragma_foreign_keys_defaults_to_off_and_updates() {
        let mut conn = Connection::open_in_memory().unwrap();
        let initial: i64 = conn
            .pragma_query_value("foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(initial, 0);

        conn.pragma_update("foreign_keys", true).unwrap();
        let updated: i64 = conn
            .pragma_query_value("foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(updated, 1);
    }

    #[test]
    fn pragma_table_info_reports_columns() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();

        let mut names = Vec::new();
        conn.pragma_table_info("t", |row| {
            let name: String = row.get(1)?;
            names.push(name);
            Ok(())
        })
        .unwrap();
        assert_eq!(names, vec!["id".to_string(), "name".to_string()]);
    }

    #[test]
    fn pragma_on_unrecognized_name_is_an_error() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn
            .pragma_query_value::<i64, _>("journal_mode", |row| row.get(0))
            .is_err());
    }

    #[test]
    fn serialize_and_deserialize_round_trip() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        let bytes = conn.serialize();

        let mut restored = Connection::open_in_memory().unwrap();
        restored.deserialize(&bytes).unwrap();
        let values: Vec<i64> = restored
            .query_map("SELECT * FROM t", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![1, 2]);
    }

    #[test]
    fn deserialize_rejects_garbage() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(conn.deserialize(b"not a real database").is_err());
    }

    #[test]
    fn scalar_function_usable_in_where_clause() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        conn.create_scalar_function("DOUBLE", |args| match args {
            [Value::Integer(n)] => Ok(Value::Integer(n * 2)),
            _ => Err(Error::FunctionNotFound("DOUBLE".into())),
        })
        .unwrap();

        let values: Vec<i64> = conn
            .query_map("SELECT * FROM t WHERE DOUBLE(a) = 4", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![2]);
    }

    #[test]
    fn removed_function_is_no_longer_found() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (2)").unwrap();
        conn.create_scalar_function("DOUBLE", |args| match args {
            [Value::Integer(n)] => Ok(Value::Integer(n * 2)),
            _ => Err(Error::FunctionNotFound("DOUBLE".into())),
        })
        .unwrap();
        conn.remove_function("DOUBLE").unwrap();

        assert!(conn
            .query_map("SELECT * FROM t WHERE DOUBLE(a) = 4", |row| row
                .get::<i64>(0))
            .is_err());
    }

    #[test]
    fn removing_unregistered_function_is_not_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(conn.remove_function("NEVER_REGISTERED").is_ok());
    }

    /// A minimal read-only vtab for `create_module` tests: exposes one
    /// column (`value`) holding whatever integers it was built with.
    struct FixedRowsVTab {
        rows: Vec<i64>,
    }

    struct FixedRowsCursor {
        rows: Vec<i64>,
        pos: usize,
    }

    impl crate::vtab::VTab for FixedRowsVTab {
        type Cursor = FixedRowsCursor;

        fn column_names(&self) -> Vec<String> {
            vec!["value".to_string()]
        }

        fn open(&self) -> Result<FixedRowsCursor> {
            Ok(FixedRowsCursor {
                rows: self.rows.clone(),
                pos: 0,
            })
        }
    }

    impl crate::vtab::VTabCursor for FixedRowsCursor {
        fn filter(&mut self, _filter: Option<&crate::dml_select::Expr>) -> Result<()> {
            Ok(())
        }

        fn next(&mut self) -> Result<()> {
            self.pos += 1;
            Ok(())
        }

        fn eof(&self) -> bool {
            self.pos >= self.rows.len()
        }

        fn column(&self, ctx: &mut crate::vtab::Context, _i: usize) -> Result<()> {
            ctx.set_result(&self.rows[self.pos])
        }
    }

    #[test]
    fn create_module_registers_a_queryable_virtual_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "v",
            FixedRowsVTab {
                rows: vec![1, 2, 3],
            },
        )
        .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM v", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn create_module_reregistering_same_name_replaces_the_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module("v", FixedRowsVTab { rows: vec![1] })
            .unwrap();
        conn.create_module("v", FixedRowsVTab { rows: vec![9, 9] })
            .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM v", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![9, 9]);
    }

    #[test]
    fn create_module_errors_if_name_already_names_a_native_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        assert_eq!(
            conn.create_module("t", FixedRowsVTab { rows: vec![1] }),
            Err(Error::TableAlreadyExists("t".to_string()))
        );
    }

    #[test]
    fn querying_an_unregistered_module_name_is_table_not_found() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            conn.query_row("SELECT * FROM never_registered"),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn drop_table_on_a_virtual_table_is_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module("v", FixedRowsVTab { rows: vec![1] })
            .unwrap();

        assert!(matches!(
            conn.execute("DROP TABLE v"),
            Err(Error::CannotDropVirtualTable(_))
        ));
    }

    #[test]
    fn alter_table_add_column_backfills_existing_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        let affected = conn.execute("ALTER TABLE t ADD COLUMN b TEXT").unwrap();
        assert_eq!(affected, 0);

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Null]);
    }

    #[test]
    fn alter_table_rename_to_renames_the_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        conn.execute("ALTER TABLE t RENAME TO t2").unwrap();

        assert!(matches!(
            conn.query_row("SELECT * FROM t"),
            Err(Error::TableNotFound(_))
        ));
        let row = conn.query_row("SELECT * FROM t2").unwrap();
        assert_eq!(row, vec![Value::Integer(1)]);
    }

    #[test]
    fn alter_table_rename_column_renames_it() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        conn.execute("ALTER TABLE t RENAME COLUMN a TO b").unwrap();

        let rows: Vec<i64> = conn.query_map("SELECT b FROM t", |row| row.get(0)).unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn create_index_and_drop_index_round_trip_without_affecting_queries() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        let affected = conn.execute("CREATE INDEX idx_a ON t (a)").unwrap();
        assert_eq!(affected, 0);

        let rows: Vec<i64> = conn.query_map("SELECT a FROM t", |row| row.get(0)).unwrap();
        assert_eq!(rows, vec![1, 2]);

        conn.execute("DROP INDEX idx_a").unwrap();
        assert!(matches!(
            conn.execute("DROP INDEX idx_a"),
            Err(Error::IndexNotFound(_))
        ));
    }

    #[test]
    fn duplicate_create_index_without_if_not_exists_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("CREATE INDEX idx_a ON t (a)").unwrap();
        assert!(matches!(
            conn.execute("CREATE INDEX idx_a ON t (a)"),
            Err(Error::IndexAlreadyExists(_))
        ));
        assert!(conn
            .execute("CREATE INDEX IF NOT EXISTS idx_a ON t (a)")
            .is_ok());
    }

    /// A `CreateVTab` test double: `USING arange(start, end)` builds a
    /// one-column (`value`) integer-range table from its own
    /// `CREATE VIRTUAL TABLE` arguments.
    struct ArgRangeVTab {
        start: i64,
        end: i64,
    }

    struct ArgRangeCursor {
        current: i64,
        end: i64,
    }

    impl crate::vtab::VTab for ArgRangeVTab {
        type Cursor = ArgRangeCursor;

        fn column_names(&self) -> Vec<String> {
            vec!["value".to_string()]
        }

        fn open(&self) -> Result<ArgRangeCursor> {
            Ok(ArgRangeCursor {
                current: self.start,
                end: self.end,
            })
        }
    }

    impl crate::vtab::CreateVTab for ArgRangeVTab {
        fn connect(args: &[String]) -> Result<Self> {
            let [start, end] = args else {
                return Err(Error::UnrecognizedStatement(
                    "arange needs exactly 2 args".to_string(),
                ));
            };
            let parse = |s: &str| {
                s.trim()
                    .parse::<i64>()
                    .map_err(|_| Error::UnrecognizedStatement(format!("not an integer: {s:?}")))
            };
            Ok(ArgRangeVTab {
                start: parse(start)?,
                end: parse(end)?,
            })
        }
    }

    impl crate::vtab::VTabCursor for ArgRangeCursor {
        fn filter(&mut self, _filter: Option<&crate::dml_select::Expr>) -> Result<()> {
            Ok(())
        }

        fn next(&mut self) -> Result<()> {
            self.current += 1;
            Ok(())
        }

        fn eof(&self) -> bool {
            self.current >= self.end
        }

        fn column(&self, ctx: &mut crate::vtab::Context, _i: usize) -> Result<()> {
            ctx.set_result(&self.current)
        }
    }

    #[test]
    fn create_virtual_table_registers_and_queries_a_module_instance() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<ArgRangeVTab>("arange").unwrap();
        conn.execute("CREATE VIRTUAL TABLE t USING arange(1, 4)")
            .unwrap();

        let values: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn create_virtual_table_with_unknown_module_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(matches!(
            conn.execute("CREATE VIRTUAL TABLE t USING nomodule(1, 2)"),
            Err(Error::ModuleNotFound(_))
        ));
    }

    #[test]
    fn create_virtual_table_with_malformed_args_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<ArgRangeVTab>("arange").unwrap();
        assert!(conn
            .execute("CREATE VIRTUAL TABLE t USING arange(not_a_number, 4)")
            .is_err());
    }

    #[test]
    fn create_virtual_table_errors_if_name_already_names_a_native_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.register_module::<ArgRangeVTab>("arange").unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        assert_eq!(
            conn.execute("CREATE VIRTUAL TABLE t USING arange(1, 4)"),
            Err(Error::TableAlreadyExists("t".to_string()))
        );
    }

    #[test]
    fn insert_into_read_only_virtual_table_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module("v", FixedRowsVTab { rows: vec![1, 2] })
            .unwrap();
        assert_eq!(
            conn.execute("INSERT INTO v VALUES (3)"),
            Err(Error::ReadOnlyVirtualTable)
        );
    }

    /// A writable vtab (issue #95): one integer column, backed by a
    /// `RefCell` so `insert` can mutate it through `&self`.
    struct AppendableVTab {
        rows: std::cell::RefCell<Vec<i64>>,
    }

    impl crate::vtab::VTab for AppendableVTab {
        type Cursor = FixedRowsCursor;

        fn column_names(&self) -> Vec<String> {
            vec!["value".to_string()]
        }

        fn open(&self) -> Result<FixedRowsCursor> {
            Ok(FixedRowsCursor {
                rows: self.rows.borrow().clone(),
                pos: 0,
            })
        }

        fn insert(&self, row: Vec<Value>) -> Result<()> {
            match row.as_slice() {
                [Value::Integer(n)] => {
                    self.rows.borrow_mut().push(*n);
                    Ok(())
                }
                other => Err(Error::ColumnCountMismatch {
                    expected: 1,
                    actual: other.len(),
                }),
            }
        }
    }

    impl crate::vtab::UpdateVTab for AppendableVTab {}

    #[test]
    fn insert_into_writable_virtual_table_via_execute() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "v",
            AppendableVTab {
                rows: std::cell::RefCell::new(vec![1]),
            },
        )
        .unwrap();

        let affected = conn.execute("INSERT INTO v VALUES (2)").unwrap();
        assert_eq!(affected, 1);

        let values: Vec<i64> = conn.query_map("SELECT * FROM v", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2]);
    }

    #[test]
    fn insert_into_writable_virtual_table_does_not_affect_last_insert_rowid() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_module(
            "v",
            AppendableVTab {
                rows: std::cell::RefCell::new(vec![]),
            },
        )
        .unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (42)").unwrap();
        assert_eq!(conn.last_insert_rowid(), 1);

        conn.execute("INSERT INTO v VALUES (1)").unwrap();
        // Virtual tables have no rowid concept -- last_insert_rowid is
        // untouched by the virtual-table insert above.
        assert_eq!(conn.last_insert_rowid(), 1);
    }

    #[test]
    fn backup_copies_table_state_into_destination() {
        let mut src = Connection::open_in_memory().unwrap();
        src.execute("CREATE TABLE t (a INTEGER)").unwrap();
        src.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        let mut dest = Connection::open_in_memory().unwrap();
        src.backup(&mut dest).unwrap();

        let values: Vec<i64> = dest.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 2]);
    }

    #[test]
    fn backup_replaces_destination_state() {
        let src = Connection::open_in_memory().unwrap();

        let mut dest = Connection::open_in_memory().unwrap();
        dest.execute("CREATE TABLE old (a INTEGER)").unwrap();

        src.backup(&mut dest).unwrap();

        assert!(!dest.table_exists("old"));
    }

    #[test]
    fn restore_is_the_mirror_of_backup() {
        let mut a = Connection::open_in_memory().unwrap();
        a.execute("CREATE TABLE t (a INTEGER)").unwrap();
        a.execute("INSERT INTO t VALUES (7)").unwrap();

        let mut b = Connection::open_in_memory().unwrap();
        b.restore(&a).unwrap();

        let values: Vec<i64> = b.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![7]);
    }

    #[test]
    fn count_star_and_sum_are_builtin_aggregates() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let row = conn.query_row("SELECT COUNT(*), SUM(a) FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(3), Value::Integer(6)]);
    }

    #[test]
    fn aggregate_respects_where_filter() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let row = conn
            .query_row("SELECT COUNT(*) FROM t WHERE a = 2")
            .unwrap();
        assert_eq!(row, vec![Value::Integer(1)]);
    }

    #[test]
    fn min_max_over_empty_table_are_null() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let row = conn.query_row("SELECT MIN(a), MAX(a) FROM t").unwrap();
        assert_eq!(row, vec![Value::Null, Value::Null]);
    }

    #[test]
    fn custom_aggregate_function_is_usable() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (2), (3)").unwrap();

        conn.create_aggregate_function(
            "PRODUCT",
            Aggregate::simple(Value::Integer(1), |acc, args| match (acc, args.first()) {
                (Value::Integer(n), Some(Value::Integer(v))) => Ok(Value::Integer(n * v)),
                (acc, _) => Ok(acc.clone()),
            }),
        )
        .unwrap();

        let row = conn.query_row("SELECT PRODUCT(a) FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(6)]);
    }

    #[test]
    fn removed_aggregate_function_is_no_longer_found() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        conn.remove_aggregate_function("COUNT").unwrap();

        assert!(conn.query_row("SELECT COUNT(*) FROM t").is_err());
    }

    #[test]
    fn window_function_broadcasts_whole_partition_value_to_every_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (grp TEXT, a INTEGER)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES ('x', 1), ('x', 2), ('y', 10), ('y', 20), ('y', 30)")
            .unwrap();

        let sums: Vec<i64> = conn
            .query_map("SELECT SUM(a) OVER (PARTITION BY grp) FROM t", |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(sums, vec![3, 3, 60, 60, 60]);
    }

    #[test]
    fn window_function_with_no_partition_by_treats_whole_table_as_one_partition() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let counts: Vec<i64> = conn
            .query_map("SELECT COUNT(*) OVER () FROM t", |row| row.get(0))
            .unwrap();
        assert_eq!(counts, vec![3, 3, 3]);
    }

    #[test]
    fn window_function_respects_where_filter() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (grp TEXT, a INTEGER)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES ('x', 1), ('x', 2), ('x', 3)")
            .unwrap();

        let sums: Vec<i64> = conn
            .query_map(
                "SELECT SUM(a) OVER (PARTITION BY grp) FROM t WHERE a = 2",
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sums, vec![2]);
    }

    #[test]
    fn create_window_function_registers_a_custom_window_aggregate() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (grp TEXT, a INTEGER)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES ('x', 2), ('x', 3)")
            .unwrap();

        conn.create_window_function(
            "PRODUCT",
            Aggregate::simple(Value::Integer(1), |acc, args| match (acc, args.first()) {
                (Value::Integer(n), Some(Value::Integer(v))) => Ok(Value::Integer(n * v)),
                (acc, _) => Ok(acc.clone()),
            }),
        )
        .unwrap();

        let products: Vec<i64> = conn
            .query_map("SELECT PRODUCT(a) OVER (PARTITION BY grp) FROM t", |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(products, vec![6, 6]);
    }

    #[test]
    fn create_window_function_is_also_usable_as_a_plain_aggregate() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (2), (3)").unwrap();

        conn.create_window_function(
            "PRODUCT",
            Aggregate::simple(Value::Integer(1), |acc, args| match (acc, args.first()) {
                (Value::Integer(n), Some(Value::Integer(v))) => Ok(Value::Integer(n * v)),
                (acc, _) => Ok(acc.clone()),
            }),
        )
        .unwrap();

        let row = conn.query_row("SELECT PRODUCT(a) FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(6)]);
    }

    #[test]
    fn removed_window_function_is_no_longer_found() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        conn.remove_window_function("COUNT").unwrap();

        assert!(conn.query_row("SELECT COUNT(*) OVER () FROM t").is_err());
    }

    #[test]
    fn collation_is_registered_and_removable_though_not_yet_consulted() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.create_collation("NOCASE_LIKE", |a, b| {
            a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
        })
        .unwrap();
        // Not enforced anywhere yet -- this only confirms registration and
        // removal don't error, same as busy_timeout/busy_handler.
        conn.remove_collation("NOCASE_LIKE").unwrap();
        assert!(conn.remove_collation("NEVER_REGISTERED").is_ok());
    }

    #[test]
    fn update_hook_fires_once_per_inserted_row() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let events = Rc::new(RefCell::new(Vec::new()));
        let events_clone = Rc::clone(&events);
        conn.update_hook(Some(move |action, db: &str, table: &str, rowid| {
            events_clone
                .borrow_mut()
                .push((action, db.to_string(), table.to_string(), rowid));
        }));

        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        assert_eq!(
            *events.borrow(),
            vec![
                (Action::Insert, "main".to_string(), "t".to_string(), 1),
                (Action::Insert, "main".to_string(), "t".to_string(), 2),
            ]
        );
    }

    #[test]
    fn update_hook_does_not_fire_for_create_table() {
        let mut conn = Connection::open_in_memory().unwrap();

        let fired = Rc::new(RefCell::new(false));
        let fired_clone = Rc::clone(&fired);
        conn.update_hook(Some(move |_, _: &str, _: &str, _| {
            *fired_clone.borrow_mut() = true;
        }));

        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(!*fired.borrow());
    }

    #[test]
    fn commit_hook_veto_rolls_back_and_fires_rollback_hook() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        conn.commit_hook(Some(|| true));
        let rolled_back = Rc::new(RefCell::new(false));
        let rolled_back_clone = Rc::clone(&rolled_back);
        conn.rollback_hook(Some(move || {
            *rolled_back_clone.borrow_mut() = true;
        }));

        let result = conn.execute("INSERT INTO t VALUES (1)");
        assert_eq!(result, Err(Error::CommitHookVetoed));
        assert!(*rolled_back.borrow());

        let rows: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn commit_hook_allowing_keeps_changes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.commit_hook(Some(|| false));

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        let rows: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn authorizer_deny_blocks_the_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.authorizer(Some(|ctx: &AuthContext| {
            if ctx.action == Action::Insert {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));

        assert_eq!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::AuthorizationDenied)
        );
        let rows: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn authorizer_allow_permits_the_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.authorizer(Some(|_: &AuthContext| Authorization::Allow));

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        let rows: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(rows, vec![1]);
    }

    #[test]
    fn trace_hook_receives_sql_text() {
        let conn = Connection::open_in_memory().unwrap();
        let traced = Rc::new(RefCell::new(Vec::new()));
        let traced_clone = Rc::clone(&traced);
        conn.trace(Some(move |sql: &str| {
            traced_clone.borrow_mut().push(sql.to_string());
        }));

        assert!(matches!(
            conn.query_row("SELECT * FROM missing"),
            Err(Error::TableNotFound(_))
        ));
        assert_eq!(*traced.borrow(), vec!["SELECT * FROM missing".to_string()]);
    }

    #[test]
    fn profile_hook_receives_sql_and_duration() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let profiled = Rc::new(RefCell::new(Vec::new()));
        let profiled_clone = Rc::clone(&profiled);
        conn.profile(Some(move |sql: &str, elapsed: std::time::Duration| {
            profiled_clone.borrow_mut().push((sql.to_string(), elapsed));
        }));

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        let calls = profiled.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "INSERT INTO t VALUES (1)");
    }

    #[test]
    fn trace_v2_fires_stmt_and_profile_events_when_both_are_in_the_mask() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let events = Rc::new(RefCell::new(Vec::new()));
        let events_clone = Rc::clone(&events);
        conn.trace_v2(
            crate::TraceEventCodes::STMT | crate::TraceEventCodes::PROFILE,
            Some(move |event: crate::TraceEvent<'_>| {
                let label = match event {
                    crate::TraceEvent::Stmt(stmt, _) => format!("stmt:{}", stmt.sql()),
                    crate::TraceEvent::Profile(stmt, _) => format!("profile:{}", stmt.sql()),
                    crate::TraceEvent::Close(_) => "close".to_string(),
                };
                events_clone.borrow_mut().push(label);
            }),
        );

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(
            *events.borrow(),
            vec![
                "stmt:INSERT INTO t VALUES (1)".to_string(),
                "profile:INSERT INTO t VALUES (1)".to_string(),
            ]
        );
    }

    #[test]
    fn trace_v2_only_fires_event_kinds_in_the_mask() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let events = Rc::new(RefCell::new(Vec::new()));
        let events_clone = Rc::clone(&events);
        conn.trace_v2(
            crate::TraceEventCodes::PROFILE,
            Some(move |event: crate::TraceEvent<'_>| {
                events_clone
                    .borrow_mut()
                    .push(matches!(event, crate::TraceEvent::Profile(_, _)));
            }),
        );

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(*events.borrow(), vec![true]);
    }

    #[test]
    fn trace_v2_fires_close_event_on_connection_close() {
        let conn = Connection::open_in_memory().unwrap();

        let closed = Rc::new(RefCell::new(false));
        let closed_clone = Rc::clone(&closed);
        conn.trace_v2(
            crate::TraceEventCodes::CLOSE,
            Some(move |event: crate::TraceEvent<'_>| {
                if let crate::TraceEvent::Close(conn_ref) = event {
                    assert!(conn_ref.is_open());
                    *closed_clone.borrow_mut() = true;
                }
            }),
        );

        conn.close().unwrap();
        assert!(*closed.borrow());
    }

    #[test]
    fn trace_v2_none_unregisters_the_hook() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let events = Rc::new(RefCell::new(Vec::new()));
        let events_clone = Rc::clone(&events);
        conn.trace_v2(
            crate::TraceEventCodes::STMT,
            Some(move |_event: crate::TraceEvent<'_>| {
                events_clone.borrow_mut().push(());
            }),
        );
        conn.trace_v2(
            crate::TraceEventCodes::STMT,
            None::<fn(crate::TraceEvent<'_>)>,
        );

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn progress_handler_returning_true_aborts_before_running() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.progress_handler(1, Some(|| true));

        assert_eq!(
            conn.execute("CREATE TABLE t (a INTEGER)"),
            Err(Error::OperationAborted)
        );
        assert!(!conn.table_exists("t"));
    }

    #[test]
    fn progress_handler_returning_false_lets_statement_run() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.progress_handler(1, Some(|| false));

        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(conn.table_exists("t"));
    }

    #[test]
    fn clearing_a_hook_with_none_stops_it_firing() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let count = Rc::new(RefCell::new(0));
        let count_clone = Rc::clone(&count);
        conn.update_hook(Some(move |_, _: &str, _: &str, _| {
            *count_clone.borrow_mut() += 1;
        }));
        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(*count.borrow(), 1);

        conn.update_hook::<fn(Action, &str, &str, i64)>(None);
        conn.execute("INSERT INTO t VALUES (2)").unwrap();
        assert_eq!(*count.borrow(), 1);
    }

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rusty_rusqlite_test_{name}.db"))
    }

    #[test]
    fn open_creates_a_new_file_and_persists_across_reopen() {
        let path = temp_db_path("creates_and_persists");
        let _ = std::fs::remove_file(&path);

        {
            let mut conn = Connection::open(&path).unwrap();
            conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
            conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();
        }
        assert!(path.exists());

        let mut reopened = Connection::open(&path).unwrap();
        let values: Vec<i64> = reopened
            .query_map("SELECT * FROM t", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![1, 2]);

        reopened.execute("INSERT INTO t VALUES (3)").unwrap();
        let values: Vec<i64> = Connection::open(&path)
            .unwrap()
            .query_map("SELECT * FROM t", |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![1, 2, 3]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_reports_its_path() {
        let path = temp_db_path("reports_path");
        let _ = std::fs::remove_file(&path);

        let conn = Connection::open(&path).unwrap();
        assert_eq!(conn.path(), Some(path.to_str().unwrap()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn in_memory_connection_has_no_path() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.path(), None);
    }

    #[test]
    fn open_without_create_flag_on_missing_path_is_an_error() {
        let path = temp_db_path("no_create_flag");
        let _ = std::fs::remove_file(&path);

        let flags = OpenFlags::READ_WRITE;
        assert!(matches!(
            Connection::open_with_flags(&path, flags),
            Err(Error::DatabaseDoesNotExist(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn read_only_connection_rejects_execute() {
        let path = temp_db_path("read_only_rejects");
        let _ = std::fs::remove_file(&path);
        {
            let mut conn = Connection::open(&path).unwrap();
            conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        }

        let mut conn =
            Connection::open_with_flags(&path, OpenFlags::READ_ONLY | OpenFlags::READ_WRITE)
                .unwrap();
        assert!(conn.is_readonly("main").unwrap());
        assert_eq!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::ReadOnlyConnection)
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_only_in_memory_connection_rejects_execute() {
        let mut conn = Connection::open_in_memory_with_flags(OpenFlags::READ_ONLY).unwrap();
        assert_eq!(
            conn.execute("CREATE TABLE t (a INTEGER)"),
            Err(Error::ReadOnlyConnection)
        );
    }

    #[test]
    fn flush_on_in_memory_connection_is_a_noop() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.flush().is_ok());
    }

    #[test]
    fn vfs_variants_ignore_the_vfs_name_and_still_work() {
        let conn =
            Connection::open_in_memory_with_flags_and_vfs(OpenFlags::default(), "unix").unwrap();
        assert!(!conn.is_readonly("main").unwrap());

        let path = temp_db_path("vfs_ignored");
        let _ = std::fs::remove_file(&path);
        let mut conn =
            Connection::open_with_flags_and_vfs(&path, OpenFlags::default(), "unix").unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(conn.table_exists("t"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn last_insert_rowid_starts_at_zero() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.last_insert_rowid(), 0);
    }

    #[test]
    fn last_insert_rowid_tracks_the_most_recent_insert() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(conn.last_insert_rowid(), 1);

        conn.execute("INSERT INTO t VALUES (2)").unwrap();
        assert_eq!(conn.last_insert_rowid(), 2);
    }

    #[test]
    fn last_insert_rowid_is_the_last_row_of_a_multi_row_insert() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();
        assert_eq!(conn.last_insert_rowid(), 3);
    }

    #[test]
    fn last_insert_rowid_tracks_whichever_table_was_last_inserted_into() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t1 (a INTEGER)").unwrap();
        conn.execute("CREATE TABLE t2 (a INTEGER)").unwrap();

        conn.execute("INSERT INTO t1 VALUES (1), (2)").unwrap();
        assert_eq!(conn.last_insert_rowid(), 2);

        conn.execute("INSERT INTO t2 VALUES (1)").unwrap();
        assert_eq!(conn.last_insert_rowid(), 1);
    }

    #[test]
    fn commit_hook_veto_does_not_update_last_insert_rowid() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.commit_hook(Some(|| true));

        assert_eq!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::CommitHookVetoed)
        );
        assert_eq!(conn.last_insert_rowid(), 0);
    }

    #[test]
    fn a_fresh_connection_is_not_interrupted() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(conn.execute("CREATE TABLE t (a INTEGER)").is_ok());
    }

    #[test]
    fn interrupt_fails_the_next_execute_call() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let handle = conn.get_interrupt_handle();
        handle.interrupt();

        assert_eq!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::Interrupted)
        );
    }

    #[test]
    fn interrupt_is_one_shot_and_auto_clears() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let handle = conn.get_interrupt_handle();
        handle.interrupt();

        assert_eq!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::Interrupted)
        );
        // The flag already auto-cleared -- this call succeeds.
        assert!(conn.execute("INSERT INTO t VALUES (2)").is_ok());
    }

    #[test]
    fn interrupt_fails_query_row_query_one_and_query_map_too() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();
        let handle = conn.get_interrupt_handle();

        handle.interrupt();
        assert_eq!(conn.query_row("SELECT * FROM t"), Err(Error::Interrupted));

        handle.interrupt();
        assert_eq!(
            conn.query_one("SELECT * FROM t", |row| row.get::<i64>(0)),
            Err(Error::Interrupted)
        );

        handle.interrupt();
        assert_eq!(
            conn.query_map("SELECT * FROM t", |row| row.get::<i64>(0)),
            Err(Error::Interrupted)
        );
    }

    #[test]
    fn interrupt_handle_works_from_another_thread() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let handle = conn.get_interrupt_handle();

        std::thread::spawn(move || handle.interrupt())
            .join()
            .unwrap();

        assert_eq!(
            conn.execute("INSERT INTO t VALUES (1)"),
            Err(Error::Interrupted)
        );
    }

    #[test]
    fn release_memory_is_a_harmless_no_op() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(conn.release_memory(), Ok(()));
    }
}
