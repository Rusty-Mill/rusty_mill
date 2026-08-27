//! Support for `#[rusty_serde(flatten)]`: merging a nested value's own
//! fields/entries into the parent object, instead of nesting it under its
//! own key.
//!
//! [`FlattenSerializer`] is the serialize-side half - it's a [`Serializer`]
//! that doesn't build a value of its own; it forwards whatever
//! struct/map the flattened field serializes as directly into an
//! already-open outer [`SerializeMap`]. The deserialize-side half needs no
//! new machinery: the derive macro collects whatever keys don't match a
//! named field into a buffered [`crate::Value`] and hands that to the
//! flattened field's own `Deserialize` impl through
//! [`crate::value::ValueDeserializer`], the same buffering machinery
//! untagged/internally-tagged enums already use.

use crate::error::Error as ErrorTrait;
use crate::impossible::Impossible;
use crate::ser::{Serialize, SerializeMap, SerializeStruct, Serializer};

/// A [`Serializer`] that only accepts struct/map-shaped values, forwarding
/// their fields/entries into an already-open outer [`SerializeMap`] rather
/// than building a nested value. `None` and unit both contribute nothing
/// (so `#[rusty_serde(flatten)]` on an `Option<Struct>` field works as
/// "maybe merge these fields in"); every other shape is a serialize-time
/// error, since there's no sound way to merge a scalar or sequence into an
/// object.
pub struct FlattenSerializer<'a, M> {
    map: &'a mut M,
}

impl<'a, M> FlattenSerializer<'a, M> {
    pub fn new(map: &'a mut M) -> Self {
        FlattenSerializer { map }
    }
}

impl<'a, M> Serializer for FlattenSerializer<'a, M>
where
    M: SerializeMap,
{
    type Ok = ();
    type Error = M::Error;

    type SerializeSeq = Impossible<(), M::Error>;
    type SerializeTuple = Impossible<(), M::Error>;
    type SerializeTupleStruct = Impossible<(), M::Error>;
    type SerializeTupleVariant = Impossible<(), M::Error>;
    type SerializeMap = FlattenMapAdapter<'a, M>;
    type SerializeStruct = FlattenMapAdapter<'a, M>;
    type SerializeStructVariant = Impossible<(), M::Error>;

    fn serialize_bool(self, _v: bool) -> Result<(), M::Error> {
        Err(unsupported("a boolean"))
    }
    fn serialize_i64(self, _v: i64) -> Result<(), M::Error> {
        Err(unsupported("an integer"))
    }
    fn serialize_u64(self, _v: u64) -> Result<(), M::Error> {
        Err(unsupported("an integer"))
    }
    fn serialize_f64(self, _v: f64) -> Result<(), M::Error> {
        Err(unsupported("a float"))
    }
    fn serialize_str(self, _v: &str) -> Result<(), M::Error> {
        Err(unsupported("a string"))
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<(), M::Error> {
        Err(unsupported("bytes"))
    }

    fn serialize_none(self) -> Result<(), M::Error> {
        Ok(())
    }
    fn serialize_some<T>(self, value: &T) -> Result<(), M::Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), M::Error> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<(), M::Error> {
        Err(unsupported("a unit variant"))
    }
    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), M::Error>
    where
        T: Serialize + ?Sized,
    {
        Err(unsupported("a newtype variant"))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, M::Error> {
        Err(unsupported("a sequence"))
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, M::Error> {
        Err(unsupported("a tuple"))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, M::Error> {
        Err(unsupported("a tuple struct"))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, M::Error> {
        Err(unsupported("a tuple variant"))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, M::Error> {
        Ok(FlattenMapAdapter { map: self.map })
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, M::Error> {
        Ok(FlattenMapAdapter { map: self.map })
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, M::Error> {
        Err(unsupported("a struct variant"))
    }
}

fn unsupported<E: ErrorTrait>(shape: &str) -> E {
    E::custom(format!("can only flatten a struct or map, found {shape}"))
}

/// Forwards a struct/map's fields/entries into the outer [`SerializeMap`]
/// this was built from, without ever calling that outer map's `end` -
/// only the field/entry write calls are shared; the outer map is still
/// the only thing that opens/closes the object.
pub struct FlattenMapAdapter<'a, M> {
    map: &'a mut M,
}

impl<'a, M: SerializeMap> SerializeMap for FlattenMapAdapter<'a, M> {
    type Ok = ();
    type Error = M::Error;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), M::Error>
    where
        T: Serialize + ?Sized,
    {
        self.map.serialize_key(key)
    }
    fn serialize_value<T>(&mut self, value: &T) -> Result<(), M::Error>
    where
        T: Serialize + ?Sized,
    {
        self.map.serialize_value(value)
    }
    fn end(self) -> Result<(), M::Error> {
        Ok(())
    }
}

impl<'a, M: SerializeMap> SerializeStruct for FlattenMapAdapter<'a, M> {
    type Ok = ();
    type Error = M::Error;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), M::Error>
    where
        T: Serialize + ?Sized,
    {
        self.map.serialize_entry(key, value)
    }
    fn end(self) -> Result<(), M::Error> {
        Ok(())
    }
}
