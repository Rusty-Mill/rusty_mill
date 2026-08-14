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
            },
        );
        Ok(())
    }

    /// Inserts one row into `table_name`, given values in the table's
    /// column-definition order. Column-list-driven inserts (reordering or
    /// omitting columns) are the caller's responsibility to expand into
    /// this shape until a catalog-aware insert path exists.
    pub fn insert_row(&mut self, table_name: &str, row: Vec<Value>) -> Result<()> {
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
        table.rows.push(row);
        Ok(())
    }

    /// Returns a table's schema and rows for scanning.
    pub fn table(&self, table_name: &str) -> Result<&Table> {
        self.tables
            .get(table_name)
            .ok_or_else(|| Error::TableNotFound(table_name.to_string()))
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
