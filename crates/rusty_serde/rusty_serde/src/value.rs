//! A `serde_json::Value`-equivalent: an owned, dynamically-typed value
//! tree. Useful on its own for working with input whose shape isn't known
//! ahead of time, and it's also what backs internally-tagged/untagged enum
//! support (see [`ValueDeserializer`]) - both need to look at a value's
//! shape before committing to how the rest of it deserializes, which means
//! buffering it into memory first.
//!
//! Both halves - [`Value`]'s own `Serialize`/`Deserialize` impls, and its
//! use as a buffer for re-driving `Deserialize` a second time - are
//! written against the generic data model (`Serializer`/`Deserializer`),
//! not anything JSON-specific, even though it's mostly reached for as a
//! JSON value today (`rusty_serde::json::Value` is a re-export of the same
//! type defined here). Any format can buffer into a `Value` and hand it
//! back out through [`ValueDeserializer`] without either side needing to
//! know which format produced the original input.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::marker::PhantomData;
use std::ops::Index;

use crate::de::{
    Deserialize, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor,
};
use crate::error::Error as ErrorTrait;
use crate::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

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

    /// Inserts a key/value entry into a `Value::Map`, in place. Overwrites
    /// and returns the previous value if `key` was already present
    /// (matching `HashMap::insert`/`serde_json::Map::insert`'s
    /// convention), otherwise appends a new entry and returns `None`.
    ///
    /// A no-op returning `None` on any non-`Map` value (including
    /// `Value::Null`, so this doesn't double as a way to turn a fresh
    /// `Value::default()`-less value into a map - construct
    /// `Value::Map(Vec::new())` explicitly first) - same "acts like an
    /// empty map rather than panicking" convention [`Value::get`] and the
    /// `Index` impls already use for a shape mismatch.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        let Value::Map(entries) = self else {
            return None;
        };
        let key = key.into();
        let value = value.into();
        match entries.iter_mut().find(|(k, _)| *k == key) {
            Some(entry) => Some(std::mem::replace(&mut entry.1, value)),
            None => {
                entries.push((key, value));
                None
            }
        }
    }

    /// Removes and returns a `Value::Map` entry by key, if present. A
    /// no-op returning `None` on any non-`Map` value or a missing key.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let Value::Map(entries) = self else {
            return None;
        };
        let pos = entries.iter().position(|(k, _)| k == key)?;
        Some(entries.remove(pos).1)
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

// ---- Re-driving `Deserialize` against an already-buffered `Value` ----
//
// A `Deserializer`'s associated `Error` type has to be concrete per impl
// (Rust doesn't allow an impl's only free type parameter to appear solely
// in an associated-item binding - E0207), so `Value` itself can't directly
// implement `Deserializer` for every possible caller's error type at once.
// `ValueDeserializer<E>` carries that `E` explicitly instead; callers pick
// it via `ValueDeserializer::<D::Error>::new(value)`, so the buffered
// value can be re-deserialized using whatever error type the surrounding
// `Deserialize` impl already needs to produce.

/// A [`Deserializer`] over an already-buffered [`Value`] rather than live
/// input. Used to give a `Deserialize` impl a *second* look at a value it
/// (or a sibling) already consumed once - untagged enums try each variant
/// against a clone of the same buffered value in turn, and internally
/// tagged enums use the same mechanism for the fields alongside the tag.
pub struct ValueDeserializer<E> {
    value: Value,
    _marker: PhantomData<E>,
}

impl<E> ValueDeserializer<E> {
    pub fn new(value: Value) -> Self {
        ValueDeserializer {
            value,
            _marker: PhantomData,
        }
    }
}

impl<'de, E> Deserializer<'de> for ValueDeserializer<E>
where
    E: ErrorTrait,
{
    type Error = E;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(b),
            Value::Int(v) => visitor.visit_i64(v),
            Value::UInt(v) => visitor.visit_u64(v),
            Value::Float(v) => visitor.visit_f64(v),
            Value::String(s) => visitor.visit_string(s),
            Value::Seq(items) => visitor.visit_seq(ValueSeqAccess::new(items)),
            Value::Map(entries) => visitor.visit_map(ValueMapAccess::new(entries)),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Null => visitor.visit_none(),
            other => visitor.visit_some(ValueDeserializer::<E>::new(other)),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::String(s) => visitor.visit_str(&s),
            _ => Err(E::custom("expected a string identifier")),
        }
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::String(s) => visitor.visit_enum(StrDeserializer::<E>::new(&s)),
            Value::Map(mut entries) if entries.len() == 1 => {
                let (variant, value) = entries.remove(0);
                visitor.visit_enum(ValueTaggedEnumAccess::<E>::new(variant, value))
            }
            _ => Err(E::custom("expected string or single-entry object for enum")),
        }
    }

    crate::forward_to_deserialize_any! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_f32 deserialize_f64 deserialize_char deserialize_str deserialize_string
        deserialize_bytes deserialize_byte_buf deserialize_unit
        deserialize_unit_struct deserialize_newtype_struct deserialize_seq deserialize_tuple
        deserialize_tuple_struct deserialize_map deserialize_struct
        deserialize_ignored_any
    }
}

struct ValueSeqAccess<E> {
    items: std::vec::IntoIter<Value>,
    _marker: PhantomData<E>,
}

impl<E> ValueSeqAccess<E> {
    fn new(items: Vec<Value>) -> Self {
        ValueSeqAccess {
            items: items.into_iter(),
            _marker: PhantomData,
        }
    }
}

impl<'de, E: ErrorTrait> SeqAccess<'de> for ValueSeqAccess<E> {
    type Error = E;

    fn next_element<T>(&mut self) -> Result<Option<T>, E>
    where
        T: Deserialize<'de>,
    {
        match self.items.next() {
            Some(v) => T::deserialize(ValueDeserializer::<E>::new(v)).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

/// `pub(crate)` (rather than private) so a format's own hand-written
/// `Deserializer` methods - like internally-tagged enum support, which
/// still has to parse the tag key itself before anything reaches `Value`
/// machinery - can build one directly from their own already-collected
/// entries, instead of needing to round-trip through a `Value::Map` first.
pub(crate) struct ValueMapAccess<E> {
    entries: std::vec::IntoIter<(String, Value)>,
    pending_value: Option<Value>,
    _marker: PhantomData<E>,
}

impl<E> ValueMapAccess<E> {
    pub(crate) fn new(entries: Vec<(String, Value)>) -> Self {
        ValueMapAccess {
            entries: entries.into_iter(),
            pending_value: None,
            _marker: PhantomData,
        }
    }
}

impl<'de, E: ErrorTrait> MapAccess<'de> for ValueMapAccess<E> {
    type Error = E;

    fn next_key<K>(&mut self) -> Result<Option<K>, E>
    where
        K: Deserialize<'de>,
    {
        match self.entries.next() {
            Some((k, v)) => {
                self.pending_value = Some(v);
                K::deserialize(StrDeserializer::<E>::new(&k)).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value<V>(&mut self) -> Result<V, E>
    where
        V: Deserialize<'de>,
    {
        let value = self
            .pending_value
            .take()
            .expect("next_value called without a preceding next_key");
        V::deserialize(ValueDeserializer::<E>::new(value))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

/// `EnumAccess`/`VariantAccess` for an ordinary externally-tagged enum
/// found *inside* a buffered `Value` (e.g. a field's value, nested inside
/// an internally-tagged enum's own fields, or a variant being tried by an
/// untagged enum).
struct ValueTaggedEnumAccess<E> {
    variant: String,
    value: Value,
    _marker: PhantomData<E>,
}

impl<E> ValueTaggedEnumAccess<E> {
    fn new(variant: String, value: Value) -> Self {
        ValueTaggedEnumAccess {
            variant,
            value,
            _marker: PhantomData,
        }
    }
}

impl<'de, E: ErrorTrait> EnumAccess<'de> for ValueTaggedEnumAccess<E> {
    type Error = E;
    type Variant = ValueTaggedVariantAccess<E>;

    fn variant<V>(self) -> Result<(V, Self::Variant), E>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(StrDeserializer::<E>::new(&self.variant))?;
        Ok((value, ValueTaggedVariantAccess::<E>::new(self.value)))
    }
}

struct ValueTaggedVariantAccess<E> {
    value: Value,
    _marker: PhantomData<E>,
}

impl<E> ValueTaggedVariantAccess<E> {
    fn new(value: Value) -> Self {
        ValueTaggedVariantAccess {
            value,
            _marker: PhantomData,
        }
    }
}

impl<'de, E: ErrorTrait> VariantAccess<'de> for ValueTaggedVariantAccess<E> {
    type Error = E;

    fn unit_variant(self) -> Result<(), E> {
        Ok(())
    }
    fn newtype_variant<T>(self) -> Result<T, E>
    where
        T: Deserialize<'de>,
    {
        T::deserialize(ValueDeserializer::<E>::new(self.value))
    }
    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Seq(items) => visitor.visit_seq(ValueSeqAccess::<E>::new(items)),
            _ => Err(E::custom("expected an array for a tuple variant")),
        }
    }
    fn struct_variant<V>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Map(entries) => visitor.visit_map(ValueMapAccess::<E>::new(entries)),
            _ => Err(E::custom("expected an object for a struct variant")),
        }
    }
}

/// Deserializes a single string as an identifier/variant tag - the
/// generic-`E` counterpart to what each format's own `deserialize_str`
/// would do, used when a [`Value`] has already reduced the input to a bare
/// `String` (an object key, an enum's tag value, ...).
struct StrDeserializer<'a, E> {
    value: &'a str,
    _marker: PhantomData<E>,
}

impl<'a, E> StrDeserializer<'a, E> {
    fn new(value: &'a str) -> Self {
        StrDeserializer {
            value,
            _marker: PhantomData,
        }
    }
}

impl<'de, 'a, E: ErrorTrait> Deserializer<'de> for StrDeserializer<'a, E> {
    type Error = E;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.value)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.value)
    }

    crate::forward_to_deserialize_any! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_f32 deserialize_f64 deserialize_char deserialize_str deserialize_string
        deserialize_bytes deserialize_byte_buf deserialize_option deserialize_unit
        deserialize_unit_struct deserialize_newtype_struct deserialize_seq deserialize_tuple
        deserialize_tuple_struct deserialize_map deserialize_struct deserialize_enum
        deserialize_ignored_any
    }
}

/// `EnumAccess` for a bare string: the shape a unit variant of an
/// externally-tagged enum takes (just `"Variant"`, no wrapper object), so
/// its `Variant` type only ever accepts `unit_variant`.
impl<'de, 'a, E: ErrorTrait> EnumAccess<'de> for StrDeserializer<'a, E> {
    type Error = E;
    type Variant = UnitOnlyVariantAccess<'a, E>;

    fn variant<V>(self) -> Result<(V, Self::Variant), E>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(StrDeserializer::<E>::new(self.value))?;
        Ok((value, UnitOnlyVariantAccess::new(self.value)))
    }
}

struct UnitOnlyVariantAccess<'a, E> {
    name: &'a str,
    _marker: PhantomData<E>,
}

impl<'a, E> UnitOnlyVariantAccess<'a, E> {
    fn new(name: &'a str) -> Self {
        UnitOnlyVariantAccess {
            name,
            _marker: PhantomData,
        }
    }
}

impl<'de, 'a, E: ErrorTrait> VariantAccess<'de> for UnitOnlyVariantAccess<'a, E> {
    type Error = E;

    fn unit_variant(self) -> Result<(), E> {
        Ok(())
    }
    fn newtype_variant<T>(self) -> Result<T, E>
    where
        T: Deserialize<'de>,
    {
        Err(E::custom(format!(
            "expected newtype variant, found unit variant `{}`",
            self.name
        )))
    }
    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        Err(E::custom(format!(
            "expected tuple variant, found unit variant `{}`",
            self.name
        )))
    }
    fn struct_variant<V>(self, _fields: &'static [&'static str], _visitor: V) -> Result<V::Value, E>
    where
        V: Visitor<'de>,
    {
        Err(E::custom(format!(
            "expected struct variant, found unit variant `{}`",
            self.name
        )))
    }
}

// ---- Building a `Value` directly from a `Serialize` value ----
//
// The serialize-side complement of `ValueDeserializer` above: instead of
// re-driving `Deserialize` against an already-buffered `Value`, this drives
// `Serialize` and buffers *into* one, without going through any wire
// format's text representation. Generic over the error type for the same
// reason `ValueDeserializer` is - a `Serializer` impl's `Error` type has to
// be concrete, so callers pick it via `ValueSerializer::<D::Error>::new()`.
//
// Enum variants are represented exactly as this crate's JSON format writes
// them on the wire (see `crate::json::ser`): a unit variant is a bare
// `Value::String`, and newtype/tuple/struct variants are a single-entry
// `Value::Map` keyed by the variant name. `#[rusty_serde(tag/content/untagged)]`
// need no special handling here - the derive macro already expresses those
// entirely in terms of `serialize_map`/`serialize_entry`, which this
// `Serializer` impl provides like any other.

/// Converts any [`Serialize`] value straight into a [`Value`] tree, without
/// going through a wire format's text representation. The
/// [`crate::json::to_value`] wrapper is what most callers want; this free
/// function exists for formats other than JSON to reuse the same machinery,
/// the same way [`ValueDeserializer`] is reused by [`crate::json::de`]'s
/// internally-tagged enum support.
pub fn to_value<E, T>(value: &T) -> Result<Value, E>
where
    E: ErrorTrait,
    T: Serialize + ?Sized,
{
    value.serialize(ValueSerializer::<E>::new())
}

/// Deserializes an already-buffered [`Value`] into a `T`. The
/// [`crate::json::from_value`] wrapper is what most callers want; this free
/// function exists for formats other than JSON to reuse the same machinery.
pub fn from_value<E, T>(value: Value) -> Result<T, E>
where
    E: ErrorTrait,
    T: for<'de> Deserialize<'de>,
{
    T::deserialize(ValueDeserializer::<E>::new(value))
}

/// A [`Serializer`] that builds a [`Value`] rather than emitting
/// wire-format output.
pub struct ValueSerializer<E> {
    _marker: PhantomData<E>,
}

impl<E> ValueSerializer<E> {
    pub fn new() -> Self {
        ValueSerializer {
            _marker: PhantomData,
        }
    }
}

impl<E> Default for ValueSerializer<E> {
    fn default() -> Self {
        ValueSerializer::new()
    }
}

impl<E: ErrorTrait> Serializer for ValueSerializer<E> {
    type Ok = Value;
    type Error = E;

    type SerializeSeq = ValueSeqBuilder<E>;
    type SerializeTuple = ValueSeqBuilder<E>;
    type SerializeTupleStruct = ValueSeqBuilder<E>;
    type SerializeTupleVariant = ValueVariantSeqBuilder<E>;
    type SerializeMap = ValueMapBuilder<E>;
    type SerializeStruct = ValueMapBuilder<E>;
    type SerializeStructVariant = ValueVariantMapBuilder<E>;

    fn serialize_bool(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn serialize_i64(self, v: i64) -> Result<Value, E> {
        Ok(Value::Int(v))
    }
    fn serialize_u64(self, v: u64) -> Result<Value, E> {
        Ok(Value::UInt(v))
    }
    fn serialize_f64(self, v: f64) -> Result<Value, E> {
        Ok(Value::Float(v))
    }
    fn serialize_str(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Value, E> {
        Ok(Value::Seq(
            v.iter().map(|&b| Value::UInt(b as u64)).collect(),
        ))
    }

    fn serialize_none(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn serialize_some<T>(self, value: &T) -> Result<Value, E>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value, E> {
        Ok(Value::String(variant.to_string()))
    }
    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value, E>
    where
        T: Serialize + ?Sized,
    {
        let inner = value.serialize(ValueSerializer::<E>::new())?;
        Ok(Value::Map(vec![(variant.to_string(), inner)]))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<ValueSeqBuilder<E>, E> {
        Ok(ValueSeqBuilder::new())
    }
    fn serialize_tuple(self, len: usize) -> Result<ValueSeqBuilder<E>, E> {
        Serializer::serialize_seq(self, Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<ValueSeqBuilder<E>, E> {
        Serializer::serialize_seq(self, Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<ValueVariantSeqBuilder<E>, E> {
        Ok(ValueVariantSeqBuilder::new(variant))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<ValueMapBuilder<E>, E> {
        Ok(ValueMapBuilder::new())
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<ValueMapBuilder<E>, E> {
        Ok(ValueMapBuilder::new())
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<ValueVariantMapBuilder<E>, E> {
        Ok(ValueVariantMapBuilder::new(variant))
    }
}

/// Shared by `SerializeSeq`/`SerializeTuple`/`SerializeTupleStruct` - all
/// three just collect elements into a `Value::Seq`, with no wrapper needed.
pub struct ValueSeqBuilder<E> {
    items: Vec<Value>,
    _marker: PhantomData<E>,
}

impl<E> ValueSeqBuilder<E> {
    fn new() -> Self {
        ValueSeqBuilder {
            items: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<E: ErrorTrait> SerializeSeq for ValueSeqBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_element<T>(&mut self, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        self.items
            .push(value.serialize(ValueSerializer::<E>::new())?);
        Ok(())
    }
    fn end(self) -> Result<Value, E> {
        Ok(Value::Seq(self.items))
    }
}

impl<E: ErrorTrait> SerializeTuple for ValueSeqBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_element<T>(&mut self, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<Value, E> {
        SerializeSeq::end(self)
    }
}

impl<E: ErrorTrait> SerializeTupleStruct for ValueSeqBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_field<T>(&mut self, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<Value, E> {
        SerializeSeq::end(self)
    }
}

/// A tuple variant's elements, wrapped in a single-entry `{variant: [...]}`
/// map on `end()` to match how this crate's JSON format writes tuple
/// variants on the wire.
pub struct ValueVariantSeqBuilder<E> {
    variant: &'static str,
    items: Vec<Value>,
    _marker: PhantomData<E>,
}

impl<E> ValueVariantSeqBuilder<E> {
    fn new(variant: &'static str) -> Self {
        ValueVariantSeqBuilder {
            variant,
            items: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<E: ErrorTrait> SerializeTupleVariant for ValueVariantSeqBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_field<T>(&mut self, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        self.items
            .push(value.serialize(ValueSerializer::<E>::new())?);
        Ok(())
    }
    fn end(self) -> Result<Value, E> {
        Ok(Value::Map(vec![(
            self.variant.to_string(),
            Value::Seq(self.items),
        )]))
    }
}

/// Shared by `SerializeMap`/`SerializeStruct` - both just collect entries
/// into a `Value::Map`, with no wrapper needed.
pub struct ValueMapBuilder<E> {
    entries: Vec<(String, Value)>,
    pending_key: Option<String>,
    _marker: PhantomData<E>,
}

impl<E> ValueMapBuilder<E> {
    fn new() -> Self {
        ValueMapBuilder {
            entries: Vec::new(),
            pending_key: None,
            _marker: PhantomData,
        }
    }
}

impl<E: ErrorTrait> SerializeMap for ValueMapBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_key<T>(&mut self, key: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        let key_value = key.serialize(ValueSerializer::<E>::new())?;
        self.pending_key = Some(value_to_map_key(key_value)?);
        Ok(())
    }
    fn serialize_value<T>(&mut self, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        let key = self
            .pending_key
            .take()
            .expect("serialize_value called without a preceding serialize_key");
        let value = value.serialize(ValueSerializer::<E>::new())?;
        self.entries.push((key, value));
        Ok(())
    }
    fn end(self) -> Result<Value, E> {
        Ok(Value::Map(self.entries))
    }
}

impl<E: ErrorTrait> SerializeStruct for ValueMapBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        let value = value.serialize(ValueSerializer::<E>::new())?;
        self.entries.push((key.to_string(), value));
        Ok(())
    }
    fn end(self) -> Result<Value, E> {
        Ok(Value::Map(self.entries))
    }
}

/// A struct variant's fields, wrapped in a single-entry `{variant: {...}}`
/// map on `end()` to match how this crate's JSON format writes struct
/// variants on the wire.
pub struct ValueVariantMapBuilder<E> {
    variant: &'static str,
    entries: Vec<(String, Value)>,
    _marker: PhantomData<E>,
}

impl<E> ValueVariantMapBuilder<E> {
    fn new(variant: &'static str) -> Self {
        ValueVariantMapBuilder {
            variant,
            entries: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<E: ErrorTrait> SerializeStructVariant for ValueVariantMapBuilder<E> {
    type Ok = Value;
    type Error = E;
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        let value = value.serialize(ValueSerializer::<E>::new())?;
        self.entries.push((key.to_string(), value));
        Ok(())
    }
    fn end(self) -> Result<Value, E> {
        Ok(Value::Map(vec![(
            self.variant.to_string(),
            Value::Map(self.entries),
        )]))
    }
}

/// Coerces a serialized key [`Value`] into the `String` a [`Value::Map`]
/// entry needs. Only scalar shapes are accepted (matching how this crate's
/// JSON format's own `KeySerializer` restricts map keys) - a sequence or
/// map key has no sound string representation.
fn value_to_map_key<E: ErrorTrait>(value: Value) -> Result<String, E> {
    match value {
        Value::String(s) => Ok(s),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Int(v) => Ok(v.to_string()),
        Value::UInt(v) => Ok(v.to_string()),
        Value::Float(v) => Ok(v.to_string()),
        other => Err(E::custom(format!(
            "map keys must be strings, found {other}"
        ))),
    }
}
