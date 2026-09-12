//! Parses and writes [`Value`] directly to/from JSON text, with **no
//! dependency on `serde`** -- available even with the `serde` feature
//! disabled. This mirrors [`crate::de`]/[`crate::ser`]'s behavior for
//! `Value` specifically (same grammar, same error messages/classification),
//! implemented as a direct recursive walk instead of going through a
//! `serde::Deserializer`/`Serializer`.

use crate::escape::write_escaped_str;
use crate::formatter::{CompactFormatter, Formatter, PrettyFormatter};
use crate::parser::Parser;
use crate::{Error, Map, Value};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Parses a whole JSON text into a [`Value`]; errors if any non-whitespace
/// trailing content follows the value.
pub(crate) fn parse_str(s: &str) -> Result<Value, Error> {
    let mut parser = Parser::new(s);
    let value = parse_value(&mut parser)?;
    parser.skip_whitespace();
    if !parser.at_end() {
        return Err(parser.error("trailing characters after JSON value"));
    }
    Ok(value)
}

/// Parses one JSON value starting at the parser's current position,
/// consuming up through (but not past) the value's last byte.
fn parse_value(parser: &mut Parser) -> Result<Value, Error> {
    parser.skip_whitespace();
    match parser.peek() {
        Some(b'n') => {
            consume_literal(parser, "null")?;
            Ok(Value::Null)
        }
        Some(b't') => {
            consume_literal(parser, "true")?;
            Ok(Value::Bool(true))
        }
        Some(b'f') => {
            consume_literal(parser, "false")?;
            Ok(Value::Bool(false))
        }
        Some(b'-' | b'0'..=b'9') => parser.parse_number(),
        Some(b'"') => parser.parse_string().map(Value::String),
        Some(b'[') => parse_array(parser),
        Some(b'{') => parse_object(parser),
        Some(other) => {
            Err(parser.error(alloc::format!("unexpected character `{}`", other as char)))
        }
        None => Err(parser.error_eof("unexpected end of input")),
    }
}

fn consume_literal(parser: &mut Parser, literal: &str) -> Result<(), Error> {
    // Matches `crate::de`'s classification: running out of input mid-literal
    // is a syntax error, not EOF -- an incomplete literal like `nul` is
    // unambiguously wrong, unlike a value cut off where more input could
    // still make it valid.
    for expected in literal.bytes() {
        match parser.bump() {
            Some(byte) if byte == expected => {}
            _ => return Err(parser.error(alloc::format!("invalid literal, expected `{literal}`"))),
        }
    }
    Ok(())
}

fn parse_array(parser: &mut Parser) -> Result<Value, Error> {
    parser.enter_nesting()?;
    parser.bump(); // opening `[`
    let mut items = Vec::new();
    let mut first = true;
    loop {
        parser.skip_whitespace();
        if parser.peek() == Some(b']') {
            break;
        }
        if !first {
            match parser.bump() {
                Some(b',') => parser.skip_whitespace(),
                None => return Err(parser.error_eof("unexpected end of input in array")),
                _ => return Err(parser.error("expected `,` or `]` in array")),
            }
            // A trailing comma leaves `]` right after the comma.
            if parser.peek() == Some(b']') {
                return Err(parser.error("expected `,` or `]` in array"));
            }
        }
        first = false;
        items.push(parse_value(parser)?);
    }
    parser.skip_whitespace();
    parser.exit_nesting();
    match parser.bump() {
        Some(b']') => Ok(Value::Array(items)),
        None => Err(parser.error_eof("unexpected end of input in array")),
        _ => Err(parser.error("expected `,` or `]` in array")),
    }
}

fn parse_object(parser: &mut Parser) -> Result<Value, Error> {
    parser.enter_nesting()?;
    parser.bump(); // opening `{`
    let mut map = Map::new();
    let mut first = true;
    loop {
        parser.skip_whitespace();
        if parser.peek() == Some(b'}') {
            break;
        }
        if !first {
            match parser.bump() {
                Some(b',') => parser.skip_whitespace(),
                None => return Err(parser.error_eof("unexpected end of input in object")),
                _ => return Err(parser.error("expected `,` or `}` in object")),
            }
            if parser.peek() == Some(b'}') {
                return Err(parser.error("expected `,` or `}` in object"));
            }
        }
        first = false;
        match parser.peek() {
            Some(b'"') => {}
            None => return Err(parser.error_eof("unexpected end of input in object")),
            _ => return Err(parser.error("expected string key in object")),
        }
        let key = parser.parse_string()?;
        parser.skip_whitespace();
        match parser.bump() {
            Some(b':') => {}
            None => return Err(parser.error_eof("unexpected end of input in object")),
            _ => return Err(parser.error("expected `:` after object key")),
        }
        let value = parse_value(parser)?;
        map.insert(key, value);
    }
    parser.skip_whitespace();
    parser.exit_nesting();
    match parser.bump() {
        Some(b'}') => Ok(Value::Object(map)),
        None => Err(parser.error_eof("unexpected end of input in object")),
        _ => Err(parser.error("expected `,` or `}` in object")),
    }
}

/// Writes `value` as compact JSON text.
pub(crate) fn to_json_string(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut CompactFormatter, &mut out);
    out
}

/// Writes `value` as pretty-printed JSON text, indented two spaces per
/// level. Empty arrays/objects render inline (`[]`, `{}`).
pub(crate) fn to_json_string_pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut PrettyFormatter::new(), &mut out);
    out
}

fn write_value<F: Formatter>(value: &Value, formatter: &mut F, out: &mut String) {
    match value {
        Value::Null => formatter.write_null(out),
        Value::Bool(b) => formatter.write_bool(out, *b),
        Value::Number(n) => formatter.write_number_str(out, &n.to_string()),
        Value::String(s) => write_escaped_str(formatter, out, s),
        Value::Array(items) => {
            formatter.begin_array(out);
            for (i, item) in items.iter().enumerate() {
                formatter.begin_array_value(out, i == 0);
                write_value(item, formatter, out);
                formatter.end_array_value(out);
            }
            formatter.end_array(out, items.is_empty());
        }
        Value::Object(map) => {
            formatter.begin_object(out);
            for (i, (key, val)) in map.iter().enumerate() {
                formatter.begin_object_key(out, i == 0);
                write_escaped_str(formatter, out, key);
                formatter.end_object_key(out);
                formatter.begin_object_value(out);
                write_value(val, formatter, out);
                formatter.end_object_value(out);
            }
            formatter.end_object(out, map.is_empty());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Number;

    #[test]
    fn parses_literals_and_scalars() {
        assert_eq!(parse_str("null").unwrap(), Value::Null);
        assert_eq!(parse_str("true").unwrap(), Value::Bool(true));
        assert_eq!(parse_str("false").unwrap(), Value::Bool(false));
        assert_eq!(parse_str("42").unwrap(), Value::Number(Number::from(42u64)));
        assert_eq!(
            parse_str("-1.5").unwrap(),
            Value::Number(Number::from_f64(-1.5).unwrap())
        );
        assert_eq!(
            parse_str(r#""hi""#).unwrap(),
            Value::String(String::from("hi"))
        );
    }

    #[test]
    fn parses_arrays_and_objects() {
        assert_eq!(parse_str("[]").unwrap(), Value::Array(Vec::new()));
        assert_eq!(
            parse_str("[1, 2, 3]").unwrap(),
            Value::Array(alloc::vec![
                Value::Number(Number::from(1u64)),
                Value::Number(Number::from(2u64)),
                Value::Number(Number::from(3u64)),
            ])
        );
        assert_eq!(parse_str("{}").unwrap(), Value::Object(Map::new()));
        let mut expected = Map::new();
        expected.insert(String::from("a"), Value::Number(Number::from(1u64)));
        expected.insert(String::from("b"), Value::Bool(true));
        assert_eq!(
            parse_str(r#"{"a": 1, "b": true}"#).unwrap(),
            Value::Object(expected)
        );
    }

    #[test]
    fn parses_nested_structures() {
        assert_eq!(
            parse_str(r#"{"outer": [1, {"inner": null}]}"#).unwrap(),
            Value::Object({
                let mut m = Map::new();
                m.insert(
                    String::from("outer"),
                    Value::Array(alloc::vec![
                        Value::Number(Number::from(1u64)),
                        Value::Object({
                            let mut inner = Map::new();
                            inner.insert(String::from("inner"), Value::Null);
                            inner
                        }),
                    ]),
                );
                m
            })
        );
    }

    #[test]
    fn skips_surrounding_whitespace() {
        assert_eq!(parse_str("  \n\t null  \r\n").unwrap(), Value::Null);
    }

    #[test]
    fn rejects_trailing_characters() {
        assert!(parse_str("null extra").is_err());
        assert!(parse_str("nulll").is_err());
    }

    #[test]
    fn rejects_empty_input() {
        assert!(parse_str("").is_err());
        assert!(parse_str("   ").is_err());
    }

    #[test]
    fn rejects_trailing_comma_in_array() {
        assert!(parse_str("[1,]").is_err());
    }

    #[test]
    fn rejects_trailing_comma_in_object() {
        assert!(parse_str(r#"{"a": 1,}"#).is_err());
    }

    #[test]
    fn rejects_missing_comma_in_array() {
        assert!(parse_str("[1 2]").is_err());
    }

    #[test]
    fn rejects_non_string_key() {
        assert!(parse_str("{1: 2}").is_err());
    }

    #[test]
    fn rejects_missing_colon() {
        assert!(parse_str(r#"{"a" 1}"#).is_err());
    }

    #[test]
    fn rejects_unterminated_array_and_object() {
        assert!(parse_str("[1, 2").is_err());
        assert!(parse_str(r#"{"a": 1"#).is_err());
    }

    #[test]
    fn duplicate_keys_last_write_wins() {
        let v = parse_str(r#"{"a": 1, "a": 2}"#).unwrap();
        let mut expected = Map::new();
        expected.insert(String::from("a"), Value::Number(Number::from(2u64)));
        assert_eq!(v, Value::Object(expected));
    }

    #[test]
    fn eof_errors_classify_as_eof() {
        assert!(parse_str("").unwrap_err().is_eof());
        assert!(parse_str("[1, 2").unwrap_err().is_eof());
        assert!(parse_str(r#"{"a": 1"#).unwrap_err().is_eof());
    }

    #[test]
    fn writes_compact_json() {
        assert_eq!(to_json_string(&Value::Null), "null");
        assert_eq!(to_json_string(&Value::Bool(true)), "true");
        assert_eq!(to_json_string(&Value::Number(Number::from(42u64))), "42");
        assert_eq!(
            to_json_string(&Value::String(String::from("hi\n"))),
            r#""hi\n""#
        );
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Bool(true));
        map.insert(String::from("b"), Value::Null);
        assert_eq!(
            to_json_string(&Value::Object(map)),
            r#"{"a":true,"b":null}"#
        );
        assert_eq!(
            to_json_string(&Value::Array(alloc::vec![
                Value::Number(Number::from(1u64)),
                Value::Number(Number::from(2u64)),
            ])),
            "[1,2]"
        );
    }

    #[test]
    fn writes_pretty_json() {
        let mut map = Map::new();
        map.insert(String::from("a"), Value::Number(Number::from(1u64)));
        let pretty = to_json_string_pretty(&Value::Object(map));
        assert_eq!(pretty, "{\n  \"a\": 1\n}");
        assert_eq!(to_json_string_pretty(&Value::Array(Vec::new())), "[]");
    }

    #[test]
    fn round_trips_through_parse_and_write() {
        let original = Value::Object({
            let mut m = Map::new();
            m.insert(
                String::from("nested"),
                Value::Array(alloc::vec![
                    Value::Number(Number::from(1u64)),
                    Value::Number(Number::from_f64(2.5).unwrap()),
                    Value::Null,
                    Value::String(String::from("quote\" and \\backslash")),
                ]),
            );
            m
        });
        let json = to_json_string(&original);
        let back = parse_str(&json).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn rejects_deeply_nested_arrays_instead_of_overflowing_the_stack() {
        // 200_000 levels of nesting is far past MAX_NESTING_DEPTH but far
        // below anything that would itself overflow the test harness's
        // stack -- the parser must bail with an `Err` well before that.
        let json = alloc::format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
        assert!(parse_str(&json).is_err());
    }
}
