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
    let bytes = data.as_bytes();

    // Padding, when present, is only ever the last one or two characters
    // (RFC 4648 §4). Anything else that looks like padding is corruption,
    // not something to strip and guess past: a decoder that silently
    // accepted `Z=9v` would push the corruption downstream.
    let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return Err(DecodeError::MisplacedPadding {
            index: bytes.len() - padding,
        });
    }
    let payload = &bytes[..bytes.len() - padding];
    if let Some(index) = payload.iter().position(|&b| b == b'=') {
        return Err(DecodeError::MisplacedPadding { index });
    }
    if let Some(index) = payload
        .iter()
        .position(|&b| decode_char(b, alphabet).is_none())
    {
        return Err(DecodeError::InvalidCharacter {
            byte: payload[index],
            index,
        });
    }

    // Padded input must be a whole number of 4-character groups. Unpadded
    // input (base64url per RFC 7515 App. C, and standard-alphabet callers
    // that simply omit it) is accepted as-is -- but a single trailing
    // character can never carry a whole byte, padded or not.
    if padding > 0 && bytes.len() % 4 != 0 {
        return Err(DecodeError::InvalidLength { len: bytes.len() });
    }
    if payload.len() % 4 == 1 {
        return Err(DecodeError::InvalidLength { len: bytes.len() });
    }

    let mut out = Vec::with_capacity(payload.len() * 3 / 4);
    let chunks = payload.chunks_exact(4);
    let rem = chunks.remainder();

    // Every byte of `payload` was validated against `alphabet` above, so
    // `decode_char` cannot fail below; `unwrap_or(0)` only spells that out
    // without a panic path.
    let val = |c: u8| decode_char(c, alphabet).unwrap_or(0) as u32;

    for chunk in chunks {
        let n = val(chunk[0]) << 18 | val(chunk[1]) << 12 | val(chunk[2]) << 6 | val(chunk[3]);
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
    }

    match rem.len() {
        2 => {
            let n = val(rem[0]) << 18 | val(rem[1]) << 12;
            out.push((n >> 16) as u8);
        }
        3 => {
            let n = val(rem[0]) << 18 | val(rem[1]) << 12 | val(rem[2]) << 6;
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
        }
        _ => {}
    }

    Ok(out)
}

/// Why a decode failed. Every variant names where in the input the
/// problem is, so a caller relaying the error (a wire-protocol peer's
/// corrupt payload, a malformed JWT segment) can say more than "bad
/// base64".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// A byte outside the alphabet (and not `=`) at `index`.
    InvalidCharacter { byte: u8, index: usize },
    /// The input's total length (`len`, padding included) cannot be valid
    /// Base64: a padded input that is not a whole number of 4-character
    /// groups, or an unpadded one with a single trailing character.
    InvalidLength { len: usize },
    /// A `=` somewhere other than the last one or two characters, or more
    /// than two of them; `index` is the first offending position.
    MisplacedPadding { index: usize },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::InvalidCharacter { byte, index } => write!(
                f,
                "invalid base64 character {:?} at index {index}",
                char::from(*byte)
            ),
            DecodeError::InvalidLength { len } => {
                write!(f, "invalid base64 length {len}")
            }
            DecodeError::MisplacedPadding { index } => {
                write!(f, "misplaced base64 padding at index {index}")
            }
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
        assert_eq!(
            decode_standard("!!!!"),
            Err(DecodeError::InvalidCharacter {
                byte: b'!',
                index: 0
            })
        );
        assert_eq!(
            decode_standard("Zm9*"),
            Err(DecodeError::InvalidCharacter {
                byte: b'*',
                index: 3
            })
        );
        // The URL-safe alphabet's `-`/`_` are not standard, and vice versa.
        assert!(matches!(
            decode_standard("-_-_"),
            Err(DecodeError::InvalidCharacter { .. })
        ));
        assert!(matches!(
            decode_url_safe("+/+/"),
            Err(DecodeError::InvalidCharacter { .. })
        ));
    }

    #[test]
    fn decode_rejects_invalid_length() {
        // Two leftover chars decode fine (see url_safe_no_pad_roundtrip's
        // "e" tail), but one leftover char is never valid Base64 -- 6 bits
        // can't recover a whole byte.
        assert_eq!(
            decode_standard("A"),
            Err(DecodeError::InvalidLength { len: 1 })
        );
        assert_eq!(
            decode_standard("Zm9vY"),
            Err(DecodeError::InvalidLength { len: 5 })
        );
        // Once padding is present the whole thing has to be 4-aligned:
        // one `=` where two are needed, or padding on an already-complete
        // group, are both malformed.
        assert_eq!(
            decode_standard("Zg="),
            Err(DecodeError::InvalidLength { len: 3 })
        );
        assert_eq!(
            decode_standard("Zm9v="),
            Err(DecodeError::InvalidLength { len: 5 })
        );
    }

    #[test]
    fn decode_accepts_missing_padding() {
        assert_eq!(decode_standard("Zg").unwrap(), b"f");
        assert_eq!(decode_standard("Zm8").unwrap(), b"fo");
        assert_eq!(decode_standard("Zm9vYg").unwrap(), b"foob");
    }

    #[test]
    fn decode_rejects_misplaced_padding() {
        assert_eq!(
            decode_standard("Z=9v"),
            Err(DecodeError::MisplacedPadding { index: 1 })
        );
        assert_eq!(
            decode_standard("Zm=v"),
            Err(DecodeError::MisplacedPadding { index: 2 })
        );
        // Padding in the middle of an otherwise-valid concatenation.
        assert_eq!(
            decode_standard("Zg==Zg=="),
            Err(DecodeError::MisplacedPadding { index: 2 })
        );
        // More than two trailing `=`.
        assert_eq!(
            decode_standard("Zm9v===="),
            Err(DecodeError::MisplacedPadding { index: 4 })
        );
    }

    #[test]
    fn decode_round_trips_every_length_remainder_and_every_byte() {
        for input in [
            &b""[..],
            &b"f"[..],
            &b"fo"[..],
            &b"foo"[..],
            &b"foob"[..],
            &b"fooba"[..],
            &b"foobar"[..],
        ] {
            assert_eq!(decode_standard(&encode_standard(input)).unwrap(), input);
            assert_eq!(
                decode_url_safe(&encode_url_safe_no_pad(input)).unwrap(),
                input
            );
        }
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        assert_eq!(
            decode_standard(&encode_standard(&all_bytes)).unwrap(),
            all_bytes
        );
        assert_eq!(
            decode_url_safe(&encode_url_safe(&all_bytes)).unwrap(),
            all_bytes
        );
    }

    #[test]
    fn decode_error_display() {
        assert_eq!(
            DecodeError::InvalidCharacter {
                byte: b'*',
                index: 3
            }
            .to_string(),
            "invalid base64 character '*' at index 3"
        );
        assert_eq!(
            DecodeError::InvalidLength { len: 5 }.to_string(),
            "invalid base64 length 5"
        );
        assert_eq!(
            DecodeError::MisplacedPadding { index: 1 }.to_string(),
            "misplaced base64 padding at index 1"
        );
    }
}
