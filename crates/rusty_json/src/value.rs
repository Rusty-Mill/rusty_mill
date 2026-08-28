use crate::{Map, Number};
use alloc::string::String;
use alloc::vec::Vec;

/// A JSON value: one of the six kinds defined by
/// [RFC 8259 §3](https://www.rfc-editor.org/rfc/rfc8259#section-3).
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// The JSON `null` literal.
    Null,
    /// A JSON boolean, `true` or `false`.
    Bool(bool),
    /// A JSON number.
    Number(Number),
    /// A JSON string.
    String(String),
    /// A JSON array.
    Array(Vec<Value>),
    /// A JSON object.
    Object(Map),
}

impl Default for Value {
    /// Returns `Value::Null`.
    fn default() -> Self {
        Value::Null
    }
}

impl Value {
    /// Looks up a key if this is an object, returning `None` otherwise
    /// (including when the key is absent).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    /// Looks up an index if this is an array, returning `None` otherwise
    /// (including when the index is out of bounds).
    pub fn get_index(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Array(arr) => arr.get(index),
            _ => None,
        }
    }

    /// Mutably looks up a key if this is an object, returning `None`
    /// otherwise (including when the key is absent).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Object(map) => map.get_mut(key),
            _ => None,
        }
    }

    /// True if this is `Value::Null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// True if this is `Value::Bool`.
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    /// True if this is `Value::Number`.
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    /// True if this is `Value::String`.
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    /// True if this is `Value::Array`.
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// True if this is a `Value::Number` constructed from an `i64` that
    /// fits without loss. See [`Number::is_i64`].
    pub fn is_i64(&self) -> bool {
        matches!(self, Value::Number(n) if n.is_i64())
    }

    /// True if this is a `Value::Number` constructed from a `u64` that
    /// fits without loss. See [`Number::is_u64`].
    pub fn is_u64(&self) -> bool {
        matches!(self, Value::Number(n) if n.is_u64())
    }

    /// True if this is a `Value::Number` constructed from a float. See
    /// [`Number::is_f64`].
    pub fn is_f64(&self) -> bool {
        matches!(self, Value::Number(n) if n.is_f64())
    }

    /// True if this is `Value::Object`.
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    /// Returns `Some(())` if this is `Value::Null`, else `None`.
    pub fn as_null(&self) -> Option<()> {
        match self {
            Value::Null => Some(()),
            _ => None,
        }
    }

    /// Returns the inner `bool`, if this is `Value::Bool`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the inner string slice, if this is `Value::String`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Returns the inner array, if this is `Value::Array`.
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Returns the inner object, if this is `Value::Object`.
    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// Returns the inner array mutably, if this is `Value::Array`.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Returns the inner object mutably, if this is `Value::Object`.
    pub fn as_object_mut(&mut self) -> Option<&mut Map> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// Returns the number as an `i64`, if this is `Value::Number` and it
    /// fits without loss. See [`Number::as_i64`].
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(n) => n.as_i64(),
            _ => None,
        }
    }

    /// Returns the number as a `u64`, if this is `Value::Number` and it
    /// fits without loss. See [`Number::as_u64`].
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) => n.as_u64(),
            _ => None,
        }
    }

    /// Returns the number as an `f64`, if this is `Value::Number`. See
    /// [`Number::as_f64`].
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(n.as_f64()),
            _ => None,
        }
    }

    /// Replaces `self` with `Value::Null`, returning the previous value.
    pub fn take(&mut self) -> Value {
        core::mem::take(self)
    }

    /// Looks up a value by [RFC 6901](https://www.rfc-editor.org/rfc/rfc6901)
    /// JSON Pointer, e.g. `value.pointer("/a/0/b")`. An empty pointer
    /// returns `self`; a non-empty one that doesn't start with `/`, or
    /// that doesn't resolve (missing key, out-of-bounds/non-numeric array
    /// index, or a step through a scalar), returns `None`.
    pub fn pointer(&self, pointer: &str) -> Option<&Value> {
        if pointer.is_empty() {
            return Some(self);
        }
        let rest = pointer.strip_prefix('/')?;
        let mut target = self;
        for token in rest.split('/') {
            let token = unescape_pointer_token(token);
            target = match target {
                Value::Object(map) => map.get(token.as_str())?,
                Value::Array(arr) => arr.get(token.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(target)
    }

    /// Mutable counterpart to [`Value::pointer`].
    pub fn pointer_mut(&mut self, pointer: &str) -> Option<&mut Value> {
        if pointer.is_empty() {
            return Some(self);
        }
        let rest = pointer.strip_prefix('/')?;
        let mut target = self;
        for token in rest.split('/') {
            let token = unescape_pointer_token(token);
            target = match target {
                Value::Object(map) => map.get_mut(token.as_str())?,
                Value::Array(arr) => arr.get_mut(token.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(target)
    }
}

/// Un-escapes a single JSON Pointer reference token: `~1` -> `/`, `~0` -> `~`
/// (in that order, per RFC 6901).
fn unescape_pointer_token(token: &str) -> String {
    if token.contains('~') {
        token.replace("~1", "/").replace("~0", "~")
    } else {
        String::from(token)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

macro_rules! impl_from_integer {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for Value {
                fn from(n: $ty) -> Self {
                    Value::Number(Number::from(n))
                }
            }
        )*
    };
}

impl_from_integer!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl From<f32> for Value {
    /// `NaN` and infinities become `Value::Null`, since JSON has no way to
    /// represent them (mirrors `serde_json::Value`'s `From<f32>`).
    fn from(f: f32) -> Self {
        Number::from_f64(f64::from(f)).map_or(Value::Null, Value::Number)
    }
}

impl From<f64> for Value {
    /// `NaN` and infinities become `Value::Null`, since JSON has no way to
    /// represent them (mirrors `serde_json::Value`'s `From<f64>`).
    fn from(f: f64) -> Self {
        Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}

impl From<Number> for Value {
    fn from(n: Number) -> Self {
        Value::Number(n)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(String::from(s))
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(v)
    }
}

impl From<Map> for Value {
    fn from(m: Map) -> Self {
        Value::Object(m)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(opt: Option<T>) -> Self {
        opt.map_or(Value::Null, Into::into)
    }
}

impl FromIterator<(String, Value)> for Value {
    /// Collects key/value pairs into a `Value::Object`.
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Value::Object(iter.into_iter().collect())
    }
}

impl FromIterator<Value> for Value {
    /// Collects values into a `Value::Array`.
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        Value::Array(iter.into_iter().collect())
    }
}

impl core::str::FromStr for Value {
    type Err = crate::Error;

    /// Parses a JSON value from a string slice; delegates to
    /// [`crate::from_str`].
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::from_str(s)
    }
}

macro_rules! impl_partial_eq_signed {
    ($($ty:ty),*) => {
        $(
            impl PartialEq<$ty> for Value {
                fn eq(&self, other: &$ty) -> bool {
                    self.as_i64() == Some(*other as i64)
                }
            }
            impl PartialEq<Value> for $ty {
                fn eq(&self, other: &Value) -> bool {
                    other == self
                }
            }
        )*
    };
}

impl_partial_eq_signed!(i8, i16, i32, i64, isize);

macro_rules! impl_partial_eq_unsigned {
    ($($ty:ty),*) => {
        $(
            impl PartialEq<$ty> for Value {
                fn eq(&self, other: &$ty) -> bool {
                    self.as_u64() == Some(*other as u64)
                }
            }
            impl PartialEq<Value> for $ty {
                fn eq(&self, other: &Value) -> bool {
                    other == self
                }
            }
        )*
    };
}

impl_partial_eq_unsigned!(u8, u16, u32, u64, usize);

impl PartialEq<f32> for Value {
    fn eq(&self, other: &f32) -> bool {
        self.as_f64() == Some(f64::from(*other))
    }
}

impl PartialEq<Value> for f32 {
    fn eq(&self, other: &Value) -> bool {
        other == self
    }
}

impl PartialEq<f64> for Value {
    fn eq(&self, other: &f64) -> bool {
        self.as_f64() == Some(*other)
    }
}

impl PartialEq<Value> for f64 {
    fn eq(&self, other: &Value) -> bool {
        other == self
    }
}

impl PartialEq<bool> for Value {
    fn eq(&self, other: &bool) -> bool {
        self.as_bool() == Some(*other)
    }
}

impl PartialEq<Value> for bool {
    fn eq(&self, other: &Value) -> bool {
        other == self
    }
}

impl PartialEq<str> for Value {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == Some(other)
    }
}

impl PartialEq<Value> for str {
    fn eq(&self, other: &Value) -> bool {
        other == self
    }
}

impl PartialEq<&str> for Value {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == Some(*other)
    }
}

impl PartialEq<Value> for &str {
    fn eq(&self, other: &Value) -> bool {
        other == self
    }
}

impl PartialEq<String> for Value {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == Some(other.as_str())
    }
}

impl PartialEq<Value> for String {
    fn eq(&self, other: &Value) -> bool {
        other == self
    }
}

impl core::ops::Index<&str> for Value {
    type Output = Value;

    /// Indexes into an object by key. Returns `&Value::Null` if this isn't
    /// an object or the key is absent, rather than panicking.
    fn index(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        self.get(key).unwrap_or(&NULL)
    }
}

impl core::ops::Index<usize> for Value {
    type Output = Value;

    /// Indexes into an array by position. Returns `&Value::Null` if this
    /// isn't an array or the index is out of bounds, rather than panicking.
    fn index(&self, index: usize) -> &Value {
        static NULL: Value = Value::Null;
        self.get_index(index).unwrap_or(&NULL)
    }
}

impl core::ops::IndexMut<&str> for Value {
    /// Mutably indexes into an object by key. If `self` is `Value::Null`,
    /// it's first promoted to an empty object; a missing key is inserted
    /// as `Value::Null`. Panics if `self` is any other non-object variant.
    fn index_mut(&mut self, key: &str) -> &mut Value {
        if self.is_null() {
            *self = Value::Object(Map::new());
        }
        match self {
            Value::Object(map) => map.entry(String::from(key)).or_insert(Value::Null),
            _ => panic!("cannot access key {key:?} of non-object Value"),
        }
    }
}

impl core::ops::IndexMut<usize> for Value {
    /// Mutably indexes into an array by position. Panics if `self` isn't
    /// an array, or if `index` is out of bounds.
    fn index_mut(&mut self, index: usize) -> &mut Value {
        match self {
            Value::Array(arr) => &mut arr[index],
            _ => panic!("cannot access index {index} of non-array Value"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_null() {
        assert_eq!(Value::default(), Value::Null);
    }

    #[test]
    fn variants_are_distinguishable() {
        assert_ne!(Value::Null, Value::Bool(false));
        assert_ne!(Value::Bool(true), Value::Bool(false));
    }

    #[test]
    fn is_predicates() {
        assert!(Value::Null.is_null());
        assert!(Value::Bool(true).is_bool());
        assert!(Value::Number(Number::from(1u64)).is_number());
        assert!(Value::String(String::from("x")).is_string());
        assert!(Value::Array(Vec::new()).is_array());
        assert!(Value::Object(Map::new()).is_object());
        assert!(Value::Number(Number::from(1u64)).is_i64());
        assert!(Value::Number(Number::from(1u64)).is_u64());
        assert!(!Value::Number(Number::from(1u64)).is_f64());
        assert!(Value::Number(Number::from_f64(1.0).unwrap()).is_f64());
        assert!(!Value::Null.is_i64());
        assert!(!Value::Null.is_bool());
    }

    #[test]
    fn as_null() {
        assert_eq!(Value::Null.as_null(), Some(()));
        assert_eq!(Value::Bool(false).as_null(), None);
    }

    #[test]
    fn as_accessors_match_and_mismatch() {
        assert_eq!(Value::Bool(true).as_bool(), Some(true));
        assert_eq!(Value::Null.as_bool(), None);
        assert_eq!(Value::String(String::from("hi")).as_str(), Some("hi"));
        assert_eq!(Value::Null.as_str(), None);
        assert_eq!(Value::Number(Number::from(7u64)).as_i64(), Some(7));
        assert_eq!(Value::Null.as_i64(), None);
        assert_eq!(
            Value::Number(Number::from_f64(1.5).unwrap()).as_f64(),
            Some(1.5)
        );
    }

    #[test]
    fn get_on_object_and_array() {
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Bool(true));
        let obj = Value::Object(map);
        assert_eq!(obj.get("a"), Some(&Value::Bool(true)));
        assert_eq!(obj.get("missing"), None);
        assert_eq!(obj.get_index(0), None);

        let arr = Value::Array(alloc::vec![Value::Bool(false)]);
        assert_eq!(arr.get_index(0), Some(&Value::Bool(false)));
        assert_eq!(arr.get_index(1), None);
        assert_eq!(arr.get("a"), None);
    }

    #[test]
    fn index_operator_returns_null_instead_of_panicking() {
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Bool(true));
        let obj = Value::Object(map);
        assert_eq!(obj["a"], Value::Bool(true));
        assert_eq!(obj["missing"], Value::Null);
        assert_eq!(obj[0], Value::Null);

        let arr = Value::Array(alloc::vec![Value::Bool(false)]);
        assert_eq!(arr[0], Value::Bool(false));
        assert_eq!(arr[5], Value::Null);
    }

    #[test]
    fn from_bool_and_integers() {
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from(42u32), Value::Number(Number::from(42u32)));
        assert_eq!(Value::from(-7i64), Value::Number(Number::from(-7i64)));
    }

    #[test]
    fn from_floats_maps_non_finite_to_null() {
        assert_eq!(
            Value::from(1.5f64),
            Value::Number(Number::from_f64(1.5).unwrap())
        );
        assert_eq!(Value::from(f64::NAN), Value::Null);
        assert_eq!(Value::from(f64::INFINITY), Value::Null);
        assert_eq!(Value::from(f32::NAN), Value::Null);
    }

    #[test]
    fn from_strings() {
        assert_eq!(Value::from("hi"), Value::String(String::from("hi")));
        assert_eq!(
            Value::from(String::from("hi")),
            Value::String(String::from("hi"))
        );
    }

    #[test]
    fn from_array_and_object() {
        let v: Value = alloc::vec![Value::Null].into();
        assert_eq!(v, Value::Array(alloc::vec![Value::Null]));

        let mut map = Map::new();
        map.insert(String::from("a"), Value::Null);
        let v: Value = map.clone().into();
        assert_eq!(v, Value::Object(map));
    }

    #[test]
    fn from_option() {
        let some: Value = Some(42u32).into();
        assert_eq!(some, Value::Number(Number::from(42u32)));
        let none: Value = Option::<u32>::None.into();
        assert_eq!(none, Value::Null);
    }

    #[test]
    fn take_replaces_with_null() {
        let mut v = Value::Bool(true);
        let taken = v.take();
        assert_eq!(taken, Value::Bool(true));
        assert_eq!(v, Value::Null);
    }

    #[test]
    fn mutable_accessors() {
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Bool(false));
        let mut obj = Value::Object(map);

        if let Some(v) = obj.get_mut("a") {
            *v = Value::Bool(true);
        }
        assert_eq!(obj.get("a"), Some(&Value::Bool(true)));
        assert_eq!(obj.get_mut("missing"), None);

        obj.as_object_mut()
            .unwrap()
            .insert(String::from("b"), Value::Null);
        assert_eq!(obj.get("b"), Some(&Value::Null));

        let mut arr = Value::Array(alloc::vec![Value::Bool(false)]);
        arr.as_array_mut().unwrap().push(Value::Null);
        assert_eq!(
            arr,
            Value::Array(alloc::vec![Value::Bool(false), Value::Null])
        );
        assert_eq!(Value::Null.as_array_mut(), None);
        assert_eq!(Value::Null.as_object_mut(), None);
    }

    #[test]
    fn partial_eq_against_primitives() {
        assert_eq!(Value::Bool(true), true);
        assert_eq!(true, Value::Bool(true));
        assert_ne!(Value::Bool(true), false);

        assert_eq!(Value::Number(Number::from(42i64)), 42i64);
        assert_eq!(42i64, Value::Number(Number::from(42i64)));
        assert_eq!(Value::Number(Number::from(42u64)), 42u64);
        assert_eq!(Value::Number(Number::from_f64(1.5).unwrap()), 1.5f64);

        assert_eq!(Value::String(String::from("hi")), "hi");
        assert_eq!("hi", Value::String(String::from("hi")));
        assert_eq!(Value::String(String::from("hi")), String::from("hi"));
        assert_eq!(String::from("hi"), Value::String(String::from("hi")));

        assert_ne!(Value::Null, true);
        assert_ne!(Value::Null, "hi");
        assert_ne!(Value::Null, 0i64);
    }

    #[test]
    fn index_mut_object_inserts_missing_key() {
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Bool(true));
        let mut obj = Value::Object(map);
        obj["a"] = Value::Bool(false);
        assert_eq!(obj["a"], Value::Bool(false));
        obj["b"] = Value::Number(Number::from(1u64));
        assert_eq!(obj["b"], Value::Number(Number::from(1u64)));
    }

    #[test]
    fn index_mut_promotes_null_to_object() {
        let mut v = Value::Null;
        v["a"] = Value::Bool(true);
        assert_eq!(v, {
            let mut m = Map::new();
            m.insert(String::from("a"), Value::Bool(true));
            Value::Object(m)
        });
    }

    #[test]
    #[should_panic(expected = "non-object")]
    fn index_mut_str_panics_on_non_object_non_null() {
        let mut v = Value::Bool(true);
        v["a"] = Value::Null;
    }

    #[test]
    fn index_mut_array() {
        let mut arr = Value::Array(alloc::vec![Value::Bool(false)]);
        arr[0] = Value::Bool(true);
        assert_eq!(arr[0], Value::Bool(true));
    }

    #[test]
    #[should_panic(expected = "non-array")]
    fn index_mut_usize_panics_on_non_array() {
        let mut v = Value::Null;
        v[0] = Value::Null;
    }

    #[test]
    #[should_panic]
    fn index_mut_usize_panics_out_of_bounds() {
        let mut arr = Value::Array(Vec::new());
        arr[0] = Value::Null;
    }

    #[test]
    fn pointer_empty_returns_self() {
        let v = Value::Bool(true);
        assert_eq!(v.pointer(""), Some(&v));
    }

    #[test]
    fn pointer_walks_object_and_array() {
        let mut inner = Map::new();
        inner.insert(String::from("b"), Value::Bool(true));
        let mut outer = Map::new();
        outer.insert(
            String::from("a"),
            Value::Array(alloc::vec![Value::Object(inner)]),
        );
        let v = Value::Object(outer);

        assert_eq!(v.pointer("/a/0/b"), Some(&Value::Bool(true)));
        assert_eq!(v.pointer("/a/0/missing"), None);
        assert_eq!(v.pointer("/a/5"), None);
        assert_eq!(v.pointer("/missing"), None);
        assert_eq!(v.pointer("no-leading-slash"), None);
    }

    #[test]
    fn pointer_unescapes_tilde_and_slash() {
        let mut map = Map::new();
        map.insert(String::from("a/b"), Value::Bool(true));
        map.insert(String::from("c~d"), Value::Bool(false));
        let v = Value::Object(map);
        assert_eq!(v.pointer("/a~1b"), Some(&Value::Bool(true)));
        assert_eq!(v.pointer("/c~0d"), Some(&Value::Bool(false)));
    }

    #[test]
    fn pointer_mut_allows_in_place_edit() {
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Bool(false));
        let mut v = Value::Object(map);
        *v.pointer_mut("/a").unwrap() = Value::Bool(true);
        assert_eq!(v.pointer("/a"), Some(&Value::Bool(true)));
        assert_eq!(v.pointer_mut("/missing"), None);
    }

    #[test]
    fn from_iterator_array() {
        let v: Value = alloc::vec![Value::Bool(true), Value::Null]
            .into_iter()
            .collect();
        assert_eq!(v, Value::Array(alloc::vec![Value::Bool(true), Value::Null]));
    }

    #[test]
    fn from_iterator_object() {
        let v: Value = alloc::vec![(String::from("a"), Value::Bool(true))]
            .into_iter()
            .collect();
        let mut expected = Map::new();
        expected.insert(String::from("a"), Value::Bool(true));
        assert_eq!(v, Value::Object(expected));
    }

    #[test]
    fn from_str_impl() {
        use core::str::FromStr;
        assert_eq!(Value::from_str("null").unwrap(), Value::Null);
        assert_eq!("true".parse::<Value>().unwrap(), Value::Bool(true));
        assert!("not json".parse::<Value>().is_err());
    }
}
