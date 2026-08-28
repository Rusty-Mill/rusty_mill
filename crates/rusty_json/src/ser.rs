use crate::formatter::{CharEscape, CompactFormatter, Formatter, PrettyFormatter};
use crate::{Error, Number};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
};

/// Serializes any `Serialize` value to a compact JSON string.
pub fn to_string<T>(value: &T) -> Result<String, Error>
where
    T: Serialize + ?Sized,
{
    to_string_with_formatter(value, CompactFormatter)
}

/// Serializes any `Serialize` value to a pretty-printed JSON string,
/// indented two spaces per level. Empty arrays/objects are rendered inline
/// (`[]`, `{}`) rather than spread across lines.
pub fn to_string_pretty<T>(value: &T) -> Result<String, Error>
where
    T: Serialize + ?Sized,
{
    to_string_with_formatter(value, PrettyFormatter::new())
}

/// Serializes any `Serialize` value to a JSON string using a custom
/// [`Formatter`] (e.g. custom indentation, HTML-safe escaping).
pub fn to_string_with_formatter<T, F>(value: &T, formatter: F) -> Result<String, Error>
where
    T: Serialize + ?Sized,
    F: Formatter,
{
    let mut serializer = Serializer::with_formatter(formatter);
    value.serialize(&mut serializer)?;
    Ok(serializer.into_string())
}

/// Serializes any `Serialize` value to a compact JSON byte vector.
pub fn to_vec<T>(value: &T) -> Result<Vec<u8>, Error>
where
    T: Serialize + ?Sized,
{
    to_string(value).map(String::into_bytes)
}

/// Serializes any `Serialize` value to a pretty-printed JSON byte vector.
pub fn to_vec_pretty<T>(value: &T) -> Result<Vec<u8>, Error>
where
    T: Serialize + ?Sized,
{
    to_string_pretty(value).map(String::into_bytes)
}

/// Serializes any `Serialize` value as compact JSON directly to a
/// [`std::io::Write`] sink.
#[cfg(feature = "std")]
pub fn to_writer<W, T>(mut writer: W, value: &T) -> Result<(), Error>
where
    W: std::io::Write,
    T: Serialize + ?Sized,
{
    writer.write_all(to_string(value)?.as_bytes())?;
    Ok(())
}

/// Serializes any `Serialize` value as pretty-printed JSON directly to a
/// [`std::io::Write`] sink.
#[cfg(feature = "std")]
pub fn to_writer_pretty<W, T>(mut writer: W, value: &T) -> Result<(), Error>
where
    W: std::io::Write,
    T: Serialize + ?Sized,
{
    writer.write_all(to_string_pretty(value)?.as_bytes())?;
    Ok(())
}

/// A `serde::Serializer` writing JSON into an in-memory `String`, with
/// syntax (whitespace, indentation, escaping) controlled by a [`Formatter`]
/// (defaulting to [`CompactFormatter`], matching [`to_string`]). Construct
/// via [`Serializer::with_formatter`] to plug in a custom one.
pub struct Serializer<F = CompactFormatter> {
    output: String,
    formatter: F,
}

impl Serializer<CompactFormatter> {
    /// A serializer producing compact output, same as [`to_string`].
    pub fn new() -> Self {
        Serializer::with_formatter(CompactFormatter)
    }
}

impl Default for Serializer<CompactFormatter> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Formatter> Serializer<F> {
    /// A serializer using a custom [`Formatter`].
    pub fn with_formatter(formatter: F) -> Self {
        Serializer {
            output: String::new(),
            formatter,
        }
    }

    /// Consumes the serializer, returning the JSON text written so far.
    pub fn into_string(self) -> String {
        self.output
    }
}

fn write_escaped_str<F: Formatter>(formatter: &mut F, out: &mut String, value: &str) {
    formatter.begin_string(out);
    let mut start = 0;
    for (i, c) in value.char_indices() {
        let escape = match c {
            '"' => Some(CharEscape::Quote),
            '\\' => Some(CharEscape::ReverseSolidus),
            '\u{0008}' => Some(CharEscape::Backspace),
            '\u{000C}' => Some(CharEscape::FormFeed),
            '\n' => Some(CharEscape::LineFeed),
            '\r' => Some(CharEscape::CarriageReturn),
            '\t' => Some(CharEscape::Tab),
            c if (c as u32) < 0x20 => Some(CharEscape::AsciiControl(c as u8)),
            _ => None,
        };
        if let Some(escape) = escape {
            if start < i {
                formatter.write_string_fragment(out, &value[start..i]);
            }
            formatter.write_char_escape(out, escape);
            start = i + c.len_utf8();
        }
    }
    if start < value.len() {
        formatter.write_string_fragment(out, &value[start..]);
    }
    formatter.end_string(out);
}

impl<'a, F: Formatter> ser::Serializer for &'a mut Serializer<F> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Compound<'a, F>;
    type SerializeTuple = Compound<'a, F>;
    type SerializeTupleStruct = Compound<'a, F>;
    type SerializeTupleVariant = Compound<'a, F>;
    type SerializeMap = Compound<'a, F>;
    type SerializeStruct = Compound<'a, F>;
    type SerializeStructVariant = Compound<'a, F>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        self.formatter.write_bool(&mut self.output, v);
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        self.formatter
            .write_number_str(&mut self.output, &Number::from(v).to_string());
        Ok(())
    }
    fn serialize_i128(self, v: i128) -> Result<(), Error> {
        match Number::from_i128(v) {
            Some(n) => {
                self.formatter
                    .write_number_str(&mut self.output, &n.to_string());
                Ok(())
            }
            None => Err(<Error as ser::Error>::custom("i128 value out of range")),
        }
    }

    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        self.serialize_u64(u64::from(v))
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        self.formatter
            .write_number_str(&mut self.output, &Number::from(v).to_string());
        Ok(())
    }
    fn serialize_u128(self, v: u128) -> Result<(), Error> {
        match Number::from_u128(v) {
            Some(n) => {
                self.formatter
                    .write_number_str(&mut self.output, &n.to_string());
                Ok(())
            }
            None => Err(<Error as ser::Error>::custom("u128 value out of range")),
        }
    }

    fn serialize_f32(self, v: f32) -> Result<(), Error> {
        self.serialize_f64(f64::from(v))
    }
    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        match Number::from_f64(v) {
            Some(n) => self
                .formatter
                .write_number_str(&mut self.output, &n.to_string()),
            None => self.formatter.write_null(&mut self.output),
        }
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<(), Error> {
        let mut buf = [0u8; 4];
        self.serialize_str(v.encode_utf8(&mut buf))
    }

    fn serialize_str(self, v: &str) -> Result<(), Error> {
        write_escaped_str(&mut self.formatter, &mut self.output, v);
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        // JSON has no byte-string type; serialize as an array of numbers,
        // same as serde_json's default (non-serde_bytes) behavior.
        let mut seq = self.serialize_seq(Some(v.len()))?;
        for byte in v {
            SerializeSeq::serialize_element(&mut seq, byte)?;
        }
        SerializeSeq::end(seq)
    }

    fn serialize_none(self) -> Result<(), Error> {
        self.formatter.write_null(&mut self.output);
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Error> {
        self.formatter.write_null(&mut self.output);
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.begin_variant_envelope(variant);
        value.serialize(&mut *self)?;
        self.end_variant_envelope();
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Compound<'a, F>, Error> {
        self.formatter.begin_array(&mut self.output);
        Ok(Compound {
            ser: self,
            first: true,
            wrap: None,
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Compound<'a, F>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Compound<'a, F>, Error> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a, F>, Error> {
        self.begin_variant_envelope(variant);
        self.formatter.begin_array(&mut self.output);
        Ok(Compound {
            ser: self,
            first: true,
            wrap: Some(Wrap::Array),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Compound<'a, F>, Error> {
        self.formatter.begin_object(&mut self.output);
        Ok(Compound {
            ser: self,
            first: true,
            wrap: None,
        })
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<Compound<'a, F>, Error> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Compound<'a, F>, Error> {
        self.begin_variant_envelope(variant);
        self.formatter.begin_object(&mut self.output);
        Ok(Compound {
            ser: self,
            first: true,
            wrap: Some(Wrap::Object),
        })
    }

    fn collect_str<T: ?Sized + core::fmt::Display>(self, value: &T) -> Result<(), Error> {
        self.serialize_str(&alloc::format!("{value}"))
    }
}

impl<F: Formatter> Serializer<F> {
    /// Starts the `{"variant": ` envelope wrapping a newtype/tuple/struct
    /// enum variant's payload.
    fn begin_variant_envelope(&mut self, variant: &str) {
        self.formatter.begin_object(&mut self.output);
        self.formatter.begin_object_key(&mut self.output, true);
        write_escaped_str(&mut self.formatter, &mut self.output, variant);
        self.formatter.end_object_key(&mut self.output);
        self.formatter.begin_object_value(&mut self.output);
    }

    /// Closes the envelope opened by [`Self::begin_variant_envelope`].
    fn end_variant_envelope(&mut self) {
        self.formatter.end_object_value(&mut self.output);
        self.formatter.end_object(&mut self.output, false);
    }
}

/// What an enum-variant compound wraps its inner `[...]`/`{...}` with, once
/// it closes: the rest of the `{"variant": ...}` envelope.
enum Wrap {
    Array,
    Object,
}

/// The shared implementation behind every `Serialize{Seq,Tuple,TupleStruct,
/// TupleVariant,Map,Struct,StructVariant}` — all of them are either a
/// comma-separated `[...]` / `{...}`, optionally wrapped in an enum-variant
/// envelope. Not constructible directly; produced by [`Serializer`]'s
/// `serde::Serializer` impl.
pub struct Compound<'a, F> {
    ser: &'a mut Serializer<F>,
    first: bool,
    wrap: Option<Wrap>,
}

impl<F: Formatter> Compound<'_, F> {
    fn close(self, array: bool) -> Result<(), Error> {
        let empty = self.first;
        if array {
            self.ser.formatter.end_array(&mut self.ser.output, empty);
        } else {
            self.ser.formatter.end_object(&mut self.ser.output, empty);
        }
        match self.wrap {
            Some(Wrap::Array) | Some(Wrap::Object) => self.ser.end_variant_envelope(),
            None => {}
        }
        Ok(())
    }
}

impl<F: Formatter> SerializeSeq for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        self.ser
            .formatter
            .begin_array_value(&mut self.ser.output, self.first);
        self.first = false;
        value.serialize(&mut *self.ser)?;
        self.ser.formatter.end_array_value(&mut self.ser.output);
        Ok(())
    }

    fn end(self) -> Result<(), Error> {
        self.close(true)
    }
}

impl<F: Formatter> SerializeTuple for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Error> {
        self.close(true)
    }
}

impl<F: Formatter> SerializeTupleStruct for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Error> {
        self.close(true)
    }
}

impl<F: Formatter> SerializeTupleVariant for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<(), Error> {
        self.close(true)
    }
}

impl<F: Formatter> SerializeMap for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Error> {
        self.ser
            .formatter
            .begin_object_key(&mut self.ser.output, self.first);
        self.first = false;
        let mut key_str = String::new();
        key.serialize(MapKeySerializer {
            output: &mut key_str,
        })?;
        write_escaped_str(&mut self.ser.formatter, &mut self.ser.output, &key_str);
        self.ser.formatter.end_object_key(&mut self.ser.output);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Error> {
        self.ser.formatter.begin_object_value(&mut self.ser.output);
        value.serialize(&mut *self.ser)?;
        self.ser.formatter.end_object_value(&mut self.ser.output);
        Ok(())
    }

    fn end(self) -> Result<(), Error> {
        self.close(false)
    }
}

impl<F: Formatter> SerializeStruct for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.ser
            .formatter
            .begin_object_key(&mut self.ser.output, self.first);
        self.first = false;
        write_escaped_str(&mut self.ser.formatter, &mut self.ser.output, key);
        self.ser.formatter.end_object_key(&mut self.ser.output);
        self.ser.formatter.begin_object_value(&mut self.ser.output);
        value.serialize(&mut *self.ser)?;
        self.ser.formatter.end_object_value(&mut self.ser.output);
        Ok(())
    }

    fn end(self) -> Result<(), Error> {
        self.close(false)
    }
}

impl<F: Formatter> SerializeStructVariant for Compound<'_, F> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<(), Error> {
        self.close(false)
    }
}

/// A restricted `Serializer` used only for map keys, since JSON object keys
/// must be strings. Accepts string-like and primitive scalar types
/// (stringified), rejects anything else.
struct MapKeySerializer<'a> {
    output: &'a mut String,
}

impl ser::Serializer for MapKeySerializer<'_> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = ser::Impossible<(), Error>;
    type SerializeTuple = ser::Impossible<(), Error>;
    type SerializeTupleStruct = ser::Impossible<(), Error>;
    type SerializeTupleVariant = ser::Impossible<(), Error>;
    type SerializeMap = ser::Impossible<(), Error>;
    type SerializeStruct = ser::Impossible<(), Error>;
    type SerializeStructVariant = ser::Impossible<(), Error>;

    fn serialize_str(self, v: &str) -> Result<(), Error> {
        self.output.push_str(v);
        Ok(())
    }

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        self.output.push_str(if v { "true" } else { "false" });
        Ok(())
    }
    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }
    fn serialize_char(self, v: char) -> Result<(), Error> {
        self.output.push(v);
        Ok(())
    }

    fn serialize_f32(self, _v: f32) -> Result<(), Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_f64(self, _v: f64) -> Result<(), Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<(), Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_none(self) -> Result<(), Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Error> {
        Err(<Error as ser::Error>::custom("map key must be a string"))
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
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<(), Error> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{from_str, Map, Value};
    use alloc::string::String as AString;
    use alloc::vec::Vec;
    use serde::Serialize;

    #[test]
    fn serializes_value_scalars() {
        assert_eq!(to_string(&Value::Null).unwrap(), "null");
        assert_eq!(to_string(&Value::Bool(true)).unwrap(), "true");
        assert_eq!(to_string(&Value::Bool(false)).unwrap(), "false");
        assert_eq!(
            to_string(&Value::Number(Number::from(42u64))).unwrap(),
            "42"
        );
        assert_eq!(
            to_string(&Value::String(AString::from("hi"))).unwrap(),
            "\"hi\""
        );
    }

    #[test]
    fn escapes_special_characters() {
        assert_eq!(
            to_string(&Value::String(AString::from("a\"b\\c\nd\te"))).unwrap(),
            r#""a\"b\\c\nd\te""#
        );
        assert_eq!(
            to_string(&Value::String(AString::from("\u{0001}"))).unwrap(),
            "\"\\u0001\""
        );
    }

    #[test]
    fn serializes_array_and_object() {
        assert_eq!(to_string(&Value::Array(Vec::new())).unwrap(), "[]");
        assert_eq!(to_string(&Value::Object(Map::new())).unwrap(), "{}");

        let arr = Value::Array(alloc::vec![
            Value::Number(Number::from(1u64)),
            Value::Bool(true)
        ]);
        assert_eq!(to_string(&arr).unwrap(), "[1,true]");

        let mut map = Map::new();
        map.insert(AString::from("a"), Value::Number(Number::from(1u64)));
        assert_eq!(to_string(&Value::Object(map)).unwrap(), r#"{"a":1}"#);
    }

    #[test]
    fn pretty_prints_scalars_like_compact() {
        assert_eq!(to_string_pretty(&Value::Null).unwrap(), "null");
        assert_eq!(
            to_string_pretty(&Value::Number(Number::from(1u64))).unwrap(),
            "1"
        );
    }

    #[test]
    fn pretty_prints_empty_containers_inline() {
        assert_eq!(to_string_pretty(&Value::Array(Vec::new())).unwrap(), "[]");
        assert_eq!(to_string_pretty(&Value::Object(Map::new())).unwrap(), "{}");
    }

    #[test]
    fn pretty_prints_array_with_indentation() {
        let arr = Value::Array(alloc::vec![
            Value::Number(Number::from(1u64)),
            Value::Number(Number::from(2u64)),
        ]);
        assert_eq!(to_string_pretty(&arr).unwrap(), "[\n  1,\n  2\n]");
    }

    #[test]
    fn pretty_prints_nested_object() {
        let mut inner = Map::new();
        inner.insert(AString::from("b"), Value::Bool(true));
        let mut outer = Map::new();
        outer.insert(AString::from("a"), Value::Object(inner));
        assert_eq!(
            to_string_pretty(&Value::Object(outer)).unwrap(),
            "{\n  \"a\": {\n    \"b\": true\n  }\n}"
        );
    }

    #[test]
    fn pretty_output_round_trips() {
        let value = from_str::<Value>(r#"{"a":[1,2,{"b":null}]}"#).unwrap();
        let pretty = to_string_pretty(&value).unwrap();
        assert_eq!(from_str::<Value>(&pretty).unwrap(), value);
    }

    #[test]
    fn round_trips_through_parser() {
        let inputs = [
            r#"null"#,
            r#"true"#,
            r#"42"#,
            r#"-1.5"#,
            r#""hello\nworld""#,
            r#"[1,2,3]"#,
            r#"{"a":1,"b":[true,null]}"#,
        ];
        for input in inputs {
            let value = from_str::<Value>(input).unwrap();
            let serialized = to_string(&value).unwrap();
            let reparsed = from_str::<Value>(&serialized).unwrap();
            assert_eq!(value, reparsed, "round-trip failed for {input}");
        }
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
    fn serializes_derived_struct() {
        let p = Point { x: 1, y: -2 };
        assert_eq!(to_string(&p).unwrap(), r#"{"x":1,"y":-2}"#);
    }

    #[test]
    fn serializes_derived_enum_variants() {
        assert_eq!(to_string(&Shape::Unit).unwrap(), r#""Unit""#);
        assert_eq!(to_string(&Shape::Newtype(5)).unwrap(), r#"{"Newtype":5}"#);
        assert_eq!(
            to_string(&Shape::Tuple(1, 2)).unwrap(),
            r#"{"Tuple":[1,2]}"#
        );
        assert_eq!(
            to_string(&Shape::Struct { w: 3, h: 4 }).unwrap(),
            r#"{"Struct":{"w":3,"h":4}}"#
        );
    }

    #[test]
    fn pretty_prints_enum_variants() {
        assert_eq!(
            to_string_pretty(&Shape::Newtype(5)).unwrap(),
            "{\n  \"Newtype\": 5\n}"
        );
        assert_eq!(
            to_string_pretty(&Shape::Tuple(1, 2)).unwrap(),
            "{\n  \"Tuple\": [\n    1,\n    2\n  ]\n}"
        );
    }

    #[test]
    fn serializes_option_and_collections() {
        assert_eq!(to_string(&Some(5i32)).unwrap(), "5");
        assert_eq!(to_string(&Option::<i32>::None).unwrap(), "null");
        assert_eq!(to_string(&alloc::vec![1, 2, 3]).unwrap(), "[1,2,3]");
        assert_eq!(to_string(&(1, "a", true)).unwrap(), r#"[1,"a",true]"#);

        let mut map: alloc::collections::BTreeMap<AString, i32> =
            alloc::collections::BTreeMap::new();
        map.insert(AString::from("a"), 1);
        map.insert(AString::from("b"), 2);
        assert_eq!(to_string(&map).unwrap(), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn round_trips_through_serde_json_for_derived_types() {
        let p = Point { x: 7, y: 8 };
        let ours = to_string(&p).unwrap();
        let theirs = serde_json::to_string(&serde_json::json!({"x": 7, "y": 8})).unwrap();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn to_vec_matches_to_string_bytes() {
        let v = Value::Array(alloc::vec![Value::Bool(true), Value::Null]);
        assert_eq!(to_vec(&v).unwrap(), to_string(&v).unwrap().into_bytes());
        assert_eq!(
            to_vec_pretty(&v).unwrap(),
            to_string_pretty(&v).unwrap().into_bytes()
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn to_writer_matches_to_string_bytes() {
        let v = Value::Array(alloc::vec![Value::Bool(true), Value::Null]);

        let mut buf = std::vec::Vec::new();
        to_writer(&mut buf, &v).unwrap();
        assert_eq!(buf, to_string(&v).unwrap().into_bytes());

        let mut buf_pretty = std::vec::Vec::new();
        to_writer_pretty(&mut buf_pretty, &v).unwrap();
        assert_eq!(buf_pretty, to_string_pretty(&v).unwrap().into_bytes());
    }

    #[cfg(feature = "std")]
    #[test]
    fn to_writer_propagates_io_errors() {
        struct FailingWriter;
        impl std::io::Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("boom"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let err = to_writer(FailingWriter, &Value::Null).unwrap_err();
        assert!(err.is_io());
    }

    /// A custom formatter: same as compact, but always renders array/object
    /// separators with a trailing space after the comma (`, `/`: `),
    /// proving `Formatter` is genuinely pluggable end-to-end.
    #[derive(Default)]
    struct SpacedFormatter;

    impl Formatter for SpacedFormatter {
        fn begin_array_value(&mut self, out: &mut String, first: bool) {
            if !first {
                out.push_str(", ");
            }
        }

        fn begin_object_key(&mut self, out: &mut String, first: bool) {
            if !first {
                out.push_str(", ");
            }
        }

        fn begin_object_value(&mut self, out: &mut String) {
            out.push_str(": ");
        }
    }

    #[test]
    fn custom_formatter_is_used() {
        let arr = Value::Array(alloc::vec![
            Value::Number(Number::from(1u64)),
            Value::Number(Number::from(2u64)),
        ]);
        assert_eq!(
            to_string_with_formatter(&arr, SpacedFormatter).unwrap(),
            "[1, 2]"
        );

        let mut map = Map::new();
        map.insert(AString::from("a"), Value::Number(Number::from(1u64)));
        map.insert(AString::from("b"), Value::Number(Number::from(2u64)));
        assert_eq!(
            to_string_with_formatter(&Value::Object(map), SpacedFormatter).unwrap(),
            r#"{"a": 1, "b": 2}"#
        );
    }

    #[test]
    fn pretty_formatter_with_custom_indent_width() {
        let arr = Value::Array(alloc::vec![Value::Number(Number::from(1u64))]);
        assert_eq!(
            to_string_with_formatter(&arr, PrettyFormatter::with_indent_width(4)).unwrap(),
            "[\n    1\n]"
        );
    }
}
