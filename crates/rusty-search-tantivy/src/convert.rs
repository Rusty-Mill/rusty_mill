use std::collections::HashMap;

use rusty_search_core::{Document, FieldType as CoreFieldType, SearchError};
use rusty_serde::Value as RustyValue;
use tantivy::schema::document::{Document as TantivyDocumentTrait, TantivyDocument};
use tantivy::schema::{Field, OwnedValue, Schema as TantivySchema};
use tantivy::Term;

use crate::schema_map::{FieldMeta, ID_FIELD_NAME};

// `tantivy::schema::document::TantivyDocument::from_json_object` is
// `tantivy`'s own API, hard-requiring literal `serde_json::Map`/`Value`
// since `tantivy` itself depends on real `serde_json` - neither
// `rusty_serde` nor any other internal replacement removes that
// boundary. `document_to_tantivy`/`tantivy_doc_to_document` convert
// between `rusty_serde::Value` (what `Document::fields` is) and
// `serde_json::Value` (what `tantivy` needs) right at that boundary,
// rather than anywhere else in this crate.

fn rusty_value_to_json(value: RustyValue) -> serde_json::Value {
    match value {
        RustyValue::Null => serde_json::Value::Null,
        RustyValue::Bool(b) => serde_json::Value::Bool(b),
        RustyValue::Int(v) => serde_json::Value::Number(v.into()),
        RustyValue::UInt(v) => serde_json::Value::Number(v.into()),
        RustyValue::Float(v) => serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        RustyValue::String(s) => serde_json::Value::String(s),
        RustyValue::Seq(items) => {
            serde_json::Value::Array(items.into_iter().map(rusty_value_to_json).collect())
        }
        RustyValue::Map(entries) => serde_json::Value::Object(
            entries
                .into_iter()
                .map(|(k, v)| (k, rusty_value_to_json(v)))
                .collect(),
        ),
    }
}

fn json_value_to_rusty(value: serde_json::Value) -> RustyValue {
    match value {
        serde_json::Value::Null => RustyValue::Null,
        serde_json::Value::Bool(b) => RustyValue::Bool(b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(v), _, _) => RustyValue::Int(v),
            (None, Some(v), _) => RustyValue::UInt(v),
            (None, None, Some(v)) => RustyValue::Float(v),
            (None, None, None) => RustyValue::Null,
        },
        serde_json::Value::String(s) => RustyValue::String(s),
        serde_json::Value::Array(items) => {
            RustyValue::Seq(items.into_iter().map(json_value_to_rusty).collect())
        }
        serde_json::Value::Object(map) => RustyValue::Map(
            map.into_iter()
                .map(|(k, v)| (k, json_value_to_rusty(v)))
                .collect(),
        ),
    }
}

/// Parses an RFC 3339 timestamp string into a Tantivy `DateTime`.
pub fn parse_date(value: &str) -> Result<tantivy::DateTime, SearchError> {
    let dt = rusty_time::DateTime::parse(value)
        .map_err(|e| SearchError::InvalidQuery(format!("invalid RFC 3339 date `{value}`: {e}")))?;
    // rusty_time has no time::OffsetDateTime-shaped type to hand to
    // tantivy::DateTime::from_utc, but from_timestamp_nanos takes a plain
    // nanoseconds-since-epoch integer - reconstructed here from whole
    // seconds plus the sub-second remainder, preserving full precision.
    let nanos = dt.timestamp() * 1_000_000_000 + dt.time().nanosecond() as i64;
    Ok(tantivy::DateTime::from_timestamp_nanos(nanos))
}

/// Builds a `Term` for exact matching (used by `Query::Term` and range
/// bounds) against `field`, converting the JSON-ish string representation
/// callers pass in the core `Query` DSL into the field's native type.
pub fn value_to_term(
    field: Field,
    field_type: CoreFieldType,
    value: &str,
) -> Result<Term, SearchError> {
    match field_type {
        CoreFieldType::Text | CoreFieldType::Keyword => Ok(Term::from_field_text(field, value)),
        CoreFieldType::I64 => value
            .parse::<i64>()
            .map(|v| Term::from_field_i64(field, v))
            .map_err(|e| SearchError::InvalidQuery(format!("expected an integer: {e}"))),
        CoreFieldType::F64 => value
            .parse::<f64>()
            .map(|v| Term::from_field_f64(field, v))
            .map_err(|e| SearchError::InvalidQuery(format!("expected a float: {e}"))),
        CoreFieldType::Bool => value
            .parse::<bool>()
            .map(|v| Term::from_field_bool(field, v))
            .map_err(|e| SearchError::InvalidQuery(format!("expected a bool: {e}"))),
        CoreFieldType::Date => parse_date(value).map(|dt| Term::from_field_date(field, dt)),
    }
}

/// Builds a `Term` for a numeric/date range bound expressed as a `Value`
/// (as carried by `Query::Range`).
pub fn json_value_to_term(
    field: Field,
    field_type: CoreFieldType,
    value: &RustyValue,
) -> Result<Term, SearchError> {
    match field_type {
        CoreFieldType::I64 => value
            .as_i64()
            .map(|v| Term::from_field_i64(field, v))
            .ok_or_else(|| SearchError::InvalidQuery("expected an integer".to_string())),
        CoreFieldType::F64 => value
            .as_f64()
            .map(|v| Term::from_field_f64(field, v))
            .ok_or_else(|| SearchError::InvalidQuery("expected a number".to_string())),
        CoreFieldType::Date => {
            let s = value.as_str().ok_or_else(|| {
                SearchError::InvalidQuery("expected an RFC 3339 date string".to_string())
            })?;
            parse_date(s).map(|dt| Term::from_field_date(field, dt))
        }
        other => Err(SearchError::InvalidQuery(format!(
            "range queries are not supported on {other:?} fields"
        ))),
    }
}

/// Converts a core [`Document`] into a Tantivy document ready for indexing,
/// assigning it an id first if it didn't already have one.
///
/// Fields not present in the index's schema are silently dropped, matching
/// `TantivyDocument::from_json_object`'s own behavior.
pub fn document_to_tantivy(
    tantivy_schema: &TantivySchema,
    document: Document,
) -> (String, TantivyDocument) {
    let id = document
        .id
        .clone()
        .unwrap_or_else(|| rusty_uuid::Uuid::new_v4().to_string());

    let mut object = match rusty_value_to_json(document.fields) {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    object.insert(
        ID_FIELD_NAME.to_string(),
        serde_json::Value::String(id.clone()),
    );

    let tantivy_doc = TantivyDocument::from_json_object(tantivy_schema, object)
        .expect("document fields were already validated against this schema");
    (id, tantivy_doc)
}

/// Converts a Tantivy document (as retrieved from a `Searcher`) back into a
/// core [`Document`], pulling the reserved id field out into `Document::id`.
pub fn tantivy_doc_to_document(
    tantivy_doc: &TantivyDocument,
    tantivy_schema: &TantivySchema,
    fields: &HashMap<String, FieldMeta>,
) -> Document {
    let named = tantivy_doc.to_named_doc(tantivy_schema);
    let mut id = None;
    let mut object = serde_json::Map::new();

    for (name, values) in named.0 {
        if name == ID_FIELD_NAME {
            if let Some(OwnedValue::Str(s)) = values.into_iter().next() {
                id = Some(s);
            }
            continue;
        }
        if !fields.contains_key(&name) {
            continue;
        }
        let mut json_values: Vec<serde_json::Value> = values
            .into_iter()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
            .collect();
        let value = if json_values.len() == 1 {
            json_values.pop().unwrap()
        } else {
            serde_json::Value::Array(json_values)
        };
        object.insert(name, value);
    }

    Document {
        id,
        fields: json_value_to_rusty(serde_json::Value::Object(object)),
    }
}
