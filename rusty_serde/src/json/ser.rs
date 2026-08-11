use std::fmt::Write as _;

use crate::error::Error as ErrorTrait;
use crate::json::error::Error;
use crate::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer as SerializerTrait,
};

/// Serialize `value` as a compact JSON string.
pub fn to_string<T>(value: &T) -> Result<String, Error>
where
    T: Serialize + ?Sized,
{
    let mut serializer = Serializer {
        output: String::new(),
    };
    value.serialize(&mut serializer)?;
    Ok(serializer.output)
}

pub struct Serializer {
    output: String,
}

fn escape_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

impl<'a> SerializerTrait for &'a mut Serializer {
    type Ok = ();
    type Error = Error;

    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        self.output.push_str(if v { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        let _ = write!(self.output, "{v}");
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        let _ = write!(self.output, "{v}");
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        if v.is_finite() {
            let _ = write!(self.output, "{v}");
            if v.fract() == 0.0 {
                self.output.push_str(".0");
            }
        } else {
            self.output.push_str("null");
        }
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<(), Error> {
        escape_str(&mut self.output, v);
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        let mut seq = SerializerTrait::serialize_seq(self, Some(v.len()))?;
        for byte in v {
            SerializeSeq::serialize_element(&mut seq, byte)?;
        }
        SerializeSeq::end(seq)
    }

    fn serialize_none(self) -> Result<(), Error> {
        self.output.push_str("null");
        Ok(())
    }

    fn serialize_some<T>(self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Error> {
        self.output.push_str("null");
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        escape_str(&mut self.output, variant);
        Ok(())
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.output.push('{');
        escape_str(&mut self.output, variant);
        self.output.push(':');
        value.serialize(&mut *self)?;
        self.output.push('}');
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Compound<'a>, Error> {
        self.output.push('[');
        Ok(Compound {
            ser: self,
            first: true,
            close: ']',
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Compound<'a>, Error> {
        SerializerTrait::serialize_seq(self, Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Compound<'a>, Error> {
        SerializerTrait::serialize_seq(self, Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Error> {
        self.output.push('{');
        escape_str(&mut self.output, variant);
        self.output.push_str(":[");
        Ok(Compound {
            ser: self,
            first: true,
            close: ']',
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Compound<'a>, Error> {
        self.output.push('{');
        Ok(Compound {
            ser: self,
            first: true,
            close: '}',
        })
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Compound<'a>, Error> {
        self.output.push('{');
        Ok(Compound {
            ser: self,
            first: true,
            close: '}',
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Error> {
        self.output.push('{');
        escape_str(&mut self.output, variant);
        self.output.push_str(":{");
        Ok(Compound {
            ser: self,
            first: true,
            close: '}',
        })
    }
}

pub struct Compound<'a> {
    ser: &'a mut Serializer,
    first: bool,
    close: char,
}

impl<'a> Compound<'a> {
    fn comma(&mut self) {
        if self.first {
            self.first = false;
        } else {
            self.ser.output.push(',');
        }
    }
}

impl<'a> SerializeSeq for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.comma();
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push(self.close);
        Ok(())
    }
}

impl<'a> SerializeTuple for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl<'a> SerializeTupleStruct for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<(), Error> {
        SerializeSeq::end(self)
    }
}

impl<'a> SerializeTupleVariant for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.comma();
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push(']');
        self.ser.output.push('}');
        Ok(())
    }
}

impl<'a> SerializeMap for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.comma();
        // JSON object keys must be strings; route the key through a
        // dedicated serializer that only accepts string-shaped output.
        let mut key_str = String::new();
        key.serialize(&mut KeySerializer {
            output: &mut key_str,
        })?;
        self.ser.output.push_str(&key_str);
        self.ser.output.push(':');
        Ok(())
    }
    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push(self.close);
        Ok(())
    }
}

impl<'a> SerializeStruct for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.comma();
        escape_str(&mut self.ser.output, key);
        self.ser.output.push(':');
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push(self.close);
        Ok(())
    }
}

impl<'a> SerializeStructVariant for Compound<'a> {
    type Ok = ();
    type Error = Error;
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.comma();
        escape_str(&mut self.ser.output, key);
        self.ser.output.push(':');
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push('}');
        self.ser.output.push('}');
        Ok(())
    }
}

/// Serializes just enough of the data model (strings, plus anything whose
/// `Display`-like scalar shape is unambiguous as a JSON object key) to back
/// map keys, since JSON only allows string keys.
struct KeySerializer<'a> {
    output: &'a mut String,
}

impl<'a> SerializerTrait for &'a mut KeySerializer<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = crate::impossible::Impossible<(), Error>;
    type SerializeTuple = crate::impossible::Impossible<(), Error>;
    type SerializeTupleStruct = crate::impossible::Impossible<(), Error>;
    type SerializeTupleVariant = crate::impossible::Impossible<(), Error>;
    type SerializeMap = crate::impossible::Impossible<(), Error>;
    type SerializeStruct = crate::impossible::Impossible<(), Error>;
    type SerializeStructVariant = crate::impossible::Impossible<(), Error>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        escape_str(self.output, if v { "true" } else { "false" });
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        escape_str(self.output, &v.to_string());
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        escape_str(self.output, &v.to_string());
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        escape_str(self.output, &v.to_string());
        Ok(())
    }
    fn serialize_str(self, v: &str) -> Result<(), Error> {
        escape_str(self.output, v);
        Ok(())
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<(), Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_none(self) -> Result<(), Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_some<T>(self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        escape_str(self.output, variant);
        Ok(())
    }
    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Error> {
        Err(Error::custom("map keys must be strings"))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Error> {
        Err(Error::custom("map keys must be strings"))
    }
}
