//! Converts any `serde::Serialize` value directly into a [`Value`], without
//! round-tripping through JSON text -- the `Value`-native counterpart of
//! [`crate::to_string`].

use crate::{Error, Map, Value};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
};

/// Serializes any `Serialize` value directly into a [`Value`], without
/// round-tripping through JSON text.
pub fn to_value<T>(value: &T) -> Result<Value, Error>
where
    T: Serialize + ?Sized,
{
    value.serialize(ValueSerializer)
}

struct ValueSerializer;

impl ser::Serializer for ValueSerializer {
    type Ok = Value;
    type Error = Error;
    type SerializeSeq = SeqCompound;
    type SerializeTuple = SeqCompound;
    type SerializeTupleStruct = SeqCompound;
    type SerializeTupleVariant = VariantSeqCompound;
    type SerializeMap = MapCompound;
    type SerializeStruct = MapCompound;
    type SerializeStructVariant = VariantMapCompound;

    fn serialize_bool(self, v: bool) -> Result<Value, Error> {
        Ok(Value::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Value, Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i16(self, v: i16) -> Result<Value, Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i32(self, v: i32) -> Result<Value, Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i64(self, v: i64) -> Result<Value, Error> {
        Ok(Value::from(v))
    }
    fn serialize_i128(self, v: i128) -> Result<Value, Error> {
        crate::Number::from_i128(v)
            .map(Value::Number)
            .ok_or_else(|| <Error as ser::Error>::custom("i128 value out of range"))
    }

    fn serialize_u8(self, v: u8) -> Result<Value, Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u16(self, v: u16) -> Result<Value, Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u32(self, v: u32) -> Result<Value, Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u64(self, v: u64) -> Result<Value, Error> {
        Ok(Value::from(v))
    }
    fn serialize_u128(self, v: u128) -> Result<Value, Error> {
        crate::Number::from_u128(v)
            .map(Value::Number)
            .ok_or_else(|| <Error as ser::Error>::custom("u128 value out of range"))
    }

    fn serialize_f32(self, v: f32) -> Result<Value, Error> {
        Ok(Value::from(v))
    }
    fn serialize_f64(self, v: f64) -> Result<Value, Error> {
        Ok(Value::from(v))
    }

    fn serialize_char(self, v: char) -> Result<Value, Error> {
        let mut buf = [0u8; 4];
        self.serialize_str(v.encode_utf8(&mut buf))
    }

    fn serialize_str(self, v: &str) -> Result<Value, Error> {
        Ok(Value::String(String::from(v)))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Value, Error> {
        // JSON has no byte-string type; serialize as an array of numbers,
        // same as this crate's text `Serializer` (Phase 1).
        Ok(Value::Array(v.iter().map(|&b| Value::from(b)).collect()))
    }

    fn serialize_none(self) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value, Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value, Error> {
        Ok(Value::String(String::from(variant)))
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value, Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value, Error> {
        let mut map = Map::new();
        map.insert(String::from(variant), to_value(value)?);
        Ok(Value::Object(map))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SeqCompound, Error> {
        Ok(SeqCompound {
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<SeqCompound, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqCompound, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<VariantSeqCompound, Error> {
        Ok(VariantSeqCompound {
            variant,
            items: Vec::with_capacity(len),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<MapCompound, Error> {
        Ok(MapCompound {
            map: Map::new(),
            next_key: None,
        })
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<MapCompound, Error> {
        Ok(MapCompound {
            map: Map::new(),
            next_key: None,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<VariantMapCompound, Error> {
        Ok(VariantMapCompound {
            variant,
            map: Map::new(),
        })
    }

    fn collect_str<T: ?Sized + core::fmt::Display>(self, value: &T) -> Result<Value, Error> {
        Ok(Value::String(alloc::format!("{value}")))
    }
}

/// Shared by `SerializeSeq`/`SerializeTuple`/`SerializeTupleStruct`: builds a
/// `Value::Array`.
struct SeqCompound {
    items: Vec<Value>,
}

impl SerializeSeq for SeqCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        self.items.push(to_value(value)?);
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        Ok(Value::Array(self.items))
    }
}

impl SerializeTuple for SeqCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, Error> {
        SerializeSeq::end(self)
    }
}

impl SerializeTupleStruct for SeqCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, Error> {
        SerializeSeq::end(self)
    }
}

/// Builds the `{"variant": [...]}` envelope for a tuple enum variant.
struct VariantSeqCompound {
    variant: &'static str,
    items: Vec<Value>,
}

impl SerializeTupleVariant for VariantSeqCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        self.items.push(to_value(value)?);
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        let mut map = Map::new();
        map.insert(String::from(self.variant), Value::Array(self.items));
        Ok(Value::Object(map))
    }
}

/// Shared by `SerializeMap`/`SerializeStruct`: builds a `Value::Object`.
struct MapCompound {
    map: Map,
    next_key: Option<String>,
}

impl SerializeMap for MapCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Error> {
        self.next_key = Some(key.serialize(KeySerializer)?);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        let key = self
            .next_key
            .take()
            .expect("serialize_value called before serialize_key");
        self.map.insert(key, to_value(value)?);
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        Ok(Value::Object(self.map))
    }
}

impl SerializeStruct for MapCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.map.insert(String::from(key), to_value(value)?);
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        Ok(Value::Object(self.map))
    }
}

/// Builds the `{"variant": {...}}` envelope for a struct enum variant.
struct VariantMapCompound {
    variant: &'static str,
    map: Map,
}

impl SerializeStructVariant for VariantMapCompound {
    type Ok = Value;
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.map.insert(String::from(key), to_value(value)?);
        Ok(())
    }

    fn end(self) -> Result<Value, Error> {
        let mut outer = Map::new();
        outer.insert(String::from(self.variant), Value::Object(self.map));
        Ok(Value::Object(outer))
    }
}

/// A restricted `Serializer` used only for map keys, since JSON object keys
/// must be strings. Accepts string-like and primitive scalar types
/// (stringified), rejects anything else. Mirrors the text `Serializer`'s
/// `MapKeySerializer`.
struct KeySerializer;

impl ser::Serializer for KeySerializer {
    type Ok = String;
    type Error = Error;
    type SerializeSeq = ser::Impossible<String, Error>;
    type SerializeTuple = ser::Impossible<String, Error>;
    type SerializeTupleStruct = ser::Impossible<String, Error>;
    type SerializeTupleVariant = ser::Impossible<String, Error>;
    type SerializeMap = ser::Impossible<String, Error>;
    type SerializeStruct = ser::Impossible<String, Error>;
    type SerializeStructVariant = ser::Impossible<String, Error>;

    fn serialize_str(self, v: &str) -> Result<String, Error> {
        Ok(String::from(v))
    }

    fn serialize_bool(self, v: bool) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i8(self, v: i8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i16(self, v: i16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i32(self, v: i32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_i64(self, v: i64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u8(self, v: u8) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u16(self, v: u16) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u32(self, v: u32) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_u64(self, v: u64) -> Result<String, Error> {
        Ok(v.to_string())
    }
    fn serialize_char(self, v: char) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_f32(self, _v: f32) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_f64(self, _v: f64) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_none(self) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<String, Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<String, Error> {
        Ok(String::from(variant))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<String, Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<String, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }

    fn collect_str<T: ?Sized + core::fmt::Display>(self, value: &T) -> Result<String, Error> {
        Ok(alloc::format!("{value}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Number;
    use serde::Serialize;

    #[test]
    fn converts_scalars() {
        assert_eq!(to_value(&true).unwrap(), Value::Bool(true));
        assert_eq!(
            to_value(&42u32).unwrap(),
            Value::Number(Number::from(42u32))
        );
        assert_eq!(
            to_value(&-7i64).unwrap(),
            Value::Number(Number::from(-7i64))
        );
        assert_eq!(to_value("hi").unwrap(), Value::String(String::from("hi")));
        assert_eq!(to_value(&Option::<i32>::None).unwrap(), Value::Null);
    }

    #[test]
    fn converts_seq_and_map() {
        assert_eq!(
            to_value(&alloc::vec![1, 2, 3]).unwrap(),
            Value::Array(alloc::vec![
                Value::from(1u32),
                Value::from(2u32),
                Value::from(3u32)
            ])
        );

        let mut btree: alloc::collections::BTreeMap<String, i32> = Default::default();
        btree.insert(String::from("a"), 1);
        btree.insert(String::from("b"), 2);
        let value = to_value(&btree).unwrap();
        let mut expected = Map::new();
        expected.insert(String::from("a"), Value::from(1u32));
        expected.insert(String::from("b"), Value::from(2u32));
        assert_eq!(value, Value::Object(expected));
    }

    #[derive(Serialize)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[derive(Serialize)]
    enum Shape {
        Unit,
        Newtype(i32),
        Tuple(i32, i32),
        Struct { w: i32, h: i32 },
    }

    #[test]
    fn converts_derived_struct() {
        let value = to_value(&Point { x: 1, y: -2 }).unwrap();
        let mut expected = Map::new();
        expected.insert(String::from("x"), Value::from(1i32));
        expected.insert(String::from("y"), Value::from(-2i32));
        assert_eq!(value, Value::Object(expected));
    }

    #[test]
    fn converts_derived_enum_variants() {
        assert_eq!(to_value(&Shape::Unit).unwrap(), Value::from("Unit"));

        let mut newtype = Map::new();
        newtype.insert(String::from("Newtype"), Value::from(5i32));
        assert_eq!(
            to_value(&Shape::Newtype(5)).unwrap(),
            Value::Object(newtype)
        );

        let mut tuple = Map::new();
        tuple.insert(
            String::from("Tuple"),
            Value::Array(alloc::vec![Value::from(1i32), Value::from(2i32)]),
        );
        assert_eq!(to_value(&Shape::Tuple(1, 2)).unwrap(), Value::Object(tuple));

        let mut inner = Map::new();
        inner.insert(String::from("w"), Value::from(3i32));
        inner.insert(String::from("h"), Value::from(4i32));
        let mut outer = Map::new();
        outer.insert(String::from("Struct"), Value::Object(inner));
        assert_eq!(
            to_value(&Shape::Struct { w: 3, h: 4 }).unwrap(),
            Value::Object(outer)
        );
    }

    #[test]
    fn matches_serde_json_output() {
        let p = Point { x: 7, y: 8 };
        let ours = to_value(&p).unwrap();
        let theirs: serde_json::Value = serde_json::to_value(&p).unwrap();
        assert_eq!(crate::to_string(&ours).unwrap(), theirs.to_string());
    }
}
