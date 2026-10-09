//! Small helpers shared by the codecs: build objects without nulls, and read
//! fields with errors that name what was being decoded.

use crate::{Error, Result};
use rusty_json::Value;

/// An object under construction; absent optional fields are left out, which
/// is what the reference encoders do.
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

    pub(crate) fn finish(self) -> Value {
        self.0
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

pub(crate) fn opt_value(v: &Value, name: &str) -> Option<Value> {
    v.get(name).filter(|f| !f.is_null()).cloned()
}

pub(crate) fn opt_array<'a>(
    v: &'a Value,
    what: &'static str,
    name: &'static str,
) -> Result<&'a [Value]> {
    match v.get(name) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) => Ok(items.as_slice()),
        Some(_) => Err(Error::decode(what, format!("{name:?} is not an array"))),
    }
}

pub(crate) fn expect_object<'a>(v: &'a Value, what: &'static str) -> Result<&'a Value> {
    if v.is_object() {
        Ok(v)
    } else {
        Err(Error::decode(what, "not an object"))
    }
}
