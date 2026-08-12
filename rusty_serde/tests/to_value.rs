//! Coverage for `rusty_serde::json::{to_value, from_value}`: converting
//! between a `Serialize`/`Deserialize` type and a `Value` tree directly,
//! without going through JSON text - the gap flagged by
//! `rusty_search`'s `Document::from_serializable`/`into_serializable`,
//! which need exactly this (`serde_json::to_value`/`from_value`) rather
//! than a string round trip.

use rusty_serde::json::{self, Value};
use rusty_serde::{Deserialize, Serialize};

#[test]
fn to_value_then_from_value_round_trips_scalars() {
    assert_eq!(json::to_value(&true).unwrap(), Value::Bool(true));
    assert_eq!(json::to_value(&-7i32).unwrap(), Value::Int(-7));
    assert_eq!(json::to_value(&7u32).unwrap(), Value::UInt(7));
    assert_eq!(json::to_value(&"hi").unwrap(), Value::String("hi".into()));
    assert_eq!(json::to_value(&Option::<i32>::None).unwrap(), Value::Null);

    let v = json::to_value(&42i32).unwrap();
    let back: i32 = json::from_value(v).unwrap();
    assert_eq!(back, 42);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Article {
    id: String,
    title: String,
    views: u32,
}

#[test]
fn struct_round_trips_through_a_value_tree() {
    let article = Article {
        id: "42".to_string(),
        title: "Rust".to_string(),
        views: 7,
    };

    let value = json::to_value(&article).unwrap();
    assert_eq!(value["id"].as_str(), Some("42"));
    assert_eq!(value["title"].as_str(), Some("Rust"));
    assert_eq!(value["views"].as_u64(), Some(7));

    let back: Article = json::from_value(value).unwrap();
    assert_eq!(back, article);
}

#[test]
fn to_value_produces_the_same_json_text_as_to_string() {
    // `to_value` preserves the source field's signedness (`u32` ->
    // `Value::UInt`), while parsing already-serialized JSON text back into
    // a `Value` collapses any integer that fits into `Value::Int` (JSON's
    // grammar has no separate signed/unsigned number syntax) - so the two
    // `Value` trees aren't always identical for non-negative integers, even
    // though both serialize back to the same JSON text.
    let article = Article {
        id: "1".to_string(),
        title: "x".to_string(),
        views: 0,
    };
    let via_value = json::to_value(&article).unwrap();
    assert_eq!(
        json::to_string(&via_value).unwrap(),
        json::to_string(&article).unwrap()
    );
}

// This is the exact shape `rusty_search`'s `Document` needs:
// `Document { id: Option<DocumentId>, #[rusty_serde(flatten)] fields: Value }`,
// built from an arbitrary application struct via `to_value`/`from_value`
// rather than a JSON-text round trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Document {
    id: Option<String>,
    #[rusty_serde(flatten)]
    fields: Value,
}

impl Document {
    fn from_serializable<T: Serialize>(value: &T) -> Result<Self, json::Error> {
        let value = json::to_value(value)?;
        let Value::Map(mut entries) = value else {
            return Err(<json::Error as rusty_serde::Error>::custom(
                "expected a JSON object",
            ));
        };
        let id = entries
            .iter()
            .position(|(k, _)| k == "id")
            .map(|i| entries.remove(i).1)
            .and_then(|v| v.as_str().map(str::to_string));
        Ok(Document {
            id,
            fields: Value::Map(entries),
        })
    }

    fn into_serializable<T: for<'de> Deserialize<'de>>(mut self) -> Result<T, json::Error> {
        if let Some(id) = self.id {
            if let Value::Map(entries) = &mut self.fields {
                entries.push(("id".to_string(), Value::String(id)));
            }
        }
        json::from_value(self.fields)
    }
}

#[test]
fn document_shaped_from_serializable_pulls_out_id_and_flattens_the_rest() {
    let article = Article {
        id: "42".to_string(),
        title: "Rust".to_string(),
        views: 7,
    };

    let doc = Document::from_serializable(&article).unwrap();
    assert_eq!(doc.id.as_deref(), Some("42"));
    assert_eq!(doc.fields["title"].as_str(), Some("Rust"));
    assert_eq!(doc.fields["views"].as_u64(), Some(7));

    let back: Article = doc.into_serializable().unwrap();
    assert_eq!(back, article);
}

#[test]
fn document_shaped_from_serializable_rejects_non_object() {
    let err = Document::from_serializable(&42).unwrap_err();
    assert!(err.to_string().contains("expected a JSON object"));
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Shape {
    Unit,
    Newtype(i32),
    Tuple(i32, i32),
    Struct { x: i32, y: i32 },
}

#[test]
fn enum_variants_round_trip_through_a_value_tree() {
    for shape in [
        Shape::Unit,
        Shape::Newtype(1),
        Shape::Tuple(1, 2),
        Shape::Struct { x: 1, y: 2 },
    ] {
        let value = json::to_value(&shape).unwrap();
        let back: Shape = json::from_value(value).unwrap();
        assert_eq!(back, shape);
    }
}

#[test]
fn unit_variant_serializes_as_a_bare_string_value() {
    assert_eq!(
        json::to_value(&Shape::Unit).unwrap(),
        Value::String("Unit".to_string())
    );
}

#[test]
fn newtype_variant_serializes_as_a_single_entry_map() {
    assert_eq!(
        json::to_value(&Shape::Newtype(5)).unwrap(),
        Value::Map(vec![("Newtype".to_string(), Value::Int(5))])
    );
}

#[test]
fn map_with_non_string_scalar_keys_coerces_them_to_strings() {
    // Matches the JSON format's own `KeySerializer`: a non-string scalar
    // key (int/bool/float) is coerced to its string form rather than
    // rejected, since JSON object keys are always strings. Deserializing
    // back into a non-`String`-keyed map is a separate, pre-existing
    // limitation of this crate unrelated to `to_value`/`from_value` - map
    // keys are always read back as strings (see `ValueMapAccess`), so only
    // `String`/`&str`-keyed maps round-trip.
    let map: std::collections::BTreeMap<i32, &str> = [(1, "a"), (2, "b")].into_iter().collect();
    let value = json::to_value(&map).unwrap();
    assert_eq!(value["1"].as_str(), Some("a"));
    assert_eq!(value["2"].as_str(), Some("b"));
}

#[test]
fn string_keyed_map_round_trips_via_to_value() {
    let map: std::collections::BTreeMap<String, i32> = [("a".to_string(), 1), ("b".to_string(), 2)]
        .into_iter()
        .collect();
    let value = json::to_value(&map).unwrap();
    assert_eq!(value["a"].as_i64(), Some(1));
    assert_eq!(value["b"].as_i64(), Some(2));

    let back: std::collections::BTreeMap<String, i32> = json::from_value(value).unwrap();
    assert_eq!(back, map);
}
