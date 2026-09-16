//! Converts a [`Value`] directly into any `serde::Deserialize` type, without
//! round-tripping through JSON text -- the `Value`-native counterpart of
//! [`crate::from_str`].

use crate::{Error, IntoIter, Value};
use alloc::string::String;
use alloc::vec;
use serde::de::{
    self, Deserialize, DeserializeOwned, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};

/// Deserializes an instance of `T` directly from a [`Value`], without
/// round-tripping through JSON text.
pub fn from_value<T>(value: Value) -> Result<T, Error>
where
    T: DeserializeOwned,
{
    T::deserialize(value)
}

impl<'de> de::Deserializer<'de> for Value {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(b),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    visitor.visit_u64(u)
                } else if let Some(i) = n.as_i64() {
                    visitor.visit_i64(i)
                } else {
                    visitor.visit_f64(n.as_f64())
                }
            }
            Value::String(s) => visitor.visit_string(s),
            Value::Array(items) => visitor.visit_seq(SeqDeserializer {
                iter: items.into_iter(),
            }),
            Value::Object(map) => visitor.visit_map(MapDeserializer {
                iter: map.into_iter(),
                value: None,
            }),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self {
            Value::Null => visitor.visit_none(),
            other => visitor.visit_some(other),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self {
            Value::Object(map) if map.len() == 1 => {
                let (variant, value) = map.into_iter().next().expect("len() == 1");
                visitor.visit_enum(EnumDeserializer {
                    variant,
                    value: Some(value),
                })
            }
            Value::String(s) => visitor.visit_enum(EnumDeserializer {
                variant: s,
                value: None,
            }),
            other => Err(<Error as de::Error>::custom(alloc::format!(
                "expected a string or a one-entry object for an enum, found {other:?}"
            ))),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple tuple_struct
        map struct identifier ignored_any
    }
}

/// Drives a JSON array's elements into a `SeqAccess`.
struct SeqDeserializer {
    iter: vec::IntoIter<Value>,
}

impl<'de> SeqAccess<'de> for SeqDeserializer {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        match self.iter.next() {
            Some(value) => seed.deserialize(value).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        match self.iter.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

/// Drives a JSON object's entries into a `MapAccess`.
struct MapDeserializer {
    iter: IntoIter,
    value: Option<Value>,
}

impl<'de> MapAccess<'de> for MapDeserializer {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        match self.iter.next() {
            Some((key, value)) => {
                self.value = Some(value);
                seed.deserialize(<String as IntoDeserializer<Error>>::into_deserializer(key))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, Error> {
        let value = self
            .value
            .take()
            .expect("next_value_seed called before next_key_seed");
        seed.deserialize(value)
    }

    fn size_hint(&self) -> Option<usize> {
        match self.iter.size_hint() {
            (lower, Some(upper)) if lower == upper => Some(upper),
            _ => None,
        }
    }
}

/// Drives an externally-tagged enum (`"Variant"` or `{"Variant": payload}`)
/// into an `EnumAccess`.
struct EnumDeserializer {
    variant: String,
    value: Option<Value>,
}

impl<'de> EnumAccess<'de> for EnumDeserializer {
    type Error = Error;
    type Variant = VariantDeserializer;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, VariantDeserializer), Error> {
        let variant = seed.deserialize(<String as IntoDeserializer<Error>>::into_deserializer(
            self.variant,
        ))?;
        Ok((variant, VariantDeserializer { value: self.value }))
    }
}

struct VariantDeserializer {
    value: Option<Value>,
}

impl<'de> VariantAccess<'de> for VariantDeserializer {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        match self.value {
            Some(value) => <() as Deserialize>::deserialize(value),
            None => Ok(()),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        match self.value {
            Some(value) => seed.deserialize(value),
            None => Err(<Error as de::Error>::custom(
                "expected a newtype variant, found a unit variant",
            )),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Error> {
        match self.value {
            Some(Value::Array(items)) => visitor.visit_seq(SeqDeserializer {
                iter: items.into_iter(),
            }),
            Some(_) => Err(<Error as de::Error>::custom(
                "expected a tuple variant's array payload",
            )),
            None => Err(<Error as de::Error>::custom(
                "expected a tuple variant, found a unit variant",
            )),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.value {
            Some(Value::Object(map)) => visitor.visit_map(MapDeserializer {
                iter: map.into_iter(),
                value: None,
            }),
            Some(_) => Err(<Error as de::Error>::custom(
                "expected a struct variant's object payload",
            )),
            None => Err(<Error as de::Error>::custom(
                "expected a struct variant, found a unit variant",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Map;
    use serde::Deserialize;

    #[test]
    fn converts_scalars() {
        assert!(from_value::<bool>(Value::Bool(true)).unwrap());
        assert_eq!(from_value::<i32>(Value::from(-7i64)).unwrap(), -7);
        assert_eq!(from_value::<u32>(Value::from(42u64)).unwrap(), 42);
        assert_eq!(from_value::<f64>(Value::from(1.5)).unwrap(), 1.5);
        assert_eq!(
            from_value::<String>(Value::from("hi")).unwrap(),
            String::from("hi")
        );
        assert_eq!(from_value::<Option<i32>>(Value::Null).unwrap(), None);
        assert_eq!(
            from_value::<Option<i32>>(Value::from(5i64)).unwrap(),
            Some(5)
        );
    }

    #[test]
    fn converts_seq_and_map() {
        let arr = Value::Array(alloc::vec![
            Value::from(1u64),
            Value::from(2u64),
            Value::from(3u64)
        ]);
        assert_eq!(
            from_value::<alloc::vec::Vec<i32>>(arr).unwrap(),
            alloc::vec![1, 2, 3]
        );

        let mut map = Map::new();
        map.insert(String::from("a"), Value::from(1u64));
        map.insert(String::from("b"), Value::from(2u64));
        let value: alloc::collections::BTreeMap<String, i32> =
            from_value(Value::Object(map)).unwrap();
        let mut expected = alloc::collections::BTreeMap::new();
        expected.insert(String::from("a"), 1);
        expected.insert(String::from("b"), 2);
        assert_eq!(value, expected);
    }

    #[derive(Deserialize, serde::Serialize, Debug, PartialEq)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    enum Shape {
        Unit,
        Newtype(i32),
        Tuple(i32, i32),
        Struct { w: i32, h: i32 },
    }

    #[test]
    fn converts_derived_struct() {
        let mut map = Map::new();
        map.insert(String::from("x"), Value::from(1i64));
        map.insert(String::from("y"), Value::from(-2i64));
        let point: Point = from_value(Value::Object(map)).unwrap();
        assert_eq!(point, Point { x: 1, y: -2 });
    }

    #[test]
    fn converts_derived_enum_variants() {
        assert_eq!(
            from_value::<Shape>(Value::from("Unit")).unwrap(),
            Shape::Unit
        );

        let mut newtype = Map::new();
        newtype.insert(String::from("Newtype"), Value::from(5i64));
        assert_eq!(
            from_value::<Shape>(Value::Object(newtype)).unwrap(),
            Shape::Newtype(5)
        );

        let mut tuple = Map::new();
        tuple.insert(
            String::from("Tuple"),
            Value::Array(alloc::vec![Value::from(1i64), Value::from(2i64)]),
        );
        assert_eq!(
            from_value::<Shape>(Value::Object(tuple)).unwrap(),
            Shape::Tuple(1, 2)
        );

        let mut inner = Map::new();
        inner.insert(String::from("w"), Value::from(3i64));
        inner.insert(String::from("h"), Value::from(4i64));
        let mut outer = Map::new();
        outer.insert(String::from("Struct"), Value::Object(inner));
        assert_eq!(
            from_value::<Shape>(Value::Object(outer)).unwrap(),
            Shape::Struct { w: 3, h: 4 }
        );
    }

    #[test]
    fn round_trips_with_to_value() {
        let original = Point { x: 9, y: -3 };
        let value = crate::to_value(&original).unwrap();
        let back: Point = from_value(value).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn errors_on_shape_mismatch() {
        let err = from_value::<i32>(Value::String(String::from("not a number"))).unwrap_err();
        assert!(err.is_data());
    }
}
