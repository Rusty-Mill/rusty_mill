//! In-memory page/record storage backend (foundation-tier `A5`).
//! Deliberately in-memory-only — see `ARCHITECTURE.md`'s non-goals for why
//! the on-disk file format isn't in scope yet.

use crate::ddl::{ColumnDef, CreateTable};
use crate::error::{Error, Result};
use crate::value::Value;
use std::collections::HashMap;

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

/// The full set of tables in a database. This is the storage layer that
/// [`crate::Connection`] will be wired to in `A8`.
#[derive(Debug, Default)]
pub struct Database {
    tables: HashMap<String, Table>,
}

impl Database {
    /// Creates a new, empty in-memory database.
    pub fn new() -> Database {
        Database {
            tables: HashMap::new(),
        }
    }

    /// Creates a table from a parsed `CREATE TABLE` statement.
    pub fn create_table(&mut self, create: &CreateTable) -> Result<()> {
        if self.tables.contains_key(&create.table_name) {
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
