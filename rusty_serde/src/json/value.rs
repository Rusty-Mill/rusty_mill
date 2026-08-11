//! A `serde_json::Value`-equivalent: an owned, dynamically-typed JSON tree.
//! Useful for working with JSON whose shape isn't known ahead of time, and
//! it's also what backs internally-tagged/untagged enum support - both need
//! to look at a value's shape before committing to how the rest of it
//! deserializes.
//!
//! `Value`'s `Serialize`/`Deserialize` impls are written against the
//! generic data model (`Serializer`/`Deserializer`, not anything
//! JSON-specific), so despite living in the `json` module for
//! discoverability, it works with any format - `Value::deserialize` on our
//! JSON `Deserializer` walks the live token stream directly via the usual
//! `visit_seq`/`visit_map` machinery, no separate parser required.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::ops::Index;

use crate::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use crate::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

/// A dynamically-typed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// Any JSON number that parsed as a negative integer.
    Int(i64),
    /// Any JSON number that parsed as a non-negative integer.
    UInt(u64),
    /// Any JSON number with a fractional part or exponent.
    Float(f64),
    String(String),
    Seq(Vec<Value>),
    /// Object entries in the order they appeared on the wire (or were
    /// inserted); look up by key with [`Value::get`] or `value["key"]`.
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::UInt(v) => i64::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::UInt(v) => Some(*v),
            Value::Int(v) => u64::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            Value::UInt(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&[Value]> {
        match self {
            Value::Seq(items) => Some(items),
            _ => None,
        }
    }

    /// Looks up a key in a `Value::Map` (linear scan - `Value` keeps
    /// entries in wire order rather than paying for a hash/tree index that
    /// most JSON blobs are too small to need). Returns `None` for any
    /// non-`Map` value, same as a missing key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

impl Index<&str> for Value {
    type Output = Value;

    /// Indexing a non-`Map` value or a missing key returns `&Value::Null`
    /// (matching `serde_json::Value`'s behavior) rather than panicking.
    fn index(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.get(key).unwrap_or(&NULL)
    }
}

impl Index<usize> for Value {
    type Output = Value;

    /// Indexing a non-`Seq` value or an out-of-range index returns
    /// `&Value::Null` rather than panicking.
    fn index(&self, index: usize) -> &Value {
        static NULL: Value = Value::Null;
        match self {
            Value::Seq(items) => items.get(index).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
}

impl fmt::Display for Value {
    /// Renders as compact JSON - the same output `json::to_string` would
    /// produce for this value.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::UInt(v) => write!(f, "{v}"),
            Value::Float(v) => {
                if v.fract() == 0.0 && v.is_finite() {
                    write!(f, "{v}.0")
                } else {
                    write!(f, "{v}")
                }
            }
            Value::String(s) => {
                f.write_str("\"")?;
                for c in s.chars() {
                    match c {
                        '"' => f.write_str("\\\"")?,
                        '\\' => f.write_str("\\\\")?,
                        '\n' => f.write_str("\\n")?,
                        '\r' => f.write_str("\\r")?,
                        '\t' => f.write_str("\\t")?,
                        c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
                        c => write!(f, "{c}")?,
                    }
                }
                f.write_str("\"")
            }
            Value::Seq(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{}:{v}", Value::String(k.clone()))?;
                }
                f.write_str("}")
            }
        }
    }
}

macro_rules! from_int {
    ($($ty:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$ty> for Value {
                fn from(v: $ty) -> Self {
                    Value::$variant(v.into())
                }
            }
        )*
    };
}

from_int! {
    i8 => Int, i16 => Int, i32 => Int, i64 => Int,
    u8 => UInt, u16 => UInt, u32 => UInt, u64 => UInt,
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v as f64)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Seq(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}
impl<T: Into<Value>> From<BTreeMap<String, T>> for Value {
    fn from(v: BTreeMap<String, T>) -> Self {
        Value::Map(v.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}
impl<T: Into<Value>> From<HashMap<String, T>> for Value {
    fn from(v: HashMap<String, T>) -> Self {
        Value::Map(v.into_iter().map(|(k, v)| (k, v.into())).collect())
    }
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Int(v) => serializer.serialize_i64(*v),
            Value::UInt(v) => serializer.serialize_u64(*v),
            Value::Float(v) => serializer.serialize_f64(*v),
            Value::String(s) => serializer.serialize_str(s),
            Value::Seq(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Map(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ValueVisitor;

        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "any JSON value")
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Int(v))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(Value::UInt(v))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(Value::Float(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_string()))
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
                Value::deserialize(deserializer)
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(item) = seq.next_element::<Value>()? {
                    items.push(item);
                }
                Ok(Value::Seq(items))
            }
            fn visit_map<A>(self, mut map: A) -> Result<Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry::<String, Value>()? {
                    entries.push(entry);
                }
                Ok(Value::Map(entries))
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}
