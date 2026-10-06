//! The one `rusty_serde::Value` <-> `serde_json::Value` conversion shared by
//! every backend that speaks `serde_json` on the wire (the HTTP backends
//! and tantivy), behind the `serde-json` feature. Each of them used to
//! carry an identical private copy.
//!
//! The conversion is deliberately simple and has three lossy points,
//! which callers must not rely on being otherwise:
//!
//! - **Non-finite floats.** `serde_json` cannot represent NaN or ±Inf, so
//!   [`value_to_json`] turns them into `Null`.
//! - **Unsigned integers.** [`json_to_value`] tries `i64` first, so a
//!   `UInt` whose value fits in `i64` comes back as `Int`; only a value
//!   above `i64::MAX` round-trips as `UInt`.
//! - **Map order and duplicate keys.** `rusty_serde::Value::Map` is an
//!   ordered list of pairs and may hold a key twice; `serde_json::Map`
//!   holds each key once (the last occurrence wins) and orders keys
//!   according to serde_json's own configuration (`preserve_order` on or
//!   off), not according to the input.

use rusty_serde::Value;
use serde_json::Value as JsonValue;

/// Converts a `rusty_serde::Value` into a `serde_json::Value`. See the
/// module docs for what is lossy.
pub fn value_to_json(value: Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(b),
        Value::Int(v) => JsonValue::Number(v.into()),
        Value::UInt(v) => JsonValue::Number(v.into()),
        Value::Float(v) => serde_json::Number::from_f64(v)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(s) => JsonValue::String(s),
        Value::Seq(items) => JsonValue::Array(items.into_iter().map(value_to_json).collect()),
        Value::Map(entries) => JsonValue::Object(
            entries
                .into_iter()
                .map(|(k, v)| (k, value_to_json(v)))
                .collect(),
        ),
    }
}

/// Converts a `serde_json::Value` into a `rusty_serde::Value`. See the
/// module docs for what is lossy.
pub fn json_to_value(value: JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(b),
        JsonValue::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(v), _, _) => Value::Int(v),
            (None, Some(v), _) => Value::UInt(v),
            (None, None, Some(v)) => Value::Float(v),
            (None, None, None) => Value::Null,
        },
        JsonValue::String(s) => Value::String(s),
        JsonValue::Array(items) => Value::Seq(items.into_iter().map(json_to_value).collect()),
        JsonValue::Object(map) => Value::Map(
            map.into_iter()
                .map(|(k, v)| (k, json_to_value(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, Value)]) -> Value {
        Value::Map(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn scalars_and_containers_round_trip() {
        let v = map(&[
            ("n", Value::Null),
            ("b", Value::Bool(true)),
            ("i", Value::Int(-7)),
            ("f", Value::Float(1.5)),
            ("s", Value::String("x".into())),
            (
                "seq",
                Value::Seq(vec![Value::Int(1), Value::String("two".into())]),
            ),
            ("nested", map(&[("k", Value::Int(2))])),
        ]);
        let back = json_to_value(value_to_json(v.clone()));
        // Key order is serde_json's business (see module docs), so compare
        // as sets of pairs rather than as ordered lists.
        let (Value::Map(a), Value::Map(b)) = (v, back) else {
            panic!("expected maps");
        };
        assert_eq!(a.len(), b.len());
        for pair in &a {
            assert!(b.contains(pair), "missing {pair:?}");
        }
    }

    #[test]
    fn non_finite_floats_become_null() {
        for f in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(value_to_json(Value::Float(f)), JsonValue::Null);
        }
        assert_eq!(value_to_json(Value::Float(0.25)), serde_json::json!(0.25));
    }

    #[test]
    fn uint_within_i64_comes_back_as_int_and_above_it_stays_uint() {
        assert_eq!(
            json_to_value(value_to_json(Value::UInt(42))),
            Value::Int(42)
        );
        assert_eq!(
            json_to_value(value_to_json(Value::UInt(i64::MAX as u64))),
            Value::Int(i64::MAX)
        );
        let big = i64::MAX as u64 + 1;
        assert_eq!(
            json_to_value(value_to_json(Value::UInt(big))),
            Value::UInt(big)
        );
        assert_eq!(
            json_to_value(value_to_json(Value::UInt(u64::MAX))),
            Value::UInt(u64::MAX)
        );
    }

    #[test]
    fn duplicate_keys_collapse_last_wins_and_key_set_is_preserved() {
        let v = map(&[
            ("a", Value::Int(1)),
            ("b", Value::Int(2)),
            ("a", Value::Int(3)),
        ]);
        let json = value_to_json(v);
        let obj = json.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["a", "b"]);
        assert_eq!(obj["a"], serde_json::json!(3));
        assert_eq!(obj["b"], serde_json::json!(2));
    }

    #[test]
    fn json_integers_map_by_width() {
        assert_eq!(json_to_value(serde_json::json!(-1)), Value::Int(-1));
        assert_eq!(
            json_to_value(serde_json::json!(u64::MAX)),
            Value::UInt(u64::MAX)
        );
        assert_eq!(json_to_value(serde_json::json!(2.5)), Value::Float(2.5));
    }
}
