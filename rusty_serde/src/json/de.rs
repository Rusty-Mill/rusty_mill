use crate::de::{
    Deserialize, Deserializer as DeserializerTrait, EnumAccess, IgnoredAny, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use crate::error::Error as ErrorTrait;
use crate::forward_to_deserialize_any;
use crate::json::error::Error;

/// Parse `s` as JSON into a `T`.
pub fn from_str<'de, T>(s: &'de str) -> Result<T, Error>
where
    T: Deserialize<'de>,
{
    let mut de = Deserializer::from_str(s);
    let value = T::deserialize(&mut de)?;
    de.skip_whitespace();
    if de.pos != de.input.len() {
        return Err(de.error("trailing characters after JSON value"));
    }
    Ok(value)
}

pub struct Deserializer<'de> {
    input: &'de [u8],
    pos: usize,
}

impl<'de> Deserializer<'de> {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &'de str) -> Self {
        Deserializer {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn position(&self) -> (usize, usize) {
        let mut line = 1;
        let mut column = 1;
        for &b in &self.input[..self.pos.min(self.input.len())] {
            if b == b'\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        (line, column)
    }

    fn error(&self, msg: impl Into<String>) -> Error {
        let (line, column) = self.position();
        Error::syntax(msg, line, column)
    }

    fn skip_whitespace(&mut self) {
        while let Some(&b) = self.input.get(self.pos) {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Result<u8, Error> {
        self.skip_whitespace();
        self.input
            .get(self.pos)
            .copied()
            .ok_or_else(|| self.error("unexpected end of input"))
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), Error> {
        let found = self.peek()?;
        if found == expected {
            self.bump();
            Ok(())
        } else {
            Err(self.error(format!(
                "expected `{}`, found `{}`",
                expected as char, found as char
            )))
        }
    }

    fn parse_literal(&mut self, literal: &str) -> Result<(), Error> {
        let bytes = literal.as_bytes();
        if self.input[self.pos..].starts_with(bytes) {
            self.pos += bytes.len();
            Ok(())
        } else {
            Err(self.error(format!("expected `{literal}`")))
        }
    }

    fn parse_string(&mut self) -> Result<String, Error> {
        self.expect_byte(b'"')?;
        let mut out = String::new();
        loop {
            let b = *self
                .input
                .get(self.pos)
                .ok_or_else(|| self.error("unterminated string"))?;
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let escape = *self
                        .input
                        .get(self.pos)
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    match escape {
                        b'"' => {
                            out.push('"');
                            self.pos += 1;
                        }
                        b'\\' => {
                            out.push('\\');
                            self.pos += 1;
                        }
                        b'/' => {
                            out.push('/');
                            self.pos += 1;
                        }
                        b'b' => {
                            out.push('\u{8}');
                            self.pos += 1;
                        }
                        b'f' => {
                            out.push('\u{c}');
                            self.pos += 1;
                        }
                        b'n' => {
                            out.push('\n');
                            self.pos += 1;
                        }
                        b'r' => {
                            out.push('\r');
                            self.pos += 1;
                        }
                        b't' => {
                            out.push('\t');
                            self.pos += 1;
                        }
                        b'u' => {
                            self.pos += 1;
                            let cp = self.parse_hex4()?;
                            let ch = if (0xD800..=0xDBFF).contains(&cp) {
                                if self.input.get(self.pos) != Some(&b'\\')
                                    || self.input.get(self.pos + 1) != Some(&b'u')
                                {
                                    return Err(self.error("expected low surrogate"));
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(self.error("invalid low surrogate"));
                                }
                                let combined = 0x10000
                                    + (((cp - 0xD800) as u32) << 10)
                                    + (low - 0xDC00) as u32;
                                char::from_u32(combined)
                                    .ok_or_else(|| self.error("invalid surrogate pair"))?
                            } else {
                                char::from_u32(cp as u32)
                                    .ok_or_else(|| self.error("invalid unicode escape"))?
                            };
                            out.push(ch);
                        }
                        other => {
                            return Err(
                                self.error(format!("invalid escape character `{}`", other as char))
                            )
                        }
                    }
                }
                _ => {
                    // Copy one UTF-8 encoded scalar value at a time.
                    let start = self.pos;
                    let width = utf8_char_width(b);
                    self.pos += width;
                    let bytes = self
                        .input
                        .get(start..self.pos)
                        .ok_or_else(|| self.error("invalid UTF-8 in string"))?;
                    let s = std::str::from_utf8(bytes)
                        .map_err(|_| self.error("invalid UTF-8 in string"))?;
                    out.push_str(s);
                }
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, Error> {
        let hex = self
            .input
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("truncated unicode escape"))?;
        let s = std::str::from_utf8(hex).map_err(|_| self.error("invalid unicode escape"))?;
        let value = u16::from_str_radix(s, 16).map_err(|_| self.error("invalid unicode escape"))?;
        self.pos += 4;
        Ok(value)
    }

    fn parse_number_raw(&mut self) -> Result<(&'de str, bool), Error> {
        let start = self.pos;
        let mut is_float = false;
        if self.input.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.input.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.error("invalid number")),
        }
        if self.input.get(self.pos) == Some(&b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number"));
            }
            while matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.input.get(self.pos), Some(b'e') | Some(b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.input.get(self.pos), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number"));
            }
            while matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.input[start..self.pos]).unwrap();
        Ok((text, is_float))
    }

    fn parse_array<V>(&mut self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.expect_byte(b'[')?;
        let value = visitor.visit_seq(SeqWalker {
            de: self,
            started: false,
        })?;
        self.skip_whitespace();
        self.expect_byte(b']')?;
        Ok(value)
    }

    fn parse_object<V>(&mut self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.expect_byte(b'{')?;
        let value = visitor.visit_map(MapWalker {
            de: self,
            started: false,
        })?;
        self.skip_whitespace();
        self.expect_byte(b'}')?;
        Ok(value)
    }

    fn end_tagged_object(&mut self) -> Result<(), Error> {
        self.skip_whitespace();
        self.expect_byte(b'}')
    }
}

fn utf8_char_width(byte: u8) -> usize {
    if byte & 0x80 == 0 {
        1
    } else if byte & 0xE0 == 0xC0 {
        2
    } else if byte & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

struct SeqWalker<'a, 'de> {
    de: &'a mut Deserializer<'de>,
    started: bool,
}

impl<'a, 'de> SeqAccess<'de> for SeqWalker<'a, 'de> {
    type Error = Error;

    fn next_element<T>(&mut self) -> Result<Option<T>, Error>
    where
        T: Deserialize<'de>,
    {
        if self.de.peek()? == b']' {
            return Ok(None);
        }
        if self.started {
            self.de.expect_byte(b',')?;
        }
        self.started = true;
        self.de.skip_whitespace();
        if self.de.peek()? == b']' {
            return Err(self.de.error("trailing comma in array"));
        }
        T::deserialize(&mut *self.de).map(Some)
    }
}

struct MapWalker<'a, 'de> {
    de: &'a mut Deserializer<'de>,
    started: bool,
}

impl<'a, 'de> MapAccess<'de> for MapWalker<'a, 'de> {
    type Error = Error;

    fn next_key<K>(&mut self) -> Result<Option<K>, Error>
    where
        K: Deserialize<'de>,
    {
        if self.de.peek()? == b'}' {
            return Ok(None);
        }
        if self.started {
            self.de.expect_byte(b',')?;
        }
        self.started = true;
        self.de.skip_whitespace();
        if self.de.peek()? == b'}' {
            return Err(self.de.error("trailing comma in object"));
        }
        K::deserialize(&mut *self.de).map(Some)
    }

    fn next_value<V>(&mut self) -> Result<V, Error>
    where
        V: Deserialize<'de>,
    {
        self.de.skip_whitespace();
        self.de.expect_byte(b':')?;
        V::deserialize(&mut *self.de)
    }
}

struct StrDeserializer<'a> {
    value: &'a str,
}

impl<'de, 'a> DeserializerTrait<'de> for StrDeserializer<'a> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.value)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.value)
    }

    forward_to_deserialize_any! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_f32 deserialize_f64 deserialize_char deserialize_str deserialize_string
        deserialize_bytes deserialize_byte_buf deserialize_option deserialize_unit
        deserialize_unit_struct deserialize_newtype_struct deserialize_seq deserialize_tuple
        deserialize_tuple_struct deserialize_map deserialize_struct deserialize_enum
        deserialize_ignored_any
    }
}

struct UnitOnlyVariantAccess<'a> {
    de: PhantomStrDeserializer<'a>,
}

struct PhantomStrDeserializer<'a>(&'a str);

impl<'de, 'a> EnumAccess<'de> for StrDeserializer<'a> {
    type Error = Error;
    type Variant = UnitOnlyVariantAccess<'a>;

    fn variant<V>(self) -> Result<(V, Self::Variant), Error>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(StrDeserializer { value: self.value })?;
        Ok((
            value,
            UnitOnlyVariantAccess {
                de: PhantomStrDeserializer(self.value),
            },
        ))
    }
}

impl<'de, 'a> VariantAccess<'de> for UnitOnlyVariantAccess<'a> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }
    fn newtype_variant<T>(self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        Err(Error::custom(format!(
            "expected newtype variant, found unit variant `{}`",
            self.de.0
        )))
    }
    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        Err(Error::custom(format!(
            "expected tuple variant, found unit variant `{}`",
            self.de.0
        )))
    }
    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        Err(Error::custom(format!(
            "expected struct variant, found unit variant `{}`",
            self.de.0
        )))
    }
}

struct TaggedEnumAccess<'a, 'de> {
    de: &'a mut Deserializer<'de>,
    variant: String,
}

impl<'a, 'de> EnumAccess<'de> for TaggedEnumAccess<'a, 'de> {
    type Error = Error;
    type Variant = TaggedVariantAccess<'a, 'de>;

    fn variant<V>(self) -> Result<(V, Self::Variant), Error>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(StrDeserializer {
            value: &self.variant,
        })?;
        Ok((value, TaggedVariantAccess { de: self.de }))
    }
}

struct TaggedVariantAccess<'a, 'de> {
    de: &'a mut Deserializer<'de>,
}

impl<'a, 'de> VariantAccess<'de> for TaggedVariantAccess<'a, 'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        self.de.skip_whitespace();
        self.de.expect_byte(b':')?;
        // Conventionally serialized as `null`, but any value is skipped.
        skip_value(self.de)?;
        self.de.end_tagged_object()
    }
    fn newtype_variant<T>(self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        self.de.skip_whitespace();
        self.de.expect_byte(b':')?;
        let value = T::deserialize(&mut *self.de)?;
        self.de.end_tagged_object()?;
        Ok(value)
    }
    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.de.skip_whitespace();
        self.de.expect_byte(b':')?;
        let value = self.de.parse_array(visitor)?;
        self.de.end_tagged_object()?;
        Ok(value)
    }
    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.de.skip_whitespace();
        self.de.expect_byte(b':')?;
        let value = self.de.parse_object(visitor)?;
        self.de.end_tagged_object()?;
        Ok(value)
    }
}

fn skip_value(de: &mut Deserializer) -> Result<(), Error> {
    struct Ignore;
    impl<'de> Visitor<'de> for Ignore {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "any value")
        }
        fn visit_bool<E>(self, _v: bool) -> Result<(), E> {
            Ok(())
        }
        fn visit_i64<E>(self, _v: i64) -> Result<(), E> {
            Ok(())
        }
        fn visit_u64<E>(self, _v: u64) -> Result<(), E> {
            Ok(())
        }
        fn visit_f64<E>(self, _v: f64) -> Result<(), E> {
            Ok(())
        }
        fn visit_str<E>(self, _v: &str) -> Result<(), E> {
            Ok(())
        }
        fn visit_unit<E>(self) -> Result<(), E> {
            Ok(())
        }
        fn visit_none<E>(self) -> Result<(), E> {
            Ok(())
        }
        fn visit_some<D>(self, deserializer: D) -> Result<(), D::Error>
        where
            D: DeserializerTrait<'de>,
        {
            deserializer.deserialize_any(Ignore)
        }
        fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
        where
            A: SeqAccess<'de>,
        {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(())
        }
        fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
        where
            A: MapAccess<'de>,
        {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            Ok(())
        }
    }
    de.deserialize_any(Ignore)
}

// ---- Internally-tagged enum support ----
//
// Every other `deserialize_*` method above can hand its `Visitor` the
// input as soon as it recognizes the shape, because JSON's own grammar
// tells you everything you need to know as you go. Internal tagging
// breaks that: `{"<tag>": "Variant", "a": 1, "b": 2}` might have the tag
// key anywhere in the object, so there's no way to know *which* variant's
// fields you're about to read until you've already read (and so have to
// buffer) every entry. `Buffered` is a minimal in-memory JSON tree for
// exactly that purpose, and `ValueDeserializer` lets the ordinary
// `Deserialize` machinery run against it exactly as it would against the
// live token stream.

enum Buffered {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Str(String),
    Seq(Vec<Buffered>),
    Map(Vec<(String, Buffered)>),
}

/// Parses one JSON value into a `Buffered` tree instead of visiting it -
/// the buffering counterpart to `deserialize_any`.
fn parse_buffered(de: &mut Deserializer) -> Result<Buffered, Error> {
    match de.peek()? {
        b'n' => {
            de.parse_literal("null")?;
            Ok(Buffered::Null)
        }
        b't' => {
            de.parse_literal("true")?;
            Ok(Buffered::Bool(true))
        }
        b'f' => {
            de.parse_literal("false")?;
            Ok(Buffered::Bool(false))
        }
        b'"' => Ok(Buffered::Str(de.parse_string()?)),
        b'[' => {
            de.bump();
            let mut items = Vec::new();
            loop {
                de.skip_whitespace();
                if de.peek()? == b']' {
                    de.bump();
                    break;
                }
                if !items.is_empty() {
                    de.expect_byte(b',')?;
                    de.skip_whitespace();
                }
                items.push(parse_buffered(de)?);
            }
            Ok(Buffered::Seq(items))
        }
        b'{' => {
            de.bump();
            let mut entries = Vec::new();
            loop {
                de.skip_whitespace();
                if de.peek()? == b'}' {
                    de.bump();
                    break;
                }
                if !entries.is_empty() {
                    de.expect_byte(b',')?;
                    de.skip_whitespace();
                }
                let key = de.parse_string()?;
                de.skip_whitespace();
                de.expect_byte(b':')?;
                entries.push((key, parse_buffered(de)?));
            }
            Ok(Buffered::Map(entries))
        }
        b'-' | b'0'..=b'9' => {
            let (text, is_float) = de.parse_number_raw()?;
            if is_float {
                Ok(Buffered::Float(
                    text.parse().map_err(|_| de.error("invalid number"))?,
                ))
            } else if let Ok(v) = text.parse::<i64>() {
                Ok(Buffered::Signed(v))
            } else if let Ok(v) = text.parse::<u64>() {
                Ok(Buffered::Unsigned(v))
            } else {
                Ok(Buffered::Float(
                    text.parse().map_err(|_| de.error("invalid number"))?,
                ))
            }
        }
        other => Err(de.error(format!("unexpected character `{}`", other as char))),
    }
}

/// A `Deserializer` over an already-parsed `Buffered` tree rather than the
/// live byte stream. Since `Buffered` owns everything (no borrowed data),
/// this works for any `'de`. Most methods forward to `deserialize_any`
/// (`Buffered` is already fully self-describing, same as the live
/// deserializer) - only the handful that need `Buffered`-specific logic
/// are written out.
struct ValueDeserializer {
    value: Buffered,
}

impl<'de> DeserializerTrait<'de> for ValueDeserializer {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Buffered::Null => visitor.visit_unit(),
            Buffered::Bool(b) => visitor.visit_bool(b),
            Buffered::Signed(v) => visitor.visit_i64(v),
            Buffered::Unsigned(v) => visitor.visit_u64(v),
            Buffered::Float(v) => visitor.visit_f64(v),
            Buffered::Str(s) => visitor.visit_string(s),
            Buffered::Seq(items) => visitor.visit_seq(BufferedSeqAccess {
                items: items.into_iter(),
            }),
            Buffered::Map(entries) => visitor.visit_map(BufferedMapAccess {
                entries: entries.into_iter(),
                pending_value: None,
            }),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Buffered::Null => visitor.visit_none(),
            other => visitor.visit_some(ValueDeserializer { value: other }),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Buffered::Str(s) => visitor.visit_str(&s),
            _ => Err(Error::custom("expected a string identifier")),
        }
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Buffered::Str(s) => visitor.visit_enum(StrDeserializer { value: &s }),
            Buffered::Map(mut entries) if entries.len() == 1 => {
                let (variant, value) = entries.remove(0);
                visitor.visit_enum(BufferedTaggedEnumAccess { variant, value })
            }
            _ => Err(Error::custom(
                "expected string or single-entry object for enum",
            )),
        }
    }

    forward_to_deserialize_any! {
        deserialize_bool deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64
        deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64
        deserialize_f32 deserialize_f64 deserialize_char deserialize_str deserialize_string
        deserialize_bytes deserialize_byte_buf deserialize_unit
        deserialize_unit_struct deserialize_newtype_struct deserialize_seq deserialize_tuple
        deserialize_tuple_struct deserialize_map deserialize_struct
        deserialize_ignored_any
    }
}

struct BufferedSeqAccess {
    items: std::vec::IntoIter<Buffered>,
}

impl<'de> SeqAccess<'de> for BufferedSeqAccess {
    type Error = Error;

    fn next_element<T>(&mut self) -> Result<Option<T>, Error>
    where
        T: Deserialize<'de>,
    {
        match self.items.next() {
            Some(v) => T::deserialize(ValueDeserializer { value: v }).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

struct BufferedMapAccess {
    entries: std::vec::IntoIter<(String, Buffered)>,
    pending_value: Option<Buffered>,
}

impl<'de> MapAccess<'de> for BufferedMapAccess {
    type Error = Error;

    fn next_key<K>(&mut self) -> Result<Option<K>, Error>
    where
        K: Deserialize<'de>,
    {
        match self.entries.next() {
            Some((k, v)) => {
                self.pending_value = Some(v);
                K::deserialize(StrDeserializer { value: &k }).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value<V>(&mut self) -> Result<V, Error>
    where
        V: Deserialize<'de>,
    {
        let value = self
            .pending_value
            .take()
            .expect("next_value called without a preceding next_key");
        V::deserialize(ValueDeserializer { value })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

/// `EnumAccess`/`VariantAccess` for an ordinary externally-tagged enum
/// found *inside* a buffered tree (e.g. one field's value, nested inside
/// an internally-tagged enum's own fields).
struct BufferedTaggedEnumAccess {
    variant: String,
    value: Buffered,
}

impl<'de> EnumAccess<'de> for BufferedTaggedEnumAccess {
    type Error = Error;
    type Variant = BufferedTaggedVariantAccess;

    fn variant<V>(self) -> Result<(V, Self::Variant), Error>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(StrDeserializer {
            value: &self.variant,
        })?;
        Ok((value, BufferedTaggedVariantAccess { value: self.value }))
    }
}

struct BufferedTaggedVariantAccess {
    value: Buffered,
}

impl<'de> VariantAccess<'de> for BufferedTaggedVariantAccess {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }
    fn newtype_variant<T>(self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        T::deserialize(ValueDeserializer { value: self.value })
    }
    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Buffered::Seq(items) => visitor.visit_seq(BufferedSeqAccess {
                items: items.into_iter(),
            }),
            _ => Err(Error::custom("expected an array for a tuple variant")),
        }
    }
    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Buffered::Map(entries) => visitor.visit_map(BufferedMapAccess {
                entries: entries.into_iter(),
                pending_value: None,
            }),
            _ => Err(Error::custom("expected an object for a struct variant")),
        }
    }
}

/// `EnumAccess`/`VariantAccess` for the *outer* internally-tagged enum
/// itself: `entries` is the buffered object with the tag entry already
/// removed, and every remaining entry becomes a field the variant's own
/// `VariantAccess::struct_variant` walks via `BufferedMapAccess`.
struct InternalTagEnumAccess {
    variant: String,
    entries: Vec<(String, Buffered)>,
}

impl<'de> EnumAccess<'de> for InternalTagEnumAccess {
    type Error = Error;
    type Variant = InternalTagVariantAccess;

    fn variant<V>(self) -> Result<(V, Self::Variant), Error>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(StrDeserializer {
            value: &self.variant,
        })?;
        Ok((
            value,
            InternalTagVariantAccess {
                entries: self.entries,
            },
        ))
    }
}

struct InternalTagVariantAccess {
    entries: Vec<(String, Buffered)>,
}

impl<'de> VariantAccess<'de> for InternalTagVariantAccess {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        // Any fields alongside the tag are ignored, consistent with how
        // unknown object keys are ignored everywhere else.
        Ok(())
    }
    fn newtype_variant<T>(self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        Err(Error::custom(
            "internally tagged enums do not support newtype variants",
        ))
    }
    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        Err(Error::custom(
            "internally tagged enums do not support tuple variants",
        ))
    }
    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(BufferedMapAccess {
            entries: self.entries.into_iter(),
            pending_value: None,
        })
    }
}

impl<'de> DeserializerTrait<'de> for &mut Deserializer<'de> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.peek()? {
            b'n' => {
                self.parse_literal("null")?;
                visitor.visit_unit()
            }
            b't' => {
                self.parse_literal("true")?;
                visitor.visit_bool(true)
            }
            b'f' => {
                self.parse_literal("false")?;
                visitor.visit_bool(false)
            }
            b'"' => {
                let s = self.parse_string()?;
                visitor.visit_string(s)
            }
            b'[' => self.parse_array(visitor),
            b'{' => self.parse_object(visitor),
            b'-' | b'0'..=b'9' => {
                let (text, is_float) = self.parse_number_raw()?;
                if is_float {
                    let v: f64 = text.parse().map_err(|_| self.error("invalid number"))?;
                    visitor.visit_f64(v)
                } else if let Ok(v) = text.parse::<i64>() {
                    visitor.visit_i64(v)
                } else if let Ok(v) = text.parse::<u64>() {
                    visitor.visit_u64(v)
                } else {
                    let v: f64 = text.parse().map_err(|_| self.error("invalid number"))?;
                    visitor.visit_f64(v)
                }
            }
            other => Err(self.error(format!("unexpected character `{}`", other as char))),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.peek()? {
            b't' => {
                self.parse_literal("true")?;
                visitor.visit_bool(true)
            }
            b'f' => {
                self.parse_literal("false")?;
                visitor.visit_bool(false)
            }
            _ => Err(self.error("expected a boolean")),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }
    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }
    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }
    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.peek()?;
        let (text, is_float) = self.parse_number_raw()?;
        if is_float {
            return Err(self.error("expected an integer, found a float"));
        }
        let v: i64 = text
            .parse()
            .map_err(|_| self.error("integer out of range"))?;
        visitor.visit_i64(v)
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }
    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }
    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }
    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.peek()?;
        let (text, is_float) = self.parse_number_raw()?;
        if is_float {
            return Err(self.error("expected an integer, found a float"));
        }
        let v: u64 = text
            .parse()
            .map_err(|_| self.error("integer out of range"))?;
        visitor.visit_u64(v)
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_f64(visitor)
    }
    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.peek()?;
        let (text, _) = self.parse_number_raw()?;
        let v: f64 = text.parse().map_err(|_| self.error("invalid number"))?;
        visitor.visit_f64(v)
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }
    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_string(visitor)
    }
    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.peek()?;
        let s = self.parse_string()?;
        visitor.visit_string(s)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_byte_buf(visitor)
    }
    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.expect_byte(b'[')?;
        let mut bytes = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek()? == b']' {
                self.bump();
                break;
            }
            if !bytes.is_empty() {
                self.expect_byte(b',')?;
                self.skip_whitespace();
            }
            let (text, is_float) = self.parse_number_raw()?;
            if is_float {
                return Err(self.error("expected a byte"));
            }
            let v: u8 = text.parse().map_err(|_| self.error("byte out of range"))?;
            bytes.push(v);
        }
        visitor.visit_byte_buf(bytes)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        if self.peek()? == b'n' {
            self.parse_literal("null")?;
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_literal("null")?;
        visitor.visit_unit()
    }
    fn deserialize_unit_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }
    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_array(visitor)
    }
    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_array(visitor)
    }
    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_array(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_object(visitor)
    }
    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_object(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.peek()? {
            b'"' => {
                let variant = self.parse_string()?;
                visitor.visit_enum(StrDeserializer { value: &variant })
            }
            b'{' => {
                self.bump();
                self.skip_whitespace();
                let variant = self.parse_string()?;
                visitor.visit_enum(TaggedEnumAccess { de: self, variant })
            }
            _ => Err(self.error("expected string or object for enum")),
        }
    }

    fn deserialize_internally_tagged_enum<V>(
        self,
        _name: &'static str,
        tag: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.expect_byte(b'{')?;
        let mut entries: Vec<(String, Buffered)> = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek()? == b'}' {
                self.bump();
                break;
            }
            if !entries.is_empty() {
                self.expect_byte(b',')?;
                self.skip_whitespace();
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            let value = parse_buffered(self)?;
            entries.push((key, value));
        }

        let tag_index = entries
            .iter()
            .position(|(k, _)| k == tag)
            .ok_or_else(|| self.error(format!("missing tag field `{tag}`")))?;
        let (_, tag_value) = entries.remove(tag_index);
        let variant = match tag_value {
            Buffered::Str(s) => s,
            _ => return Err(self.error(format!("tag field `{tag}` must be a string"))),
        };
        visitor.visit_enum(InternalTagEnumAccess { variant, entries })
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_string(visitor)
    }
    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        skip_value(self)?;
        visitor.visit_unit()
    }
}
