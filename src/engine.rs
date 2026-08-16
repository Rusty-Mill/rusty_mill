//! Execution engine (foundation-tier `A7`): ties the parser ASTs, storage
//! layer, and expression evaluator together into an actual query path.
//! Deliberately scoped to single-table scan + filter + project — no
//! joins, aggregates, subqueries, or indexes yet.

use crate::aggregate::Aggregate;
use crate::ddl::{AlterTable, CreateIndex, CreateTable, DropIndex, DropTable};
use crate::dml_delete::Delete;
use crate::dml_insert::{Insert, InsertSource};
use crate::dml_select::{
    describe_aggregate_call, AggregateArg, CompoundOp, CompoundSelect, Expr, JoinCondition,
    JoinKind, Select, SelectColumns, WithSelect,
};
use crate::dml_update::Update;
use crate::error::{Error, Result};
use crate::eval::{
    evaluate_bool_with_functions, evaluate_with_functions, resolve_column_index, ScalarFn,
};
use crate::storage::Database;
use crate::value::Value;
use std::collections::HashMap;

/// Executes a `CREATE TABLE` statement.
pub fn execute_create_table(db: &mut Database, create: &CreateTable) -> Result<()> {
    db.create_table(create)
}

/// Executes a `DROP TABLE` statement (issue #120).
pub fn execute_drop_table(db: &mut Database, drop: &DropTable) -> Result<()> {
    db.drop_table(&drop.table_name, drop.if_exists)
}

/// Executes an `UPDATE` statement (issue #128), returning each updated
/// row's rowid (in table row order) — the same "affected count via
/// `.len()`, plus real rowids for `update_hook`" shape
/// [`execute_insert_returning_rowids`] already established for `INSERT`.
pub fn execute_update(db: &mut Database, update: &Update) -> Result<Vec<i64>> {
    db.update_rows(
        &update.table_name,
        &update.assignments,
        update.filter.as_ref(),
    )
}

/// Executes a `DELETE` statement (issue #129), returning each deleted
/// row's rowid (in table row order) — same "affected count via
/// `.len()`, plus real rowids for `update_hook`" shape [`execute_update`]
/// already established.
pub fn execute_delete(db: &mut Database, delete: &Delete) -> Result<Vec<i64>> {
    db.delete_rows(&delete.table_name, delete.filter.as_ref())
}

/// Executes an `ALTER TABLE` statement (issue #121).
pub fn execute_alter_table(db: &mut Database, alter: &AlterTable) -> Result<()> {
    db.alter_table(&alter.table_name, &alter.action)
}

/// Executes a `CREATE INDEX` statement (issue #122) — records metadata
/// only, see [`crate::ddl::CreateIndex`]'s doc comment.
pub fn execute_create_index(db: &mut Database, create: &CreateIndex) -> Result<()> {
    db.create_index(
        &create.index_name,
        &create.table_name,
        &create.columns,
        create.if_not_exists,
    )
}

/// Executes a `DROP INDEX` statement (issue #122).
pub fn execute_drop_index(db: &mut Database, drop: &DropIndex) -> Result<()> {
    db.drop_index(&drop.index_name, drop.if_exists)
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
    let value_rows = resolve_insert_source(db, insert, &table_column_names)?;

    let mut rowids = Vec::with_capacity(value_rows.len());
    for values in value_rows {
        // `OrConflict::Ignore` returns `None` for a silently skipped row
        // (issue #123) -- not pushed, so it doesn't count toward the
        // statement's affected-row count or fire `update_hook`.
        if let Some(rowid) = db.insert_row_returning_rowid_with_conflict(
            &insert.table_name,
            values,
            insert.or_conflict,
        )? {
            rowids.push(rowid);
        }
    }
    Ok(rowids)
}

/// Resolves `insert.source` into concrete `Value` rows, in target
/// `table_column_names` order (column-list expansion applied, `NULL`
/// filled for any column not named).
///
/// For [`InsertSource::Select`] (issue #124), the `SELECT` is fully
/// executed first via the plain [`execute_select`] — eager, not
/// streaming, and function-call/aggregate-free (no access to a
/// [`crate::Connection`]'s registered scalar functions/aggregates from
/// here; a `SELECT` using one would need `Connection::execute`'s own
/// dispatch instead, which isn't wired up for this sub-form — a
/// documented scope cut, not silently dropped support). Its output rows
/// are then treated exactly like a `VALUES` row: expanded through the
/// same [`expand_row`] column-list logic (via a round-trip through
/// [`Expr::Literal`], since `expand_row` operates on `Expr`) rather than
/// duplicating that reordering logic for already-resolved `Value`s.
fn resolve_insert_source(
    db: &Database,
    insert: &Insert,
    table_column_names: &[String],
) -> Result<Vec<Vec<Value>>> {
    match &insert.source {
        InsertSource::Values(rows) => rows
            .iter()
            .map(|row| {
                let expanded = match &insert.columns {
                    None => row.clone(),
                    Some(names) => expand_row(table_column_names, names, row)?,
                };
                expanded
                    .iter()
                    .map(resolve_insert_value)
                    .collect::<Result<Vec<Value>>>()
            })
            .collect(),
        InsertSource::Select(select) => {
            let (_, rows) = execute_select(db, select)?;
            match &insert.columns {
                None => Ok(rows),
                Some(names) => rows
                    .into_iter()
                    .map(|row| {
                        let row_exprs: Vec<Expr> = row.into_iter().map(Expr::Literal).collect();
                        let expanded = expand_row(table_column_names, names, &row_exprs)?;
                        expanded
                            .iter()
                            .map(resolve_insert_value)
                            .collect::<Result<Vec<Value>>>()
                    })
                    .collect(),
            }
        }
    }
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
///
/// **Scope note (issue #124):** `INSERT ... SELECT` into a virtual
/// table isn't supported — errors clearly rather than being silently
/// mishandled. `INSERT ... SELECT` support only covers native-table
/// targets, matching that issue's own focus.
pub fn execute_insert_into_virtual_table(db: &mut Database, insert: &Insert) -> Result<usize> {
    let column_names = db.virtual_table_column_names(&insert.table_name)?;
    let rows = match &insert.source {
        InsertSource::Values(rows) => rows,
        InsertSource::Select(_) => {
            return Err(Error::UnrecognizedStatement(
                "INSERT ... SELECT into a virtual table is not supported".to_string(),
            ))
        }
    };

    let mut affected = 0;
    for row in rows {
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
    // GROUP BY/HAVING (issue #125) only make sense alongside an
    // aggregate select list -- see execute_select_with_aggregates.
    if !select.group_by.is_empty() || select.having.is_some() {
        return Err(Error::UnrecognizedStatement(
            "GROUP BY/HAVING require an aggregate SELECT list".to_string(),
        ));
    }
    // A joined row source's WHERE may reference more than one table's
    // columns, so (issue #130) it can't be pushed into `db.scan` the way
    // a single-table filter can -- `scan_joined` deliberately doesn't
    // take one; it's applied once, below, over the fully joined rows.
    let (column_names, rows) = if select.joins.is_empty() {
        db.scan(&select.table_name, select.filter.as_ref())?
    } else {
        scan_joined(db, select, functions)?
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

    match &select.columns {
        SelectColumns::All => {
            // Bare (unqualified) output names, matching real SQLite's own
            // `SELECT *` convention -- a no-op for the non-joined path,
            // where `column_names` is already bare.
            let output_names = column_names.iter().map(|c| bare_name(c)).collect();
            let rows = matching_rows.into_iter().cloned().collect();
            Ok((output_names, dedup_rows(select.distinct, rows)))
        }
        SelectColumns::Named(names) => {
            let indices = names
                .iter()
                .map(|n| resolve_column_index(&column_names, n))
                .collect::<Result<Vec<usize>>>()?;
            let output_names = names.iter().map(|n| bare_name(n)).collect();
            let rows = matching_rows
                .into_iter()
                .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
                .collect();
            Ok((output_names, dedup_rows(select.distinct, rows)))
        }
        SelectColumns::Aggregates(_) => Err(Error::UnrecognizedStatement(
            "aggregate select lists need execute_select_with_aggregates".to_string(),
        )),
        SelectColumns::Window(_) => Err(Error::UnrecognizedStatement(
            "window select lists need execute_select_with_window".to_string(),
        )),
    }
}

/// The last `.`-separated segment of a (possibly `"qualifier.column"`-
/// qualified — issue #130) column name, e.g. `"t1.a"` → `"a"`, `"a"` →
/// `"a"`. Used for result-column naming: real SQLite's own convention is
/// that a joined query's output columns are named by their bare column
/// name (not table-qualified), even though resolution internally needs
/// the qualified form to disambiguate. `pub(crate)` so [`crate::Statement::
/// column_names`] (a different module) can report the exact same names
/// its `query*` methods' rows actually come back under.
pub(crate) fn bare_name(qualified: &str) -> String {
    qualified
        .rsplit('.')
        .next()
        .unwrap_or(qualified)
        .to_string()
}

/// Builds one combined row source from `select.table_name`/`table_alias`
/// and `select.joins` (issue #130) via a nested-loop join — the natural
/// fit for this crate's in-memory, index-free model (see the issue's own
/// note on why). Each join folds into the accumulated rows left to
/// right, so a 3+-table join chain works the same way a 2-table one
/// does. Returned column names are always `"qualifier.column"` (a
/// table's alias, or its own name if unaliased) — see
/// [`crate::eval::resolve_column_index`] for how an unqualified
/// reference can still resolve against these when unambiguous.
///
/// **Scope, stated plainly:** only [`execute_select_with_functions`]
/// calls this. `GROUP BY`/aggregate and window `SELECT` lists don't
/// support `select.joins` being non-empty — a documented scope cut (see
/// each function's own guard), not a silently wrong combination.
fn scan_joined(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let left_qualifier = select
        .table_alias
        .clone()
        .unwrap_or_else(|| select.table_name.clone());
    let (left_cols, mut rows) = db.scan(&select.table_name, None)?;
    let mut columns: Vec<String> = left_cols
        .iter()
        .map(|c| format!("{left_qualifier}.{c}"))
        .collect();

    for join in &select.joins {
        let right_qualifier = join
            .table
            .alias
            .clone()
            .unwrap_or_else(|| join.table.name.clone());
        let (right_cols, right_rows) = db.scan(&join.table.name, None)?;
        let right_columns: Vec<String> = right_cols
            .iter()
            .map(|c| format!("{right_qualifier}.{c}"))
            .collect();

        let mut combined_columns = columns.clone();
        combined_columns.extend(right_columns.clone());

        // Resolved once per join, outside the row-pair loop below --
        // `USING (col, ...)` names are the same for every row pair.
        let using_pairs: Vec<(usize, usize)> = match &join.condition {
            JoinCondition::Using(names) => names
                .iter()
                .map(|name| {
                    let left_idx = resolve_column_index(&columns, name)?;
                    let right_idx = resolve_column_index(&right_columns, name)?;
                    Ok((left_idx, columns.len() + right_idx))
                })
                .collect::<Result<Vec<(usize, usize)>>>()?,
            _ => Vec::new(),
        };

        let right_len = right_cols.len();
        let mut new_rows = Vec::new();
        for left_row in &rows {
            let mut matched = false;
            for right_row in &right_rows {
                let mut combined = left_row.clone();
                combined.extend(right_row.iter().cloned());
                let keep = match &join.condition {
                    JoinCondition::None => true,
                    JoinCondition::Using(_) => using_pairs
                        .iter()
                        .all(|&(li, ri)| combined[li] == combined[ri]),
                    JoinCondition::On(cond) => {
                        evaluate_bool_with_functions(cond, &combined_columns, &combined, functions)?
                    }
                };
                if keep {
                    matched = true;
                    new_rows.push(combined);
                }
            }
            // LEFT JOIN: a left row with no matching right row still
            // appears once, right-side columns NULL-padded. INNER/CROSS
            // simply drop it (CROSS never reaches here unmatched unless
            // the right side has zero rows, in which case dropping is
            // exactly the correct empty-Cartesian-product result).
            if !matched && join.kind == JoinKind::Left {
                let mut combined = left_row.clone();
                combined.extend(std::iter::repeat_n(Value::Null, right_len));
                new_rows.push(combined);
            }
        }

        columns = combined_columns;
        rows = new_rows;
    }

    Ok((columns, rows))
}

/// Runs one `SELECT` core, dispatching to
/// [`execute_select_with_aggregates`]/[`execute_select_with_window`]/
/// [`execute_select_with_functions`] by `select.columns`'s kind — the
/// same dispatch [`crate::Connection::run_select`] does, reused here for
/// [`execute_compound_select`]'s free-function (no-`Connection`) form.
fn dispatch_select(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    match &select.columns {
        SelectColumns::Aggregates(_) => {
            execute_select_with_aggregates(db, select, functions, aggregates)
        }
        SelectColumns::Window(_) => execute_select_with_window(db, select, functions, aggregates),
        _ => execute_select_with_functions(db, select, functions),
    }
}

/// Executes a compound `SELECT` (`UNION`/`UNION ALL`/`INTERSECT`/
/// `EXCEPT` — issue #126): runs `compound.first`, then folds each
/// `compound.rest` entry into the running result left-associatively via
/// [`combine_rows`]. Each side runs through the exact same path a
/// standalone [`Select`] would ([`dispatch_select`]) — no new execution
/// model, per the issue's own scope note. A column-count mismatch
/// between any two combined sides is [`Error::ColumnCountMismatch`].
/// The output's column names come from `compound.first` alone (real
/// SQLite doesn't require — or use — the other sides' column names).
pub fn execute_compound_select(
    db: &Database,
    compound: &CompoundSelect,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let (column_names, mut rows) = dispatch_select(db, &compound.first, functions, aggregates)?;

    for (op, select) in &compound.rest {
        let (next_column_names, next_rows) = dispatch_select(db, select, functions, aggregates)?;
        if next_column_names.len() != column_names.len() {
            return Err(Error::ColumnCountMismatch {
                expected: column_names.len(),
                actual: next_column_names.len(),
            });
        }
        rows = combine_rows(*op, rows, next_rows);
    }

    Ok((column_names, rows))
}

/// Executes a `WITH` statement (issue #127), free-function counterpart
/// to [`crate::Connection::run_with_select`] — for API symmetry with
/// [`execute_compound_select`], for callers that already have a
/// [`Database`] rather than a [`crate::Connection`]. Materializes each
/// CTE's `SELECT` in declaration order, registering it via
/// [`Database::insert_cte`] before moving to the next (so a later CTE
/// can reference an earlier one), then runs `with_select.body` the same
/// way. CTEs are always cleared afterward — success or error.
pub fn execute_with_select(
    db: &Database,
    with_select: &WithSelect,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let result = (|| {
        for cte in &with_select.ctes {
            let (columns, rows) = execute_compound_select(db, &cte.select, functions, aggregates)?;
            db.insert_cte(cte.name.clone(), columns, rows);
        }
        execute_compound_select(db, &with_select.body, functions, aggregates)
    })();
    db.clear_ctes();
    result
}

/// Combines two already-executed sides' rows per `op` (issue #126) — a
/// pure `Vec` operation, no re-scanning or re-evaluation. `UNION`/
/// `INTERSECT`/`EXCEPT` (no `ALL`) all dedup their result, matching real
/// SQL's own implicitly-`DISTINCT` semantics for those three; `UNION
/// ALL` alone doesn't. `left`'s rows lead in the combined output, same
/// left-to-right order [`dedup_rows`] already preserves elsewhere in
/// this crate.
pub(crate) fn combine_rows(
    op: CompoundOp,
    mut left: Vec<Vec<Value>>,
    right: Vec<Vec<Value>>,
) -> Vec<Vec<Value>> {
    match op {
        CompoundOp::UnionAll => {
            left.extend(right);
            left
        }
        CompoundOp::Union => {
            left.extend(right);
            dedup_rows(true, left)
        }
        CompoundOp::Intersect => {
            let kept = left.into_iter().filter(|row| right.contains(row)).collect();
            dedup_rows(true, kept)
        }
        CompoundOp::Except => {
            let kept = left
                .into_iter()
                .filter(|row| !right.contains(row))
                .collect();
            dedup_rows(true, kept)
        }
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

/// Executes an aggregate `SELECT` (e.g. `SELECT COUNT(*), SUM(a) FROM
/// t`), folding every row matching `select.filter` through each
/// aggregate call's `step`. Bucketed by `select.group_by` (issue #125)
/// into one output row per distinct group — an empty `group_by` is one
/// implicit whole-table group (this crate's pre-#125 behavior:
/// ungrouped aggregation always produces exactly one row, even over zero
/// matching rows, e.g. `COUNT(*)` = 0; genuine grouping with zero
/// matching rows instead produces zero groups/rows, since there's
/// nothing to bucket). A `select.having` filter, if given, runs after
/// aggregation, once per group, evaluated against a synthetic row of
/// `select.group_by`'s key values followed by each aggregate call's
/// finalized value (named per [`describe_aggregate_call`], so e.g.
/// `HAVING COUNT(*) > 1` can reference the group's own `COUNT(*)`
/// result — `dml_select.rs`'s parser recognizes `IDENT(...)` inside
/// `HAVING` as an aggregate-call reference specifically to make this
/// possible). Errors if `select.columns` isn't
/// [`SelectColumns::Aggregates`].
///
/// **Scope, stated plainly (issue #125):** the `GROUP BY` column(s)
/// themselves aren't projected into the output row — see
/// [`SelectColumns::Aggregates`]'s own doc comment for why.
///
/// `select.distinct` (issue #116) dedups the final grouped rows — no
/// longer always a no-op now that `GROUP BY` can produce more than one
/// output row.
pub fn execute_select_with_aggregates(
    db: &Database,
    select: &Select,
    functions: &HashMap<String, Box<ScalarFn>>,
    aggregates: &HashMap<String, Aggregate>,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    // Scope cut (issue #130): an aggregate SELECT list combined with a
    // JOIN isn't supported yet -- errors clearly rather than silently
    // aggregating over just `select.table_name` and ignoring the joins.
    if !select.joins.is_empty() {
        return Err(Error::UnrecognizedStatement(
            "JOIN is not yet supported with an aggregate SELECT list".to_string(),
        ));
    }
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
    for call in calls {
        aggs.push(
            aggregates
                .get(&call.name)
                .ok_or_else(|| Error::FunctionNotFound(call.name.clone()))?,
        );
    }

    // (group key, one accumulator per aggregate call), in
    // first-occurrence order -- linear (not hashed) group lookup, same
    // `Value`-has-no-`Hash`/`Eq` constraint as `execute_select_with_
    // window`'s own partition lookup.
    let mut groups: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
    for row in &rows {
        let keep = match &select.filter {
            Some(filter) => evaluate_bool_with_functions(filter, &column_names, row, functions)?,
            None => true,
        };
        if !keep {
            continue;
        }
        let key = partition_key(&select.group_by, &column_names, row)?;
        let idx = match groups.iter().position(|(k, _)| *k == key) {
            Some(idx) => idx,
            None => {
                groups.push((key, aggs.iter().map(|a| a.init.clone()).collect()));
                groups.len() - 1
            }
        };
        for (i, call) in calls.iter().enumerate() {
            let arg_value = match &call.arg {
                AggregateArg::Star => Value::Integer(1),
                AggregateArg::Expr(expr) => {
                    evaluate_with_functions(expr, &column_names, row, functions)?
                }
            };
            groups[idx].1[i] = (aggs[i].step)(&groups[idx].1[i], &[arg_value])?;
        }
    }

    // Ungrouped aggregation over zero matching rows still produces one
    // row (e.g. `SELECT COUNT(*) FROM t WHERE 1 = 0` is `[0]`, not zero
    // rows) -- genuine `GROUP BY` with zero matching rows produces zero
    // groups instead, since there's nothing to bucket.
    if groups.is_empty() && select.group_by.is_empty() {
        groups.push((Vec::new(), aggs.iter().map(|a| a.init.clone()).collect()));
    }

    let having_column_names: Vec<String> = select
        .group_by
        .iter()
        .cloned()
        .chain(calls.iter().map(describe_aggregate_call))
        .collect();

    let mut result_rows = Vec::with_capacity(groups.len());
    for (key, accumulators) in groups {
        let result_row = aggs
            .iter()
            .zip(accumulators)
            .map(|(agg, acc)| (agg.finalize)(acc))
            .collect::<Result<Vec<Value>>>()?;

        if let Some(having) = &select.having {
            let having_row: Vec<Value> = key.into_iter().chain(result_row.clone()).collect();
            let keep =
                evaluate_bool_with_functions(having, &having_column_names, &having_row, functions)?;
            if !keep {
                continue;
            }
        }
        result_rows.push(result_row);
    }

    let column_names = calls.iter().map(describe_aggregate_call).collect();
    Ok((column_names, dedup_rows(select.distinct, result_rows)))
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
    // GROUP BY/HAVING (issue #125) is an aggregate-select-list-only
    // concept here -- window functions already have their own
    // per-partition grouping via PARTITION BY.
    if !select.group_by.is_empty() || select.having.is_some() {
        return Err(Error::UnrecognizedStatement(
            "GROUP BY/HAVING are not supported with a window SELECT list".to_string(),
        ));
    }
    // Scope cut (issue #130): a window SELECT list combined with a JOIN
    // isn't supported yet -- see execute_select_with_aggregates's own
    // identical guard.
    if !select.joins.is_empty() {
        return Err(Error::UnrecognizedStatement(
            "JOIN is not yet supported with a window SELECT list".to_string(),
        ));
    }
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
    use crate::{parse_compound_select, parse_create_table, parse_insert, parse_select, tokenize};

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
    fn insert_select_copies_rows_from_another_table() {
        let mut db = setup();
        let create =
            parse_create_table(&tokenize("CREATE TABLE u (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();

        let insert = parse_insert(&tokenize("INSERT INTO u SELECT * FROM t").unwrap()).unwrap();
        let affected = execute_insert(&mut db, &insert).unwrap();
        assert_eq!(affected, 3);
        assert_eq!(db.table("u").unwrap().rows.len(), 3);
    }

    #[test]
    fn insert_select_with_filter_and_projection() {
        let mut db = setup();
        let create = parse_create_table(&tokenize("CREATE TABLE u (b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();

        let insert =
            parse_insert(&tokenize("INSERT INTO u SELECT b FROM t WHERE a > 1").unwrap()).unwrap();
        let affected = execute_insert(&mut db, &insert).unwrap();
        assert_eq!(affected, 2);
        assert_eq!(
            db.table("u").unwrap().rows,
            vec![vec![Value::Text("y".into())], vec![Value::Text("z".into())]]
        );
    }

    #[test]
    fn insert_select_with_empty_result_inserts_zero_rows_without_erroring() {
        let mut db = setup();
        let create =
            parse_create_table(&tokenize("CREATE TABLE u (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();

        let insert =
            parse_insert(&tokenize("INSERT INTO u SELECT * FROM t WHERE a > 100").unwrap())
                .unwrap();
        let affected = execute_insert(&mut db, &insert).unwrap();
        assert_eq!(affected, 0);
        assert!(db.table("u").unwrap().rows.is_empty());
    }

    #[test]
    fn insert_select_with_explicit_column_list() {
        let mut db = setup();
        let create = parse_create_table(&tokenize("CREATE TABLE u (a INTEGER)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();

        let insert =
            parse_insert(&tokenize("INSERT INTO u (a) SELECT a FROM t WHERE a = 2").unwrap())
                .unwrap();
        execute_insert(&mut db, &insert).unwrap();
        assert_eq!(db.table("u").unwrap().rows, vec![vec![Value::Integer(2)]]);
    }

    #[test]
    fn insert_select_column_count_mismatch_errors_clearly() {
        let mut db = setup();
        let create = parse_create_table(&tokenize("CREATE TABLE u (a INTEGER)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();

        // t has two columns (a, b); u only has one.
        let insert = parse_insert(&tokenize("INSERT INTO u SELECT * FROM t").unwrap()).unwrap();
        assert!(matches!(
            execute_insert(&mut db, &insert),
            Err(Error::ColumnCountMismatch { .. })
        ));
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

    /// `t` (from [`setup`]: `a`/`b` = (1,'x'), (2,'y'), (3,'z')) plus a
    /// second table `u` for compound-`SELECT` (issue #126) tests.
    fn setup_compound() -> Database {
        let mut db = setup();
        let create =
            parse_create_table(&tokenize("CREATE TABLE u (a INTEGER, b TEXT)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        let insert =
            parse_insert(&tokenize("INSERT INTO u VALUES (2, 'y'), (4, 'w')").unwrap()).unwrap();
        execute_insert(&mut db, &insert).unwrap();
        db
    }

    fn run_compound(db: &Database, sql: &str) -> Vec<Vec<Value>> {
        let compound = parse_compound_select(&tokenize(sql).unwrap()).unwrap();
        execute_compound_select(db, &compound, &HashMap::new(), &HashMap::new())
            .unwrap()
            .1
    }

    #[test]
    fn union_combines_and_dedups_overlapping_rows() {
        let db = setup_compound();
        let mut rows = run_compound(&db, "SELECT a FROM t UNION SELECT a FROM u");
        rows.sort_by_key(|r| match r[0] {
            Value::Integer(n) => n,
            _ => unreachable!(),
        });
        assert_eq!(
            rows,
            vec![
                vec![Value::Integer(1)],
                vec![Value::Integer(2)],
                vec![Value::Integer(3)],
                vec![Value::Integer(4)],
            ]
        );
    }

    #[test]
    fn union_all_keeps_duplicates() {
        let db = setup_compound();
        let rows = run_compound(&db, "SELECT a FROM t UNION ALL SELECT a FROM u");
        // t has 3 rows, u has 2 -- UNION ALL never dedups, so 5 total,
        // including the (2) that both sides share.
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn intersect_keeps_only_rows_present_on_both_sides() {
        let db = setup_compound();
        let rows = run_compound(&db, "SELECT a FROM t INTERSECT SELECT a FROM u");
        assert_eq!(rows, vec![vec![Value::Integer(2)]]);
    }

    #[test]
    fn except_keeps_only_left_side_rows_absent_on_the_right() {
        let db = setup_compound();
        let mut rows = run_compound(&db, "SELECT a FROM t EXCEPT SELECT a FROM u");
        rows.sort_by_key(|r| match r[0] {
            Value::Integer(n) => n,
            _ => unreachable!(),
        });
        assert_eq!(rows, vec![vec![Value::Integer(1)], vec![Value::Integer(3)]]);
    }

    #[test]
    fn non_overlapping_union_keeps_every_row() {
        let mut db = Database::new();
        let create = parse_create_table(&tokenize("CREATE TABLE t (a INTEGER)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        execute_insert(
            &mut db,
            &parse_insert(&tokenize("INSERT INTO t VALUES (1)").unwrap()).unwrap(),
        )
        .unwrap();
        let create = parse_create_table(&tokenize("CREATE TABLE u (a INTEGER)").unwrap()).unwrap();
        execute_create_table(&mut db, &create).unwrap();
        execute_insert(
            &mut db,
            &parse_insert(&tokenize("INSERT INTO u VALUES (2)").unwrap()).unwrap(),
        )
        .unwrap();

        let mut rows = run_compound(&db, "SELECT a FROM t UNION SELECT a FROM u");
        rows.sort_by_key(|r| match r[0] {
            Value::Integer(n) => n,
            _ => unreachable!(),
        });
        assert_eq!(rows, vec![vec![Value::Integer(1)], vec![Value::Integer(2)]]);
    }

    #[test]
    fn compound_select_column_count_mismatch_errors_clearly() {
        let db = setup_compound();
        let compound =
            parse_compound_select(&tokenize("SELECT a, b FROM t UNION SELECT a FROM u").unwrap())
                .unwrap();
        assert!(matches!(
            execute_compound_select(&db, &compound, &HashMap::new(), &HashMap::new()),
            Err(Error::ColumnCountMismatch { .. })
        ));
    }
}
