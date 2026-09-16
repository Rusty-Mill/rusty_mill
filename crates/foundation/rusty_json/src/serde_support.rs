//! `serde::Serialize`/`Deserialize` impls for [`Value`], so it can be used
//! with any serde data format, not just this crate's own parser/serializer.

use crate::{Map, Number, Value};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serializer};

impl serde::Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    serializer.serialize_u64(u)
                } else if let Some(i) = n.as_i64() {
                    serializer.serialize_i64(i)
                } else {
                    serializer.serialize_f64(n.as_f64())
                }
            }
            Value::String(s) => serializer.serialize_str(s),
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Object(map) => {
                let mut ser_map = serializer.serialize_map(Some(map.len()))?;
                for (key, val) in map {
                    ser_map.serialize_entry(key, val)?;
                }
                ser_map.end()
            }
        }
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a valid JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(Number::from(v)))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(Number::from(v)))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        // Non-finite floats become `Null`, same as `Value::from(f64)`
        // (Phase 1) -- JSON has no way to represent them.
        Ok(Value::from(v))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(String::from(v)))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut vec = Vec::new();
        while let Some(elem) = seq.next_element()? {
            vec.push(elem);
        }
        Ok(Value::Array(vec))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some((key, val)) = map.next_entry()? {
            object.insert(key, val);
        }
        Ok(Value::Object(object))
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_through_real_serde_json_serializer() {
        // Drives our `impl Serialize for Value` through an actual external
        // serde data format (not our own writer), proving it's genuinely
        // serde-compatible rather than just internally consistent.
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Number(Number::from(1u64)));
        let value = Value::Object(map);

        assert_eq!(serde_json::to_string(&value).unwrap(), r#"{"a":1}"#);
    }

    #[test]
    fn serializes_all_variants_through_serde_json() {
        let values = alloc::vec![
            Value::Null,
            Value::Bool(true),
            Value::Number(Number::from(1u64)),
            Value::Number(Number::from(-1i64)),
            Value::Number(Number::from_f64(1.5).unwrap()),
            Value::String(String::from("s")),
            Value::Array(Vec::new()),
            Value::Object(Map::new()),
        ];
        let expected = ["null", "true", "1", "-1", "1.5", "\"s\"", "[]", "{}"];
        for (v, want) in values.iter().zip(expected) {
            assert_eq!(serde_json::to_string(v).unwrap(), want);
        }
    }

    #[test]
    fn nested_array_and_object_through_serde_json() {
        let value = Value::Array(alloc::vec![
            Value::Number(Number::from(1u64)),
            Value::Object({
                let mut m = Map::new();
                m.insert(String::from("k"), Value::Bool(true));
                m
            }),
        ]);
        assert_eq!(serde_json::to_string(&value).unwrap(), r#"[1,{"k":true}]"#);
    }

    #[test]
    fn deserializes_through_real_serde_json_deserializer() {
        // Drives our `impl Deserialize for Value` through serde_json's
        // actual `Deserializer`, not our own parser.
        let v: Value = serde_json::from_str(r#"{"a":1,"b":[true,null]}"#).unwrap();
        let mut expected = Map::new();
        expected.insert(String::from("a"), Value::Number(Number::from(1u64)));
        expected.insert(
            String::from("b"),
            Value::Array(alloc::vec![Value::Bool(true), Value::Null]),
        );
        assert_eq!(v, Value::Object(expected));
    }

    #[test]
    fn deserializes_all_scalar_kinds() {
        assert_eq!(serde_json::from_str::<Value>("null").unwrap(), Value::Null);
        assert_eq!(
            serde_json::from_str::<Value>("true").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            serde_json::from_str::<Value>("42").unwrap(),
            Value::Number(Number::from(42u64))
        );
        assert_eq!(
            serde_json::from_str::<Value>("-1.5").unwrap(),
            Value::Number(Number::from_f64(-1.5).unwrap())
        );
        assert_eq!(
            serde_json::from_str::<Value>("\"hi\"").unwrap(),
            Value::String(String::from("hi"))
        );
    }

    #[test]
    fn round_trips_through_serde_json_both_ways() {
        let original = Value::Object({
            let mut m = Map::new();
            m.insert(
                String::from("nested"),
                Value::Array(alloc::vec![
                    Value::Number(Number::from(1u64)),
                    Value::Number(Number::from_f64(2.5).unwrap()),
                    Value::Null,
                ]),
            );
            m
        });
        let json = serde_json::to_string(&original).unwrap();
        let back: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }
}
