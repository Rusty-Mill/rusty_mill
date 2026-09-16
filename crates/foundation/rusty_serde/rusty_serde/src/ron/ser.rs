use std::fmt::Write as _;

use crate::ron::error::Error;
use crate::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer as SerializerTrait,
};

/// Serialize `value` in this crate's RON-like format.
///
/// This is deliberately not a spec-compliant [RON](https://github.com/ron-rs/ron)
/// implementation. The point isn't to reimplement that format byte-for-byte,
/// it's to give the data model a second, genuinely different concrete
/// syntax (bracket choice per shape, unquoted struct field names,
/// non-string map keys) to prove `Serialize`/`Deserialize` impls don't know
/// or care which format they're driven by.
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
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn escape_char(out: &mut String, c: char) {
    out.push('\'');
    match c {
        '\'' => out.push_str("\\'"),
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        c => out.push(c),
    }
    out.push('\'');
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
        } else if v.is_nan() {
            self.output.push_str("NaN");
        } else if v > 0.0 {
            self.output.push_str("inf");
        } else {
            self.output.push_str("-inf");
        }
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<(), Error> {
        escape_char(&mut self.output, v);
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
        self.output.push_str("None");
        Ok(())
    }

    fn serialize_some<T>(self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.output.push_str("Some(");
        value.serialize(&mut *self)?;
        self.output.push(')');
        Ok(())
    }

    fn serialize_unit(self) -> Result<(), Error> {
        self.output.push_str("()");
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        self.output.push_str(variant);
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
        self.output.push_str(variant);
        self.output.push('(');
        value.serialize(&mut *self)?;
        self.output.push(')');
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Compound<'a>, Error> {
        self.output.push('[');
        Ok(Compound {
            ser: self,
            first: true,
            close: "]",
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
        self.output.push_str(variant);
        self.output.push('[');
        Ok(Compound {
            ser: self,
            first: true,
            close: "]",
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Compound<'a>, Error> {
        self.output.push('{');
        Ok(Compound {
            ser: self,
            first: true,
            close: "}",
        })
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Compound<'a>, Error> {
        self.output.push('{');
        Ok(Compound {
            ser: self,
            first: true,
            close: "}",
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a>, Error> {
        self.output.push_str(variant);
        self.output.push('{');
        Ok(Compound {
            ser: self,
            first: true,
            close: "}",
        })
    }
}

pub struct Compound<'a> {
    ser: &'a mut Serializer,
    first: bool,
    close: &'static str,
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
        self.ser.output.push_str(self.close);
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
        SerializeSeq::end(self)
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
        // Unlike JSON, keys aren't restricted to strings - whatever shape
        // `key` serializes as is written directly.
        key.serialize(&mut *self.ser)
    }
    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Error>
    where
        T: Serialize + ?Sized,
    {
        self.ser.output.push(':');
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push_str(self.close);
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
        self.ser.output.push_str(key);
        self.ser.output.push(':');
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        self.ser.output.push_str(self.close);
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
        self.ser.output.push_str(key);
        self.ser.output.push(':');
        value.serialize(&mut *self.ser)
    }
    fn end(self) -> Result<(), Error> {
        SerializeMap::end(self)
    }
}
