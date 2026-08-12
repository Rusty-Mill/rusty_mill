use std::collections::HashMap;

use rusqlite::types::Value as SqlValue;
use rusqlite::Row;
use rusty_search_core::{Document, FieldType as CoreFieldType, SearchError};
use serde_json::Value as JsonValue;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::schema_map::FieldMeta;

/// Validates (but doesn't reformat) an RFC 3339 timestamp string, the same
/// representation `rusty-search-tantivy` uses for `Date` fields.
pub fn validate_date(value: &str) -> Result<(), SearchError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|_| ())
        .map_err(|e| SearchError::InvalidQuery(format!("invalid RFC 3339 date `{value}`: {e}")))
}

/// Converts a `Query::Term`/`Query::Range` bound's string/JSON
/// representation into the SQL value bound against a field's typed
/// `content` column.
pub fn value_to_sql(field_type: CoreFieldType, value: &str) -> Result<SqlValue, SearchError> {
    match field_type {
        CoreFieldType::Text | CoreFieldType::Keyword => Ok(SqlValue::Text(value.to_string())),
        CoreFieldType::Date => {
            validate_date(value)?;
            Ok(SqlValue::Text(value.to_string()))
        }
        CoreFieldType::I64 => value
            .parse::<i64>()
            .map(SqlValue::Integer)
            .map_err(|e| SearchError::InvalidQuery(format!("expected an integer: {e}"))),
        CoreFieldType::F64 => value
            .parse::<f64>()
            .map(SqlValue::Real)
            .map_err(|e| SearchError::InvalidQuery(format!("expected a float: {e}"))),
        CoreFieldType::Bool => value
            .parse::<bool>()
            .map(|b| SqlValue::Integer(b as i64))
            .map_err(|e| SearchError::InvalidQuery(format!("expected a bool: {e}"))),
    }
}

/// Converts a `Query::Range` bound, carried as a JSON value, into the SQL
/// value bound against a field's typed `content` column.
pub fn json_value_to_sql(
    field_type: CoreFieldType,
    value: &JsonValue,
) -> Result<SqlValue, SearchError> {
    match field_type {
        CoreFieldType::I64 => value
            .as_i64()
            .map(SqlValue::Integer)
            .ok_or_else(|| SearchError::InvalidQuery("expected an integer".to_string())),
        CoreFieldType::F64 => value
            .as_f64()
            .map(SqlValue::Real)
            .ok_or_else(|| SearchError::InvalidQuery("expected a number".to_string())),
        CoreFieldType::Date | CoreFieldType::Keyword | CoreFieldType::Text => {
            let s = value
                .as_str()
                .ok_or_else(|| SearchError::InvalidQuery("expected a string value".to_string()))?;
            if field_type == CoreFieldType::Date {
                validate_date(s)?;
            }
            Ok(SqlValue::Text(s.to_string()))
        }
        CoreFieldType::Bool => value
            .as_bool()
            .map(|b| SqlValue::Integer(b as i64))
            .ok_or_else(|| SearchError::InvalidQuery("expected a bool".to_string())),
    }
}

/// Converts a document field's JSON value into the SQL value stored in its
/// `content` column, coercing loosely (e.g. a JSON number for a `Keyword`
/// field is stringified) rather than rejecting - matching how
/// `Document::set` accepts anything `Into<Value>`.
fn field_value_to_sql(field_type: CoreFieldType, value: &JsonValue) -> SqlValue {
    match field_type {
        CoreFieldType::Text | CoreFieldType::Keyword | CoreFieldType::Date => match value {
            JsonValue::String(s) => SqlValue::Text(s.clone()),
            other => SqlValue::Text(other.to_string()),
        },
        CoreFieldType::I64 => value
            .as_i64()
            .map(SqlValue::Integer)
            .unwrap_or(SqlValue::Null),
        CoreFieldType::F64 => value.as_f64().map(SqlValue::Real).unwrap_or(SqlValue::Null),
        CoreFieldType::Bool => value
            .as_bool()
            .map(|b| SqlValue::Integer(b as i64))
            .unwrap_or(SqlValue::Null),
    }
}

/// A document ready to insert: its id, the values for every `content`
/// column (in schema field order), and the text values for every `idx_fts`
/// column (in the same relative order the FTS5 table was created with).
pub struct PreparedDocument {
    pub id: String,
    pub content_values: Vec<SqlValue>,
    pub fts_values: Vec<String>,
}

/// Converts a core [`Document`] into column values ready to bind into
/// `INSERT` statements, assigning it an id first if it didn't already have
/// one - same client-side id generation `rusty-search-tantivy` uses.
///
/// Fields not present in the index's schema are silently dropped, and
/// missing schema fields on the document bind as SQL `NULL`.
pub fn document_to_row(
    fields: &HashMap<String, FieldMeta>,
    field_order: &[String],
    document: Document,
) -> PreparedDocument {
    let id = document
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut content_values = Vec::with_capacity(field_order.len());
    let mut fts_values = Vec::new();
    for name in field_order {
        let meta = fields
            .get(name)
            .expect("field_order only lists known fields");
        let value = document.fields.get(name);
        let sql_value = value
            .map(|v| field_value_to_sql(meta.field_type, v))
            .unwrap_or(SqlValue::Null);
        if meta.fts_indexed {
            let text = match value {
                Some(JsonValue::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            };
            fts_values.push(text);
        }
        content_values.push(sql_value);
    }

    PreparedDocument {
        id,
        content_values,
        fts_values,
    }
}

/// Reconstructs a core [`Document`] from a `content` table row, omitting
/// fields whose schema marks them `stored: false`.
pub fn row_to_document(row: &Row<'_>, fields: &HashMap<String, FieldMeta>) -> Document {
    let id: String = row.get("_id").expect("_id column is always present");
    let mut object = serde_json::Map::new();

    for (name, meta) in fields {
        if !meta.stored {
            continue;
        }
        let value = sql_value_from_row(row, name, meta.field_type);
        if let Some(value) = value {
            object.insert(name.clone(), value);
        }
    }

    Document {
        id: Some(id),
        fields: object,
    }
}

fn sql_value_from_row(row: &Row<'_>, name: &str, field_type: CoreFieldType) -> Option<JsonValue> {
    match field_type {
        CoreFieldType::Text | CoreFieldType::Keyword | CoreFieldType::Date => row
            .get::<_, Option<String>>(name)
            .ok()
            .flatten()
            .map(JsonValue::String),
        CoreFieldType::I64 => row
            .get::<_, Option<i64>>(name)
            .ok()
            .flatten()
            .map(|v| JsonValue::Number(v.into())),
        CoreFieldType::F64 => row
            .get::<_, Option<f64>>(name)
            .ok()
            .flatten()
            .and_then(serde_json::Number::from_f64)
            .map(JsonValue::Number),
        CoreFieldType::Bool => row
            .get::<_, Option<i64>>(name)
            .ok()
            .flatten()
            .map(|v| JsonValue::Bool(v != 0)),
    }
}
