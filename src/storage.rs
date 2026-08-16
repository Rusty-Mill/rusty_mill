//! In-memory page/record storage backend (foundation-tier `A5`).
//! Deliberately in-memory-only — see `ARCHITECTURE.md`'s non-goals for why
//! the on-disk file format isn't in scope yet.

use crate::ddl::{AlterTableAction, ColumnDef, CreateTable};
use crate::dml_select::Expr;
use crate::error::{Error, Result};
use crate::eval::{evaluate, evaluate_bool3};
use crate::value::Value;
use std::collections::HashMap;
use std::fmt;

/// A source of rows a `SELECT` can scan, standing in for a concrete
/// [`Table`] — see `docs/adr/0003-tablesource.md`. Implemented by
/// `Table` itself (native tables) and by any virtual table registered
/// via [`crate::Connection::create_module`].
pub trait TableSource {
    fn column_names(&self) -> &[String];
    /// `filter`, if given, is the query's `WHERE` clause — an
    /// opportunistic hint, not a contract: an implementation MAY use it
    /// to skip computing rows that can't match, but the caller
    /// re-evaluates `filter` against every returned row regardless, so
    /// ignoring it (returning everything) is always correct, just
    /// unoptimized. See the ADR for why this isn't real SQLite's
    /// `IndexInfo`/`best_index` negotiation.
    fn scan(&self, filter: Option<&Expr>) -> Result<Vec<Vec<Value>>>;

    /// Inserts a new row (issue #95). Default: read-only, errors with
    /// [`Error::ReadOnlyVirtualTable`]. [`Table`] (native tables) never
    /// goes through this path — `INSERT` into a native table uses
    /// [`Database::insert_row`] directly, unrelated to `TableSource`.
    fn insert(&self, _row: Vec<Value>) -> Result<()> {
        Err(Error::ReadOnlyVirtualTable)
    }

    /// Notifies this source that the enclosing
    /// [`crate::Transaction`]/[`crate::Savepoint`] has begun/committed/
    /// rolled back (issue #95). Defaults: no-op — most virtual tables
    /// don't need to know about transaction boundaries, and
    /// `Transaction`/`Savepoint`'s own snapshot/restore already covers
    /// native tables, unrelated to this.
    fn begin(&self) -> Result<()> {
        Ok(())
    }
    fn commit(&self) -> Result<()> {
        Ok(())
    }
    fn rollback(&self) -> Result<()> {
        Ok(())
    }
}

impl TableSource for Table {
    fn column_names(&self) -> &[String] {
        &self.column_names
    }

    fn scan(&self, _filter: Option<&Expr>) -> Result<Vec<Vec<Value>>> {
        Ok(self.rows.clone())
    }
}

/// A single table's schema and row data.
#[derive(Debug, Clone)]
pub struct Table {
    pub column_names: Vec<String>,
    /// Full column definitions (type name, `PRIMARY KEY`/`NOT NULL`), in
    /// the same order as `column_names`. Kept alongside `column_names`
    /// rather than replacing it, since most existing call sites only need
    /// the names.
    pub columns: Vec<ColumnDef>,
    pub rows: Vec<Vec<Value>>,
    /// Each row's SQLite-style rowid, index-aligned with `rows` (i.e.
    /// `row_ids[i]` is `rows[i]`'s rowid). Monotonically increasing,
    /// assigned in [`Database::insert_row`], never reused — this crate
    /// has no `DELETE` yet, so the "reuse the highest deleted rowid"
    /// question that would otherwise arise doesn't come up.
    pub row_ids: Vec<i64>,
}

/// A `CREATE INDEX`-recorded index's metadata (issue #122) — see
/// [`crate::ddl::CreateIndex`]'s doc comment for why this is
/// non-accelerating (recorded schema metadata only).
#[derive(Debug, Clone, PartialEq)]
pub struct IndexMetadata {
    pub table_name: String,
    pub columns: Vec<String>,
}

/// The full set of tables in a database. This is the storage layer that
/// [`crate::Connection`] will be wired to in `A8`.
#[derive(Default)]
pub struct Database {
    tables: HashMap<String, Table>,
    /// Registered via [`Database::register_virtual_table`]
    /// ([`crate::Connection::create_module`]'s the public API for it).
    /// Checked by [`Database::scan`] after native tables — see
    /// `docs/adr/0003-tablesource.md`.
    virtual_tables: HashMap<String, Box<dyn TableSource>>,
    /// Recorded via [`Database::create_index`] (issue #122) — metadata
    /// only, never consulted by [`Database::scan`]. See
    /// [`IndexMetadata`]'s doc comment.
    indexes: HashMap<String, IndexMetadata>,
}

impl fmt::Debug for Database {
    // Hand-written: `Box<dyn TableSource>` isn't `Debug`, so `Database`
    // can't derive it while holding `virtual_tables`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("tables", &self.tables)
            .field("virtual_table_count", &self.virtual_tables.len())
            .field("indexes", &self.indexes)
            .finish()
    }
}

impl Database {
    /// Creates a new, empty in-memory database.
    pub fn new() -> Database {
        Database {
            tables: HashMap::new(),
            virtual_tables: HashMap::new(),
            indexes: HashMap::new(),
        }
    }

    /// Creates a table from a parsed `CREATE TABLE` statement. A name
    /// collision is [`Error::TableAlreadyExists`], unless `create`
    /// carries `IF NOT EXISTS` (issue #119) — then it's a silent no-op,
    /// keeping the existing table as-is (no schema comparison against
    /// `create`'s columns).
    pub fn create_table(&mut self, create: &CreateTable) -> Result<()> {
        if self.tables.contains_key(&create.table_name) {
            if create.if_not_exists {
                return Ok(());
            }
            return Err(Error::TableAlreadyExists(create.table_name.clone()));
        }
        let column_names = create.columns.iter().map(|c| c.name.clone()).collect();
        self.tables.insert(
            create.table_name.clone(),
            Table {
                column_names,
                columns: create.columns.clone(),
                rows: Vec::new(),
                row_ids: Vec::new(),
            },
        );
        Ok(())
    }

    /// Drops `table_name`, removing its schema and all rows. A missing
    /// table is [`Error::TableNotFound`], unless `if_exists` is set
    /// (issue #120) — then it's a silent no-op, mirroring [`Database::
    /// create_table`]'s own `IF NOT EXISTS` no-op convention (#119).
    ///
    /// A registered virtual table by this name is
    /// [`Error::CannotDropVirtualTable`] regardless of `if_exists` — see
    /// that variant's doc comment for why dropping one isn't supported.
    pub fn drop_table(&mut self, table_name: &str, if_exists: bool) -> Result<()> {
        if self.tables.remove(table_name).is_some() {
            return Ok(());
        }
        if self.virtual_tables.contains_key(table_name) {
            return Err(Error::CannotDropVirtualTable(table_name.to_string()));
        }
        if if_exists {
            return Ok(());
        }
        Err(Error::TableNotFound(table_name.to_string()))
    }

    /// Applies an `ALTER TABLE table_name ...` action (issue #121) — see
    /// [`crate::ddl::AlterTable`]'s own doc comment for the supported
    /// forms and their scope.
    ///
    /// `ADD COLUMN` additionally rejects a `PRIMARY KEY`/`UNIQUE` column
    /// and a `NOT NULL` column with no `DEFAULT` — see the inline
    /// comments below for why, matching real SQLite's own `ADD COLUMN`
    /// restrictions rather than silently leaving the table in a state
    /// that violates its own declared constraints.
    pub fn alter_table(&mut self, table_name: &str, action: &AlterTableAction) -> Result<()> {
        match action {
            AlterTableAction::AddColumn(column) => {
                let table = self
                    .tables
                    .get_mut(table_name)
                    .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?;
                if table.column_names.contains(&column.name) {
                    return Err(Error::DuplicateColumn(column.name.clone()));
                }
                // Real SQLite rejects adding a PRIMARY KEY/UNIQUE column
                // outright (there's no sensible way to backfill a
                // uniqueness constraint across existing rows without a
                // user-supplied value per row) -- mirrored here rather
                // than silently accepting a constraint this crate would
                // then never actually enforce for the backfilled rows.
                if column.primary_key || column.unique {
                    return Err(Error::ConstraintViolation(format!(
                        "Cannot add a PRIMARY KEY or UNIQUE column via ALTER TABLE: {}",
                        column.name
                    )));
                }
                // Real SQLite also rejects `NOT NULL` here unless a
                // `DEFAULT` is given -- otherwise every existing row
                // would immediately violate the new constraint via a
                // backfilled `NULL`.
                if column.not_null && column.default.is_none() {
                    return Err(Error::ConstraintViolation(format!(
                        "Cannot add a NOT NULL column with no default value: {}",
                        column.name
                    )));
                }
                // Evaluated with no columns/row in scope: a `DEFAULT`
                // used to backfill existing rows must be a self-contained
                // constant (real SQLite's own restriction) -- an
                // `Expr::Column` reference here errors `UnknownColumn`
                // rather than silently reading `NULL` or the new row's
                // own not-yet-existing value.
                let default_value = match &column.default {
                    Some(expr) => evaluate(expr, &[], &[])?,
                    None => Value::Null,
                };
                for row in &mut table.rows {
                    row.push(default_value.clone());
                }
                table.column_names.push(column.name.clone());
                table.columns.push(column.clone());
                Ok(())
            }
            AlterTableAction::RenameTo(new_name) => {
                if self.tables.contains_key(new_name) {
                    return Err(Error::TableAlreadyExists(new_name.clone()));
                }
                let table = self
                    .tables
                    .remove(table_name)
                    .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?;
                self.tables.insert(new_name.clone(), table);
                Ok(())
            }
            AlterTableAction::RenameColumn { old_name, new_name } => {
                let table = self
                    .tables
                    .get_mut(table_name)
                    .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?;
                let idx = table
                    .column_names
                    .iter()
                    .position(|c| c == old_name)
                    .ok_or_else(|| Error::UnknownColumn(old_name.clone()))?;
                if table.column_names.iter().any(|c| c == new_name) {
                    return Err(Error::DuplicateColumn(new_name.clone()));
                }
                table.column_names[idx] = new_name.clone();
                table.columns[idx].name = new_name.clone();
                Ok(())
            }
        }
    }

    /// Records a `CREATE INDEX` statement's metadata (issue #122) —
    /// non-accelerating, see [`crate::ddl::CreateIndex`]'s doc comment.
    /// A name collision is [`Error::IndexAlreadyExists`], unless
    /// `if_not_exists` is set — then it's a silent no-op, mirroring
    /// [`Database::create_table`]'s own `IF NOT EXISTS` convention.
    /// `table_name` must exist and every column in `columns` must be one
    /// of its columns — both checked eagerly rather than only at index
    /// creation time going unnoticed, since there's no later "does this
    /// index still make sense" check without real index storage.
    pub fn create_index(
        &mut self,
        index_name: &str,
        table_name: &str,
        columns: &[String],
        if_not_exists: bool,
    ) -> Result<()> {
        if self.indexes.contains_key(index_name) {
            if if_not_exists {
                return Ok(());
            }
            return Err(Error::IndexAlreadyExists(index_name.to_string()));
        }
        let table = self
            .tables
            .get(table_name)
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?;
        for column in columns {
            if !table.column_names.contains(column) {
                return Err(Error::UnknownColumn(column.clone()));
            }
        }
        self.indexes.insert(
            index_name.to_string(),
            IndexMetadata {
                table_name: table_name.to_string(),
                columns: columns.to_vec(),
            },
        );
        Ok(())
    }

    /// Removes a `CREATE INDEX`-recorded index's metadata (issue #122).
    /// A missing index is [`Error::IndexNotFound`], unless `if_exists`
    /// is set — then it's a silent no-op, mirroring [`Database::
    /// drop_table`]'s own `IF EXISTS` convention.
    pub fn drop_index(&mut self, index_name: &str, if_exists: bool) -> Result<()> {
        if self.indexes.remove(index_name).is_some() {
            return Ok(());
        }
        if if_exists {
            return Ok(());
        }
        Err(Error::IndexNotFound(index_name.to_string()))
    }

    /// Returns a recorded index's metadata, if one by this name exists —
    /// used by tests and by anything wanting to introspect what's been
    /// declared (e.g. a future `PRAGMA index_list`).
    pub fn index(&self, index_name: &str) -> Option<&IndexMetadata> {
        self.indexes.get(index_name)
    }

    /// Inserts one row into `table_name`, given values in the table's
    /// column-definition order. Column-list-driven inserts (reordering or
    /// omitting columns) are the caller's responsibility to expand into
    /// this shape until a catalog-aware insert path exists.
    pub fn insert_row(&mut self, table_name: &str, row: Vec<Value>) -> Result<()> {
        self.insert_row_returning_rowid(table_name, row).map(|_| ())
    }

    /// Like [`Database::insert_row`], but returns the row's newly
    /// assigned rowid — added alongside the original (rather than
    /// changing its return type) so this doesn't break the already-shipped
    /// `Result<()>` signature.
    pub fn insert_row_returning_rowid(&mut self, table_name: &str, row: Vec<Value>) -> Result<i64> {
        let table = self
            .tables
            .get_mut(table_name)
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?;
        if row.len() != table.column_names.len() {
            return Err(Error::ColumnCountMismatch {
                expected: table.column_names.len(),
                actual: row.len(),
            });
        }
        check_constraints(table, table_name, &row)?;
        let rowid = table.row_ids.iter().max().copied().unwrap_or(0) + 1;
        table.rows.push(row);
        table.row_ids.push(rowid);
        Ok(rowid)
    }

    /// Returns a table's schema and rows for scanning.
    pub fn table(&self, table_name: &str) -> Result<&Table> {
        self.tables
            .get(table_name)
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))
    }

    /// Returns `table_name`'s column names and rows for a `SELECT` scan,
    /// checking native tables first, then registered virtual tables —
    /// the dispatch point [`TableSource`] exists for. Used by
    /// `engine.rs`'s `execute_select*` functions instead of
    /// [`Database::table`] directly, so a virtual table can stand in for
    /// a native one. See `docs/adr/0003-tablesource.md`.
    pub fn scan(
        &self,
        table_name: &str,
        filter: Option<&Expr>,
    ) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
        if let Some(table) = self.tables.get(table_name) {
            return Ok((table.column_names.clone(), table.scan(filter)?));
        }
        if let Some(source) = self.virtual_tables.get(table_name) {
            return Ok((source.column_names().to_vec(), source.scan(filter)?));
        }
        Err(Error::TableNotFound(table_name.to_string()))
    }

    /// Registers a virtual table under `name`, checked by
    /// [`Database::scan`] after native tables. `pub(crate)` only —
    /// [`crate::Connection::create_module`] is the public API that
    /// calls this.
    pub(crate) fn register_virtual_table(&mut self, name: String, source: Box<dyn TableSource>) {
        self.virtual_tables.insert(name, source);
    }

    /// Returns a registered virtual table's column names (issue #95 —
    /// used by `engine::execute_insert_into_virtual_table` for
    /// column-list expansion, the same role [`Database::table`]'s
    /// `column_names` field plays for native `INSERT`).
    pub fn virtual_table_column_names(&self, table_name: &str) -> Result<Vec<String>> {
        self.virtual_tables
            .get(table_name)
            .map(|source| source.column_names().to_vec())
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))
    }

    /// Inserts a row into a registered virtual table (issue #95),
    /// via [`TableSource::insert`] — errors with
    /// [`Error::ReadOnlyVirtualTable`] unless the table's [`crate::VTab`]
    /// overrode [`crate::VTab::insert`].
    pub fn insert_into_virtual_table(&mut self, table_name: &str, row: Vec<Value>) -> Result<()> {
        self.virtual_tables
            .get(table_name)
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?
            .insert(row)
    }

    /// Notifies every registered virtual table that a
    /// [`crate::Transaction`]/[`crate::Savepoint`] has begun/committed/
    /// rolled back (issue #95), via [`TableSource::begin`]/`commit`/
    /// `rollback`. Stops at the first error rather than a two-phase
    /// protocol — not a real concern with this crate's single-writer,
    /// in-memory model, but stated plainly: an earlier-notified virtual
    /// table in the same call isn't rolled back if a later one errors.
    pub(crate) fn notify_virtual_tables_begin(&self) -> Result<()> {
        self.virtual_tables.values().try_for_each(|s| s.begin())
    }

    pub(crate) fn notify_virtual_tables_commit(&self) -> Result<()> {
        self.virtual_tables.values().try_for_each(|s| s.commit())
    }

    pub(crate) fn notify_virtual_tables_rollback(&self) -> Result<()> {
        self.virtual_tables.values().try_for_each(|s| s.rollback())
    }

    /// Returns a mutable reference to a single cell, addressed by its row's
    /// plain position within `Table::rows` (**not** a SQLite rowid — this
    /// crate's storage has no rowid concept yet) and column position
    /// within `Table::column_names`. Used by [`crate::blob::Blob`] for
    /// in-place incremental writes.
    pub fn cell_mut(
        &mut self,
        table_name: &str,
        row_index: usize,
        column_index: usize,
    ) -> Result<&mut Value> {
        let table = self
            .tables
            .get_mut(table_name)
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))?;
        let row_count = table.rows.len();
        let row = table
            .rows
            .get_mut(row_index)
            .ok_or(Error::IndexOutOfBounds {
                index: row_index,
                len: row_count,
            })?;
        let col_count = row.len();
        row.get_mut(column_index).ok_or(Error::IndexOutOfBounds {
            index: column_index,
            len: col_count,
        })
    }

    /// Snapshots the current table state, for transaction/savepoint
    /// rollback. A full clone — simple and correct for this in-memory
    /// engine's current scale, not the copy-on-write/undo-log approach a
    /// real storage engine would use. Worth revisiting if profiling ever
    /// shows it matters.
    pub fn snapshot(&self) -> HashMap<String, Table> {
        self.tables.clone()
    }

    /// Restores table state from a snapshot taken by [`Database::snapshot`].
    pub fn restore(&mut self, snapshot: HashMap<String, Table>) {
        self.tables = snapshot;
    }

    /// Returns all tables by name, for callers (e.g. `serialize`) that
    /// need to walk the full set rather than look up one table.
    pub fn tables(&self) -> &HashMap<String, Table> {
        &self.tables
    }

    /// Inserts a table directly, bypassing [`Database::create_table`]'s
    /// validation (no duplicate-name check, no `CREATE TABLE`-derived
    /// column list). Used by [`crate::serialize::deserialize`] to
    /// reconstruct a `Database` from previously-serialized state, which
    /// is already known-valid — re-validating it through the normal
    /// creation path would be redundant.
    pub fn insert_table_raw(&mut self, name: String, table: Table) {
        self.tables.insert(name, table);
    }
}

/// Checks `row` (about to be inserted into `table`) against its declared
/// `PRIMARY KEY`/`UNIQUE`/`NOT NULL`/`CHECK` constraints (issue #118),
/// called from [`Database::insert_row_returning_rowid`] before the row is
/// added to `table.rows`.
///
/// **Scope, stated plainly:** this crate only parses column-level
/// constraints (`ddl.rs` has no table-level `PRIMARY KEY (a, b)` /
/// `UNIQUE (a, b)` clause), so a composite (multi-column) key isn't
/// representable here — each `primary_key`/`unique`-flagged column is
/// checked independently. `PRIMARY KEY` and `UNIQUE` both use SQL's own
/// NULL-is-distinct-from-NULL rule: a `NULL` value never conflicts with
/// anything, including another `NULL` already in the table.
///
/// `CHECK` uses [`evaluate_bool3`] so a `NULL` result passes (matching
/// real SQLite: only exactly-`FALSE` is a violation) — but note that
/// this crate's `BinaryOp` comparisons (`=`/`<`/`>=`/...) don't
/// themselves propagate `NULL` yet (a separate, pre-existing gap in
/// `eval.rs`, out of scope here), so e.g. `CHECK (age >= 0)` with `age`
/// `NULL` currently evaluates to plain `FALSE`, not `NULL` — a real
/// SQLite would let that row through.
fn check_constraints(table: &Table, table_name: &str, row: &[Value]) -> Result<()> {
    for (i, col) in table.columns.iter().enumerate() {
        let value = &row[i];

        if col.not_null && *value == Value::Null {
            return Err(Error::ConstraintViolation(format!(
                "NOT NULL constraint failed: {table_name}.{}",
                col.name
            )));
        }

        if (col.primary_key || col.unique)
            && *value != Value::Null
            && table.rows.iter().any(|existing| existing[i] == *value)
        {
            let kind = if col.primary_key {
                "PRIMARY KEY"
            } else {
                "UNIQUE"
            };
            return Err(Error::ConstraintViolation(format!(
                "{kind} constraint failed: {table_name}.{}",
                col.name
            )));
        }

        if let Some(check) = &col.check {
            let satisfied = evaluate_bool3(check, &table.column_names, row)?.unwrap_or(true);
            if !satisfied {
                return Err(Error::ConstraintViolation(format!(
                    "CHECK constraint failed: {table_name}.{}",
                    col.name
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddl::parse_drop_table;
    use crate::parse_create_table;
    use crate::tokenize;

    fn create(sql: &str) -> CreateTable {
        parse_create_table(&tokenize(sql).unwrap()).unwrap()
    }

    #[test]
    fn creates_table_and_scans_empty() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        let table = db.table("t").unwrap();
        assert_eq!(table.column_names, vec!["a", "b"]);
        assert!(table.rows.is_empty());
    }

    #[test]
    fn duplicate_create_table_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.create_table(&create("CREATE TABLE t (a INTEGER)")),
            Err(Error::TableAlreadyExists(_))
        ));
    }

    #[test]
    fn create_table_if_not_exists_is_a_no_op_on_collision() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        // A second CREATE TABLE IF NOT EXISTS (even with a different
        // schema) is a silent no-op -- the original table, rows
        // included, is left untouched.
        db.create_table(&create("CREATE TABLE IF NOT EXISTS t (a INTEGER, b TEXT)"))
            .unwrap();

        let table = db.table("t").unwrap();
        assert_eq!(table.column_names, vec!["a"]);
        assert_eq!(table.rows, vec![vec![Value::Integer(1)]]);
    }

    #[test]
    fn create_table_if_not_exists_still_creates_a_new_table() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE IF NOT EXISTS t (a INTEGER)"))
            .unwrap();
        assert!(db.table("t").is_ok());
    }

    #[test]
    fn drop_table_removes_schema_and_rows() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        let drop = parse_drop_table(&tokenize("DROP TABLE t").unwrap()).unwrap();
        db.drop_table(&drop.table_name, drop.if_exists).unwrap();

        assert!(matches!(db.table("t"), Err(Error::TableNotFound(_))));
    }

    #[test]
    fn drop_missing_table_without_if_exists_is_an_error() {
        let mut db = Database::new();
        assert!(matches!(
            db.drop_table("missing", false),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn drop_missing_table_with_if_exists_is_a_no_op() {
        let mut db = Database::new();
        assert!(db.drop_table("missing", true).is_ok());
    }

    #[test]
    fn dropped_table_can_be_recreated() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();
        db.drop_table("t", false).unwrap();

        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        assert!(db.table("t").unwrap().rows.is_empty());
    }

    #[test]
    fn alter_table_add_column_backfills_null_by_default() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        db.alter_table(
            "t",
            &AlterTableAction::AddColumn(ColumnDef {
                name: "b".into(),
                type_name: Some("TEXT".into()),
                ..Default::default()
            }),
        )
        .unwrap();

        let table = db.table("t").unwrap();
        assert_eq!(table.column_names, vec!["a", "b"]);
        assert_eq!(table.rows, vec![vec![Value::Integer(1), Value::Null]]);
    }

    #[test]
    fn alter_table_add_column_backfills_declared_default() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        db.alter_table(
            "t",
            &AlterTableAction::AddColumn(ColumnDef {
                name: "b".into(),
                type_name: Some("INTEGER".into()),
                default: Some(Expr::Literal(Value::Integer(0))),
                ..Default::default()
            }),
        )
        .unwrap();

        let table = db.table("t").unwrap();
        assert_eq!(table.rows, vec![vec![Value::Integer(1), Value::Integer(0)]]);

        // New rows use the table's ordinary column-count check, not the
        // default -- ADD COLUMN's default only backfills existing rows.
        db.insert_row("t", vec![Value::Integer(2), Value::Integer(9)])
            .unwrap();
        assert_eq!(db.table("t").unwrap().rows.len(), 2);
    }

    #[test]
    fn alter_table_add_column_duplicate_name_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.alter_table(
                "t",
                &AlterTableAction::AddColumn(ColumnDef {
                    name: "a".into(),
                    ..Default::default()
                }),
            ),
            Err(Error::DuplicateColumn(_))
        ));
    }

    #[test]
    fn alter_table_add_column_not_null_without_default_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.alter_table(
                "t",
                &AlterTableAction::AddColumn(ColumnDef {
                    name: "b".into(),
                    not_null: true,
                    ..Default::default()
                }),
            ),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn alter_table_add_column_not_null_with_default_is_allowed() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(db
            .alter_table(
                "t",
                &AlterTableAction::AddColumn(ColumnDef {
                    name: "b".into(),
                    not_null: true,
                    default: Some(Expr::Literal(Value::Integer(0))),
                    ..Default::default()
                }),
            )
            .is_ok());
    }

    #[test]
    fn alter_table_add_column_primary_key_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.alter_table(
                "t",
                &AlterTableAction::AddColumn(ColumnDef {
                    name: "b".into(),
                    primary_key: true,
                    ..Default::default()
                }),
            ),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn alter_table_add_column_missing_table_is_an_error() {
        let mut db = Database::new();
        assert!(matches!(
            db.alter_table(
                "missing",
                &AlterTableAction::AddColumn(ColumnDef {
                    name: "b".into(),
                    ..Default::default()
                }),
            ),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn alter_table_rename_to_renames_the_table() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        db.alter_table("t", &AlterTableAction::RenameTo("t2".into()))
            .unwrap();

        assert!(matches!(db.table("t"), Err(Error::TableNotFound(_))));
        assert_eq!(db.table("t2").unwrap().rows, vec![vec![Value::Integer(1)]]);
    }

    #[test]
    fn alter_table_rename_to_an_existing_name_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.create_table(&create("CREATE TABLE t2 (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.alter_table("t", &AlterTableAction::RenameTo("t2".into())),
            Err(Error::TableAlreadyExists(_))
        ));
    }

    #[test]
    fn alter_table_rename_to_missing_table_is_an_error() {
        let mut db = Database::new();
        assert!(matches!(
            db.alter_table("missing", &AlterTableAction::RenameTo("t2".into())),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn alter_table_rename_column_renames_it() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1), Value::Text("x".into())])
            .unwrap();

        db.alter_table(
            "t",
            &AlterTableAction::RenameColumn {
                old_name: "a".into(),
                new_name: "c".into(),
            },
        )
        .unwrap();

        let table = db.table("t").unwrap();
        assert_eq!(table.column_names, vec!["c", "b"]);
        assert_eq!(table.columns[0].name, "c");
        assert_eq!(
            table.rows,
            vec![vec![Value::Integer(1), Value::Text("x".into())]]
        );
    }

    #[test]
    fn alter_table_rename_column_missing_column_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.alter_table(
                "t",
                &AlterTableAction::RenameColumn {
                    old_name: "missing".into(),
                    new_name: "c".into(),
                },
            ),
            Err(Error::UnknownColumn(_))
        ));
    }

    #[test]
    fn alter_table_rename_column_to_an_existing_name_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        assert!(matches!(
            db.alter_table(
                "t",
                &AlterTableAction::RenameColumn {
                    old_name: "a".into(),
                    new_name: "b".into(),
                },
            ),
            Err(Error::DuplicateColumn(_))
        ));
    }

    #[test]
    fn create_index_records_metadata() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        db.create_index("idx_a", "t", &["a".to_string()], false)
            .unwrap();

        assert_eq!(
            db.index("idx_a"),
            Some(&IndexMetadata {
                table_name: "t".to_string(),
                columns: vec!["a".to_string()],
            })
        );
    }

    #[test]
    fn create_index_on_a_missing_table_is_an_error() {
        let mut db = Database::new();
        assert!(matches!(
            db.create_index("idx_a", "missing", &["a".to_string()], false),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn create_index_on_a_missing_column_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        assert!(matches!(
            db.create_index("idx_z", "t", &["z".to_string()], false),
            Err(Error::UnknownColumn(_))
        ));
    }

    #[test]
    fn duplicate_create_index_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.create_index("idx_a", "t", &["a".to_string()], false)
            .unwrap();
        assert!(matches!(
            db.create_index("idx_a", "t", &["a".to_string()], false),
            Err(Error::IndexAlreadyExists(_))
        ));
    }

    #[test]
    fn create_index_if_not_exists_is_a_no_op_on_collision() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        db.create_index("idx_a", "t", &["a".to_string()], false)
            .unwrap();
        db.create_index("idx_a", "t", &["b".to_string()], true)
            .unwrap();

        // Left untouched -- still indexing "a", not "b".
        assert_eq!(db.index("idx_a").unwrap().columns, vec!["a".to_string()]);
    }

    #[test]
    fn drop_index_removes_metadata() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.create_index("idx_a", "t", &["a".to_string()], false)
            .unwrap();
        db.drop_index("idx_a", false).unwrap();
        assert_eq!(db.index("idx_a"), None);
    }

    #[test]
    fn drop_missing_index_without_if_exists_is_an_error() {
        let mut db = Database::new();
        assert!(matches!(
            db.drop_index("missing", false),
            Err(Error::IndexNotFound(_))
        ));
    }

    #[test]
    fn drop_missing_index_with_if_exists_is_a_no_op() {
        let mut db = Database::new();
        assert!(db.drop_index("missing", true).is_ok());
    }

    #[test]
    fn an_indexed_table_still_scans_and_queries_normally() {
        // Indexes are recorded metadata only (issue #122) -- creating
        // one has no effect on scanning/querying the underlying table.
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();
        db.insert_row("t", vec![Value::Integer(2)]).unwrap();
        db.create_index("idx_a", "t", &["a".to_string()], false)
            .unwrap();

        let (_, rows) = db.scan("t", None).unwrap();
        assert_eq!(rows, vec![vec![Value::Integer(1)], vec![Value::Integer(2)]]);
    }

    #[test]
    fn inserts_and_scans_rows() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1), Value::Text("x".into())])
            .unwrap();
        db.insert_row("t", vec![Value::Integer(2), Value::Text("y".into())])
            .unwrap();
        let table = db.table("t").unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0], Value::Integer(1));
    }

    #[test]
    fn insert_into_missing_table_is_an_error() {
        let mut db = Database::new();
        assert!(matches!(
            db.insert_row("missing", vec![Value::Integer(1)]),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn not_null_violation_is_a_constraint_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER NOT NULL)"))
            .unwrap();
        assert!(matches!(
            db.insert_row("t", vec![Value::Null]),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn not_null_is_satisfied_by_a_non_null_value() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER NOT NULL)"))
            .unwrap();
        assert!(db.insert_row("t", vec![Value::Integer(1)]).is_ok());
    }

    #[test]
    fn primary_key_violation_is_a_constraint_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (id INTEGER PRIMARY KEY)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();
        assert!(matches!(
            db.insert_row("t", vec![Value::Integer(1)]),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn unique_violation_is_a_constraint_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (email TEXT UNIQUE)"))
            .unwrap();
        db.insert_row("t", vec![Value::Text("a@example.com".into())])
            .unwrap();
        assert!(matches!(
            db.insert_row("t", vec![Value::Text("a@example.com".into())]),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn unique_allows_multiple_nulls() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (email TEXT UNIQUE)"))
            .unwrap();
        db.insert_row("t", vec![Value::Null]).unwrap();
        assert!(db.insert_row("t", vec![Value::Null]).is_ok());
    }

    #[test]
    fn check_violation_is_a_constraint_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (age INTEGER CHECK (age >= 0))"))
            .unwrap();
        assert!(matches!(
            db.insert_row("t", vec![Value::Integer(-1)]),
            Err(Error::ConstraintViolation(_))
        ));
    }

    #[test]
    fn check_is_satisfied_by_a_passing_value() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (age INTEGER CHECK (age >= 0))"))
            .unwrap();
        assert!(db.insert_row("t", vec![Value::Integer(0)]).is_ok());
    }

    #[test]
    fn check_treats_null_as_passing() {
        // Real SQLite's own rule: a `CHECK` only fails on an
        // exactly-`FALSE` result -- `NULL` (unknown) passes, same as
        // `TRUE`. Built directly (bypassing SQL parsing): a bare-column
        // condition like `CHECK (flag)` isn't expressible through this
        // crate's WHERE-style grammar (`parse_comparison` always
        // requires an operator after its left operand — see its own doc
        // comment), and a comparison like `age >= 0` wouldn't exercise
        // this path either, since this crate's `BinaryOp` comparisons
        // don't yet propagate `NULL` themselves (a separate,
        // pre-existing gap in `eval.rs`, out of scope here) and would
        // evaluate a `NULL` operand to plain `FALSE` instead.
        let mut db = Database::new();
        db.create_table(&CreateTable {
            table_name: "t".into(),
            columns: vec![ColumnDef {
                name: "flag".into(),
                check: Some(Expr::Column("flag".into())),
                ..Default::default()
            }],
            if_not_exists: false,
        })
        .unwrap();
        assert!(db.insert_row("t", vec![Value::Null]).is_ok());
    }

    #[test]
    fn multiple_constraints_on_one_insert_are_all_enforced() {
        let mut db = Database::new();
        db.create_table(&create(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, age INTEGER NOT NULL CHECK (age >= 0))",
        ))
        .unwrap();
        db.insert_row("t", vec![Value::Integer(1), Value::Integer(30)])
            .unwrap();

        // Duplicate primary key.
        assert!(matches!(
            db.insert_row("t", vec![Value::Integer(1), Value::Integer(20)]),
            Err(Error::ConstraintViolation(_))
        ));
        // NOT NULL violation.
        assert!(matches!(
            db.insert_row("t", vec![Value::Integer(2), Value::Null]),
            Err(Error::ConstraintViolation(_))
        ));
        // CHECK violation.
        assert!(matches!(
            db.insert_row("t", vec![Value::Integer(3), Value::Integer(-1)]),
            Err(Error::ConstraintViolation(_))
        ));
        // Satisfies everything.
        assert!(db
            .insert_row("t", vec![Value::Integer(4), Value::Integer(40)])
            .is_ok());
    }

    #[test]
    fn cell_mut_allows_in_place_write() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        *db.cell_mut("t", 0, 0).unwrap() = Value::Integer(99);

        assert_eq!(db.table("t").unwrap().rows[0][0], Value::Integer(99));
    }

    #[test]
    fn cell_mut_reports_out_of_range_row_and_column() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();

        assert_eq!(
            db.cell_mut("t", 5, 0),
            Err(Error::IndexOutOfBounds { index: 5, len: 1 })
        );
        assert_eq!(
            db.cell_mut("t", 0, 5),
            Err(Error::IndexOutOfBounds { index: 5, len: 1 })
        );
    }

    #[test]
    fn insert_row_returning_rowid_assigns_increasing_rowids() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();

        let first = db
            .insert_row_returning_rowid("t", vec![Value::Integer(1)])
            .unwrap();
        let second = db
            .insert_row_returning_rowid("t", vec![Value::Integer(2)])
            .unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(db.table("t").unwrap().row_ids, vec![1, 2]);
    }

    #[test]
    fn plain_insert_row_still_assigns_rowids_internally() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(1)]).unwrap();
        db.insert_row("t", vec![Value::Integer(2)]).unwrap();

        assert_eq!(db.table("t").unwrap().row_ids, vec![1, 2]);
    }

    #[test]
    fn rowid_counters_are_independent_per_table() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t1 (a INTEGER)"))
            .unwrap();
        db.create_table(&create("CREATE TABLE t2 (a INTEGER)"))
            .unwrap();

        db.insert_row("t1", vec![Value::Integer(1)]).unwrap();
        db.insert_row("t1", vec![Value::Integer(2)]).unwrap();
        db.insert_row("t2", vec![Value::Integer(1)]).unwrap();

        assert_eq!(db.table("t1").unwrap().row_ids, vec![1, 2]);
        assert_eq!(db.table("t2").unwrap().row_ids, vec![1]);
    }

    #[test]
    fn insert_with_wrong_column_count_is_an_error() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER, b TEXT)"))
            .unwrap();
        assert!(matches!(
            db.insert_row("t", vec![Value::Integer(1)]),
            Err(Error::ColumnCountMismatch {
                expected: 2,
                actual: 1
            })
        ));
    }

    struct ConstantSource {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    }

    impl TableSource for ConstantSource {
        fn column_names(&self) -> &[String] {
            &self.columns
        }
        fn scan(&self, _filter: Option<&Expr>) -> Result<Vec<Vec<Value>>> {
            Ok(self.rows.clone())
        }
    }

    #[test]
    fn scan_dispatches_to_a_registered_virtual_table() {
        let mut db = Database::new();
        db.register_virtual_table(
            "v".to_string(),
            Box::new(ConstantSource {
                columns: vec!["a".to_string()],
                rows: vec![vec![Value::Integer(1)], vec![Value::Integer(2)]],
            }),
        );

        let (columns, rows) = db.scan("v", None).unwrap();
        assert_eq!(columns, vec!["a".to_string()]);
        assert_eq!(rows, vec![vec![Value::Integer(1)], vec![Value::Integer(2)]]);
    }

    #[test]
    fn scan_prefers_a_native_table_over_a_virtual_table_with_the_same_name() {
        let mut db = Database::new();
        db.create_table(&create("CREATE TABLE t (a INTEGER)"))
            .unwrap();
        db.insert_row("t", vec![Value::Integer(99)]).unwrap();
        db.register_virtual_table(
            "t".to_string(),
            Box::new(ConstantSource {
                columns: vec!["a".to_string()],
                rows: vec![vec![Value::Integer(1)]],
            }),
        );

        let (_, rows) = db.scan("t", None).unwrap();
        assert_eq!(rows, vec![vec![Value::Integer(99)]]);
    }

    #[test]
    fn scan_on_unregistered_name_is_table_not_found() {
        let db = Database::new();
        assert!(matches!(
            db.scan("missing", None),
            Err(Error::TableNotFound(_))
        ));
    }

    #[test]
    fn drop_table_on_a_virtual_table_is_rejected() {
        let mut db = Database::new();
        db.register_virtual_table(
            "v".to_string(),
            Box::new(ConstantSource {
                columns: vec!["a".to_string()],
                rows: vec![vec![Value::Integer(1)]],
            }),
        );

        assert!(matches!(
            db.drop_table("v", false),
            Err(Error::CannotDropVirtualTable(_))
        ));
        // Rejected regardless of IF EXISTS -- it isn't a "missing table"
        // case IF EXISTS is meant to no-op.
        assert!(matches!(
            db.drop_table("v", true),
            Err(Error::CannotDropVirtualTable(_))
        ));
    }
}
