//! Execution engine (foundation-tier `A7`): ties the parser ASTs, storage
//! layer, and expression evaluator together into an actual query path.
//! Deliberately scoped to single-table scan + filter + project — no
//! joins, aggregates, subqueries, or indexes yet.

use crate::aggregate::Aggregate;
use crate::ddl::CreateTable;
use crate::dml_insert::Insert;
use crate::dml_select::{AggregateArg, Expr, Select, SelectColumns};
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
        let values = expanded
            .iter()
            .map(resolve_insert_value)
            .collect::<Result<Vec<Value>>>()?;
        rowids.push(db.insert_row_returning_rowid(&insert.table_name, values)?);
    }
    Ok(rowids)
}

/// Executes an `INSERT` into a *registered virtual table* (issue #95),
/// returning the number of rows inserted. Resolves each row the same
/// way [`execute_insert_returning_rowids`] does for native tables
/// (column-list expansion, parameter/literal resolution), but writes
/// through [`crate::storage::Database::insert_into_virtual_table`]
/// (→ [`crate::storage::TableSource::insert`]) instead of
/// [`crate::storage::Database::insert_row`]. Virtual tables have no
/// rowid concept (see `src/vtab.rs`'s module doc comment), so unlike
/// [`execute_insert_returning_rowids`] this returns just the affected
/// count — there's nothing rowid-shaped to return.
pub fn execute_insert_into_virtual_table(db: &mut Database, insert: &Insert) -> Result<usize> {
    let column_names = db.virtual_table_column_names(&insert.table_name)?;

    let mut affected = 0;
    for row in &insert.rows {
        let expanded = match &insert.columns {
            None => row.clone(),
            Some(names) => expand_row(&column_names, names, row)?,
        };
        let values = expanded
            .iter()
            .map(resolve_insert_value)
            .collect::<Result<Vec<Value>>>()?;
        db.insert_into_virtual_table(&insert.table_name, values)?;
        affected += 1;
    }
    Ok(affected)
}

/// Resolves an `INSERT` value slot into a concrete [`Value`] —
/// [`Expr::Literal`] as-is, [`Expr::Parameter`] as [`Value::Null`]
/// (matching real SQLite's unbound-parameter default; `crate::Statement`
/// pre-substitutes bound values into a `Literal`-only tree before this
/// ever runs, so this fallback only fires for callers with no bindings
/// to consult). The `INSERT` parser never produces any other `Expr`
/// variant here, but the match stays exhaustive rather than assuming
/// that forever.
fn resolve_insert_value(expr: &Expr) -> Result<Value> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Parameter(_) => Ok(Value::Null),
        other => Err(Error::UnrecognizedStatement(format!(
            "unsupported INSERT value expression: {other:?}"
        ))),
    }
}

fn expand_row(
    table_column_names: &[String],
    given_names: &[String],
    given_values: &[Expr],
) -> Result<Vec<Expr>> {
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
            None => Ok(Expr::Literal(Value::Null)),
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
    let (column_names, rows) = db.scan(&select.table_name, select.filter.as_ref())?;

    let mut matching_rows = Vec::new();
    for row in &rows {
        let keep = match &select.filter {
            Some(filter) => evaluate_bool_with_functions(filter, &column_names, row, functions)?,
            None => true,
        };
        if keep {
            matching_rows.push(row);
        }
    }

    match &select.columns {
        SelectColumns::All => {
            let rows = matching_rows.into_iter().cloned().collect();
            Ok((column_names, dedup_rows(select.distinct, rows)))
        }
        SelectColumns::Named(names) => {
            let indices = names
                .iter()
                .map(|n| {
                    column_names
                        .iter()
                        .position(|c| c == n)
                        .ok_or_else(|| Error::UnknownColumn(n.clone()))
                })
                .collect::<Result<Vec<usize>>>()?;
            let rows = matching_rows
                .into_iter()
                .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
                .collect();
            Ok((names.clone(), dedup_rows(select.distinct, rows)))
        }
        SelectColumns::Aggregates(_) => Err(Error::UnrecognizedStatement(
            "aggregate select lists need execute_select_with_aggregates".to_string(),
        )),
        SelectColumns::Window(_) => Err(Error::UnrecognizedStatement(
            "window select lists need execute_select_with_window".to_string(),
        )),
    }
}

/// Dedups `rows` (preserving first-occurrence order) if `distinct`, a
/// no-op otherwise — backs `SELECT DISTINCT` (issue #116). Linear
/// membership check, not hash-based: `Value` doesn't implement `Hash`/
/// `Eq` (a `Real(f64)` payload can't), same constraint already noted on
/// `execute_select_with_window`'s partition lookup. Row counts in this
/// crate's tables are small enough that this isn't a practical concern.
fn dedup_rows(distinct: bool, rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    if !distinct {
        return rows;
    }
    let mut result: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
    for row in rows {
        if !result.contains(&row) {
            result.push(row);
        }
    }
    result
}

/// Executes a whole-table aggregate `SELECT` (e.g. `SELECT COUNT(*), SUM(a)
/// FROM t`), folding every row matching `select.filter` through each
/// aggregate call's `step` and producing exactly one output row — this
/// crate has no `GROUP BY`, so there's no per-group fan-out. Errors if
/// `select.columns` isn't [`SelectColumns::Aggregates`]. `select.distinct`
/// is a syntactic no-op here (issue #116) — there's always exactly one
/// output row, nothing to dedup.
pub fn execute_select_with_aggregates(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let (column_names, rows) = db.scan(&select.table_name, select.filter.as_ref())?;
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

    for row in &rows {
        let keep = match &select.filter {
            Some(filter) => evaluate_bool_with_functions(filter, &column_names, row, functions)?,
            None => true,
        };
        if !keep {
            continue;
        }
        for (i, call) in calls.iter().enumerate() {
            let arg_value = match &call.arg {
                AggregateArg::Star => Value::Integer(1),
                AggregateArg::Expr(expr) => {
                    evaluate_with_functions(expr, &column_names, row, functions)?
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

/// Executes a window-function `SELECT` (e.g.
/// `SELECT SUM(a) OVER (PARTITION BY b) FROM t`). Every matching row
/// (per `select.filter`) keeps its own output row — unlike
/// [`execute_select_with_aggregates`], this doesn't collapse to one row
/// — but per [`crate::dml_select::WindowCall`]'s documented scope, every
/// row within the same partition gets the same whole-partition aggregate
/// value (no `ORDER BY`/running-frame support). Errors if
/// `select.columns` isn't [`SelectColumns::Window`].
pub fn execute_select_with_window(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let (column_names, rows) = db.scan(&select.table_name, select.filter.as_ref())?;
    let calls = match &select.columns {
        SelectColumns::Window(calls) => calls,
        _ => {
            return Err(Error::UnrecognizedStatement(
                "execute_select_with_window called on a non-window SELECT".to_string(),
            ))
        }
    };

    let mut matching_rows = Vec::new();
    for row in &rows {
        let keep = match &select.filter {
            Some(filter) => evaluate_bool_with_functions(filter, &column_names, row, functions)?,
            None => true,
        };
        if keep {
            matching_rows.push(row);
        }
    }

    let mut result_columns: Vec<Vec<Value>> = Vec::with_capacity(calls.len());
    for call in calls {
        let agg = aggregates
            .get(&call.name)
            .ok_or_else(|| Error::FunctionNotFound(call.name.clone()))?;

        // Linear (not hashed) partition lookup: `Value` doesn't
        // implement `Hash`/`Eq` (a `Real(f64)` payload can't), so a
        // `HashMap<Vec<Value>, _>` isn't available here — partition
        // counts in this crate's tables are small enough that this
        // isn't a practical concern.
        let mut partitions: Vec<(Vec<Value>, Value)> = Vec::new();
        let mut row_partition: Vec<usize> = Vec::with_capacity(matching_rows.len());

        for row in &matching_rows {
            let key = partition_key(&call.partition_by, &column_names, row)?;
            let arg_value = match &call.arg {
                AggregateArg::Star => Value::Integer(1),
                AggregateArg::Expr(expr) => {
                    evaluate_with_functions(expr, &column_names, row, functions)?
                }
            };
            let idx = match partitions.iter().position(|(k, _)| *k == key) {
                Some(idx) => idx,
                None => {
                    partitions.push((key, agg.init.clone()));
                    partitions.len() - 1
                }
            };
            partitions[idx].1 = (agg.step)(&partitions[idx].1, &[arg_value])?;
            row_partition.push(idx);
        }

        let values = row_partition
            .iter()
            .map(|&idx| (agg.finalize)(partitions[idx].1.clone()))
            .collect::<Result<Vec<Value>>>()?;
        result_columns.push(values);
    }

    let rows = (0..matching_rows.len())
        .map(|i| result_columns.iter().map(|col| col[i].clone()).collect())
        .collect();
    let column_names = calls.iter().map(describe_window_call).collect();

    Ok((column_names, dedup_rows(select.distinct, rows)))
}

/// Looks up `partition_by`'s column values in `row`, for grouping rows
/// into partitions in [`execute_select_with_window`].
fn partition_key(
    partition_by: &[String],
    column_names: &[String],
    row: &[Value],
) -> Result<Vec<Value>> {
    partition_by
        .iter()
        .map(|col| {
            column_names
                .iter()
                .position(|c| c == col)
                .map(|idx| row[idx].clone())
                .ok_or_else(|| Error::UnknownColumn(col.clone()))
        })
        .collect()
}

/// A result-column name for a window call, e.g. `SUM(a) OVER (PARTITION
/// BY b)` or `COUNT(*) OVER ()`. `pub(crate)` so [`crate::Statement`]
/// can reuse it for `Statement::column_names` on a window `SELECT`.
pub(crate) fn describe_window_call(call: &crate::dml_select::WindowCall) -> String {
    let arg = match &call.arg {
        AggregateArg::Star => "*".to_string(),
        AggregateArg::Expr(expr) => match expr.as_ref() {
            crate::dml_select::Expr::Column(name) => name.clone(),
            _ => "expr".to_string(),
        },
    };
    if call.partition_by.is_empty() {
        format!("{}({arg}) OVER ()", call.name)
    } else {
        format!(
            "{}({arg}) OVER (PARTITION BY {})",
            call.name,
            call.partition_by.join(", ")
        )
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

    struct ConstantSource {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    }

    impl crate::storage::TableSource for ConstantSource {
        fn column_names(&self) -> &[String] {
            &self.columns
        }
        fn scan(&self, _filter: Option<&Expr>) -> Result<Vec<Vec<Value>>> {
            Ok(self.rows.clone())
        }
    }

    #[test]
    fn select_star_scans_a_virtual_table_end_to_end() {
        let mut db = Database::new();
        db.register_virtual_table(
            "v".to_string(),
            Box::new(ConstantSource {
                columns: vec!["a".to_string(), "b".to_string()],
                rows: vec![
                    vec![Value::Integer(1), Value::Text("x".into())],
                    vec![Value::Integer(2), Value::Text("y".into())],
                ],
            }),
        );

        let select = parse_select(&tokenize("SELECT * FROM v WHERE a = 2").unwrap()).unwrap();
        let (cols, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(cols, vec!["a", "b"]);
        assert_eq!(rows, vec![vec![Value::Integer(2), Value::Text("y".into())]]);
    }

    #[test]
    fn select_named_columns_projects_a_virtual_table() {
        let mut db = Database::new();
        db.register_virtual_table(
            "v".to_string(),
            Box::new(ConstantSource {
                columns: vec!["a".to_string(), "b".to_string()],
                rows: vec![vec![Value::Integer(1), Value::Text("x".into())]],
            }),
        );

        let select = parse_select(&tokenize("SELECT b FROM v").unwrap()).unwrap();
        let (cols, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(cols, vec!["b"]);
        assert_eq!(rows, vec![vec![Value::Text("x".into())]]);
    }

    fn setup_with_duplicates() -> Database {
        let mut db = Database::new();
        let create =
            parse_create_table(&tokenize("CREATE TABLE t (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        let insert = parse_insert(
            &tokenize("INSERT INTO t VALUES (1, 'x'), (2, 'x'), (1, 'x'), (3, 'y')").unwrap(),
        )
        .unwrap();
        execute_insert(&mut db, &insert).unwrap();
        db
    }

    #[test]
    fn select_distinct_dedups_full_rows() {
        let db = setup_with_duplicates();
        let select = parse_select(&tokenize("SELECT DISTINCT a, b FROM t").unwrap()).unwrap();
        let (_, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(
            rows,
            vec![
                vec![Value::Integer(1), Value::Text("x".into())],
                vec![Value::Integer(2), Value::Text("x".into())],
                vec![Value::Integer(3), Value::Text("y".into())],
            ]
        );
    }

    #[test]
    fn select_distinct_on_a_single_projected_column_dedups_after_projection() {
        let db = setup_with_duplicates();
        let select = parse_select(&tokenize("SELECT DISTINCT b FROM t").unwrap()).unwrap();
        let (_, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(
            rows,
            vec![vec![Value::Text("x".into())], vec![Value::Text("y".into())]]
        );
    }

    #[test]
    fn select_without_distinct_keeps_duplicates() {
        let db = setup_with_duplicates();
        let select = parse_select(&tokenize("SELECT a, b FROM t").unwrap()).unwrap();
        let (_, rows) = execute_select(&db, &select).unwrap();
        assert_eq!(rows.len(), 4);
    }
}
