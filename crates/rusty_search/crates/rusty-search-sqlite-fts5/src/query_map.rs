use std::collections::HashMap;

use rusty_search_core::{
    FieldType as CoreFieldType, Query as CoreQuery, SearchError, SearchRequest, Sort, SortOrder,
};
use rusty_sqlite::rusqlite::types::Value as SqlValue;

use crate::convert;
use crate::schema_map::{quote_ident, FieldMeta};

/// A `Query` tree translated into a SQL boolean expression against the
/// `content` table, plus (if the tree contained a `Query::Match`) the FTS5
/// `MATCH` expression needed to compute relevance scores for it.
struct Plan {
    predicate: String,
    params: Vec<SqlValue>,
    match_query: Option<String>,
}

/// A fully compiled search: the `SELECT`/`COUNT` SQL text and the
/// positional parameters each needs, ready to bind and execute.
pub struct CompiledSearch {
    pub select_sql: String,
    pub select_params: Vec<SqlValue>,
    pub count_sql: String,
    pub count_params: Vec<SqlValue>,
}

/// Compiles a [`SearchRequest`] against `fields` into SQL.
///
/// At most one `Query::Match` clause is supported per request - like
/// `rusty-search-meilisearch` (see ADR-0003), scoring a query tree with more
/// than one full-text clause well requires combining multiple independent
/// relevance signals, which this backend doesn't attempt; a second
/// `Query::Match` anywhere in the tree is rejected with
/// [`SearchError::InvalidQuery`] rather than approximated. Everything else -
/// arbitrary `Term`/`Range` combinations, and `must_not` wrapping a bare
/// `MatchAll`/`Match` (which trips up `rusty-search-meilisearch`/
/// `rusty-search-algolia`) - translates directly into SQL without
/// restriction, since a plain `NOT (...)` is always well-formed here.
pub fn compile(
    request: &SearchRequest,
    fields: &HashMap<String, FieldMeta>,
) -> Result<CompiledSearch, SearchError> {
    let plan = build_plan(&request.query, fields)?;

    let join_clause = if plan.match_query.is_some() {
        "LEFT JOIN (SELECT rowid, bm25(idx_fts) AS bm25 FROM idx_fts WHERE idx_fts MATCH ?) AS scores ON scores.rowid = content.rowid"
    } else {
        ""
    };
    let score_expr = if plan.match_query.is_some() {
        "COALESCE(-scores.bm25, 0.0)"
    } else {
        "0.0"
    };

    let mut base_params = Vec::new();
    if let Some(match_query) = &plan.match_query {
        base_params.push(SqlValue::Text(match_query.clone()));
    }
    base_params.extend(plan.params.iter().cloned());

    let order_by = build_order_by(&request.sort, fields)?;

    let select_sql = format!(
        "SELECT content.*, {score_expr} AS score FROM content {join_clause} WHERE {} ORDER BY {order_by} LIMIT ? OFFSET ?",
        plan.predicate,
    );
    let mut select_params = base_params.clone();
    select_params.push(SqlValue::Integer(request.limit as i64));
    select_params.push(SqlValue::Integer(request.offset as i64));

    let count_sql = format!(
        "SELECT COUNT(*) FROM content {join_clause} WHERE {}",
        plan.predicate,
    );

    Ok(CompiledSearch {
        select_sql,
        select_params,
        count_sql,
        count_params: base_params,
    })
}

fn build_plan(query: &CoreQuery, fields: &HashMap<String, FieldMeta>) -> Result<Plan, SearchError> {
    let mut params = Vec::new();
    let mut match_query = None;
    let predicate = build_predicate(query, fields, &mut params, &mut match_query)?;
    Ok(Plan {
        predicate,
        params,
        match_query,
    })
}

fn build_predicate(
    query: &CoreQuery,
    fields: &HashMap<String, FieldMeta>,
    params: &mut Vec<SqlValue>,
    match_query: &mut Option<String>,
) -> Result<String, SearchError> {
    match query {
        CoreQuery::MatchAll => Ok("1".to_string()),

        CoreQuery::Term { field, value } => {
            let meta = lookup(fields, field)?;
            params.push(convert::value_to_sql(meta.field_type, value)?);
            Ok(format!("{} = ?", quote_ident(field)))
        }

        CoreQuery::Range { field, gte, lte } => {
            let meta = lookup(fields, field)?;
            let mut parts = Vec::new();
            if let Some(v) = gte {
                params.push(convert::json_value_to_sql(meta.field_type, v)?);
                parts.push(format!("{} >= ?", quote_ident(field)));
            }
            if let Some(v) = lte {
                params.push(convert::json_value_to_sql(meta.field_type, v)?);
                parts.push(format!("{} <= ?", quote_ident(field)));
            }
            if parts.is_empty() {
                Ok("1".to_string())
            } else {
                Ok(format!("({})", parts.join(" AND ")))
            }
        }

        CoreQuery::Match { field, value } => {
            let meta = lookup(fields, field)?;
            if meta.field_type != CoreFieldType::Text || !meta.fts_indexed {
                return Err(SearchError::InvalidQuery(format!(
                    "field `{field}` is not a full-text-indexed Text field"
                )));
            }
            if match_query.is_some() {
                return Err(SearchError::InvalidQuery(
                    "rusty-search-sqlite-fts5 supports at most one Query::Match clause per search"
                        .to_string(),
                ));
            }
            *match_query = Some(fts_match_expr(field, value));
            Ok("scores.rowid IS NOT NULL".to_string())
        }

        CoreQuery::Bool {
            must,
            should,
            must_not,
            filter,
        } => {
            let mut required = Vec::new();
            for q in must.iter().chain(filter.iter()) {
                required.push(build_predicate(q, fields, params, match_query)?);
            }
            for q in must_not {
                let frag = build_predicate(q, fields, params, match_query)?;
                required.push(format!("NOT ({frag})"));
            }
            // Matches Query's documented semantics: `should` only filters
            // when it's the only clause type present at this level: with
            // any `must`/`filter` sibling, `should` is optional and dropped
            // here entirely (it would otherwise need its own,
            // non-filtering scoring contribution this backend's bm25-only
            // scoring doesn't attempt).
            if must.is_empty() && filter.is_empty() && !should.is_empty() {
                let mut should_frags = Vec::with_capacity(should.len());
                for q in should {
                    should_frags.push(build_predicate(q, fields, params, match_query)?);
                }
                required.push(format!("({})", should_frags.join(" OR ")));
            }
            if required.is_empty() {
                Ok("1".to_string())
            } else {
                Ok(format!("({})", required.join(" AND ")))
            }
        }
    }
}

/// Builds a column-filtered FTS5 `MATCH` expression, quoting every
/// whitespace-separated token of `value` as its own string literal so
/// arbitrary user input can't be interpreted as FTS5 query syntax
/// (`AND`/`OR`/`NOT`/`-`/`*`/column filters, ...). Consecutive quoted
/// tokens are implicitly ANDed by FTS5, so this still requires every word
/// in `value` to appear (in any order) in `field`.
fn fts_match_expr(field: &str, value: &str) -> String {
    let tokens: Vec<String> = value
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    let phrase = if tokens.is_empty() {
        "\"\"".to_string()
    } else {
        tokens.join(" ")
    };
    format!("{} : ({phrase})", quote_ident(field))
}

fn build_order_by(
    sorts: &[Sort],
    fields: &HashMap<String, FieldMeta>,
) -> Result<String, SearchError> {
    if sorts.is_empty() {
        return Ok("score DESC".to_string());
    }
    let mut parts = Vec::with_capacity(sorts.len());
    for sort in sorts {
        match sort {
            Sort::Score => parts.push("score DESC".to_string()),
            Sort::Field { name, order } => {
                lookup(fields, name)?;
                let dir = match order {
                    SortOrder::Asc => "ASC",
                    SortOrder::Desc => "DESC",
                };
                parts.push(format!("content.{} {dir}", quote_ident(name)));
            }
        }
    }
    Ok(parts.join(", "))
}

fn lookup<'a>(
    fields: &'a HashMap<String, FieldMeta>,
    name: &str,
) -> Result<&'a FieldMeta, SearchError> {
    fields
        .get(name)
        .ok_or_else(|| SearchError::InvalidQuery(format!("unknown field `{name}`")))
}
