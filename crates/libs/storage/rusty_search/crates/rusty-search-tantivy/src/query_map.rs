use std::collections::HashMap;

use rusty_search_core::{Query as CoreQuery, SearchError};
use std::ops::Bound;
use tantivy::query::{
    AllQuery, BooleanQuery, EmptyQuery, Occur, Query as TantivyQuery, RangeQuery, TermQuery,
};
use tantivy::schema::{FieldType, IndexRecordOption};
use tantivy::tokenizer::TokenStream;
use tantivy::{Index, Term};

use crate::convert::{json_value_to_term, value_to_term};
use crate::schema_map::FieldMeta;

/// Translates a core [`CoreQuery`] into a boxed Tantivy query, looking up
/// field metadata by name to pick the right `Term`/`Query` construction for
/// each field's type.
pub fn build_query(
    index: &Index,
    fields: &HashMap<String, FieldMeta>,
    query: &CoreQuery,
) -> Result<Box<dyn TantivyQuery>, SearchError> {
    match query {
        CoreQuery::MatchAll => Ok(Box::new(AllQuery)),

        CoreQuery::Term { field, value } => {
            let meta = lookup(fields, field)?;
            let term = value_to_term(meta.field, meta.field_type, value)?;
            Ok(Box::new(TermQuery::new(term, IndexRecordOption::Basic)))
        }

        CoreQuery::Match { field, value } => {
            let meta = lookup(fields, field)?;
            build_match_query(index, field, meta, value)
        }

        CoreQuery::Range { field, gte, lte } => {
            let meta = lookup(fields, field)?;
            let lower = match gte {
                Some(v) => Bound::Included(json_value_to_term(meta.field, meta.field_type, v)?),
                None => Bound::Unbounded,
            };
            let upper = match lte {
                Some(v) => Bound::Included(json_value_to_term(meta.field, meta.field_type, v)?),
                None => Bound::Unbounded,
            };
            Ok(Box::new(RangeQuery::new(lower, upper)))
        }

        CoreQuery::Bool {
            must,
            should,
            must_not,
            filter,
        } => {
            let mut clauses: Vec<(Occur, Box<dyn TantivyQuery>)> = Vec::new();
            for q in must.iter().chain(filter.iter()) {
                clauses.push((Occur::Must, build_query(index, fields, q)?));
            }
            for q in should {
                clauses.push((Occur::Should, build_query(index, fields, q)?));
            }
            for q in must_not {
                clauses.push((Occur::MustNot, build_query(index, fields, q)?));
            }
            Ok(Box::new(BooleanQuery::new(clauses)))
        }
    }
}

fn lookup<'a>(
    fields: &'a HashMap<String, FieldMeta>,
    name: &str,
) -> Result<&'a FieldMeta, SearchError> {
    fields
        .get(name)
        .ok_or_else(|| SearchError::InvalidQuery(format!("unknown field `{name}`")))
}

/// Builds an analyzed, field-scoped match query: tokenizes `value` with
/// `meta.field`'s own configured analyzer and combines the resulting terms
/// with `Occur::Must`, restricted to `meta.field` alone.
///
/// Unlike `QueryParser::parse_query`, this never hands the caller's string
/// to a free-text parser - there is no way for `value` to override the
/// target field, use boolean/range/wildcard syntax, or otherwise reach
/// outside `meta.field`.
fn build_match_query(
    index: &Index,
    field_name: &str,
    meta: &FieldMeta,
    value: &str,
) -> Result<Box<dyn TantivyQuery>, SearchError> {
    let schema = index.schema();
    let entry = schema.get_field_entry(meta.field);
    let text_options = match entry.field_type() {
        FieldType::Str(text_options) => text_options,
        // Non-text fields have no analyzer to tokenize against; a "match"
        // on them reduces to an exact term against the field's native type.
        _ => {
            let term = value_to_term(meta.field, meta.field_type, value)?;
            return Ok(Box::new(TermQuery::new(term, IndexRecordOption::Basic)));
        }
    };
    let indexing_options = text_options.get_indexing_options().ok_or_else(|| {
        SearchError::InvalidQuery(format!(
            "field `{field_name}` is not indexed for match queries"
        ))
    })?;
    let tokenizer_name = indexing_options.tokenizer();
    let mut analyzer = index.tokenizers().get(tokenizer_name).ok_or_else(|| {
        SearchError::InvalidQuery(format!("unknown tokenizer `{tokenizer_name}`"))
    })?;

    let mut terms = Vec::new();
    let mut token_stream = analyzer.token_stream(value);
    token_stream.process(&mut |token| {
        terms.push(Term::from_field_text(meta.field, &token.text));
    });

    match terms.len() {
        0 => Ok(Box::new(EmptyQuery)),
        1 => Ok(Box::new(TermQuery::new(
            terms.into_iter().next().expect("checked len == 1 above"),
            IndexRecordOption::Basic,
        ))),
        _ => {
            let clauses: Vec<(Occur, Box<dyn TantivyQuery>)> = terms
                .into_iter()
                .map(|term| {
                    (
                        Occur::Must,
                        Box::new(TermQuery::new(term, IndexRecordOption::Basic))
                            as Box<dyn TantivyQuery>,
                    )
                })
                .collect();
            Ok(Box::new(BooleanQuery::new(clauses)))
        }
    }
}
