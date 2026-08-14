//! Execution engine (foundation-tier `A7`): ties the parser ASTs, storage
//! layer, and expression evaluator together into an actual query path.
//! Deliberately scoped to single-table scan + filter + project — no
//! joins, aggregates, subqueries, or indexes yet.

use crate::ddl::CreateTable;
use crate::dml_insert::Insert;
use crate::dml_select::{Select, SelectColumns};
use crate::error::{Error, Result};
use crate::eval::evaluate_bool;
use crate::storage::Database;
use crate::value::Value;

/// Executes a `CREATE TABLE` statement.
pub fn execute_create_table(db: &mut Database, create: &CreateTable) -> Result<()> {
    db.create_table(create)
}

/// Executes an `INSERT` statement, returning the number of rows inserted.
///
/// When `insert.columns` names an explicit (and possibly reordered or
/// partial) column list, each row is expanded into full
/// table-definition order, filling any column not named with `NULL`.
pub fn execute_insert(db: &mut Database, insert: &Insert) -> Result<usize> {
    let table_column_names = db.table(&insert.table_name)?.column_names.clone();

    for row in &insert.rows {
        let expanded = match &insert.columns {
            None => row.clone(),
            Some(names) => expand_row(&table_column_names, names, row)?,
        };
        db.insert_row(&insert.table_name, expanded)?;
    }
    Ok(insert.rows.len())
}

fn expand_row(
    table_column_names: &[String],
    given_names: &[String],
    given_values: &[Value],
) -> Result<Vec<Value>> {
    if given_names.len() != given_values.len() {
        return Err(Error::ColumnCountMismatch {
            expected: given_names.len(),
            actual: given_values.len(),
        });
    }
    table_column_names
        .iter()
        .map(|col| match given_names.iter().position(|n| n == col) {
            Some(idx) => Ok(given_values[idx].clone()),
            None => Ok(Value::Null),
        })
        .collect()
}

/// Executes a single-table `SELECT`, returning the result's column names
/// and rows.
pub fn execute_select(db: &Database, select: &Select) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let table = db.table(&select.table_name)?;

    let mut matching_rows = Vec::new();
    for row in &table.rows {
        let keep = match &select.filter {
            Some(filter) => evaluate_bool(filter, &table.column_names, row)?,
            None => true,
        };
        if keep {
            matching_rows.push(row);
        }
    }

    match &select.columns {
        SelectColumns::All => {
            let rows = matching_rows.into_iter().cloned().collect();
            Ok((table.column_names.clone(), rows))
        }
        SelectColumns::Named(names) => {
            let indices = names
                .iter()
                .map(|n| {
                    table
                        .column_names
                        .iter()
                        .position(|c| c == n)
                        .ok_or_else(|| Error::UnknownColumn(n.clone()))
                })
                .collect::<Result<Vec<usize>>>()?;
            let rows = matching_rows
                .into_iter()
                .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
                .collect();
            Ok((names.clone(), rows))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_create_table, parse_insert, parse_select, tokenize};

    fn setup() -> Database {
        let mut db = Database::new();
        let create =
            parse_create_table(&tokenize("CREATE TABLE t (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        let insert =
            parse_insert(&tokenize("INSERT INTO t VALUES (1, 'x'), (2, 'y'), (3, 'z')").unwrap())
                .unwrap();
        execute_insert(&mut db, &insert).unwrap();
        db
    }

    #[test]
    fn selects_all_rows_unfiltered() {
        let db = setup();
        let select = parse_select(&tokenize("SELECT * FROM t").unwrap()).unwrap();
        let (cols, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(cols, vec!["a", "b"]);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn selects_with_where_filter() {
        let db = setup();
        let select = parse_select(&tokenize("SELECT * FROM t WHERE a = 2").unwrap()).unwrap();
        let (_, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(rows, vec![vec![Value::Integer(2), Value::Text("y".into())]]);
    }

    #[test]
    fn projects_named_columns() {
        let db = setup();
        let select = parse_select(&tokenize("SELECT b FROM t WHERE a = 1").unwrap()).unwrap();
        let (cols, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(cols, vec!["b"]);
        assert_eq!(rows, vec![vec![Value::Text("x".into())]]);
    }

    #[test]
    fn insert_with_reordered_column_list() {
        let mut db = Database::new();
        let create =
            parse_create_table(&tokenize("CREATE TABLE t (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        let insert =
            parse_insert(&tokenize("INSERT INTO t (b, a) VALUES ('x', 1)").unwrap()).unwrap();
        execute_insert(&mut db, &insert).unwrap();
        let table = db.table("t").unwrap();
        assert_eq!(
            table.rows[0],
            vec![Value::Integer(1), Value::Text("x".into())]
        );
    }

    #[test]
    fn insert_with_partial_column_list_nulls_the_rest() {
        let mut db = Database::new();
        let create =
            parse_create_table(&tokenize("CREATE TABLE t (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        let insert = parse_insert(&tokenize("INSERT INTO t (a) VALUES (1)").unwrap()).unwrap();
        execute_insert(&mut db, &insert).unwrap();
        let table = db.table("t").unwrap();
        assert_eq!(table.rows[0], vec![Value::Integer(1), Value::Null]);
    }
}
