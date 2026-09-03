//! Hand-rolled Base64 (RFC 4648), including the URL-safe alphabet used
//! throughout OAuth/JOSE (PKCE challenges, JWT segments).
//!
//! Extracted from `rusty_oauth::encoding::base64`, which already had the
//! complete surface (encode/decode, both alphabets) that `rusty_acp`,
//! `rusty-mcp`, and `rusty_a2a` each needed but were instead pulling from
//! the external `base64` crate for. `rusty_request`'s own `src/base64.rs`
//! was considered as a base for this instead, but it's private
//! (`pub(crate)`), encode-only, and standard-alphabet-only -- extending it
//! to cover decode and URL-safe would have meant building a second base64
//! crate from scratch, when `rusty_oauth`'s already covered it.
//!
//! The chunking here uses `chunks_exact`/`remainder` rather than the
//! newer `slice::as_chunks` that `rusty_oauth`'s original copy used --
//! `as_chunks` is not yet stable at `rusty_acp`'s own `rust-version =
//! "1.86"` floor (confirmed against a real `+1.86` toolchain), and this
//! crate needs to compile under every consumer's MSRV, not just the
//! workspace root's.

const STANDARD_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Padding {
    Include,
    Omit,
}

fn encode_with(data: &[u8], alphabet: &[u8; 64], padding: Padding) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let chunks = data.chunks_exact(3);
    let rem = chunks.remainder();

    for chunk in chunks {
        let n = (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | chunk[2] as u32;
        out.push(alphabet[((n >> 18) & 0x3f) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3f) as usize] as char);
        out.push(alphabet[((n >> 6) & 0x3f) as usize] as char);
        out.push(alphabet[(n & 0x3f) as usize] as char);
    }

    match rem.len() {
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(alphabet[((n >> 18) & 0x3f) as usize] as char);
            out.push(alphabet[((n >> 12) & 0x3f) as usize] as char);
            if padding == Padding::Include {
                out.push('=');
                out.push('=');
            }
        }
        2 => {
            let n = (rem[0] as u32) << 16 | (rem[1] as u32) << 8;
            out.push(alphabet[((n >> 18) & 0x3f) as usize] as char);
            out.push(alphabet[((n >> 12) & 0x3f) as usize] as char);
            out.push(alphabet[((n >> 6) & 0x3f) as usize] as char);
            if padding == Padding::Include {
                out.push('=');
            }
        }
        _ => {}
    }

    out
}

fn decode_char(c: u8, alphabet: &[u8; 64]) -> Option<u8> {
    alphabet.iter().position(|&a| a == c).map(|p| p as u8)
}

fn decode_with(data: &str, alphabet: &[u8; 64]) -> Result<Vec<u8>, DecodeError> {
    let bytes: Vec<u8> = data.bytes().filter(|&b| b != b'=').collect();
    if bytes.iter().any(|&b| decode_char(b, alphabet).is_none()) {
        return Err(DecodeError::InvalidCharacter);
    }

    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let chunks = bytes.chunks_exact(4);
    let rem = chunks.remainder();

    for chunk in chunks {
        let vals: [u8; 4] = [
            decode_char(chunk[0], alphabet).unwrap(),
            decode_char(chunk[1], alphabet).unwrap(),
            decode_char(chunk[2], alphabet).unwrap(),
            decode_char(chunk[3], alphabet).unwrap(),
        ];
        let n = (vals[0] as u32) << 18
            | (vals[1] as u32) << 12
            | (vals[2] as u32) << 6
            | vals[3] as u32;
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
    }

    match rem.len() {
        0 => {}
        2 => {
            let vals = [
                decode_char(rem[0], alphabet).unwrap(),
                decode_char(rem[1], alphabet).unwrap(),
            ];
            let n = (vals[0] as u32) << 18 | (vals[1] as u32) << 12;
            out.push((n >> 16) as u8);
        }
        3 => {
            let vals = [
                decode_char(rem[0], alphabet).unwrap(),
                decode_char(rem[1], alphabet).unwrap(),
                decode_char(rem[2], alphabet).unwrap(),
            ];
            let n = (vals[0] as u32) << 18 | (vals[1] as u32) << 12 | (vals[2] as u32) << 6;
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
        }
        _ => return Err(DecodeError::InvalidLength),
    }

    Ok(out)
}

/// Why a decode failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    InvalidCharacter,
    InvalidLength,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::InvalidCharacter => write!(f, "invalid base64 character"),
            DecodeError::InvalidLength => write!(f, "invalid base64 length"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Encodes `data` using standard Base64 with padding.
pub fn encode_standard(data: &[u8]) -> String {
    encode_with(data, STANDARD_ALPHABET, Padding::Include)
}

/// Decodes standard (padded or unpadded) Base64.
pub fn decode_standard(data: &str) -> Result<Vec<u8>, DecodeError> {
    decode_with(data, STANDARD_ALPHABET)
}

/// Encodes `data` using the URL-safe Base64 alphabet, without padding
/// (`base64url` as used by PKCE, RFC 7636 §4.2, and JOSE, RFC 7515 App. C).
pub fn encode_url_safe_no_pad(data: &[u8]) -> String {
    encode_with(data, URL_SAFE_ALPHABET, Padding::Omit)
}

/// Encodes `data` using the URL-safe Base64 alphabet, with padding.
pub fn encode_url_safe(data: &[u8]) -> String {
    encode_with(data, URL_SAFE_ALPHABET, Padding::Include)
}

/// Decodes `base64url` (padded or unpadded).
pub fn decode_url_safe(data: &str) -> Result<Vec<u8>, DecodeError> {
    decode_with(data, URL_SAFE_ALPHABET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (raw, encoded) in cases {
            assert_eq!(encode_standard(raw), *encoded);
            assert_eq!(decode_standard(encoded).unwrap(), *raw);
        }
    }

    #[test]
    fn url_safe_no_pad_roundtrip() {
        let data = b"\xff\xfe\xfd\x00\x01subjects?_d&e";
        let encoded = encode_url_safe_no_pad(data);
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
        assert_eq!(decode_url_safe(&encoded).unwrap(), data);
    }

    #[test]
    fn url_safe_padded_roundtrip() {
        let data = b"f";
        let encoded = encode_url_safe(data);
        assert_eq!(encoded, "Zg==");
        assert_eq!(decode_url_safe(&encoded).unwrap(), data);
    }

    #[test]
    fn decode_rejects_invalid_chars() {
        assert_eq!(decode_standard("!!!!"), Err(DecodeError::InvalidCharacter));
    }

    #[test]
    fn decode_rejects_invalid_length() {
        // Two leftover chars decode fine (see url_safe_no_pad_roundtrip's
        // "e" tail), but one leftover char is never valid Base64 -- 6 bits
        // can't recover a whole byte.
        assert_eq!(decode_standard("A"), Err(DecodeError::InvalidLength));
    }

    #[test]
    fn decode_error_display() {
        assert_eq!(
            DecodeError::InvalidCharacter.to_string(),
            "invalid base64 character"
        );
        assert_eq!(
            DecodeError::InvalidLength.to_string(),
            "invalid base64 length"
        );
    }
}
