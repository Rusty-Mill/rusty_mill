//! The [`Wire`] trait and the helpers every hand-written codec shares.
//! Encoding emits only the members the spec defines and leaves out `None`;
//! decoding ignores members it does not know, so a newer peer still parses.

use crate::{Error, Result};
use rusty_json::Value;

/// A type with a JSON wire form.
pub trait Wire: Sized {
    /// Encode to a JSON value.
    fn to_value(&self) -> Value;

    /// Decode from a JSON value.
    ///
    /// # Errors
    /// [`Error::Decode`] when a required member is missing or mistyped.
    fn from_value(v: &Value) -> Result<Self>;

    /// Encode to compact JSON text.
    fn to_json(&self) -> String {
        self.to_value().to_json_string()
    }

    /// Decode from JSON text.
    ///
    /// # Errors
    /// [`Error::Json`] when `s` is not JSON, else as [`Wire::from_value`].
    fn from_json(s: &str) -> Result<Self> {
        Self::from_value(&Value::from_json_str(s)?)
    }
}

/// An object under construction; `None` members are left out.
pub(crate) struct Obj(Value);

impl Obj {
    pub(crate) fn new() -> Self {
        Obj(Value::object())
    }

    pub(crate) fn set(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key, value);
        self
    }

    pub(crate) fn opt(self, key: &str, value: Option<impl Into<Value>>) -> Self {
        match value {
            Some(v) => self.set(key, v),
            None => self,
        }
    }

    pub(crate) fn opt_value(self, key: &str, value: &Option<Value>) -> Self {
        match value {
            Some(v) => self.set(key, v.clone()),
            None => self,
        }
    }

    pub(crate) fn done(self) -> Value {
        self.0
    }
}

pub(crate) fn object<'a>(v: &'a Value, what: &'static str) -> Result<&'a Value> {
    if v.is_object() {
        Ok(v)
    } else {
        Err(Error::decode(what, "not an object"))
    }
}

pub(crate) fn field<'a>(v: &'a Value, what: &'static str, name: &'static str) -> Result<&'a Value> {
    v.get(name)
        .filter(|f| !f.is_null())
        .ok_or_else(|| Error::decode(what, format!("missing {name:?}")))
}

pub(crate) fn string(v: &Value, what: &'static str, name: &'static str) -> Result<String> {
    field(v, what, name)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| Error::decode(what, format!("{name:?} is not a string")))
}

pub(crate) fn opt_string(
    v: &Value,
    what: &'static str,
    name: &'static str,
) -> Result<Option<String>> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Error::decode(what, format!("{name:?} is not a string"))),
    }
}

pub(crate) fn opt_bool(v: &Value, what: &'static str, name: &'static str) -> Result<Option<bool>> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(Error::decode(what, format!("{name:?} is not a boolean"))),
    }
}

pub(crate) fn opt_u64(v: &Value, what: &'static str, name: &'static str) -> Result<Option<u64>> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| Error::decode(what, format!("{name:?} is not an unsigned integer"))),
    }
}

/// A member kept as raw JSON (annotations, icons, `_meta`, schemas).
pub(crate) fn opt_value(v: &Value, name: &str) -> Option<Value> {
    v.get(name).filter(|f| !f.is_null()).cloned()
}

pub(crate) fn array<'a>(
    v: &'a Value,
    what: &'static str,
    name: &'static str,
) -> Result<&'a [Value]> {
    field(v, what, name)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| Error::decode(what, format!("{name:?} is not an array")))
}

pub(crate) fn opt_array<'a>(
    v: &'a Value,
    what: &'static str,
    name: &'static str,
) -> Result<&'a [Value]> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(&[]),
        Some(_) => array(v, what, name),
    }
}

pub(crate) fn decode_all<T: Wire>(items: &[Value]) -> Result<Vec<T>> {
    items.iter().map(T::from_value).collect()
}

pub(crate) fn encode_all<T: Wire>(items: &[T]) -> Value {
    Value::Array(items.iter().map(Wire::to_value).collect())
}
