use crate::{Error, Number, Value};
use alloc::string::String;

pub(crate) struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        Parser {
            input: input.as_bytes(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    pub(crate) fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(byte)
    }

    pub(crate) fn error(&self, msg: impl Into<String>) -> Error {
        Error::new(msg, self.line, self.column)
    }

    pub(crate) fn error_eof(&self, msg: impl Into<String>) -> Error {
        Error::eof(msg, self.line, self.column)
    }

    pub(crate) fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.bump();
        }
    }

    /// True if the cursor has consumed the entire input.
    pub(crate) fn at_end(&self) -> bool {
        self.pos == self.input.len()
    }

    /// The number of input bytes consumed so far.
    pub(crate) fn byte_pos(&self) -> usize {
        self.pos
    }

    /// Parses a JSON number per RFC 8259 §6:
    /// `number = [ "-" ] int [ frac ] [ exp ]`.
    pub(crate) fn parse_number(&mut self) -> Result<Value, Error> {
        let start = self.pos;

        if self.peek() == Some(b'-') {
            self.bump();
        }

        match self.peek() {
            Some(b'0') => {
                self.bump();
            }
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.bump();
                }
            }
            _ => return Err(self.error("invalid number: expected a digit")),
        }

        let mut is_float = false;

        if self.peek() == Some(b'.') {
            is_float = true;
            self.bump();
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number: expected a digit after `.`"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump();
            }
        }

        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.bump();
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump();
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("invalid number: expected a digit in exponent"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump();
            }
        }

        // Safe: we only ever consumed ASCII digits and `-`/`.`/`e`/`E`/`+`.
        let raw = core::str::from_utf8(&self.input[start..self.pos]).unwrap();

        if is_float {
            let f: f64 = raw.parse().map_err(|_| self.error("invalid number"))?;
            return Number::from_f64(f)
                .map(Value::Number)
                .ok_or_else(|| self.error("number is not finite"));
        }

        let negative = raw.starts_with('-');
        let magnitude = if negative { &raw[1..] } else { raw };

        if let Ok(mag) = magnitude.parse::<u64>() {
            if !negative {
                return Ok(Value::Number(Number::from(mag)));
            }
            if mag == 1u64 << 63 {
                return Ok(Value::Number(Number::from(i64::MIN)));
            }
            if let Ok(n) = i64::try_from(mag) {
                return Ok(Value::Number(Number::from(-n)));
            }
        }

        // Integer too large for i64/u64: fall back to a lossy f64, same as
        // serde_json does for out-of-range integers.
        let f: f64 = raw.parse().map_err(|_| self.error("invalid number"))?;
        Number::from_f64(f)
            .map(Value::Number)
            .ok_or_else(|| self.error("number is not finite"))
    }

    /// Parses a JSON string per RFC 8259 §7, assuming the opening `"` has
    /// not yet been consumed.
    pub(crate) fn parse_string(&mut self) -> Result<String, Error> {
        self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error_eof("unterminated string")),
                Some(b'"') => {
                    self.bump();
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.bump();
                    self.parse_escape(&mut out)?;
                }
                Some(byte) if byte < 0x20 => {
                    return Err(self.error("unescaped control character in string"));
                }
                Some(byte) => {
                    let len = utf8_len(byte);
                    let start = self.pos;
                    for _ in 0..len {
                        if self.bump().is_none() {
                            return Err(self.error("invalid utf-8 in string"));
                        }
                    }
                    let s = core::str::from_utf8(&self.input[start..self.pos])
                        .map_err(|_| self.error("invalid utf-8 in string"))?;
                    out.push_str(s);
                }
            }
        }
    }

    fn parse_escape(&mut self, out: &mut String) -> Result<(), Error> {
        match self.bump() {
            Some(b'"') => out.push('"'),
            Some(b'\\') => out.push('\\'),
            Some(b'/') => out.push('/'),
            Some(b'b') => out.push('\u{0008}'),
            Some(b'f') => out.push('\u{000C}'),
            Some(b'n') => out.push('\n'),
            Some(b'r') => out.push('\r'),
            Some(b't') => out.push('\t'),
            Some(b'u') => {
                let cp = self.parse_hex4()?;
                if (0xD800..=0xDBFF).contains(&cp) {
                    if self.peek() != Some(b'\\') {
                        return Err(self.error("unpaired high surrogate in \\u escape"));
                    }
                    self.bump();
                    if self.bump() != Some(b'u') {
                        return Err(self.error("expected \\u low surrogate escape"));
                    }
                    let low = self.parse_hex4()?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(self.error("invalid low surrogate in \\u escape"));
                    }
                    let scalar =
                        0x10000u32 + (u32::from(cp - 0xD800) << 10) + u32::from(low - 0xDC00);
                    let ch = char::from_u32(scalar)
                        .ok_or_else(|| self.error("invalid unicode scalar value"))?;
                    out.push(ch);
                } else if (0xDC00..=0xDFFF).contains(&cp) {
                    return Err(self.error("unpaired low surrogate in \\u escape"));
                } else {
                    let ch = char::from_u32(u32::from(cp))
                        .ok_or_else(|| self.error("invalid unicode scalar value"))?;
                    out.push(ch);
                }
            }
            _ => return Err(self.error("invalid escape sequence")),
        }
        Ok(())
    }

    fn parse_hex4(&mut self) -> Result<u16, Error> {
        let mut value: u16 = 0;
        for _ in 0..4 {
            let byte = self
                .bump()
                .ok_or_else(|| self.error_eof("unexpected end of input in \\u escape"))?;
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(self.error("invalid hex digit in \\u escape")),
            };
            value = value * 16 + u16::from(digit);
        }
        Ok(value)
    }
}

/// Number of UTF-8 bytes in the character starting with `byte`, per the
/// leading-byte bit pattern. Malformed leading bytes are treated as
/// length 1 and will fail `str::from_utf8` validation at the call site.
fn utf8_len(byte: u8) -> usize {
    if byte & 0x80 == 0 {
        1
    } else if byte & 0xE0 == 0xC0 {
        2
    } else if byte & 0xF0 == 0xE0 {
        3
    } else if byte & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Map;
    use alloc::vec::Vec;

    #[test]
    fn parses_literals() {
        assert_eq!(crate::from_str::<Value>("null").unwrap(), Value::Null);
        assert_eq!(crate::from_str::<Value>("true").unwrap(), Value::Bool(true));
        assert_eq!(
            crate::from_str::<Value>("false").unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn skips_surrounding_whitespace() {
        assert_eq!(
            crate::from_str::<Value>("  \n\t null  \r\n").unwrap(),
            Value::Null
        );
    }

    #[test]
    fn rejects_invalid_literal() {
        assert!(crate::from_str::<Value>("nul").is_err());
        assert!(crate::from_str::<Value>("truee").is_err());
        assert!(crate::from_str::<Value>("nulll").is_err());
    }

    #[test]
    fn rejects_empty_input() {
        assert!(crate::from_str::<Value>("").is_err());
        assert!(crate::from_str::<Value>("   ").is_err());
    }

    #[test]
    fn error_reports_line_and_column() {
        let err = crate::from_str::<Value>("\n\n  bogus").unwrap_err();
        assert_eq!(err.line(), 3);
        assert_eq!(err.column(), 3);
    }

    fn num(s: &str) -> Value {
        crate::from_str::<Value>(s).unwrap()
    }

    #[test]
    fn parses_integers() {
        assert_eq!(num("0"), Value::Number(Number::from(0u64)));
        assert_eq!(num("42"), Value::Number(Number::from(42u64)));
        assert_eq!(num("-42"), Value::Number(Number::from(-42i64)));
        assert_eq!(num("-0"), Value::Number(Number::from(0u64)));
    }

    #[test]
    fn parses_i64_min() {
        assert_eq!(
            num("-9223372036854775808"),
            Value::Number(Number::from(i64::MIN))
        );
    }

    #[test]
    fn parses_floats() {
        assert_eq!(num("1.5"), Value::Number(Number::from_f64(1.5).unwrap()));
        assert_eq!(num("1e10"), Value::Number(Number::from_f64(1e10).unwrap()));
        assert_eq!(
            num("1.5e-2"),
            Value::Number(Number::from_f64(1.5e-2).unwrap())
        );
        assert_eq!(
            num("-1.5E+2"),
            Value::Number(Number::from_f64(-1.5e2).unwrap())
        );
    }

    #[test]
    fn rejects_leading_zero() {
        assert!(crate::from_str::<Value>("01").is_err());
        assert!(crate::from_str::<Value>("-01").is_err());
    }

    #[test]
    fn rejects_malformed_numbers() {
        assert!(crate::from_str::<Value>("-").is_err());
        assert!(crate::from_str::<Value>(".5").is_err());
        assert!(crate::from_str::<Value>("1.").is_err());
        assert!(crate::from_str::<Value>("1e").is_err());
        assert!(crate::from_str::<Value>("1e+").is_err());
        assert!(crate::from_str::<Value>("+1").is_err());
    }

    #[test]
    fn overflow_falls_back_to_f64() {
        // Larger than u64::MAX; must not error, must not panic.
        let v = num("100000000000000000000000000000");
        match v {
            Value::Number(n) => assert!(n.is_f64()),
            _ => panic!("expected a number"),
        }
    }

    fn s(input: &str) -> String {
        match crate::from_str::<Value>(input).unwrap() {
            Value::String(s) => s,
            other => panic!("expected a string, got {other:?}"),
        }
    }

    #[test]
    fn parses_plain_strings() {
        assert_eq!(s(r#""hello""#), "hello");
        assert_eq!(s(r#""""#), "");
    }

    #[test]
    fn parses_simple_escapes() {
        assert_eq!(s(r#""a\"b""#), "a\"b");
        assert_eq!(s(r#""a\\b""#), "a\\b");
        assert_eq!(s(r#""a\/b""#), "a/b");
        assert_eq!(s(r#""a\nb""#), "a\nb");
        assert_eq!(s(r#""a\tb""#), "a\tb");
        assert_eq!(s(r#""a\rb""#), "a\rb");
        assert_eq!(s(r#""a\bb""#), "a\u{0008}b");
        assert_eq!(s(r#""a\fb""#), "a\u{000C}b");
    }

    #[test]
    fn parses_unicode_escape() {
        assert_eq!(s(r#""A""#), "A");
        assert_eq!(s(r#""é""#), "\u{e9}");
    }

    #[test]
    fn parses_surrogate_pair() {
        // U+1F600 GRINNING FACE, encoded as a UTF-16 surrogate pair.
        assert_eq!(s(r#""😀""#), "\u{1F600}");
    }

    #[test]
    fn passes_through_raw_utf8() {
        assert_eq!(s("\"caf\u{e9}\""), "caf\u{e9}");
        assert_eq!(s("\"\u{1F600}\""), "\u{1F600}");
    }

    #[test]
    fn rejects_unpaired_surrogate() {
        assert!(crate::from_str::<Value>(r#""\ud83d""#).is_err());
        assert!(crate::from_str::<Value>(r#""\ude00""#).is_err());
        assert!(crate::from_str::<Value>(r#""\ud83dX""#).is_err());
    }

    #[test]
    fn rejects_unescaped_control_char() {
        assert!(crate::from_str::<Value>("\"a\nb\"").is_err());
        assert!(crate::from_str::<Value>("\"a\tb\"").is_err());
    }

    #[test]
    fn rejects_unterminated_string() {
        assert!(crate::from_str::<Value>(r#""abc"#).is_err());
    }

    #[test]
    fn rejects_invalid_escape() {
        assert!(crate::from_str::<Value>(r#""\x41""#).is_err());
    }

    #[test]
    fn parses_empty_array() {
        assert_eq!(
            crate::from_str::<Value>("[]").unwrap(),
            Value::Array(Vec::new())
        );
        assert_eq!(
            crate::from_str::<Value>("[  ]").unwrap(),
            Value::Array(Vec::new())
        );
    }

    #[test]
    fn parses_array_of_values() {
        assert_eq!(
            crate::from_str::<Value>("[1, 2, 3]").unwrap(),
            Value::Array(alloc::vec![
                Value::Number(Number::from(1u64)),
                Value::Number(Number::from(2u64)),
                Value::Number(Number::from(3u64)),
            ])
        );
    }

    #[test]
    fn parses_nested_arrays() {
        assert_eq!(
            crate::from_str::<Value>("[[1], []]").unwrap(),
            Value::Array(alloc::vec![
                Value::Array(alloc::vec![Value::Number(Number::from(1u64))]),
                Value::Array(Vec::new()),
            ])
        );
    }

    #[test]
    fn rejects_trailing_comma_in_array() {
        assert!(crate::from_str::<Value>("[1,]").is_err());
    }

    #[test]
    fn rejects_missing_comma_in_array() {
        assert!(crate::from_str::<Value>("[1 2]").is_err());
    }

    #[test]
    fn rejects_unterminated_array() {
        assert!(crate::from_str::<Value>("[1, 2").is_err());
    }

    #[test]
    fn parses_empty_object() {
        assert_eq!(
            crate::from_str::<Value>("{}").unwrap(),
            Value::Object(Map::new())
        );
        assert_eq!(
            crate::from_str::<Value>("{ }").unwrap(),
            Value::Object(Map::new())
        );
    }

    #[test]
    fn parses_object_with_entries() {
        let mut expected = Map::new();
        expected.insert(
            alloc::string::String::from("a"),
            Value::Number(Number::from(1u64)),
        );
        expected.insert(alloc::string::String::from("b"), Value::Bool(true));
        assert_eq!(
            crate::from_str::<Value>(r#"{"a": 1, "b": true}"#).unwrap(),
            Value::Object(expected)
        );
    }

    #[test]
    fn parses_nested_object() {
        assert_eq!(
            crate::from_str::<Value>(r#"{"outer": {"inner": [1, 2]}}"#).unwrap(),
            Value::Object({
                let mut m = Map::new();
                m.insert(
                    alloc::string::String::from("outer"),
                    Value::Object({
                        let mut inner = Map::new();
                        inner.insert(
                            alloc::string::String::from("inner"),
                            Value::Array(alloc::vec![
                                Value::Number(Number::from(1u64)),
                                Value::Number(Number::from(2u64)),
                            ]),
                        );
                        inner
                    }),
                );
                m
            })
        );
    }

    #[test]
    fn duplicate_keys_last_write_wins() {
        let v = crate::from_str::<Value>(r#"{"a": 1, "a": 2}"#).unwrap();
        let mut expected = Map::new();
        expected.insert(
            alloc::string::String::from("a"),
            Value::Number(Number::from(2u64)),
        );
        assert_eq!(v, Value::Object(expected));
    }

    #[test]
    fn rejects_non_string_key() {
        assert!(crate::from_str::<Value>("{1: 2}").is_err());
    }

    #[test]
    fn rejects_missing_colon() {
        assert!(crate::from_str::<Value>(r#"{"a" 1}"#).is_err());
    }

    #[test]
    fn rejects_trailing_comma_in_object() {
        assert!(crate::from_str::<Value>(r#"{"a": 1,}"#).is_err());
    }

    #[test]
    fn rejects_unterminated_object() {
        assert!(crate::from_str::<Value>(r#"{"a": 1"#).is_err());
    }

    #[test]
    fn eof_errors_classify_as_eof() {
        assert!(crate::from_str::<Value>("").unwrap_err().is_eof());
        assert!(crate::from_str::<Value>(r#""abc"#).unwrap_err().is_eof());
        assert!(crate::from_str::<Value>("[1, 2").unwrap_err().is_eof());
        assert!(crate::from_str::<Value>(r#"{"a": 1"#).unwrap_err().is_eof());
        assert!(crate::from_str::<Value>(r#"{"a""#).unwrap_err().is_eof());
        assert!(crate::from_str::<Value>(r#"{"a": 1,"#)
            .unwrap_err()
            .is_eof());
    }

    #[test]
    fn non_eof_syntax_errors_classify_as_syntax() {
        assert!(crate::from_str::<Value>("nul").unwrap_err().is_syntax());
        assert!(crate::from_str::<Value>("[1 2]").unwrap_err().is_syntax());
        assert!(crate::from_str::<Value>("{1: 2}").unwrap_err().is_syntax());
    }
}
