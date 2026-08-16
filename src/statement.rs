//! `Statement`: a prepared, reusable SQL statement (Part B gap rows
//! "Statement: execution", "Statement: querying", "Statement: column
//! introspection", "Statement: parameter introspection", "Statement:
//! parameter binding", "Statement: diagnostics").
//!
//! **Parameter binding** (`?`/`?N`/`:name`/`@name`/`$name`) is real: see
//! `docs/adr/0002-parameter-markers.md` for the design.
//! [`Connection::prepare`] resolves every marker to a 1-based index
//! (SQLite's own numbering rule) once, at prepare time;
//! [`Statement::raw_bind_parameter`] stores a value against an index; and
//! [`Statement::execute`]/`query*` substitute bound values (or
//! `Value::Null` for an unbound index, matching real SQLite) into a
//! fully-concrete copy of the parsed statement before handing it to the
//! existing (parameter-oblivious) `engine`/`eval` functions — so those
//! functions' already-shipped signatures never needed to change; they
//! only gained one new `Expr::Parameter` match arm, unavoidable for any
//! new AST variant.
//!
//! **`columns`/`columns_with_metadata`/`column_metadata` aren't
//! provided:** in real `rusqlite`, all three are behind opt-in Cargo
//! features (`column_decltype`/`column_metadata`), not part of the
//! default API surface this crate targets. `column_metadata` in
//! particular returns a raw `&CStr`-tuple straight out of SQLite's C
//! API, which has no honest equivalent in a from-scratch engine with no
//! C interop. [`Statement::column_names`]/[`Statement::column_name`]/
//! [`Statement::column_index`]/[`Statement::column_count`] (all part of
//! the default surface) cover the rest of column introspection.
//!
//! **Also out of scope for now:** unlike [`crate::Connection::execute`],
//! [`Statement::execute`] doesn't fire `trace`/`profile`/`commit_hook`/
//! `update_hook`/the authorizer, or update `last_insert_rowid`/`changes`/
//! `total_changes`. Wiring a prepared statement into the same hook
//! machinery `execute` uses is real, valuable work — left for a
//! deliberate follow-up rather than folded into an already-large first
//! cut. [`Statement::execute`] does still respect
//! [`crate::OpenFlags::READ_ONLY`] and persist to a file-backed
//! connection (see [`crate::Connection::open`]), since those are
//! correctness guarantees, not observability.

use crate::connection::{is_with_select, leading_keyword, Connection};
use crate::ddl::{parse_create_table, CreateTable};
use crate::dml_delete::{parse_delete, Delete};
use crate::dml_insert::{parse_insert, Insert, InsertSource};
use crate::dml_select::{
    describe_aggregate_call, parse_compound_select, parse_param_marker, parse_with_select,
    AggregateArg, AggregateCall, CompoundSelect, Cte, Expr, ParamMarker, Select, SelectColumns,
    WithSelect,
};
use crate::dml_update::{parse_update, Update};
use crate::engine::{
    describe_window_call, execute_create_table, execute_delete, execute_insert_returning_rowids,
    execute_update,
};
use crate::error::{Error, Result};
use crate::row::Row;
use crate::rows::{AndThenRows, Rows};
use crate::token::tokenize;
use crate::tosql::ToSql;
use crate::value::Value;
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) enum StatementKind {
    CreateTable(CreateTable),
    Insert(Insert),
    Select(CompoundSelect),
    With(WithSelect),
    Update(Update),
    Delete(Delete),
}

/// Tokenizes and parses `sql`, resolving parameter markers to 1-based
/// indices the same way [`Statement::prepare`] always has. Split out from
/// [`Statement::prepare`] so [`Connection::prepare_cached`] can reuse it
/// on a cache miss without needing a `&mut Connection` yet — parsing
/// itself doesn't touch the connection at all.
pub(crate) fn parse_statement(sql: &str) -> Result<(StatementKind, Vec<Option<String>>)> {
    let tokens = tokenize(sql)?;
    let mut kind = match leading_keyword(&tokens) {
        Some(kw) if kw.eq_ignore_ascii_case("CREATE") => {
            StatementKind::CreateTable(parse_create_table(&tokens)?)
        }
        Some(kw) if kw.eq_ignore_ascii_case("INSERT") => {
            StatementKind::Insert(parse_insert(&tokens)?)
        }
        Some(kw) if kw.eq_ignore_ascii_case("SELECT") => {
            StatementKind::Select(parse_compound_select(&tokens)?)
        }
        Some(kw) if kw.eq_ignore_ascii_case("UPDATE") => {
            StatementKind::Update(parse_update(&tokens)?)
        }
        Some(kw) if kw.eq_ignore_ascii_case("DELETE") => {
            StatementKind::Delete(parse_delete(&tokens)?)
        }
        _ if is_with_select(&tokens) => StatementKind::With(parse_with_select(&tokens)?),
        _ => return Err(Error::UnrecognizedStatement(sql.to_string())),
    };

    // Left-to-right over the SQL text: select-list (aggregate args)
    // before `WHERE`, matching where each would appear in the
    // original text -- needed so index assignment agrees with
    // `expanded_sql`'s independent text-level scan.
    let mut resolver = ParamResolver::new();
    match &mut kind {
        StatementKind::CreateTable(_) => {}
        StatementKind::Insert(insert) => match &mut insert.source {
            InsertSource::Values(rows) => {
                for row in rows {
                    for expr in row {
                        resolver.rewrite(expr);
                    }
                }
            }
            InsertSource::Select(select) => rewrite_select_params(select, &mut resolver),
        },
        StatementKind::Select(compound) => {
            rewrite_select_params(&mut compound.first, &mut resolver);
            for (_, select) in &mut compound.rest {
                rewrite_select_params(select, &mut resolver);
            }
        }
        StatementKind::With(with_select) => {
            for cte in &mut with_select.ctes {
                rewrite_select_params(&mut cte.select.first, &mut resolver);
                for (_, select) in &mut cte.select.rest {
                    rewrite_select_params(select, &mut resolver);
                }
            }
            rewrite_select_params(&mut with_select.body.first, &mut resolver);
            for (_, select) in &mut with_select.body.rest {
                rewrite_select_params(select, &mut resolver);
            }
        }
        StatementKind::Update(update) => {
            // Left-to-right over the SQL text: `SET` assignments before
            // `WHERE`, matching where each appears in the original text.
            for (_, expr) in &mut update.assignments {
                resolver.rewrite(expr);
            }
            if let Some(filter) = &mut update.filter {
                resolver.rewrite(filter);
            }
        }
        StatementKind::Delete(delete) => {
            if let Some(filter) = &mut delete.filter {
                resolver.rewrite(filter);
            }
        }
    }

    Ok((kind, resolver.names))
}

/// Rewrites every `?`/`?N`/`:name`/`@name`/`$name` marker in `select`
/// (select-list aggregate/window args, then `WHERE`, left-to-right over
/// the SQL text) to a resolved 1-based index via `resolver` — shared by
/// a top-level `SELECT` and an `INSERT ... SELECT` source (issue #124),
/// since both need the exact same walk.
fn rewrite_select_params(select: &mut Select, resolver: &mut ParamResolver) {
    if let SelectColumns::Aggregates(calls) = &mut select.columns {
        for call in calls {
            if let AggregateArg::Expr(e) = &mut call.arg {
                resolver.rewrite(e);
            }
        }
    }
    if let SelectColumns::Window(calls) = &mut select.columns {
        for call in calls {
            if let AggregateArg::Expr(e) = &mut call.arg {
                resolver.rewrite(e);
            }
        }
    }
    if let Some(filter) = &mut select.filter {
        resolver.rewrite(filter);
    }
    if let Some(having) = &mut select.having {
        resolver.rewrite(having);
    }
}

/// Default capacity of a fresh [`Connection`]'s prepared-statement cache
/// (issue #106), matching real `rusqlite`'s own default.
pub(crate) const DEFAULT_STATEMENT_CACHE_CAPACITY: usize = 16;

/// A cached SQL string's already-tokenized-and-parsed form, keyed by exact
/// SQL text (byte-for-byte — no normalization, matching real `rusqlite`'s
/// cache key). Backs [`Connection::prepare_cached`].
///
/// **Design deviation, stated plainly:** real `rusqlite::Connection::
/// prepare_cached` returns a `CachedStatement` that, when dropped, returns
/// the *same* underlying prepared-statement object to the cache for reuse
/// — the cache holds live statement handles. This crate's cache instead
/// holds the parsed [`StatementKind`]/parameter-name list (cheap to
/// `Clone`) and hands back a fresh [`Statement`] built from a clone of it
/// on every call — no `Drop`-based return-to-cache dance, no shared
/// mutable statement object between calls. Achieves the same real
/// benefit (skip re-tokenizing/re-parsing the SQL text) through a
/// simpler mechanism; each returned `Statement` starts with fresh
/// (empty) bindings, same as [`Connection::prepare`].
pub(crate) struct StatementCache {
    capacity: usize,
    entries: HashMap<String, CachedPlan>,
    /// Recency order, least-recently-used first. Small (`capacity`-bounded)
    /// by construction, so a linear scan on touch/evict is fine — same
    /// "simplicity over micro-optimization" tradeoff this crate makes
    /// elsewhere (e.g. `Database`'s `HashMap`-backed tables).
    order: Vec<String>,
}

#[derive(Clone)]
struct CachedPlan {
    kind: StatementKind,
    param_names: Vec<Option<String>>,
}

impl StatementCache {
    pub(crate) fn new(capacity: usize) -> StatementCache {
        StatementCache {
            capacity,
            entries: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Changes the cache's capacity, evicting least-recently-used entries
    /// immediately if the new capacity is smaller than the current entry
    /// count. A capacity of `0` disables caching (every
    /// [`Connection::prepare_cached`] call re-parses).
    pub(crate) fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.evict_to_capacity();
    }

    /// Discards every cached entry. Capacity is unchanged.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    fn touch(&mut self, sql: &str) {
        if let Some(pos) = self.order.iter().position(|s| s == sql) {
            let key = self.order.remove(pos);
            self.order.push(key);
        }
    }

    fn evict_to_capacity(&mut self) {
        while self.order.len() > self.capacity {
            let oldest = self.order.remove(0);
            self.entries.remove(&oldest);
        }
    }

    /// Returns a clone of `sql`'s cached parsed form, marking it as
    /// most-recently-used. `None` on a cache miss.
    pub(crate) fn get(&mut self, sql: &str) -> Option<(StatementKind, Vec<Option<String>>)> {
        let plan = self.entries.get(sql)?.clone();
        self.touch(sql);
        Some((plan.kind, plan.param_names))
    }

    /// Caches `sql`'s already-parsed form. A no-op if the cache's capacity
    /// is `0`.
    pub(crate) fn insert(
        &mut self,
        sql: &str,
        kind: StatementKind,
        param_names: Vec<Option<String>>,
    ) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.contains_key(sql) {
            self.touch(sql);
        } else {
            self.order.push(sql.to_string());
        }
        self.entries
            .insert(sql.to_string(), CachedPlan { kind, param_names });
        self.evict_to_capacity();
    }
}

/// A prepared, reusable SQL statement, created via [`Connection::prepare`].
pub struct Statement<'conn> {
    conn: &'conn mut Connection,
    kind: StatementKind,
    sql: String,
    /// `param_names[i]` is index `i + 1`'s name, if it was a
    /// `:name`/`@name`/`$name` marker (`None` for `?`/`?N`). Computed
    /// once at prepare time — see `docs/adr/0002-parameter-markers.md`.
    param_names: Vec<Option<String>>,
    /// Values bound via [`Statement::raw_bind_parameter`], keyed by
    /// 1-based index.
    bindings: HashMap<usize, Value>,
    /// The most recent [`Statement::query`]/[`Statement::raw_query`]
    /// result set, kept alive on `self` so the [`Rows`] handed back can
    /// borrow from it instead of the query needing to return owned data.
    last_result: Option<(Vec<String>, Vec<Vec<Value>>)>,
}

impl<'conn> Statement<'conn> {
    pub(crate) fn prepare(conn: &'conn mut Connection, sql: &str) -> Result<Statement<'conn>> {
        let (kind, param_names) = parse_statement(sql)?;
        Ok(Statement::from_parsed(conn, sql, kind, param_names))
    }

    /// Builds a [`Statement`] from an already-parsed `kind`/`param_names`
    /// pair — either freshly produced by [`parse_statement`], or a clone
    /// handed back by [`StatementCache`] (issue #106,
    /// [`Connection::prepare_cached`]). Always starts with fresh (empty)
    /// bindings and no result set, same as [`Statement::prepare`].
    pub(crate) fn from_parsed(
        conn: &'conn mut Connection,
        sql: &str,
        kind: StatementKind,
        param_names: Vec<Option<String>>,
    ) -> Statement<'conn> {
        Statement {
            conn,
            kind,
            sql: sql.to_string(),
            param_names,
            bindings: HashMap::new(),
            last_result: None,
        }
    }

    /// Binds `value` to the parameter at `index` (1-based, matching
    /// SQLite's own convention — see [`Statement::parameter_index`] to
    /// look one up by name). Overwrites any previous binding for that
    /// index. Takes effect on the next [`Statement::execute`]/`query*`
    /// call.
    pub fn raw_bind_parameter<T: ToSql>(&mut self, index: usize, value: T) -> Result<()> {
        self.bindings.insert(index, value.to_sql());
        Ok(())
    }

    /// Clears every binding set via [`Statement::raw_bind_parameter`] —
    /// every parameter reverts to unbound (`Value::Null` when next
    /// executed/queried, matching real SQLite's unbound-parameter
    /// default).
    pub fn clear_bindings(&mut self) {
        self.bindings.clear();
    }

    /// Like [`Statement::raw_bind_parameter`], but resolves `index`
    /// through [`crate::BindIndex`] first — so a `&str` name works
    /// directly (`stmt.bind_parameter(":name", value)`) instead of
    /// needing a separate [`Statement::parameter_index`] lookup.
    pub fn bind_parameter<I: crate::params::BindIndex, T: ToSql>(
        &mut self,
        index: I,
        value: T,
    ) -> Result<()> {
        let idx = index.idx(self)?;
        self.raw_bind_parameter(idx, value)
    }

    /// Binds `params` (see [`crate::Params`]) into every `?`/`?N`
    /// position in order, then runs [`Statement::execute`]. The
    /// ergonomic counterpart to real `rusqlite::Statement::execute(params)`
    /// — kept as a new method rather than changing
    /// [`Statement::execute`]'s already-shipped no-argument signature.
    pub fn execute_with_params<P: crate::params::Params>(&mut self, params: P) -> Result<usize> {
        params.bind_all(self)?;
        self.execute()
    }

    /// Binds `params` (see [`crate::Params`]), then runs
    /// [`Statement::query_map`]. The ergonomic counterpart to real
    /// `rusqlite::Statement::query_map(params, f)`.
    pub fn query_map_with_params<P, T, F>(&mut self, params: P, f: F) -> Result<Vec<T>>
    where
        P: crate::params::Params,
        F: FnMut(Row<'_>) -> Result<T>,
    {
        params.bind_all(self)?;
        self.query_map(f)
    }

    /// Substitutes `expr`'s `Parameter` nodes with their bound value (or
    /// `Value::Null` if unbound) into a fully-concrete copy. Every
    /// `Parameter` here is already `ParamMarker::Numbered` — resolved by
    /// [`Statement::prepare`]'s [`ParamResolver`] pass.
    fn resolve_expr(&self, expr: &Expr) -> Expr {
        match expr {
            Expr::Parameter(ParamMarker::Numbered(idx)) => {
                Expr::Literal(self.bindings.get(idx).cloned().unwrap_or(Value::Null))
            }
            Expr::Parameter(_) => {
                unreachable!("Statement::prepare resolves every Parameter to Numbered")
            }
            Expr::BinaryOp { op, left, right } => Expr::BinaryOp {
                op: *op,
                left: Box::new(self.resolve_expr(left)),
                right: Box::new(self.resolve_expr(right)),
            },
            Expr::FunctionCall { name, args } => Expr::FunctionCall {
                name: name.clone(),
                args: args.iter().map(|a| self.resolve_expr(a)).collect(),
            },
            Expr::And(left, right) => Expr::And(
                Box::new(self.resolve_expr(left)),
                Box::new(self.resolve_expr(right)),
            ),
            Expr::Or(left, right) => Expr::Or(
                Box::new(self.resolve_expr(left)),
                Box::new(self.resolve_expr(right)),
            ),
            Expr::Not(inner) => Expr::Not(Box::new(self.resolve_expr(inner))),
            Expr::Like {
                left,
                pattern,
                escape,
                negate,
            } => Expr::Like {
                left: Box::new(self.resolve_expr(left)),
                pattern: Box::new(self.resolve_expr(pattern)),
                escape: escape.as_ref().map(|e| Box::new(self.resolve_expr(e))),
                negate: *negate,
            },
            Expr::Glob {
                left,
                pattern,
                negate,
            } => Expr::Glob {
                left: Box::new(self.resolve_expr(left)),
                pattern: Box::new(self.resolve_expr(pattern)),
                negate: *negate,
            },
            Expr::Between {
                expr,
                low,
                high,
                negate,
            } => Expr::Between {
                expr: Box::new(self.resolve_expr(expr)),
                low: Box::new(self.resolve_expr(low)),
                high: Box::new(self.resolve_expr(high)),
                negate: *negate,
            },
            Expr::InList { expr, list, negate } => Expr::InList {
                expr: Box::new(self.resolve_expr(expr)),
                list: list.iter().map(|e| self.resolve_expr(e)).collect(),
                negate: *negate,
            },
            Expr::Case {
                operand,
                branches,
                else_result,
            } => Expr::Case {
                operand: operand.as_ref().map(|o| Box::new(self.resolve_expr(o))),
                branches: branches
                    .iter()
                    .map(|(c, r)| (self.resolve_expr(c), self.resolve_expr(r)))
                    .collect(),
                else_result: else_result.as_ref().map(|e| Box::new(self.resolve_expr(e))),
            },
            Expr::Column(_) | Expr::Literal(_) => expr.clone(),
        }
    }

    fn resolved_insert(&self, insert: &Insert) -> Insert {
        let source = match &insert.source {
            InsertSource::Values(rows) => InsertSource::Values(
                rows.iter()
                    .map(|row| row.iter().map(|e| self.resolve_expr(e)).collect())
                    .collect(),
            ),
            InsertSource::Select(select) => InsertSource::Select(self.resolve_select(select)),
        };
        Insert {
            table_name: insert.table_name.clone(),
            columns: insert.columns.clone(),
            source,
            or_conflict: insert.or_conflict,
        }
    }

    /// Resolves every `Expr::Parameter` in an `UPDATE` statement's `SET`
    /// assignments and `WHERE` filter (issue #128) — the same
    /// bound-value-substitution [`Statement::resolved_insert`] does for
    /// `INSERT`.
    fn resolved_update(&self, update: &Update) -> Update {
        Update {
            table_name: update.table_name.clone(),
            assignments: update
                .assignments
                .iter()
                .map(|(name, expr)| (name.clone(), self.resolve_expr(expr)))
                .collect(),
            filter: update.filter.as_ref().map(|f| self.resolve_expr(f)),
        }
    }

    /// Resolves every `Expr::Parameter` in a `DELETE` statement's `WHERE`
    /// filter (issue #129) — the same bound-value-substitution
    /// [`Statement::resolved_update`] does for `UPDATE`.
    fn resolved_delete(&self, delete: &Delete) -> Delete {
        Delete {
            table_name: delete.table_name.clone(),
            filter: delete.filter.as_ref().map(|f| self.resolve_expr(f)),
        }
    }

    fn resolve_aggregate_arg(&self, arg: &AggregateArg) -> AggregateArg {
        match arg {
            AggregateArg::Star => AggregateArg::Star,
            AggregateArg::Expr(e) => AggregateArg::Expr(Box::new(self.resolve_expr(e))),
        }
    }

    /// Resolves every `Expr::Parameter` across this statement's
    /// (possibly compound — issue #126) `SELECT`: `compound.first`, then
    /// each `compound.rest` entry, via [`Statement::resolve_select`].
    fn resolved_compound_select(&self) -> Result<CompoundSelect> {
        let compound = self.select()?;
        Ok(self.resolve_compound(compound))
    }

    /// Resolves every `Expr::Parameter` across a `WITH` statement (issue
    /// #127): each CTE's (possibly compound) `SELECT`, in order, then the
    /// body — mirroring [`Statement::resolved_compound_select`].
    fn resolved_with_select(&self) -> Result<WithSelect> {
        match &self.kind {
            StatementKind::With(with_select) => Ok(WithSelect {
                ctes: with_select
                    .ctes
                    .iter()
                    .map(|cte| Cte {
                        name: cte.name.clone(),
                        select: self.resolve_compound(&cte.select),
                    })
                    .collect(),
                body: self.resolve_compound(&with_select.body),
            }),
            _ => Err(Error::UnrecognizedStatement(
                "query*() called on a non-SELECT statement -- use execute() instead".to_string(),
            )),
        }
    }

    fn resolve_compound(&self, compound: &CompoundSelect) -> CompoundSelect {
        CompoundSelect {
            first: self.resolve_select(&compound.first),
            rest: compound
                .rest
                .iter()
                .map(|(op, select)| (*op, self.resolve_select(select)))
                .collect(),
        }
    }

    /// Runs this statement's `SELECT` (plain, compound, or `WITH` — issue
    /// #127), dispatching to the right resolve+run pair based on
    /// [`StatementKind`]. Shared by every `query*`/`exists` method so
    /// none of them need their own `Select`-vs-`With` branch.
    fn run_query(&self) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
        match &self.kind {
            StatementKind::Select(_) => {
                let compound = self.resolved_compound_select()?;
                self.conn.run_compound_select(&compound)
            }
            StatementKind::With(_) => {
                let with_select = self.resolved_with_select()?;
                self.conn.run_with_select(&with_select)
            }
            _ => Err(Error::UnrecognizedStatement(
                "query*() called on a non-SELECT statement -- use execute() instead".to_string(),
            )),
        }
    }

    /// Resolves every `Expr::Parameter` in `select` (select-list
    /// aggregate/window args, then `WHERE`) to its bound value — the
    /// shared body [`Statement::resolved_select`] and [`Statement::
    /// resolved_insert`]'s `INSERT ... SELECT` case (issue #124) both
    /// need, since an `INSERT ... SELECT`'s nested `SELECT` isn't
    /// `self.select()`-reachable (that only returns `self.kind`'s own
    /// top-level `Select`).
    fn resolve_select(&self, select: &Select) -> Select {
        let columns = match &select.columns {
            SelectColumns::Aggregates(calls) => SelectColumns::Aggregates(
                calls
                    .iter()
                    .map(|c| AggregateCall {
                        name: c.name.clone(),
                        arg: self.resolve_aggregate_arg(&c.arg),
                    })
                    .collect(),
            ),
            SelectColumns::Window(calls) => SelectColumns::Window(
                calls
                    .iter()
                    .map(|c| crate::dml_select::WindowCall {
                        name: c.name.clone(),
                        arg: self.resolve_aggregate_arg(&c.arg),
                        partition_by: c.partition_by.clone(),
                    })
                    .collect(),
            ),
            other => other.clone(),
        };
        Select {
            columns,
            table_name: select.table_name.clone(),
            filter: select.filter.as_ref().map(|f| self.resolve_expr(f)),
            distinct: select.distinct,
            group_by: select.group_by.clone(),
            having: select.having.as_ref().map(|h| self.resolve_expr(h)),
        }
    }

    /// Runs this statement (`CREATE TABLE`/`INSERT`/`UPDATE`/`DELETE` —
    /// issues #128/#129), returning the number of rows affected (`0` for
    /// `CREATE TABLE`). Errors if this is a `SELECT` — use [`Statement::
    /// query_map`]/[`Statement::query_row`]/[`Statement::query_one`]
    /// instead.
    pub fn execute(&mut self) -> Result<usize> {
        if self.conn.is_readonly(crate::MAIN_DB)? {
            return Err(Error::ReadOnlyConnection);
        }
        let affected = match &self.kind {
            StatementKind::CreateTable(create) => {
                execute_create_table(self.conn.db_mut(), create)?;
                0
            }
            StatementKind::Insert(insert) => {
                let resolved = self.resolved_insert(insert);
                execute_insert_returning_rowids(self.conn.db_mut(), &resolved)?.len()
            }
            StatementKind::Update(update) => {
                let resolved = self.resolved_update(update);
                execute_update(self.conn.db_mut(), &resolved)?.len()
            }
            StatementKind::Delete(delete) => {
                let resolved = self.resolved_delete(delete);
                execute_delete(self.conn.db_mut(), &resolved)?.len()
            }
            StatementKind::Select(_) | StatementKind::With(_) => {
                return Err(Error::UnrecognizedStatement(
                    "execute() called on a SELECT statement -- use query*() instead".to_string(),
                ))
            }
        };
        self.conn.flush()?;
        Ok(affected)
    }

    fn select(&self) -> Result<&CompoundSelect> {
        match &self.kind {
            StatementKind::Select(select) => Ok(select),
            _ => Err(Error::UnrecognizedStatement(
                "query*() called on a non-SELECT statement -- use execute() instead".to_string(),
            )),
        }
    }

    /// The `Select` whose columns describe this statement's result set:
    /// a plain/compound `SELECT`'s own first branch, or (issue #127) a
    /// `WITH` statement's body's first branch — same rule real SQLite
    /// uses (a compound `SELECT`'s result columns always come from its
    /// first branch, `WITH`'s CTEs don't contribute any).
    fn result_select(&self) -> Result<&Select> {
        match &self.kind {
            StatementKind::Select(compound) => Ok(&compound.first),
            StatementKind::With(with_select) => Ok(&with_select.body.first),
            _ => Err(Error::UnrecognizedStatement(
                "query*() called on a non-SELECT statement -- use execute() instead".to_string(),
            )),
        }
    }

    /// Runs this `SELECT`, mapping every matching row through `f`.
    pub fn query_map<T, F>(&self, mut f: F) -> Result<Vec<T>>
    where
        F: FnMut(Row<'_>) -> Result<T>,
    {
        let (columns, rows) = self.run_query()?;
        rows.iter()
            .map(|values| f(Row::new(&columns, values)))
            .collect()
    }

    /// Runs this `SELECT`, expecting exactly one row, returning its
    /// values in result-column order. Errors with
    /// [`Error::QueryReturnedNoRows`] if no row matched.
    pub fn query_row(&self) -> Result<Vec<Value>> {
        let (_, mut rows) = self.run_query()?;
        if rows.is_empty() {
            return Err(Error::QueryReturnedNoRows);
        }
        Ok(rows.remove(0))
    }

    /// Like [`Statement::query_row`], but maps the single matching row
    /// through `f` instead of returning its raw values.
    pub fn query_one<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Row<'_>) -> Result<T>,
    {
        let (columns, mut rows) = self.run_query()?;
        if rows.is_empty() {
            return Err(Error::QueryReturnedNoRows);
        }
        let values = rows.remove(0);
        f(Row::new(&columns, &values))
    }

    /// Runs this `SELECT`, returning a lazy [`Rows`] iterator over the
    /// result set. Unlike [`Statement::query_map`] (which eagerly
    /// collects into a `Vec`), this is the same shape as real
    /// `rusqlite::Statement::query`.
    pub fn query(&mut self) -> Result<Rows<'_>> {
        let result = self.run_query()?;
        self.last_result = Some(result);
        let (columns, rows) = self.last_result.as_ref().expect("just assigned Some above");
        Ok(Rows::new(columns, rows))
    }

    /// Like [`Statement::query`], with each row mapped through a
    /// fallible-in-any-error-type closure — see [`AndThenRows`].
    pub fn query_and_then<T, E, F>(&mut self, f: F) -> Result<AndThenRows<'_, F>>
    where
        F: FnMut(Row<'_>) -> std::result::Result<T, E>,
        E: From<Error>,
    {
        Ok(self.query()?.and_then(f))
    }

    /// Runs this `SELECT`, returning whether it matched at least one row.
    pub fn exists(&self) -> Result<bool> {
        let (_, rows) = self.run_query()?;
        Ok(!rows.is_empty())
    }

    /// Like [`Statement::query`]. Real `rusqlite::Statement::raw_query`
    /// skips the higher-level `Params`-trait binding step `query`
    /// otherwise goes through; since [`Statement`] binds only through
    /// [`Statement::raw_bind_parameter`] (no `Params` trait yet — see
    /// issue #44), the two are identical here — kept as a separate
    /// method purely for name-level parity with call sites migrating
    /// from `rusqlite`.
    pub fn raw_query(&mut self) -> Result<Rows<'_>> {
        self.query()
    }

    /// This statement's result-column names, in order. Errors if this
    /// isn't a `SELECT`. For a compound `SELECT` (`UNION`/`INTERSECT`/
    /// `EXCEPT` — issue #126), these come from the first branch alone,
    /// same as real SQLite (the other branches' column names aren't
    /// required to match, and aren't used for anything if they don't).
    pub fn column_names(&self) -> Result<Vec<String>> {
        let first = self.result_select()?;
        match &first.columns {
            SelectColumns::All => Ok(self
                .conn
                .db()
                .table(&first.table_name)?
                .column_names
                .clone()),
            SelectColumns::Named(names) => Ok(names.clone()),
            SelectColumns::Aggregates(calls) => {
                Ok(calls.iter().map(describe_aggregate_call).collect())
            }
            SelectColumns::Window(calls) => Ok(calls.iter().map(describe_window_call).collect()),
        }
    }

    /// The number of columns in this statement's result set. Errors if
    /// this isn't a `SELECT`.
    pub fn column_count(&self) -> Result<usize> {
        Ok(self.column_names()?.len())
    }

    /// The name of the result column at `index`. Errors if this isn't a
    /// `SELECT`, or if `index` is out of range.
    pub fn column_name(&self, index: usize) -> Result<String> {
        let names = self.column_names()?;
        let len = names.len();
        names
            .into_iter()
            .nth(index)
            .ok_or(Error::IndexOutOfBounds { index, len })
    }

    /// The position of the result column named `name`. Errors if this
    /// isn't a `SELECT`, or if no result column has that name.
    pub fn column_index(&self, name: &str) -> Result<usize> {
        let names = self.column_names()?;
        names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error::UnknownColumn(name.to_string()))
    }

    /// Returns whether this statement is a `SELECT` (and so is run via
    /// [`Statement::query_map`]/[`Statement::query_row`]/
    /// [`Statement::query_one`] rather than [`Statement::execute`]).
    pub fn is_query(&self) -> bool {
        matches!(self.kind, StatementKind::Select(_) | StatementKind::With(_))
    }

    /// The number of `?`/`:name`-style parameters in this statement,
    /// per SQLite's own index-assignment rule (see
    /// `docs/adr/0002-parameter-markers.md`) — e.g. `WHERE a = ? AND b = ?`
    /// has 2, but `WHERE a = :x AND b = :x` has 1 (the repeated name
    /// reuses its first index).
    pub fn parameter_count(&self) -> usize {
        self.param_names.len()
    }

    /// The name of the parameter at `index` (1-based, matching SQLite's
    /// own convention), if it was a `:name`/`@name`/`$name` marker —
    /// `None` for a `?`/`?N` marker, or if `index` is out of range.
    pub fn parameter_name(&self, index: usize) -> Option<&str> {
        index
            .checked_sub(1)
            .and_then(|i| self.param_names.get(i))
            .and_then(|n| n.as_deref())
    }

    /// The index of the parameter named `name` (sigil included, e.g.
    /// `":foo"`), if this statement has one.
    pub fn parameter_index(&self, name: &str) -> Result<Option<usize>> {
        Ok(self
            .param_names
            .iter()
            .position(|n| n.as_deref() == Some(name))
            .map(|i| i + 1))
    }

    /// This statement's SQL text with every bound parameter substituted
    /// in as a literal (unbound parameters become `NULL`, matching real
    /// SQLite). A parameter-free statement's `expanded_sql` is just its
    /// original text.
    pub fn expanded_sql(&self) -> Option<String> {
        Some(substitute_params(&self.sql, &self.bindings))
    }

    /// Returns whether this statement can't modify the database — `true`
    /// for a `SELECT`, `false` for `CREATE TABLE`/`INSERT`/`UPDATE`/
    /// `DELETE`.
    pub fn readonly(&self) -> bool {
        self.is_query()
    }

    /// Returns whether this statement is `EXPLAIN`/`EXPLAIN QUERY PLAN`
    /// (`0` = neither, `1` = `EXPLAIN`, `2` = `EXPLAIN QUERY PLAN`,
    /// matching `sqlite3_stmt_isexplain`'s convention). Always `0` — this
    /// crate's parser doesn't recognize the `EXPLAIN` keyword at all yet,
    /// so no statement can be one.
    pub fn is_explain(&self) -> i32 {
        0
    }

    /// A per-statement execution counter, as SQLite's
    /// `sqlite3_stmt_status` would report. Always `0` — this crate's
    /// engine has no virtual machine (see `ARCHITECTURE.md`) to count
    /// fetch/sort/index/etc. operations for; stored as an honest `0`
    /// rather than omitted, matching the "not enforced, not silently
    /// dropped" treatment already given to `Connection::busy_timeout`.
    pub fn get_status(&self, _status: StatementStatus) -> i32 {
        0
    }

    /// The mirror of [`Statement::get_status`]: resets its counters. A
    /// no-op, for the same reason `get_status` always reports `0`.
    pub fn reset_status(&self) {}

    /// Finalizes this statement, consuming it. A no-op beyond dropping
    /// the guard — there's no separate C-level statement handle to
    /// release, so this exists purely for call-site parity with real
    /// `rusqlite`.
    pub fn finalize(self) -> Result<()> {
        Ok(())
    }
}

/// A counter [`Statement::get_status`] would report on, mirroring
/// SQLite's `SQLITE_STMTSTATUS_*` constants. Inert scaffolding today —
/// see [`Statement::get_status`]'s doc comment for why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementStatus {
    FullscanStep,
    Sort,
    AutoIndex,
    VmStep,
    RunExplainQueryPlan,
}

/// Assigns each [`ParamMarker`] encountered, in left-to-right occurrence
/// order, a 1-based index per SQLite's own rule — see
/// `docs/adr/0002-parameter-markers.md`. Used once by [`Statement::prepare`]
/// to rewrite the parsed tree's `Parameter` nodes to `ParamMarker::Numbered`,
/// and independently by [`substitute_params`] to walk `expanded_sql`'s raw
/// SQL text in the same order.
struct ParamResolver {
    next_auto: usize,
    named_index: HashMap<String, usize>,
    /// `names[i]` is index `i + 1`'s name, if any.
    names: Vec<Option<String>>,
}

impl ParamResolver {
    fn new() -> ParamResolver {
        ParamResolver {
            next_auto: 1,
            named_index: HashMap::new(),
            names: Vec::new(),
        }
    }

    fn ensure_len(&mut self, len: usize) {
        while self.names.len() < len {
            self.names.push(None);
        }
    }

    fn resolve(&mut self, marker: &ParamMarker) -> usize {
        match marker {
            ParamMarker::Anonymous => {
                let idx = self.next_auto;
                self.next_auto += 1;
                self.ensure_len(idx);
                idx
            }
            ParamMarker::Numbered(n) => {
                self.ensure_len(*n);
                if *n >= self.next_auto {
                    self.next_auto = n + 1;
                }
                *n
            }
            ParamMarker::Named(name) => {
                if let Some(&idx) = self.named_index.get(name) {
                    idx
                } else {
                    let idx = self.next_auto;
                    self.next_auto += 1;
                    self.ensure_len(idx);
                    self.names[idx - 1] = Some(name.clone());
                    self.named_index.insert(name.clone(), idx);
                    idx
                }
            }
        }
    }

    fn rewrite(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Parameter(marker) => {
                let idx = self.resolve(marker);
                *marker = ParamMarker::Numbered(idx);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.rewrite(left);
                self.rewrite(right);
            }
            Expr::FunctionCall { args, .. } => {
                for a in args {
                    self.rewrite(a);
                }
            }
            Expr::And(left, right) | Expr::Or(left, right) => {
                self.rewrite(left);
                self.rewrite(right);
            }
            Expr::Not(inner) => self.rewrite(inner),
            Expr::Like {
                left,
                pattern,
                escape,
                ..
            } => {
                self.rewrite(left);
                self.rewrite(pattern);
                if let Some(e) = escape {
                    self.rewrite(e);
                }
            }
            Expr::Glob { left, pattern, .. } => {
                self.rewrite(left);
                self.rewrite(pattern);
            }
            Expr::Between {
                expr, low, high, ..
            } => {
                self.rewrite(expr);
                self.rewrite(low);
                self.rewrite(high);
            }
            Expr::InList { expr, list, .. } => {
                self.rewrite(expr);
                for item in list {
                    self.rewrite(item);
                }
            }
            Expr::Case {
                operand,
                branches,
                else_result,
            } => {
                if let Some(o) = operand {
                    self.rewrite(o);
                }
                for (c, r) in branches {
                    self.rewrite(c);
                    self.rewrite(r);
                }
                if let Some(e) = else_result {
                    self.rewrite(e);
                }
            }
            Expr::Column(_) | Expr::Literal(_) => {}
        }
    }
}

/// Renders `value` as it would appear as a SQL literal (used by
/// [`substitute_params`]).
fn value_to_sql_literal(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Blob(b) => {
            let hex: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("X'{hex}'")
        }
    }
}

/// A minimal, string-literal-aware re-scan of `sql`, replacing each
/// `?`/`?N`/`:name`/`@name`/`$name` marker with its bound value's SQL
/// literal text (or `NULL` if unbound) — powers [`Statement::expanded_sql`].
/// Independent of (but index-assignment-consistent with) the AST-level
/// [`ParamResolver`] pass `Statement::prepare` already ran, since
/// `expanded_sql` only has the original text to work from, not the parsed
/// tree.
fn substitute_params(sql: &str, bindings: &HashMap<usize, Value>) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::new();
    let mut resolver = ParamResolver::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        if c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.extend(&chars[start..i]);
            continue;
        }

        if c == '?' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let spec: String = chars[start + 1..i].iter().collect();
            let idx = resolver.resolve(&parse_param_marker(&spec));
            out.push_str(
                &bindings
                    .get(&idx)
                    .map(value_to_sql_literal)
                    .unwrap_or_else(|| "NULL".to_string()),
            );
            continue;
        }

        if c == ':' || c == '@' || c == '$' {
            let start = i;
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j > start + 1 {
                let spec: String = chars[start..j].iter().collect();
                let idx = resolver.resolve(&parse_param_marker(&spec));
                out.push_str(
                    &bindings
                        .get(&idx)
                        .map(value_to_sql_literal)
                        .unwrap_or_else(|| "NULL".to_string()),
                );
                i = j;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_and_executes_create_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        let mut stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(!stmt.is_query());
        assert_eq!(stmt.execute().unwrap(), 0);
        assert!(conn.table_exists("t"));
    }

    #[test]
    fn prepared_insert_is_reusable_across_multiple_executes() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(stmt.execute().unwrap(), 1);
        assert_eq!(stmt.execute().unwrap(), 1);
        assert_eq!(stmt.execute().unwrap(), 1);

        let values: Vec<i64> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![1, 1, 1]);
    }

    #[test]
    fn execute_on_select_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert!(stmt.execute().is_err());
    }

    #[test]
    fn query_map_on_non_select_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        let stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(stmt.query_map(|row: Row<'_>| row.get::<i64>(0)).is_err());
    }

    #[test]
    fn query_map_runs_the_prepared_select() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let stmt = conn.prepare("SELECT * FROM t WHERE a = 2").unwrap();
        let values: Vec<i64> = stmt.query_map(|row| row.get(0)).unwrap();
        assert_eq!(values, vec![2]);
    }

    #[test]
    fn query_row_and_query_one_work() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (7)").unwrap();

        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.query_row().unwrap(), vec![Value::Integer(7)]);

        let doubled: i64 = stmt
            .query_one(|row| row.get::<i64>(0).map(|n| n * 2))
            .unwrap();
        assert_eq!(doubled, 14);
    }

    #[test]
    fn query_row_with_no_matches_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.query_row(), Err(Error::QueryReturnedNoRows));
    }

    #[test]
    fn column_names_for_select_star() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.column_names().unwrap(), vec!["a", "b"]);
        assert_eq!(stmt.column_count().unwrap(), 2);
        assert_eq!(stmt.column_name(1).unwrap(), "b");
    }

    #[test]
    fn column_names_for_named_columns() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn.prepare("SELECT b FROM t").unwrap();
        assert_eq!(stmt.column_names().unwrap(), vec!["b"]);
    }

    #[test]
    fn column_names_for_aggregate_select() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT COUNT(*), SUM(a) FROM t").unwrap();
        assert_eq!(stmt.column_names().unwrap(), vec!["COUNT(*)", "SUM(a)"]);
    }

    #[test]
    fn column_names_for_window_select() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (grp TEXT, a INTEGER)")
            .unwrap();
        let stmt = conn
            .prepare("SELECT SUM(a) OVER (PARTITION BY grp), COUNT(*) OVER () FROM t")
            .unwrap();
        assert_eq!(
            stmt.column_names().unwrap(),
            vec!["SUM(a) OVER (PARTITION BY grp)", "COUNT(*) OVER ()"]
        );
    }

    #[test]
    fn bound_parameter_usable_inside_a_window_call_argument() {
        // This crate's expression grammar has no infix arithmetic (`+`),
        // so the bound parameter is the window call's whole argument
        // rather than part of a larger expression like `a + ?`.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (grp TEXT, a INTEGER)")
            .unwrap();
        conn.execute("INSERT INTO t VALUES ('x', 1), ('x', 2)")
            .unwrap();

        let mut stmt = conn
            .prepare("SELECT SUM(?) OVER (PARTITION BY grp) FROM t")
            .unwrap();
        stmt.raw_bind_parameter(1, 10i64).unwrap();
        let sums: Vec<i64> = stmt.query_map(|row| row.get(0)).unwrap();
        assert_eq!(sums, vec![20, 20]);
    }

    #[test]
    fn column_name_out_of_range_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(
            stmt.column_name(5),
            Err(Error::IndexOutOfBounds { index: 5, len: 1 })
        );
    }

    #[test]
    fn column_names_on_non_select_is_an_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        let stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(stmt.column_names().is_err());
    }

    #[test]
    fn execute_on_read_only_connection_is_an_error() {
        let mut conn = Connection::open_in_memory_with_flags(crate::OpenFlags::READ_ONLY).unwrap();
        assert!(conn.prepare("CREATE TABLE t (a INTEGER)").is_ok());
        let mut stmt = conn.prepare("CREATE TABLE t (a INTEGER)").unwrap();
        assert_eq!(stmt.execute(), Err(Error::ReadOnlyConnection));
    }

    #[test]
    fn query_returns_a_lazy_rows_iterator() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        let values: Result<Vec<i64>> = stmt
            .query()
            .unwrap()
            .map(|r| r.and_then(|row| row.get::<i64>(0)))
            .collect();
        assert_eq!(values.unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn raw_query_behaves_like_query() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (5)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        let values: Result<Vec<i64>> = stmt
            .raw_query()
            .unwrap()
            .map(|r| r.and_then(|row| row.get::<i64>(0)))
            .collect();
        assert_eq!(values.unwrap(), vec![5]);
    }

    #[test]
    fn query_and_then_propagates_custom_errors() {
        #[derive(Debug, PartialEq)]
        enum MyError {
            Inner(Error),
            TooBig,
        }
        impl From<Error> for MyError {
            fn from(e: Error) -> MyError {
                MyError::Inner(e)
            }
        }

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (5)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t").unwrap();
        let result: std::result::Result<Vec<i64>, MyError> = stmt
            .query_and_then(|row| {
                let n = row.get::<i64>(0)?;
                if n > 3 {
                    Err(MyError::TooBig)
                } else {
                    Ok(n)
                }
            })
            .unwrap()
            .collect();
        assert_eq!(result, Err(MyError::TooBig));
    }

    #[test]
    fn exists_reflects_whether_any_row_matched() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1)").unwrap();

        assert!(conn
            .prepare("SELECT * FROM t WHERE a = 1")
            .unwrap()
            .exists()
            .unwrap());
        assert!(!conn
            .prepare("SELECT * FROM t WHERE a = 2")
            .unwrap()
            .exists()
            .unwrap());
    }

    #[test]
    fn column_index_finds_a_named_column() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.column_index("b").unwrap(), 1);
        assert!(stmt.column_index("missing").is_err());
    }

    #[test]
    fn readonly_distinguishes_select_from_mutating_statements() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let select = conn.prepare("SELECT * FROM t").unwrap();
        assert!(select.readonly());

        let create = conn.prepare("CREATE TABLE t2 (a INTEGER)").unwrap();
        assert!(!create.readonly());

        let insert = conn.prepare("INSERT INTO t VALUES (1)").unwrap();
        assert!(!insert.readonly());
    }

    #[test]
    fn is_explain_is_always_false() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.is_explain(), 0);
    }

    #[test]
    fn status_is_inert() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert_eq!(stmt.get_status(StatementStatus::FullscanStep), 0);
        stmt.reset_status();
        assert_eq!(stmt.get_status(StatementStatus::Sort), 0);
    }

    #[test]
    fn finalize_consumes_the_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t").unwrap();
        assert!(stmt.finalize().is_ok());
    }

    #[test]
    fn parameter_count_and_name_for_anonymous_and_named_markers() {
        // This crate's `WHERE` grammar is a single comparison -- no
        // `AND`/`OR` combining yet (a pre-existing limitation, unrelated
        // to parameter binding) -- so both markers are placed inside one
        // function call's argument list instead.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let stmt = conn
            .prepare("SELECT * FROM t WHERE F(?, :name) = 1")
            .unwrap();
        assert_eq!(stmt.parameter_count(), 2);
        assert_eq!(stmt.parameter_name(1), None);
        assert_eq!(stmt.parameter_name(2), Some(":name"));
        assert_eq!(stmt.parameter_index(":name").unwrap(), Some(2));
        assert_eq!(stmt.parameter_index(":missing").unwrap(), None);
    }

    #[test]
    fn repeated_named_parameter_reuses_one_index() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b INTEGER)")
            .unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE F(:x, :x) = 1").unwrap();
        assert_eq!(stmt.parameter_count(), 1);
        assert_eq!(stmt.parameter_name(1), Some(":x"));
    }

    #[test]
    fn numbered_parameter_bumps_the_auto_counter() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b INTEGER, c INTEGER)")
            .unwrap();
        // ?2 claims index 2; the next bare `?` should become index 3, not 2.
        let stmt = conn
            .prepare("SELECT * FROM t WHERE F(?2, ?, ?1) = 1")
            .unwrap();
        assert_eq!(stmt.parameter_count(), 3);
    }

    #[test]
    fn raw_bind_parameter_and_execute_insert_with_anonymous_markers() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?)").unwrap();
        stmt.raw_bind_parameter(1, 42i64).unwrap();
        stmt.raw_bind_parameter(2, "hi").unwrap();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(42), Value::Text("hi".into())]);
    }

    #[test]
    fn unbound_parameter_defaults_to_null() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?)").unwrap();
        stmt.raw_bind_parameter(1, 1i64).unwrap();
        // Index 2 left unbound on purpose.
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Null]);
    }

    #[test]
    fn clear_bindings_reverts_to_unbound() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (?)").unwrap();
        stmt.raw_bind_parameter(1, 9i64).unwrap();
        stmt.clear_bindings();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Null]);
    }

    #[test]
    fn rebinding_an_index_overwrites_the_previous_value() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (?)").unwrap();
        stmt.raw_bind_parameter(1, 1i64).unwrap();
        stmt.raw_bind_parameter(1, 2i64).unwrap();
        stmt.execute().unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(2)]);
    }

    #[test]
    fn bound_parameter_usable_in_where_clause() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t WHERE a = ?").unwrap();
        stmt.raw_bind_parameter(1, 2i64).unwrap();
        let values: Vec<i64> = stmt.query_map(|row| row.get(0)).unwrap();
        assert_eq!(values, vec![2]);
    }

    #[test]
    fn bind_parameter_resolves_a_name_directly() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t WHERE a = :x").unwrap();
        stmt.bind_parameter(":x", 2i64).unwrap();
        assert_eq!(stmt.query_map(|row| row.get::<i64>(0)).unwrap(), vec![2]);
    }

    #[test]
    fn execute_with_params_binds_and_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();

        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?)").unwrap();
        stmt.execute_with_params((1i64, "x")).unwrap();

        let row = conn.query_row("SELECT * FROM t").unwrap();
        assert_eq!(row, vec![Value::Integer(1), Value::Text("x".into())]);
    }

    #[test]
    fn query_map_with_params_binds_and_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t WHERE a = ?").unwrap();
        let values: Vec<i64> = stmt
            .query_map_with_params((2i64,), |row| row.get(0))
            .unwrap();
        assert_eq!(values, vec![2]);
    }

    #[test]
    fn named_parameter_usable_in_where_clause() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t WHERE a = :target").unwrap();
        let idx = stmt.parameter_index(":target").unwrap().unwrap();
        stmt.raw_bind_parameter(idx, 3i64).unwrap();
        let values: Vec<i64> = stmt.query_map(|row| row.get(0)).unwrap();
        assert_eq!(values, vec![3]);
    }

    #[test]
    fn rebinding_and_reexecuting_a_where_clause_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        let mut stmt = conn.prepare("SELECT * FROM t WHERE a = ?").unwrap();
        stmt.raw_bind_parameter(1, 1i64).unwrap();
        assert_eq!(stmt.query_map(|row| row.get::<i64>(0)).unwrap(), vec![1]);

        stmt.raw_bind_parameter(1, 2i64).unwrap();
        assert_eq!(stmt.query_map(|row| row.get::<i64>(0)).unwrap(), vec![2]);
    }

    #[test]
    fn expanded_sql_with_no_bindings_is_the_original_text() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE a = 1").unwrap();
        assert_eq!(
            stmt.expanded_sql(),
            Some("SELECT * FROM t WHERE a = 1".to_string())
        );
    }

    #[test]
    fn expanded_sql_substitutes_bound_values() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER, b TEXT)").unwrap();
        let mut stmt = conn
            .prepare("SELECT * FROM t WHERE a = ? AND b = :name")
            .unwrap();
        stmt.raw_bind_parameter(1, 5i64).unwrap();
        stmt.raw_bind_parameter(2, "it's").unwrap();
        assert_eq!(
            stmt.expanded_sql(),
            Some("SELECT * FROM t WHERE a = 5 AND b = 'it''s'".to_string())
        );
    }

    #[test]
    fn expanded_sql_substitutes_unbound_as_null() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE a = ?").unwrap();
        assert_eq!(
            stmt.expanded_sql(),
            Some("SELECT * FROM t WHERE a = NULL".to_string())
        );
    }

    #[test]
    fn parameter_inside_string_literal_is_not_mistaken_for_a_marker() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a TEXT)").unwrap();
        let stmt = conn.prepare("SELECT * FROM t WHERE a = '?'").unwrap();
        assert_eq!(stmt.parameter_count(), 0);
        assert_eq!(
            stmt.expanded_sql(),
            Some("SELECT * FROM t WHERE a = '?'".to_string())
        );
    }

    #[test]
    fn statement_cache_hit_returns_an_equivalent_parsed_form() {
        let mut cache = StatementCache::new(4);
        let (kind, names) = parse_statement("SELECT * FROM t WHERE a = ?").unwrap();
        cache.insert("SELECT * FROM t WHERE a = ?", kind, names.clone());

        let (_, cached_names) = cache.get("SELECT * FROM t WHERE a = ?").unwrap();
        assert_eq!(cached_names, names);
    }

    #[test]
    fn statement_cache_miss_returns_none() {
        let mut cache = StatementCache::new(4);
        assert!(cache.get("SELECT 1").is_none());
    }

    #[test]
    fn statement_cache_evicts_least_recently_used_entry_over_capacity() {
        let mut cache = StatementCache::new(2);
        for sql in ["CREATE TABLE a (x INTEGER)", "CREATE TABLE b (x INTEGER)"] {
            let (kind, names) = parse_statement(sql).unwrap();
            cache.insert(sql, kind, names);
        }
        // Touch "a" so "b" becomes the least-recently-used entry.
        assert!(cache.get("CREATE TABLE a (x INTEGER)").is_some());

        let (kind, names) = parse_statement("CREATE TABLE c (x INTEGER)").unwrap();
        cache.insert("CREATE TABLE c (x INTEGER)", kind, names);

        assert!(cache.get("CREATE TABLE a (x INTEGER)").is_some());
        assert!(cache.get("CREATE TABLE b (x INTEGER)").is_none());
        assert!(cache.get("CREATE TABLE c (x INTEGER)").is_some());
    }

    #[test]
    fn statement_cache_capacity_zero_never_caches() {
        let mut cache = StatementCache::new(0);
        let sql = "CREATE TABLE t (a INTEGER)";
        let (kind, names) = parse_statement(sql).unwrap();
        cache.insert(sql, kind, names);
        assert!(cache.get(sql).is_none());
    }

    #[test]
    fn statement_cache_shrinking_capacity_evicts_immediately() {
        let mut cache = StatementCache::new(4);
        for sql in [
            "CREATE TABLE a (x INTEGER)",
            "CREATE TABLE b (x INTEGER)",
            "CREATE TABLE c (x INTEGER)",
        ] {
            let (kind, names) = parse_statement(sql).unwrap();
            cache.insert(sql, kind, names);
        }
        cache.set_capacity(1);
        assert!(cache.get("CREATE TABLE a (x INTEGER)").is_none());
        assert!(cache.get("CREATE TABLE b (x INTEGER)").is_none());
        assert!(cache.get("CREATE TABLE c (x INTEGER)").is_some());
    }

    #[test]
    fn prepare_cached_reuses_a_previously_parsed_statement() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.execute("INSERT INTO t VALUES (1), (2)").unwrap();

        {
            let stmt = conn.prepare_cached("SELECT * FROM t").unwrap();
            let values: Vec<i64> = stmt.query_map(|row: Row<'_>| row.get(0)).unwrap();
            assert_eq!(values, vec![1, 2]);
        }
        // Second call is a cache hit on the same SQL text.
        {
            let stmt = conn.prepare_cached("SELECT * FROM t").unwrap();
            let values: Vec<i64> = stmt.query_map(|row: Row<'_>| row.get(0)).unwrap();
            assert_eq!(values, vec![1, 2]);
        }
    }

    #[test]
    fn prepare_cached_starts_with_fresh_bindings_on_a_cache_hit() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();

        {
            let mut stmt = conn.prepare_cached("INSERT INTO t VALUES (?)").unwrap();
            stmt.raw_bind_parameter(1, 1i64).unwrap();
            assert_eq!(stmt.execute().unwrap(), 1);
        }
        // A fresh prepare_cached call must not inherit the previous
        // call's binding — an unbound `?` reports NULL, same as a brand
        // new `prepare`.
        {
            let mut stmt = conn.prepare_cached("INSERT INTO t VALUES (?)").unwrap();
            assert_eq!(stmt.execute().unwrap(), 1);
        }

        let values: Vec<Option<i64>> = conn.query_map("SELECT * FROM t", |row| row.get(0)).unwrap();
        assert_eq!(values, vec![Some(1), None]);
    }

    #[test]
    fn prepare_cached_propagates_a_parse_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(conn.prepare_cached("NOT VALID SQL").is_err());
    }

    #[test]
    fn set_prepared_statement_cache_capacity_zero_disables_caching_end_to_end() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        conn.set_prepared_statement_cache_capacity(0);

        // Still works with caching disabled -- every call just re-parses.
        assert!(conn.prepare_cached("SELECT * FROM t").is_ok());
        assert!(conn.prepare_cached("SELECT * FROM t").is_ok());
    }

    #[test]
    fn flush_prepared_statement_cache_clears_cached_entries() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (a INTEGER)").unwrap();
        assert!(conn.prepare_cached("SELECT * FROM t").is_ok());

        conn.flush_prepared_statement_cache();

        // Still works after a flush -- just re-parses and re-caches.
        assert!(conn.prepare_cached("SELECT * FROM t").is_ok());
    }
}
