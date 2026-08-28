//! [`Map`]: a real newtype (not a bare alias) around a sorted map, with its
//! own `.entry()` API and iterator types, matching `serde_json::Map`'s
//! default (non-`preserve_order`) shape.

use crate::Value;
use alloc::borrow::Borrow;
use alloc::collections::btree_map::{self, BTreeMap};
use alloc::string::String;
use core::fmt;
use core::iter::FromIterator;
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};

/// An owned JSON object: a string-keyed map of [`Value`]s, iterating in
/// sorted key order (matching `serde_json::Map`'s default, non-
/// `preserve_order` behavior). A real type with its own `.entry()` API and
/// iterator types, not a bare `BTreeMap` alias.
#[derive(Clone, Default)]
pub struct Map {
    inner: BTreeMap<String, Value>,
}

impl Map {
    /// An empty map.
    pub fn new() -> Self {
        Map {
            inner: BTreeMap::new(),
        }
    }

    /// Inserts a key/value pair, returning the previous value if the key
    /// was already present.
    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.inner.insert(key, value)
    }

    /// Looks up a value by key.
    pub fn get<Q>(&self, key: &Q) -> Option<&Value>
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.inner.get(key)
    }

    /// Mutably looks up a value by key.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut Value>
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.inner.get_mut(key)
    }

    /// True if `key` is present.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.inner.contains_key(key)
    }

    /// Removes and returns the value at `key`, if present.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<Value>
    where
        String: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.inner.remove(key)
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True if there are no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// An iterator over `(&key, &value)` pairs, in sorted key order.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.inner.iter(),
        }
    }

    /// An iterator over `(&key, &mut value)` pairs, in sorted key order.
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut {
            inner: self.inner.iter_mut(),
        }
    }

    /// An iterator over the keys, in sorted order.
    pub fn keys(&self) -> Keys<'_> {
        Keys {
            inner: self.inner.keys(),
        }
    }

    /// An iterator over the values, in sorted key order.
    pub fn values(&self) -> Values<'_> {
        Values {
            inner: self.inner.values(),
        }
    }

    /// A mutable iterator over the values, in sorted key order.
    pub fn values_mut(&mut self) -> ValuesMut<'_> {
        ValuesMut {
            inner: self.inner.values_mut(),
        }
    }

    /// Consumes the map, returning an iterator over its values in sorted
    /// key order.
    pub fn into_values(self) -> IntoValues {
        IntoValues {
            inner: self.inner.into_values(),
        }
    }

    /// The entry API: inspect or modify the value at `key`, inserting a
    /// default only if it's missing, without a second lookup.
    pub fn entry(&mut self, key: String) -> Entry<'_> {
        match self.inner.entry(key) {
            btree_map::Entry::Occupied(inner) => Entry::Occupied(OccupiedEntry { inner }),
            btree_map::Entry::Vacant(inner) => Entry::Vacant(VacantEntry { inner }),
        }
    }
}

impl fmt::Debug for Map {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl PartialEq for Map {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<Q> core::ops::Index<&Q> for Map
where
    String: Borrow<Q>,
    Q: Ord + ?Sized,
{
    type Output = Value;

    /// Panics if the key is absent (this is `Map`'s own `Index`, distinct
    /// from `Value`'s, which returns `Value::Null` instead of panicking).
    fn index(&self, key: &Q) -> &Value {
        self.inner.get(key).expect("no entry found for key")
    }
}

impl FromIterator<(String, Value)> for Map {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Map {
            inner: BTreeMap::from_iter(iter),
        }
    }
}

impl Extend<(String, Value)> for Map {
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }
}

impl IntoIterator for Map {
    type Item = (String, Value);
    type IntoIter = IntoIter;

    fn into_iter(self) -> IntoIter {
        IntoIter {
            inner: self.inner.into_iter(),
        }
    }
}

impl<'a> IntoIterator for &'a Map {
    type Item = (&'a String, &'a Value);
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Map {
    type Item = (&'a String, &'a mut Value);
    type IntoIter = IterMut<'a>;

    fn into_iter(self) -> IterMut<'a> {
        self.iter_mut()
    }
}

macro_rules! wrap_iterator {
    ($name:ident, $item:ty, $inner:ty) => {
        /// An iterator produced by [`Map`].
        pub struct $name {
            inner: $inner,
        }

        impl Iterator for $name {
            type Item = $item;

            fn next(&mut self) -> Option<Self::Item> {
                self.inner.next()
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.inner.size_hint()
            }
        }

        impl DoubleEndedIterator for $name {
            fn next_back(&mut self) -> Option<Self::Item> {
                self.inner.next_back()
            }
        }

        impl ExactSizeIterator for $name {
            fn len(&self) -> usize {
                self.inner.len()
            }
        }
    };
}

macro_rules! wrap_borrowed_iterator {
    ($name:ident<$lt:lifetime>, $item:ty, $inner:ty) => {
        /// An iterator produced by [`Map`].
        pub struct $name<$lt> {
            inner: $inner,
        }

        impl<$lt> Iterator for $name<$lt> {
            type Item = $item;

            fn next(&mut self) -> Option<Self::Item> {
                self.inner.next()
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.inner.size_hint()
            }
        }

        impl<$lt> DoubleEndedIterator for $name<$lt> {
            fn next_back(&mut self) -> Option<Self::Item> {
                self.inner.next_back()
            }
        }

        impl<$lt> ExactSizeIterator for $name<$lt> {
            fn len(&self) -> usize {
                self.inner.len()
            }
        }
    };
}

wrap_iterator!(IntoIter, (String, Value), btree_map::IntoIter<String, Value>);
wrap_iterator!(IntoValues, Value, btree_map::IntoValues<String, Value>);
wrap_borrowed_iterator!(
    Iter<'a>,
    (&'a String, &'a Value),
    btree_map::Iter<'a, String, Value>
);
wrap_borrowed_iterator!(
    IterMut<'a>,
    (&'a String, &'a mut Value),
    btree_map::IterMut<'a, String, Value>
);
wrap_borrowed_iterator!(Keys<'a>, &'a String, btree_map::Keys<'a, String, Value>);
wrap_borrowed_iterator!(Values<'a>, &'a Value, btree_map::Values<'a, String, Value>);
wrap_borrowed_iterator!(
    ValuesMut<'a>,
    &'a mut Value,
    btree_map::ValuesMut<'a, String, Value>
);

/// A view into a single entry in a [`Map`], from [`Map::entry`].
pub enum Entry<'a> {
    /// The key is present.
    Occupied(OccupiedEntry<'a>),
    /// The key is absent.
    Vacant(VacantEntry<'a>),
}

impl<'a> Entry<'a> {
    /// The entry's key, whether occupied or vacant.
    pub fn key(&self) -> &String {
        match self {
            Entry::Occupied(e) => e.key(),
            Entry::Vacant(e) => e.key(),
        }
    }

    /// Returns the existing value, or inserts and returns `default`.
    pub fn or_insert(self, default: Value) -> &'a mut Value {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(default),
        }
    }

    /// Returns the existing value, or inserts and returns the result of
    /// calling `default`.
    pub fn or_insert_with<F: FnOnce() -> Value>(self, default: F) -> &'a mut Value {
        match self {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => e.insert(default()),
        }
    }

    /// Applies `f` to the value if the entry is occupied, then returns the
    /// entry unchanged (still usable with `or_insert`/`or_insert_with`).
    pub fn and_modify<F: FnOnce(&mut Value)>(self, f: F) -> Self {
        match self {
            Entry::Occupied(mut e) => {
                f(e.get_mut());
                Entry::Occupied(e)
            }
            Entry::Vacant(e) => Entry::Vacant(e),
        }
    }
}

/// An occupied [`Entry`].
pub struct OccupiedEntry<'a> {
    inner: btree_map::OccupiedEntry<'a, String, Value>,
}

impl<'a> OccupiedEntry<'a> {
    /// The entry's key.
    pub fn key(&self) -> &String {
        self.inner.key()
    }

    /// A shared reference to the entry's value.
    pub fn get(&self) -> &Value {
        self.inner.get()
    }

    /// A mutable reference to the entry's value, borrowing the entry.
    pub fn get_mut(&mut self) -> &mut Value {
        self.inner.get_mut()
    }

    /// Converts into a mutable reference with the map's own lifetime.
    pub fn into_mut(self) -> &'a mut Value {
        self.inner.into_mut()
    }

    /// Replaces the value, returning the old one.
    pub fn insert(&mut self, value: Value) -> Value {
        self.inner.insert(value)
    }

    /// Removes and returns the entry's value.
    pub fn remove(self) -> Value {
        self.inner.remove()
    }
}

/// A vacant [`Entry`].
pub struct VacantEntry<'a> {
    inner: btree_map::VacantEntry<'a, String, Value>,
}

impl<'a> VacantEntry<'a> {
    /// The entry's key.
    pub fn key(&self) -> &String {
        self.inner.key()
    }

    /// Inserts `value`, returning a mutable reference to it.
    pub fn insert(self, value: Value) -> &'a mut Value {
        self.inner.insert(value)
    }
}

impl Serialize for Map {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Map {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MapVisitor;

        impl<'de> Visitor<'de> for MapVisitor {
            type Value = Map;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Map, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut map = Map::new();
                while let Some((key, value)) = access.next_entry()? {
                    map.insert(key, value);
                }
                Ok(map)
            }
        }

        deserializer.deserialize_map(MapVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    fn m(pairs: &[(&str, Value)]) -> Map {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn insert_get_remove() {
        let mut map = Map::new();
        assert!(map.is_empty());
        assert_eq!(map.insert(String::from("a"), Value::Bool(true)), None);
        assert_eq!(
            map.insert(String::from("a"), Value::Bool(false)),
            Some(Value::Bool(true))
        );
        assert_eq!(map.get("a"), Some(&Value::Bool(false)));
        assert!(map.contains_key("a"));
        assert_eq!(map.len(), 1);
        assert_eq!(map.remove("a"), Some(Value::Bool(false)));
        assert_eq!(map.get("a"), None);
        assert!(map.is_empty());
    }

    #[test]
    fn iterates_in_sorted_key_order() {
        let map = m(&[("b", Value::Null), ("a", Value::Null), ("c", Value::Null)]);
        let keys: Vec<&String> = map.keys().collect();
        assert_eq!(
            keys,
            alloc::vec![&String::from("a"), &String::from("b"), &String::from("c")]
        );
    }

    #[test]
    fn iter_and_iter_mut() {
        let mut map = m(&[("a", Value::Number(crate::Number::from(1u64)))]);
        for (_, v) in map.iter_mut() {
            *v = Value::Bool(true);
        }
        let entries: Vec<(&String, &Value)> = map.iter().collect();
        assert_eq!(
            entries,
            alloc::vec![(&String::from("a"), &Value::Bool(true))]
        );
    }

    #[test]
    fn into_iter_by_value_and_by_ref() {
        let map = m(&[("a", Value::Bool(true))]);
        let owned: Vec<(String, Value)> = map.clone().into_iter().collect();
        assert_eq!(owned, alloc::vec![(String::from("a"), Value::Bool(true))]);

        let borrowed: Vec<(&String, &Value)> = (&map).into_iter().collect();
        assert_eq!(
            borrowed,
            alloc::vec![(&String::from("a"), &Value::Bool(true))]
        );
    }

    #[test]
    fn values_and_into_values() {
        let map = m(&[("a", Value::Bool(true)), ("b", Value::Bool(false))]);
        let values: Vec<&Value> = map.values().collect();
        assert_eq!(values, alloc::vec![&Value::Bool(true), &Value::Bool(false)]);
        let values: Vec<Value> = map.into_values().collect();
        assert_eq!(values, alloc::vec![Value::Bool(true), Value::Bool(false)]);
    }

    #[test]
    fn entry_or_insert_on_vacant_and_occupied() {
        let mut map = Map::new();
        *map.entry(String::from("a"))
            .or_insert(Value::Number(crate::Number::from(1u64))) =
            Value::Number(crate::Number::from(1u64));
        assert_eq!(
            map.get("a"),
            Some(&Value::Number(crate::Number::from(1u64)))
        );

        let v = map.entry(String::from("a")).or_insert(Value::Bool(true));
        assert_eq!(*v, Value::Number(crate::Number::from(1u64)));
    }

    #[test]
    fn entry_and_modify() {
        let mut map = m(&[("a", Value::Number(crate::Number::from(1u64)))]);
        map.entry(String::from("a")).and_modify(|v| {
            if let Value::Number(n) = v {
                *v = Value::Number(crate::Number::from(n.as_u64().unwrap() + 1));
            }
        });
        assert_eq!(
            map.get("a"),
            Some(&Value::Number(crate::Number::from(2u64)))
        );

        map.entry(String::from("missing"))
            .and_modify(|_| panic!("must not run on vacant entry"))
            .or_insert(Value::Bool(true));
        assert_eq!(map.get("missing"), Some(&Value::Bool(true)));
    }

    #[test]
    fn index_operator_panics_on_missing_key() {
        let map = m(&[("a", Value::Bool(true))]);
        assert_eq!(map["a"], Value::Bool(true));
    }

    #[test]
    #[should_panic(expected = "no entry found for key")]
    fn index_operator_panics_message() {
        let map = Map::new();
        let _ = &map["missing"];
    }

    #[test]
    fn debug_matches_btreemap_style() {
        let map = m(&[("a", Value::Bool(true))]);
        let debug = alloc::format!("{map:?}");
        assert!(debug.contains("\"a\""));
    }

    #[test]
    fn serializes_and_deserializes_through_serde_json() {
        let map = m(&[("a", Value::Bool(true)), ("b", Value::Null)]);
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"{"a":true,"b":null}"#);
        let back: Map = serde_json::from_str(&json).unwrap();
        assert_eq!(back, map);
    }
}
