use rusty_search_core::Document;
use serde_json::Value as JsonValue;

use crate::schema_map::KEY_FIELD;

// Azure AI Search's wire protocol goes through reqwest's real-serde-backed
// `.json()` convenience methods, so `serde_json::Value` stays this crate's
// HTTP-body type throughout `lib.rs`/`query_map.rs`. `rusty_serde::Value`
// is only `Document::fields`'s type; these two functions convert at
// exactly that boundary.

pub(crate) fn rusty_value_to_json(value: rusty_serde::Value) -> JsonValue {
    use rusty_serde::Value as RustyValue;
    match value {
        RustyValue::Null => JsonValue::Null,
        RustyValue::Bool(b) => JsonValue::Bool(b),
        RustyValue::Int(v) => JsonValue::Number(v.into()),
        RustyValue::UInt(v) => JsonValue::Number(v.into()),
        RustyValue::Float(v) => serde_json::Number::from_f64(v)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        RustyValue::String(s) => JsonValue::String(s),
        RustyValue::Seq(items) => {
            JsonValue::Array(items.into_iter().map(rusty_value_to_json).collect())
        }
        RustyValue::Map(entries) => JsonValue::Object(
            entries
                .into_iter()
                .map(|(k, v)| (k, rusty_value_to_json(v)))
                .collect(),
        ),
    }
}

fn json_value_to_rusty(value: JsonValue) -> rusty_serde::Value {
    use rusty_serde::Value as RustyValue;
    match value {
        JsonValue::Null => RustyValue::Null,
        JsonValue::Bool(b) => RustyValue::Bool(b),
        JsonValue::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(v), _, _) => RustyValue::Int(v),
            (None, Some(v), _) => RustyValue::UInt(v),
            (None, None, Some(v)) => RustyValue::Float(v),
            (None, None, None) => RustyValue::Null,
        },
        JsonValue::String(s) => RustyValue::String(s),
        JsonValue::Array(items) => {
            RustyValue::Seq(items.into_iter().map(json_value_to_rusty).collect())
        }
        JsonValue::Object(map) => RustyValue::Map(
            map.into_iter()
                .map(|(k, v)| (k, json_value_to_rusty(v)))
                .collect(),
        ),
    }
}

/// Metadata keys Azure AI Search adds to every search hit alongside the
/// document's own fields, stripped back out when converting a hit into a
/// core [`Document`].
const METADATA_KEYS: [&str; 3] = [
    "@search.score",
    "@search.highlights",
    "@search.rerankerScore",
];

/// Converts a core [`Document`] into an Azure AI Search document body,
/// assigning it an id first if it didn't already have one (matching the
/// other remote backends' convention), keyed under the fixed field name
/// `"id"` - see [`KEY_FIELD`]'s docs for why that's fixed rather than
/// configurable.
pub fn document_to_json(document: Document) -> (String, JsonValue) {
    let id = document
        .id
        .clone()
        .unwrap_or_else(|| rusty_uuid::Uuid::new_v4().to_string());
    let mut fields = match rusty_value_to_json(document.fields) {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    fields.insert(KEY_FIELD.to_string(), JsonValue::String(id.clone()));
    (id, JsonValue::Object(fields))
}

/// Converts an Azure AI Search document (as returned in a search hit) back
/// into a core [`Document`], stripping the `@search.*` metadata fields
/// Azure adds alongside the document's own fields.
pub fn json_to_document(value: JsonValue) -> Document {
    let mut fields = match value {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let id = fields
        .remove(KEY_FIELD)
        .and_then(|v| v.as_str().map(str::to_string));
    for key in METADATA_KEYS {
        fields.remove(key);
    }
    Document {
        id,
        fields: json_value_to_rusty(JsonValue::Object(fields)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_to_json_generates_an_id_when_missing() {
        let doc = Document::new().set("title", "no id yet");
        let (id, json) = document_to_json(doc);
        assert!(!id.is_empty());
        assert_eq!(json["id"], id);
        assert_eq!(json["title"], "no id yet");
    }

    #[test]
    fn document_to_json_keeps_an_existing_id() {
        let doc = Document::new().with_id("7").set("title", "has id");
        let (id, json) = document_to_json(doc);
        assert_eq!(id, "7");
        assert_eq!(json["id"], "7");
    }

    #[test]
    fn json_to_document_strips_search_metadata() {
        let value = serde_json::json!({
            "id": "1",
            "title": "hello",
            "@search.score": 1.23,
            "@search.highlights": {},
        });
        let doc = json_to_document(value);
        assert_eq!(doc.id.as_deref(), Some("1"));
        assert_eq!(
            doc.get("title"),
            Some(&rusty_serde::Value::String("hello".to_string()))
        );
        assert!(doc.get("@search.score").is_none());
        assert!(doc.get("@search.highlights").is_none());
    }
}
