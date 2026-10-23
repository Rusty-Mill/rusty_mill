//! Deserializes string pairs (query fields, path parameters) into a
//! handler's types: a map or struct by name, a tuple or sequence by
//! position, or a single scalar when there is exactly one pair. Values are
//! parsed as the target asks (`u32`, `bool`, ...), as `serde_urlencoded`
//! does.

use serde::de::value::{Error, MapDeserializer, SeqDeserializer};
use serde::de::{self, IntoDeserializer, Visitor};
use serde::forward_to_deserialize_any;

/// Name/value pairs, as a whole.
pub(crate) struct Pairs<'a>(pub(crate) &'a [(String, String)]);

impl<'a> Pairs<'a> {
    fn single(&self) -> Result<Value<'a>, Error> {
        match self.0 {
            [(_, value)] => Ok(Value(value)),
            pairs => Err(de::Error::custom(format!(
                "expected 1 value, found {}",
                pairs.len()
            ))),
        }
    }

    fn map<'de>(&self) -> MapDeserializer<'de, impl Iterator<Item = (&'a str, Value<'a>)>, Error> {
        MapDeserializer::new(self.0.iter().map(|(k, v)| (k.as_str(), Value(v))))
    }

    fn seq(&self) -> SeqDeserializer<impl Iterator<Item = Value<'a>>, Error> {
        SeqDeserializer::new(self.0.iter().map(|(_, v)| Value(v)))
    }
}

macro_rules! single {
    ($($method:ident)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            self.single()?.$method(visitor)
        }
    )*};
}

impl<'de, 'a> de::Deserializer<'de> for Pairs<'a> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        self.map().deserialize_any(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_map(self.map())
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_map(self.map())
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_seq(self.seq())
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_seq(self.seq())
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_seq(self.seq())
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.single()?.deserialize_enum(name, variants, visitor)
    }

    single! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_f32 deserialize_f64 deserialize_char deserialize_str
        deserialize_string deserialize_bytes deserialize_byte_buf deserialize_option
        deserialize_unit deserialize_identifier
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }
}

/// One string value, parsed as the target asks.
struct Value<'a>(&'a str);

macro_rules! parse {
    ($($method:ident => $visit:ident),*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self.0.parse() {
                Ok(v) => visitor.$visit(v),
                Err(e) => Err(de::Error::custom(format!("{:?}: {e}", self.0))),
            }
        }
    )*};
}

impl<'de, 'a> de::Deserializer<'de> for Value<'a> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_str(self.0)
    }

    parse! {
        deserialize_bool => visit_bool,
        deserialize_i8 => visit_i8, deserialize_i16 => visit_i16,
        deserialize_i32 => visit_i32, deserialize_i64 => visit_i64,
        deserialize_u8 => visit_u8, deserialize_u16 => visit_u16,
        deserialize_u32 => visit_u32, deserialize_u64 => visit_u64,
        deserialize_f32 => visit_f32, deserialize_f64 => visit_f64,
        deserialize_char => visit_char
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_enum(self.0.into_deserializer())
    }

    forward_to_deserialize_any! {
        str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
        identifier ignored_any
    }
}

impl<'de, 'a> IntoDeserializer<'de, Error> for Value<'a> {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashMap;

    use serde::Deserialize;

    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn de<T: de::DeserializeOwned>(items: &[(&str, &str)]) -> Result<T, Error> {
        T::deserialize(Pairs(&pairs(items)))
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Page {
        limit: u32,
        cursor: Option<String>,
        verbose: bool,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum Kind {
        Task,
        Message,
    }

    #[test]
    fn struct_parses_fields_and_defaults_missing_options() {
        let page: Page = de(&[("limit", "10"), ("verbose", "true")]).unwrap();
        assert_eq!(
            page,
            Page {
                limit: 10,
                cursor: None,
                verbose: true
            }
        );
    }

    #[test]
    fn map_keeps_strings() {
        let m: HashMap<String, String> = de(&[("session", "a b")]).unwrap();
        assert_eq!(m["session"], "a b");
    }

    #[test]
    fn tuple_is_positional() {
        let t: (String, usize) = de(&[("name", "x"), ("n", "3")]).unwrap();
        assert_eq!(t, ("x".to_string(), 3));
    }

    #[test]
    fn single_scalar_and_enum() {
        assert_eq!(de::<u64>(&[("id", "42")]).unwrap(), 42);
        assert_eq!(de::<Kind>(&[("kind", "task")]).unwrap(), Kind::Task);
        assert_ne!(de::<Kind>(&[("kind", "message")]).unwrap(), Kind::Task);
    }

    #[test]
    fn bad_number_and_wrong_arity_fail() {
        assert!(de::<Page>(&[("limit", "ten"), ("verbose", "true")]).is_err());
        assert!(de::<u64>(&[("a", "1"), ("b", "2")]).is_err());
        assert!(de::<Page>(&[("verbose", "true")]).is_err());
    }
}
