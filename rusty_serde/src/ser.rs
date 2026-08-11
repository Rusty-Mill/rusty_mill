//! The serialization half of the data model: [`Serialize`] describes how to
//! walk a Rust value, [`Serializer`] describes how a format turns that walk
//! into bytes/text. This mirrors serde's split on purpose - it's the reason
//! `#[derive(Serialize)]` can stay format-agnostic.

use crate::error::Error;

/// A type that knows how to visit itself against any [`Serializer`].
pub trait Serialize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
}

/// A format capable of turning a value's structure into its own output type.
///
/// Each `serialize_*` method corresponds to one shape in the data model
/// (bool, integer, string, sequence, map, struct, enum variant, ...). Compound
/// shapes (`seq`, `map`, `struct`, ...) return a builder object so the caller
/// can push elements/fields one at a time without the serializer needing to
/// buffer the whole value up front.
pub trait Serializer: Sized {
    type Ok;
    type Error: Error;

    type SerializeSeq: SerializeSeq<Ok = Self::Ok, Error = Self::Error>;
    type SerializeTuple: SerializeTuple<Ok = Self::Ok, Error = Self::Error>;
    type SerializeTupleStruct: SerializeTupleStruct<Ok = Self::Ok, Error = Self::Error>;
    type SerializeTupleVariant: SerializeTupleVariant<Ok = Self::Ok, Error = Self::Error>;
    type SerializeMap: SerializeMap<Ok = Self::Ok, Error = Self::Error>;
    type SerializeStruct: SerializeStruct<Ok = Self::Ok, Error = Self::Error>;
    type SerializeStructVariant: SerializeStructVariant<Ok = Self::Ok, Error = Self::Error>;

    /// Whether the output is meant to be human-readable (text formats like
    /// this crate's JSON/RON) or not (a hypothetical compact binary
    /// format). Lets a hand-written `Serialize` impl choose a
    /// representation per format - e.g. an ISO-8601 string for a
    /// timestamp type here, vs. a raw integer for a binary format.
    /// Defaults to `true`; a binary format would override it to `false`.
    fn is_human_readable(&self) -> bool {
        true
    }

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error>;

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.serialize_i64(v as i64)
    }
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error>;

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.serialize_u64(v as u64)
    }
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error>;

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.serialize_f64(v as f64)
    }
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error>;

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        let mut buf = [0u8; 4];
        self.serialize_str(v.encode_utf8(&mut buf))
    }
    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error>;
    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error>;

    fn serialize_none(self) -> Result<Self::Ok, Self::Error>;
    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized;

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error>;
    fn serialize_unit_struct(self, name: &'static str) -> Result<Self::Ok, Self::Error> {
        let _ = name;
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error>;

    fn serialize_newtype_struct<T>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized,
    {
        let _ = name;
        value.serialize(self)
    }
    fn serialize_newtype_variant<T>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: Serialize + ?Sized;

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error>;
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error>;
    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error>;
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error>;

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error>;
    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error>;
    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error>;

    /// Serializes an iterator as a sequence, without collecting it into a
    /// `Vec` first. Default impl built on [`Self::serialize_seq`] plus
    /// [`SerializeSeq::serialize_element`]; a length is passed through only
    /// when the iterator's own [`Iterator::size_hint`] is exact.
    fn collect_seq<I>(self, iter: I) -> Result<Self::Ok, Self::Error>
    where
        I: IntoIterator,
        I::Item: Serialize,
    {
        let iter = iter.into_iter();
        let mut seq = self.serialize_seq(exact_size_hint(&iter))?;
        for item in iter {
            SerializeSeq::serialize_element(&mut seq, &item)?;
        }
        SerializeSeq::end(seq)
    }

    /// Serializes an iterator of key/value pairs as a map, without
    /// collecting it into a `HashMap`/`BTreeMap` first. Default impl built
    /// on [`Self::serialize_map`] plus [`SerializeMap::serialize_entry`].
    fn collect_map<K, V, I>(self, iter: I) -> Result<Self::Ok, Self::Error>
    where
        K: Serialize,
        V: Serialize,
        I: IntoIterator<Item = (K, V)>,
    {
        let iter = iter.into_iter();
        let mut map = self.serialize_map(exact_size_hint(&iter))?;
        for (k, v) in iter {
            SerializeMap::serialize_entry(&mut map, &k, &v)?;
        }
        SerializeMap::end(map)
    }

    /// Serializes any [`Display`](std::fmt::Display) value as a string.
    ///
    /// The default impl just does the naive thing
    /// (`self.serialize_str(&value.to_string())`); formats that can stream
    /// text directly (this crate's JSON/RON formats included) can override
    /// this to skip that intermediate allocation.
    fn collect_str<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: std::fmt::Display + ?Sized,
    {
        self.serialize_str(&value.to_string())
    }
}

fn exact_size_hint<I: Iterator>(iter: &I) -> Option<usize> {
    match iter.size_hint() {
        (lower, Some(upper)) if lower == upper => Some(lower),
        _ => None,
    }
}

pub trait SerializeSeq {
    type Ok;
    type Error: Error;
    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

pub trait SerializeTuple {
    type Ok;
    type Error: Error;
    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

pub trait SerializeTupleStruct {
    type Ok;
    type Error: Error;
    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

pub trait SerializeTupleVariant {
    type Ok;
    type Error: Error;
    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

pub trait SerializeMap {
    type Ok;
    type Error: Error;
    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;

    fn serialize_entry<K, V>(&mut self, key: &K, value: &V) -> Result<(), Self::Error>
    where
        K: Serialize + ?Sized,
        V: Serialize + ?Sized,
    {
        self.serialize_key(key)?;
        self.serialize_value(value)
    }
}

pub trait SerializeStruct {
    type Ok;
    type Error: Error;
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

pub trait SerializeStructVariant {
    type Ok;
    type Error: Error;
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize + ?Sized;
    fn end(self) -> Result<Self::Ok, Self::Error>;
}
