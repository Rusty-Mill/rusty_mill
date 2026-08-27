use crate::de::{
    Deserialize, Deserializer as DeserializerTrait, EnumAccess, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use crate::forward_to_deserialize_any;
use crate::ron::error::Error;

/// Parse `s` (in this crate's RON-like format) into a `T`.
pub fn from_str<'de, T>(s: &'de str) -> Result<T, Error>
where
    T: Deserialize<'de>,
{
    let mut de = Deserializer::from_str(s);
    let value = T::deserialize(&mut de)?;
    de.skip_whitespace();
    if de.pos != de.input.len() {
        return Err(de.error("trailing characters after value"));
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

    fn error(&self, msg: impl Into<String>) -> Error {
        Error::syntax(msg, self.pos)
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

    /// A bare, unquoted identifier - used for `true`/`false`/`None`/`Some`,
    /// enum variant tags, and (unlike JSON) struct field names. Always
    /// borrowed straight from the input: identifiers never contain escapes,
    /// so there's nothing that would force an owned copy.
    fn parse_ident(&mut self) -> Result<&'de str, Error> {
        self.skip_whitespace();
        let start = self.pos;
        match self.input.get(self.pos) {
            Some(&b) if b.is_ascii_alphabetic() || b == b'_' => self.pos += 1,
            _ => return Err(self.error("expected an identifier")),
        }
        while matches!(self.input.get(self.pos), Some(b) if b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.pos += 1;
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| self.error("invalid UTF-8 in identifier"))
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
                            self.expect_byte(b'{')?;
                            let start = self.pos;
                            while matches!(self.input.get(self.pos), Some(b) if b.is_ascii_hexdigit())
                            {
                                self.pos += 1;
                            }
                            let hex = std::str::from_utf8(&self.input[start..self.pos])
                                .map_err(|_| self.error("invalid unicode escape"))?;
                            let cp = u32::from_str_radix(hex, 16)
                                .map_err(|_| self.error("invalid unicode escape"))?;
                            self.expect_byte(b'}')?;
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| self.error("invalid unicode escape"))?;
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

    fn parse_char(&mut self) -> Result<char, Error> {
        self.expect_byte(b'\'')?;
        let b = *self
            .input
            .get(self.pos)
            .ok_or_else(|| self.error("unterminated char literal"))?;
        let ch = if b == b'\\' {
            self.pos += 1;
            let escape = *self
                .input
                .get(self.pos)
                .ok_or_else(|| self.error("unterminated escape"))?;
            self.pos += 1;
            match escape {
                b'\'' => '\'',
                b'\\' => '\\',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                other => {
                    return Err(self.error(format!("invalid escape character `{}`", other as char)))
                }
            }
        } else {
            let start = self.pos;
            let width = utf8_char_width(b);
            self.pos += width;
            let bytes = self
                .input
                .get(start..self.pos)
                .ok_or_else(|| self.error("invalid UTF-8 in char literal"))?;
            let s = std::str::from_utf8(bytes).map_err(|_| self.error("invalid UTF-8"))?;
            s.chars()
                .next()
                .ok_or_else(|| self.error("empty char literal"))?
        };
        self.expect_byte(b'\'')?;
        Ok(ch)
    }

    fn parse_number_raw(&mut self) -> Result<(&'de str, bool), Error> {
        let start = self.pos;
        let mut is_float = false;
        if self.input.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        if !matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
            return Err(self.error("invalid number"));
        }
        while matches!(self.input.get(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
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

    fn parse_seq_bracket<V>(&mut self, open: u8, close: u8, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.expect_byte(open)?;
        let value = visitor.visit_seq(SeqWalker {
            de: self,
            started: false,
            close,
        })?;
        self.skip_whitespace();
        self.expect_byte(close)?;
        Ok(value)
    }

    fn parse_map_brace<V>(&mut self, visitor: V) -> Result<V::Value, Error>
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
    close: u8,
}

impl<'a, 'de> SeqAccess<'de> for SeqWalker<'a, 'de> {
    type Error = Error;

    fn next_element<T>(&mut self) -> Result<Option<T>, Error>
    where
        T: Deserialize<'de>,
    {
        self.de.skip_whitespace();
        if self.de.peek()? == self.close {
            return Ok(None);
        }
        if self.started {
            self.de.expect_byte(b',')?;
            self.de.skip_whitespace();
            if self.de.peek()? == self.close {
                return Ok(None);
            }
        }
        self.started = true;
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
        self.de.skip_whitespace();
        if self.de.peek()? == b'}' {
            return Ok(None);
        }
        if self.started {
            self.de.expect_byte(b',')?;
            self.de.skip_whitespace();
            if self.de.peek()? == b'}' {
                return Ok(None);
            }
        }
        self.started = true;
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

/// Feeds a single already-parsed identifier (an enum variant tag, or a
/// struct field name) back through `Deserialize` - e.g. to let the
/// derive-generated field/variant identifier enum match against it via
/// `deserialize_identifier`.
struct IdentDeserializer<'de> {
    value: &'de str,
}

impl<'de> DeserializerTrait<'de> for IdentDeserializer<'de> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value)
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

struct VariantTagAccess<'a, 'de> {
    de: &'a mut Deserializer<'de>,
    variant: &'de str,
}

impl<'a, 'de> EnumAccess<'de> for VariantTagAccess<'a, 'de> {
    type Error = Error;
    type Variant = VariantDataAccess<'a, 'de>;

    fn variant<V>(self) -> Result<(V, Self::Variant), Error>
    where
        V: Deserialize<'de>,
    {
        let value = V::deserialize(IdentDeserializer {
            value: self.variant,
        })?;
        Ok((value, VariantDataAccess { de: self.de }))
    }
}

struct VariantDataAccess<'a, 'de> {
    de: &'a mut Deserializer<'de>,
}

impl<'a, 'de> VariantAccess<'de> for VariantDataAccess<'a, 'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }
    fn newtype_variant<T>(self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        self.de.expect_byte(b'(')?;
        let value = T::deserialize(&mut *self.de)?;
        self.de.skip_whitespace();
        self.de.expect_byte(b')')?;
        Ok(value)
    }
    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.de.parse_seq_bracket(b'[', b']', visitor)
    }
    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.de.parse_map_brace(visitor)
    }
}

impl<'de> DeserializerTrait<'de> for &mut Deserializer<'de> {
    type Error = Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.peek()? {
            b'"' => {
                let s = self.parse_string()?;
                visitor.visit_string(s)
            }
            b'\'' => {
                let c = self.parse_char()?;
                visitor.visit_char(c)
            }
            b'[' => self.parse_seq_bracket(b'[', b']', visitor),
            b'{' => self.parse_map_brace(visitor),
            b'(' => {
                self.bump();
                self.skip_whitespace();
                if self.peek()? == b')' {
                    self.bump();
                    return visitor.visit_unit();
                }
                let value = visitor.visit_seq(SeqWalker {
                    de: self,
                    started: false,
                    close: b')',
                })?;
                self.skip_whitespace();
                self.expect_byte(b')')?;
                Ok(value)
            }
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
            b if b.is_ascii_alphabetic() || b == b'_' => {
                let ident = self.parse_ident()?;
                match ident {
                    "true" => visitor.visit_bool(true),
                    "false" => visitor.visit_bool(false),
                    "None" => visitor.visit_none(),
                    "Some" => {
                        self.expect_byte(b'(')?;
                        let value = visitor.visit_some(&mut *self)?;
                        self.skip_whitespace();
                        self.expect_byte(b')')?;
                        Ok(value)
                    }
                    tag => match self.peek() {
                        Ok(b'(') => {
                            self.bump();
                            let value = self.deserialize_any(visitor)?;
                            self.skip_whitespace();
                            self.expect_byte(b')')?;
                            Ok(value)
                        }
                        Ok(b'[') => self.parse_seq_bracket(b'[', b']', visitor),
                        Ok(b'{') => self.parse_map_brace(visitor),
                        _ => visitor.visit_borrowed_str(tag),
                    },
                }
            }
            other => Err(self.error(format!("unexpected character `{}`", other as char))),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        match self.parse_ident()? {
            "true" => visitor.visit_bool(true),
            "false" => visitor.visit_bool(false),
            other => Err(self.error(format!("expected `true`/`false`, found `{other}`"))),
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
            return Err(self.error("invalid number"));
        }
        let v: i64 = text.parse().map_err(|_| self.error("invalid number"))?;
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
            return Err(self.error("invalid number"));
        }
        let v: u64 = text.parse().map_err(|_| self.error("invalid number"))?;
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
        let c = self.parse_char()?;
        visitor.visit_char(c)
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
                return Err(self.error("invalid byte"));
            }
            let v: u8 = text.parse().map_err(|_| self.error("invalid byte"))?;
            bytes.push(v);
        }
        visitor.visit_byte_buf(bytes)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.skip_whitespace();
        if self.input[self.pos..].starts_with(b"None") {
            self.pos += 4;
            visitor.visit_none()
        } else {
            self.parse_literal("Some")?;
            self.expect_byte(b'(')?;
            let value = visitor.visit_some(&mut *self)?;
            self.skip_whitespace();
            self.expect_byte(b')')?;
            Ok(value)
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.expect_byte(b'(')?;
        self.skip_whitespace();
        self.expect_byte(b')')?;
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
        self.parse_seq_bracket(b'[', b']', visitor)
    }
    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_seq_bracket(b'[', b']', visitor)
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
        self.parse_seq_bracket(b'[', b']', visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        self.parse_map_brace(visitor)
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
        self.parse_map_brace(visitor)
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
        let variant = self.parse_ident()?;
        visitor.visit_enum(VariantTagAccess { de: self, variant })
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        // Normally a bare identifier (how a struct's own known fields are
        // written), but `#[rusty_serde(flatten)]` re-serializes the
        // non-flattened fields alongside the flattened ones through the
        // generic map-entry path, which quotes keys like any other string -
        // so a field name has to be accepted in either form here.
        if self.peek()? == b'"' {
            let s = self.parse_string()?;
            visitor.visit_string(s)
        } else {
            let ident = self.parse_ident()?;
            visitor.visit_borrowed_str(ident)
        }
    }
    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Error>
    where
        V: Visitor<'de>,
    {
        skip_value(self)?;
        visitor.visit_unit()
    }
}

/// Consumes and discards one value's worth of tokens, for
/// `deserialize_ignored_any` (skipping over unknown struct fields and map
/// entries the target type doesn't care about). This works directly against
/// the raw grammar rather than through `Visitor`/`EnumAccess`, since there's
/// no typed value to build - just syntax to walk past.
fn skip_value(de: &mut Deserializer) -> Result<(), Error> {
    match de.peek()? {
        b'"' => {
            de.parse_string()?;
            Ok(())
        }
        b'\'' => {
            de.parse_char()?;
            Ok(())
        }
        b'[' => skip_seq_like(de, b'[', b']'),
        b'{' => skip_map_like(de),
        b'(' => skip_seq_like(de, b'(', b')'),
        b'-' | b'0'..=b'9' => {
            de.parse_number_raw()?;
            Ok(())
        }
        b if b.is_ascii_alphabetic() || b == b'_' => {
            let ident = de.parse_ident()?;
            match ident {
                "Some" => {
                    de.expect_byte(b'(')?;
                    skip_value(de)?;
                    de.skip_whitespace();
                    de.expect_byte(b')')
                }
                // A bare word: `true`/`false`/`None`, a unit enum variant,
                // or (followed by data) a tagged variant - whatever data
                // follows is skipped the same way any other value is.
                _ => match de.peek() {
                    Ok(b'(') => skip_seq_like(de, b'(', b')'),
                    Ok(b'[') => skip_seq_like(de, b'[', b']'),
                    Ok(b'{') => skip_map_like(de),
                    _ => Ok(()),
                },
            }
        }
        other => Err(de.error(format!("unexpected character `{}`", other as char))),
    }
}

/// Skips a bracketed, comma-separated list of values: `(...)` and `[...]`.
fn skip_seq_like(de: &mut Deserializer, open: u8, close: u8) -> Result<(), Error> {
    de.expect_byte(open)?;
    de.skip_whitespace();
    if de.peek()? == close {
        de.bump();
        return Ok(());
    }
    loop {
        skip_value(de)?;
        de.skip_whitespace();
        match de.peek()? {
            b',' => {
                de.bump();
                de.skip_whitespace();
                if de.peek()? == close {
                    de.bump();
                    return Ok(());
                }
            }
            b if b == close => {
                de.bump();
                return Ok(());
            }
            _ => return Err(de.error("expected `,` or a closing bracket")),
        }
    }
}

/// Skips a brace-delimited, comma-separated list of `key:value` pairs.
fn skip_map_like(de: &mut Deserializer) -> Result<(), Error> {
    de.expect_byte(b'{')?;
    de.skip_whitespace();
    if de.peek()? == b'}' {
        de.bump();
        return Ok(());
    }
    loop {
        skip_value(de)?;
        de.skip_whitespace();
        de.expect_byte(b':')?;
        skip_value(de)?;
        de.skip_whitespace();
        match de.peek()? {
            b',' => {
                de.bump();
                de.skip_whitespace();
                if de.peek()? == b'}' {
                    de.bump();
                    return Ok(());
                }
            }
            b'}' => {
                de.bump();
                return Ok(());
            }
            _ => return Err(de.error("expected `,` or `}`")),
        }
    }
}
