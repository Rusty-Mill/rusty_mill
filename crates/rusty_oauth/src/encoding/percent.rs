//! Percent-encoding (RFC 3986) for `application/x-www-form-urlencoded`
//! bodies and query strings, as required throughout RFC 6749.

/// RFC 3986 §2.3 "unreserved" characters, which never need escaping.
fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encodes `input` per RFC 3986, leaving only unreserved characters
/// untouched. Suitable for both path/query components and form values.
pub fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

/// Decodes a percent-encoded string. Invalid escapes are rejected.
pub fn decode(input: &str) -> Result<String, DecodeError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                let hi = hex_val(bytes[i + 1]).ok_or(DecodeError::InvalidEscape)?;
                let lo = hex_val(bytes[i + 2]).ok_or(DecodeError::InvalidEscape)?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| DecodeError::InvalidUtf8)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    InvalidEscape,
    InvalidUtf8,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "truncated percent-escape"),
            DecodeError::InvalidEscape => write!(f, "invalid percent-escape"),
            DecodeError::InvalidUtf8 => write!(f, "decoded bytes are not valid UTF-8"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Builds an `application/x-www-form-urlencoded` body from ordered key/value
/// pairs, as used for token requests (RFC 6749 §4.1.3) and metadata.
pub fn form_urlencode<'a, I>(pairs: I) -> String
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = String::new();
    for (i, (k, v)) in pairs.into_iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        out.push_str(&encode(k));
        out.push('=');
        out.push_str(&encode(v));
    }
    out
}

/// Parses an `application/x-www-form-urlencoded` body into key/value pairs.
pub fn form_urldecode(body: &str) -> Result<Vec<(String, String)>, DecodeError> {
    let mut out = Vec::new();
    if body.is_empty() {
        return Ok(out);
    }
    for pair in body.split('&') {
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        let k = decode(&k.replace('+', " "))?;
        let v = decode(&v.replace('+', " "))?;
        out.push((k, v));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_reserved_chars() {
        let raw = "hello world/foo?bar=baz&x=y#frag+plus";
        let encoded = encode(raw);
        assert!(!encoded.contains(' '));
        assert_eq!(decode(&encoded).unwrap(), raw);
    }

    #[test]
    fn unreserved_untouched() {
        let raw = "abcXYZ019-._~";
        assert_eq!(encode(raw), raw);
    }

    #[test]
    fn form_urlencode_basic() {
        let body = form_urlencode([
            ("grant_type", "authorization_code"),
            ("code", "abc 123"),
            ("redirect_uri", "https://example.com/cb"),
        ]);
        assert_eq!(
            body,
            "grant_type=authorization_code&code=abc%20123&redirect_uri=https%3A%2F%2Fexample.com%2Fcb"
        );
    }

    #[test]
    fn form_urldecode_basic() {
        let pairs = form_urldecode("a=1&b=hello+world&c=x%3Dy").unwrap();
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "hello world".to_string()),
                ("c".to_string(), "x=y".to_string()),
            ]
        );
    }
}
