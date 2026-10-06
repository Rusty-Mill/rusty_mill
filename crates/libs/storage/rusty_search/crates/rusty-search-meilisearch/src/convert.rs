use rusty_search_core::Document;
use serde_json::Value as JsonValue;

// The `rusty_serde::Value` <-> `serde_json::Value` conversion itself is
// shared with every other serde_json-speaking backend; see
// `rusty_search_core::serde_json_bridge` for its (documented) lossy points.
pub(crate) use rusty_search_core::serde_json_bridge::{
    json_to_value as json_value_to_rusty, value_to_json as rusty_value_to_json,
};

// The official `meilisearch-sdk` crate's document methods take/return real
// `serde_json::Value`, so `serde_json::Value` stays this crate's wire type
// throughout `lib.rs`/`query_map.rs`. `rusty_serde::Value` is only
// `Document::fields`'s type; these two functions convert at exactly that
// boundary.

/// Meilisearch stores a document's primary key *inside* the document body
/// (unlike Elasticsearch's separate `_id`/`_source`, or Tantivy's reserved
/// field), so this backend always uses `"id"` as the primary key and keeps
/// it out of `Document::fields` on the way back, mirroring how the other
/// backends keep their own id representation out of `fields`.
pub const PRIMARY_KEY: &str = "id";

/// Converts a core [`Document`] into the JSON object Meilisearch expects,
/// assigning it an id first if it didn't already have one - matching the
/// other backends' convention of generating one client-side.
pub fn document_to_json(document: Document) -> (String, JsonValue) {
    let id = document
        .id
        .clone()
        .unwrap_or_else(|| rusty_uuid::Uuid::new_v4().to_string());
    let mut fields = match rusty_value_to_json(document.fields) {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    fields.insert(PRIMARY_KEY.to_string(), JsonValue::String(id.clone()));
    (id, JsonValue::Object(fields))
}

/// Converts a Meilisearch document (as returned by search or document
/// fetch) back into a core [`Document`], pulling `"id"` out into
/// [`Document::id`].
pub fn json_to_document(value: JsonValue) -> Document {
    let mut fields = match value {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let id = fields
        .remove(PRIMARY_KEY)
        .and_then(|v| v.as_str().map(str::to_string));
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
    fn json_to_document_pulls_id_out_of_fields() {
        let value = serde_json::json!({ "id": "1", "title": "hello" });
        let doc = json_to_document(value);
        assert_eq!(doc.id.as_deref(), Some("1"));
        assert!(doc.get("id").is_none());
        assert_eq!(
            doc.get("title"),
            Some(&rusty_serde::Value::String("hello".to_string()))
        );
    }
}
