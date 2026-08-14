//! Execution engine (foundation-tier `A7`): ties the parser ASTs, storage
//! layer, and expression evaluator together into an actual query path.
//! Deliberately scoped to single-table scan + filter + project — no
//! joins, aggregates, subqueries, or indexes yet.

use crate::aggregate::Aggregate;
use crate::ddl::CreateTable;
use crate::dml_insert::Insert;
use crate::dml_select::{AggregateArg, Select, SelectColumns};
use crate::error::{Error, Result};
use crate::eval::{evaluate_bool_with_functions, evaluate_with_functions, ScalarFn};
use crate::storage::Database;
use crate::value::Value;
use std::collections::HashMap;

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
    execute_insert_returning_rowids(db, insert).map(|rowids| rowids.len())
}

/// Like [`execute_insert`], but returns each inserted row's newly
/// assigned rowid, in insertion order — added alongside the original
/// (rather than changing its return type) so this doesn't break the
/// already-shipped `Result<usize>` signature. `Connection::execute` uses
/// this to power `last_insert_rowid` and real (rather than
/// row-position-based) `update_hook` rowids.
pub fn execute_insert_returning_rowids(db: &mut Database, insert: &Insert) -> Result<Vec<i64>> {
    let table_column_names = db.table(&insert.table_name)?.column_names.clone();

    let mut rowids = Vec::with_capacity(insert.rows.len());
    for row in &insert.rows {
        let expanded = match &insert.columns {
            None => row.clone(),
            Some(names) => expand_row(&table_column_names, names, row)?,
        };
        rowids.push(db.insert_row_returning_rowid(&insert.table_name, expanded)?);
    }
    Ok(rowids)
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
/// and rows. Errors if `select.filter` contains a function call — see
/// [`execute_select_with_functions`].
pub fn execute_select(db: &Database, select: &Select) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    execute_select_with_functions(db, select, &HashMap::new())
}

/// Like [`execute_select`], but resolves scalar function calls in
/// `select.filter` against `functions` (name → implementation). Part B
/// gap row "Connection + functions module: scalar SQL functions" —
/// `Connection` registers functions here via `create_scalar_function`.
pub fn execute_select_with_functions(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let table = db.table(&select.table_name)?;

    let mut matching_rows = Vec::new();
    for row in &table.rows {
        let keep = match &select.filter {
            Some(filter) => {
                evaluate_bool_with_functions(filter, &table.column_names, row, functions)?
            }
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
        SelectColumns::Aggregates(_) => Err(Error::UnrecognizedStatement(
            "aggregate select lists need execute_select_with_aggregates".to_string(),
        )),
    }
}

/// Executes a whole-table aggregate `SELECT` (e.g. `SELECT COUNT(*), SUM(a)
/// FROM t`), folding every row matching `select.filter` through each
/// aggregate call's `step` and producing exactly one output row — this
/// crate has no `GROUP BY`, so there's no per-group fan-out. Errors if
/// `select.columns` isn't [`SelectColumns::Aggregates`].
pub fn execute_select_with_aggregates(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let table = db.table(&select.table_name)?;
    let calls = match &select.columns {
        SelectColumns::Aggregates(calls) => calls,
        _ => {
            return Err(Error::UnrecognizedStatement(
                "execute_select_with_aggregates called on a non-aggregate SELECT".to_string(),
            ))
        }
    };

    let mut aggs = Vec::with_capacity(calls.len());
    let mut accumulators = Vec::with_capacity(calls.len());
    for call in calls {
        let agg = aggregates
            .get(&call.name)
            .ok_or_else(|| Error::FunctionNotFound(call.name.clone()))?;
        accumulators.push(agg.init.clone());
        aggs.push(agg);
    }

    for row in &table.rows {
        let keep = match &select.filter {
            Some(filter) => {
                evaluate_bool_with_functions(filter, &table.column_names, row, functions)?
            }
            None => true,
        };
        if !keep {
            continue;
        }
        for (i, call) in calls.iter().enumerate() {
            let arg_value = match &call.arg {
                AggregateArg::Star => Value::Integer(1),
                AggregateArg::Expr(expr) => {
                    evaluate_with_functions(expr, &table.column_names, row, functions)?
                }
            };
            accumulators[i] = (aggs[i].step)(&accumulators[i], &[arg_value])?;
        }
    }

    let result_row = aggs
        .iter()
        .zip(accumulators)
        .map(|(agg, acc)| (agg.finalize)(acc))
        .collect::<Result<Vec<Value>>>()?;
    let column_names = calls.iter().map(describe_aggregate_call).collect();

    Ok((column_names, vec![result_row]))
}

/// A result-column name for an aggregate call, e.g. `COUNT(*)` or
/// `SUM(a)`. Simplified relative to real SQLite's full result-column-name
/// inference: any non-column expression argument is just shown as `expr`.
/// `pub(crate)` so [`crate::Statement`] (a different module) can reuse it
/// for `Statement::column_names` on an aggregate `SELECT`.
pub(crate) fn describe_aggregate_call(call: &crate::dml_select::AggregateCall) -> String {
    let arg = match &call.arg {
        AggregateArg::Star => "*".to_string(),
        AggregateArg::Expr(expr) => match expr.as_ref() {
            crate::dml_select::Expr::Column(name) => name.clone(),
            _ => "expr".to_string(),
        },
    };
    format!("{}({arg})", call.name)
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

    #[test]
    fn selects_with_registered_function_in_filter() {
        let db = setup();
        let select =
            parse_select(&tokenize("SELECT * FROM t WHERE DOUBLE(a) = 4").unwrap()).unwrap();

        let mut functions: HashMap<String, Box<crate::eval::ScalarFn>> = HashMap::new();
        functions.insert(
            "DOUBLE".to_string(),
            Box::new(|args: &[Value]| match args {
                [Value::Integer(n)] => Ok(Value::Integer(n * 2)),
                _ => Err(Error::FunctionNotFound("DOUBLE".into())),
            }),
        );

        let (_, rows) = execute_select_with_functions(&db, &select, &functions).unwrap();
        assert_eq!(rows, vec![vec![Value::Integer(2), Value::Text("y".into())]]);
    }

    #[test]
    fn execute_select_errors_on_unregistered_function() {
        let db = setup();
        let select =
            parse_select(&tokenize("SELECT * FROM t WHERE DOUBLE(a) = 4").unwrap()).unwrap();
        assert!(execute_select(&db, &select).is_err());
    }
}
