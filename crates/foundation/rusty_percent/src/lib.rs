//! Percent-encoding for URL components and query strings, with no
//! dependencies.
//!
//! [`encode`] keeps only the RFC 3986 unreserved characters. The decoders are
//! lenient the way servers need to be: an escape that is not `%` plus two hex
//! digits is left as written. They differ only in what they do with `+` and
//! with bytes that are not UTF-8:
//!
//! | function | `+` | invalid UTF-8 |
//! |---|---|---|
//! | [`decode`] | kept | replaced with U+FFFD |
//! | [`decode_form`] | space | replaced with U+FFFD |
//! | [`decode_utf8`] | kept | `None` |
//!
//! This is for components and query values; `rusty_url` is the WHATWG URL
//! parser with its own spec-driven encode sets.

#![no_std]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

const UPPER_HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Percent-encodes everything except `A-Za-z0-9-_.~`, with uppercase hex.
pub fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(UPPER_HEX[usize::from(byte >> 4)]));
            out.push(char::from(UPPER_HEX[usize::from(byte & 0x0f)]));
        }
    }
    out
}

/// Decodes `%XX` escapes, keeping `+`. Invalid UTF-8 becomes U+FFFD.
pub fn decode(input: &str) -> Cow<'_, str> {
    decode_lossy(input, false)
}

/// Decodes `%XX` escapes and turns `+` into a space, as an
/// `application/x-www-form-urlencoded` query value is read. A `%2B` stays a
/// literal `+`. Invalid UTF-8 becomes U+FFFD.
pub fn decode_form(input: &str) -> Cow<'_, str> {
    decode_lossy(input, true)
}

/// Decodes `%XX` escapes, keeping `+`. `None` when the result is not UTF-8,
/// for callers that must not accept a lossy reading (path checks).
pub fn decode_utf8(input: &str) -> Option<String> {
    String::from_utf8(unescape(input.as_bytes(), false)).ok()
}

fn decode_lossy(input: &str, plus_is_space: bool) -> Cow<'_, str> {
    if !input
        .bytes()
        .any(|b| b == b'%' || (plus_is_space && b == b'+'))
    {
        return Cow::Borrowed(input);
    }
    match String::from_utf8(unescape(input.as_bytes(), plus_is_space)) {
        Ok(decoded) => Cow::Owned(decoded),
        Err(error) => Cow::Owned(String::from_utf8_lossy(error.as_bytes()).into_owned()),
    }
}

fn unescape(input: &[u8], plus_is_space: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'%' => {
                let high = input.get(i + 1).copied().and_then(hex_digit);
                let low = input.get(i + 2).copied().and_then(hex_digit);
                if let (Some(high), Some(low)) = (high, low) {
                    out.push((high << 4) | low);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_all_but_unreserved() {
        assert_eq!(encode("node-1_2.3~4"), "node-1_2.3~4");
        assert_eq!(encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(
            encode("2026-01-01T00:00:00+00:00"),
            "2026-01-01T00%3A00%3A00%2B00%3A00"
        );
        assert_eq!(encode("é"), "%C3%A9");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn decode_keeps_plus_and_leaves_bad_escapes_alone() {
        assert_eq!(decode("a%20b+c"), "a b+c");
        assert_eq!(decode("100%25"), "100%");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("100%2"), "100%2");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%C3%A9"), "é");
    }

    #[test]
    fn a_trailing_escape_is_decoded() {
        assert_eq!(decode("abc%41"), "abcA");
        assert_eq!(decode_form("abc%41"), "abcA");
    }

    #[test]
    fn decode_form_turns_plus_into_space_but_not_an_escaped_plus() {
        assert_eq!(decode_form("a%2Bb+c"), "a+b c");
        assert_eq!(
            decode_form("2026-08-05T12%3A00%3A00%2B00%3A00"),
            "2026-08-05T12:00:00+00:00"
        );
    }

    #[test]
    fn unescaped_input_is_not_copied() {
        assert!(matches!(decode("plain"), Cow::Borrowed(_)));
        assert!(matches!(decode_form("plain"), Cow::Borrowed(_)));
        assert!(matches!(decode_form("a+b"), Cow::Owned(_)));
    }

    #[test]
    fn invalid_utf8_is_lossy_or_rejected_by_the_variant() {
        assert_eq!(decode("%ff"), "\u{FFFD}");
        assert_eq!(decode_form("a%ffb"), "a\u{FFFD}b");
        assert_eq!(decode_utf8("%ff"), None);
        assert_eq!(decode_utf8("a%20b").as_deref(), Some("a b"));
        assert_eq!(decode_utf8("%zz").as_deref(), Some("%zz"));
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let text = "a b&c=d/é?+~%";
        assert_eq!(decode(&encode(text)), text);
        assert_eq!(decode_form(&encode(text)), text);
    }
}
