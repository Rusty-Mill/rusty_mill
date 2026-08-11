//! A placeholder associated type for `Serializer` impls that only support a
//! subset of the data model (e.g. a JSON map-key serializer, which only
//! ever produces scalars and therefore never actually constructs a
//! `SerializeSeq`/`SerializeMap`/...). Every method is unreachable because
//! the only way to obtain an `Impossible` is through a `serialize_*` method
//! that always returns `Err` first.

use std::marker::PhantomData;

use crate::error::Error;
use crate::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};

pub struct Impossible<Ok, E> {
    void: std::convert::Infallible,
    _marker: PhantomData<(Ok, E)>,
}

impl<Ok, E: Error> SerializeSeq for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_element<T>(&mut self, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}

impl<Ok, E: Error> SerializeTuple for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_element<T>(&mut self, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}

impl<Ok, E: Error> SerializeTupleStruct for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_field<T>(&mut self, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}

impl<Ok, E: Error> SerializeTupleVariant for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_field<T>(&mut self, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}

impl<Ok, E: Error> SerializeMap for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_key<T>(&mut self, _key: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn serialize_value<T>(&mut self, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}

impl<Ok, E: Error> SerializeStruct for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_field<T>(&mut self, _key: &'static str, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}

impl<Ok, E: Error> SerializeStructVariant for Impossible<Ok, E> {
    type Ok = Ok;
    type Error = E;
    fn serialize_field<T>(&mut self, _key: &'static str, _value: &T) -> Result<(), E>
    where
        T: Serialize + ?Sized,
    {
        match self.void {}
    }
    fn end(self) -> Result<Ok, E> {
        match self.void {}
    }
}
