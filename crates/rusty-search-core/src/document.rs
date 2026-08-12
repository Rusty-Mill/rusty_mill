use rusty_serde::{Deserialize, Serialize, Value};

/// A document identifier, unique within a single index.
pub type DocumentId = String;

/// An engine-agnostic document: an optional id plus a bag of named fields.
///
/// This mirrors the role of a row in SQLAlchemy Core - a plain, dynamically
/// typed record that any backend can serialize into its own storage format.
/// Application code typically converts its own structs to/from `Document`
/// via `rusty_serde::json`, e.g. `Document::from_serializable(&my_struct)`.
///
/// `fields` is always the `Value::Map` variant - every constructor
/// (`Document::default`/`new`, `Document::from_serializable`) establishes
/// that invariant, and `Document::set` relies on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: Option<DocumentId>,
    #[rusty_serde(flatten)]
    pub fields: Value,
}

impl Default for Document {
    fn default() -> Self {
        Document {
            id: None,
            fields: Value::Map(Vec::new()),
        }
    }
}

impl Document {
    /// Creates an empty document with no id and no fields.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the document id, consuming and returning `self` for chaining.
    pub fn with_id(mut self, id: impl Into<DocumentId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets a field value, consuming and returning `self` for chaining.
    pub fn set(mut self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        let key = field.into();
        let value = value.into();
        let Value::Map(entries) = &mut self.fields else {
            unreachable!("Document::fields is always the Value::Map variant");
        };
        match entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => entry.1 = value,
            None => entries.push((key, value)),
        }
        self
    }

    /// Reads a field's raw value, if present.
    pub fn get(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }

    /// Builds a `Document` from any `Serialize` type. The type must
    /// serialize to an object. an `id` field, if present, is pulled out
    /// into [`Document::id`].
    pub fn from_serializable<T: Serialize>(value: &T) -> Result<Self, rusty_serde::json::Error> {
        let value = rusty_serde::json::to_value(value)?;
        let mut entries = match value {
            Value::Map(entries) => entries,
            other => {
                return Err(<rusty_serde::json::Error as rusty_serde::Error>::custom(
                    format!("expected an object, got {other}"),
                ))
            }
        };
        let id = entries
            .iter()
            .position(|(k, _)| k == "id")
            .map(|i| entries.remove(i).1)
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                Value::Int(n) => Some(n.to_string()),
                Value::UInt(n) => Some(n.to_string()),
                Value::Float(n) => Some(n.to_string()),
                _ => None,
            });
        Ok(Document {
            id,
            fields: Value::Map(entries),
        })
    }

    /// Deserializes the document's fields (and `id`, if the target type has
    /// one) into a concrete type.
    pub fn into_serializable<T: for<'de> Deserialize<'de>>(
        mut self,
    ) -> Result<T, rusty_serde::json::Error> {
        if let Some(id) = self.id {
            if let Value::Map(entries) = &mut self.fields {
                entries.push(("id".to_string(), Value::String(id)));
            }
        }
        rusty_serde::json::from_value(self.fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Article {
        id: String,
        title: String,
        views: u32,
    }

    #[test]
    fn builder_sets_id_and_fields() {
        let doc = Document::new().with_id("1").set("title", "hello");
        assert_eq!(doc.id.as_deref(), Some("1"));
        assert_eq!(doc.get("title"), Some(&Value::String("hello".into())));
    }

    #[test]
    fn roundtrips_through_serializable() {
        let article = Article {
            id: "42".to_string(),
            title: "Rust".to_string(),
            views: 7,
        };
        let doc = Document::from_serializable(&article).unwrap();
        assert_eq!(doc.id.as_deref(), Some("42"));
        assert_eq!(doc.get("title"), Some(&Value::String("Rust".into())));

        let back: Article = doc.into_serializable().unwrap();
        assert_eq!(back, article);
    }

    #[test]
    fn from_serializable_rejects_non_object() {
        let err = Document::from_serializable(&42).unwrap_err();
        assert!(err.to_string().contains("expected an object"));
    }
}
