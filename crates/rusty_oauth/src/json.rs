//! A minimal, hand-rolled JSON parser/serializer.
//!
//! OAuth token, introspection, and metadata responses (RFC 6749 §5.1,
//! RFC 7662, RFC 8414) are JSON objects, and JWTs (RFC 7519) encode their
//! header/claims as JSON. This module implements just enough of RFC 8259
//! to read and write them, without pulling in `serde`.

use std::fmt;

/// A JSON value. Objects preserve insertion order (via `Vec`, not a hash
/// map) so re-serialization is deterministic.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64()
            .filter(|n| *n >= 0.0 && n.fract() == 0.0)
            .map(|n| n as u64)
    }

    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn object<I: IntoIterator<Item = (String, Value)>>(entries: I) -> Value {
        Value::Object(entries.into_iter().collect())
    }

    /// Serializes this value to compact JSON text.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out);
        out
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Number(n as f64)
    }
}

impl From<u64> for Value {
    fn from(n: u64) -> Self {
        Value::Number(n as f64)
    }
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() && n.abs() < 1e15 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{}", n));
            }
        }
        Value::String(s) => write_json_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_value(v, out);
            }
            out.push('}');
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JSON parse error at byte {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Parses a complete JSON document into a [`Value`].
pub fn parse(input: &str) -> Result<Value, ParseError> {
    let bytes = input.as_bytes();
    let mut pos = 0usize;
    skip_ws(bytes, &mut pos);
    let value = parse_value(bytes, &mut pos)?;
    skip_ws(bytes, &mut pos);
    if pos != bytes.len() {
        return Err(ParseError {
            message: "trailing characters after JSON value".to_string(),
            position: pos,
        });
    }
    Ok(value)
}

fn err(message: &str, position: usize) -> ParseError {
    ParseError {
        message: message.to_string(),
        position,
    }
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn parse_value(bytes: &[u8], pos: &mut usize) -> Result<Value, ParseError> {
    skip_ws(bytes, pos);
    if *pos >= bytes.len() {
        return Err(err("unexpected end of input", *pos));
    }
    match bytes[*pos] {
        b'{' => parse_object(bytes, pos),
        b'[' => parse_array(bytes, pos),
        b'"' => parse_string(bytes, pos).map(Value::String),
        b't' => parse_literal(bytes, pos, "true", Value::Bool(true)),
        b'f' => parse_literal(bytes, pos, "false", Value::Bool(false)),
        b'n' => parse_literal(bytes, pos, "null", Value::Null),
        b'-' | b'0'..=b'9' => parse_number(bytes, pos),
        _ => Err(err("unexpected character", *pos)),
    }
}

fn parse_literal(
    bytes: &[u8],
    pos: &mut usize,
    lit: &str,
    value: Value,
) -> Result<Value, ParseError> {
    let lit_bytes = lit.as_bytes();
    if bytes.len() - *pos < lit_bytes.len() || &bytes[*pos..*pos + lit_bytes.len()] != lit_bytes {
        return Err(err(&format!("expected `{lit}`"), *pos));
    }
    *pos += lit_bytes.len();
    Ok(value)
}

fn parse_object(bytes: &[u8], pos: &mut usize) -> Result<Value, ParseError> {
    *pos += 1; // consume '{'
    let mut entries = Vec::new();
    skip_ws(bytes, pos);
    if *pos < bytes.len() && bytes[*pos] == b'}' {
        *pos += 1;
        return Ok(Value::Object(entries));
    }
    loop {
        skip_ws(bytes, pos);
        if *pos >= bytes.len() || bytes[*pos] != b'"' {
            return Err(err("expected string key", *pos));
        }
        let key = parse_string(bytes, pos)?;
        skip_ws(bytes, pos);
        if *pos >= bytes.len() || bytes[*pos] != b':' {
            return Err(err("expected ':'", *pos));
        }
        *pos += 1;
        let value = parse_value(bytes, pos)?;
        entries.push((key, value));
        skip_ws(bytes, pos);
        match bytes.get(*pos) {
            Some(b',') => {
                *pos += 1;
            }
            Some(b'}') => {
                *pos += 1;
                break;
            }
            _ => return Err(err("expected ',' or '}'", *pos)),
        }
    }
    Ok(Value::Object(entries))
}

fn parse_array(bytes: &[u8], pos: &mut usize) -> Result<Value, ParseError> {
    *pos += 1; // consume '['
    let mut items = Vec::new();
    skip_ws(bytes, pos);
    if *pos < bytes.len() && bytes[*pos] == b']' {
        *pos += 1;
        return Ok(Value::Array(items));
    }
    loop {
        let value = parse_value(bytes, pos)?;
        items.push(value);
        skip_ws(bytes, pos);
        match bytes.get(*pos) {
            Some(b',') => {
                *pos += 1;
            }
            Some(b']') => {
                *pos += 1;
                break;
            }
            _ => return Err(err("expected ',' or ']'", *pos)),
        }
    }
    Ok(Value::Array(items))
}

fn parse_string(bytes: &[u8], pos: &mut usize) -> Result<String, ParseError> {
    *pos += 1; // consume opening quote
    let mut out = String::new();
    loop {
        if *pos >= bytes.len() {
            return Err(err("unterminated string", *pos));
        }
        match bytes[*pos] {
            b'"' => {
                *pos += 1;
                return Ok(out);
            }
            b'\\' => {
                *pos += 1;
                if *pos >= bytes.len() {
                    return Err(err("unterminated escape", *pos));
                }
                match bytes[*pos] {
                    b'"' => {
                        out.push('"');
                        *pos += 1;
                    }
                    b'\\' => {
                        out.push('\\');
                        *pos += 1;
                    }
                    b'/' => {
                        out.push('/');
                        *pos += 1;
                    }
                    b'b' => {
                        out.push('\u{0008}');
                        *pos += 1;
                    }
                    b'f' => {
                        out.push('\u{000C}');
                        *pos += 1;
                    }
                    b'n' => {
                        out.push('\n');
                        *pos += 1;
                    }
                    b'r' => {
                        out.push('\r');
                        *pos += 1;
                    }
                    b't' => {
                        out.push('\t');
                        *pos += 1;
                    }
                    b'u' => {
                        *pos += 1;
                        let cp = parse_hex4(bytes, pos)?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            // High surrogate: expect a following \uXXXX low surrogate.
                            if bytes.get(*pos) != Some(&b'\\') || bytes.get(*pos + 1) != Some(&b'u')
                            {
                                return Err(err("unpaired UTF-16 surrogate", *pos));
                            }
                            *pos += 2;
                            let low = parse_hex4(bytes, pos)?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(err("invalid low surrogate", *pos));
                            }
                            let c = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                            let c =
                                char::from_u32(c).ok_or_else(|| err("invalid codepoint", *pos))?;
                            out.push(c);
                        } else {
                            let c =
                                char::from_u32(cp).ok_or_else(|| err("invalid codepoint", *pos))?;
                            out.push(c);
                        }
                    }
                    _ => return Err(err("invalid escape character", *pos)),
                }
            }
            b => {
                // Consume one UTF-8 codepoint's worth of bytes.
                let width = utf8_width(b);
                if *pos + width > bytes.len() {
                    return Err(err("truncated UTF-8 sequence", *pos));
                }
                let s = std::str::from_utf8(&bytes[*pos..*pos + width])
                    .map_err(|_| err("invalid UTF-8", *pos))?;
                out.push_str(s);
                *pos += width;
            }
        }
    }
}

fn utf8_width(first_byte: u8) -> usize {
    if first_byte & 0x80 == 0 {
        1
    } else if first_byte & 0xE0 == 0xC0 {
        2
    } else if first_byte & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

fn parse_hex4(bytes: &[u8], pos: &mut usize) -> Result<u32, ParseError> {
    if *pos + 4 > bytes.len() {
        return Err(err("truncated unicode escape", *pos));
    }
    let s = std::str::from_utf8(&bytes[*pos..*pos + 4])
        .map_err(|_| err("invalid unicode escape", *pos))?;
    let v = u32::from_str_radix(s, 16).map_err(|_| err("invalid unicode escape", *pos))?;
    *pos += 4;
    Ok(v)
}

fn parse_number(bytes: &[u8], pos: &mut usize) -> Result<Value, ParseError> {
    let start = *pos;
    if bytes.get(*pos) == Some(&b'-') {
        *pos += 1;
    }
    while bytes.get(*pos).is_some_and(|b| b.is_ascii_digit()) {
        *pos += 1;
    }
    if bytes.get(*pos) == Some(&b'.') {
        *pos += 1;
        while bytes.get(*pos).is_some_and(|b| b.is_ascii_digit()) {
            *pos += 1;
        }
    }
    if matches!(bytes.get(*pos), Some(b'e') | Some(b'E')) {
        *pos += 1;
        if matches!(bytes.get(*pos), Some(b'+') | Some(b'-')) {
            *pos += 1;
        }
        while bytes.get(*pos).is_some_and(|b| b.is_ascii_digit()) {
            *pos += 1;
        }
    }
    let s = std::str::from_utf8(&bytes[start..*pos]).unwrap();
    s.parse::<f64>()
        .map(Value::Number)
        .map_err(|_| err("invalid number", start))
}

/// Convenience: builds a flat `Value::Object` from string-keyed pairs,
/// skipping any entry whose value is `Value::Null` -- useful for the
/// optional fields common in OAuth requests/responses.
pub fn object_skip_null(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_response() {
        let json = r#"{
            "access_token": "2YotnFZFEjr1zCsicMWpAA",
            "token_type": "example",
            "expires_in": 3600,
            "refresh_token": "tGzv3JOkF0XG5Qx2TlKWIA",
            "example_parameter": "example_value"
        }"#;
        let v = parse(json).unwrap();
        assert_eq!(
            v.get("access_token").unwrap().as_str(),
            Some("2YotnFZFEjr1zCsicMWpAA")
        );
        assert_eq!(v.get("expires_in").unwrap().as_u64(), Some(3600));
    }

    #[test]
    fn roundtrip_serialize() {
        let v = Value::object([
            ("a".to_string(), Value::from("b")),
            ("n".to_string(), Value::from(42i64)),
            ("t".to_string(), Value::Bool(true)),
            ("z".to_string(), Value::Null),
            (
                "arr".to_string(),
                Value::Array(vec![Value::from(1i64), Value::from(2i64)]),
            ),
        ]);
        let s = v.to_json();
        let parsed = parse(&s).unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn escapes_strings() {
        let v = Value::from("line1\nline2\t\"quoted\"\\");
        let s = v.to_json();
        let parsed = parse(&s).unwrap();
        assert_eq!(parsed, v);
    }

    #[test]
    fn parses_unicode_escape() {
        let v = parse(r#""café""#).unwrap();
        assert_eq!(v.as_str(), Some("café"));
    }

    #[test]
    fn parses_surrogate_pair() {
        // U+1F600 GRINNING FACE, as a UTF-16 surrogate pair.
        let v = parse(r#""😀""#).unwrap();
        assert_eq!(v.as_str(), Some("\u{1F600}"));
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("{}x").is_err());
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse("{\"a\":}").is_err());
        assert!(parse("[1,2,]").is_err());
    }

    #[test]
    fn parses_nested_metadata_document() {
        let json = r#"{
            "issuer": "https://example.com",
            "authorization_endpoint": "https://example.com/authorize",
            "token_endpoint": "https://example.com/token",
            "response_types_supported": ["code", "token"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"]
        }"#;
        let v = parse(json).unwrap();
        let types = v
            .get("response_types_supported")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].as_str(), Some("code"));
    }
}
